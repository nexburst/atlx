//! `atlx bitbucket` (alias `bb`): clap definitions, dispatch and the
//! command registry.
//!
//! Grammar: `bb <resource> [<sub-resource>] <verb>`. The repository is
//! `--repo workspace/slug` everywhere; it defaults to `bitbucket.repo` in
//! `.atlx.toml`, else the bitbucket.org `origin` remote of the current
//! directory. Workspace-wide commands take `--workspace`, which defaults to
//! that repository's workspace, else the saved default workspace.

use super::Out;
use super::util::{DEFAULT_CONCURRENCY, batch, concurrency_arg};
use crate::bitbucket::{self, Listed, Paging, records as rec};
use crate::client::Client;
use crate::config::Config;
use crate::paging;
use crate::schema::{CommandMeta, Effect};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

mod pipelines;
mod prs;
mod repos;

// --- shared arguments --------------------------------------------------------

#[derive(Args, Clone, Default)]
pub(crate) struct RepoArg {
    /// Repository as workspace/slug (or a slug in the default workspace, or
    /// a Bitbucket URL) [default: .atlx.toml, else the git origin remote]
    #[arg(long, short = 'r', value_name = "WORKSPACE/SLUG")]
    pub repo: Option<String>,
}

#[derive(Args, Clone, Default)]
pub(crate) struct ListArgs {
    /// How many records to return [default: 25, at most 1000]
    #[arg(long, value_name = "N", value_parser = positive)]
    pub limit: Option<usize>,
    /// Continue from a previous result's `next`
    #[arg(long, value_name = "CURSOR")]
    pub cursor: Option<String>,
    /// As many as the per-call cap allows (1000)
    #[arg(long, conflicts_with = "limit")]
    pub all: bool,
}

impl ListArgs {
    pub fn paging(&self) -> Paging {
        Paging {
            limit: paging::effective_limit(self.limit, self.all),
            cursor: self.cursor.clone(),
        }
    }
}

/// `--strict` and `--concurrency` of a multi-ID `get`.
#[derive(Args, Clone)]
pub(crate) struct BatchArgs {
    /// Fail the whole command if any ID fails
    #[arg(long)]
    pub strict: bool,
    /// How many to read at once (1 to 20)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY, value_parser = concurrency_arg)]
    pub concurrency: usize,
}

/// A whole number of 1 or more.
pub(crate) fn positive(s: &str) -> std::result::Result<usize, String> {
    match s.trim().parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err("expected a whole number of 1 or more".into()),
    }
}

/// A whole number of 1 or more, as an ID.
pub(crate) fn positive_id(s: &str) -> std::result::Result<u64, String> {
    match s.trim().parse::<u64>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err("expected a whole number of 1 or more".into()),
    }
}

/// What every Bitbucket command needs: the client and the defaults.
pub(crate) struct Ctx<'a> {
    pub c: &'a Client,
    /// The saved default workspace.
    pub default_ws: Option<&'a str>,
    /// `.atlx.toml`'s `bitbucket.repo`, else the git origin remote.
    pub default_repo: Option<&'a str>,
}

impl Ctx<'_> {
    /// The REST path of `--repo`, else the default repository.
    pub fn repo(&self, r: &RepoArg) -> Result<String> {
        self.repo_of(r.repo.as_deref())
    }

    pub fn repo_of(&self, repo: Option<&str>) -> Result<String> {
        let ws = self
            .default_ws
            .or_else(|| self.default_repo.and_then(|r| r.split('/').next()));
        bitbucket::repo_path(repo.or(self.default_repo), None, ws)
    }

    /// The repository of a pull request argument: `--repo`, else a pull
    /// request URL's own repository, else the default.
    pub fn pr_repo(&self, r: &RepoArg, pr: &str) -> Result<String> {
        match (&r.repo, bitbucket::pr_url_repo(pr)) {
            (None, Some(from_url)) => self.repo_of(Some(&from_url)),
            _ => self.repo(r),
        }
    }

    /// `--workspace`, else `--repo`'s workspace, else the default
    /// repository's, else the saved default workspace.
    pub fn workspace(&self, ws: Option<&str>, repo: Option<&str>) -> Result<String> {
        if let Some(ws) = ws {
            return Ok(ws.to_string());
        }
        if let Some(r) = repo.or(self.default_repo) {
            return bitbucket::workspace_of(r, None, self.default_ws);
        }
        Ok(bitbucket::workspace_or(None, self.default_ws)?.to_string())
    }
}

/// A list result: `data` is the records, `next` the cursor or `null`.
pub(crate) fn listed((items, next): Listed) -> Value {
    json!({"ok": true, "data": items, "next": next})
}

/// A list result that cannot be resumed: `next` is null.
pub(crate) fn complete(items: Vec<Value>) -> Value {
    listed((items, None))
}

/// Several IDs read one by one (stdin with `-`), in input order.
pub(crate) fn get_many(
    ids: Vec<String>,
    b: &BatchArgs,
    f: impl Fn(&str) -> Result<Value> + Sync,
) -> Result<Value> {
    batch(ids, b.strict, b.concurrency, f)
}

// --- the tree ----------------------------------------------------------------

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum BbCmd {
    /// Repositories, and their files, branches, commits and settings
    Repo {
        #[command(subcommand)]
        cmd: repos::RepoCmd,
    },
    /// Pull requests, their comments, tasks and reviewers
    Pr {
        #[command(subcommand)]
        cmd: prs::PrCmd,
    },
    /// Pipeline runs, their steps and logs, and Pipelines settings
    Pipeline {
        #[command(subcommand)]
        cmd: pipelines::PipelineCmd,
    },
    /// Deployments to environments
    Deployment {
        #[command(subcommand)]
        cmd: pipelines::DeploymentCmd,
    },
    /// Deployment environments and their variables
    Environment {
        #[command(subcommand)]
        cmd: pipelines::EnvironmentCmd,
    },
    /// Workspaces and their members
    Workspace {
        #[command(subcommand)]
        cmd: repos::WorkspaceCmd,
    },
}

pub(crate) fn bb_cmd(cfg: &Config, cmd: BbCmd) -> Result<Out> {
    let (c, default_ws) = cfg.bitbucket()?;
    let ctx = Ctx {
        c: &c,
        default_ws: default_ws.as_deref(),
        default_repo: cfg.default_repo.as_deref(),
    };
    dispatch(&ctx, cmd)
}

pub(crate) fn dispatch(ctx: &Ctx, cmd: BbCmd) -> Result<Out> {
    match cmd {
        BbCmd::Repo { cmd } => repos::repo(ctx, cmd),
        BbCmd::Workspace { cmd } => repos::workspace(ctx, cmd).map(Out::Data),
        BbCmd::Pr { cmd } => prs::pr(ctx, cmd).map(Out::Data),
        BbCmd::Pipeline { cmd } => pipelines::pipeline(ctx, cmd),
        BbCmd::Deployment { cmd } => pipelines::deployment(ctx, cmd).map(Out::Data),
        BbCmd::Environment { cmd } => pipelines::environment(ctx, cmd).map(Out::Data),
    }
}

// --- registry ----------------------------------------------------------------

const fn read(path: &'static str, r: &'static crate::schema::Record) -> CommandMeta {
    CommandMeta::new(path, Effect::Read).record(r)
}

const fn write(path: &'static str, r: &'static crate::schema::Record) -> CommandMeta {
    CommandMeta::new(path, Effect::Write).record(r)
}

const fn destructive(path: &'static str, r: &'static crate::schema::Record) -> CommandMeta {
    CommandMeta::new(path, Effect::Destructive).record(r)
}

pub(crate) const META: &[CommandMeta] = &[
    // repositories
    read("bitbucket repo list", &rec::REPO).examples(&[
        "atlx bitbucket repo list --workspace acme --name api",
        "atlx bitbucket repo list -q 'project.key = \"CORE\"' --limit 50",
    ]),
    read("bitbucket repo get", &rec::REPO).examples(&[
        "atlx bitbucket repo get acme/api",
        "atlx bitbucket repo get acme/api acme/web",
    ]),
    write("bitbucket repo create", &rec::REPO).examples(&[
        "atlx bitbucket repo create acme/tools --project CORE",
        "atlx bitbucket repo create acme/site --public --description 'Docs site'",
    ]),
    read("bitbucket repo file get", &rec::FILE).examples(&[
        "atlx bitbucket repo file get README.md --repo acme/api",
        "atlx bitbucket repo file get src/main.rs --ref v1.2.0 --content-offset 20000",
    ]),
    read("bitbucket repo file list", &rec::FILE_ENTRY).examples(&[
        "atlx bitbucket repo file list --repo acme/api",
        "atlx bitbucket repo file list --path src --depth 3 --ref main",
    ]),
    read("bitbucket repo file download", &rec::DOWNLOAD).examples(&[
        "atlx bitbucket repo file download assets/logo.png --to ./logo.png",
    ]),
    read("bitbucket repo file search", &rec::CODE_HIT).examples(&[
        "atlx bitbucket repo file search 'fn main' --repo acme/api",
        "atlx bitbucket repo file search 'TODO ext:rs' --workspace acme --limit 50",
    ]),
    read("bitbucket repo branch list", &rec::BRANCH).examples(&[
        "atlx bitbucket repo branch list --name feature/",
        "atlx bitbucket repo branch list --author 'Ann Lee' --sort name",
    ]),
    read("bitbucket repo branch get", &rec::BRANCH)
        .examples(&["atlx bitbucket repo branch get main develop --repo acme/api"]),
    write("bitbucket repo branch create", &rec::BRANCH).examples(&[
        "atlx bitbucket repo branch create feature/login --from main",
        "atlx bitbucket repo branch create hotfix/1 --from 3f2a9c1",
    ]),
    destructive("bitbucket repo branch delete", &rec::DELETED)
        .examples(&["atlx bitbucket repo branch delete feature/old --yes"]),
    read("bitbucket repo commit list", &rec::COMMIT).examples(&[
        "atlx bitbucket repo commit list --branch develop --limit 10",
        "atlx bitbucket repo commit list --include feature/x --exclude main --path src/",
    ]),
    read("bitbucket repo commit get", &rec::COMMIT)
        .examples(&["atlx bitbucket repo commit get 3f2a9c1 --repo acme/api"]),
    write("bitbucket repo commit create", &rec::COMMIT_RESULT).examples(&[
        "atlx bitbucket repo commit create -m 'Update docs' --branch docs --file README.md",
        "atlx bitbucket repo commit create -m 'Drop old config' --default-branch --delete old.yml --parent 3f2a9c1",
    ]),
    read("bitbucket repo contributor list", &rec::CONTRIBUTOR).examples(&[
        "atlx bitbucket repo contributor list --since 90d",
        "atlx bitbucket repo contributor list --range v1.0..main --limit 5",
    ]),
    read("bitbucket repo report list", &rec::REPORT)
        .examples(&["atlx bitbucket repo report list --commit 3f2a9c1"]),
    write("bitbucket repo report create", &rec::REPORT).examples(&[
        "atlx bitbucket repo report create lint --commit 3f2a9c1 --report @report.json",
    ]),
    read("bitbucket repo default-reviewer list", &rec::USER).examples(&[
        "atlx bitbucket repo default-reviewer list --repo acme/api",
        "atlx bitbucket repo default-reviewer list --effective",
    ]),
    read("bitbucket repo default-reviewer get", &rec::USER)
        .examples(&["atlx bitbucket repo default-reviewer get '{1a2b3c4d-0000-4000-8000-000000000001}'"]),
    destructive("bitbucket repo default-reviewer add", &rec::USER)
        .examples(&["atlx bitbucket repo default-reviewer add 'Ann Lee' --yes"]),
    destructive("bitbucket repo default-reviewer remove", &rec::DELETED)
        .examples(&["atlx bitbucket repo default-reviewer remove 'Ann Lee' --yes"]),
    // workspaces
    read("bitbucket workspace get", &rec::WORKSPACE).examples(&[
        "atlx bitbucket workspace get",
        "atlx bitbucket workspace get acme",
    ]),
    read("bitbucket workspace member list", &rec::MEMBER).examples(&[
        "atlx bitbucket workspace member list --name ann",
        "atlx bitbucket workspace member list --workspace acme --permissions",
    ]),
    read("bitbucket workspace member get", &rec::MEMBER).examples(&[
        "atlx bitbucket workspace member get me",
        "atlx bitbucket workspace member get 'Ann Lee' --workspace acme",
    ]),
    // pull requests
    read("bitbucket pr list", &rec::PR_ROW).examples(&[
        "atlx bitbucket pr list --repo acme/api --state OPEN --dest main",
        "atlx bitbucket pr list --mine --waiting",
        "atlx bitbucket pr list --state MERGED --merged-since 7d",
    ]),
    read("bitbucket pr search", &rec::PR_ROW).examples(&[
        "atlx bitbucket pr search 'login timeout' --state ALL",
        "atlx bitbucket pr search flaky --in title --mine",
    ]),
    read("bitbucket pr get", &rec::PR).examples(&[
        "atlx bitbucket pr get 42 --repo acme/api",
        "atlx bitbucket pr get 42 43 44",
        "atlx bitbucket pr get https://bitbucket.org/acme/api/pull-requests/42",
    ]),
    write("bitbucket pr create", &rec::PR_ROW).examples(&[
        "atlx bitbucket pr create --title 'Add login' --source feature/login --body-file pr.md",
        "atlx bitbucket pr create --title Fix --source fix/1 --dest develop --reviewer 'Ann Lee' --default-reviewers --draft",
    ]),
    write("bitbucket pr update", &rec::PR_ROW).examples(&[
        "atlx bitbucket pr update 42 --title 'Add login (v2)'",
        "atlx bitbucket pr update 42 --ready --keep-source-branch",
    ]),
    destructive("bitbucket pr merge", &rec::PR_ROW).examples(&[
        "atlx bitbucket pr merge 42 --strategy squash --if-head 3f2a9c1b7e44 --yes",
        "atlx bitbucket pr merge 42 --dry-run",
    ]),
    destructive("bitbucket pr decline", &rec::PR_ROW)
        .examples(&["atlx bitbucket pr decline 42 --yes"]),
    write("bitbucket pr approve", &rec::REVIEW).examples(&["atlx bitbucket pr approve 42"]),
    write("bitbucket pr unapprove", &rec::REVIEW).examples(&["atlx bitbucket pr unapprove 42"]),
    write("bitbucket pr request-changes", &rec::REVIEW).examples(&[
        "atlx bitbucket pr request-changes 42",
        "atlx bitbucket pr request-changes 42 --withdraw",
    ]),
    read("bitbucket pr diff", &rec::PR_DIFF).examples(&[
        "atlx bitbucket pr diff 42 --path src/auth.rs",
        "atlx bitbucket pr diff 42 --grep validate_token",
        "atlx bitbucket pr diff 42 --diff-offset 40000",
    ]),
    read("bitbucket pr diffstat", &rec::DIFFSTAT).examples(&[
        "atlx bitbucket pr diffstat 42",
        "atlx bitbucket pr diffstat 42 --conflicts",
    ]),
    read("bitbucket pr commits", &rec::COMMIT).examples(&["atlx bitbucket pr commits 42"]),
    read("bitbucket pr activity", &rec::ACTIVITY).examples(&[
        "atlx bitbucket pr activity 42 --type comment --limit 50",
    ]),
    read("bitbucket pr checks", &rec::CHECKS).examples(&[
        "atlx bitbucket pr checks 42",
        "atlx bitbucket pr checks 42 --no-conflicts",
    ]),
    read("bitbucket pr comment list", &rec::PR_COMMENT)
        .examples(&["atlx bitbucket pr comment list --pr 42"]),
    read("bitbucket pr comment get", &rec::PR_COMMENT)
        .examples(&["atlx bitbucket pr comment get 901 902 --pr 42"]),
    write("bitbucket pr comment create", &rec::PR_COMMENT).examples(&[
        "atlx bitbucket pr comment create --pr 42 --body 'Looks good'",
        "atlx bitbucket pr comment create --pr 42 --body-file note.md --path src/auth.rs --line 18",
        "atlx bitbucket pr comment create --pr 42 --body Agreed --reply-to 901",
    ]),
    write("bitbucket pr comment update", &rec::PR_COMMENT)
        .examples(&["atlx bitbucket pr comment update 901 --pr 42 --body 'Fixed wording'"]),
    destructive("bitbucket pr comment delete", &rec::DELETED)
        .examples(&["atlx bitbucket pr comment delete 901 --pr 42 --yes"]),
    write("bitbucket pr comment resolve", &rec::PR_COMMENT)
        .examples(&["atlx bitbucket pr comment resolve 901 --pr 42"]),
    write("bitbucket pr comment reopen", &rec::PR_COMMENT)
        .examples(&["atlx bitbucket pr comment reopen 901 --pr 42"]),
    read("bitbucket pr task list", &rec::PR_TASK)
        .examples(&["atlx bitbucket pr task list --pr 42"]),
    read("bitbucket pr task get", &rec::PR_TASK)
        .examples(&["atlx bitbucket pr task get 7 --pr 42"]),
    write("bitbucket pr task create", &rec::PR_TASK).examples(&[
        "atlx bitbucket pr task create --pr 42 --body 'Add a test'",
        "atlx bitbucket pr task create --pr 42 --body 'Rename this' --comment 901",
    ]),
    write("bitbucket pr task update", &rec::PR_TASK)
        .examples(&["atlx bitbucket pr task update 7 --pr 42 --body 'Add two tests'"]),
    destructive("bitbucket pr task delete", &rec::DELETED)
        .examples(&["atlx bitbucket pr task delete 7 --pr 42 --yes"]),
    write("bitbucket pr task resolve", &rec::PR_TASK)
        .examples(&["atlx bitbucket pr task resolve 7 --pr 42"]),
    write("bitbucket pr task reopen", &rec::PR_TASK)
        .examples(&["atlx bitbucket pr task reopen 7 --pr 42"]),
    read("bitbucket pr reviewer list", &rec::REVIEWER)
        .examples(&["atlx bitbucket pr reviewer list --pr 42"]),
    write("bitbucket pr reviewer add", &rec::REVIEWER).examples(&[
        "atlx bitbucket pr reviewer add 'Ann Lee' bob --pr 42",
        "atlx bitbucket pr reviewer add me --pr 42",
    ]),
    write("bitbucket pr reviewer remove", &rec::REVIEWER)
        .examples(&["atlx bitbucket pr reviewer remove bob --pr 42"]),
    // pipelines
    read("bitbucket pipeline list", &rec::PIPELINE).examples(&[
        "atlx bitbucket pipeline list --branch main --result FAILED",
        "atlx bitbucket pipeline list --pattern nightly --limit 5",
    ]),
    read("bitbucket pipeline get", &rec::PIPELINE).examples(&[
        "atlx bitbucket pipeline get 1234",
        "atlx bitbucket pipeline get 1234 1235",
    ]),
    write("bitbucket pipeline run", &rec::PIPELINE).examples(&[
        "atlx bitbucket pipeline run --branch main",
        "atlx bitbucket pipeline run --branch main --custom nightly --variable ENV=staging",
        "atlx bitbucket pipeline run --rerun 1234",
    ]),
    read("bitbucket pipeline log", &rec::LOG).examples(&[
        "atlx bitbucket pipeline log --failed --branch main",
        "atlx bitbucket pipeline log 1234 --step 2 --log-offset 200",
        "atlx bitbucket pipeline log 1234 --grep 'error:' -i -C 2",
    ]),
    read("bitbucket pipeline step list", &rec::STEP)
        .examples(&["atlx bitbucket pipeline step list --pipeline 1234"]),
    read("bitbucket pipeline step get", &rec::STEP).examples(&[
        "atlx bitbucket pipeline step get 2 --pipeline 1234",
        "atlx bitbucket pipeline step get Test --pipeline 1234 --test-cases",
    ]),
    read("bitbucket pipeline config get", &rec::PIPELINE_CONFIG)
        .examples(&["atlx bitbucket pipeline config get"]),
    write("bitbucket pipeline config update", &rec::PIPELINE_CONFIG)
        .examples(&["atlx bitbucket pipeline config update --enabled false"]),
    // deployments
    read("bitbucket deployment list", &rec::DEPLOYMENT).examples(&[
        "atlx bitbucket deployment list --environment production --limit 5",
        "atlx bitbucket deployment list --environment staging --status FAILED --started-after 7d",
    ]),
    read("bitbucket deployment get", &rec::DEPLOYMENT).examples(&[
        "atlx bitbucket deployment get '{0b1c2d3e-0000-4000-8000-000000000001}'",
    ]),
    read("bitbucket deployment commits", &rec::DEPLOYMENT_DIFF).examples(&[
        "atlx bitbucket deployment commits '{0b1c2d3e-0000-4000-8000-000000000001}'",
    ]),
    read("bitbucket environment list", &rec::ENVIRONMENT)
        .examples(&["atlx bitbucket environment list --repo acme/api"]),
    read("bitbucket environment variable list", &rec::VARIABLE)
        .examples(&["atlx bitbucket environment variable list --environment production"]),
];

#[cfg(test)]
mod tests;

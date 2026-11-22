//! `bb branch create|delete`, `commit`, `workspace`, repository extras and
//! `search prs`.

use super::lifecycle::{PathOf, QueryScope};
use super::{RepoArgs, positive_limit};
use crate::bitbucket::{self, lifecycle::Role, repos};
use crate::cli::util::{self, require_yes};
use crate::client::{ApiError, Client};
use crate::config::Config;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use repos::Change;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

#[derive(Args)]
pub(crate) struct BranchCreateArgs {
    /// Name of the new branch
    #[arg(long, alias = "name")]
    branch: String,
    /// Branch or commit hash to start from [default: the main branch]
    #[arg(long)]
    from: Option<String>,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct BranchDeleteArgs {
    /// Branch name
    name: String,
    /// Confirm the delete (required)
    #[arg(long, short)]
    yes: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommitCmd {
    /// Commit history, of the main branch by default
    Query {
        /// Branch to read
        #[arg(long, conflicts_with = "include")]
        branch: Option<String>,
        /// Revisions to include; repeatable (atlx extra)
        #[arg(long)]
        include: Vec<String>,
        /// Revisions to exclude; repeatable (atlx extra)
        #[arg(long)]
        exclude: Vec<String>,
        /// Only commits touching this path (atlx extra)
        #[arg(long)]
        path: Option<String>,
        #[arg(long, short = 'n', default_value_t = 20, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Commit files without a checkout
    Create(CommitCreateArgs),
    /// Code Insights reports
    Report {
        #[command(subcommand)]
        cmd: ReportCmd,
    },
}

#[derive(Args)]
pub(crate) struct CommitCreateArgs {
    /// Commit message
    #[arg(long, short = 'm')]
    message: String,
    /// Local file to commit at the same relative path, or
    /// repoPath=localPath (atlx extra); repeatable
    #[arg(long)]
    file: Vec<String>,
    /// Local file for the --file-remote in the same position
    #[arg(long)]
    file_local: Vec<PathBuf>,
    /// Repository path for the --file-local in the same position
    #[arg(long)]
    file_remote: Vec<String>,
    /// JSON object mapping repository paths to text or to {"file": localPath}
    #[arg(long)]
    files_from: Option<PathBuf>,
    /// Repository path to delete; repeatable
    #[arg(long)]
    delete: Vec<String>,
    /// Branch to commit to
    #[arg(
        long,
        conflicts_with = "default_branch",
        required_unless_present = "default_branch"
    )]
    branch: Option<String>,
    /// Commit to the repository's default branch
    #[arg(long)]
    default_branch: bool,
    /// Author, as "Name <email>"
    #[arg(long)]
    author: Option<String>,
    /// Fail unless the branch is still at this commit
    #[arg(long)]
    parent: Option<String>,
    /// Confirm the commit (required)
    #[arg(long, short)]
    yes: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Subcommand)]
pub(crate) enum ReportCmd {
    /// Create or replace a report
    Create {
        /// Commit hash
        commit: String,
        /// Report ID
        #[arg(value_name = "report-id")]
        report_id: String,
        /// Report JSON
        #[arg(long)]
        report: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// A commit's reports
    Query {
        /// Commit hash
        commit: String,
        #[arg(long, short = 'n', default_value_t = 100, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCmd {
    /// Workspace details
    Get {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Workspace members
    Member {
        #[command(subcommand)]
        cmd: MemberCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemberCmd {
    /// List members
    Query {
        /// Nickname or display name contains (atlx extra)
        #[arg(long, short = 'q')]
        query: Option<String>,
        /// Include permission levels (needs permission to read them)
        #[arg(long)]
        include_permissions: bool,
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One member
    Get {
        /// UUID, account ID, nickname, display name or me
        member: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Args)]
pub(crate) struct RepoCreateArgs {
    /// Slug of the new repository
    slug: String,
    /// Display name
    #[arg(long)]
    name: Option<String>,
    /// Description
    #[arg(long, short = 'd')]
    description: Option<String>,
    /// Primary language
    #[arg(long)]
    language: Option<String>,
    /// Project key
    #[arg(long)]
    project: Option<String>,
    /// Private (the default)
    #[arg(long, conflicts_with = "public")]
    private: bool,
    /// Public: visible to everyone
    #[arg(long)]
    public: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct ListFilesArgs {
    /// Directory [default: the root]
    #[arg(long)]
    path: Option<String>,
    /// Branch, tag or commit [default: the main branch]
    #[arg(long = "ref")]
    git_ref: Option<String>,
    /// Descend this many levels (atlx extra)
    #[arg(long)]
    max_depth: Option<u32>,
    #[arg(long, short = 'n', default_value_t = 100, value_parser = positive_limit)]
    limit: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct UrlArgs {
    /// Repository slug, or workspace/slug
    slug: Option<String>,
    /// File or directory in it (atlx extra)
    #[arg(long)]
    path: Option<String>,
    /// Branch, tag or commit (atlx extra)
    #[arg(long = "ref")]
    git_ref: Option<String>,
    /// Line in --path (atlx extra)
    #[arg(long, requires = "path")]
    line: Option<u64>,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct ContributorsArgs {
    /// Branch, tag or commit [default: the main branch]
    #[arg(long = "ref", conflicts_with = "range")]
    git_ref: Option<String>,
    /// from..to
    #[arg(long)]
    range: Option<String>,
    /// Only commits since this day (YYYY-MM-DD or 30d; atlx extra)
    #[arg(long)]
    since: Option<String>,
    #[arg(long, short = 'n', default_value_t = 10, value_parser = positive_limit)]
    limit: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct SearchPrsArgs {
    /// Text to look for
    text: String,
    /// repo (default), or me: yours across recently updated repositories
    #[arg(long, value_enum)]
    scope: Option<QueryScope>,
    /// Where to look
    #[arg(long, value_enum, default_value = "both")]
    field: repos::Field,
    /// OPEN, MERGED, DECLINED, SUPERSEDED
    #[arg(long, default_value = "OPEN")]
    state: String,
    /// author (default) or reviewer, with --scope me
    #[arg(long, value_enum)]
    role: Option<Role>,
    #[arg(long, short = 'n', default_value_t = 25, value_parser = positive_limit)]
    limit: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

fn workspace(repo: &RepoArgs, default_ws: Option<&str>) -> Result<String> {
    match repo.repo.as_deref() {
        Some(r) => bitbucket::workspace_of(r, repo.workspace.as_deref(), default_ws),
        None => Ok(bitbucket::workspace_or(repo.workspace.as_deref(), default_ws)?.to_string()),
    }
}

pub(super) fn branch_create(c: &Client, path: PathOf, a: BranchCreateArgs) -> Result<Value> {
    repos::branch_create(c, &path(&a.repo)?, &a.branch, a.from.as_deref())
}

pub(super) fn branch_delete(c: &Client, path: PathOf, a: BranchDeleteArgs) -> Result<Value> {
    require_yes(a.yes, "deleting the branch")?;
    repos::branch_delete(c, &path(&a.repo)?, &a.name)
}

/// `--file`: a local path, committed at the same path, or `repo=local`.
fn file_flag(value: &str) -> Change {
    let same = |p: &str| Change::Local {
        repo_path: p.strip_prefix("./").unwrap_or(p).to_string(),
        local: PathBuf::from(p),
    };
    match value.split_once('=') {
        Some((remote, local)) if !Path::new(value).is_file() => Change::Local {
            repo_path: remote.to_string(),
            local: PathBuf::from(local),
        },
        _ => same(value),
    }
}

fn manifest(text: &str) -> Result<Vec<Change>> {
    let bad = || {
        usage(
            "--files-from must be a JSON object of repository paths to text or {\"file\": localPath}",
        )
    };
    let v: Value = serde_json::from_str(text).map_err(|_| bad())?;
    let mut out = Vec::new();
    for (repo_path, entry) in v.as_object().ok_or_else(bad)? {
        let repo_path = repo_path.clone();
        out.push(match (entry.as_str(), entry["file"].as_str()) {
            (Some(text), _) => Change::Text {
                repo_path,
                text: text.to_string(),
            },
            (None, Some(file)) => Change::Local {
                repo_path,
                local: PathBuf::from(file),
            },
            (None, None) => return Err(bad()),
        });
    }
    Ok(out)
}

fn commit_create(c: &Client, path: PathOf, a: CommitCreateArgs) -> Result<Value> {
    require_yes(a.yes, "the commit")?;
    if a.file_local.len() != a.file_remote.len() {
        return Err(usage("give one --file-remote for each --file-local"));
    }
    let mut changes: Vec<Change> = a.file.iter().map(|f| file_flag(f)).collect();
    for (local, remote) in a.file_local.into_iter().zip(a.file_remote) {
        changes.push(Change::Local {
            repo_path: remote,
            local,
        });
    }
    if let Some(p) = &a.files_from {
        let text =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        changes.extend(manifest(&text)?);
    }
    changes.extend(
        a.delete
            .into_iter()
            .map(|repo_path| Change::Delete { repo_path }),
    );
    repos::commit_create(
        c,
        &path(&a.repo)?,
        &repos::NewCommit {
            message: &a.message,
            branch: a.branch.as_deref(),
            author: a.author.as_deref(),
            parent: a.parent.as_deref(),
            changes: &changes,
        },
    )
}

pub(super) fn commit(c: &Client, path: PathOf, cmd: CommitCmd) -> Result<Value> {
    match cmd {
        CommitCmd::Query {
            branch,
            include,
            exclude,
            path: file_path,
            limit,
            repo,
        } => {
            let include: Vec<String> = branch.into_iter().chain(include).collect();
            repos::commit_query(
                c,
                &path(&repo)?,
                &repos::CommitQuery {
                    include: &include,
                    exclude: &exclude,
                    file_path: file_path.as_deref(),
                    limit,
                },
            )
        }
        CommitCmd::Create(a) => commit_create(c, path, a),
        CommitCmd::Report {
            cmd:
                ReportCmd::Create {
                    commit,
                    report_id,
                    report,
                    repo,
                },
        } => {
            let report: Value = serde_json::from_str(&report)
                .map_err(|e| usage(format!("--report is not valid JSON: {e}")))?;
            repos::report_create(c, &path(&repo)?, &commit, &report_id, &report)
        }
        CommitCmd::Report {
            cmd:
                ReportCmd::Query {
                    commit,
                    limit,
                    repo,
                },
        } => repos::reports(c, &path(&repo)?, &commit, limit),
    }
}

pub(super) fn workspace_cmd(
    c: &Client,
    default_ws: Option<&str>,
    cmd: WorkspaceCmd,
) -> Result<Value> {
    match cmd {
        WorkspaceCmd::Get { repo } => repos::workspace_get(c, &workspace(&repo, default_ws)?),
        WorkspaceCmd::Member {
            cmd:
                MemberCmd::Query {
                    query,
                    include_permissions,
                    limit,
                    repo,
                },
        } => {
            let ws = workspace(&repo, default_ws)?;
            repos::members(c, &ws, query.as_deref(), include_permissions, limit)
        }
        WorkspaceCmd::Member {
            cmd: MemberCmd::Get { member, repo },
        } => repos::member(c, &workspace(&repo, default_ws)?, &member),
    }
}

pub(super) fn repo_create(
    c: &Client,
    default_ws: Option<&str>,
    a: RepoCreateArgs,
) -> Result<Value> {
    if a.repo.repo.is_some() {
        return Err(usage(
            "name the new repository with the argument and --workspace, not --repo",
        ));
    }
    let path = bitbucket::repo_path(Some(&a.slug), a.repo.workspace.as_deref(), default_ws)?;
    repos::repo_create(
        c,
        &path,
        &repos::NewRepo {
            name: a.name.as_deref(),
            description: a.description.as_deref(),
            language: a.language.as_deref(),
            project: a.project.as_deref(),
            private: a.private || !a.public,
        },
    )
}

pub(super) fn list_files(c: &Client, path: PathOf, a: ListFilesArgs) -> Result<Value> {
    repos::list_files(
        c,
        &path(&a.repo)?,
        a.git_ref.as_deref(),
        a.path.as_deref(),
        a.max_depth,
        a.limit,
    )
}

/// `repo url` sends nothing, so it runs before credentials are needed.
pub(super) fn url(cfg: &Config, a: &UrlArgs) -> Result<Value> {
    let path = bitbucket::repo_path(
        a.slug.as_deref().or(a.repo.repo.as_deref()),
        a.repo.workspace.as_deref(),
        cfg.bitbucket.workspace.as_deref(),
    )?;
    let url = repos::web_url(&path, a.path.as_deref(), a.git_ref.as_deref(), a.line)?;
    let mut parts = path.trim_start_matches("/repositories/").split('/');
    let decode =
        |part: &str| urlencoding::decode(part).map_or(part.to_string(), |s| s.into_owned());
    let workspace = decode(parts.next().unwrap_or_default());
    let repo = decode(parts.next().unwrap_or_default());
    Ok(json!({"workspace": workspace, "repo": repo, "url": url}))
}

pub(super) fn contributors(c: &Client, path: PathOf, a: ContributorsArgs) -> Result<Value> {
    let since = a.since.as_deref().map(crate::time::to_date).transpose()?;
    repos::contributors(
        c,
        &path(&a.repo)?,
        &repos::Contributors {
            git_ref: a.git_ref.as_deref(),
            range: a.range.as_deref(),
            since: since.as_deref(),
            limit: a.limit,
        },
    )
}

pub(super) fn search_prs(
    c: &Client,
    path: PathOf,
    default_ws: Option<&str>,
    a: SearchPrsArgs,
) -> Result<Value> {
    if a.scope != Some(QueryScope::Me) {
        if a.role.is_some() {
            return Err(usage("--role goes with --scope me"));
        }
        return repos::search_prs(c, &path(&a.repo)?, &a.text, a.field, &a.state, a.limit);
    }
    let ws = workspace(&a.repo, default_ws)?;
    let mine = bitbucket::lifecycle::Mine {
        workspace: &ws,
        repo: None,
        role: Some(a.role.unwrap_or(Role::Author)),
        waiting: false,
        state: &a.state,
        limit: a.limit,
        repo_limit: 25,
        repo_role: "member",
        concurrency: util::DEFAULT_CONCURRENCY,
    };
    repos::search_my_prs(c, &mine, &a.text, a.field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_flags_and_manifests() {
        let local = |repo_path: &str, p: &str| Change::Local {
            repo_path: repo_path.into(),
            local: p.into(),
        };
        assert_eq!(file_flag("./docs/a.md"), local("docs/a.md", "./docs/a.md"));
        assert_eq!(
            file_flag("docs/a.md=/tmp/new.md"),
            local("docs/a.md", "/tmp/new.md")
        );
        let m = manifest(r#"{"a.txt": "hi", "b.bin": {"file": "local.bin"}}"#).unwrap();
        assert!(m.contains(&local("b.bin", "local.bin")));
        assert_eq!(m.len(), 2);
        assert!(manifest("[]").is_err() && manifest(r#"{"a": 1}"#).is_err());
    }
}

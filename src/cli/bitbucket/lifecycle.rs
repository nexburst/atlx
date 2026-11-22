//! `bb pull-requests` lifecycle commands and `bb inbox`.

use super::{RepoArgs, positive_limit};
use crate::cli::util::{body_arg, require_yes};
use crate::client::{ApiError, Client};
use crate::{bitbucket, jira};
use anyhow::Result;
use bitbucket::lifecycle::{self, Role, User};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;

/// Resolves a command's repository flags to its REST path.
pub(super) type PathOf<'a> = &'a dyn Fn(&RepoArgs) -> Result<String>;

/// `--scope` on `pull-requests query`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum QueryScope {
    Repo,
    Me,
}

/// `--scope` on `inbox`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InboxScope {
    Workspace,
    Repo,
}

/// `--role` on `inbox`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InboxRole {
    Reviewer,
    Author,
    /// Both (atlx extra)
    All,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// Pull request number or URL
    #[arg(long)]
    pull_request: String,
    /// New title
    #[arg(long)]
    title: Option<String>,
    /// New description; `-` reads stdin
    #[arg(long, conflicts_with_all = ["description_file", "clear_description"])]
    description: Option<String>,
    /// Read the new description from a file
    #[arg(
        long,
        visible_alias = "desc-file",
        conflicts_with = "clear_description"
    )]
    description_file: Option<String>,
    /// Empty the description
    #[arg(long)]
    clear_description: bool,
    /// New destination branch
    #[arg(long)]
    dest: Option<String>,
    /// Replace the reviewers: names, nicknames, account IDs, {uuid}s or me
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    reviewer: Vec<String>,
    /// Replace the reviewers, by account ID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    reviewer_account_id: Vec<String>,
    /// Replace the reviewers, by UUID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    reviewer_uuid: Vec<String>,
    /// Add reviewers
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    add_reviewer: Vec<String>,
    /// Add reviewers, by account ID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    add_reviewer_account_id: Vec<String>,
    /// Add reviewers, by UUID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    add_reviewer_uuid: Vec<String>,
    /// Remove reviewers
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    remove_reviewer: Vec<String>,
    /// Remove reviewers, by account ID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    remove_reviewer_account_id: Vec<String>,
    /// Remove reviewers, by UUID
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    remove_reviewer_uuid: Vec<String>,
    /// Remove every reviewer
    #[arg(long, conflicts_with_all = ["reviewer", "reviewer_account_id", "reviewer_uuid", "remove_reviewer", "remove_reviewer_account_id", "remove_reviewer_uuid"])]
    clear_reviewers: bool,
    /// Close the source branch on merge
    #[arg(long, conflicts_with = "keep_source_branch")]
    close_source_branch: bool,
    /// Keep the source branch on merge
    #[arg(long)]
    keep_source_branch: bool,
    /// Turn into a draft
    #[arg(long, conflicts_with = "ready")]
    draft: bool,
    /// Mark ready for review
    #[arg(long)]
    ready: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct PrArgs {
    /// Pull request number or URL
    id: String,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct MergeabilityArgs {
    /// Pull request number or URL
    id: String,
    /// Skip the merge conflict check
    #[arg(long)]
    exclude_git_mergeability_check: bool,
    /// Don't report on how far the source branch is behind
    #[arg(long)]
    exclude_maximum_commits_behind: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct MergeStatusArgs {
    /// Pull request number or URL
    #[arg(long)]
    pull_request: String,
    /// Merge task ID, as a merge still in progress prints it
    #[arg(long)]
    task: String,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct ConflictsArgs {
    /// Pull request number or URL
    id: String,
    #[arg(long, short = 'n', default_value_t = 100, value_parser = positive_limit)]
    limit: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct ForCommitArgs {
    /// Commit hash
    commit: String,
    #[arg(long, short = 'n', default_value_t = 25, value_parser = positive_limit)]
    limit: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Subcommand)]
pub(crate) enum DefaultReviewerCmd {
    /// The repository's default reviewers
    Query {
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One default reviewer
    Get {
        /// User UUID
        uuid: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Make a user a default reviewer
    Add {
        /// User UUID
        uuid: String,
        /// Confirm the change (required)
        #[arg(long, short)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Stop a user being a default reviewer
    Remove {
        /// User UUID
        uuid: String,
        /// Confirm the change (required)
        #[arg(long, short)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum EffectiveDefaultReviewerCmd {
    /// Default reviewers, including those inherited from the project
    Query {
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Args)]
pub(crate) struct InboxArgs {
    /// workspace (default), or only --repo
    #[arg(long, value_enum)]
    scope: Option<InboxScope>,
    /// reviewer: not approved by you yet (default); author: yours, with
    /// changes requested; all: both
    #[arg(long, value_enum, default_value = "reviewer")]
    role: InboxRole,
    /// OPEN, MERGED, DECLINED, SUPERSEDED
    #[arg(long, visible_alias = "status", default_value = "OPEN")]
    state: String,
    #[arg(long, short = 'n', default_value_t = 25, value_parser = positive_limit)]
    limit: usize,
    /// Most recently updated repositories to scan; each costs a request
    #[arg(long, default_value_t = 25, value_parser = positive_limit)]
    repo_limit: usize,
    /// Scan repositories where you have this role
    #[arg(long, default_value = "member")]
    repo_role: String,
    /// How many repositories to read at once (1 to 20)
    #[arg(long, default_value_t = crate::cli::util::DEFAULT_CONCURRENCY, value_parser = crate::cli::util::concurrency_arg)]
    concurrency: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

fn users<'a>(names: &'a [String], accounts: &'a [String], uuids: &'a [String]) -> Vec<User<'a>> {
    names
        .iter()
        .map(|s| User::Any(s.trim()))
        .chain(accounts.iter().map(|s| User::AccountId(s.trim())))
        .chain(uuids.iter().map(|s| User::Uuid(s.trim())))
        .collect()
}

pub(super) fn reviewers(
    c: &Client,
    path: &str,
    names: &[String],
    accounts: Option<&str>,
    uuids: Option<&str>,
    defaults: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let accounts = accounts.map(jira::csv).unwrap_or_default();
    let uuids = uuids.map(jira::csv).unwrap_or_default();
    lifecycle::new_reviewers(c, path, &users(names, &accounts, &uuids), defaults)
}

pub(super) fn update(c: &Client, path: PathOf, a: UpdateArgs) -> Result<Value> {
    let id = bitbucket::pr_number(&a.pull_request)?;
    let mut description = body_arg(a.description, a.description_file)?;
    if a.clear_description {
        description = Some(String::new());
    }
    let replace = users(&a.reviewer, &a.reviewer_account_id, &a.reviewer_uuid);
    let set = (a.clear_reviewers || !replace.is_empty()).then_some(replace);
    let flag = |on: bool, off: bool| (on || off).then_some(on);
    lifecycle::pr_update(
        c,
        &path(&a.repo)?,
        id,
        &lifecycle::PrUpdate {
            title: a.title.as_deref(),
            description: description.as_deref(),
            dest: a.dest.as_deref(),
            set,
            add: users(
                &a.add_reviewer,
                &a.add_reviewer_account_id,
                &a.add_reviewer_uuid,
            ),
            remove: users(
                &a.remove_reviewer,
                &a.remove_reviewer_account_id,
                &a.remove_reviewer_uuid,
            ),
            close_source_branch: flag(a.close_source_branch, a.keep_source_branch),
            draft: flag(a.draft, a.ready),
        },
    )
}

pub(super) fn remove_request_changes(c: &Client, path: PathOf, a: PrArgs) -> Result<Value> {
    lifecycle::remove_request_changes(c, &path(&a.repo)?, bitbucket::pr_number(&a.id)?)
}

pub(super) fn mergeability(c: &Client, path: PathOf, a: MergeabilityArgs) -> Result<Value> {
    let exclude = lifecycle::Exclude {
        git_mergeability: a.exclude_git_mergeability_check,
        commits_behind: a.exclude_maximum_commits_behind,
    };
    lifecycle::mergeability(c, &path(&a.repo)?, bitbucket::pr_number(&a.id)?, exclude)
}

pub(super) fn merge_status(c: &Client, path: PathOf, a: MergeStatusArgs) -> Result<Value> {
    let id = bitbucket::pr_number(&a.pull_request)?;
    lifecycle::merge_status(c, &path(&a.repo)?, id, &a.task)
}

pub(super) fn conflicts(c: &Client, path: PathOf, a: ConflictsArgs) -> Result<Value> {
    lifecycle::conflicts(c, &path(&a.repo)?, bitbucket::pr_number(&a.id)?, a.limit)
}

pub(super) fn for_commit(c: &Client, path: PathOf, a: ForCommitArgs) -> Result<Value> {
    lifecycle::for_commit(c, &path(&a.repo)?, &a.commit, a.limit)
}

pub(super) fn default_reviewer(c: &Client, path: PathOf, cmd: DefaultReviewerCmd) -> Result<Value> {
    match cmd {
        DefaultReviewerCmd::Query { limit, repo } => {
            lifecycle::default_reviewers(c, &path(&repo)?, false, limit)
        }
        DefaultReviewerCmd::Get { uuid, repo } => {
            lifecycle::default_reviewer_get(c, &path(&repo)?, &uuid)
        }
        DefaultReviewerCmd::Add { uuid, yes, repo } => {
            require_yes(yes, "adding the default reviewer")?;
            lifecycle::default_reviewer_add(c, &path(&repo)?, &uuid)
        }
        DefaultReviewerCmd::Remove { uuid, yes, repo } => {
            require_yes(yes, "removing the default reviewer")?;
            lifecycle::default_reviewer_remove(c, &path(&repo)?, &uuid)
        }
    }
}

pub(super) fn effective_default_reviewer(
    c: &Client,
    path: PathOf,
    cmd: EffectiveDefaultReviewerCmd,
) -> Result<Value> {
    let EffectiveDefaultReviewerCmd::Query { limit, repo } = cmd;
    lifecycle::default_reviewers(c, &path(&repo)?, true, limit)
}

fn workspace(repo: &RepoArgs, default_ws: Option<&str>) -> Result<String> {
    match repo.repo.as_deref() {
        Some(r) => bitbucket::workspace_of(r, repo.workspace.as_deref(), default_ws),
        None => Ok(bitbucket::workspace_or(repo.workspace.as_deref(), default_ws)?.to_string()),
    }
}

pub(super) fn inbox(
    c: &Client,
    path: PathOf,
    default_ws: Option<&str>,
    a: InboxArgs,
) -> Result<Value> {
    if a.scope == Some(InboxScope::Repo) && a.repo.repo.is_none() {
        return Err(ApiError::usage("--scope repo needs --repo").into());
    }
    // Naming a repository narrows the inbox to it.
    let repo = a.repo.repo.is_some().then(|| path(&a.repo)).transpose()?;
    lifecycle::mine(
        c,
        &lifecycle::Mine {
            workspace: &workspace(&a.repo, default_ws)?,
            repo: repo.as_deref(),
            role: match a.role {
                InboxRole::Reviewer => Some(Role::Reviewer),
                InboxRole::Author => Some(Role::Author),
                InboxRole::All => None,
            },
            waiting: true,
            state: &a.state,
            limit: a.limit,
            repo_limit: a.repo_limit,
            repo_role: &a.repo_role,
            concurrency: a.concurrency,
        },
    )
}

/// `pull-requests query --scope me`.
pub(super) fn query_mine(
    c: &Client,
    repo: &RepoArgs,
    default_ws: Option<&str>,
    role: Option<Role>,
    q: &bitbucket::PrQuery,
    merged: bool,
) -> Result<Value> {
    let narrowed = merged
        || q.page.is_some()
        || [
            q.author,
            q.reviewer,
            q.source,
            q.dest,
            q.text,
            q.created_since,
            q.created_before,
            q.updated_since,
            q.updated_before,
        ]
        .iter()
        .any(Option::is_some);
    if narrowed {
        return Err(ApiError::usage(
            "--scope me takes only --state, --role and --limit; use --repo for the other filters",
        )
        .into());
    }
    lifecycle::mine(
        c,
        &lifecycle::Mine {
            workspace: &workspace(repo, default_ws)?,
            repo: None,
            role: Some(role.unwrap_or(Role::Author)),
            waiting: false,
            state: q.state.unwrap_or("OPEN"),
            limit: q.limit,
            repo_limit: 25,
            repo_role: "member",
            concurrency: crate::cli::util::DEFAULT_CONCURRENCY,
        },
    )
}

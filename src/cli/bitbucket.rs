//! `atlx bitbucket` (alias `bb`): clap definitions and dispatch.

use super::Out;
use super::util::{
    DEFAULT_CONCURRENCY, batch, body_arg, concurrency_arg, deprecated, require_yes, text_arg,
};
use crate::bitbucket;
use crate::client::{ApiError, Client};
use crate::config::Config;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

mod lifecycle;

#[derive(Args)]
pub(crate) struct RepoArgs {
    /// Bitbucket workspace slug
    #[arg(long, short = 'w')]
    workspace: Option<String>,
    /// Repository slug, workspace/repo, or Bitbucket URL
    #[arg(long, short = 'r')]
    repo: Option<String>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum BbCmd {
    /// Repositories
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    /// Pull requests
    #[command(name = "pull-requests", visible_alias = "prs")]
    PullRequests {
        #[command(subcommand)]
        cmd: PrCmd,
    },
    /// Branches
    Branch {
        #[command(subcommand)]
        cmd: BranchCmd,
    },
    /// Pipelines
    Pipeline {
        #[command(subcommand)]
        cmd: PipelineCmd,
    },
    /// Pull requests waiting on you
    ///
    /// Bitbucket has no workspace-wide reviewer query, so this scans your
    /// most recently updated repositories (25 unless --repo-limit says
    /// otherwise), at one request or more each.
    Inbox(lifecycle::InboxArgs),
    /// Search code in a workspace
    Search {
        #[command(subcommand)]
        cmd: BbSearchCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum BbSearchCmd {
    /// Search code across repositories in a workspace
    Code {
        #[arg(required = true)]
        query: Vec<String>,
        #[arg(long)]
        workspace: Option<String>,
        /// Restrict search to a repository slug
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum BranchCmd {
    /// List or search the branches of a repository
    Query {
        /// Branch name substring
        #[arg(long, short = 'q')]
        query: Option<String>,
        /// Head commit author: display name or account ID
        #[arg(long)]
        author: Option<String>,
        /// Sort field: name, -name, target.date or -target.date
        #[arg(long, default_value = "-target.date")]
        sort: String,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum RepoCmd {
    /// List repositories in a workspace
    ///
    /// `--repo workspace/slug` (or a URL) also selects the workspace.
    Query {
        /// Search by name
        #[arg(long, short = 'q')]
        query: Option<String>,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long, default_value = "-updated_on")]
        sort: String,
        /// owner, admin, contributor, member
        #[arg(long)]
        role: Option<String>,
    },
    /// Repository details
    Get {
        slug: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Read a file or list a directory from the repository
    File {
        /// File or directory path within the repository
        path: String,
        /// Branch name, tag, or commit hash (defaults to the main branch)
        #[arg(long = "ref")]
        git_ref: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Old name of `bb branch query`
    #[command(hide = true)]
    Branches {
        /// Filter branch names
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum PrCmd {
    /// List pull requests in a repository
    Query {
        text: Vec<String>,
        /// OPEN, MERGED, DECLINED, SUPERSEDED
        #[arg(long)]
        state: Option<String>,
        /// Nickname, display name, account ID, or {uuid}
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        dest: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        created_since: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long)]
        updated_since: Option<String>,
        #[arg(long)]
        updated_before: Option<String>,
        /// Nickname, account ID, {uuid} or me
        #[arg(long)]
        reviewer: Option<String>,
        /// Merged on or after this day; reads each candidate's activity
        #[arg(long)]
        merged_since: Option<String>,
        /// Merged before this day; reads each candidate's activity
        #[arg(long)]
        merged_before: Option<String>,
        /// repo (default), or me: yours across recently updated repositories
        #[arg(long, value_enum)]
        scope: Option<lifecycle::QueryScope>,
        /// Your part in them, with --scope me (default author)
        #[arg(long, value_enum)]
        role: Option<bitbucket::lifecycle::Role>,
        /// One page, counted from 1
        #[arg(long, value_parser = positive_limit)]
        page: Option<usize>,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One or more pull requests by number or URL
    Get {
        ids: Vec<String>,
        #[arg(long)]
        comments: bool,
        /// File-level diff stats
        #[arg(long)]
        diff: bool,
        /// Build/CI statuses
        #[arg(long)]
        statuses: bool,
        /// Comments, diff stats and statuses
        #[arg(long)]
        full: bool,
        #[arg(long)]
        strict: bool,
        /// How many pull requests to fetch at once (1 to 20)
        #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY, value_parser = concurrency_arg)]
        batch_concurrency: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Create a pull request
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        source: String,
        /// Defaults to the repository main branch
        #[arg(long)]
        dest: Option<String>,
        #[arg(long, conflicts_with = "description_file")]
        description: Option<String>,
        #[arg(long, visible_alias = "desc-file")]
        description_file: Option<String>,
        /// Reviewers: names, nicknames, account IDs, {uuid}s or me
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        reviewer: Vec<String>,
        /// Add the repository's default reviewers (atlx extra)
        #[arg(long)]
        default_reviewers: bool,
        /// Comma-separated Bitbucket user UUIDs
        #[arg(long)]
        reviewer_uuid: Option<String>,
        /// Comma-separated Atlassian account IDs
        #[arg(long)]
        reviewer_account_id: Option<String>,
        #[arg(long)]
        close_source_branch: bool,
        #[arg(long)]
        draft: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Pull request comments
    Comment {
        #[command(subcommand)]
        cmd: PrCommentCmd,
    },
    /// Raw unified diff for a pull request
    Diff {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Find lines in a pull request diff and the line numbers to comment on
    DiffLine {
        /// Pull request number or URL
        id: String,
        /// Text the diff line must contain
        #[arg(long)]
        text: String,
        /// Only look in this file
        #[arg(long)]
        path: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Files changed by a pull request, with lines added and removed
    Diffstat {
        /// Pull request number or URL
        id: String,
        #[arg(long, short = 'n', default_value_t = 100, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Commits in a pull request
    Commits {
        /// Pull request number or URL
        #[arg(value_name = "pullRequest")]
        pull_request: String,
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Pull request as an mbox patch (for `git am`)
    Patch {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Activity feed for one or more pull requests
    Activity {
        /// Pull request numbers or URLs
        #[arg(required = true)]
        ids: Vec<String>,
        /// Filter by event type (e.g. comment, approval, update)
        #[arg(long = "type")]
        event_type: Option<String>,
        /// Maximum activities per pull request
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        /// Fail the batch if any pull request fails
        #[arg(long)]
        strict: bool,
        /// How many pull requests to fetch at once (1 to 20)
        #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY, value_parser = concurrency_arg)]
        batch_concurrency: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Pull request tasks
    Task {
        #[command(subcommand)]
        cmd: PrTaskCmd,
    },
    /// Old name of `bb pull-requests task query`
    #[command(hide = true)]
    Tasks {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Approve a pull request
    Approve {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Remove your approval from a pull request
    Unapprove {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Request changes on a pull request
    RequestChanges {
        /// Pull request number or URL
        id: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Change a pull request's fields and reviewers
    Update(lifecycle::UpdateArgs),
    /// Withdraw your request for changes
    RemoveRequestChanges(lifecycle::PrArgs),
    /// What blocks a merge, as far as the REST API tells
    Mergeability(lifecycle::MergeabilityArgs),
    /// The state of a merge still in progress
    MergeStatus(lifecycle::MergeStatusArgs),
    /// Files with merge conflicts
    Conflicts(lifecycle::ConflictsArgs),
    /// Pull requests that contain a commit
    ForCommit(lifecycle::ForCommitArgs),
    /// Default reviewers of a repository
    DefaultReviewer {
        #[command(subcommand)]
        cmd: lifecycle::DefaultReviewerCmd,
    },
    /// Default reviewers, inherited ones included
    EffectiveDefaultReviewer {
        #[command(subcommand)]
        cmd: lifecycle::EffectiveDefaultReviewerCmd,
    },
    /// Merge a pull request
    Merge {
        /// Pull request number or URL
        #[arg(long, conflicts_with = "id")]
        pull_request: Option<String>,
        /// Old form of --pull-request
        #[arg(hide = true)]
        id: Option<String>,
        /// Defaults to the repository's merge strategy
        #[arg(long, value_enum, conflicts_with = "strategy")]
        merge_strategy: Option<bitbucket::MergeStrategy>,
        /// Commit message for the merge
        #[arg(long, conflicts_with = "message")]
        merge_message: Option<String>,
        /// Delete the source branch after this merge
        #[arg(long, conflicts_with = "keep_source_branch")]
        close_source_branch: bool,
        /// Keep the source branch even if the pull request was set to close it
        #[arg(long)]
        keep_source_branch: bool,
        /// Wait until the merge has finished
        #[arg(long)]
        wait: bool,
        /// Seconds --wait may take [default: 90]
        #[arg(long, requires = "wait", visible_alias = "timeout")]
        wait_timeout: Option<u64>,
        /// Old form of --merge-strategy
        #[arg(long, value_enum, hide = true)]
        strategy: Option<bitbucket::MergeStrategy>,
        /// Old form of --merge-message
        #[arg(long, hide = true)]
        message: Option<String>,
        /// Accepted for compatibility; no confirmation is asked
        #[arg(long, short, hide = true)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Decline a pull request
    Decline {
        /// Pull request number or URL
        id: String,
        /// Accepted for compatibility; no confirmation is asked
        #[arg(long, short, hide = true)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

/// A whole number of 1 or more (ids, line numbers, limits).
fn positive(s: &str) -> std::result::Result<u64, String> {
    match s.trim().parse::<u64>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err("expected a whole number of 1 or more".into()),
    }
}

fn positive_limit(s: &str) -> std::result::Result<usize, String> {
    positive(s).map(|n| usize::try_from(n).unwrap_or(usize::MAX))
}

#[derive(Subcommand)]
pub(crate) enum PrTaskCmd {
    /// List the tasks on a pull request
    Query {
        /// Pull request number or URL
        #[arg(value_name = "pullRequest")]
        pull_request: String,
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Add a task to a pull request, optionally tied to a comment
    Create {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// Task text; `-` reads stdin
        #[arg(long)]
        text: String,
        /// Comment id to attach the task to
        #[arg(long, value_name = "commentId", value_parser = positive)]
        comment: Option<u64>,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Show one task
    Get {
        /// Pull request number or URL
        #[arg(value_name = "pullRequest")]
        pull_request: String,
        /// Task id
        #[arg(long, value_parser = positive)]
        task: u64,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Change a task's text and/or state
    Update {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// Task id
        #[arg(long, value_parser = positive)]
        task: u64,
        /// New task text; `-` reads stdin
        #[arg(long, required_unless_present_any = ["resolve", "reopen"])]
        text: Option<String>,
        /// Mark the task done
        #[arg(long, conflicts_with = "reopen")]
        resolve: bool,
        /// Mark the task open again
        #[arg(long)]
        reopen: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Delete a task (needs --yes)
    Delete {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// Task id
        #[arg(long, value_parser = positive)]
        task: u64,
        /// Confirm the deletion
        #[arg(long, short)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Mark a task done
    Resolve {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// Task id
        #[arg(long, value_parser = positive)]
        task: u64,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Mark a done task open again
    Reopen {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// Task id
        #[arg(long, value_parser = positive)]
        task: u64,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

/// The pull request and comment a comment command acts on.
#[derive(Args)]
pub(crate) struct CommentRef {
    /// Pull request number or URL
    #[arg(long)]
    pull_request: String,
    /// Comment id
    #[arg(long, value_parser = positive)]
    comment: u64,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Subcommand)]
pub(crate) enum PrCommentCmd {
    /// Add a general, reply, pending, or inline comment
    Create {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// `-` reads stdin
        #[arg(long)]
        text: String,
        /// Comment id this one answers
        #[arg(long, value_name = "commentId", value_parser = positive)]
        reply_to: Option<u64>,
        /// Keep the comment unpublished until the review is finished in Bitbucket
        #[arg(long)]
        pending: bool,
        /// Inline comment on this file path
        #[arg(long)]
        path: Option<String>,
        /// New-side line number
        #[arg(long, requires = "path", value_parser = positive,
              conflicts_with_all = ["start_line", "end_line", "start_from_line", "end_from_line"])]
        line: Option<u64>,
        /// First new-side line of a range (with --end-line)
        #[arg(long, requires_all = ["path", "end_line"], value_parser = positive)]
        start_line: Option<u64>,
        /// Last new-side line of a range (with --start-line)
        #[arg(long, requires_all = ["path", "start_line"], value_parser = positive)]
        end_line: Option<u64>,
        /// Old-side line number
        #[arg(long, requires = "path", value_parser = positive,
              conflicts_with_all = ["start_line", "end_line", "start_from_line", "end_from_line"])]
        from_line: Option<u64>,
        /// First old-side line of a range (with --end-from-line)
        #[arg(long, requires_all = ["path", "end_from_line"], value_parser = positive)]
        start_from_line: Option<u64>,
        /// Last old-side line of a range (with --start-from-line)
        #[arg(long, requires_all = ["path", "start_from_line"], value_parser = positive)]
        end_from_line: Option<u64>,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// List the comments on a pull request
    Query {
        /// Pull request number or URL
        #[arg(value_name = "pullRequest")]
        pull_request: String,
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Show one comment
    Get {
        /// Pull request number or URL
        #[arg(value_name = "pullRequest")]
        pull_request: String,
        /// Comment id
        #[arg(long, value_parser = positive)]
        comment: u64,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Replace a comment's text
    Update {
        #[command(flatten)]
        target: CommentRef,
        /// New comment text; `-` reads stdin
        #[arg(long)]
        text: String,
    },
    /// Delete a comment (needs --yes)
    Delete {
        #[command(flatten)]
        target: CommentRef,
        /// Confirm the deletion
        #[arg(long, short)]
        yes: bool,
    },
    /// Mark a comment thread resolved
    Resolve {
        #[command(flatten)]
        target: CommentRef,
    },
    /// Reopen a resolved comment thread
    Reopen {
        #[command(flatten)]
        target: CommentRef,
    },
}

#[derive(Subcommand)]
pub(crate) enum PipelineCmd {
    /// Recent pipeline runs
    Query {
        /// PENDING, IN_PROGRESS, COMPLETED
        #[arg(long)]
        state: Option<String>,
        /// PASSED, FAILED, STOPPED, ERROR
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// PUSH, MANUAL, SCHEDULE, PULLREQUEST
        #[arg(long)]
        trigger: Option<String>,
        #[arg(long, default_value_t = 15)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One pipeline run with its steps
    Get {
        /// Build number, {uuid}, or result URL
        #[arg(long)]
        pipeline: String,
        /// Include step logs
        #[arg(long)]
        logs: bool,
        /// With --logs, only failed steps
        #[arg(long)]
        failed_steps: bool,
        /// Max log lines per step (0 = all)
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// Limit logs to one step (name or uuid)
        #[arg(long)]
        step: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

pub(crate) fn bb_cmd(cfg: &Config, cmd: BbCmd) -> Result<Out> {
    let (c, default_ws) = cfg.bitbucket()?;
    let dws = default_ws.as_deref();
    let path = |r: &RepoArgs, slug: Option<&str>| {
        bitbucket::repo_path(slug.or(r.repo.as_deref()), r.workspace.as_deref(), dws)
    };
    let data = match cmd {
        BbCmd::Repo {
            cmd:
                RepoCmd::Query {
                    query,
                    limit,
                    repo,
                    project,
                    language,
                    sort,
                    role,
                },
        } => {
            let q = bitbucket::RepoQuery {
                query: query.as_deref(),
                project: project.as_deref(),
                language: language.as_deref(),
                sort: &sort,
                role: role.as_deref(),
                limit,
            };
            let ws = match repo.repo.as_deref() {
                Some(r) => bitbucket::workspace_of(r, repo.workspace.as_deref(), dws)?,
                None => bitbucket::workspace_or(repo.workspace.as_deref(), dws)?.to_string(),
            };
            bitbucket::repo_query(&c, &ws, &q)?
        }
        BbCmd::Repo {
            cmd: RepoCmd::Get { slug, repo },
        } => bitbucket::repo_get(&c, &path(&repo, slug.as_deref())?)?,
        BbCmd::Repo {
            cmd:
                RepoCmd::File {
                    path: file_path,
                    git_ref,
                    repo,
                },
        } => {
            return Ok(
                match bitbucket::repo_file(&c, &path(&repo, None)?, &file_path, git_ref.as_deref())?
                {
                    bitbucket::RepoFile::Dir(v) => Out::Data(v),
                    bitbucket::RepoFile::Raw(s) => Out::Raw(s),
                },
            );
        }
        BbCmd::Branch {
            cmd:
                BranchCmd::Query {
                    query,
                    author,
                    sort,
                    limit,
                    repo,
                },
        } => bitbucket::branch_query(
            &c,
            &path(&repo, None)?,
            &bitbucket::BranchQuery {
                query: query.as_deref(),
                author: author.as_deref(),
                sort: Some(&sort),
                limit,
            },
        )?,
        BbCmd::Repo {
            cmd: RepoCmd::Branches { query, limit, repo },
        } => {
            deprecated("bb repo branches", "bb branch query");
            bitbucket::branch_query(
                &c,
                &path(&repo, None)?,
                &bitbucket::BranchQuery {
                    query: query.as_deref(),
                    limit,
                    ..Default::default()
                },
            )?
        }
        BbCmd::Inbox(a) => lifecycle::inbox(&c, &|r| path(r, None), dws, a)?,
        BbCmd::PullRequests { cmd } => match cmd {
            PrCmd::Query {
                text,
                state,
                author,
                source,
                dest,
                query,
                created_since,
                created_before,
                updated_since,
                updated_before,
                reviewer,
                merged_since,
                merged_before,
                scope,
                role,
                page,
                limit,
                repo,
            } => {
                let text = query.or_else(|| (!text.is_empty()).then(|| text.join(" ")));
                let q = bitbucket::PrQuery {
                    state: state.as_deref(),
                    author: author.as_deref(),
                    source: source.as_deref(),
                    dest: dest.as_deref(),
                    text: text.as_deref(),
                    created_since: created_since.as_deref(),
                    created_before: created_before.as_deref(),
                    updated_since: updated_since.as_deref(),
                    updated_before: updated_before.as_deref(),
                    reviewer: reviewer.as_deref(),
                    page,
                    limit,
                };
                let merged = merged_since.is_some() || merged_before.is_some();
                if scope == Some(lifecycle::QueryScope::Me) {
                    lifecycle::query_mine(&c, &repo, dws, role, &q, merged)?
                } else {
                    if role.is_some() {
                        return Err(ApiError::usage("--role goes with --scope me").into());
                    }
                    let p = path(&repo, None)?;
                    let me = match q.reviewer {
                        Some(r) if r.eq_ignore_ascii_case("me") => {
                            Some(bitbucket::lifecycle::my_uuid(&c)?)
                        }
                        _ => None,
                    };
                    let q = bitbucket::PrQuery {
                        reviewer: me.as_deref().or(q.reviewer),
                        ..q
                    };
                    if merged {
                        let (since, before) = (merged_since.as_deref(), merged_before.as_deref());
                        bitbucket::lifecycle::merged(&c, &p, &q, since, before)?
                    } else {
                        bitbucket::pr_query(&c, &p, &q)?
                    }
                }
            }
            PrCmd::Get {
                ids,
                comments,
                diff,
                statuses,
                full,
                strict,
                batch_concurrency,
                repo,
            } => {
                let p = path(&repo, None)?;
                let o = bitbucket::PrGet {
                    comments: comments || full,
                    diff: diff || full,
                    statuses: statuses || full,
                };
                bb_batch(
                    &p,
                    batch(ids, strict, batch_concurrency, |id| {
                        bitbucket::pr_get(&c, &p, bitbucket::pr_number(id)?, &o)
                    })?,
                )
            }
            PrCmd::Create {
                title,
                source,
                dest,
                description,
                description_file,
                reviewer,
                default_reviewers,
                reviewer_uuid,
                reviewer_account_id,
                close_source_branch,
                draft,
                repo,
            } => {
                let description = body_arg(description, description_file)?;
                let p = path(&repo, None)?;
                let (reviewer_uuids, reviewer_account_ids) = lifecycle::reviewers(
                    &c,
                    &p,
                    &reviewer,
                    reviewer_account_id.as_deref(),
                    reviewer_uuid.as_deref(),
                    default_reviewers,
                )?;
                bitbucket::pr_create(
                    &c,
                    &p,
                    bitbucket::NewPr {
                        title: &title,
                        source: &source,
                        dest: dest.as_deref(),
                        description: description.as_deref(),
                        reviewer_uuids,
                        reviewer_account_ids,
                        close_source_branch,
                        draft,
                    },
                )?
            }
            PrCmd::Comment { cmd } => comment_cmd(&c, &|r| path(r, None), cmd)?,
            PrCmd::DiffLine {
                id,
                text,
                path: file,
                repo,
            } => bitbucket::pr_diff_line(
                &c,
                &path(&repo, None)?,
                bitbucket::pr_number(&id)?,
                &text,
                file.as_deref(),
            )?,
            PrCmd::Diffstat { id, limit, repo } => {
                bitbucket::pr_diffstat(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?, limit)?
            }
            PrCmd::Commits {
                pull_request,
                limit,
                repo,
            } => bitbucket::pr_commits(
                &c,
                &path(&repo, None)?,
                bitbucket::pr_number(&pull_request)?,
                limit,
            )?,
            PrCmd::Patch { id, repo } => {
                return Ok(Out::Raw(bitbucket::pr_patch(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::pr_number(&id)?,
                )?));
            }
            PrCmd::Diff { id, repo } => {
                return Ok(Out::Raw(bitbucket::pr_diff(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::pr_number(&id)?,
                )?));
            }
            PrCmd::Activity {
                ids,
                event_type,
                limit,
                strict,
                batch_concurrency,
                repo,
            } => {
                let p = path(&repo, None)?;
                bb_batch(
                    &p,
                    batch(ids, strict, batch_concurrency, |id| {
                        bitbucket::pr_activity(
                            &c,
                            &p,
                            bitbucket::pr_number(id)?,
                            event_type.as_deref(),
                            limit,
                        )
                    })?,
                )
            }
            PrCmd::Task { cmd } => task_cmd(&c, &|r| path(r, None), cmd)?,
            PrCmd::Tasks { id, repo } => {
                deprecated("bb pull-requests tasks", "bb pull-requests task query");
                bitbucket::pr_tasks(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?, 500)?
            }
            PrCmd::Approve { id, repo } => {
                bitbucket::pr_approve(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?)?
            }
            PrCmd::Unapprove { id, repo } => {
                bitbucket::pr_unapprove(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?)?
            }
            PrCmd::RequestChanges { id, repo } => {
                bitbucket::pr_request_changes(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?)?
            }
            PrCmd::Merge {
                pull_request,
                id,
                merge_strategy,
                merge_message,
                close_source_branch,
                keep_source_branch,
                wait,
                wait_timeout,
                strategy,
                message,
                yes: _,
                repo,
            } => {
                if id.is_some() {
                    deprecated(
                        "bb pull-requests merge <id>",
                        "bb pull-requests merge --pull-request <id>",
                    );
                }
                if strategy.is_some() {
                    deprecated("--strategy", "--merge-strategy");
                }
                if message.is_some() {
                    deprecated("--message", "--merge-message");
                }
                let Some(pr) = pull_request.or(id) else {
                    return Err(ApiError::usage("pass --pull-request <id>").into());
                };
                let pr_id = bitbucket::pr_number(&pr)?;
                let message = merge_message.or(message);
                let o = bitbucket::PrMerge {
                    strategy: merge_strategy.or(strategy),
                    close_source_branch,
                    keep_source_branch,
                    message: message.as_deref(),
                };
                let defaults = bitbucket::lifecycle::Wait::default();
                let wait = wait.then(|| bitbucket::lifecycle::Wait {
                    timeout: wait_timeout.map_or(defaults.timeout, std::time::Duration::from_secs),
                    ..defaults
                });
                bitbucket::lifecycle::merge(&c, &path(&repo, None)?, pr_id, &o, wait)?
            }
            PrCmd::Update(a) => lifecycle::update(&c, &|r| path(r, None), a)?,
            PrCmd::RemoveRequestChanges(a) => {
                lifecycle::remove_request_changes(&c, &|r| path(r, None), a)?
            }
            PrCmd::Mergeability(a) => lifecycle::mergeability(&c, &|r| path(r, None), a)?,
            PrCmd::MergeStatus(a) => lifecycle::merge_status(&c, &|r| path(r, None), a)?,
            PrCmd::Conflicts(a) => lifecycle::conflicts(&c, &|r| path(r, None), a)?,
            PrCmd::ForCommit(a) => lifecycle::for_commit(&c, &|r| path(r, None), a)?,
            PrCmd::DefaultReviewer { cmd } => {
                lifecycle::default_reviewer(&c, &|r| path(r, None), cmd)?
            }
            PrCmd::EffectiveDefaultReviewer { cmd } => {
                lifecycle::effective_default_reviewer(&c, &|r| path(r, None), cmd)?
            }
            PrCmd::Decline { id, yes: _, repo } => {
                let pr_id = bitbucket::pr_number(&id)?;
                bitbucket::pr_decline(&c, &path(&repo, None)?, pr_id)?
            }
        },
        BbCmd::Pipeline {
            cmd:
                PipelineCmd::Query {
                    state,
                    status,
                    branch,
                    trigger,
                    limit,
                    repo,
                },
        } => {
            let q = bitbucket::PipelineQuery {
                state: state.as_deref(),
                status: status.as_deref(),
                branch: branch.as_deref(),
                trigger: trigger.as_deref(),
                limit,
            };
            bitbucket::pipeline_query(&c, &path(&repo, None)?, &q)?
        }
        BbCmd::Pipeline {
            cmd:
                PipelineCmd::Get {
                    pipeline,
                    logs,
                    failed_steps,
                    lines,
                    step,
                    repo,
                },
        } => {
            let o = bitbucket::PipelineGet {
                logs,
                failed_steps,
                lines,
                step: step.as_deref(),
            };
            bitbucket::pipeline_get(&c, &path(&repo, None)?, &pipeline, &o)?
        }
        BbCmd::Search {
            cmd:
                BbSearchCmd::Code {
                    query,
                    workspace,
                    repo,
                    limit,
                },
        } => {
            let ws = bitbucket::workspace_or(workspace.as_deref(), dws)?;
            bitbucket::search_code(&c, ws, &query.join(" "), repo.as_deref(), limit)?
        }
    };
    Ok(Out::Data(data))
}

type RepoPath<'a> = &'a dyn Fn(&RepoArgs) -> Result<String>;

/// `bb pull-requests comment ...`
fn comment_cmd(c: &Client, path: RepoPath, cmd: PrCommentCmd) -> Result<Value> {
    let pr = |s: &str| bitbucket::pr_number(s);
    match cmd {
        PrCommentCmd::Create {
            pull_request,
            text,
            reply_to,
            pending,
            path: file,
            line,
            start_line,
            end_line,
            from_line,
            start_from_line,
            end_from_line,
            repo,
        } => {
            let text = text_arg(text)?;
            let n = bitbucket::NewComment {
                text: &text,
                reply_to,
                pending,
                path: file.as_deref(),
                anchor: bitbucket::Anchor {
                    line,
                    from_line,
                    start_line,
                    end_line,
                    start_from_line,
                    end_from_line,
                },
            };
            // Check the anchor before resolving the repository or sending.
            bitbucket::comment_body(&n)?;
            bitbucket::pr_comment(c, &path(&repo)?, pr(&pull_request)?, n)
        }
        PrCommentCmd::Query {
            pull_request,
            limit,
            repo,
        } => bitbucket::pr_comment_query(c, &path(&repo)?, pr(&pull_request)?, limit),
        PrCommentCmd::Get {
            pull_request,
            comment,
            repo,
        } => bitbucket::pr_comment_get(c, &path(&repo)?, pr(&pull_request)?, comment),
        PrCommentCmd::Update { target: t, text } => {
            let text = text_arg(text)?;
            bitbucket::pr_comment_update(c, &path(&t.repo)?, pr(&t.pull_request)?, t.comment, &text)
        }
        PrCommentCmd::Delete { target: t, yes } => {
            require_yes(yes, "deleting the comment")?;
            bitbucket::pr_comment_delete(c, &path(&t.repo)?, pr(&t.pull_request)?, t.comment)
        }
        PrCommentCmd::Resolve { target: t } => {
            bitbucket::pr_comment_resolve(c, &path(&t.repo)?, pr(&t.pull_request)?, t.comment)
        }
        PrCommentCmd::Reopen { target: t } => {
            bitbucket::pr_comment_reopen(c, &path(&t.repo)?, pr(&t.pull_request)?, t.comment)
        }
    }
}

/// `bb pull-requests task ...`
fn task_cmd(c: &Client, path: RepoPath, cmd: PrTaskCmd) -> Result<Value> {
    let pr = |s: &str| bitbucket::pr_number(s);
    let change = |c: &Client, repo: &RepoArgs, id: &str, task, t: bitbucket::TaskChange| {
        bitbucket::pr_task_update(c, &path(repo)?, pr(id)?, task, &t)
    };
    match cmd {
        PrTaskCmd::Query {
            pull_request,
            limit,
            repo,
        } => bitbucket::pr_tasks(c, &path(&repo)?, pr(&pull_request)?, limit),
        PrTaskCmd::Create {
            pull_request,
            text,
            comment,
            repo,
        } => {
            let text = text_arg(text)?;
            bitbucket::pr_task_create(c, &path(&repo)?, pr(&pull_request)?, &text, comment)
        }
        PrTaskCmd::Get {
            pull_request,
            task,
            repo,
        } => bitbucket::pr_task_get(c, &path(&repo)?, pr(&pull_request)?, task),
        PrTaskCmd::Update {
            pull_request,
            task,
            text,
            resolve,
            reopen,
            repo,
        } => {
            let text = text.map(text_arg).transpose()?;
            let resolved = match (resolve, reopen) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            let t = bitbucket::TaskChange {
                text: text.as_deref(),
                resolved,
            };
            change(c, &repo, &pull_request, task, t)
        }
        PrTaskCmd::Delete {
            pull_request,
            task,
            yes,
            repo,
        } => {
            require_yes(yes, "deleting the task")?;
            bitbucket::pr_task_delete(c, &path(&repo)?, pr(&pull_request)?, task)
        }
        PrTaskCmd::Resolve {
            pull_request,
            task,
            repo,
        } => change(
            c,
            &repo,
            &pull_request,
            task,
            bitbucket::TaskChange {
                text: None,
                resolved: Some(true),
            },
        ),
        PrTaskCmd::Reopen {
            pull_request,
            task,
            repo,
        } => change(
            c,
            &repo,
            &pull_request,
            task,
            bitbucket::TaskChange {
                text: None,
                resolved: Some(false),
            },
        ),
    }
}

/// twg's Bitbucket batch shape: `{workspace, repository, items, summary:
/// {requested, succeeded, failed}, partial}`, with no `data` envelope.
/// A single result passes through unchanged.
fn bb_batch(repo_path: &str, v: Value) -> Value {
    let (Some(items), Some(summary)) = (v.get("items"), v.get("summary")) else {
        return v;
    };
    let mut parts = repo_path
        .trim_start_matches("/repositories/")
        .splitn(2, '/');
    let failed = summary["failed"].as_u64().unwrap_or(0);
    let mut out = json!({
        "workspace": parts.next(),
        "repository": parts.next(),
        "items": items,
        "summary": {
            "requested": summary["requested"],
            "succeeded": summary["succeeded"],
            "failed": failed,
        },
    });
    if failed > 0 {
        out["partial"] = true.into();
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    #[test]
    fn parses_pull_request_lifecycle() {
        for ok in [
            "atlx bb pull-requests update --pull-request 4 --title x --add-reviewer ann ben --remove-reviewer-uuid {a}",
            "atlx bb pull-requests update --pull-request 4 --clear-description --ready --keep-source-branch",
            "atlx bb pull-requests create --title x --source b --reviewer 'Cy Lee' --default-reviewers",
            "atlx bb pull-requests merge --pull-request 4 --wait --timeout 5",
            "atlx bb pull-requests merge-status --pull-request 4 --task t-1",
            "atlx bb pull-requests mergeability 4 --exclude-git-mergeability-check",
            "atlx bb pull-requests remove-request-changes 4",
            "atlx bb pull-requests conflicts 4",
            "atlx bb pull-requests for-commit abc123 -n 5",
            "atlx bb pull-requests query --scope me --role reviewer",
            "atlx bb pull-requests query --merged-since 7d --reviewer me",
            "atlx bb pull-requests query --page 2",
            "atlx bb pull-requests default-reviewer add {a} --yes",
            "atlx bb pull-requests effective-default-reviewer query",
            "atlx bb inbox --role all --repo-limit 5 --status OPEN",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx bb pull-requests update --pull-request 4 --draft --ready",
            "atlx bb pull-requests update --pull-request 4 --clear-reviewers --reviewer ann",
            "atlx bb pull-requests merge --pull-request 4 --wait-timeout 5",
            "atlx bb pull-requests query --page 0",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_bitbucket_repo_flags() {
        for bad in [
            "atlx bb repo get --repo api --repo web",
            "atlx bb prs query --repo api --repo web",
            "atlx bb pipeline query --repo api --repo web",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn bb_review_loop_commands_parse() {
        for ok in [
            "atlx bb pull-requests comment create --pull-request 42 --text hi --reply-to 7 --pending -r acme/api",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path src/a.rs -r api",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path src/a.rs --line 3 --from-line 2 -r api",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --start-line 1 --end-line 4 --start-from-line 1 --end-from-line 3 -r api",
            "atlx bb pull-requests comment query 42 -n 10 -r api",
            "atlx bb pull-requests comment get 42 --comment 9 -r api",
            "atlx bb pull-requests comment update --pull-request 42 --comment 9 --text new -r api",
            "atlx bb pull-requests comment delete --pull-request 42 --comment 9 --yes -r api",
            "atlx bb pull-requests comment delete --pull-request 42 --comment 9 -r api",
            "atlx bb pull-requests comment resolve --pull-request 42 --comment 9 -r api",
            "atlx bb pull-requests comment reopen --pull-request 42 --comment 9 -r api",
            "atlx bb pull-requests task create --pull-request 42 --text t --comment 9 -r api",
            "atlx bb pull-requests task get 42 --task 6 -r api",
            "atlx bb pull-requests task update --pull-request 42 --task 6 --text t --resolve -r api",
            "atlx bb pull-requests task update --pull-request 42 --task 6 --reopen -r api",
            "atlx bb pull-requests task delete --pull-request 42 --task 6 -y -r api",
            "atlx bb pull-requests task resolve --pull-request 42 --task 6 -r api",
            "atlx bb pull-requests task reopen --pull-request 42 --task 6 -r api",
            "atlx bb pull-requests diffstat 42 -n 5 -r api",
            "atlx bb pull-requests commits 42 --limit 5 -r api",
            "atlx bb pull-requests patch 42 -r api",
            "atlx bb pull-requests diff-line 42 --text validate --path src/a.rs -r api",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            // Inline flags need --path.
            "atlx bb pull-requests comment create --pull-request 42 --text hi --line 3",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --start-from-line 3 --end-from-line 4",
            // A single line and a range don't mix.
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --line 3 --start-line 1 --end-line 4",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --from-line 3 --start-from-line 1 --end-from-line 4",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --line 3 --end-from-line 4",
            // Ranges need both ends.
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --start-line 3",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --end-from-line 3",
            // Ids and line numbers are positive.
            "atlx bb pull-requests comment create --pull-request 42 --text hi --path a --line 0",
            "atlx bb pull-requests comment create --pull-request 42 --text hi --reply-to abc",
            "atlx bb pull-requests comment get 42 --comment x",
            "atlx bb pull-requests comment query 42 -n 0",
            "atlx bb pull-requests comment get 42",
            "atlx bb pull-requests comment update --pull-request 42 --comment 9",
            "atlx bb pull-requests task create --pull-request 42",
            "atlx bb pull-requests task update --pull-request 42 --task 6",
            "atlx bb pull-requests task update --pull-request 42 --task 6 --resolve --reopen",
            "atlx bb pull-requests task get 42 --task 0",
            "atlx bb pull-requests diffstat 42 -n 0",
            "atlx bb pull-requests diff-line 42",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn bb_twg_names_and_old_forms() {
        for ok in [
            "atlx bb pull-requests merge --pull-request 1 -r acme/api --merge-strategy squash --merge-message m --keep-source-branch",
            "atlx bb pull-requests merge 1 --repo acme/api --strategy merge_commit --message m --yes",
            "atlx bb pull-requests merge --pull-request 1 --repo api",
            "atlx bb pull-requests decline 1 --repo api",
            "atlx bb pull-requests decline 1 --repo api --yes",
            "atlx bb pull-requests activity 1 2 3 --repo api --strict --batch-concurrency 2 -n 5",
            "atlx bb pull-requests task query 7 -w acme -r api -n 10",
            "atlx bb pull-requests tasks 7 --repo api",
            "atlx bb branch query -r acme/api -q feat --author alice --sort name -n 5",
            "atlx bb repo branches --repo api",
            "atlx bb repo query -q api -n 5 --repo acme/api",
            "atlx bb repo get -w acme -r api",
            "atlx bb pull-requests get 1 2 --repo api --batch-concurrency 20",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx bb pull-requests merge --pull-request 1 2 --repo api",
            "atlx bb pull-requests merge --pull-request 1 --repo api --merge-strategy invalid",
            "atlx bb pull-requests merge 1 --merge-strategy squash --strategy squash",
            "atlx bb pull-requests merge 1 --close-source-branch --keep-source-branch",
            "atlx bb pull-requests get 1 2 --batch-concurrency 0",
            "atlx bb pull-requests get 1 2 --batch-concurrency 21",
            "atlx bb pull-requests activity --repo api",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }
}

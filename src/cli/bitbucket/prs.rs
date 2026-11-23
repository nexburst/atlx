//! `bb pr ...`: pull requests, their comments, tasks and reviewers.

use super::{BatchArgs, Ctx, ListArgs, RepoArg, complete, get_many, listed, positive_id};
use crate::bitbucket::{self, lifecycle, repos};
use crate::cli::util::{self, body_arg};
use crate::client::ApiError;
use crate::output::notes;
use anyhow::Result;
use bitbucket::lifecycle::{Role, User};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// `--body` / `--body-file` (Markdown; Bitbucket stores it as written).
#[derive(Args, Clone, Default)]
pub(crate) struct BodyArgs {
    /// Text, as Markdown; `-` reads stdin
    #[arg(long, conflicts_with = "body_file")]
    body: Option<String>,
    /// Read the text from a file
    #[arg(long, value_name = "FILE")]
    body_file: Option<String>,
}

impl BodyArgs {
    fn get(self) -> Result<Option<String>> {
        body_arg(self.body, self.body_file)
    }

    fn required(self) -> Result<String> {
        self.get()?
            .ok_or_else(|| usage("pass --body or --body-file"))
    }
}

/// The pull request a sub-resource command acts on.
#[derive(Args, Clone)]
pub(crate) struct PrRef {
    /// Pull request number or URL
    #[arg(long, value_name = "PR")]
    pr: String,
    #[command(flatten)]
    repo: RepoArg,
}

impl PrRef {
    fn resolve(&self, ctx: &Ctx) -> Result<(String, u64)> {
        Ok((
            ctx.pr_repo(&self.repo, &self.pr)?,
            bitbucket::pr_number(&self.pr)?,
        ))
    }
}

/// A pull request given as the command's own argument.
#[derive(Args, Clone)]
pub(crate) struct PrArg {
    /// Pull request number or URL
    #[arg(value_name = "PR")]
    pr: String,
    #[command(flatten)]
    repo: RepoArg,
}

impl PrArg {
    fn resolve(&self, ctx: &Ctx) -> Result<(String, u64)> {
        Ok((
            ctx.pr_repo(&self.repo, &self.pr)?,
            bitbucket::pr_number(&self.pr)?,
        ))
    }
}

/// `pr list --mine` and `pr search --mine`.
#[derive(Args, Clone)]
pub(crate) struct MineArgs {
    /// Your pull requests, across the workspace's recently updated
    /// repositories (or only --repo)
    #[arg(long)]
    mine: bool,
    /// With --mine: author, reviewer or participant [default: author or reviewer]
    #[arg(long, value_enum, requires = "mine")]
    role: Option<Role>,
    /// With --mine: only what waits on you (reviews you haven't approved,
    /// your pull requests with changes requested)
    #[arg(long, requires = "mine")]
    waiting: bool,
    /// With --mine: the workspace to scan [default: the repository's, else the saved one]
    #[arg(long, value_name = "SLUG", requires = "mine")]
    workspace: Option<String>,
    /// With --mine: how many recently updated repositories to scan (each costs a request)
    #[arg(long, value_name = "N", default_value_t = 25, value_parser = super::positive)]
    repo_limit: usize,
    /// With --mine: how many repositories to read at once (1 to 20)
    #[arg(long, value_name = "N", default_value_t = util::DEFAULT_CONCURRENCY, value_parser = util::concurrency_arg)]
    concurrency: usize,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum PrCmd {
    /// Pull requests of a repository, most recently updated first
    List(ListPrArgs),
    /// Pull requests whose title or description contains the text
    Search {
        /// Text to look for
        #[arg(required = true)]
        text: Vec<String>,
        /// Where to look
        #[arg(long = "in", value_enum, default_value = "both")]
        field: repos::Field,
        /// OPEN (default), MERGED, DECLINED, SUPERSEDED, or ALL
        #[arg(long)]
        state: Option<String>,
        #[command(flatten)]
        mine: MineArgs,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more pull requests, with description and reviewers (`-`
    /// reads numbers from stdin)
    Get {
        /// Numbers or URLs
        #[arg(required = true, value_name = "PR")]
        prs: Vec<String>,
        /// Start the description at this character
        #[arg(long, value_name = "N")]
        body_offset: Option<usize>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Open a pull request
    Create {
        #[arg(long)]
        title: String,
        /// Source branch
        #[arg(long)]
        source: String,
        /// Destination branch [default: the main branch]
        #[arg(long)]
        dest: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
        /// Reviewers: names, nicknames, account IDs, {uuid}s or me; repeatable
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        reviewer: Vec<String>,
        /// Add the repository's default reviewers
        #[arg(long)]
        default_reviewers: bool,
        /// Delete the source branch when merged
        #[arg(long)]
        close_source_branch: bool,
        #[arg(long)]
        draft: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Change a pull request's title, description, destination or flags
    Update {
        #[command(flatten)]
        pr: PrArg,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
        /// Empty the description
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        clear_body: bool,
        /// New destination branch
        #[arg(long)]
        dest: Option<String>,
        /// Delete the source branch when merged
        #[arg(long, conflicts_with = "keep_source_branch")]
        close_source_branch: bool,
        /// Keep the source branch when merged
        #[arg(long)]
        keep_source_branch: bool,
        /// Turn into a draft
        #[arg(long, conflicts_with = "ready")]
        draft: bool,
        /// Mark ready for review
        #[arg(long)]
        ready: bool,
    },
    /// Merge a pull request, waiting until Bitbucket has finished
    Merge {
        #[command(flatten)]
        pr: PrArg,
        /// [default: the repository's]
        #[arg(long, value_enum)]
        strategy: Option<bitbucket::MergeStrategy>,
        /// Merge commit message
        #[arg(long)]
        message: Option<String>,
        /// Delete the source branch
        #[arg(long, conflicts_with = "keep_source_branch")]
        close_source_branch: bool,
        /// Keep the source branch even if the pull request says to close it
        #[arg(long)]
        keep_source_branch: bool,
        /// Merge only if the source branch is still at this commit (the
        /// `head` that `pr get` printed); exit 6 otherwise
        #[arg(long, value_name = "SHA")]
        if_head: Option<String>,
        /// Return as soon as Bitbucket accepts the merge
        #[arg(long)]
        no_wait: bool,
        /// Seconds to wait for the merge [default: 90]
        #[arg(long, value_name = "SECONDS", conflicts_with = "no_wait")]
        timeout: Option<u64>,
        /// Confirm the merge
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Decline a pull request
    Decline {
        #[command(flatten)]
        pr: PrArg,
        /// Confirm
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Approve a pull request
    Approve(PrArg),
    /// Withdraw your approval
    Unapprove(PrArg),
    /// Request changes on a pull request
    RequestChanges {
        #[command(flatten)]
        pr: PrArg,
        /// Withdraw your request instead
        #[arg(long)]
        withdraw: bool,
    },
    /// The diff (the first 40 000 characters; --diff-offset continues)
    Diff {
        #[command(flatten)]
        pr: PrArg,
        /// Only this file
        #[arg(long)]
        path: Option<String>,
        /// The mbox patch (for `git am`) instead
        #[arg(long, conflicts_with_all = ["path", "grep"])]
        patch: bool,
        /// Only diff lines containing this text, with the line numbers an
        /// inline comment takes
        #[arg(long, value_name = "TEXT")]
        grep: Option<String>,
        /// Start at this character
        #[arg(long, value_name = "N")]
        diff_offset: Option<usize>,
    },
    /// Files changed, with lines added and removed
    Diffstat {
        #[command(flatten)]
        pr: PrArg,
        /// Only files with merge conflicts
        #[arg(long)]
        conflicts: bool,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Commits in a pull request
    Commits {
        #[command(flatten)]
        pr: PrArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Events: comments, approvals, updates
    Activity {
        #[command(flatten)]
        pr: PrArg,
        /// Only this kind: comment, approval, update, changes_requested
        #[arg(long = "type", value_name = "TYPE")]
        kind: Option<String>,
        #[command(flatten)]
        list: ListArgs,
    },
    /// What gates the merge: build statuses, approvals, tasks, conflicts
    Checks {
        #[command(flatten)]
        pr: PrArg,
        /// Skip the merge conflict check
        #[arg(long)]
        no_conflicts: bool,
    },
    /// Comments
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },
    /// Tasks
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Reviewers of one pull request
    Reviewer {
        #[command(subcommand)]
        cmd: ReviewerCmd,
    },
}

#[derive(Args)]
pub(crate) struct ListPrArgs {
    /// OPEN (default), MERGED, DECLINED, SUPERSEDED, or ALL; comma-separated
    #[arg(long)]
    state: Option<String>,
    /// Nickname, display name, account ID or {uuid}
    #[arg(long)]
    author: Option<String>,
    /// Nickname, account ID, {uuid} or me
    #[arg(long)]
    reviewer: Option<String>,
    /// Source branch
    #[arg(long)]
    source: Option<String>,
    /// Destination branch
    #[arg(long)]
    dest: Option<String>,
    /// BBQL filter, e.g. 'comment_count > 3'
    #[arg(long, short = 'q', value_name = "BBQL")]
    query: Option<String>,
    /// Created on or after (YYYY-MM-DD or 7d)
    #[arg(long, value_name = "DATE")]
    created_since: Option<String>,
    #[arg(long, value_name = "DATE")]
    created_before: Option<String>,
    #[arg(long, value_name = "DATE")]
    updated_since: Option<String>,
    #[arg(long, value_name = "DATE")]
    updated_before: Option<String>,
    /// Merged on or after; reads each candidate's activity
    #[arg(long, value_name = "DATE")]
    merged_since: Option<String>,
    /// Merged before; reads each candidate's activity
    #[arg(long, value_name = "DATE")]
    merged_before: Option<String>,
    /// Pull requests that contain this commit
    #[arg(long, value_name = "HASH", conflicts_with_all = ["state", "author", "reviewer", "source", "dest", "query", "created_since", "created_before", "updated_since", "updated_before", "merged_since", "merged_before", "mine"])]
    commit: Option<String>,
    #[command(flatten)]
    mine: MineArgs,
    #[command(flatten)]
    repo: RepoArg,
    #[command(flatten)]
    list: ListArgs,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommentCmd {
    /// Comments, oldest first, without deleted ones
    List {
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more comments
    Get {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// A general, reply, pending or inline comment
    Create {
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        body: BodyArgs,
        /// The comment this one answers
        #[arg(long, value_name = "ID", value_parser = positive_id)]
        reply_to: Option<u64>,
        /// Keep it unpublished until the review is finished in Bitbucket
        #[arg(long)]
        pending: bool,
        /// Inline comment on this file
        #[arg(long)]
        path: Option<String>,
        /// New-side line (see `pr diff --grep`)
        #[arg(long, requires = "path", value_parser = positive_id,
              conflicts_with_all = ["start_line", "end_line", "start_from_line", "end_from_line"])]
        line: Option<u64>,
        /// First new-side line of a range (with --end-line)
        #[arg(long, requires_all = ["path", "end_line"], value_parser = positive_id)]
        start_line: Option<u64>,
        /// Last new-side line of a range (with --start-line)
        #[arg(long, requires_all = ["path", "start_line"], value_parser = positive_id)]
        end_line: Option<u64>,
        /// Old-side line
        #[arg(long, requires = "path", value_parser = positive_id,
              conflicts_with_all = ["start_line", "end_line", "start_from_line", "end_from_line"])]
        from_line: Option<u64>,
        /// First old-side line of a range (with --end-from-line)
        #[arg(long, requires_all = ["path", "end_from_line"], value_parser = positive_id)]
        start_from_line: Option<u64>,
        /// Last old-side line of a range (with --start-from-line)
        #[arg(long, requires_all = ["path", "start_from_line"], value_parser = positive_id)]
        end_from_line: Option<u64>,
    },
    /// Replace a comment's text
    Update {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Delete a comment
    Delete {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
        /// Confirm the deletion
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Mark a comment thread resolved
    Resolve {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
    },
    /// Reopen a resolved comment thread
    Reopen {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
    },
}

#[derive(Subcommand)]
pub(crate) enum TaskCmd {
    /// Tasks
    List {
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more tasks
    Get {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Add a task, optionally tied to a comment
    Create {
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        body: BodyArgs,
        /// Comment to attach it to
        #[arg(long, value_name = "ID", value_parser = positive_id)]
        comment: Option<u64>,
    },
    /// Replace a task's text
    Update {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Delete a task
    Delete {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
        /// Confirm the deletion
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Mark a task done
    Resolve {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
    },
    /// Mark a done task open again
    Reopen {
        #[arg(value_name = "ID", value_parser = positive_id)]
        id: u64,
        #[command(flatten)]
        pr: PrRef,
    },
}

#[derive(Subcommand)]
pub(crate) enum ReviewerCmd {
    /// Reviewers with their review state
    List {
        #[command(flatten)]
        pr: PrRef,
    },
    /// Add reviewers: names, nicknames, account IDs, {uuid}s or me
    Add {
        #[arg(required = true, value_name = "USER")]
        users: Vec<String>,
        #[command(flatten)]
        pr: PrRef,
    },
    /// Remove reviewers
    Remove {
        #[arg(required = true, value_name = "USER")]
        users: Vec<String>,
        #[command(flatten)]
        pr: PrRef,
    },
}

fn any_users(names: &[String]) -> Vec<User<'_>> {
    names.iter().map(|s| User::Any(s.trim())).collect()
}

fn id_arg(s: &str) -> Result<u64> {
    positive_id(s).map_err(|_| usage(format!("'{s}' is not an ID")))
}

/// A `--mine` scan, with what it could not see as warnings.
fn mine_scan(
    ctx: &Ctx,
    m: &MineArgs,
    repo: &RepoArg,
    state: Option<&str>,
    limit: usize,
    run: impl FnOnce(&lifecycle::Mine) -> Result<lifecycle::Found>,
) -> Result<Value> {
    let ws = ctx.workspace(m.workspace.as_deref(), repo.repo.as_deref())?;
    // Only an explicit --repo narrows the scan to one repository.
    let one = repo
        .repo
        .as_deref()
        .map(|r| ctx.repo_of(Some(r)))
        .transpose()?;
    let o = lifecycle::Mine {
        workspace: &ws,
        repo: one.as_deref(),
        role: m.role,
        waiting: m.waiting,
        state,
        limit,
        repo_limit: m.repo_limit,
        concurrency: m.concurrency,
    };
    let found = run(&o)?;
    for s in &found.skipped {
        notes::warn_value(json!({
            "code": "repo_skipped",
            "message": format!("could not read {}", s["repository"].as_str().unwrap_or("a repository")),
            "repository": s["repository"],
        }));
    }
    if found.more {
        notes::warn(
            "incomplete",
            format!(
                "scanned {} repositories; more may match: raise --limit or --repo-limit",
                found.repos_scanned
            ),
        );
    }
    Ok(complete(found.items))
}

pub(super) fn pr(ctx: &Ctx, cmd: PrCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        PrCmd::List(a) => list(ctx, a),
        PrCmd::Search {
            text,
            field,
            state,
            mine,
            repo,
            list,
        } => {
            let text = text.join(" ");
            if mine.mine {
                let limit = list.paging().limit;
                return mine_scan(ctx, &mine, &repo, state.as_deref(), limit, |o| {
                    repos::search_my_prs(c, o, &text, field)
                });
            }
            Ok(listed(repos::search_prs(
                c,
                &ctx.repo(&repo)?,
                &text,
                field,
                state.as_deref(),
                &list.paging(),
            )?))
        }
        PrCmd::Get {
            prs, repo, batch, ..
        } => get_many(prs, &batch, |id| {
            bitbucket::pr_get(c, &ctx.pr_repo(&repo, id)?, bitbucket::pr_number(id)?)
        }),
        PrCmd::Create {
            title,
            source,
            dest,
            body,
            reviewer,
            default_reviewers,
            close_source_branch,
            draft,
            repo,
        } => {
            let description = body.get()?;
            let p = ctx.repo(&repo)?;
            let (reviewer_uuids, reviewer_account_ids) =
                lifecycle::new_reviewers(c, &p, &any_users(&reviewer), default_reviewers)?;
            bitbucket::pr_create(
                c,
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
            )
        }
        PrCmd::Update {
            pr,
            title,
            body,
            clear_body,
            dest,
            close_source_branch,
            keep_source_branch,
            draft,
            ready,
        } => {
            let (p, id) = pr.resolve(ctx)?;
            let mut description = body.get()?;
            if clear_body {
                description = Some(String::new());
            }
            let flag = |on: bool, off: bool| (on || off).then_some(on);
            lifecycle::pr_update(
                c,
                &p,
                id,
                &lifecycle::PrUpdate {
                    title: title.as_deref(),
                    description: description.as_deref(),
                    dest: dest.as_deref(),
                    close_source_branch: flag(close_source_branch, keep_source_branch),
                    draft: flag(draft, ready),
                    ..Default::default()
                },
            )
        }
        PrCmd::Merge {
            pr,
            strategy,
            message,
            close_source_branch,
            keep_source_branch,
            if_head,
            no_wait,
            timeout,
            ..
        } => {
            let (p, id) = pr.resolve(ctx)?;
            let o = bitbucket::PrMerge {
                strategy,
                close_source_branch,
                keep_source_branch,
                message: message.as_deref(),
                if_head: if_head.as_deref(),
            };
            let defaults = lifecycle::Wait::default();
            let wait = (!no_wait).then(|| lifecycle::Wait {
                timeout: timeout.map_or(defaults.timeout, std::time::Duration::from_secs),
                ..defaults
            });
            lifecycle::merge(c, &p, id, &o, wait)
        }
        PrCmd::Decline { pr, .. } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_decline(c, &p, id)
        }
        PrCmd::Approve(pr) => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_approve(c, &p, id)
        }
        PrCmd::Unapprove(pr) => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_unapprove(c, &p, id)
        }
        PrCmd::RequestChanges { pr, withdraw } => {
            let (p, id) = pr.resolve(ctx)?;
            if withdraw {
                lifecycle::remove_request_changes(c, &p, id)
            } else {
                bitbucket::pr_request_changes(c, &p, id)
            }
        }
        PrCmd::Diff {
            pr,
            path,
            patch,
            grep,
            ..
        } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_diff_view(
                c,
                &p,
                id,
                &bitbucket::DiffOpts {
                    path: path.as_deref(),
                    patch,
                    grep: grep.as_deref(),
                },
            )
        }
        PrCmd::Diffstat {
            pr,
            conflicts,
            list,
        } => {
            let (p, id) = pr.resolve(ctx)?;
            Ok(listed(bitbucket::pr_diffstat(
                c,
                &p,
                id,
                conflicts,
                &list.paging(),
            )?))
        }
        PrCmd::Commits { pr, list } => {
            let (p, id) = pr.resolve(ctx)?;
            Ok(listed(bitbucket::pr_commits(c, &p, id, &list.paging())?))
        }
        PrCmd::Activity { pr, kind, list } => {
            let (p, id) = pr.resolve(ctx)?;
            Ok(listed(bitbucket::pr_activity(
                c,
                &p,
                id,
                kind.as_deref(),
                &list.paging(),
            )?))
        }
        PrCmd::Checks { pr, no_conflicts } => {
            let (p, id) = pr.resolve(ctx)?;
            lifecycle::checks(
                c,
                &p,
                id,
                lifecycle::Exclude {
                    conflicts: no_conflicts,
                },
            )
        }
        PrCmd::Comment { cmd } => comment(ctx, cmd),
        PrCmd::Task { cmd } => task(ctx, cmd),
        PrCmd::Reviewer { cmd } => reviewer(ctx, cmd),
    }
}

fn list(ctx: &Ctx, a: ListPrArgs) -> Result<Value> {
    let c = ctx.c;
    let paging = a.list.paging();
    if a.mine.mine {
        let filtered = [
            &a.author,
            &a.reviewer,
            &a.source,
            &a.dest,
            &a.query,
            &a.created_since,
            &a.created_before,
            &a.updated_since,
            &a.updated_before,
            &a.merged_since,
            &a.merged_before,
        ]
        .iter()
        .any(|o| o.is_some());
        if filtered || a.list.cursor.is_some() {
            return Err(usage(
                "--mine takes only --state, --role, --waiting, --workspace, --repo and --limit",
            ));
        }
        return mine_scan(
            ctx,
            &a.mine,
            &a.repo,
            a.state.as_deref(),
            paging.limit,
            |o| lifecycle::mine(c, o),
        );
    }
    if a.mine.role.is_some() || a.mine.waiting {
        return Err(usage("--role and --waiting go with --mine"));
    }
    let p = ctx.repo(&a.repo)?;
    if let Some(commit) = &a.commit {
        return Ok(listed(lifecycle::for_commit(c, &p, commit, &paging)?));
    }
    let me;
    let reviewer = match a.reviewer.as_deref() {
        Some(r) if r.eq_ignore_ascii_case("me") => {
            me = lifecycle::my_uuid(c)?;
            Some(me.as_str())
        }
        other => other,
    };
    let q = bitbucket::PrQuery {
        state: a.state.as_deref(),
        author: a.author.as_deref(),
        source: a.source.as_deref(),
        dest: a.dest.as_deref(),
        text: None,
        created_since: a.created_since.as_deref(),
        created_before: a.created_before.as_deref(),
        updated_since: a.updated_since.as_deref(),
        updated_before: a.updated_before.as_deref(),
        reviewer,
        bbql: a.query.as_deref(),
    };
    if a.merged_since.is_some() || a.merged_before.is_some() {
        return Ok(listed(lifecycle::merged(
            c,
            &p,
            &q,
            a.merged_since.as_deref(),
            a.merged_before.as_deref(),
            &paging,
        )?));
    }
    Ok(listed(bitbucket::pr_list(c, &p, &q, &paging)?))
}

fn comment(ctx: &Ctx, cmd: CommentCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        CommentCmd::List { pr, list } => {
            let (p, id) = pr.resolve(ctx)?;
            Ok(listed(bitbucket::pr_comment_list(
                c,
                &p,
                id,
                &list.paging(),
            )?))
        }
        CommentCmd::Get { ids, pr, batch } => {
            let (p, id) = pr.resolve(ctx)?;
            get_many(ids, &batch, |cid| {
                bitbucket::pr_comment_get(c, &p, id, id_arg(cid)?)
            })
        }
        CommentCmd::Create {
            pr,
            body,
            reply_to,
            pending,
            path,
            line,
            start_line,
            end_line,
            from_line,
            start_from_line,
            end_from_line,
        } => {
            let text = body.required()?;
            let n = bitbucket::NewComment {
                text: &text,
                reply_to,
                pending,
                path: path.as_deref(),
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
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_comment(c, &p, id, n)
        }
        CommentCmd::Update { id: cid, pr, body } => {
            let text = body.required()?;
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_comment_update(c, &p, id, cid, &text)
        }
        CommentCmd::Delete { id: cid, pr, .. } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_comment_delete(c, &p, id, cid)
        }
        CommentCmd::Resolve { id: cid, pr } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_comment_resolve(c, &p, id, cid)
        }
        CommentCmd::Reopen { id: cid, pr } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_comment_reopen(c, &p, id, cid)
        }
    }
}

fn task(ctx: &Ctx, cmd: TaskCmd) -> Result<Value> {
    let c = ctx.c;
    let change = |pr: &PrRef, tid: u64, t: bitbucket::TaskChange| -> Result<Value> {
        let (p, id) = pr.resolve(ctx)?;
        bitbucket::pr_task_update(c, &p, id, tid, &t)
    };
    match cmd {
        TaskCmd::List { pr, list } => {
            let (p, id) = pr.resolve(ctx)?;
            Ok(listed(bitbucket::pr_tasks(c, &p, id, &list.paging())?))
        }
        TaskCmd::Get { ids, pr, batch } => {
            let (p, id) = pr.resolve(ctx)?;
            get_many(ids, &batch, |tid| {
                bitbucket::pr_task_get(c, &p, id, id_arg(tid)?)
            })
        }
        TaskCmd::Create { pr, body, comment } => {
            let text = body.required()?;
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_task_create(c, &p, id, &text, comment)
        }
        TaskCmd::Update { id, pr, body } => {
            let text = body.required()?;
            change(
                &pr,
                id,
                bitbucket::TaskChange {
                    text: Some(&text),
                    resolved: None,
                },
            )
        }
        TaskCmd::Delete { id: tid, pr, .. } => {
            let (p, id) = pr.resolve(ctx)?;
            bitbucket::pr_task_delete(c, &p, id, tid)
        }
        TaskCmd::Resolve { id, pr } => change(
            &pr,
            id,
            bitbucket::TaskChange {
                text: None,
                resolved: Some(true),
            },
        ),
        TaskCmd::Reopen { id, pr } => change(
            &pr,
            id,
            bitbucket::TaskChange {
                text: None,
                resolved: Some(false),
            },
        ),
    }
}

fn reviewer(ctx: &Ctx, cmd: ReviewerCmd) -> Result<Value> {
    let c = ctx.c;
    let (pr, users, add) = match cmd {
        ReviewerCmd::List { pr } => {
            let (p, id) = pr.resolve(ctx)?;
            let all = lifecycle::reviewers(c, &p, id)?;
            return Ok(complete(all.as_array().cloned().unwrap_or_default()));
        }
        ReviewerCmd::Add { users, pr } => (pr, users, true),
        ReviewerCmd::Remove { users, pr } => (pr, users, false),
    };
    let (p, id) = pr.resolve(ctx)?;
    let who = any_users(&users);
    let change = lifecycle::PrUpdate {
        add: if add { who.clone() } else { Vec::new() },
        remove: if add { Vec::new() } else { who },
        ..Default::default()
    };
    let updated = lifecycle::pr_update(c, &p, id, &change)?;
    let all = lifecycle::reviewers_of(&updated);
    Ok(complete(all.as_array().cloned().unwrap_or_default()))
}

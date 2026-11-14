//! `atlx bitbucket` (alias `bb`): clap definitions and dispatch.

use super::Out;
use super::util::{DEFAULT_CONCURRENCY, batch, body_arg, concurrency_arg, deprecated, text_arg};
use crate::client::ApiError;
use crate::config::Config;
use crate::{bitbucket, jira};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

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
        #[arg(long, default_value_t = 25)]
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
}

#[derive(Subcommand)]
pub(crate) enum PrCommentCmd {
    /// Add a general, reply, or inline comment
    Create {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// `-` reads stdin
        #[arg(long)]
        text: String,
        #[arg(long)]
        reply_to: Option<u64>,
        /// Inline comment on this file path
        #[arg(long)]
        path: Option<String>,
        #[arg(long, requires = "path")]
        line: Option<u64>,
        #[command(flatten)]
        repo: RepoArgs,
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
                    limit,
                };
                bitbucket::pr_query(&c, &path(&repo, None)?, &q)?
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
                reviewer_uuid,
                reviewer_account_id,
                close_source_branch,
                draft,
                repo,
            } => {
                let description = body_arg(description, description_file)?;
                bitbucket::pr_create(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::NewPr {
                        title: &title,
                        source: &source,
                        dest: dest.as_deref(),
                        description: description.as_deref(),
                        reviewer_uuids: reviewer_uuid.as_deref().map(jira::csv).unwrap_or_default(),
                        reviewer_account_ids: reviewer_account_id
                            .as_deref()
                            .map(jira::csv)
                            .unwrap_or_default(),
                        close_source_branch,
                        draft,
                    },
                )?
            }
            PrCmd::Comment {
                cmd:
                    PrCommentCmd::Create {
                        pull_request,
                        text,
                        reply_to,
                        path: file,
                        line,
                        repo,
                    },
            } => {
                let text = text_arg(text)?;
                bitbucket::pr_comment(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::pr_number(&pull_request)?,
                    bitbucket::NewComment {
                        text: &text,
                        reply_to,
                        path: file.as_deref(),
                        line,
                    },
                )?
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
            PrCmd::Task {
                cmd:
                    PrTaskCmd::Query {
                        pull_request,
                        limit,
                        repo,
                    },
            } => bitbucket::pr_tasks(
                &c,
                &path(&repo, None)?,
                bitbucket::pr_number(&pull_request)?,
                limit,
            )?,
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
                bitbucket::pr_merge(&c, &path(&repo, None)?, pr_id, &o)?
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

//! `atlx bitbucket` (alias `bb`): clap definitions and dispatch.

use super::Out;
use super::util::{batch, body_arg, require_yes, text_arg};
use crate::config::Config;
use crate::{bitbucket, jira};
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Args)]
pub(crate) struct RepoArgs {
    /// Bitbucket workspace slug
    #[arg(long)]
    workspace: Option<String>,
    /// Repository slug, workspace/repo, or Bitbucket URL
    #[arg(long)]
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
pub(crate) enum RepoCmd {
    /// List repositories in a workspace
    Query {
        /// Search by name
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        workspace: Option<String>,
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
    /// List branches in a repository
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
    /// Activity feed for a pull request
    Activity {
        /// Pull request number or URL
        id: String,
        /// Filter by event type (e.g. comment, approval, update)
        #[arg(long = "type")]
        event_type: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Tasks on a pull request
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
    /// Merge a pull request (requires --yes)
    Merge {
        /// Pull request number or URL
        id: String,
        #[arg(long, value_enum)]
        strategy: Option<bitbucket::MergeStrategy>,
        #[arg(long)]
        close_source_branch: bool,
        #[arg(long)]
        message: Option<String>,
        /// Confirm merge
        #[arg(long, short)]
        yes: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Decline a pull request (requires --yes)
    Decline {
        /// Pull request number or URL
        id: String,
        /// Confirm decline
        #[arg(long, short)]
        yes: bool,
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
                    workspace,
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
            bitbucket::repo_query(&c, bitbucket::workspace_or(workspace.as_deref(), dws)?, &q)?
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
        BbCmd::Repo {
            cmd: RepoCmd::Branches { query, limit, repo },
        } => bitbucket::repo_branches(&c, &path(&repo, None)?, query.as_deref(), limit)?,
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
                repo,
            } => {
                let p = path(&repo, None)?;
                let o = bitbucket::PrGet {
                    comments: comments || full,
                    diff: diff || full,
                    statuses: statuses || full,
                };
                batch(ids, strict, |id| {
                    bitbucket::pr_get(&c, &p, bitbucket::pr_number(id)?, &o)
                })?
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
                id,
                event_type,
                limit,
                repo,
            } => bitbucket::pr_activity(
                &c,
                &path(&repo, None)?,
                bitbucket::pr_number(&id)?,
                event_type.as_deref(),
                limit,
            )?,
            PrCmd::Tasks { id, repo } => {
                bitbucket::pr_tasks(&c, &path(&repo, None)?, bitbucket::pr_number(&id)?)?
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
                id,
                strategy,
                close_source_branch,
                message,
                yes,
                repo,
            } => {
                let pr_id = bitbucket::pr_number(&id)?;
                require_yes(yes, &format!("merging pull request #{pr_id}"))?;
                let o = bitbucket::PrMerge {
                    strategy,
                    close_source_branch,
                    message: message.as_deref(),
                };
                bitbucket::pr_merge(&c, &path(&repo, None)?, pr_id, &o)?
            }
            PrCmd::Decline { id, yes, repo } => {
                let pr_id = bitbucket::pr_number(&id)?;
                require_yes(yes, &format!("declining pull request #{pr_id}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn bb_confirmation_and_args() {
        assert!(require_yes(false, "merging pull request #1").is_err());
        assert!(require_yes(true, "merging pull request #1").is_ok());
        assert!(
            Cli::try_parse_from([
                "atlx",
                "bb",
                "pull-requests",
                "merge",
                "1",
                "--repo",
                "acme/api",
                "--strategy",
                "merge_commit",
                "--yes"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "atlx",
                "bb",
                "pull-requests",
                "merge",
                "1",
                "--repo",
                "acme/api",
                "--strategy",
                "invalid"
            ])
            .is_err()
        );
    }
}

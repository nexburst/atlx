//! `bb repo ...` (files, branches, commits, contributors, Code Insights
//! reports, default reviewers) and `bb workspace ...`.

use super::{BatchArgs, Ctx, ListArgs, RepoArg, complete, get_many, listed};
use crate::bitbucket::{self, lifecycle, repos};
use crate::cli::Out;
use crate::client::ApiError;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use repos::Change;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum RepoCmd {
    /// List the repositories of a workspace, most recently updated first
    List {
        /// Workspace slug [default: the repository's, else the saved one]
        #[arg(long, value_name = "SLUG")]
        workspace: Option<String>,
        /// Name contains
        #[arg(long)]
        name: Option<String>,
        /// BBQL filter, e.g. 'is_private = false'
        #[arg(long, short = 'q', value_name = "BBQL")]
        query: Option<String>,
        /// Project key
        #[arg(long)]
        project: Option<String>,
        /// Main language
        #[arg(long)]
        language: Option<String>,
        /// Only repositories where you are owner, admin, contributor or member
        #[arg(long)]
        role: Option<String>,
        /// Sort field, `-` for descending
        #[arg(long, allow_hyphen_values = true, default_value = "-updated_on")]
        sort: String,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more repositories (workspace/slug; `-` reads them from stdin)
    Get {
        /// Repositories [default: the current one]
        #[arg(value_name = "REPO")]
        repos: Vec<String>,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Create a repository (private unless --public)
    Create {
        /// The new repository: workspace/slug, or a slug in the default workspace
        #[arg(value_name = "REPO")]
        repo: String,
        /// Display name [default: the slug]
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Main language
        #[arg(long)]
        language: Option<String>,
        /// Project key
        #[arg(long)]
        project: Option<String>,
        /// Visible to everyone
        #[arg(long)]
        public: bool,
    },
    /// Files: read, list, download, search
    File {
        #[command(subcommand)]
        cmd: FileCmd,
    },
    /// Branches
    Branch {
        #[command(subcommand)]
        cmd: BranchCmd,
    },
    /// Commits
    Commit {
        #[command(subcommand)]
        cmd: CommitCmd,
    },
    /// Commit authors and their commit counts
    Contributor {
        #[command(subcommand)]
        cmd: ContributorCmd,
    },
    /// Code Insights reports on commits
    Report {
        #[command(subcommand)]
        cmd: ReportCmd,
    },
    /// Users added to every new pull request as reviewers
    DefaultReviewer {
        #[command(subcommand)]
        cmd: DefaultReviewerCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum FileCmd {
    /// A file's text (the first 20 000 characters; --content-offset continues)
    Get {
        /// Path in the repository
        path: String,
        /// Branch, tag or commit [default: the main branch]
        #[arg(long = "ref", value_name = "REF")]
        git_ref: Option<String>,
        /// Start the text at this character
        #[arg(long, value_name = "N")]
        content_offset: Option<usize>,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// A directory's entries
    List {
        /// Directory [default: the root]
        #[arg(long)]
        path: Option<String>,
        /// Branch, tag or commit [default: the main branch]
        #[arg(long = "ref", value_name = "REF")]
        git_ref: Option<String>,
        /// Descend this many levels
        #[arg(long, value_name = "N")]
        depth: Option<u32>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Save a file, text or binary, to disk
    Download {
        /// Path in the repository
        path: String,
        /// Local file to write [default: the file's name, here]
        #[arg(long, value_name = "FILE")]
        to: Option<PathBuf>,
        /// Replace the local file if it exists
        #[arg(long)]
        overwrite: bool,
        /// Branch, tag or commit [default: the main branch]
        #[arg(long = "ref", value_name = "REF")]
        git_ref: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Search code; `repo:`, `path:`, `ext:`, `lang:` words in the text filter
    Search {
        /// Text to look for
        #[arg(required = true)]
        text: Vec<String>,
        /// Search the whole workspace [default: the current repository]
        #[arg(long, value_name = "SLUG")]
        workspace: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum BranchCmd {
    /// Branches, newest head commit first
    List {
        /// Name contains
        #[arg(long)]
        name: Option<String>,
        /// BBQL filter
        #[arg(long, short = 'q', value_name = "BBQL")]
        query: Option<String>,
        /// Head commit author: name (or part of it) or account ID
        #[arg(long)]
        author: Option<String>,
        /// name, -name, target.date or -target.date
        #[arg(long, allow_hyphen_values = true, default_value = "-target.date")]
        sort: String,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more branches
    Get {
        #[arg(required = true, value_name = "NAME")]
        names: Vec<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Create a branch
    Create {
        /// Name of the new branch
        name: String,
        /// Branch or commit to start from [default: the main branch]
        #[arg(long)]
        from: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Delete a branch
    Delete {
        name: String,
        /// Confirm the deletion
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommitCmd {
    /// Commit history, newest first, of the main branch by default
    List {
        /// Branch to read
        #[arg(long, conflicts_with = "include")]
        branch: Option<String>,
        /// Revisions to include; repeatable
        #[arg(long, value_name = "REV")]
        include: Vec<String>,
        /// Revisions to exclude; repeatable
        #[arg(long, value_name = "REV")]
        exclude: Vec<String>,
        /// Only commits touching this path
        #[arg(long)]
        path: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more commits
    Get {
        #[arg(required = true, value_name = "HASH")]
        hashes: Vec<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Commit files to a branch without a checkout
    Create(CommitCreateArgs),
}

#[derive(Args)]
pub(crate) struct CommitCreateArgs {
    /// Commit message
    #[arg(long, short = 'm')]
    message: String,
    /// Local file to commit at the same relative path, or repoPath=localPath;
    /// repeatable
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
    /// Commit to the repository's main branch
    #[arg(long)]
    default_branch: bool,
    /// Author, as "Name <email>"
    #[arg(long)]
    author: Option<String>,
    /// Fail unless the branch is still at this commit
    #[arg(long)]
    parent: Option<String>,
    #[command(flatten)]
    repo: RepoArg,
}

#[derive(Subcommand)]
pub(crate) enum ContributorCmd {
    /// Commit authors by commit count, over the 1000 most recent commits
    List {
        /// Branch, tag or commit [default: the main branch]
        #[arg(long = "ref", value_name = "REF", conflicts_with = "range")]
        git_ref: Option<String>,
        /// Commits in to but not in from: from..to
        #[arg(long)]
        range: Option<String>,
        /// Only commits since this day (YYYY-MM-DD or 30d)
        #[arg(long)]
        since: Option<String>,
        /// How many authors [default: 10]
        #[arg(long, value_name = "N", default_value_t = 10, value_parser = super::positive)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum ReportCmd {
    /// A commit's reports
    List {
        /// Commit hash
        #[arg(long)]
        commit: String,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Create or replace a report
    Create {
        /// Report ID (your own, e.g. the tool's name)
        id: String,
        /// Commit hash
        #[arg(long)]
        commit: String,
        /// Report JSON, or @file to read it from a file
        #[arg(long)]
        report: String,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum DefaultReviewerCmd {
    /// The repository's default reviewers
    List {
        /// Include those inherited from the project
        #[arg(long)]
        effective: bool,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One default reviewer
    Get {
        /// UUID, account ID, nickname, display name or me
        user: String,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Make a user a default reviewer of every new pull request
    Add {
        /// UUID, account ID, nickname, display name or me
        user: String,
        /// Confirm the change
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Stop a user being a default reviewer
    Remove {
        /// UUID, account ID, nickname, display name or me
        user: String,
        /// Confirm the change
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCmd {
    /// One or more workspaces [default: the current one]
    Get {
        #[arg(value_name = "SLUG")]
        slugs: Vec<String>,
        #[command(flatten)]
        batch: BatchArgs,
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
    List {
        /// Nickname or display name contains
        #[arg(long)]
        name: Option<String>,
        /// List permission levels instead (needs admin rights)
        #[arg(long)]
        permissions: bool,
        /// Workspace slug [default: the repository's, else the saved one]
        #[arg(long, value_name = "SLUG")]
        workspace: Option<String>,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more members
    Get {
        /// UUID, account ID, nickname, display name or me
        #[arg(required = true, value_name = "USER")]
        users: Vec<String>,
        /// Workspace slug [default: the repository's, else the saved one]
        #[arg(long, value_name = "SLUG")]
        workspace: Option<String>,
        #[command(flatten)]
        batch: BatchArgs,
    },
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

fn commit_create(ctx: &Ctx, a: CommitCreateArgs) -> Result<Value> {
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
        ctx.c,
        &ctx.repo(&a.repo)?,
        &repos::NewCommit {
            message: &a.message,
            branch: a.branch.as_deref(),
            author: a.author.as_deref(),
            parent: a.parent.as_deref(),
            changes: &changes,
        },
    )
}

/// `--report`: JSON, or `@file`.
fn report_arg(s: &str) -> Result<Value> {
    let text = match s.strip_prefix('@') {
        Some(f) => std::fs::read_to_string(f).with_context(|| format!("reading {f}"))?,
        None => s.to_string(),
    };
    serde_json::from_str(&text).map_err(|e| usage(format!("--report is not valid JSON: {e}")))
}

/// A user given by name or ID, as the UUID a default reviewer path takes.
fn reviewer_uuid(ctx: &Ctx, path: &str, who: &str) -> Result<String> {
    let u = lifecycle::resolve_user(ctx.c, path, lifecycle::User::Any(who.trim()))?;
    match u["uuid"].as_str() {
        Some(id) => Ok(id.to_string()),
        None => {
            // An account ID: find its UUID among the workspace members.
            let ws = bitbucket::split_path(path).0;
            let m = repos::member(ctx.c, &ws, who)?;
            m["user"]["uuid"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| usage(format!("no UUID found for '{who}'")))
        }
    }
}

pub(super) fn repo(ctx: &Ctx, cmd: RepoCmd) -> Result<Out> {
    let c = ctx.c;
    let data = match cmd {
        RepoCmd::List {
            workspace,
            name,
            query,
            project,
            language,
            role,
            sort,
            list,
        } => {
            let ws = ctx.workspace(workspace.as_deref(), None)?;
            let o = bitbucket::RepoList {
                name: name.as_deref(),
                project: project.as_deref(),
                language: language.as_deref(),
                sort: Some(&sort),
                role: role.as_deref(),
                bbql: query.as_deref(),
            };
            listed(bitbucket::repo_list(c, &ws, &o, &list.paging())?)
        }
        RepoCmd::Get { repos, batch } => {
            if repos.is_empty() {
                bitbucket::repo_get(c, &ctx.repo_of(None)?)?
            } else {
                get_many(repos, &batch, |r| {
                    bitbucket::repo_get(c, &ctx.repo_of(Some(r))?)
                })?
            }
        }
        RepoCmd::Create {
            repo,
            name,
            description,
            language,
            project,
            public,
        } => repos::repo_create(
            c,
            &ctx.repo_of(Some(&repo))?,
            &repos::NewRepo {
                name: name.as_deref(),
                description: description.as_deref(),
                language: language.as_deref(),
                project: project.as_deref(),
                private: !public,
            },
        )?,
        RepoCmd::File { cmd } => file(ctx, cmd)?,
        RepoCmd::Branch { cmd } => branch(ctx, cmd)?,
        RepoCmd::Commit { cmd } => match cmd {
            CommitCmd::List {
                branch,
                include,
                exclude,
                path,
                repo,
                list,
            } => {
                let include: Vec<String> = branch.into_iter().chain(include).collect();
                listed(repos::commit_list(
                    c,
                    &ctx.repo(&repo)?,
                    &repos::CommitQuery {
                        include: &include,
                        exclude: &exclude,
                        file_path: path.as_deref(),
                    },
                    &list.paging(),
                )?)
            }
            CommitCmd::Get {
                hashes,
                repo,
                batch,
            } => {
                let p = ctx.repo(&repo)?;
                get_many(hashes, &batch, |h| repos::commit_get(c, &p, h))?
            }
            CommitCmd::Create(a) => commit_create(ctx, a)?,
        },
        RepoCmd::Contributor {
            cmd:
                ContributorCmd::List {
                    git_ref,
                    range,
                    since,
                    limit,
                    repo,
                },
        } => {
            let since = since.as_deref().map(crate::time::to_date).transpose()?;
            let v = repos::contributors(
                c,
                &ctx.repo(&repo)?,
                &repos::Contributors {
                    git_ref: git_ref.as_deref(),
                    range: range.as_deref(),
                    since: since.as_deref(),
                    limit,
                },
            )?;
            complete(v.as_array().cloned().unwrap_or_default())
        }
        RepoCmd::Report { cmd } => match cmd {
            ReportCmd::List { commit, repo, list } => listed(repos::reports(
                c,
                &ctx.repo(&repo)?,
                &commit,
                &list.paging(),
            )?),
            ReportCmd::Create {
                id,
                commit,
                report,
                repo,
            } => {
                let report = report_arg(&report)?;
                repos::report_create(c, &ctx.repo(&repo)?, &commit, &id, &report)?
            }
        },
        RepoCmd::DefaultReviewer { cmd } => match cmd {
            DefaultReviewerCmd::List {
                effective,
                repo,
                list,
            } => listed(lifecycle::default_reviewers(
                c,
                &ctx.repo(&repo)?,
                effective,
                &list.paging(),
            )?),
            DefaultReviewerCmd::Get { user, repo } => {
                let p = ctx.repo(&repo)?;
                lifecycle::default_reviewer_get(c, &p, &reviewer_uuid(ctx, &p, &user)?)?
            }
            DefaultReviewerCmd::Add { user, repo, .. } => {
                let p = ctx.repo(&repo)?;
                lifecycle::default_reviewer_add(c, &p, &reviewer_uuid(ctx, &p, &user)?)?
            }
            DefaultReviewerCmd::Remove { user, repo, .. } => {
                let p = ctx.repo(&repo)?;
                lifecycle::default_reviewer_remove(c, &p, &reviewer_uuid(ctx, &p, &user)?)?
            }
        },
    };
    Ok(Out::Data(data))
}

fn file(ctx: &Ctx, cmd: FileCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        FileCmd::Get {
            path,
            git_ref,
            repo,
            ..
        } => repos::file_get(c, &ctx.repo(&repo)?, &path, git_ref.as_deref()),
        FileCmd::List {
            path,
            git_ref,
            depth,
            repo,
            list,
        } => Ok(listed(repos::file_list(
            c,
            &ctx.repo(&repo)?,
            git_ref.as_deref(),
            path.as_deref(),
            depth,
            &list.paging(),
        )?)),
        FileCmd::Download {
            path,
            to,
            overwrite,
            git_ref,
            repo,
        } => {
            let dest = match to {
                Some(t) => t,
                None => {
                    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                    if name.is_empty() {
                        return Err(usage("pass --to with a file name"));
                    }
                    PathBuf::from(name)
                }
            };
            repos::file_download(
                c,
                &ctx.repo(&repo)?,
                &path,
                git_ref.as_deref(),
                &dest,
                overwrite,
            )
        }
        FileCmd::Search {
            text,
            workspace,
            repo,
            list,
        } => {
            // An explicit --workspace searches all of it; otherwise the
            // repository (given or current) narrows the search.
            let narrow = match (&workspace, &repo.repo) {
                (_, Some(r)) => Some(r.clone()),
                (Some(_), None) => None,
                (None, None) => ctx.default_repo.map(String::from),
            };
            let ws = ctx.workspace(workspace.as_deref(), narrow.as_deref())?;
            let slug = narrow
                .as_deref()
                .map(|r| ctx.repo_of(Some(r)).map(|p| bitbucket::split_path(&p).1))
                .transpose()?;
            Ok(listed(bitbucket::search_code(
                c,
                &ws,
                &text.join(" "),
                slug.as_deref(),
                &list.paging(),
            )?))
        }
    }
}

fn branch(ctx: &Ctx, cmd: BranchCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        BranchCmd::List {
            name,
            query,
            author,
            sort,
            repo,
            list,
        } => Ok(listed(bitbucket::branch_list(
            c,
            &ctx.repo(&repo)?,
            &bitbucket::BranchQuery {
                name: name.as_deref(),
                author: author.as_deref(),
                sort: Some(&sort),
                bbql: query.as_deref(),
            },
            &list.paging(),
        )?)),
        BranchCmd::Get { names, repo, batch } => {
            let p = ctx.repo(&repo)?;
            get_many(names, &batch, |n| repos::branch_get(c, &p, n))
        }
        BranchCmd::Create { name, from, repo } => {
            repos::branch_create(c, &ctx.repo(&repo)?, &name, from.as_deref())
        }
        BranchCmd::Delete { name, repo, .. } => repos::branch_delete(c, &ctx.repo(&repo)?, &name),
    }
}

pub(super) fn workspace(ctx: &Ctx, cmd: WorkspaceCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        WorkspaceCmd::Get { slugs, batch } => {
            if slugs.is_empty() {
                repos::workspace_get(c, &ctx.workspace(None, None)?)
            } else {
                get_many(slugs, &batch, |s| repos::workspace_get(c, s))
            }
        }
        WorkspaceCmd::Member {
            cmd:
                MemberCmd::List {
                    name,
                    permissions,
                    workspace,
                    list,
                },
        } => {
            let ws = ctx.workspace(workspace.as_deref(), None)?;
            Ok(listed(repos::members(
                c,
                &ws,
                name.as_deref(),
                permissions,
                &list.paging(),
            )?))
        }
        WorkspaceCmd::Member {
            cmd:
                MemberCmd::Get {
                    users,
                    workspace,
                    batch,
                },
        } => {
            let ws = ctx.workspace(workspace.as_deref(), None)?;
            get_many(users, &batch, |u| repos::member(c, &ws, u))
        }
    }
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
        assert_eq!(report_arg(r#"{"title": "x"}"#).unwrap()["title"], "x");
        assert!(report_arg("nope").is_err());
    }
}

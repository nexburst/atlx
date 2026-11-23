//! `confluence page attachment|label|version|task ...`.

use super::{Batch, Paging, Yes};
use crate::client::{ApiError, Client};
use crate::confluence::{self, ContentType, attachments, extras, history, labels};
use crate::markup::BodyFormat;
use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The page or blog post a sub-resource command acts on.
#[derive(Args)]
pub(crate) struct PageRef {
    /// Page or blog post ID, URL, or exact title with --space
    #[arg(long)]
    page: String,
    /// Space key or ID for looking --page up by exact title
    #[arg(long)]
    space: Option<String>,
}

impl PageRef {
    fn id(&self, c: &Client) -> Result<String> {
        confluence::resolve_id(c, &self.page, self.space.as_deref())
    }
}

#[derive(Subcommand)]
pub(crate) enum AttachmentCmd {
    /// List a page's attachments
    List {
        #[command(flatten)]
        page: PageRef,
        /// Only this media type, e.g. image/png
        #[arg(long)]
        media_type: Option<String>,
        /// Only this file name
        #[arg(long)]
        filename: Option<String>,
        /// current (default), archived or trashed
        #[arg(long, value_parser = ["current", "archived", "trashed"], default_value = "current")]
        status: String,
        /// created-date or modified-date; prefix - for newest first
        #[arg(long, allow_hyphen_values = true)]
        sort: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Attachment details by ID (att12345) or URL
    Get {
        /// Attachment IDs or URLs; `-` reads them from stdin
        #[arg(value_name = "ID", required = true)]
        ids: Vec<String>,
        /// This version instead of the latest
        #[arg(long)]
        version: Option<u64>,
        #[command(flatten)]
        batch: Batch,
    },
    /// Save an attachment to a local file
    Download {
        /// Attachment ID (att12345) or URL
        #[arg(value_name = "ID")]
        id: String,
        /// File to write (default: the attachment's name, in the current
        /// directory)
        #[arg(long)]
        out: Option<PathBuf>,
        /// This version instead of the latest
        #[arg(long)]
        version: Option<u64>,
        /// Replace the file if it exists
        #[arg(long)]
        force: bool,
    },
    /// Attach local files; a file with the name of an existing attachment
    /// becomes its new version
    Upload {
        #[command(flatten)]
        page: PageRef,
        /// File to upload; repeat for several
        #[arg(long, required = true)]
        file: Vec<String>,
        /// Comment for the new attachment version
        #[arg(long)]
        comment: Option<String>,
        /// Don't notify watchers
        #[arg(long)]
        minor_edit: bool,
    },
    /// Move an attachment to the trash, or with --purge delete it from the
    /// trash for good
    Delete {
        /// Attachment ID (att12345) or URL
        #[arg(value_name = "ID")]
        id: String,
        /// Delete it for good; it must be in the trash already
        #[arg(long)]
        purge: bool,
        #[command(flatten)]
        yes: Yes,
    },
}

#[derive(Subcommand)]
pub(crate) enum LabelCmd {
    /// List a page's labels
    List {
        #[command(flatten)]
        page: PageRef,
        /// page, blogpost or custom-content, which skips probing for it
        #[arg(long = "type", value_enum)]
        kind: Option<labels::LabelTarget>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add labels; prints the page's labels afterwards
    Create {
        /// Label names (commas also separate them)
        #[arg(value_name = "LABEL", required = true)]
        labels: Vec<String>,
        #[command(flatten)]
        page: PageRef,
        /// Label prefix
        #[arg(long, default_value = "global")]
        prefix: String,
    },
    /// Remove labels
    Delete {
        /// Label names (commas also separate them)
        #[arg(value_name = "LABEL", required = true)]
        labels: Vec<String>,
        #[command(flatten)]
        page: PageRef,
        /// Label prefix
        #[arg(long, default_value = "global")]
        prefix: String,
        #[command(flatten)]
        yes: Yes,
    },
}

#[derive(Subcommand)]
pub(crate) enum VersionCmd {
    /// List versions, newest first
    List {
        #[command(flatten)]
        page: PageRef,
        #[command(flatten)]
        paging: Paging,
    },
    /// One version, with the title and body it had
    Get {
        /// Version number
        #[arg(value_name = "N")]
        number: u64,
        #[command(flatten)]
        page: PageRef,
        /// The body as markdown (default), adf, storage or plain
        #[arg(long, value_enum, default_value_t)]
        body_format: BodyFormat,
    },
    /// A unified diff between two versions' bodies
    Diff {
        #[command(flatten)]
        page: PageRef,
        /// Older version number
        #[arg(long)]
        from: u64,
        /// Newer version number
        #[arg(long)]
        to: u64,
        /// Compare markdown (default) or adf (one top-level node per line)
        #[arg(long, value_enum, default_value_t)]
        body_format: BodyFormat,
    },
    /// Make an old version the latest again (adds a new version)
    Restore {
        /// Version number to bring back
        #[arg(value_name = "N")]
        number: u64,
        #[command(flatten)]
        page: PageRef,
        /// Message for the new version
        #[arg(long)]
        message: Option<String>,
        /// Bring back that version's title too
        #[arg(long)]
        restore_title: bool,
        #[command(flatten)]
        yes: Yes,
    },
}

/// `--status` on `task list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum TaskState {
    Open,
    Complete,
}

#[derive(Subcommand)]
pub(crate) enum TaskCmd {
    /// List the tasks of a page, or every task you can see
    List {
        /// Page or blog post ID, URL, or exact title with --space
        #[arg(long)]
        page: Option<String>,
        /// Space key or ID for looking --page up by exact title
        #[arg(long, requires = "page")]
        space: Option<String>,
        /// page or blogpost, which skips probing for it
        #[arg(long = "type", value_enum, requires = "page")]
        kind: Option<ContentType>,
        /// open, complete or all (default)
        #[arg(long, value_enum, default_value = "all")]
        status: extras::TaskFilter,
        #[command(flatten)]
        paging: Paging,
    },
    /// Tasks by ID
    Get {
        /// Task IDs; `-` reads them from stdin
        #[arg(value_name = "ID", required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        batch: Batch,
    },
    /// Complete or reopen a task
    Update {
        /// Task ID
        #[arg(value_name = "ID")]
        id: String,
        /// complete or open
        #[arg(long, value_enum)]
        status: TaskState,
    },
}

pub(super) fn attachment(c: &Client, cmd: AttachmentCmd) -> Result<Value> {
    match cmd {
        AttachmentCmd::List {
            page,
            media_type,
            filename,
            status,
            sort,
            paging,
        } => {
            let id = page.id(c)?;
            paging.collect(250, |cur, n| {
                attachments::list(
                    c,
                    &id,
                    None,
                    &attachments::ListOpts {
                        limit: n,
                        cursor: cur,
                        sort: sort.as_deref(),
                        status: Some(status.as_str()),
                        media_type: media_type.as_deref(),
                        filename: filename.as_deref(),
                    },
                )
            })
        }
        AttachmentCmd::Get {
            ids,
            version,
            batch,
        } => batch.run(ids, |id| attachments::get(c, id, version)),
        AttachmentCmd::Download {
            id,
            out,
            version,
            force,
        } => attachments::download(c, &id, out.as_deref(), version, force),
        AttachmentCmd::Upload {
            page,
            file,
            comment,
            minor_edit,
        } => {
            let id = page.id(c)?;
            // One at a time: parallel writes to one page's attachments conflict.
            crate::cli::util::batch(file, false, 1, |f| {
                attachments::upload(c, &id, None, Path::new(f), comment.as_deref(), minor_edit)
            })
        }
        AttachmentCmd::Delete { id, purge, .. } => attachments::delete(c, &id, purge),
    }
}

pub(super) fn label(c: &Client, cmd: LabelCmd) -> Result<Value> {
    match cmd {
        LabelCmd::List { page, kind, paging } => {
            let id = page.id(c)?;
            paging.collect(250, |cur, n| labels::list(c, &id, None, kind, n, cur))
        }
        LabelCmd::Create {
            labels: names,
            page,
            prefix,
        } => labels::add(c, &page.page, page.space.as_deref(), &names, &prefix),
        LabelCmd::Delete {
            labels: names,
            page,
            prefix,
            ..
        } => labels::remove(c, &page.page, page.space.as_deref(), &names, &prefix),
    }
}

pub(super) fn version(c: &Client, cmd: VersionCmd) -> Result<Value> {
    match cmd {
        VersionCmd::List { page, paging } => {
            let id = page.id(c)?;
            paging.collect(250, |cur, n| history::list(c, &id, None, n, cur))
        }
        VersionCmd::Get {
            number,
            page,
            body_format,
        } => history::get(c, &page.page, page.space.as_deref(), number, body_format),
        VersionCmd::Diff {
            page,
            from,
            to,
            body_format,
        } => history::diff(c, &page.page, page.space.as_deref(), from, to, body_format),
        VersionCmd::Restore {
            number,
            page,
            message,
            restore_title,
            ..
        } => history::restore(
            c,
            &page.page,
            page.space.as_deref(),
            number,
            message.as_deref(),
            restore_title,
        ),
    }
}

pub(super) fn task(c: &Client, cmd: TaskCmd) -> Result<Value> {
    match cmd {
        TaskCmd::List {
            page,
            space,
            kind,
            status,
            paging,
        } => {
            let id = page
                .as_deref()
                .map(|p| confluence::resolve_id(c, p, space.as_deref()))
                .transpose()?;
            paging.collect(250, |cur, n| {
                extras::tasks(c, id.as_deref(), kind, status, n, cur)
            })
        }
        TaskCmd::Get { ids, batch } => batch.run(ids, |id| extras::task(c, id)),
        TaskCmd::Update { id, status } => {
            if id.trim().is_empty() {
                return Err(ApiError::usage("pass a task ID").into());
            }
            extras::task_set(c, &id, status == TaskState::Complete)
        }
    }
}

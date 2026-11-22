//! `jira workitem attachment|worklog|watcher|vote`.

use super::{PAGE, issue_ref, offset_envelope};
use crate::cli::util::{require_yes, text_arg};
use crate::client::{ApiError, Client};
use crate::jira::{self, attachments, people, worklogs};
use crate::markup::TextFormat;
use crate::output::envelope::envelope;
use crate::time::epoch_millis;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum AttachmentCmd {
    /// The files attached to a work item
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
    /// One attachment's metadata
    Get {
        /// Attachment ID or ARI
        #[arg(long)]
        attachment_id: String,
    },
    /// Save an attachment to a local file
    Download(SaveArgs),
    /// Save an attachment's thumbnail to a local file
    Thumbnail(SaveArgs),
    /// Attach local files to a work item
    Upload {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// File to attach; repeat for several
        #[arg(long, required = true)]
        file: Vec<PathBuf>,
    },
    /// Delete an attachment
    Delete {
        /// Attachment ID or ARI
        #[arg(long)]
        attachment_id: String,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Args)]
pub(crate) struct SaveArgs {
    /// Attachment ID or ARI
    #[arg(long)]
    attachment_id: String,
    /// File to write (default: the attachment's name, in the current
    /// directory)
    #[arg(long, alias = "out")]
    output_path: Option<PathBuf>,
    /// Replace the file if it exists (atlx extra)
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
pub(crate) enum WatcherCmd {
    /// Who watches a work item
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
    /// Add a watcher (you, by default)
    Add(WatchArgs),
    /// Remove a watcher (you, by default)
    Remove(WatchArgs),
}

#[derive(Args)]
pub(crate) struct WatchArgs {
    /// Work item key, ID, ARI or URL
    #[arg(long, alias = "issue-id")]
    id: String,
    /// User ARI, account ID, email, name or `me`
    #[arg(long, default_value = "me")]
    watcher_id: String,
}

#[derive(Subcommand)]
pub(crate) enum VoteCmd {
    /// The votes on a work item
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
    /// Vote for a work item as yourself
    Add {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
    /// Take your vote back
    Remove {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum WorklogCmd {
    /// Log time on a work item
    Add {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        #[command(flatten)]
        entry: EntryArgs,
    },
    /// Change a worklog
    Update {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Worklog ID or ARI
        #[arg(long)]
        id: String,
        #[command(flatten)]
        entry: EntryArgs,
    },
    /// One page of a work item's worklogs
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Page size [default: 100]
        #[arg(long)]
        first: Option<usize>,
        /// Cursor from the previous page's pageInfo.nextCursor
        #[arg(long)]
        after: Option<String>,
        /// Only work started after this (epoch ms, date, timestamp or 7d)
        #[arg(long)]
        started_after: Option<String>,
        /// Only work started before this (same forms)
        #[arg(long)]
        started_before: Option<String>,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
    /// One worklog
    Get {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Worklog ID or ARI
        #[arg(long)]
        id: String,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
    /// Delete a worklog
    Delete {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Worklog ID or ARI
        #[arg(long)]
        id: String,
        #[command(flatten)]
        options: WriteArgs,
        /// Put this much back on the estimate (with manual adjustment)
        #[arg(long)]
        increase_by: Option<String>,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// IDs of worklogs changed since a time, site-wide
    Changed {
        /// Epoch ms, date, timestamp or an age like 7d
        #[arg(long)]
        since: Option<String>,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
    /// IDs of worklogs deleted since a time, site-wide
    Deleted {
        /// Epoch ms, date, timestamp or an age like 7d
        #[arg(long)]
        since: Option<String>,
    },
}

/// Flags shared by every worklog write.
#[derive(Args)]
pub(crate) struct WriteArgs {
    /// How the remaining estimate changes
    #[arg(long, value_enum)]
    adjust_estimate: Option<worklogs::Adjust>,
    /// The remaining estimate to set (with new adjustment)
    #[arg(long)]
    new_estimate: Option<String>,
    /// Notify watchers (true or false)
    #[arg(long, value_name = "BOOL")]
    notify_users: Option<bool>,
    /// Write even if the work item isn't editable (needs admin rights)
    #[arg(long)]
    override_editable: bool,
}

#[derive(Args)]
pub(crate) struct EntryArgs {
    /// Time spent, e.g. 2h or "1d 4h"
    #[arg(long, conflicts_with = "time_spent_seconds")]
    time_spent: Option<String>,
    /// Time spent, in seconds
    #[arg(long)]
    time_spent_seconds: Option<u64>,
    /// When the work started: timestamp, date (UTC), epoch ms, or an age
    /// like 3h
    #[arg(long)]
    started: Option<String>,
    /// What was done; `-` reads stdin
    #[arg(long)]
    comment: Option<String>,
    /// Format of --comment
    #[arg(long, value_enum, default_value_t)]
    comment_format: TextFormat,
    /// REST visibility object
    #[arg(long, value_name = "JSON")]
    visibility_json: Option<String>,
    /// REST entity properties array
    #[arg(long, value_name = "JSON")]
    properties_json: Option<String>,
    #[command(flatten)]
    options: WriteArgs,
    /// Take this much off the estimate (with manual adjustment)
    #[arg(long)]
    reduce_by: Option<String>,
    /// REST expand value
    #[arg(long)]
    expand: Option<String>,
}

impl WriteArgs {
    fn opts(&self) -> worklogs::WriteOpts<'_> {
        worklogs::WriteOpts {
            adjust: self.adjust_estimate,
            new_estimate: self.new_estimate.as_deref(),
            notify_users: self.notify_users,
            override_editable: self.override_editable,
            ..Default::default()
        }
    }
}

impl EntryArgs {
    fn opts(&self) -> worklogs::WriteOpts<'_> {
        worklogs::WriteOpts {
            reduce_by: self.reduce_by.as_deref(),
            expand: self.expand.as_deref(),
            ..self.options.opts()
        }
    }
}

fn json_arg(flag: &str, text: Option<&str>) -> Result<Option<Value>> {
    text.map(|t| {
        serde_json::from_str(t)
            .map_err(|e| ApiError::usage(format!("{flag} is not valid JSON: {e}")).into())
    })
    .transpose()
}

fn millis(value: Option<&str>) -> Result<Option<i64>> {
    value.map(epoch_millis).transpose()
}

pub(super) fn attachment(c: &Client, cmd: AttachmentCmd) -> Result<Value> {
    const KIND: &str = "jira.attachment";
    let site = jira::site(c);
    match cmd {
        AttachmentCmd::Query { issue_id } => {
            let key = issue_ref(&issue_id);
            Ok(envelope("jira.workitem.attachment.query", KIND)
                .request(json!({"site": site, "issueId": key}))
                .data(attachments::query(c, &key)?))
        }
        AttachmentCmd::Get { attachment_id } => {
            let id = jira::entity_id(&attachment_id);
            Ok(envelope("jira.workitem.attachment.get", KIND)
                .request(json!({"site": site, "attachmentId": id}))
                .data(attachments::get(c, &id)?))
        }
        AttachmentCmd::Download(a) => save(c, a, "download", attachments::Bytes::Content),
        AttachmentCmd::Thumbnail(a) => save(c, a, "thumbnail", attachments::Bytes::Thumbnail),
        AttachmentCmd::Upload { issue_id, file } => {
            let key = issue_ref(&issue_id);
            let names: Vec<String> = file.iter().map(|f| f.display().to_string()).collect();
            Ok(envelope("jira.workitem.attachment.upload", KIND)
                .request(json!({"site": site, "issueId": key, "files": names}))
                .data(attachments::upload(c, &key, &file)?))
        }
        AttachmentCmd::Delete { attachment_id, yes } => {
            require_yes(yes, "deleting the attachment")?;
            let id = jira::entity_id(&attachment_id);
            Ok(envelope("jira.workitem.attachment.delete", KIND)
                .request(json!({"site": site, "attachmentId": id}))
                .data(attachments::delete(c, &id)?))
        }
    }
}

fn save(c: &Client, a: SaveArgs, verb: &str, what: attachments::Bytes) -> Result<Value> {
    let id = jira::entity_id(&a.attachment_id);
    let data = attachments::download(c, &id, what, a.output_path.as_deref(), a.force)?;
    Ok(envelope(
        &format!("jira.workitem.attachment.{verb}"),
        "jira.attachment",
    )
    .request(json!({
        "site": jira::site(c),
        "attachmentId": id,
        "outputPath": a.output_path.map(|p| p.display().to_string()),
    }))
    .data(data))
}

pub(super) fn watcher(c: &Client, cmd: WatcherCmd) -> Result<Value> {
    const KIND: &str = "jira.watcher";
    let site = jira::site(c);
    let (verb, key, data) = match cmd {
        WatcherCmd::Query { issue_id } => {
            let key = issue_ref(&issue_id);
            return Ok(envelope("jira.workitem.watcher.query", KIND)
                .request(json!({"site": site, "issueId": key}))
                .data(people::watchers(c, &key)?));
        }
        WatcherCmd::Add(a) => {
            let key = issue_ref(&a.id);
            let data = people::watcher_add(c, &key, &a.watcher_id)?;
            ("add", key, data)
        }
        WatcherCmd::Remove(a) => {
            let key = issue_ref(&a.id);
            let data = people::watcher_remove(c, &key, &a.watcher_id)?;
            ("remove", key, data)
        }
    };
    Ok(envelope(&format!("jira.workitem.watcher.{verb}"), KIND)
        .request(json!({"site": site, "id": key, "watcherId": data["accountId"]}))
        .data(data))
}

pub(super) fn vote(c: &Client, cmd: VoteCmd) -> Result<Value> {
    let (verb, key, data) = match cmd {
        VoteCmd::Query { issue_id } => {
            let key = issue_ref(&issue_id);
            let data = people::votes(c, &key)?;
            ("query", key, data)
        }
        VoteCmd::Add { issue_id } => {
            let key = issue_ref(&issue_id);
            let data = people::vote(c, &key, true)?;
            ("add", key, data)
        }
        VoteCmd::Remove { issue_id } => {
            let key = issue_ref(&issue_id);
            let data = people::vote(c, &key, false)?;
            ("remove", key, data)
        }
    };
    Ok(envelope(&format!("jira.workitem.vote.{verb}"), "jira.vote")
        .request(json!({"site": jira::site(c), "issueId": key}))
        .data(data))
}

pub(super) fn worklog(c: &Client, cmd: WorklogCmd) -> Result<Value> {
    const KIND: &str = "jira.worklog";
    let site = jira::site(c);
    match cmd {
        WorklogCmd::Add { issue_id, entry } => write(c, &issue_id, None, entry),
        WorklogCmd::Update {
            issue_id,
            id,
            entry,
        } => write(c, &issue_id, Some(&id), entry),
        WorklogCmd::Query {
            issue_id,
            first,
            after,
            started_after,
            started_before,
            expand,
        } => {
            let key = issue_ref(&issue_id);
            let (from, to) = (
                millis(started_after.as_deref())?,
                millis(started_before.as_deref())?,
            );
            let page = worklogs::query(
                c,
                &key,
                jira::offset_cursor(after.as_deref())?,
                first.unwrap_or(PAGE),
                from,
                to,
                expand.as_deref(),
            )?;
            let mut request = super::page_request(c, &key, first, after.as_deref());
            if let Some(v) = from {
                request["startedAfter"] = v.into();
            }
            if let Some(v) = to {
                request["startedBefore"] = v.into();
            }
            Ok(offset_envelope(
                "jira.workitem.worklog.query",
                KIND,
                request,
                page,
            ))
        }
        WorklogCmd::Get {
            issue_id,
            id,
            expand,
        } => {
            let (key, id) = (issue_ref(&issue_id), jira::entity_id(&id));
            Ok(envelope("jira.workitem.worklog.get", KIND)
                .request(json!({"site": site, "issueId": key, "id": id}))
                .data(worklogs::get(c, &key, &id, expand.as_deref())?))
        }
        WorklogCmd::Delete {
            issue_id,
            id,
            options,
            increase_by,
            yes,
        } => {
            require_yes(yes, "deleting the worklog")?;
            let (key, id) = (issue_ref(&issue_id), jira::entity_id(&id));
            let opts = worklogs::WriteOpts {
                increase_by: increase_by.as_deref(),
                ..options.opts()
            };
            Ok(envelope("jira.workitem.worklog.delete", KIND)
                .request(json!({"site": site, "issueId": key, "id": id}))
                .data(worklogs::delete(c, &key, &id, &opts)?))
        }
        WorklogCmd::Changed { since, expand } => {
            let since = millis(since.as_deref())?;
            let page = worklogs::feed(c, worklogs::Feed::Changed, since, expand.as_deref())?;
            Ok(feed_out("changed", site, since, page))
        }
        WorklogCmd::Deleted { since } => {
            let since = millis(since.as_deref())?;
            let page = worklogs::feed(c, worklogs::Feed::Deleted, since, None)?;
            Ok(feed_out("deleted", site, since, page))
        }
    }
}

/// twg prints the IDs as `data` and where the page ends as `meta.sync`.
fn feed_out(kind: &str, site: impl Into<Value>, since: Option<i64>, page: Value) -> Value {
    let sync = json!({
        "type": kind,
        "since": page["since"],
        "until": page["until"],
        "lastPage": page["lastPage"],
    });
    let site: Value = site.into();
    let rows = page["values"].as_array().cloned().unwrap_or_default();
    envelope(&format!("jira.workitem.worklog.{kind}"), "jira.worklog")
        .request(json!({"site": site, "since": since}))
        .meta("sync", sync)
        .data(rows.into())
}

/// `add`, or `update` when there is a worklog ID.
fn write(c: &Client, issue: &str, id: Option<&str>, a: EntryArgs) -> Result<Value> {
    let key = issue_ref(issue);
    let comment = a.comment.clone().map(text_arg).transpose()?;
    let entry = worklogs::Entry {
        time_spent: a.time_spent.as_deref(),
        seconds: a.time_spent_seconds,
        started: millis(a.started.as_deref())?,
        comment: comment.as_deref().map(|t| (t, a.comment_format)),
        visibility: json_arg("--visibility-json", a.visibility_json.as_deref())?,
        properties: json_arg("--properties-json", a.properties_json.as_deref())?,
    };
    let mut request = json!({"site": jira::site(c), "issueId": key});
    let (verb, data) = match id {
        Some(id) => {
            let id = jira::entity_id(id);
            let data = worklogs::update(c, &key, &id, &entry, &a.opts())?;
            request["id"] = id.into();
            ("update", data)
        }
        None => ("add", worklogs::add(c, &key, &entry, &a.opts())?),
    };
    Ok(
        envelope(&format!("jira.workitem.worklog.{verb}"), "jira.worklog")
            .request(request)
            .data(data),
    )
}

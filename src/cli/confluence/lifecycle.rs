//! `confluence content` lifecycle commands and `confluence space` writes.

use crate::cli::util::require_yes;
use crate::client::{ApiError, Client};
use crate::confluence::{self, lifecycle};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::Args;
use serde_json::{Value, json};
use std::time::Duration;

fn usage(message: &str) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// The content a lifecycle command acts on.
#[derive(Args)]
pub(crate) struct ActArgs {
    /// Content ID, ARI or URL, or an exact title with --reference-space
    id: String,
    /// Space key or ID for looking up an exact title
    #[arg(long)]
    reference_space: Option<String>,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

impl ActArgs {
    fn target(&self, c: &Client, action: &str) -> Result<String> {
        require_yes(self.yes, action)?;
        lifecycle::exact_content(c, &self.id, self.reference_space.as_deref())
    }
}

#[derive(Args)]
pub(crate) struct ArchiveArgs {
    #[command(flatten)]
    act: ActArgs,
    /// Wait until it is archived (atlx extra)
    #[arg(long)]
    wait: bool,
    /// Seconds --wait may take (atlx extra)
    #[arg(long, default_value_t = 120, requires = "wait")]
    wait_timeout: u64,
}

#[derive(Args)]
pub(crate) struct MoveArgs {
    #[command(flatten)]
    act: ActArgs,
    /// Make it the last child of this content
    #[arg(long, conflicts_with_all = ["before", "after", "target"])]
    parent_id: Option<String>,
    /// Put it before this sibling
    #[arg(long, conflicts_with_all = ["after", "target"])]
    before: Option<String>,
    /// Put it after this sibling
    #[arg(long, conflicts_with = "target")]
    after: Option<String>,
    /// What --position is relative to (atlx extra)
    #[arg(long)]
    target: Option<String>,
    /// append (default), before or after --target (atlx extra)
    #[arg(long, value_enum, requires = "target")]
    position: Option<lifecycle::Position>,
}

#[derive(Args)]
pub(crate) struct CopyArgs {
    #[command(flatten)]
    act: ActArgs,
    /// Copy under this content
    #[arg(
        long,
        alias = "destination-page",
        conflicts_with = "space_key",
        required_unless_present = "space_key"
    )]
    parent_content_id: Option<String>,
    /// Copy to the top of this space
    #[arg(long, alias = "destination-space")]
    space_key: Option<String>,
    /// Title of the copy
    #[arg(long)]
    title: Option<String>,
    /// Copy the attachments too (atlx extra)
    #[arg(long)]
    copy_attachments: bool,
    /// Copy the labels too (atlx extra)
    #[arg(long)]
    copy_labels: bool,
    /// Copy the restrictions too (atlx extra)
    #[arg(long)]
    copy_permissions: bool,
}

#[derive(Args)]
pub(crate) struct DraftArgs {
    #[command(flatten)]
    act: ActArgs,
    /// page or blogpost, which skips probing for it
    #[arg(long, value_enum)]
    content_type: Option<confluence::ContentType>,
}

#[derive(Args)]
pub(crate) struct PublishArgs {
    #[command(flatten)]
    draft: DraftArgs,
    /// Publish under this title
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
pub(crate) struct SpaceRef {
    /// Space key, ID, ARI or space URL; names and content URLs are refused
    #[arg(value_name = "id-or-key", conflicts_with_all = ["id", "key"])]
    id_or_key: Option<String>,
    /// Space ID, key, ARI or URL
    #[arg(long, conflicts_with = "key")]
    id: Option<String>,
    /// Space key
    #[arg(long)]
    key: Option<String>,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

impl SpaceRef {
    fn target(&self, action: &str) -> Result<&str> {
        require_yes(self.yes, action)?;
        self.id_or_key
            .as_deref()
            .or(self.id.as_deref())
            .or(self.key.as_deref())
            .ok_or_else(|| usage("pass the space as an argument, or with --key or --id"))
    }
}

#[derive(Args)]
pub(crate) struct SpaceCreateArgs {
    /// Key: letters and digits
    #[arg(long)]
    key: String,
    /// Name
    #[arg(long)]
    name: String,
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// Only you can see it
    #[arg(long)]
    private: bool,
    /// Print the request without sending it
    #[arg(long)]
    dry_run: bool,
    /// Confirm (required, except with --dry-run)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct SpaceUpdateArgs {
    #[command(flatten)]
    space: SpaceRef,
    /// New name
    #[arg(long)]
    name: Option<String>,
    /// New description
    #[arg(long)]
    description: Option<String>,
    /// global or personal
    #[arg(long = "type", value_parser = ["global", "personal"])]
    kind: Option<String>,
    /// current or archived
    #[arg(long, value_parser = ["current", "archived"])]
    status: Option<String>,
    /// Homepage ID or URL (atlx extra)
    #[arg(long)]
    homepage: Option<String>,
}

fn out(command: &str, kind: &str, request: Value, data: Value) -> Value {
    envelope(
        &format!("confluence.{command}"),
        &format!("confluence.{kind}"),
    )
    .request(request)
    .data(data)
}

pub(super) fn trash(c: &Client, a: ActArgs) -> Result<Value> {
    let id = a.target(c, "moving the content to the trash")?;
    let data = lifecycle::trash(c, &id)?;
    Ok(out("content.trash", "content", json!({"id": id}), data))
}

pub(super) fn purge(c: &Client, a: ActArgs) -> Result<Value> {
    let id = a.target(c, "deleting the content for good")?;
    let data = lifecycle::purge(c, &id)?;
    Ok(out("content.purge", "content", json!({"id": id}), data))
}

pub(super) fn untrash(c: &Client, a: ActArgs) -> Result<Value> {
    let id = a.target(c, "restoring the content")?;
    let data = lifecycle::untrash(c, &id)?;
    Ok(out("content.untrash", "content", json!({"id": id}), data))
}

pub(super) fn archive(c: &Client, a: ArchiveArgs) -> Result<Value> {
    let id = a.act.target(c, "archiving the content")?;
    let wait = a.wait.then(|| lifecycle::Wait {
        timeout: Duration::from_secs(a.wait_timeout),
        interval: Duration::from_secs(2),
    });
    let data = lifecycle::archive(c, &id, wait)?;
    Ok(out("content.archive", "content", json!({"id": id}), data))
}

pub(super) fn move_content(c: &Client, a: MoveArgs) -> Result<Value> {
    use lifecycle::Position;
    let (position, to) = match (&a.parent_id, &a.before, &a.after, &a.target) {
        (Some(p), _, _, _) => (Position::Append, p),
        (_, Some(b), _, _) => (Position::Before, b),
        (_, _, Some(x), _) => (Position::After, x),
        (_, _, _, Some(t)) => (a.position.unwrap_or(Position::Append), t),
        _ => return Err(usage("pass --parent-id, --before, --after or --target")),
    };
    let id = a.act.target(c, "moving the content")?;
    let to = lifecycle::exact_content(c, to, a.act.reference_space.as_deref())?;
    let data = lifecycle::move_content(c, &id, position, &to)?;
    Ok(out(
        "content.move",
        "content",
        json!({"id": id, "targetId": to}),
        data,
    ))
}

pub(super) fn copy(c: &Client, a: CopyArgs) -> Result<Value> {
    let id = a.act.target(c, "copying the content")?;
    let parent = a
        .parent_content_id
        .as_deref()
        .map(|p| lifecycle::exact_content(c, p, a.act.reference_space.as_deref()))
        .transpose()?;
    let to = match (&parent, &a.space_key) {
        (Some(p), _) => lifecycle::CopyTo::Parent(p),
        (None, Some(s)) => lifecycle::CopyTo::Space(s),
        (None, None) => return Err(usage("pass --parent-content-id or --space-key")),
    };
    let opts = lifecycle::CopyOpts {
        to,
        title: a.title.as_deref(),
        attachments: a.copy_attachments,
        labels: a.copy_labels,
        permissions: a.copy_permissions,
    };
    let data = lifecycle::copy(c, &id, &opts)?;
    let request = json!({"contentId": id, "parentContentId": parent, "spaceKey": a.space_key});
    Ok(out("content.copy", "content", request, data))
}

pub(super) fn publish(c: &Client, a: PublishArgs) -> Result<Value> {
    let id = a.draft.act.target(c, "publishing the draft")?;
    let data = lifecycle::publish(c, &id, a.draft.content_type, a.title.as_deref())?;
    Ok(out("content.publish", "content", json!({"id": id}), data))
}

pub(super) fn delete_draft(c: &Client, a: DraftArgs) -> Result<Value> {
    let id = a.act.target(c, "deleting the draft")?;
    let data = lifecycle::delete_draft(c, &id, a.content_type)?;
    Ok(out(
        "content.delete-draft",
        "content",
        json!({"id": id}),
        data,
    ))
}

pub(super) fn space_create(c: &Client, a: SpaceCreateArgs) -> Result<Value> {
    if !a.dry_run {
        require_yes(a.yes, "creating the space")?;
    }
    let data = lifecycle::space_create(
        c,
        &lifecycle::NewSpace {
            key: &a.key,
            name: &a.name,
            description: a.description.as_deref(),
            private: a.private,
            dry_run: a.dry_run,
        },
    )?;
    Ok(out("space.create", "space", json!({"key": a.key}), data))
}

pub(super) fn space_update(c: &Client, a: SpaceUpdateArgs) -> Result<Value> {
    let space = a.space.target("changing the space")?;
    let change = lifecycle::SpaceChange {
        name: a.name.as_deref(),
        description: a.description.as_deref(),
        kind: a.kind.as_deref(),
        status: a.status.as_deref(),
        homepage: a.homepage.as_deref(),
    };
    let data = lifecycle::space_update(c, space, &change)?;
    Ok(out("space.update", "space", json!({"id": space}), data))
}

pub(super) fn space_status(c: &Client, a: SpaceRef, archived: bool) -> Result<Value> {
    let (verb, status) = if archived {
        ("archive", "archived")
    } else {
        ("unarchive", "current")
    };
    let space = a.target(if archived {
        "archiving the space"
    } else {
        "unarchiving the space"
    })?;
    let change = lifecycle::SpaceChange {
        status: Some(status),
        ..Default::default()
    };
    let data = lifecycle::space_update(c, space, &change)?;
    Ok(out(
        &format!("space.{verb}"),
        "space",
        json!({"id": space}),
        data,
    ))
}

pub(super) fn space_delete(c: &Client, a: SpaceRef) -> Result<Value> {
    let space = a.target("deleting the space and everything in it, for good")?;
    let data = lifecycle::space_delete(c, space)?;
    Ok(out("space.delete", "space", json!({"id": space}), data))
}

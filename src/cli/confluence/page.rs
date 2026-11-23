//! `confluence page ...`: pages and blog posts.

use super::{Batch, BodyArgs, Paging, Yes, comment, content, send_prepared};
use crate::cli::{Out, util};
use crate::client::{ApiError, Client};
use crate::confluence::{self, ContentType, edits, extras, lifecycle, tree};
use crate::markup::BodyFormat;
use anyhow::Result;
use clap::{ArgGroup, Args, Subcommand};
use serde_json::Value;
use std::time::Duration;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
pub(crate) enum PageCmd {
    /// List the pages or blog posts of a space, or the child pages of a page
    List(ListArgs),
    /// Read pages or blog posts by ID, URL or exact title; the body is
    /// Markdown unless --body-format says otherwise
    Get(GetArgs),
    /// Create a page or blog post
    Create(CreateArgs),
    /// Replace the title or the whole body (needs --if-version)
    ///
    /// A Markdown or plain body is refused when the current body holds
    /// content Markdown cannot express (macros, layouts, ...): use `page
    /// edit` to change only part of it, or pass the body as ADF.
    Update(UpdateArgs),
    /// Change parts of the body with edit operations (needs --if-version)
    ///
    /// --edits takes a JSON array of operations, each {"name": ..., ...}:
    /// replaceNode, insertNodeBefore, insertNodeAfter, deleteNode,
    /// removeNodes, moveNode, appendNodeToEnd, setAttrs (by ADF `localId`,
    /// from `page get --outline` or `--body-format adf`), replaceText
    /// ({find, replace, count}), and replaceSection, appendToSection,
    /// prependToSection, deleteSection, insertSectionAfter (by `heading`).
    /// A `value` is Markdown, or ADF with "format": "adf". Nodes no
    /// operation touches are sent back unchanged.
    Edit(EditArgs),
    /// Move to the trash; with --purge delete from the trash for good; with
    /// --draft delete a draft that was never published
    Delete(DeleteArgs),
    /// Bring a page or blog post back from the trash
    Restore(KindRef),
    /// Move a page under or next to another page
    Move(MoveArgs),
    /// Copy a page under a parent or into a space
    Copy(CopyArgs),
    /// Archive a page
    Archive(ArchiveArgs),
    /// Publish a draft
    Publish(PublishArgs),
    /// Watch a page or blog post (get notified about changes)
    Watch(Target),
    /// Stop watching a page or blog post
    Unwatch(Target),
    /// A page's ancestors and its descendants
    Tree(TreeArgs),
    /// Footer and inline comments
    Comment {
        #[command(subcommand)]
        cmd: comment::CommentCmd,
    },
    /// Attached files
    Attachment {
        #[command(subcommand)]
        cmd: content::AttachmentCmd,
    },
    /// Labels
    Label {
        #[command(subcommand)]
        cmd: content::LabelCmd,
    },
    /// Version history
    Version {
        #[command(subcommand)]
        cmd: content::VersionCmd,
    },
    /// Tasks (checkboxes) written in pages and blog posts
    Task {
        #[command(subcommand)]
        cmd: content::TaskCmd,
    },
    /// The status lozenge shown on a page (content state)
    State {
        #[command(subcommand)]
        cmd: StateCmd,
    },
}

/// The status a page list asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Status {
    #[default]
    Current,
    Draft,
    Archived,
    Trashed,
}

#[derive(Args)]
pub(crate) struct ListArgs {
    /// Space key or ID whose pages to list
    #[arg(long, required_unless_present = "parent", conflicts_with = "parent")]
    space: Option<String>,
    /// List the child pages of this page (ID or URL)
    #[arg(long)]
    parent: Option<String>,
    /// page (default) or blogpost; with --space only
    #[arg(long = "type", value_enum, default_value_t, conflicts_with = "parent")]
    kind: ContentType,
    /// current (default), draft, archived or trashed; with --space only
    #[arg(long, value_enum, default_value_t, conflicts_with = "parent")]
    status: Status,
    #[command(flatten)]
    paging: Paging,
}

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Page or blog post IDs or URLs (or exact titles with --space); `-`
    /// reads them from stdin
    #[arg(value_name = "ID", required = true)]
    ids: Vec<String>,
    /// Space key or ID for looking IDs up by exact title
    #[arg(long)]
    space: Option<String>,
    /// page or blogpost, which skips probing for it
    #[arg(long = "type", value_enum)]
    kind: Option<ContentType>,
    /// The body as markdown (default), adf (JSON with node localIds),
    /// storage (as Confluence stores it) or plain text
    #[arg(long, value_enum, default_value_t)]
    body_format: BodyFormat,
    /// Only the section under this heading, subsections included;
    /// repeatable. Case-insensitive: an exact heading wins, else any heading
    /// containing the text
    #[arg(long = "section", value_name = "HEADING")]
    sections: Vec<String>,
    /// Return the headings (with their localIds and word counts) instead of
    /// the body
    #[arg(long, conflicts_with_all = ["sections", "body_only"])]
    outline: bool,
    /// Start the body at this character (continue a cut body; see
    /// `truncated` in the result)
    #[arg(long, value_name = "N")]
    body_offset: Option<usize>,
    /// Print only the body, as it is (one ID)
    #[arg(long)]
    body_only: bool,
    /// Read the draft instead of the published version
    #[arg(long)]
    draft: bool,
    #[command(flatten)]
    batch: Batch,
}

#[derive(Args)]
pub(crate) struct CreateArgs {
    /// Space key or ID
    #[arg(long)]
    space: String,
    #[arg(long)]
    title: String,
    #[command(flatten)]
    body: BodyArgs,
    /// Parent page (ID or URL, or exact title in --space)
    #[arg(long)]
    parent: Option<String>,
    /// page (default) or blogpost
    #[arg(long = "type", value_enum, default_value_t)]
    kind: ContentType,
    /// Create it as a draft
    #[arg(long)]
    draft: bool,
    /// Only you can see and edit it
    #[arg(long)]
    private: bool,
    /// Page width: narrow, wide or max
    #[arg(long, value_enum)]
    width: Option<lifecycle::Width>,
    /// Title emoji: an emoji, or its hex code point such as 1f44d
    #[arg(long)]
    emoji: Option<String>,
}

/// Flags every version write shares.
#[derive(Args)]
pub(crate) struct VersionArgs {
    /// The `version` of the page you read and changed; a newer version on
    /// the server is a conflict (exit 6) and nothing is written
    #[arg(long, value_name = "N", required = true)]
    if_version: u64,
    /// Message for the new version
    #[arg(long)]
    version_message: Option<String>,
    /// Ask for a minor edit (watchers may still be notified)
    #[arg(long)]
    minor_edit: bool,
    /// Change the draft instead of the published version
    #[arg(long)]
    draft: bool,
    /// page or blogpost, which skips probing for it
    #[arg(long = "type", value_enum)]
    kind: Option<ContentType>,
}

impl VersionArgs {
    fn opts(&self) -> confluence::VersionOpts<'_> {
        confluence::VersionOpts {
            if_version: self.if_version,
            message: self.version_message.as_deref(),
            minor_edit: self.minor_edit,
            draft: self.draft,
            kind: self.kind,
        }
    }
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// Page or blog post ID or URL
    #[arg(value_name = "ID")]
    id: String,
    #[command(flatten)]
    version: VersionArgs,
    /// New title
    #[arg(long)]
    title: Option<String>,
    #[command(flatten)]
    body: BodyArgs,
    /// Replace a body even though Markdown cannot express all of it, losing
    /// that content (needs --yes)
    #[arg(long)]
    allow_lossy: bool,
    /// Confirm --allow-lossy
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
#[command(group(ArgGroup::new("ops").required(true).args(["edits", "edits_file"])))]
pub(crate) struct EditArgs {
    /// Page or blog post ID or URL
    #[arg(value_name = "ID")]
    id: String,
    #[command(flatten)]
    version: VersionArgs,
    /// JSON array of edit operations (`-` reads stdin)
    #[arg(long, allow_hyphen_values = true)]
    edits: Option<String>,
    /// Read the edit operations from a JSON file
    #[arg(long, value_name = "PATH")]
    edits_file: Option<String>,
    /// New title as well
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Page or blog post ID, URL, or exact title with --space
    #[arg(value_name = "ID")]
    id: String,
    /// Space key or ID for looking the ID up by exact title
    #[arg(long)]
    space: Option<String>,
    /// Delete it from the trash for good (it must be in the trash)
    #[arg(long, conflicts_with = "draft")]
    purge: bool,
    /// Delete its draft, which must never have been published
    #[arg(long)]
    draft: bool,
    /// page or blogpost (with --draft), which skips probing for it
    #[arg(long = "type", value_enum)]
    kind: Option<ContentType>,
    #[command(flatten)]
    yes: Yes,
}

#[derive(Args)]
pub(crate) struct Target {
    /// Page or blog post ID, URL, or exact title with --space
    #[arg(value_name = "ID")]
    id: String,
    /// Space key or ID for looking the ID up by exact title
    #[arg(long)]
    space: Option<String>,
}

impl Target {
    fn resolve(&self, c: &Client) -> Result<String> {
        lifecycle::exact_content(c, &self.id, self.space.as_deref())
    }
}

#[derive(Args)]
pub(crate) struct KindRef {
    #[command(flatten)]
    target: Target,
}

#[derive(Args)]
#[command(group(ArgGroup::new("to").required(true).args(["parent", "before", "after"])))]
pub(crate) struct MoveArgs {
    #[command(flatten)]
    target: Target,
    /// Make it the last child of this page
    #[arg(long)]
    parent: Option<String>,
    /// Put it just before this page, under the same parent
    #[arg(long)]
    before: Option<String>,
    /// Put it just after this page, under the same parent
    #[arg(long)]
    after: Option<String>,
}

#[derive(Args)]
#[command(group(ArgGroup::new("to").required(true).args(["parent", "to_space"])))]
pub(crate) struct CopyArgs {
    #[command(flatten)]
    target: Target,
    /// Copy it under this page
    #[arg(long)]
    parent: Option<String>,
    /// Copy it to the top of this space (key or ID)
    #[arg(long = "to-space", value_name = "SPACE")]
    to_space: Option<String>,
    /// Title of the copy
    #[arg(long)]
    title: Option<String>,
    /// Copy the attachments too
    #[arg(long)]
    attachments: bool,
    /// Copy the labels too
    #[arg(long)]
    labels: bool,
    /// Copy the restrictions too
    #[arg(long)]
    permissions: bool,
}

#[derive(Args)]
pub(crate) struct ArchiveArgs {
    #[command(flatten)]
    target: Target,
    /// Wait until Confluence has finished archiving it
    #[arg(long)]
    wait: bool,
    /// Seconds to wait with --wait
    #[arg(long, default_value_t = 120, requires = "wait")]
    timeout: u64,
    #[command(flatten)]
    yes: Yes,
}

#[derive(Args)]
pub(crate) struct PublishArgs {
    #[command(flatten)]
    target: Target,
    /// page or blogpost, which skips probing for it
    #[arg(long = "type", value_enum)]
    kind: Option<ContentType>,
    /// Title to publish it with (default: the draft's)
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
pub(crate) struct TreeArgs {
    /// Page ID or URL
    #[arg(value_name = "ID")]
    id: String,
    /// How many levels below it (default 2, at most 10; at most 500 nodes)
    #[arg(long, default_value_t = 2)]
    depth: usize,
    /// Only this type of descendant, with whatever contains it
    #[arg(long = "type", value_enum)]
    kind: Option<tree::NodeType>,
}

#[derive(Subcommand)]
pub(crate) enum StateCmd {
    /// The status shown on a page
    Get(StateGetArgs),
    /// Set the status (publishes a new version)
    Update(StateSetArgs),
    /// The statuses a page can be given
    List(Target),
}

#[derive(Args)]
pub(crate) struct StateGetArgs {
    #[command(flatten)]
    target: Target,
    /// The draft's status
    #[arg(long)]
    draft: bool,
}

#[derive(Args)]
#[command(group(ArgGroup::new("state").required(true).args(["state_id", "name"])))]
pub(crate) struct StateSetArgs {
    #[command(flatten)]
    at: StateGetArgs,
    /// An existing status, by ID (from `page state list`)
    #[arg(long, conflicts_with_all = ["name", "color"])]
    state_id: Option<u64>,
    /// A status by name (up to 20 characters)
    #[arg(long, requires = "color")]
    name: Option<String>,
    /// Its color, like #ffc400
    #[arg(long, requires = "name")]
    color: Option<String>,
}

pub(super) fn run(c: &Client, cmd: PageCmd) -> Result<Out> {
    let data = match cmd {
        PageCmd::List(a) => list(c, a)?,
        PageCmd::Get(a) => return get(c, a),
        PageCmd::Create(a) => create(c, a)?,
        PageCmd::Update(a) => return update(c, a),
        PageCmd::Edit(a) => return edit(c, a),
        PageCmd::Delete(a) => delete(c, a)?,
        PageCmd::Restore(a) => lifecycle::untrash(c, &a.target.resolve(c)?)?,
        PageCmd::Move(a) => {
            use lifecycle::Position;
            let (position, to) = match (&a.parent, &a.before, &a.after) {
                (Some(p), _, _) => (Position::Append, p),
                (_, Some(b), _) => (Position::Before, b),
                (_, _, Some(x)) => (Position::After, x),
                _ => unreachable!("clap requires one"),
            };
            let id = a.target.resolve(c)?;
            let to = lifecycle::exact_content(c, to, a.target.space.as_deref())?;
            lifecycle::move_content(c, &id, position, &to)?
        }
        PageCmd::Copy(a) => {
            let id = a.target.resolve(c)?;
            let parent = a
                .parent
                .as_deref()
                .map(|p| lifecycle::exact_content(c, p, a.target.space.as_deref()))
                .transpose()?;
            let to = match (&parent, &a.to_space) {
                (Some(p), _) => lifecycle::CopyTo::Parent(p),
                (None, Some(s)) => lifecycle::CopyTo::Space(s),
                _ => unreachable!("clap requires one"),
            };
            let opts = lifecycle::CopyOpts {
                to,
                title: a.title.as_deref(),
                attachments: a.attachments,
                labels: a.labels,
                permissions: a.permissions,
            };
            lifecycle::copy(c, &id, &opts)?
        }
        PageCmd::Archive(a) => {
            let id = a.target.resolve(c)?;
            let wait = a.wait.then(|| lifecycle::Wait {
                timeout: Duration::from_secs(a.timeout),
                interval: Duration::from_secs(2),
            });
            lifecycle::archive(c, &id, wait)?
        }
        PageCmd::Publish(a) => {
            let id = a.target.resolve(c)?;
            lifecycle::publish(c, &id, a.kind, a.title.as_deref())?
        }
        PageCmd::Watch(t) => extras::watch_content(c, &t.resolve(c)?, true)?,
        PageCmd::Unwatch(t) => extras::watch_content(c, &t.resolve(c)?, false)?,
        PageCmd::Tree(a) => {
            let id = confluence::content_id(&a.id)?;
            tree::page(c, &id, a.depth, a.kind)?
        }
        PageCmd::Comment { cmd } => comment::run(c, cmd)?,
        PageCmd::Attachment { cmd } => content::attachment(c, cmd)?,
        PageCmd::Label { cmd } => content::label(c, cmd)?,
        PageCmd::Version { cmd } => content::version(c, cmd)?,
        PageCmd::Task { cmd } => content::task(c, cmd)?,
        PageCmd::State { cmd } => state(c, cmd)?,
    };
    Ok(Out::Data(data))
}

fn list(c: &Client, a: ListArgs) -> Result<Value> {
    match (&a.space, &a.parent) {
        (_, Some(parent)) => a
            .paging
            .collect(250, |cur, n| confluence::children(c, parent, n, cur)),
        (Some(space), None) => {
            let status = format!("{:?}", a.status).to_lowercase();
            a.paging.collect(250, |cur, n| {
                confluence::list(c, space, a.kind, &status, n, cur)
            })
        }
        (None, None) => unreachable!("clap requires --space or --parent"),
    }
}

fn get(c: &Client, a: GetArgs) -> Result<Out> {
    let _ = a.body_offset; // applied by the output shaping
    let opts = confluence::GetOpts {
        format: a.body_format,
        draft: a.draft,
        space: a.space.as_deref(),
        kind: a.kind,
        sections: &a.sections,
        outline: a.outline,
    };
    if a.body_only {
        let ids = util::ids_arg(a.ids)?;
        let [one] = ids.as_slice() else {
            return Err(ApiError::usage("--body-only takes exactly one ID").into());
        };
        let v = confluence::get(c, one, &opts)?;
        let body = match &v["body"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        return Ok(Out::Body { body, full: v });
    }
    Ok(Out::Data(
        a.batch.run(a.ids, |r| confluence::get(c, r, &opts))?,
    ))
}

fn create(c: &Client, a: CreateArgs) -> Result<Value> {
    // Check everything local before the first request.
    let body = a.body.required()?;
    let emoji = a.emoji.as_deref().map(lifecycle::emoji_code).transpose()?;
    let parent = a
        .parent
        .as_deref()
        .map(|p| lifecycle::exact_content(c, p, Some(&a.space)))
        .transpose()?;
    let data = confluence::create(
        c,
        confluence::NewContent {
            space: &a.space,
            content_type: a.kind,
            title: &a.title,
            body: &body,
            parent: parent.as_deref(),
            draft: a.draft,
            private: a.private,
        },
    )?;
    if emoji.is_some() || a.width.is_some() {
        let id = data["id"].as_str().unwrap_or_default();
        // It exists by now: say so, or a retry makes a second one.
        lifecycle::set_appearance(c, a.kind, id, emoji.as_deref(), a.width).map_err(|e| {
            let mut err = ApiError::from_anyhow(&e);
            err.message = format!(
                "created {id}, but could not set its width or emoji (don't create it again): {}",
                err.message
            );
            anyhow::Error::from(err)
        })?;
    }
    Ok(data)
}

fn update(c: &Client, a: UpdateArgs) -> Result<Out> {
    if a.allow_lossy && !a.yes && !crate::cli::dry_run_requested() {
        return Err(util::needs_yes(
            "--allow-lossy (it destroys what Markdown cannot express)",
        ));
    }
    if a.title.is_none() && !a.body.given() {
        return Err(
            ApiError::usage("nothing to update; pass --title or --body/--body-file").into(),
        );
    }
    let body = a.body.parse()?;
    let u = confluence::Update {
        title: a.title.as_deref(),
        body: body.as_ref(),
        check_loss: a.body.needs_loss_check(),
        allow_lossy: a.allow_lossy,
    };
    let p = confluence::prepare_update(c, &a.id, &u, &a.version.opts())?;
    send_prepared(c, &p)
}

fn edit(c: &Client, a: EditArgs) -> Result<Out> {
    let json = util::body_arg(a.edits, a.edits_file)?.unwrap_or_default();
    let ops = edits::parse(&json)?;
    let p = confluence::prepare_edit(c, &a.id, &ops, a.title.as_deref(), &a.version.opts())?;
    send_prepared(c, &p)
}

fn delete(c: &Client, a: DeleteArgs) -> Result<Value> {
    let id = lifecycle::exact_content(c, &a.id, a.space.as_deref())?;
    if a.draft {
        lifecycle::delete_draft(c, &id, a.kind)
    } else if a.purge {
        lifecycle::purge(c, &id)
    } else {
        lifecycle::trash(c, &id)
    }
}

fn state(c: &Client, cmd: StateCmd) -> Result<Value> {
    match cmd {
        StateCmd::Get(a) => extras::state(c, &a.target.resolve(c)?, a.draft),
        StateCmd::Update(a) => {
            let state = extras::state_input(a.state_id, a.name.as_deref(), a.color.as_deref())?;
            let id = a.at.target.resolve(c)?;
            extras::state_set(c, &id, a.at.draft, &state)
        }
        StateCmd::List(t) => extras::states_available(c, &t.resolve(c)?),
    }
}

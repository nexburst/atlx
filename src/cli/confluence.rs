//! `atlx confluence`: clap definitions and dispatch.

use super::Out;
use super::util::{batch, body_arg, deprecated, require_yes};
use crate::client::{self, ApiError};
use crate::confluence;
use crate::markup::TextFormat;
use crate::output::envelope::envelope;
use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

mod comments;
mod extras;
mod lifecycle;

pub(crate) use comments::ConfCommentCmd;

pub(crate) const META: &[crate::schema::CommandMeta] = &[];

#[derive(Subcommand)]
pub(crate) enum ConfCmd {
    /// CQL and text search
    Search {
        #[command(subcommand)]
        cmd: ConfSearchCmd,
    },
    /// Pages and blog posts
    Content {
        #[command(subcommand)]
        cmd: ContentCmd,
    },
    /// Spaces
    Space {
        #[command(subcommand)]
        cmd: SpaceCmd,
    },
    /// Templates for new pages
    Templates {
        #[command(subcommand)]
        cmd: extras::TemplatesCmd,
    },
    /// Page ancestors and descendant tree
    Tree {
        /// Page or space (same as --target)
        #[arg(value_name = "ref", conflicts_with = "target")]
        reference: Option<String>,
        /// Page ID or URL, or space key, ID or URL
        #[arg(long)]
        target: Option<String>,
        /// Max descendant depth (default 2, capped at 10); at most 500 nodes
        #[arg(long, default_value_t = 2)]
        depth: usize,
        /// Only this type, with whatever contains it
        #[arg(long = "type", value_enum)]
        node_type: Option<confluence::tree::NodeType>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfSearchCmd {
    /// Search with CQL
    Query {
        cql_arg: Vec<String>,
        #[arg(long)]
        cql: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Include matched text excerpts
        #[arg(long)]
        excerpt: bool,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Search recent pages and blog posts with natural text
    Text {
        text: Vec<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// YYYY-MM-DD or a duration like 7d, 2w, 1m
        #[arg(long, visible_alias = "since")]
        updated_since: Option<String>,
        #[arg(long)]
        created_since: Option<String>,
        #[arg(long)]
        excerpt: bool,
        #[arg(long)]
        cursor: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ContentCmd {
    /// Read one or more pages/blog posts by ID or URL
    Get {
        refs: Vec<String>,
        #[arg(long = "id")]
        id_flags: Vec<String>,
        /// summary (excerpt), outline (headings) or full (body)
        #[arg(long, value_enum)]
        detail: Option<confluence::Detail>,
        /// Body format for --detail full: html (storage, the default), md,
        /// or adf (atlx extra); url prints only the web address
        #[arg(long, value_enum)]
        format: Option<GetFormat>,
        /// Only the section under this heading, subsections included;
        /// repeatable. Case-insensitive: an exact heading wins, else any
        /// heading containing the text. Defaults to Markdown.
        #[arg(long = "section", value_name = "heading")]
        sections: Vec<String>,
        /// Print only the body
        #[arg(long)]
        body_only: bool,
        /// Fetch the draft version
        #[arg(long)]
        draft: bool,
        /// Space key or ID for resolving non-ID/URL references by exact title
        #[arg(long)]
        reference_space: Option<String>,
        /// Accepted for twg compatibility (metadata is always included)
        #[arg(long)]
        include_metadata: bool,
        /// Fail the batch if any input fails
        #[arg(long)]
        strict: bool,
    },
    /// Create a page or blog post
    Create {
        /// Space key or ID
        #[arg(long)]
        space_id: String,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long, value_enum)]
        content_type: confluence::ContentType,
        #[arg(long)]
        title: String,
        /// Inline body (`-` reads stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
        /// Format of the body (required)
        #[arg(long, value_enum)]
        format: Option<TextFormat>,
        #[arg(long)]
        draft: bool,
        /// Only you can see and edit it
        #[arg(long)]
        private: bool,
        /// narrow, wide or max
        #[arg(long, value_enum)]
        content_width: Option<confluence::lifecycle::Width>,
        /// An emoji, or its hex code point such as 1f44d
        #[arg(long)]
        title_emoji: Option<String>,
        /// Space key or ID for looking up --parent-id by exact title
        #[arg(long)]
        reference_space: Option<String>,
        /// Confirm (required)
        #[arg(long, short)]
        yes: bool,
        /// Confirm you read `confluence content body-formats` (required)
        #[arg(long)]
        ack_body_formats: bool,
    },
    /// Update a page or blog post (new version)
    Update {
        reference: Option<String>,
        #[arg(long)]
        id: Option<String>,
        /// snapshotToken of the read you edited (required); rejects stale edits
        #[arg(long)]
        snapshot_token: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        version_message: Option<String>,
        /// Ask for a minor edit; undocumented in REST v2, so watchers may still be notified
        #[arg(long)]
        minor_edit: bool,
        #[arg(long, conflicts_with_all = ["body_file", "edits", "edits_file"])]
        body: Option<String>,
        #[arg(long, conflicts_with_all = ["edits", "edits_file"])]
        body_file: Option<String>,
        /// JSON array of targeted edit operations (`-` reads stdin)
        #[arg(long, conflicts_with = "edits_file")]
        edits: Option<String>,
        /// Read the edit operations from a JSON file
        #[arg(long)]
        edits_file: Option<String>,
        /// Format of --body or --body-file (required with them)
        #[arg(long, value_enum)]
        format: Option<TextFormat>,
        /// Validate and show the request without writing; with --edits, also
        /// the resulting body and a diff
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        draft: bool,
        /// Confirm (required unless --dry-run)
        #[arg(long, short)]
        yes: bool,
        /// Confirm you read `confluence content body-formats` (required to change the body)
        #[arg(long)]
        ack_body_formats: bool,
    },
    /// List content in a space
    List {
        #[arg(long)]
        space: String,
        #[arg(long, value_enum, default_value_t)]
        content_type: confluence::ContentType,
        #[arg(long, default_value = "current")]
        status: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Comments on a page or blog post
    #[command(visible_alias = "comment")]
    Comments {
        #[command(subcommand)]
        cmd: ConfCommentCmd,
    },
    /// Child pages of a page
    Children {
        reference: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Labels on a page or blog post
    ///
    /// `labels <ref>` (the old atlx form) still lists them directly.
    #[command(visible_alias = "label", args_conflicts_with_subcommands = true)]
    Labels {
        /// Old form: page or blog post ID or URL
        #[arg(hide = true)]
        reference: Option<String>,
        #[arg(long, hide = true, default_value_t = 25)]
        limit: usize,
        #[command(subcommand)]
        cmd: Option<LabelsCmd>,
    },
    /// Delete a page or blog post: to the trash, or for good with --purge
    Delete {
        reference: String,
        /// Confirm (required)
        #[arg(long, short)]
        yes: bool,
        /// Permanently purge instead of moving to trash
        #[arg(long)]
        purge: bool,
    },
    /// Tasks written in pages and blog posts
    Tasks {
        #[command(subcommand)]
        cmd: extras::TasksCmd,
    },
    /// The status shown on content
    Status {
        #[command(subcommand)]
        cmd: extras::StatusCmd,
    },
    /// The statuses content can be given
    ListAvailableStatuses(extras::AvailableArgs),
    /// Watch content
    Watch(extras::WatchArgs),
    /// Stop watching content
    Unwatch(extras::WatchArgs),
    /// Move content under or beside other content
    Move(lifecycle::MoveArgs),
    /// Copy content under a parent or to a space
    Copy(lifecycle::CopyArgs),
    /// Move content to the trash
    Trash(lifecycle::ActArgs),
    /// Bring content back from the trash
    Untrash(lifecycle::ActArgs),
    /// Delete trashed content for good
    Purge(lifecycle::ActArgs),
    /// Archive a page
    Archive(lifecycle::ArchiveArgs),
    /// Publish a draft
    Publish(lifecycle::PublishArgs),
    /// Delete a draft that was never published
    DeleteDraft(lifecycle::DraftArgs),
    /// Version history of a page or blog post
    Versions {
        #[command(subcommand)]
        cmd: VersionsCmd,
    },
    /// Version history (same as `versions`, without restore)
    History {
        #[command(subcommand)]
        cmd: HistoryCmd,
    },
    /// Files attached to a page or blog post
    Attachments {
        #[command(subcommand)]
        cmd: AttachmentsCmd,
    },
    /// Explain the body formats atlx reads and writes (offline)
    BodyFormats {
        /// One format to explain: html, md (markdown), plain or adf
        format: Option<String>,
        /// Show the formats one content type supports
        #[arg(long, value_enum)]
        content_type: Option<confluence::formats::GuideType>,
        /// Accepted for twg compatibility; atlx's guide has no linked pages
        #[arg(long, num_args = 1.., value_name = "path")]
        reference: Vec<String>,
    },
}

/// `--format` on `content get`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum GetFormat {
    Html,
    #[value(alias = "markdown")]
    Md,
    /// The ADF document as JSON, with node `localId`s for `--edits` (atlx
    /// extra)
    #[value(hide = true)]
    Adf,
    /// Only the web URL
    Url,
}

/// The page or blog post that a versions/labels/attachments command acts on.
#[derive(Args)]
pub(crate) struct Target {
    /// Page or blog post ID, URL, or exact title (with --reference-space)
    #[arg(long)]
    id: String,
    /// Space key or ID for looking up --id by exact title
    #[arg(long)]
    reference_space: Option<String>,
}

#[derive(Args)]
pub(crate) struct VersionListArgs {
    #[command(flatten)]
    target: Target,
    /// Versions per page, newest first (1-250)
    #[arg(long, default_value_t = 25)]
    limit: usize,
    #[arg(long)]
    cursor: Option<String>,
}

#[derive(Args)]
pub(crate) struct VersionGetArgs {
    #[command(flatten)]
    target: Target,
    /// Version number
    #[arg(long)]
    at_version: u64,
    /// Also return that version's body: md (markdown) or html
    #[arg(long, value_enum)]
    format: Option<confluence::ReadFormat>,
}

#[derive(Args)]
pub(crate) struct VersionDiffArgs {
    #[command(flatten)]
    target: Target,
    /// Older version number
    #[arg(long)]
    from: u64,
    /// Newer version number
    #[arg(long)]
    to: u64,
    /// Compare the bodies as md (markdown, default) or html
    #[arg(long, value_enum, default_value = "md")]
    format: confluence::ReadFormat,
}

#[derive(Subcommand)]
pub(crate) enum VersionsCmd {
    /// List versions, newest first
    List(VersionListArgs),
    /// One version's details, and its body with --format
    Get(VersionGetArgs),
    /// Unified diff between two versions' bodies
    Diff(VersionDiffArgs),
    /// Make an old version the latest again (adds a new version)
    Restore {
        #[command(flatten)]
        target: Target,
        /// Version number to bring back
        #[arg(long)]
        at_version: u64,
        /// Message for the new version
        #[arg(long)]
        message: Option<String>,
        /// Bring back that version's title too
        #[arg(long)]
        restore_title: bool,
        /// Confirm the restore (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum HistoryCmd {
    /// List versions, newest first
    List(VersionListArgs),
    /// One version's details, and its body with --format
    Get(VersionGetArgs),
    /// Unified diff between two versions' bodies
    Diff(VersionDiffArgs),
}

#[derive(Subcommand)]
pub(crate) enum AttachmentsCmd {
    /// List attachments
    List {
        #[command(flatten)]
        target: Target,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        /// created-date or modified-date; prefix - for descending
        #[arg(long, allow_hyphen_values = true)]
        sort: Option<String>,
        /// current (default), archived or trashed
        #[arg(long, default_value = "current")]
        status: String,
        /// Only attachments of this media type, e.g. image/png
        #[arg(long)]
        media_type: Option<String>,
        /// Only attachments with this file name
        #[arg(long)]
        filename: Option<String>,
    },
    /// One attachment's metadata
    Get {
        /// Attachment ID (att12345) or URL
        #[arg(long)]
        attachment_id: String,
        /// Attachment version (default: latest)
        #[arg(long)]
        at_version: Option<u64>,
    },
    /// Save an attachment to a local file
    Download {
        /// Attachment ID (att12345) or URL
        #[arg(long)]
        attachment_id: String,
        /// File to write (default: the attachment's name, in the current
        /// directory)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Attachment version (default: latest)
        #[arg(long)]
        at_version: Option<u64>,
        /// Replace the file if it exists
        #[arg(long)]
        force: bool,
    },
    /// Attach a local file; a file with the same name gets a new version
    Upload {
        #[command(flatten)]
        target: Target,
        /// File to upload; repeat for several (atlx extra)
        #[arg(long, required = true)]
        file: Vec<String>,
        /// Comment for this attachment version
        #[arg(long)]
        comment: Option<String>,
        /// Don't notify watchers of the new version (atlx extra)
        #[arg(long)]
        minor_edit: bool,
        /// Confirm the upload (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Delete an attachment (moves it to the trash)
    Delete {
        /// Attachment ID (att12345) or URL
        #[arg(long)]
        attachment_id: String,
        /// Delete for good; it must be in the trash already (atlx extra)
        #[arg(long)]
        purge: bool,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum LabelsCmd {
    /// List the labels on a piece of content
    List {
        #[command(flatten)]
        target: Target,
        /// Content type, which skips probing for it
        #[arg(long, value_enum)]
        content_type: Option<confluence::labels::LabelTarget>,
    },
    /// Add labels to a page or blog post
    Add {
        #[command(flatten)]
        target: Target,
        /// Label name; repeatable, and commas separate several
        #[arg(long)]
        label: Vec<String>,
        /// Label prefix
        #[arg(long, default_value = "global")]
        prefix: String,
    },
    /// Remove labels from a page or blog post
    Remove {
        #[command(flatten)]
        target: Target,
        /// Label name; repeatable, and commas separate several
        #[arg(long)]
        label: Vec<String>,
        /// Label prefix
        #[arg(long, default_value = "global")]
        prefix: String,
        /// Confirm the removal (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Watch a label: get notified about content that gets it
    Watch {
        /// Label name
        #[arg(long)]
        name: String,
        /// Confirm (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Stop watching a label
    Unwatch {
        /// Label name
        #[arg(long)]
        name: String,
        /// Confirm (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum SpaceCmd {
    /// List spaces
    List {
        /// global or personal
        #[arg(long = "type")]
        kind: Option<String>,
        /// current or archived
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated space keys
        #[arg(long)]
        keys: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        /// Include each space's labels
        #[arg(long)]
        labels: bool,
    },
    /// Get a space by key or ID
    Get {
        /// Space key or ID
        #[arg(conflicts_with = "key")]
        identifier: Option<String>,
        /// Same as the positional key
        #[arg(long, hide = true)]
        key: Option<String>,
        /// Include the space's labels
        #[arg(long)]
        labels: bool,
    },
    /// Current user's personal space
    Me,
    /// Create a space
    Create(lifecycle::SpaceCreateArgs),
    /// Change a space
    Update(lifecycle::SpaceUpdateArgs),
    /// Archive a space
    Archive(lifecycle::SpaceRef),
    /// Bring an archived space back
    Unarchive(lifecycle::SpaceRef),
    /// Delete a space and everything in it, for good
    Delete(lifecycle::SpaceRef),
    /// Watch a space
    Watch(extras::SpaceWatchArgs),
    /// Stop watching a space
    Unwatch(extras::SpaceWatchArgs),
    /// Space permissions
    Permission {
        #[command(subcommand)]
        cmd: extras::PermissionCmd,
    },
}

pub(crate) fn confluence_cmd(c: &client::Client, cmd: ConfCmd) -> Result<Out> {
    let data = match cmd {
        ConfCmd::Search {
            cmd:
                ConfSearchCmd::Query {
                    cql_arg,
                    cql,
                    limit,
                    excerpt,
                    cursor,
                },
        } => {
            let cql = cql.or_else(|| (!cql_arg.is_empty()).then(|| cql_arg.join(" ")));
            let Some(cql) = cql else {
                bail!("pass --cql or a CQL argument")
            };
            let data = confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?;
            envelope("confluence.search.query", "confluence.search-result")
                .request(json!({"cql": cql, "limit": limit, "cursor": cursor}))
                .next_cursor(data["nextCursor"].as_str())
                .data(data)
        }
        ConfCmd::Search {
            cmd:
                ConfSearchCmd::Text {
                    text,
                    query,
                    limit,
                    updated_since,
                    created_since,
                    excerpt,
                    cursor,
                },
        } => {
            let text = query.or_else(|| (!text.is_empty()).then(|| text.join(" ")));
            let Some(text) = text else {
                bail!("pass search text or --query")
            };
            let cql =
                confluence::text_cql(&text, updated_since.as_deref(), created_since.as_deref())?;
            let data = confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?;
            envelope("confluence.search.text", "confluence.search-result")
                .request(json!({"query": text, "cql": cql, "limit": limit, "cursor": cursor}))
                .next_cursor(data["nextCursor"].as_str())
                .data(data)
        }
        ConfCmd::Content { cmd } => match cmd {
            ContentCmd::Get {
                refs,
                id_flags,
                detail,
                format,
                sections,
                body_only,
                draft,
                reference_space,
                include_metadata: _,
                strict,
            } => {
                let inputs: Vec<String> = refs.into_iter().chain(id_flags).collect();
                if format == Some(GetFormat::Url) {
                    if body_only || !sections.is_empty() || detail.is_some() {
                        return Err(ApiError::usage(
                            "--format url returns only the web address; drop --detail, --section and --body-only",
                        )
                        .into());
                    }
                    let opts = confluence::GetOpts {
                        detail: confluence::Detail::Summary,
                        format: confluence::ReadFormat::Html,
                        draft,
                        reference_space: reference_space.as_deref(),
                        sections: &[],
                    };
                    let request = json!({
                        "ids": if inputs.len() == 1 { json!(inputs[0]) } else { json!(inputs) },
                        "format": "url",
                    });
                    let data = batch(inputs, strict, 1, |r| confluence::get_url(c, r, &opts))?;
                    let text = match data["items"].as_array() {
                        Some(items) => items
                            .iter()
                            .map(|i| i["data"]["url"].as_str().unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        None => data["url"].as_str().unwrap_or_default().to_string(),
                    };
                    return Ok(Out::Doc {
                        text,
                        full: envelope("confluence.content.get", "confluence.content")
                            .request(request)
                            .data(data),
                    });
                }
                let format = format.map(|f| match f {
                    GetFormat::Md => confluence::ReadFormat::Md,
                    GetFormat::Adf => confluence::ReadFormat::Adf,
                    _ => confluence::ReadFormat::Html,
                });
                // --section implies --detail full, in Markdown unless asked.
                let detail = if body_only || !sections.is_empty() {
                    if detail.is_some_and(|d| d != confluence::Detail::Full) {
                        return Err(ApiError::usage(
                            "--section and --body-only return a body; use them without --detail or with --detail full",
                        )
                        .into());
                    }
                    confluence::Detail::Full
                } else {
                    detail.unwrap_or_default()
                };
                let format = format.unwrap_or(if sections.is_empty() {
                    confluence::ReadFormat::Html
                } else {
                    confluence::ReadFormat::Md
                });
                let opts = confluence::GetOpts {
                    detail,
                    format,
                    draft,
                    reference_space: reference_space.as_deref(),
                    sections: &sections,
                };
                let mut request = json!({
                    "ids": if inputs.len() == 1 { json!(inputs[0]) } else { json!(inputs) },
                    "detail": format!("{:?}", opts.detail).to_lowercase(),
                    "format": format!("{:?}", opts.format).to_lowercase(),
                });
                if !sections.is_empty() {
                    request["sections"] = json!(sections);
                }
                let get = envelope("confluence.content.get", "confluence.content").request(request);
                if body_only {
                    let [one] = inputs.as_slice() else {
                        bail!("--body-only takes exactly one ID")
                    };
                    let v = confluence::get(c, one, &opts)?;
                    let body = v["body"]["value"].as_str().unwrap_or_default().to_string();
                    return Ok(Out::Body {
                        body,
                        full: get.data(v),
                    });
                }
                get.data(batch(inputs, strict, 1, |r| confluence::get(c, r, &opts))?)
            }
            ContentCmd::Create {
                space_id,
                parent_id,
                content_type,
                title,
                body,
                body_file,
                format,
                draft,
                private,
                content_width,
                title_emoji,
                reference_space,
                yes,
                ack_body_formats,
            } => {
                use confluence::lifecycle as life;
                if body.is_none() && body_file.is_none() {
                    return Err(ApiError::usage(
                        "pass --body or --body-file: pages and blog posts need a body",
                    )
                    .into());
                }
                let format = require_format(format)?;
                require_ack(ack_body_formats)?;
                require_yes(yes, "creating the content")?;
                let emoji = title_emoji.as_deref().map(life::emoji_code).transpose()?;
                let parent_id = match (parent_id, &reference_space) {
                    (Some(p), Some(_)) => {
                        Some(life::exact_content(c, &p, reference_space.as_deref())?)
                    }
                    (p, _) => p,
                };
                let body = body_arg(body, body_file)?.unwrap_or_default();
                let data = confluence::create(
                    c,
                    confluence::NewContent {
                        space: &space_id,
                        content_type,
                        title: &title,
                        body: &body,
                        format,
                        parent: parent_id.as_deref(),
                        draft,
                        private,
                    },
                )?;
                if emoji.is_some() || content_width.is_some() {
                    let id = data["id"].as_str().unwrap_or_default();
                    // It exists by now: say so, or a retry makes a second one.
                    life::set_appearance(c, content_type, id, emoji.as_deref(), content_width)
                        .map_err(|e| {
                            let mut err = ApiError::from_anyhow(&e);
                            err.message = format!(
                                "created {id}, but could not set its width or emoji (don't create it again): {}",
                                err.message
                            );
                            anyhow::Error::from(err)
                        })?;
                }
                envelope("confluence.content.create", "confluence.content")
                    .request(json!({"spaceId": space_id, "title": title, "parentId": parent_id}))
                    .data(data)
            }
            ContentCmd::Update {
                reference,
                id,
                snapshot_token,
                title,
                version_message,
                minor_edit,
                body,
                body_file,
                edits,
                edits_file,
                format,
                dry_run,
                draft,
                yes,
                ack_body_formats,
            } => {
                let Some(reference) = reference.or(id) else {
                    bail!("pass a content ID/URL or --id")
                };
                let ops = match body_arg(edits, edits_file)? {
                    Some(json) => Some(confluence::edits::parse(&json)?),
                    None => None,
                };
                let has_body = body.is_some() || body_file.is_some();
                if title.is_none() && !has_body && ops.is_none() {
                    return Err(ApiError::usage(
                        "nothing to update; pass --title, --body/--body-file or --edits/--edits-file",
                    )
                    .into());
                }
                // twg's order: format, acknowledgement, token, confirmation.
                let format = if has_body {
                    require_format(format)?
                } else {
                    format.unwrap_or_default()
                };
                if has_body || ops.is_some() {
                    require_ack(ack_body_formats)?;
                }
                if snapshot_token.is_none() {
                    return Err(ApiError::usage(
                        "pass --snapshot-token with the snapshotToken of the read you edited",
                    )
                    .into());
                }
                if !dry_run {
                    require_yes(yes, "updating the content")?;
                }
                let body = body_arg(body, body_file)?;
                let data = confluence::update(
                    c,
                    &reference,
                    confluence::Edit {
                        title: title.as_deref(),
                        body: body.as_deref().map(|b| (b, format)),
                        version_message: version_message.as_deref(),
                        minor_edit,
                        snapshot_token: snapshot_token.as_deref(),
                        dry_run,
                        draft,
                        edits: ops.as_deref(),
                    },
                )?;
                let mut request =
                    json!({"id": reference, "snapshotToken": snapshot_token, "dryRun": dry_run});
                if let Some(ops) = &ops {
                    request["edits"] = ops.len().into();
                }
                envelope("confluence.content.update", "confluence.content")
                    .request(request)
                    .data(data)
            }
            ContentCmd::List {
                space,
                content_type,
                status,
                limit,
                cursor,
            } => {
                let data =
                    confluence::list(c, &space, content_type, &status, limit, cursor.as_deref())?;
                envelope("confluence.content.list", "confluence.content")
                    .request(
                        json!({"space": space, "status": status, "limit": limit, "cursor": cursor}),
                    )
                    .next_cursor(data["nextCursor"].as_str())
                    .data(data)
            }
            ContentCmd::Comments { cmd } => comments::run(c, cmd)?,
            ContentCmd::Children {
                reference,
                limit,
                cursor,
            } => {
                let data = confluence::children(c, &reference, limit, cursor.as_deref())?;
                envelope("confluence.content.children", "confluence.content")
                    .request(json!({"id": reference, "limit": limit, "cursor": cursor}))
                    .next_cursor(data["nextCursor"].as_str())
                    .data(data)
            }
            ContentCmd::Labels { cmd: Some(cmd), .. } => labels_cmd(c, cmd)?,
            ContentCmd::Tasks { cmd } => extras::tasks(c, cmd)?,
            ContentCmd::Status { cmd } => extras::status(c, cmd)?,
            ContentCmd::ListAvailableStatuses(a) => extras::available(c, a)?,
            ContentCmd::Watch(a) => extras::watch(c, a, true)?,
            ContentCmd::Unwatch(a) => extras::watch(c, a, false)?,
            ContentCmd::Labels {
                reference: Some(reference),
                limit,
                cmd: None,
            } => {
                deprecated(
                    "confluence content labels <ref>",
                    "confluence content labels list --id <ref>",
                );
                let data = confluence::labels(c, &reference, limit)?;
                envelope("confluence.content.labels.list", "confluence.label")
                    .request(json!({"id": reference}))
                    .next_cursor(data["nextCursor"].as_str())
                    .data(data)
            }
            ContentCmd::Versions { cmd } => match cmd {
                VersionsCmd::List(a) => versions_list(c, a, "versions")?,
                VersionsCmd::Get(a) => versions_get(c, a, "versions")?,
                VersionsCmd::Diff(a) => versions_diff(c, a, "versions")?,
                VersionsCmd::Restore {
                    target,
                    at_version,
                    message,
                    restore_title,
                    yes,
                } => {
                    require_yes(yes, "restoring an old version")?;
                    let data = confluence::history::restore(
                        c,
                        &target.id,
                        target.reference_space.as_deref(),
                        at_version,
                        message.as_deref(),
                        restore_title,
                    )?;
                    envelope("confluence.content.versions.restore", "confluence.version")
                        .request(json!({
                            "id": target.id,
                            "atVersion": at_version,
                            "message": message,
                            "restoreTitle": restore_title,
                        }))
                        .data(data)
                }
            },
            ContentCmd::History { cmd } => match cmd {
                HistoryCmd::List(a) => versions_list(c, a, "history")?,
                HistoryCmd::Get(a) => versions_get(c, a, "history")?,
                HistoryCmd::Diff(a) => versions_diff(c, a, "history")?,
            },
            ContentCmd::Attachments { cmd } => attachments_cmd(c, cmd)?,
            ContentCmd::BodyFormats {
                format,
                content_type,
                reference,
            } => return body_formats(format.as_deref(), content_type, &reference),
            ContentCmd::Labels {
                reference: None, ..
            } => {
                return Err(ApiError::usage(
                    "pass a subcommand: confluence content labels list --id <ref>",
                )
                .into());
            }
            ContentCmd::Move(a) => lifecycle::move_content(c, a)?,
            ContentCmd::Copy(a) => lifecycle::copy(c, a)?,
            ContentCmd::Trash(a) => lifecycle::trash(c, a)?,
            ContentCmd::Untrash(a) => lifecycle::untrash(c, a)?,
            ContentCmd::Purge(a) => lifecycle::purge(c, a)?,
            ContentCmd::Archive(a) => lifecycle::archive(c, a)?,
            ContentCmd::Publish(a) => lifecycle::publish(c, a)?,
            ContentCmd::DeleteDraft(a) => lifecycle::delete_draft(c, a)?,
            ContentCmd::Delete {
                reference,
                yes,
                purge,
            } => {
                let action = if purge {
                    "deleting the content for good"
                } else {
                    "deleting the content"
                };
                require_yes(yes, action)?;
                envelope("confluence.content.delete", "confluence.content")
                    .request(json!({"id": reference, "purge": purge}))
                    .data(confluence::delete(c, &reference, purge)?)
            }
        },
        ConfCmd::Space {
            cmd:
                SpaceCmd::List {
                    kind,
                    status,
                    keys,
                    limit,
                    cursor,
                    labels,
                },
        } => {
            let mut v = confluence::spaces(
                c,
                kind.as_deref(),
                status.as_deref(),
                keys.as_deref(),
                limit,
                cursor.as_deref(),
            )?;
            if labels {
                confluence::add_space_labels(c, &mut v)?;
            }
            envelope("confluence.space.list", "confluence.space")
                .request(json!({"type": kind, "status": status, "keys": keys, "limit": limit, "cursor": cursor}))
                .next_cursor(v["nextCursor"].as_str())
                .data(v)
        }
        ConfCmd::Space {
            cmd:
                SpaceCmd::Get {
                    identifier,
                    key,
                    labels,
                },
        } => {
            let Some(identifier) = identifier.or(key) else {
                return Err(ApiError::usage("pass a space key or ID").into());
            };
            let mut v = confluence::space(c, &identifier)?;
            if labels {
                confluence::add_space_labels(c, &mut v)?;
            }
            envelope("confluence.space.get", "confluence.space")
                .request(json!({"id": identifier, "labels": labels}))
                .data(v)
        }
        ConfCmd::Space { cmd: SpaceCmd::Me } => envelope("confluence.space.me", "confluence.space")
            .request(json!({}))
            .data(confluence::space_me(c)?),
        ConfCmd::Space {
            cmd: SpaceCmd::Create(a),
        } => lifecycle::space_create(c, a)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Update(a),
        } => lifecycle::space_update(c, a)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Archive(a),
        } => lifecycle::space_status(c, a, true)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Unarchive(a),
        } => lifecycle::space_status(c, a, false)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Delete(a),
        } => lifecycle::space_delete(c, a)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Watch(a),
        } => extras::space_watch(c, a, true)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Unwatch(a),
        } => extras::space_watch(c, a, false)?,
        ConfCmd::Space {
            cmd: SpaceCmd::Permission { cmd },
        } => extras::permission(c, cmd)?,
        ConfCmd::Templates { cmd } => extras::templates(c, cmd)?,
        ConfCmd::Tree {
            reference,
            target,
            depth,
            node_type,
        } => {
            let Some(reference) = target.or(reference) else {
                return Err(ApiError::usage("pass a page or space, or --target").into());
            };
            envelope("confluence.tree", "confluence.tree")
                .request(json!({"target": reference, "depth": depth}))
                .data(confluence::tree::tree(c, &reference, depth, node_type)?)
        }
    };
    Ok(Out::Data(data))
}

/// Commands that need no site or credentials, answered before a client is
/// built: `content body-formats`.
pub(crate) fn offline(cmd: &ConfCmd) -> Option<Result<Out>> {
    match cmd {
        ConfCmd::Content {
            cmd:
                ContentCmd::BodyFormats {
                    format,
                    content_type,
                    reference,
                },
        } => Some(body_formats(format.as_deref(), *content_type, reference)),
        _ => None,
    }
}

/// `content body-formats`: printed without an envelope; text
/// output is the guide itself.
fn body_formats(
    format: Option<&str>,
    content_type: Option<confluence::formats::GuideType>,
    reference: &[String],
) -> Result<Out> {
    if !reference.is_empty() {
        return Err(ApiError::usage(
            "atlx's body-format guide has no linked documents; omit --reference",
        )
        .into());
    }
    let data = confluence::formats::guide(format, content_type)?;
    let text = data["doc"]
        .as_str()
        .unwrap_or_default()
        .trim_end()
        .to_string();
    Ok(Out::Doc { text, full: data })
}

fn labels_cmd(c: &client::Client, cmd: LabelsCmd) -> Result<Value> {
    use confluence::labels;
    Ok(match cmd {
        LabelsCmd::List {
            target,
            content_type,
        } => {
            let data = labels::list(
                c,
                &target.id,
                target.reference_space.as_deref(),
                content_type,
                250,
            )?;
            envelope("confluence.content.labels.list", "confluence.label")
                .request(json!({"id": target.id, "contentType": content_type.map(|t| format!("{t:?}").to_lowercase())}))
                .next_cursor(data["nextCursor"].as_str())
                .data(data)
        }
        LabelsCmd::Add {
            target,
            label,
            prefix,
        } => envelope("confluence.content.labels.add", "confluence.label")
            .request(json!({"id": target.id, "labels": label, "prefix": prefix}))
            .data(labels::add(
                c,
                &target.id,
                target.reference_space.as_deref(),
                &label,
                &prefix,
            )?),
        LabelsCmd::Remove {
            target,
            label,
            prefix,
            yes,
        } => {
            require_yes(yes, "removing labels")?;
            envelope("confluence.content.labels.remove", "confluence.label")
                .request(json!({"id": target.id, "labels": label, "prefix": prefix}))
                .data(labels::remove(
                    c,
                    &target.id,
                    target.reference_space.as_deref(),
                    &label,
                    &prefix,
                )?)
        }
        LabelsCmd::Watch { name, yes } => {
            require_yes(yes, "watching the label")?;
            envelope("confluence.content.labels.watch", "confluence.label")
                .request(json!({"name": name}))
                .data(labels::watch(c, &name, true)?)
        }
        LabelsCmd::Unwatch { name, yes } => {
            require_yes(yes, "unwatching the label")?;
            envelope("confluence.content.labels.unwatch", "confluence.label")
                .request(json!({"name": name}))
                .data(labels::watch(c, &name, false)?)
        }
    })
}

fn versions_list(c: &client::Client, a: VersionListArgs, group: &str) -> Result<Value> {
    let data = confluence::history::list(
        c,
        &a.target.id,
        a.target.reference_space.as_deref(),
        a.limit,
        a.cursor.as_deref(),
    )?;
    Ok(envelope(
        &format!("confluence.content.{group}.list"),
        "confluence.version",
    )
    .request(json!({"id": a.target.id, "limit": a.limit, "cursor": a.cursor}))
    .next_cursor(data["nextCursor"].as_str())
    .data(data))
}

fn versions_get(c: &client::Client, a: VersionGetArgs, group: &str) -> Result<Value> {
    let data = confluence::history::get(
        c,
        &a.target.id,
        a.target.reference_space.as_deref(),
        a.at_version,
        a.format,
    )?;
    Ok(envelope(
        &format!("confluence.content.{group}.get"),
        "confluence.version",
    )
    .request(json!({
        "id": a.target.id,
        "atVersion": a.at_version,
        "format": a.format.map(|f| format!("{f:?}").to_lowercase()),
    }))
    .data(data))
}

fn versions_diff(c: &client::Client, a: VersionDiffArgs, group: &str) -> Result<Value> {
    let data = confluence::history::diff(
        c,
        &a.target.id,
        a.target.reference_space.as_deref(),
        a.from,
        a.to,
        a.format,
    )?;
    Ok(envelope(
        &format!("confluence.content.{group}.diff"),
        "confluence.version-diff",
    )
    .request(json!({
        "id": a.target.id,
        "from": a.from,
        "to": a.to,
        "format": format!("{:?}", a.format).to_lowercase(),
    }))
    .data(data))
}

fn attachments_cmd(c: &client::Client, cmd: AttachmentsCmd) -> Result<Value> {
    use confluence::attachments;
    const KIND: &str = "confluence.attachment";
    Ok(match cmd {
        AttachmentsCmd::List {
            target,
            limit,
            cursor,
            sort,
            status,
            media_type,
            filename,
        } => {
            let data = attachments::list(
                c,
                &target.id,
                target.reference_space.as_deref(),
                &attachments::ListOpts {
                    limit,
                    cursor: cursor.as_deref(),
                    sort: sort.as_deref(),
                    status: Some(status.as_str()),
                    media_type: media_type.as_deref(),
                    filename: filename.as_deref(),
                },
            )?;
            envelope("confluence.content.attachments.list", KIND)
                .request(json!({
                    "id": target.id,
                    "limit": limit,
                    "cursor": cursor,
                    "sort": sort,
                    "status": status,
                    "mediaType": media_type,
                    "filename": filename,
                }))
                .next_cursor(data["nextCursor"].as_str())
                .data(data)
        }
        AttachmentsCmd::Get {
            attachment_id,
            at_version,
        } => envelope("confluence.content.attachments.get", KIND)
            .request(json!({"attachmentId": attachment_id, "atVersion": at_version}))
            .data(attachments::get(c, &attachment_id, at_version)?),
        AttachmentsCmd::Download {
            attachment_id,
            out,
            at_version,
            force,
        } => envelope("confluence.content.attachments.download", KIND)
            .request(json!({
                "attachmentId": attachment_id,
                "out": out.as_ref().map(|p| p.display().to_string()),
                "atVersion": at_version,
                "force": force,
            }))
            .data(attachments::download(
                c,
                &attachment_id,
                out.as_deref(),
                at_version,
                force,
            )?),
        AttachmentsCmd::Upload {
            target,
            file,
            comment,
            minor_edit,
            yes,
        } => {
            require_yes(yes, "uploading the attachment")?;
            envelope("confluence.content.attachments.upload", KIND)
                .request(json!({
                    "id": target.id,
                    "file": if file.len() == 1 { json!(file[0]) } else { json!(file) },
                    "comment": comment,
                }))
                // One at a time: parallel writes to one page's attachments conflict.
                .data(batch(file, false, 1, |f| {
                    attachments::upload(
                        c,
                        &target.id,
                        target.reference_space.as_deref(),
                        Path::new(f),
                        comment.as_deref(),
                        minor_edit,
                    )
                })?)
        }
        AttachmentsCmd::Delete {
            attachment_id,
            purge,
            yes,
        } => {
            require_yes(yes, "deleting the attachment")?;
            envelope("confluence.content.attachments.delete", KIND)
                .request(json!({"attachmentId": attachment_id}))
                .data(attachments::delete(c, &attachment_id, purge)?)
        }
    })
}

/// `--id`, or the old positional reference (with a deprecation note).
fn content_ref(
    positional: Option<String>,
    id: Option<String>,
    old: &str,
    new: &str,
) -> Result<String> {
    match (id, positional) {
        (Some(id), _) => Ok(id),
        (None, Some(r)) => {
            deprecated(old, new);
            Ok(r)
        }
        (None, None) => Err(ApiError::usage("pass --id with a page or blog post ID or URL").into()),
    }
}

fn require_format(format: Option<TextFormat>) -> Result<TextFormat> {
    format.ok_or_else(|| ApiError::usage("pass --format to say what the body is written in").into())
}

fn require_ack(acknowledged: bool) -> Result<()> {
    if !acknowledged {
        return Err(ApiError::usage(
            "read `atlx confluence content body-formats`, then pass --ack-body-formats",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::tests::shell_words;
    use crate::cli::{Cli, Cmd};
    use clap::Parser;

    #[test]
    fn parses_confluence_content_get_strict_and_space_list_cursor() {
        let cli = Cli::try_parse_from(shell_words("atlx confluence content get 123 456 --strict"))
            .unwrap();
        match cli.cmd {
            Cmd::Confluence {
                cmd:
                    ConfCmd::Content {
                        cmd: ContentCmd::Get { refs, strict, .. },
                    },
            } => {
                assert_eq!(refs, vec!["123", "456"]);
                assert!(strict);
            }
            _ => panic!("unexpected parsed command"),
        }

        let cli = Cli::try_parse_from(shell_words(
            "atlx confluence space list --limit 10 --cursor cur%3D",
        ))
        .unwrap();
        match cli.cmd {
            Cmd::Confluence {
                cmd:
                    ConfCmd::Space {
                        cmd: SpaceCmd::List { limit, cursor, .. },
                    },
            } => {
                assert_eq!(limit, 10);
                assert_eq!(cursor.as_deref(), Some("cur%3D"));
            }
            _ => panic!("unexpected parsed command"),
        }
    }

    #[test]
    fn parses_content_and_space_lifecycle() {
        for ok in [
            "atlx confluence content move 12345 --parent-id 67890 --yes",
            "atlx confluence content move 12345 --target 67890 --position before --yes",
            "atlx confluence content copy 12345 --parent-content-id 67890 --title Copy --yes",
            "atlx confluence content copy 12345 --destination-space ENG --copy-labels --yes",
            "atlx confluence content trash 12345 --yes",
            "atlx confluence content untrash 12345 --yes",
            "atlx confluence content purge 12345 --yes",
            "atlx confluence content archive 12345 --wait --yes",
            "atlx confluence content publish 12345 --content-type page --title T --yes",
            "atlx confluence content delete-draft 12345 --yes",
            "atlx confluence content create --space-id ENG --content-type page --title T --private --content-width wide --title-emoji 1f44d",
            "atlx confluence space create --key NEW --name New --dry-run",
            "atlx confluence space update ENG --name New --yes",
            "atlx confluence space archive --key ENG --yes",
            "atlx confluence space unarchive --id 99 --yes",
            "atlx confluence space delete ENG --yes",
            "atlx confluence tree --target ENG --type whiteboard --depth 10",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx confluence content move 1 --parent-id 2 --before 3",
            "atlx confluence content copy 1 --yes",
            "atlx confluence space delete ENG --key OTHER",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_comment_threads() {
        for ok in [
            "atlx confluence content comments list --id 12345 --sort -created-date --status current --include-replies true",
            "atlx confluence content comments list --id 12345 --comment-type inline --resolution-status open --content-type page",
            "atlx confluence content comments get --comment-id 777 --include-replies false",
            "atlx confluence content comments create --id 12345 --body x --text-selection words --text-selection-match-index 1",
            "atlx confluence content comments reply --comment-id 777 --body Thanks",
            "atlx confluence content comments update --comment-id 777 --body x --expected-version 2",
            "atlx confluence content comments delete --comment-id 777 --yes",
            "atlx confluence content comments resolve --comment-id 777",
            "atlx confluence content comments reopen --comment-id 777 --comment-type inline",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx confluence content comments create --id 12345 --body x --text-selection-match-count 2",
            "atlx confluence content comments list --id 12345 --status trashed",
            "atlx confluence content comments reply --body x",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_versions_labels_attachments_and_formats() {
        for ok in [
            "atlx confluence content get 12345 --format url",
            "atlx confluence content versions list --id 12345 --limit 5 --cursor c",
            "atlx confluence content versions get --id 12345 --at-version 2 --format md",
            "atlx confluence content versions diff --id 12345 --from 1 --to 2 --format html",
            "atlx confluence content versions restore --id 12345 --at-version 2 --restore-title --message undo --yes",
            "atlx confluence content history list --id Runbook --reference-space ENG",
            "atlx confluence content history get --id 12345 --at-version 2",
            "atlx confluence content history diff --id 12345 --from 1 --to 2",
            "atlx confluence content labels list --id 12345 --content-type whiteboard",
            "atlx confluence content labels add --id 12345 --label a,b --label c --prefix global",
            "atlx confluence content labels remove --id 12345 --label a --yes",
            "atlx confluence content labels watch --name runbook --yes",
            "atlx confluence content labels unwatch --name runbook --yes",
            "atlx confluence content attachments list --id 12345 --media-type image/png --filename a.png --sort -modified-date --status current",
            "atlx confluence content attachments get --attachment-id att456 --at-version 1",
            "atlx confluence content attachments download --attachment-id att456 --out a.png --force",
            "atlx confluence content attachments upload --id 12345 --file a --file b --minor-edit --yes",
            "atlx confluence content attachments delete --attachment-id att456 --purge --yes",
            "atlx confluence content body-formats md",
            "atlx confluence content body-formats --content-type live_doc",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx confluence content versions get --id 12345",
            "atlx confluence content versions diff --id 12345 --from 1",
            "atlx confluence content history restore --id 12345 --at-version 2",
            "atlx confluence content attachments upload --id 12345 --yes",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_outline_section_and_edits() {
        for ok in [
            "atlx confluence content get 12345 --detail outline",
            "atlx confluence content get 12345 --section Rollback --section 'Deploy plan'",
            "atlx confluence content get 12345 --section Rollback --format html --body-only",
            "atlx confluence content get 12345 --detail full --format adf",
            "atlx confluence content update 12345 --snapshot-token 7 --edits '[]' --ack-body-formats -y",
            "atlx confluence content update 12345 --edits-file edits.json --dry-run --title T",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx confluence content update 12345 --edits '[]' --edits-file e.json",
            "atlx confluence content update 12345 --edits '[]' --body x",
            "atlx confluence content update 12345 --edits-file e.json --body-file b.html",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
        let cli = Cli::try_parse_from(shell_words(
            "atlx confluence content get 12345 --section A --section B",
        ))
        .unwrap();
        let Cmd::Confluence {
            cmd:
                ConfCmd::Content {
                    cmd:
                        ContentCmd::Get {
                            sections, format, ..
                        },
                },
        } = cli.cmd
        else {
            panic!("unexpected parsed command");
        };
        assert_eq!(sections, ["A", "B"]);
        assert_eq!(format, None);
    }

    #[test]
    fn confluence_delete_and_old_forms_parse() {
        for ok in [
            "atlx confluence content delete 12345",
            "atlx confluence content delete 12345 --yes --purge",
            "atlx confluence content labels 12345 --limit 5",
            "atlx confluence content labels list --id 12345",
            "atlx confluence content label list --id 12345",
            "atlx confluence content comments list 12345 --inline --format html",
            "atlx confluence content comments list --id 12345 --comment-type inline --body-format html",
            "atlx confluence content comments create 12345 --body hi --format markdown",
            "atlx confluence content comments create --id 12345 --body-file c.md --body-format md",
            "atlx confluence tree 12345",
            "atlx confluence tree --target 12345 --depth 3",
            "atlx confluence space get --key ENG --labels",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx confluence content comments list 12345 --id 1",
            "atlx confluence content comments list --id 1 --inline --comment-type footer",
            "atlx confluence content comments create --id 1 --body a --body-file b",
            "atlx confluence content labels 12345 list --id 1",
            "atlx confluence tree 1 --target 2",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }

        let cli = Cli::try_parse_from(shell_words(
            "atlx confluence content comments list --id 12345 --comment-type inline",
        ))
        .unwrap();
        let Cmd::Confluence {
            cmd:
                ConfCmd::Content {
                    cmd:
                        ContentCmd::Comments {
                            cmd:
                                ConfCommentCmd::List {
                                    id,
                                    comment_type,
                                    body_format,
                                    ..
                                },
                        },
                },
        } = cli.cmd
        else {
            panic!("unexpected parsed command");
        };
        assert_eq!(id.as_deref(), Some("12345"));
        assert_eq!(comment_type, Some(confluence::comments::Surface::Inline));
        assert_eq!(body_format, None);
    }
}

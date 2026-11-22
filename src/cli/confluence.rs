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
    /// Page ancestors and descendant tree
    Tree {
        /// Page ID or URL (same as --target)
        #[arg(value_name = "ref", conflicts_with = "target")]
        reference: Option<String>,
        /// Page ID or URL
        #[arg(long)]
        target: Option<String>,
        /// Max descendant depth (default 2, capped at 5)
        #[arg(long, default_value_t = 2)]
        depth: usize,
    },
}

/// `--comment-type` on comment commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum CommentType {
    Footer,
    Inline,
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
        #[arg(long, value_enum, default_value_t)]
        format: TextFormat,
        #[arg(long)]
        draft: bool,
        /// Accepted for twg compatibility
        #[arg(long, short)]
        yes: bool,
        /// Accepted for twg compatibility
        #[arg(long)]
        ack_body_formats: bool,
    },
    /// Update a page or blog post (new version)
    Update {
        reference: Option<String>,
        #[arg(long)]
        id: Option<String>,
        /// Version from a previous get/create/update; rejects stale edits
        #[arg(long)]
        snapshot_token: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        version_message: Option<String>,
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
        #[arg(long, value_enum, default_value_t)]
        format: TextFormat,
        /// Validate and show the request without writing; with --edits, also
        /// the resulting body and a diff
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        draft: bool,
        #[arg(long, short)]
        yes: bool,
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
    /// Delete a page or blog post (atlx extra; moves it to the trash unless
    /// --purge)
    Delete {
        reference: String,
        /// Accepted for compatibility; no confirmation is asked
        #[arg(long, short, hide = true)]
        yes: bool,
        /// Permanently purge instead of moving to trash
        #[arg(long)]
        purge: bool,
    },
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
        /// REST sort order, e.g. -created-date or title
        #[arg(long)]
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
pub(crate) enum ConfCommentCmd {
    /// List footer or inline comments
    List {
        /// Old form of --id
        #[arg(hide = true, conflicts_with = "id")]
        reference: Option<String>,
        /// Page or blog post ID or URL
        #[arg(long)]
        id: Option<String>,
        /// footer (default) or inline
        #[arg(long, value_enum, conflicts_with = "inline")]
        comment_type: Option<CommentType>,
        /// Comment body format: md (default) or html
        #[arg(long, value_enum, conflicts_with = "format")]
        body_format: Option<confluence::ReadFormat>,
        /// Old form of --comment-type inline
        #[arg(long, hide = true)]
        inline: bool,
        /// Old form of --body-format
        #[arg(long, value_enum, hide = true)]
        format: Option<confluence::ReadFormat>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Add a footer comment
    Create {
        /// Old form of --id
        #[arg(hide = true, conflicts_with = "id")]
        reference: Option<String>,
        /// Page or blog post ID or URL
        #[arg(long)]
        id: Option<String>,
        /// Comment body (`-` reads stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the comment body from a file
        #[arg(long)]
        body_file: Option<String>,
        /// Body format: markdown/md (default), html or plain
        #[arg(long, value_enum, conflicts_with = "format")]
        body_format: Option<TextFormat>,
        /// Old form of --body-format (default html)
        #[arg(long, value_enum, hide = true)]
        format: Option<TextFormat>,
        /// Only footer comments can be created so far
        #[arg(long, value_enum)]
        comment_type: Option<CommentType>,
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
                ..
            } => {
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
                    },
                )?;
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
                ..
            } => {
                let Some(reference) = reference.or(id) else {
                    bail!("pass a content ID/URL or --id")
                };
                let ops = match body_arg(edits, edits_file)? {
                    Some(json) => Some(confluence::edits::parse(&json)?),
                    None => None,
                };
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
            ContentCmd::Comments { cmd } => match cmd {
                ConfCommentCmd::List {
                    reference,
                    id,
                    comment_type,
                    body_format,
                    inline,
                    format,
                    limit,
                    cursor,
                } => {
                    let reference = content_ref(
                        reference,
                        id,
                        "confluence content comments list <ref>",
                        "--id <ref>",
                    )?;
                    if inline {
                        deprecated("--inline", "--comment-type inline");
                    }
                    if format.is_some() {
                        deprecated("--format", "--body-format");
                    }
                    let inline = inline || comment_type == Some(CommentType::Inline);
                    let format = body_format.or(format).unwrap_or(confluence::ReadFormat::Md);
                    let data = confluence::comments_list(
                        c,
                        &reference,
                        inline,
                        format,
                        limit,
                        cursor.as_deref(),
                    )?;
                    envelope("confluence.content.comments.list", "confluence.comment")
                        .request(json!({
                            "id": reference,
                            "commentType": if inline { "inline" } else { "footer" },
                            "limit": limit,
                            "cursor": cursor,
                        }))
                        .next_cursor(data["nextCursor"].as_str())
                        .data(data)
                }
                ConfCommentCmd::Create {
                    reference,
                    id,
                    body,
                    body_file,
                    body_format,
                    format,
                    comment_type,
                } => {
                    let old_form = reference.is_some();
                    let reference = content_ref(
                        reference,
                        id,
                        "confluence content comments create <ref>",
                        "--id <ref>",
                    )?;
                    if comment_type == Some(CommentType::Inline) {
                        return Err(ApiError::usage(
                            "inline comments need a text selection, which atlx doesn't support yet; \
                             omit --comment-type to add a footer comment",
                        )
                        .into());
                    }
                    if format.is_some() {
                        deprecated("--format", "--body-format");
                    }
                    let Some(body) = body_arg(body, body_file)? else {
                        return Err(ApiError::usage("pass --body or --body-file").into());
                    };
                    // twg defaults to markdown; the old positional form kept
                    // atlx's earlier html default.
                    let default = if old_form {
                        TextFormat::Html
                    } else {
                        TextFormat::Markdown
                    };
                    let format = body_format.or(format).unwrap_or(default);
                    envelope("confluence.content.comments.create", "confluence.comment")
                        .request(json!({"id": reference, "commentType": "footer"}))
                        .data(confluence::comment_create(c, &reference, &body, format)?)
                }
            },
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
            ContentCmd::Delete {
                reference,
                yes: _,
                purge,
            } => envelope("confluence.content.delete", "confluence.content")
                .request(json!({"id": reference, "purge": purge}))
                .data(confluence::delete(c, &reference, purge)?),
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
        ConfCmd::Tree {
            reference,
            target,
            depth,
        } => {
            let Some(reference) = target.or(reference) else {
                return Err(ApiError::usage("pass a page ID or URL, or --target").into());
            };
            envelope("confluence.tree", "confluence.tree")
                .request(json!({"target": reference, "depth": depth}))
                .data(confluence::tree(c, &reference, depth)?)
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

/// `content body-formats`: printed without an envelope, as twg does; text
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
            "atlx confluence content attachments list --id 12345 --media-type image/png --filename a.png --sort title --status current",
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
    fn confluence_delete_needs_no_yes_and_old_forms_parse() {
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
        assert_eq!(comment_type, Some(CommentType::Inline));
        assert_eq!(body_format, None);
    }
}

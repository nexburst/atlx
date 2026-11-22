//! `confluence content comments`.

use super::content_ref;
use crate::cli::util::{body_arg, deprecated, require_yes};
use crate::client::{ApiError, Client};
use crate::confluence::{self, comments};
use crate::markup::TextFormat;
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};

const KIND: &str = "confluence.comment";

/// `--content-type` on `comments list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ParentType {
    Page,
    Blogpost,
}

impl ParentType {
    fn kind(self) -> confluence::ContentType {
        match self {
            ParentType::Page => confluence::ContentType::Page,
            ParentType::Blogpost => confluence::ContentType::Blogpost,
        }
    }
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ConfCommentCmd {
    /// List footer or inline comments
    List {
        /// Old form of --id
        #[arg(hide = true, conflicts_with = "id")]
        reference: Option<String>,
        /// Page or blog post ID, URL, or exact title (with --reference-space)
        #[arg(long)]
        id: Option<String>,
        /// Space key or ID for looking up --id by exact title
        #[arg(long)]
        reference_space: Option<String>,
        /// page or blogpost, which skips probing for it
        #[arg(long, value_enum)]
        content_type: Option<ParentType>,
        /// footer (default) or inline
        #[arg(long, value_enum, conflicts_with = "inline")]
        comment_type: Option<comments::Surface>,
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
        /// Oldest (created-date) or newest (-created-date) first
        #[arg(long, value_enum, allow_hyphen_values = true)]
        sort: Option<comments::Sort>,
        /// current or draft
        #[arg(long, value_parser = ["current", "draft"])]
        status: Option<String>,
        /// Inline comments only; open includes reopened and dangling ones
        #[arg(long, value_enum)]
        resolution_status: Option<comments::Resolution>,
        /// Nest each comment's replies under it (true or false; default
        /// false)
        #[arg(long, value_name = "BOOL")]
        include_replies: Option<bool>,
    },
    /// One comment, with its replies
    Get {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// footer or inline [default: detected]
        #[arg(long, value_enum)]
        comment_type: Option<comments::Surface>,
        /// Comment body format: md (default) or html
        #[arg(long, value_enum)]
        body_format: Option<confluence::ReadFormat>,
        /// Nest the replies (true or false; default true)
        #[arg(long, value_name = "BOOL")]
        include_replies: Option<bool>,
    },
    /// Add a footer comment, or an inline one on selected text
    Create {
        /// Old form of --id
        #[arg(hide = true, conflicts_with = "id")]
        reference: Option<String>,
        /// Page or blog post ID, URL, or exact title (with --reference-space)
        #[arg(long)]
        id: Option<String>,
        /// Space key or ID for looking up --id by exact title
        #[arg(long)]
        reference_space: Option<String>,
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
        /// footer, or inline (the default with --text-selection)
        #[arg(long, value_enum)]
        comment_type: Option<comments::Surface>,
        /// The exact text an inline comment is anchored to
        #[arg(long)]
        text_selection: Option<String>,
        /// How often that text appears [default: counted]
        #[arg(long, requires = "text_selection")]
        text_selection_match_count: Option<usize>,
        /// Which appearance, from 0 [default: 0]
        #[arg(long, requires = "text_selection")]
        text_selection_match_index: Option<usize>,
    },
    /// Reply to a comment
    Reply {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// Reply body (`-` reads stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the reply from a file
        #[arg(long)]
        body_file: Option<String>,
        /// Body format: markdown/md (default), html or plain
        #[arg(long, value_enum)]
        body_format: Option<TextFormat>,
    },
    /// Replace a comment's body
    Update {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// footer or inline [default: detected]
        #[arg(long, value_enum)]
        comment_type: Option<comments::Surface>,
        /// New body (`-` reads stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the new body from a file
        #[arg(long)]
        body_file: Option<String>,
        /// Body format: markdown/md (default), html or plain
        #[arg(long, value_enum)]
        body_format: Option<TextFormat>,
        /// Fail unless the comment is still at this version
        #[arg(long)]
        expected_version: Option<u64>,
    },
    /// Delete a comment for good
    Delete {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Resolve an inline comment
    Resolve {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// inline; REST can't resolve footer comments
        #[arg(long, value_enum)]
        comment_type: Option<comments::Surface>,
    },
    /// Reopen a resolved inline comment
    Reopen {
        /// Comment ID, ARI or URL
        #[arg(long)]
        comment_id: String,
        /// inline; REST can't reopen footer comments
        #[arg(long, value_enum)]
        comment_type: Option<comments::Surface>,
    },
}

fn required_body(body: Option<String>, file: Option<String>) -> Result<String> {
    body_arg(body, file)?.ok_or_else(|| ApiError::usage("pass --body or --body-file").into())
}

fn out(verb: &str, request: Value, data: Value) -> Value {
    envelope(&format!("confluence.content.comments.{verb}"), KIND)
        .request(request)
        .data(data)
}

pub(super) fn run(c: &Client, cmd: ConfCommentCmd) -> Result<Value> {
    use comments::Surface;
    Ok(match cmd {
        ConfCommentCmd::List {
            reference,
            id,
            reference_space,
            content_type,
            comment_type,
            body_format,
            inline,
            format,
            limit,
            cursor,
            sort,
            status,
            resolution_status,
            include_replies,
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
            let surface = if inline {
                Surface::Inline
            } else {
                comment_type.unwrap_or(Surface::Footer)
            };
            let data = comments::list(
                c,
                &reference,
                reference_space.as_deref(),
                &comments::ListOpts {
                    surface,
                    format: body_format.or(format).unwrap_or(confluence::ReadFormat::Md),
                    limit,
                    cursor: cursor.as_deref(),
                    sort,
                    status: status.as_deref(),
                    resolution: resolution_status,
                    content_type: content_type.map(ParentType::kind),
                    replies: include_replies.unwrap_or(false),
                },
            )?;
            envelope("confluence.content.comments.list", KIND)
                .request(json!({
                    "id": reference,
                    "commentType": surface.name(),
                    "limit": limit,
                    "cursor": cursor,
                }))
                .next_cursor(data["nextCursor"].as_str())
                .data(data)
        }
        ConfCommentCmd::Get {
            comment_id,
            comment_type,
            body_format,
            include_replies,
        } => {
            let replies = include_replies.unwrap_or(true);
            let format = body_format.unwrap_or(confluence::ReadFormat::Md);
            let data = comments::get(c, &comment_id, comment_type, format, replies)?;
            let request = json!({"commentId": comment_id, "includeReplies": replies});
            out("get", request, data)
        }
        ConfCommentCmd::Create {
            reference,
            id,
            reference_space,
            body,
            body_file,
            body_format,
            format,
            comment_type,
            text_selection,
            text_selection_match_count,
            text_selection_match_index,
        } => {
            let old_form = reference.is_some();
            let reference = content_ref(
                reference,
                id,
                "confluence content comments create <ref>",
                "--id <ref>",
            )?;
            match (comment_type, &text_selection) {
                (Some(Surface::Inline), None) => {
                    return Err(ApiError::usage("inline comments need --text-selection").into());
                }
                (Some(Surface::Footer), Some(_)) => {
                    return Err(ApiError::usage(
                        "--text-selection is for inline comments; drop --comment-type footer",
                    )
                    .into());
                }
                _ => {}
            }
            if format.is_some() {
                deprecated("--format", "--body-format");
            }
            let body = required_body(body, body_file)?;
            // twg defaults to markdown; the old positional form kept atlx's
            // earlier html default.
            let default = if old_form {
                TextFormat::Html
            } else {
                TextFormat::Markdown
            };
            let selection = text_selection.as_deref().map(|text| comments::Selection {
                text,
                count: text_selection_match_count,
                index: text_selection_match_index,
            });
            let data = comments::create(
                c,
                &reference,
                reference_space.as_deref(),
                &body,
                body_format.or(format).unwrap_or(default),
                selection,
            )?;
            let request = json!({"id": reference, "commentType": data["commentType"]});
            out("create", request, data)
        }
        ConfCommentCmd::Reply {
            comment_id,
            body,
            body_file,
            body_format,
        } => {
            let body = required_body(body, body_file)?;
            let format = body_format.unwrap_or(TextFormat::Markdown);
            let data = comments::reply(c, &comment_id, &body, format)?;
            out("reply", json!({"commentId": comment_id}), data)
        }
        ConfCommentCmd::Update {
            comment_id,
            comment_type,
            body,
            body_file,
            body_format,
            expected_version,
        } => {
            let body = required_body(body, body_file)?;
            let format = body_format.unwrap_or(TextFormat::Markdown);
            let data = comments::update(
                c,
                &comment_id,
                comment_type,
                &body,
                format,
                expected_version,
            )?;
            let request = json!({"commentId": comment_id, "expectedVersion": expected_version});
            out("update", request, data)
        }
        ConfCommentCmd::Delete { comment_id, yes } => {
            require_yes(yes, "deleting the comment")?;
            let data = comments::delete(c, &comment_id)?;
            out("delete", json!({"commentId": comment_id}), data)
        }
        ConfCommentCmd::Resolve {
            comment_id,
            comment_type,
        } => {
            let data = comments::set_resolved(c, &comment_id, comment_type, true)?;
            out("resolve", json!({"commentId": comment_id}), data)
        }
        ConfCommentCmd::Reopen {
            comment_id,
            comment_type,
        } => {
            let data = comments::set_resolved(c, &comment_id, comment_type, false)?;
            out("reopen", json!({"commentId": comment_id}), data)
        }
    })
}

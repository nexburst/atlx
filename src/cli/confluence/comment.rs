//! `confluence page comment ...`.

use super::{Batch, BodyArgs, Paging};
use crate::cli::util;
use crate::client::{ApiError, Client};
use crate::confluence::{ContentType, comments};
use crate::markup::BodyFormat;
use anyhow::Result;
use clap::{ArgGroup, Args, Subcommand};
use serde_json::Value;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommentCmd {
    /// List the footer (default) or inline comments of a page or blog post
    List(ListArgs),
    /// Read comments by ID, ARI or URL
    Get(GetArgs),
    /// Add a footer comment, an inline comment on selected text, or a reply
    Create(CreateArgs),
    /// Replace a comment's body, or resolve or reopen an inline comment
    Update(UpdateArgs),
    /// Delete a comment for good
    Delete(DeleteArgs),
}

#[derive(Args)]
pub(crate) struct ListArgs {
    /// Page or blog post ID, URL, or exact title with --space
    #[arg(long)]
    page: String,
    /// Space key or ID for looking --page up by exact title
    #[arg(long)]
    space: Option<String>,
    /// page or blogpost, which skips probing for it
    #[arg(long = "type", value_enum)]
    kind: Option<ContentType>,
    /// Inline comments instead of footer comments
    #[arg(long)]
    inline: bool,
    /// Inline comments in this state; open also takes in reopened and
    /// dangling ones
    #[arg(long, value_enum, requires = "inline")]
    resolution: Option<comments::Resolution>,
    /// current (default) or draft
    #[arg(long, value_parser = ["current", "draft"])]
    status: Option<String>,
    /// Oldest (created-date) or newest (-created-date) first
    #[arg(long, value_enum, allow_hyphen_values = true)]
    sort: Option<comments::Sort>,
    /// Nest each comment's replies under it (at most 500 in all)
    #[arg(long)]
    replies: bool,
    /// Bodies as markdown (default), adf, storage or plain
    #[arg(long, value_enum, default_value_t)]
    body_format: BodyFormat,
    #[command(flatten)]
    paging: Paging,
}

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Comment IDs, ARIs or URLs; `-` reads them from stdin
    #[arg(value_name = "ID", required = true)]
    ids: Vec<String>,
    /// They are inline comments (skips probing footer comments first)
    #[arg(long)]
    inline: bool,
    /// Nest the replies
    #[arg(long)]
    replies: bool,
    /// Bodies as markdown (default), adf, storage or plain
    #[arg(long, value_enum, default_value_t)]
    body_format: BodyFormat,
    #[command(flatten)]
    batch: Batch,
}

#[derive(Args)]
#[command(group(ArgGroup::new("on").required(true).args(["page", "comment"])))]
pub(crate) struct CreateArgs {
    /// Comment on this page or blog post (ID, URL, or exact title with
    /// --space)
    #[arg(long)]
    page: Option<String>,
    /// Reply to this comment instead
    #[arg(long, conflicts_with_all = ["space", "text_selection"])]
    comment: Option<String>,
    /// Space key or ID for looking --page up by exact title
    #[arg(long)]
    space: Option<String>,
    #[command(flatten)]
    body: BodyArgs,
    /// Make it an inline comment anchored to this exact text
    #[arg(long)]
    text_selection: Option<String>,
    /// How often that text appears in the page [default: counted]
    #[arg(long, requires = "text_selection")]
    text_selection_match_count: Option<usize>,
    /// Which appearance to anchor to, from 0 [default: 0]
    #[arg(long, requires = "text_selection")]
    text_selection_match_index: Option<usize>,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// Comment ID, ARI or URL
    #[arg(value_name = "ID")]
    id: String,
    #[command(flatten)]
    body: BodyArgs,
    /// The comment's `version` you read; a newer one is a conflict (exit 6)
    #[arg(long, value_name = "N")]
    if_version: Option<u64>,
    /// Resolve the inline comment
    #[arg(long, conflicts_with_all = ["reopen", "body", "body_file"])]
    resolve: bool,
    /// Reopen the resolved inline comment
    #[arg(long, conflicts_with_all = ["body", "body_file"])]
    reopen: bool,
    /// It is an inline comment (skips probing footer comments first)
    #[arg(long)]
    inline: bool,
    /// Replace a body even though Markdown cannot express all of it, losing
    /// that content (needs --yes)
    #[arg(long)]
    allow_lossy: bool,
    /// Confirm --allow-lossy
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Comment ID, ARI or URL
    #[arg(value_name = "ID")]
    id: String,
    #[command(flatten)]
    yes: super::Yes,
}

fn surface(inline: bool) -> Option<comments::Surface> {
    inline.then_some(comments::Surface::Inline)
}

pub(super) fn run(c: &Client, cmd: CommentCmd) -> Result<Value> {
    use comments::Surface;
    match cmd {
        CommentCmd::List(a) => {
            let id = crate::confluence::resolve_id(c, &a.page, a.space.as_deref())?;
            let s = if a.inline {
                Surface::Inline
            } else {
                Surface::Footer
            };
            a.paging.collect(250, |cur, n| {
                comments::list(
                    c,
                    &id,
                    None,
                    &comments::ListOpts {
                        surface: s,
                        format: a.body_format,
                        limit: n,
                        cursor: cur,
                        sort: a.sort,
                        status: a.status.as_deref(),
                        resolution: a.resolution,
                        content_type: a.kind,
                        replies: a.replies,
                    },
                )
            })
        }
        CommentCmd::Get(a) => a.batch.run(a.ids, |id| {
            comments::get(c, id, surface(a.inline), a.body_format, a.replies)
        }),
        CommentCmd::Create(a) => {
            let body = a.body.required()?;
            match (&a.comment, &a.page) {
                (Some(parent), _) => comments::reply(c, parent, &body),
                (None, Some(page)) => {
                    let selection = a.text_selection.as_deref().map(|text| comments::Selection {
                        text,
                        count: a.text_selection_match_count,
                        index: a.text_selection_match_index,
                    });
                    comments::create(c, page, a.space.as_deref(), &body, selection)
                }
                (None, None) => unreachable!("clap requires --page or --comment"),
            }
        }
        CommentCmd::Update(a) => {
            if a.resolve || a.reopen {
                return comments::set_resolved(c, &a.id, surface(a.inline), a.resolve);
            }
            if a.allow_lossy && !a.yes && !crate::cli::dry_run_requested() {
                return Err(util::needs_yes(
                    "--allow-lossy (it destroys what Markdown cannot express)",
                ));
            }
            let body = a.body.parse()?.ok_or_else(|| {
                ApiError::usage("pass --body or --body-file, or --resolve or --reopen")
            })?;
            comments::update(
                c,
                &a.id,
                surface(a.inline),
                &body,
                comments::Replace {
                    if_version: a.if_version,
                    check_loss: a.body.needs_loss_check(),
                    allow_lossy: a.allow_lossy,
                },
            )
        }
        CommentCmd::Delete(a) => comments::delete(c, &a.id),
    }
}

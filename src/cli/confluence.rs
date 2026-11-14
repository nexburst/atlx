//! `atlx confluence`: clap definitions and dispatch.

use super::Out;
use super::util::{batch, body_arg, deprecated};
use crate::client::{self, ApiError};
use crate::confluence;
use crate::markup::TextFormat;
use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};

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
        /// summary (excerpt) or full (body)
        #[arg(long, value_enum)]
        detail: Option<confluence::Detail>,
        /// Body format for --detail full: html (storage) or md
        #[arg(long, value_enum, default_value_t)]
        format: confluence::ReadFormat,
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
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        format: TextFormat,
        /// Validate and show the request without writing
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
}

#[derive(Subcommand)]
pub(crate) enum LabelsCmd {
    /// List the labels on a page or blog post
    List {
        /// Page or blog post ID or URL
        #[arg(long)]
        id: String,
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
            confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?
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
            confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?
        }
        ConfCmd::Content { cmd } => match cmd {
            ContentCmd::Get {
                refs,
                id_flags,
                detail,
                format,
                body_only,
                draft,
                reference_space,
                include_metadata: _,
                strict,
            } => {
                let inputs: Vec<String> = refs.into_iter().chain(id_flags).collect();
                let detail = if body_only {
                    confluence::Detail::Full
                } else {
                    detail.unwrap_or_default()
                };
                let opts = confluence::GetOpts {
                    detail,
                    format,
                    draft,
                    reference_space: reference_space.as_deref(),
                };
                if body_only {
                    let [one] = inputs.as_slice() else {
                        bail!("--body-only takes exactly one ID")
                    };
                    let v = confluence::get(c, one, &opts)?;
                    let body = v["body"].as_str().unwrap_or_default().to_string();
                    return Ok(Out::Body { body, full: v });
                }
                batch(inputs, strict, 1, |r| confluence::get(c, r, &opts))?
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
                confluence::create(
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
                )?
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
                format,
                dry_run,
                draft,
                ..
            } => {
                let Some(reference) = reference.or(id) else {
                    bail!("pass a content ID/URL or --id")
                };
                let body = body_arg(body, body_file)?;
                confluence::update(
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
                    },
                )?
            }
            ContentCmd::List {
                space,
                content_type,
                status,
                limit,
                cursor,
            } => confluence::list(c, &space, content_type, &status, limit, cursor.as_deref())?,
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
                    confluence::comments_list(
                        c,
                        &reference,
                        inline,
                        format,
                        limit,
                        cursor.as_deref(),
                    )?
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
                    confluence::comment_create(c, &reference, &body, format)?
                }
            },
            ContentCmd::Children {
                reference,
                limit,
                cursor,
            } => confluence::children(c, &reference, limit, cursor.as_deref())?,
            ContentCmd::Labels {
                cmd: Some(LabelsCmd::List { id }),
                ..
            } => confluence::labels(c, &id, 250)?,
            ContentCmd::Labels {
                reference: Some(reference),
                limit,
                cmd: None,
            } => {
                deprecated(
                    "confluence content labels <ref>",
                    "confluence content labels list --id <ref>",
                );
                confluence::labels(c, &reference, limit)?
            }
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
            } => confluence::delete(c, &reference, purge)?,
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
            v
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
            v
        }
        ConfCmd::Space { cmd: SpaceCmd::Me } => confluence::space_me(c)?,
        ConfCmd::Tree {
            reference,
            target,
            depth,
        } => {
            let Some(reference) = target.or(reference) else {
                return Err(ApiError::usage("pass a page ID or URL, or --target").into());
            };
            confluence::tree(c, &reference, depth)?
        }
    };
    Ok(Out::Data(data))
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

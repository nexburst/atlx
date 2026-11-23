//! `confluence space|template|label ...`.

use super::{Batch, Paging, Yes};
use crate::client::{ApiError, Client};
use crate::confluence::{self, extras, labels, lifecycle, tree};
use crate::markup::BodyFormat;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;

/// A space a command acts on.
#[derive(Args)]
pub(crate) struct SpaceRef {
    /// Space key, ID, ARI or URL
    #[arg(value_name = "SPACE")]
    space: String,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum SpaceCmd {
    /// List spaces
    List {
        /// global or personal
        #[arg(long = "type", value_parser = ["global", "personal"])]
        kind: Option<String>,
        /// current or archived
        #[arg(long, value_parser = ["current", "archived"])]
        status: Option<String>,
        /// Only these space keys; repeatable, commas also separate them
        #[arg(long = "key", value_name = "KEY")]
        keys: Vec<String>,
        /// Also fetch each space's labels (one request per space; shown
        /// with --full or --fields labels)
        #[arg(long)]
        labels: bool,
        #[command(flatten)]
        paging: Paging,
    },
    /// Spaces by key or ID, or your personal space
    Get {
        /// Space keys or IDs; `-` reads them from stdin
        #[arg(value_name = "SPACE", required_unless_present = "personal")]
        spaces: Vec<String>,
        /// Your personal space
        #[arg(long, conflicts_with = "spaces")]
        personal: bool,
        /// Also fetch the labels (shown with --full or --fields labels)
        #[arg(long)]
        labels: bool,
        #[command(flatten)]
        batch: Batch,
    },
    /// Create a space
    Create {
        /// Space key: letters and digits
        #[arg(long)]
        key: String,
        #[arg(long)]
        name: String,
        /// Plain-text description
        #[arg(long)]
        description: Option<String>,
        /// Only you can see it
        #[arg(long)]
        private: bool,
    },
    /// Change a space's name, description, type, status or home page
    Update {
        #[command(flatten)]
        space: SpaceRef,
        #[arg(long)]
        name: Option<String>,
        /// Plain-text description
        #[arg(long)]
        description: Option<String>,
        /// global or personal
        #[arg(long = "type", value_parser = ["global", "personal"])]
        kind: Option<String>,
        /// current or archived
        #[arg(long, value_parser = ["current", "archived"])]
        status: Option<String>,
        /// Home page ID or URL
        #[arg(long)]
        homepage: Option<String>,
    },
    /// Archive a space
    Archive {
        #[command(flatten)]
        space: SpaceRef,
        #[command(flatten)]
        yes: Yes,
    },
    /// Bring an archived space back
    Restore {
        #[command(flatten)]
        space: SpaceRef,
    },
    /// Delete a space and everything in it, for good
    Delete {
        #[command(flatten)]
        space: SpaceRef,
        #[command(flatten)]
        yes: Yes,
    },
    /// Watch a space (get notified about changes in it)
    Watch {
        #[command(flatten)]
        space: SpaceRef,
    },
    /// Stop watching a space
    Unwatch {
        #[command(flatten)]
        space: SpaceRef,
    },
    /// A space's page tree, from its top-level pages
    Tree {
        #[command(flatten)]
        space: SpaceRef,
        /// How many levels (default 2, at most 10; at most 500 nodes)
        #[arg(long, default_value_t = 2)]
        depth: usize,
        /// Only this type of node, with whatever contains it
        #[arg(long = "type", value_enum)]
        kind: Option<tree::NodeType>,
    },
    /// Space permissions
    Permission {
        #[command(subcommand)]
        cmd: PermissionCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum PermissionCmd {
    /// The permissions granted in a space
    List {
        /// Space key or ID
        #[arg(long)]
        space: String,
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum TemplateCmd {
    /// List templates and blueprints
    List {
        /// A space's templates (key or ID)
        #[arg(long)]
        space: Option<String>,
        /// global, space (default with --space), or available (both)
        #[arg(long, value_enum)]
        scope: Option<extras::Scope>,
        /// all (default), page or blueprint
        #[arg(long = "type", value_enum, default_value = "all")]
        kind: extras::TemplateType,
        /// Only templates whose ID, name or description contains this text
        #[arg(short, long)]
        query: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// One template, with its body
    Get {
        /// Template ID
        #[arg(value_name = "ID")]
        id: String,
        /// The body as markdown (default), adf, storage or plain
        #[arg(long, value_enum, default_value_t)]
        body_format: BodyFormat,
    },
}

#[derive(Subcommand)]
pub(crate) enum LabelCmd {
    /// Get notified about content that gets this label
    Watch {
        /// Label name
        #[arg(value_name = "LABEL")]
        name: String,
    },
    /// Stop watching a label
    Unwatch {
        /// Label name
        #[arg(value_name = "LABEL")]
        name: String,
    },
}

fn status_change(status: &'static str) -> lifecycle::SpaceChange<'static> {
    lifecycle::SpaceChange {
        status: Some(status),
        ..Default::default()
    }
}

pub(super) fn run(c: &Client, cmd: SpaceCmd) -> Result<Value> {
    Ok(match cmd {
        SpaceCmd::List {
            kind,
            status,
            keys,
            labels,
            paging,
        } => {
            let keys: Vec<&str> = keys
                .iter()
                .flat_map(|k| k.split(','))
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .collect();
            let keys = (!keys.is_empty()).then(|| keys.join(","));
            let mut v = paging.collect(250, |cur, n| {
                confluence::spaces(
                    c,
                    kind.as_deref(),
                    status.as_deref(),
                    keys.as_deref(),
                    n,
                    cur,
                )
            })?;
            if labels {
                confluence::add_space_labels(c, &mut v["data"])?;
            }
            v
        }
        SpaceCmd::Get {
            spaces,
            personal,
            labels,
            batch,
        } => {
            let one = |s: &str| -> Result<Value> {
                let mut v = confluence::space(c, s)?;
                if labels {
                    confluence::add_space_labels(c, &mut v)?;
                }
                Ok(v)
            };
            if personal {
                let mut v = confluence::space_me(c)?;
                if labels {
                    confluence::add_space_labels(c, &mut v)?;
                }
                v
            } else {
                batch.run(spaces, one)?
            }
        }
        SpaceCmd::Create {
            key,
            name,
            description,
            private,
        } => lifecycle::space_create(
            c,
            &lifecycle::NewSpace {
                key: &key,
                name: &name,
                description: description.as_deref(),
                private,
            },
        )?,
        SpaceCmd::Update {
            space,
            name,
            description,
            kind,
            status,
            homepage,
        } => lifecycle::space_update(
            c,
            &space.space,
            &lifecycle::SpaceChange {
                name: name.as_deref(),
                description: description.as_deref(),
                kind: kind.as_deref(),
                status: status.as_deref(),
                homepage: homepage.as_deref(),
            },
        )?,
        SpaceCmd::Archive { space, .. } => {
            lifecycle::space_update(c, &space.space, &status_change("archived"))?
        }
        SpaceCmd::Restore { space } => {
            lifecycle::space_update(c, &space.space, &status_change("current"))?
        }
        SpaceCmd::Delete { space, .. } => lifecycle::space_delete(c, &space.space)?,
        SpaceCmd::Watch { space } => extras::watch_space(c, &space.space, true)?,
        SpaceCmd::Unwatch { space } => extras::watch_space(c, &space.space, false)?,
        SpaceCmd::Tree { space, depth, kind } => tree::space(c, &space.space, depth, kind)?,
        SpaceCmd::Permission {
            cmd: PermissionCmd::List { space, paging },
        } => paging.collect(250, |cur, n| extras::permissions(c, Some(&space), n, cur))?,
    })
}

pub(super) fn template(c: &Client, cmd: TemplateCmd) -> Result<Value> {
    match cmd {
        TemplateCmd::List {
            space,
            scope,
            kind,
            query,
            paging,
        } => {
            if scope == Some(extras::Scope::Space) && space.is_none() {
                return Err(ApiError::usage("--scope space needs --space").into());
            }
            paging.collect_offset(|start, n| {
                extras::templates(
                    c,
                    &extras::TemplateQuery {
                        space: space.as_deref(),
                        scope,
                        kind,
                        text: query.as_deref(),
                        start,
                        limit: n,
                    },
                )
            })
        }
        TemplateCmd::Get { id, body_format } => extras::template(c, &id, body_format),
    }
}

pub(super) fn label(c: &Client, cmd: LabelCmd) -> Result<Value> {
    match cmd {
        LabelCmd::Watch { name } => labels::watch(c, &name, true),
        LabelCmd::Unwatch { name } => labels::watch(c, &name, false),
    }
}

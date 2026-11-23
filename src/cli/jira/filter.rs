//! `jira filter` and its shares, columns and subscriptions.

use super::{Confirm, Many, Paging, listed};
use crate::cli::util::text_arg;
use crate::client::{ApiError, Client};
use crate::jira::{self, admin};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum FilterCmd {
    /// Saved filters, by name, owner, group or project; or your favourites
    List {
        /// Name contains
        #[arg(short, long)]
        query: Option<String>,
        /// Owner: me, an email, a name or an account ID
        #[arg(long)]
        owner: Option<String>,
        /// Shared with this group (name)
        #[arg(long)]
        group: Option<String>,
        /// Shared with this project (ID)
        #[arg(long)]
        project: Option<String>,
        /// Sort, e.g. name or -id
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
        /// Admins: also filters not shared with you
        #[arg(long)]
        override_share_permissions: bool,
        /// Only your favourites
        #[arg(long, conflicts_with_all = ["query", "owner", "group", "project", "order_by", "override_share_permissions"])]
        favourite: bool,
        #[command(flatten)]
        paging: Paging,
    },
    /// Filters by ID, URL or exact name
    Get {
        /// Filter IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a filter
    Create {
        /// Name
        #[arg(long)]
        name: String,
        /// JQL
        #[arg(long)]
        jql: String,
        #[command(flatten)]
        fields: FilterFields,
    },
    /// Change a filter: name, JQL, description, favourite, owner
    Update {
        /// Filter ID, URL or exact name
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New JQL
        #[arg(long)]
        jql: Option<String>,
        #[command(flatten)]
        fields: FilterFields,
        /// Give the filter to this account ID
        #[arg(long)]
        owner: Option<String>,
    },
    /// Delete a filter
    Delete {
        /// Filter ID, URL or exact name
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Who a filter is shared with
    Share {
        #[command(subcommand)]
        cmd: ShareCmd,
    },
    /// The issue navigator columns of a filter
    Column {
        #[command(subcommand)]
        cmd: ColumnCmd,
    },
    /// Email subscriptions to a filter
    Subscription {
        #[command(subcommand)]
        cmd: SubscriptionCmd,
    },
}

#[derive(Args)]
pub(crate) struct FilterFields {
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// Make it one of your favourites (true or false)
    #[arg(long, value_name = "BOOL")]
    favourite: Option<bool>,
}

#[derive(Args)]
pub(crate) struct FilterArg {
    /// Filter ID, URL or exact name
    #[arg(long)]
    filter: String,
}

#[derive(Subcommand)]
pub(crate) enum ShareCmd {
    /// A filter's share permissions
    List {
        #[command(flatten)]
        filter: FilterArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Share a filter: a REST share permission such as
    /// {"type":"group","groupname":"dev"}
    Create {
        #[command(flatten)]
        filter: FilterArg,
        /// The share permission as JSON (`-` reads stdin)
        #[arg(long, value_name = "JSON")]
        permission_json: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Remove a share permission
    Delete {
        /// Share permission ID
        id: String,
        #[command(flatten)]
        filter: FilterArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum ColumnCmd {
    /// A filter's columns
    List {
        #[command(flatten)]
        filter: FilterArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Set a filter's columns
    Update {
        #[command(flatten)]
        filter: FilterArg,
        /// Field ID; repeat in display order
        #[arg(long, required = true)]
        column: Vec<String>,
    },
    /// Reset a filter's columns to the default
    Delete {
        #[command(flatten)]
        filter: FilterArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum SubscriptionCmd {
    /// A filter's subscriptions
    List {
        #[command(flatten)]
        filter: FilterArg,
        #[command(flatten)]
        paging: Paging,
    },
}

const FILTER_EXPAND: &str =
    "description,jql,owner,favourite,sharePermissions,editPermissions,viewUrl";

pub(super) fn run(c: &Client, cmd: FilterCmd) -> Result<Value> {
    let resolve = |r: &str| admin::resolve_filter(c, r);
    match cmd {
        FilterCmd::List {
            query,
            owner,
            group,
            project,
            order_by,
            override_share_permissions,
            favourite,
            paging,
        } => {
            if favourite {
                return paging.local(admin::favourite_filters(c)?);
            }
            let q = admin::FilterQuery {
                name: query.as_deref(),
                owner: owner.as_deref(),
                group: group.as_deref(),
                project_id: project.as_deref(),
                order_by: order_by.as_deref(),
                override_shares: override_share_permissions.then_some(true),
                ..Default::default()
            };
            Ok(listed(admin::filters(
                c,
                &q,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        FilterCmd::Get { ids, many } => many.run(ids, |id| {
            admin::filter_get(c, &resolve(id)?, Some(FILTER_EXPAND))
        }),
        FilterCmd::Create { name, jql, fields } => {
            let mut m = Map::new();
            m.insert("name".into(), name.into());
            m.insert("jql".into(), jql.into());
            if let Some(d) = fields.description {
                m.insert("description".into(), d.into());
            }
            if let Some(f) = fields.favourite {
                m.insert("favourite".into(), f.into());
            }
            admin::filter_create(c, m, Some(FILTER_EXPAND), None)
        }
        FilterCmd::Update {
            id,
            name,
            jql,
            fields,
            owner,
        } => {
            let id = resolve(&id)?;
            let mut m = Map::new();
            for (k, v) in [
                ("name", name),
                ("jql", jql),
                ("description", fields.description),
            ] {
                if let Some(v) = v {
                    m.insert(k.into(), v.into());
                }
            }
            if m.is_empty() && fields.favourite.is_none() && owner.is_none() {
                return Err(ApiError::usage("nothing to change: pass at least one field").into());
            }
            if !m.is_empty() || fields.favourite.is_some() {
                admin::filter_update(c, &id, m, fields.favourite)?;
            }
            if let Some(o) = owner {
                admin::filter_owner(c, &id, &o)?;
            }
            admin::filter_get(c, &id, Some(FILTER_EXPAND))
        }
        FilterCmd::Delete { id, confirm: _ } => admin::filter_delete(c, &resolve(&id)?, None),
        FilterCmd::Share { cmd } => match cmd {
            ShareCmd::List { filter, paging } => {
                paging.local(admin::filter_shares(c, &resolve(&filter.filter)?)?)
            }
            ShareCmd::Create {
                filter,
                permission_json,
                confirm: _,
            } => {
                let permission =
                    jira::json_value("--permission-json", &text_arg(permission_json)?)?;
                let created = admin::filter_share_add(c, &resolve(&filter.filter)?, &permission)?;
                Ok(created)
            }
            ShareCmd::Delete {
                id,
                filter,
                confirm: _,
            } => admin::filter_share_remove(c, &resolve(&filter.filter)?, &id),
        },
        FilterCmd::Column { cmd } => {
            match cmd {
                ColumnCmd::List { filter, paging } => {
                    paging.local(admin::filter_columns(c, &resolve(&filter.filter)?)?)
                }
                ColumnCmd::Update { filter, column } => Ok(Value::Array(
                    admin::filter_columns_set(c, &resolve(&filter.filter)?, &column)?,
                )),
                ColumnCmd::Delete { filter, confirm: _ } => {
                    admin::filter_columns_reset(c, &resolve(&filter.filter)?)
                }
            }
        }
        FilterCmd::Subscription {
            cmd: SubscriptionCmd::List { filter, paging },
        } => paging.local(admin::filter_subscriptions(c, &resolve(&filter.filter)?)?),
    }
}

//! `jira dashboard`, its gadgets and gadget item properties.

use super::{Confirm, Many, Paging, listed};
use crate::cli::util::text_arg;
use crate::client::{ApiError, Client};
use crate::jira::{self, dashboards};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum DashboardCmd {
    /// Dashboards, by name, owner, group or project
    List {
        /// Name contains
        #[arg(short, long)]
        query: Option<String>,
        /// Owner's account ID (`me` for yours)
        #[arg(long)]
        owner: Option<String>,
        /// Shared with this group (name)
        #[arg(long)]
        group: Option<String>,
        /// Shared with this project (ID)
        #[arg(long)]
        project: Option<String>,
        /// Sort, e.g. name or -popularity
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Dashboards by ID
    Get {
        /// Dashboard IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a dashboard (private unless permissions are given)
    Create {
        #[command(flatten)]
        edit: EditArgs,
    },
    /// Change a dashboard's name, description or permissions (the update
    /// replaces its permissions, so it needs --yes)
    Update {
        /// Dashboard ID
        id: String,
        #[command(flatten)]
        edit: EditArgs,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Copy a dashboard
    Copy {
        /// Dashboard ID
        id: String,
        #[command(flatten)]
        edit: EditArgs,
    },
    /// Delete a dashboard
    Delete {
        /// Dashboard ID
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Gadgets on a dashboard
    Gadget {
        #[command(subcommand)]
        cmd: GadgetCmd,
    },
    /// Properties of a dashboard item (gadget)
    Property {
        #[command(subcommand)]
        cmd: PropertyCmd,
    },
}

#[derive(Args)]
pub(crate) struct EditArgs {
    /// Name
    #[arg(long)]
    name: Option<String>,
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// Share permissions: a REST JSON array
    #[arg(long, value_name = "JSON")]
    share_permissions_json: Option<String>,
    /// Edit permissions: a REST JSON array
    #[arg(long, value_name = "JSON")]
    edit_permissions_json: Option<String>,
    /// Admins: act on dashboards you could not otherwise change
    #[arg(long)]
    extend_admin_permissions: bool,
}

impl EditArgs {
    fn parse(&self) -> Result<(Option<Value>, Option<Value>)> {
        let j = |flag: &str, v: &Option<String>| -> Result<Option<Value>> {
            v.as_deref().map(|t| jira::json_value(flag, t)).transpose()
        };
        Ok((
            j("--share-permissions-json", &self.share_permissions_json)?,
            j("--edit-permissions-json", &self.edit_permissions_json)?,
        ))
    }
}

#[derive(Args)]
pub(crate) struct DashboardArg {
    /// Dashboard ID
    #[arg(long)]
    dashboard: String,
}

#[derive(Args)]
pub(crate) struct GadgetFields {
    /// REST gadget object; the flags below are laid over it
    #[arg(long, value_name = "JSON")]
    gadget_json: Option<String>,
    /// Title
    #[arg(long)]
    title: Option<String>,
    /// blue, red, yellow, green, cyan, purple, gray or white
    #[arg(long)]
    color: Option<String>,
    /// Row (with --column)
    #[arg(long, requires = "column")]
    row: Option<u32>,
    /// Column (with --row)
    #[arg(long, requires = "row")]
    column: Option<u32>,
}

impl GadgetFields {
    fn gadget(&self) -> Result<dashboards::Gadget<'_>> {
        Ok(dashboards::Gadget {
            json: self
                .gadget_json
                .as_deref()
                .map(|t| jira::json_value("--gadget-json", t))
                .transpose()?,
            title: self.title.as_deref(),
            color: self.color.as_deref(),
            row: self.row,
            column: self.column,
        })
    }
}

#[derive(Subcommand)]
pub(crate) enum GadgetCmd {
    /// A dashboard's gadgets, or (with --catalog) those that can be added
    List {
        /// Dashboard ID
        #[arg(long, required_unless_present = "catalog")]
        dashboard: Option<String>,
        /// The gadgets available to add instead
        #[arg(long, conflicts_with = "dashboard")]
        catalog: bool,
        /// Only gadgets with this module key; repeatable
        #[arg(long, requires = "dashboard")]
        module_key: Vec<String>,
        /// Only gadgets with this URI; repeatable
        #[arg(long, requires = "dashboard")]
        uri: Vec<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add a gadget by module key or URI
    Create {
        #[command(flatten)]
        dashboard: DashboardArg,
        /// The gadget's module key
        #[arg(long, conflicts_with = "uri")]
        module_key: Option<String>,
        /// The gadget's spec URI
        #[arg(long)]
        uri: Option<String>,
        /// Skip Jira's check of the module key or URI
        #[arg(long)]
        skip_validation: bool,
        #[command(flatten)]
        fields: GadgetFields,
    },
    /// Change a gadget's title, colour or position
    Update {
        /// Gadget ID
        id: String,
        #[command(flatten)]
        dashboard: DashboardArg,
        #[command(flatten)]
        fields: GadgetFields,
    },
    /// Remove a gadget
    Delete {
        /// Gadget ID
        id: String,
        #[command(flatten)]
        dashboard: DashboardArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Args)]
pub(crate) struct ItemArgs {
    /// Dashboard ID
    #[arg(long)]
    dashboard: String,
    /// Item (gadget) ID
    #[arg(long)]
    item: String,
}

#[derive(Subcommand)]
pub(crate) enum PropertyCmd {
    /// An item's property keys
    List {
        #[command(flatten)]
        item: ItemArgs,
        #[command(flatten)]
        paging: Paging,
    },
    /// One property's value
    Get {
        /// Property key
        key: String,
        #[command(flatten)]
        item: ItemArgs,
    },
    /// Create or replace a property
    Update {
        /// Property key
        key: String,
        #[command(flatten)]
        item: ItemArgs,
        /// The value as JSON (`-` reads stdin)
        #[arg(long, allow_hyphen_values = true)]
        value: String,
    },
    /// Delete a property
    Delete {
        /// Property key
        key: String,
        #[command(flatten)]
        item: ItemArgs,
        #[command(flatten)]
        confirm: Confirm,
    },
}

pub(super) fn run(c: &Client, cmd: DashboardCmd) -> Result<Value> {
    match cmd {
        DashboardCmd::List {
            query,
            owner,
            group,
            project,
            order_by,
            paging,
        } => {
            let owner = owner.map(|o| jira::resolve_account(c, &o)).transpose()?;
            let q = dashboards::Search {
                name: query.as_deref(),
                account_id: owner.as_deref(),
                group_name: group.as_deref(),
                project_id: project.as_deref(),
                order_by: order_by.as_deref(),
                ..Default::default()
            };
            Ok(listed(dashboards::list(
                c,
                &q,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        DashboardCmd::Get { ids, many } => many.run(ids, |id| dashboards::get(c, id)),
        DashboardCmd::Create { edit } => {
            let (share, perms) = edit.parse()?;
            dashboards::create(c, &edit_of(&edit, share, perms))
        }
        DashboardCmd::Update {
            id,
            edit,
            confirm: _,
        } => {
            let (share, perms) = edit.parse()?;
            dashboards::update(c, &id, &edit_of(&edit, share, perms))
        }
        DashboardCmd::Copy { id, edit } => {
            let (share, perms) = edit.parse()?;
            dashboards::copy(c, &id, &edit_of(&edit, share, perms))
        }
        DashboardCmd::Delete { id, confirm: _ } => dashboards::delete(c, &id),
        DashboardCmd::Gadget { cmd } => match cmd {
            GadgetCmd::List {
                dashboard,
                catalog,
                module_key,
                uri,
                paging,
            } => {
                let all = if catalog {
                    dashboards::catalog(c)?
                } else {
                    let d = dashboard.ok_or_else(|| ApiError::usage("pass --dashboard"))?;
                    dashboards::gadgets(c, &d, &module_key, &uri)?
                };
                paging.local(all)
            }
            GadgetCmd::Create {
                dashboard,
                module_key,
                uri,
                skip_validation,
                fields,
            } => dashboards::gadget_add(
                c,
                &dashboard.dashboard,
                &fields.gadget()?,
                module_key.as_deref(),
                uri.as_deref(),
                skip_validation.then_some(true),
            ),
            GadgetCmd::Update {
                id,
                dashboard,
                fields,
            } => dashboards::gadget_update(c, &dashboard.dashboard, &id, &fields.gadget()?),
            GadgetCmd::Delete {
                id,
                dashboard,
                confirm: _,
            } => dashboards::gadget_delete(c, &dashboard.dashboard, &id),
        },
        DashboardCmd::Property { cmd } => match cmd {
            PropertyCmd::List { item, paging } => {
                paging.local(dashboards::properties(c, &item.dashboard, &item.item)?)
            }
            PropertyCmd::Get { key, item } => {
                dashboards::property(c, &item.dashboard, &item.item, &key)
            }
            PropertyCmd::Update { key, item, value } => {
                let value = jira::json_value("--value", &text_arg(value)?)?;
                dashboards::property_set(c, &item.dashboard, &item.item, &key, &value)
            }
            PropertyCmd::Delete {
                key,
                item,
                confirm: _,
            } => dashboards::property_delete(c, &item.dashboard, &item.item, &key),
        },
    }
}

fn edit_of<'a>(
    e: &'a EditArgs,
    share: Option<Value>,
    perms: Option<Value>,
) -> dashboards::Edit<'a> {
    dashboards::Edit {
        name: e.name.as_deref(),
        description: e.description.as_deref(),
        share,
        edit: perms,
        admin: e.extend_admin_permissions.then_some(true),
    }
}

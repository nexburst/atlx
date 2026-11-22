//! `jira dashboard`.

use crate::cli::util::{require_yes, text_arg};
use crate::client::{ApiError, Client};
use crate::jira::{self, dashboards};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Subcommand)]
pub(crate) enum DashboardCmd {
    /// Search dashboards
    Query(QueryArgs),
    /// One dashboard
    Get {
        /// Dashboard ID, ARI or URL
        id: String,
    },
    /// Create a dashboard, private unless shares are given
    Create(WriteArgs),
    /// Change a dashboard, keeping what isn't given
    Update {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        id: String,
        #[command(flatten)]
        set: WriteArgs,
    },
    /// Copy a dashboard, private unless shares are given
    Copy {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        id: String,
        #[command(flatten)]
        set: WriteArgs,
    },
    /// Delete a dashboard
    Delete {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        id: String,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Change the owner or permissions of several dashboards
    BulkEdit(BulkArgs),
    /// Gadgets on a dashboard
    Gadget {
        #[command(subcommand)]
        cmd: GadgetCmd,
    },
    /// Properties of a dashboard item
    ItemProperty {
        #[command(subcommand)]
        cmd: ItemPropertyCmd,
    },
}

#[derive(Args)]
pub(crate) struct QueryArgs {
    /// Text in the name
    #[arg(long)]
    dashboard_name: Option<String>,
    /// The owner's account ID
    #[arg(long, conflicts_with = "owner")]
    account_id: Option<String>,
    /// The owner's username (deprecated by Jira)
    #[arg(long)]
    owner: Option<String>,
    /// Shared with this group (name)
    #[arg(long, conflicts_with = "group_id")]
    group_name: Option<String>,
    /// Shared with this group (ID)
    #[arg(long)]
    group_id: Option<String>,
    /// Shared with this space (ID)
    #[arg(long)]
    project_id: Option<String>,
    /// Sort field, e.g. name or -id
    #[arg(long, allow_hyphen_values = true)]
    order_by: Option<String>,
    /// REST expand value
    #[arg(long)]
    expand: Option<String>,
    /// Dashboards to return
    #[arg(long, default_value_t = 50)]
    first: usize,
    /// Zero-based offset of the first dashboard
    #[arg(long, default_value_t = 0)]
    start_at: usize,
}

#[derive(Args)]
pub(crate) struct WriteArgs {
    /// Dashboard name
    #[arg(long)]
    name: Option<String>,
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// REST share permissions array, or - for stdin
    #[arg(long)]
    share_permissions_json: Option<String>,
    /// REST edit permissions array, or - for stdin
    #[arg(long)]
    edit_permissions_json: Option<String>,
    /// Admins: act on dashboards you can't edit (true or false)
    #[arg(long, value_name = "BOOL")]
    extend_admin_permissions: Option<bool>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum EditAction {
    #[value(name = "changeOwner")]
    ChangeOwner,
    #[value(name = "changePermission")]
    ChangePermission,
    #[value(name = "addPermission")]
    AddPermission,
    #[value(name = "removePermission")]
    RemovePermission,
}

impl EditAction {
    fn name(self) -> &'static str {
        match self {
            EditAction::ChangeOwner => "changeOwner",
            EditAction::ChangePermission => "changePermission",
            EditAction::AddPermission => "addPermission",
            EditAction::RemovePermission => "removePermission",
        }
    }
}

#[derive(Args)]
pub(crate) struct BulkArgs {
    /// Dashboard ID, ARI or URL; repeatable
    #[arg(long, required = true)]
    id: Vec<String>,
    /// What to do
    #[arg(long, value_enum)]
    action: EditAction,
    /// REST changeOwnerDetails object
    #[arg(long)]
    change_owner_details_json: Option<String>,
    /// REST permissionDetails object
    #[arg(long)]
    permission_details_json: Option<String>,
    /// Admins: act on dashboards you can't edit (true or false)
    #[arg(long, value_name = "BOOL")]
    extend_admin_permissions: Option<bool>,
    /// Confirm the change (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Subcommand)]
pub(crate) enum GadgetCmd {
    /// The gadgets on a dashboard
    Query {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        dashboard_id: String,
        /// Only this gadget; repeatable
        #[arg(long)]
        gadget_id: Vec<String>,
        /// Only this module key; repeatable
        #[arg(long)]
        module_key: Vec<String>,
        /// Only this URI; repeatable
        #[arg(long)]
        uri: Vec<String>,
    },
    /// Gadgets that can be added
    Catalog {
        #[command(subcommand)]
        cmd: CatalogCmd,
    },
    /// Add a gadget
    Add(GadgetAddArgs),
    /// Change a gadget's title, color or place
    Update {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        dashboard_id: String,
        /// Gadget ID
        #[arg(long)]
        gadget_id: String,
        #[command(flatten)]
        set: GadgetArgs,
    },
    /// Remove a gadget
    Delete {
        /// Dashboard ID, ARI or URL
        #[arg(long)]
        dashboard_id: String,
        /// Gadget ID
        #[arg(long)]
        gadget_id: String,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum CatalogCmd {
    /// List them
    Query,
}

#[derive(Args)]
pub(crate) struct GadgetArgs {
    /// REST gadget object; the flags win over it
    #[arg(long)]
    gadget_json: Option<String>,
    /// Title
    #[arg(long)]
    title: Option<String>,
    /// blue, red, yellow, green, cyan, purple, gray or white
    #[arg(long)]
    color: Option<String>,
    /// Zero-based row, with --column
    #[arg(long, requires = "column")]
    row: Option<u32>,
    /// Zero-based column, with --row
    #[arg(long, requires = "row")]
    column: Option<u32>,
}

#[derive(Args)]
pub(crate) struct GadgetAddArgs {
    /// Dashboard ID, ARI or URL
    #[arg(long)]
    dashboard_id: String,
    /// The gadget's module key
    #[arg(long, conflicts_with = "uri")]
    module_key: Option<String>,
    /// The gadget's URI
    #[arg(long)]
    uri: Option<String>,
    /// Skip checking the key or URI (true or false)
    #[arg(long, value_name = "BOOL")]
    ignore_uri_and_module_key_validation: Option<bool>,
    #[command(flatten)]
    set: GadgetArgs,
}

#[derive(Subcommand)]
pub(crate) enum ItemPropertyCmd {
    /// An item's property keys
    Query(ItemRef),
    /// One property's value
    Get(PropertyRef),
    /// Create or replace a property
    Set {
        #[command(flatten)]
        at: PropertyRef,
        /// The value as JSON, or - for stdin
        #[arg(long, allow_hyphen_values = true)]
        value_json: String,
    },
    /// Delete a property
    Delete {
        #[command(flatten)]
        at: PropertyRef,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Args)]
pub(crate) struct ItemRef {
    /// Dashboard ID, ARI or URL
    #[arg(long)]
    dashboard_id: String,
    /// Item ID
    #[arg(long)]
    item_id: String,
}

#[derive(Args)]
pub(crate) struct PropertyRef {
    #[command(flatten)]
    item: ItemRef,
    /// Property key
    #[arg(long)]
    property_key: String,
}

fn parse(flag: &str, text: Option<String>) -> Result<Option<Value>> {
    text.map(|t| {
        serde_json::from_str(&text_arg(t)?)
            .map_err(|e| ApiError::usage(format!("{flag} is not valid JSON: {e}")).into())
    })
    .transpose()
}

impl WriteArgs {
    fn edit(&self) -> Result<dashboards::Edit<'_>> {
        Ok(dashboards::Edit {
            name: self.name.as_deref(),
            description: self.description.as_deref(),
            share: parse(
                "--share-permissions-json",
                self.share_permissions_json.clone(),
            )?,
            edit: parse(
                "--edit-permissions-json",
                self.edit_permissions_json.clone(),
            )?,
            admin: self.extend_admin_permissions,
        })
    }
}

impl GadgetArgs {
    fn gadget(&self) -> Result<dashboards::Gadget<'_>> {
        Ok(dashboards::Gadget {
            json: parse("--gadget-json", self.gadget_json.clone())?,
            title: self.title.as_deref(),
            color: self.color.as_deref(),
            row: self.row,
            column: self.column,
        })
    }
}

pub(super) fn run(c: &Client, cmd: DashboardCmd) -> Result<Value> {
    let (name, request, data) = match cmd {
        DashboardCmd::Query(a) => {
            let o = dashboards::Search {
                name: a.dashboard_name.as_deref(),
                account_id: a.account_id.as_deref(),
                owner: a.owner.as_deref(),
                group_name: a.group_name.as_deref(),
                group_id: a.group_id.as_deref(),
                project_id: a.project_id.as_deref(),
                order_by: a.order_by.as_deref(),
                expand: a.expand.as_deref(),
                start: a.start_at,
                limit: a.first,
            };
            let request =
                json!({"first": a.first, "startAt": a.start_at, "dashboardName": a.dashboard_name});
            ("query", request, dashboards::query(c, &o)?)
        }
        DashboardCmd::Get { id } => ("get", json!({"id": id}), dashboards::get(c, &id)?),
        DashboardCmd::Create(set) => {
            let data = dashboards::create(c, &set.edit()?)?;
            ("create", json!({"name": set.name}), data)
        }
        DashboardCmd::Update { id, set } => {
            let data = dashboards::update(c, &id, &set.edit()?)?;
            ("update", json!({"id": id}), data)
        }
        DashboardCmd::Copy { id, set } => {
            let data = dashboards::copy(c, &id, &set.edit()?)?;
            ("copy", json!({"id": id, "name": set.name}), data)
        }
        DashboardCmd::Delete { id, yes } => {
            require_yes(yes, "deleting the dashboard")?;
            ("delete", json!({"id": id}), dashboards::delete(c, &id)?)
        }
        DashboardCmd::BulkEdit(a) => {
            require_yes(a.yes, "the bulk edit")?;
            let action = a.action.name();
            let data = dashboards::bulk_edit(
                c,
                &a.id,
                action,
                parse("--change-owner-details-json", a.change_owner_details_json)?,
                parse("--permission-details-json", a.permission_details_json)?,
                a.extend_admin_permissions,
            )?;
            let errors = &data["entityErrors"];
            if errors.as_object().is_some_and(|m| !m.is_empty()) {
                crate::cli::util::mark_partial();
            }
            ("bulk-edit", json!({"ids": a.id, "action": action}), data)
        }
        DashboardCmd::Gadget { cmd } => gadget(c, cmd)?,
        DashboardCmd::ItemProperty { cmd } => property(c, cmd)?,
    };
    let mut request = request;
    request["site"] = jira::site(c).into();
    let kind = match name {
        "query" => "jira.dashboard-search-result",
        "gadget.query" => "jira.dashboard-gadget",
        "gadget.catalog.query" => "jira.dashboard-gadget-catalog",
        "item-property.query" => "jira.dashboard-item-property",
        _ => "jira.dashboard",
    };
    Ok(envelope(&format!("jira.dashboard.{name}"), kind)
        .request(request)
        .data(data))
}

fn gadget(c: &Client, cmd: GadgetCmd) -> Result<(&'static str, Value, Value)> {
    Ok(match cmd {
        GadgetCmd::Catalog {
            cmd: CatalogCmd::Query,
        } => ("gadget.catalog.query", json!({}), dashboards::catalog(c)?),
        GadgetCmd::Query {
            dashboard_id: id,
            gadget_id,
            module_key,
            uri,
        } => {
            let data = dashboards::gadgets(c, &id, &module_key, &uri, &gadget_id)?;
            ("gadget.query", json!({"dashboardId": id}), data)
        }
        GadgetCmd::Add(a) => {
            let data = dashboards::gadget_add(
                c,
                &a.dashboard_id,
                &a.set.gadget()?,
                a.module_key.as_deref(),
                a.uri.as_deref(),
                a.ignore_uri_and_module_key_validation,
            )?;
            ("gadget.add", json!({"dashboardId": a.dashboard_id}), data)
        }
        GadgetCmd::Update {
            dashboard_id: id,
            gadget_id,
            set,
        } => {
            let data = dashboards::gadget_update(c, &id, &gadget_id, &set.gadget()?)?;
            (
                "gadget.update",
                json!({"dashboardId": id, "gadgetId": gadget_id}),
                data,
            )
        }
        GadgetCmd::Delete {
            dashboard_id: id,
            gadget_id,
            yes,
        } => {
            require_yes(yes, "removing the gadget")?;
            let data = dashboards::gadget_delete(c, &id, &gadget_id)?;
            (
                "gadget.delete",
                json!({"dashboardId": id, "gadgetId": gadget_id}),
                data,
            )
        }
    })
}

fn property(c: &Client, cmd: ItemPropertyCmd) -> Result<(&'static str, Value, Value)> {
    let request = |i: &ItemRef, key: Option<&str>| json!({"dashboardId": i.dashboard_id, "itemId": i.item_id, "propertyKey": key});
    Ok(match cmd {
        ItemPropertyCmd::Query(i) => {
            let data = dashboards::properties(c, &i.dashboard_id, &i.item_id)?;
            ("item-property.query", request(&i, None), data)
        }
        ItemPropertyCmd::Get(at) => {
            let (i, key) = (&at.item, at.property_key.as_str());
            let data = dashboards::property(c, &i.dashboard_id, &i.item_id, key)?;
            ("item-property.get", request(i, Some(key)), data)
        }
        ItemPropertyCmd::Set { at, value_json } => {
            let (i, key) = (&at.item, at.property_key.as_str());
            let value = parse("--value-json", Some(value_json))?.unwrap_or(Value::Null);
            let data = dashboards::property_set(c, &i.dashboard_id, &i.item_id, key, &value)?;
            ("item-property.set", request(i, Some(key)), data)
        }
        ItemPropertyCmd::Delete { at, yes } => {
            require_yes(yes, "deleting the property")?;
            let (i, key) = (&at.item, at.property_key.as_str());
            let data = dashboards::property_delete(c, &i.dashboard_id, &i.item_id, key)?;
            ("item-property.delete", request(i, Some(key)), data)
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    fn parses(args: &str) -> bool {
        Cli::try_parse_from(["atlx"].into_iter().chain(args.split_whitespace())).is_ok()
    }

    #[test]
    fn parses_dashboard_commands() {
        for args in [
            "jira dashboard query --dashboard-name Team --order-by -name --first 5 --start-at 5",
            "jira dashboard get 10000",
            "jira dashboard create --name Team --share-permissions-json []",
            "jira dashboard update --id 10000 --description New --extend-admin-permissions true",
            "jira dashboard copy --id 10000 --name Copy",
            "jira dashboard delete --id 10000 --yes",
            "jira dashboard bulk-edit --id 1 --id 2 --action changeOwner --change-owner-details-json {} --yes",
            "jira dashboard gadget catalog query",
            "jira dashboard gadget query --dashboard-id 1 --module-key k",
            "jira dashboard gadget add --dashboard-id 1 --uri /x --row 0 --column 1",
            "jira dashboard gadget update --dashboard-id 1 --gadget-id 2 --title T",
            "jira dashboard gadget delete --dashboard-id 1 --gadget-id 2 --yes",
            "jira dashboard item-property query --dashboard-id 1 --item-id 2",
            "jira dashboard item-property get --dashboard-id 1 --item-id 2 --property-key k",
            "jira dashboard item-property set --dashboard-id 1 --item-id 2 --property-key k --value-json -1",
            "jira dashboard item-property delete --dashboard-id 1 --item-id 2 --property-key k --yes",
        ] {
            assert!(parses(args), "{args}");
        }
        for args in [
            "jira dashboard bulk-edit --id 1 --action delete",
            "jira dashboard gadget add --dashboard-id 1 --row 0",
            "jira dashboard gadget add --dashboard-id 1 --uri /x --module-key k",
        ] {
            assert!(!parses(args), "{args}");
        }
    }
}

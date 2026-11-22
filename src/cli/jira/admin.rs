//! `jira space` administration and `jira filter`.

use super::{offset_envelope, space_ref};
use crate::cli::util::require_yes;
use crate::client::{ApiError, Client};
use crate::jira::{self, admin};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// A REST body from the flags that were given.
fn body<const N: usize>(fields: [(&str, Option<Value>); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
        .collect()
}

fn json_arg(flag: &str, text: Option<&str>) -> Result<Option<Value>> {
    text.map(|t| {
        serde_json::from_str(t).map_err(|e| usage(format!("{flag} is not valid JSON: {e}")))
    })
    .transpose()
}

/// A category ID, where -1 clears it.
fn category(v: Option<&str>) -> Result<Option<Value>> {
    v.map(|v| {
        v.trim()
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| usage("--category-id takes a number"))
    })
    .transpose()
}

fn out(c: &Client, command: &str, kind: &str, request: Value, data: Value) -> Result<Value> {
    let mut request = request;
    request["site"] = jira::site(c).into();
    Ok(
        envelope(&format!("jira.{command}"), &format!("jira.{kind}"))
            .request(request)
            .data(data),
    )
}

#[derive(Args)]
pub(crate) struct SpaceTarget {
    /// Space key, ID, ARI or /projects/ URL
    #[arg(long)]
    id_or_key: String,
}

#[derive(Args)]
pub(crate) struct ConfirmedSpace {
    /// Space key, ID, ARI or /projects/ URL; names and work items are refused
    #[arg(long)]
    id_or_key: String,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct IssueTypesArgs {
    /// Space key, ID, ARI or URL
    #[arg(long)]
    id_or_key: String,
    /// Most issue types to return
    #[arg(long, default_value_t = 50)]
    first: usize,
}

#[derive(Args)]
pub(crate) struct SpaceFields {
    /// Project lead's account ID
    #[arg(long)]
    lead_account_id: Option<String>,
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// A link shown with the space
    #[arg(long)]
    url: Option<String>,
    /// PROJECT_LEAD or UNASSIGNED
    #[arg(long)]
    assignee_type: Option<String>,
    /// Category ID; -1 removes it
    #[arg(long, allow_hyphen_values = true)]
    category_id: Option<String>,
}

#[derive(Args)]
pub(crate) struct SpaceCreateArgs {
    /// Key of the new space
    #[arg(long)]
    key: String,
    /// Name
    #[arg(long)]
    name: String,
    /// business, service_desk or software
    #[arg(long)]
    project_type_key: Option<String>,
    /// Project template key
    #[arg(long)]
    project_template_key: Option<String>,
    #[command(flatten)]
    fields: SpaceFields,
}

#[derive(Args)]
pub(crate) struct SpaceUpdateArgs {
    /// Space key, ID, ARI or /projects/ URL
    #[arg(long)]
    id_or_key: String,
    /// New name
    #[arg(long)]
    name: Option<String>,
    /// New key
    #[arg(long)]
    key: Option<String>,
    /// REST expand value
    #[arg(long)]
    expand: Option<String>,
    #[command(flatten)]
    fields: SpaceFields,
}

#[derive(Subcommand)]
pub(crate) enum StatusCmd {
    /// The statuses of each issue type in a space
    Query(SpaceTarget),
}

#[derive(Subcommand)]
pub(crate) enum SpaceTypesCmd {
    /// Space (project) types: all, or the given ones
    Query {
        /// Type key or ARI; repeatable
        #[arg(long)]
        id: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum NotificationSchemeCmd {
    /// A space's notification scheme
    Get {
        /// Space key or ID
        #[arg(long)]
        key: String,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum VersionsCmd {
    /// One page of a space's versions
    Query {
        /// Space key or ID
        #[arg(long, alias = "space")]
        key: String,
        /// Name or description contains
        #[arg(long, alias = "query")]
        search_string: Option<String>,
        /// Comma-separated: released, unreleased, archived
        #[arg(long, alias = "status")]
        filter: Option<String>,
        /// Sort field, e.g. name or -releaseDate
        #[arg(long, allow_hyphen_values = true)]
        sort_by: Option<String>,
        /// Cursor from the previous page's pageInfo.nextCursor
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Args)]
pub(crate) struct VersionFields {
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// YYYY-MM-DD
    #[arg(long)]
    start_date: Option<String>,
    /// YYYY-MM-DD
    #[arg(long)]
    release_date: Option<String>,
    /// true or false
    #[arg(long, value_name = "BOOL")]
    released: Option<bool>,
    /// true or false
    #[arg(long, value_name = "BOOL")]
    archived: Option<bool>,
}

#[derive(Subcommand)]
pub(crate) enum VersionCmd {
    /// Create a version
    Create {
        /// Space key or ID
        #[arg(long)]
        key: String,
        /// Name
        #[arg(long)]
        name: String,
        #[command(flatten)]
        fields: VersionFields,
    },
    /// One version
    Get {
        /// Version ID
        id: String,
    },
    /// Change a version
    Update {
        /// Version ID
        #[arg(long)]
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        fields: VersionFields,
    },
    /// Delete a version
    Delete {
        /// Version ID
        #[arg(long)]
        id: String,
        /// Version to give its fix-version work items
        #[arg(long)]
        move_fix_issues_to: Option<String>,
        /// Version to give its affected-version work items
        #[arg(long)]
        move_affected_issues_to: Option<String>,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
    },
    /// Delete a version, moving its work items to another
    Merge {
        /// Version to delete
        #[arg(long)]
        id: String,
        /// Version that receives the work items
        #[arg(long)]
        move_issues_to: String,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
    },
    /// Reorder a version
    Move {
        /// Version ID
        #[arg(long)]
        id: String,
        /// Put it after this version (URL or ID)
        #[arg(
            long,
            conflicts_with = "position",
            required_unless_present = "position"
        )]
        after: Option<String>,
        /// First, Last, Earlier or Later
        #[arg(long, value_parser = ["First", "Last", "Earlier", "Later"])]
        position: Option<String>,
    },
    /// How many work items use a version
    Counts {
        /// Version ID
        #[arg(long)]
        id: String,
    },
}

#[derive(Args)]
pub(crate) struct ComponentFields {
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// Lead's account ID
    #[arg(long)]
    lead_account_id: Option<String>,
    /// PROJECT_DEFAULT, COMPONENT_LEAD, PROJECT_LEAD or UNASSIGNED
    #[arg(long)]
    assignee_type: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum ComponentCmd {
    /// A space's components
    Query {
        /// Space key or ID
        #[arg(long, alias = "space")]
        key: String,
    },
    /// One component
    Get {
        /// Component ID
        id: String,
    },
    /// Create a component
    Create {
        /// Space key or ID
        #[arg(long)]
        key: String,
        /// Name
        #[arg(long)]
        name: String,
        #[command(flatten)]
        fields: ComponentFields,
    },
    /// Change a component
    Update {
        /// Component ID
        #[arg(long)]
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        fields: ComponentFields,
    },
    /// Delete a component
    Delete {
        /// Component ID
        #[arg(long)]
        id: String,
        /// Component to give its work items
        #[arg(long)]
        move_issues_to: Option<String>,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
    },
    /// How many work items use a component
    Counts {
        /// Component ID
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum FilterCmd {
    /// Search saved filters
    Query {
        /// Name contains
        #[arg(long, alias = "name")]
        filter_name: Option<String>,
        /// Owner: me, an email, a name or an account ID
        #[arg(long, conflicts_with = "owner_account_id")]
        owner: Option<String>,
        /// Owner's account ID
        #[arg(long)]
        owner_account_id: Option<String>,
        /// Shared with this group (name)
        #[arg(long, conflicts_with = "group_id")]
        group_name: Option<String>,
        /// Shared with this group (ID)
        #[arg(long)]
        group_id: Option<String>,
        /// Shared with this space (ID)
        #[arg(long)]
        project_id: Option<String>,
        /// These filters; repeatable
        #[arg(long)]
        filter_id: Vec<String>,
        /// Sort field, e.g. name or -id
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
        /// Admins: see filters not shared with you (true or false)
        #[arg(long, value_name = "BOOL")]
        override_share_permissions: Option<bool>,
        /// Only your favourites (atlx extra)
        #[arg(long, conflicts_with_all = ["filter_name", "owner", "owner_account_id", "group_name", "group_id", "project_id", "filter_id", "order_by", "expand", "start_at"])]
        favourite: bool,
        #[arg(long, default_value_t = 50)]
        first: usize,
        /// Zero-based offset of the first filter
        #[arg(long)]
        start_at: Option<usize>,
    },
    /// One filter
    Get {
        /// Filter ID, ARI, URL or exact name
        id: String,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
    /// Save a new filter
    Create {
        /// Name
        #[arg(long)]
        name: String,
        /// JQL
        #[arg(long)]
        jql: Option<String>,
        /// Description
        #[arg(long)]
        description: Option<String>,
        /// true or false
        #[arg(long, value_name = "BOOL")]
        favourite: Option<bool>,
        /// REST share permissions array
        #[arg(long, value_name = "JSON")]
        share_permissions_json: Option<String>,
        /// REST edit permissions array
        #[arg(long, value_name = "JSON")]
        edit_permissions_json: Option<String>,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
        /// Admins: skip share permission checks (true or false)
        #[arg(long, value_name = "BOOL")]
        override_share_permissions: Option<bool>,
    },
    /// Change a filter; what you don't pass is kept
    Update {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New JQL
        #[arg(long)]
        jql: Option<String>,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// true or false
        #[arg(long, value_name = "BOOL")]
        favourite: Option<bool>,
        /// Replaces the share permissions
        #[arg(long, value_name = "JSON")]
        share_permissions_json: Option<String>,
        /// Replaces the edit permissions
        #[arg(long, value_name = "JSON")]
        edit_permissions_json: Option<String>,
    },
    /// Delete a filter
    Delete {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// Admins: skip share permission checks (true or false)
        #[arg(long, value_name = "BOOL")]
        override_share_permissions: Option<bool>,
        /// Confirm the delete (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Make a filter a favourite
    AddFavourite {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
    },
    /// Stop a filter being a favourite
    RemoveFavourite {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// REST expand value
        #[arg(long)]
        expand: Option<String>,
    },
    /// Give a filter another owner
    ChangeOwner {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// New owner's account ID
        #[arg(long)]
        account_id: String,
    },
    /// A filter's columns
    Columns {
        #[command(subcommand)]
        cmd: ColumnsCmd,
    },
    /// Go back to the default columns
    ResetColumns {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// Confirm the reset (required)
        #[arg(long, short)]
        yes: bool,
    },
    /// Who a filter is shared with
    Share {
        #[command(subcommand)]
        cmd: ShareCmd,
    },
    /// A filter's subscriptions
    Subscription {
        #[command(subcommand)]
        cmd: SubscriptionCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum ColumnsCmd {
    /// Get the columns
    Get {
        /// Filter ID, ARI, URL or exact name
        id: String,
    },
    /// Set the columns, in order
    Set {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// Field ID; repeat in column order
        #[arg(long, required = true)]
        column: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ShareCmd {
    /// List share permissions
    Query {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
    },
    /// Share a filter
    Add {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// REST share permission, e.g. {"type":"project","projectId":"10000"}
        #[arg(long, value_name = "JSON")]
        permission_json: String,
    },
    /// Stop a share
    Remove {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        /// Share permission ID
        #[arg(long)]
        permission_id: String,
        /// Confirm the removal (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum SubscriptionCmd {
    /// List subscriptions
    Query {
        /// Filter ID, ARI, URL or exact name
        #[arg(long)]
        id: String,
        #[arg(long, default_value_t = 50)]
        first: usize,
        /// Zero-based offset
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
}

pub(super) fn space_create(c: &Client, a: SpaceCreateArgs) -> Result<Value> {
    let f = a.fields;
    let data = admin::space_create(
        c,
        body([
            ("key", Some(a.key.as_str().into())),
            ("name", Some(a.name.into())),
            ("projectTypeKey", a.project_type_key.map(Value::from)),
            (
                "projectTemplateKey",
                a.project_template_key.map(Value::from),
            ),
            ("leadAccountId", f.lead_account_id.map(Value::from)),
            ("description", f.description.map(Value::from)),
            ("url", f.url.map(Value::from)),
            ("assigneeType", f.assignee_type.map(Value::from)),
            ("categoryId", category(f.category_id.as_deref())?),
        ]),
    )?;
    out(c, "space.create", "space", json!({"key": a.key}), data)
}

pub(super) fn space_update(c: &Client, a: SpaceUpdateArgs) -> Result<Value> {
    let f = a.fields;
    let fields = body([
        ("key", a.key.map(Value::from)),
        ("name", a.name.map(Value::from)),
        ("leadAccountId", f.lead_account_id.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("url", f.url.map(Value::from)),
        ("assigneeType", f.assignee_type.map(Value::from)),
        ("categoryId", category(f.category_id.as_deref())?),
    ]);
    let data = admin::space_update(c, &a.id_or_key, fields, a.expand.as_deref())?;
    out(
        c,
        "space.update",
        "space",
        json!({"idOrKey": a.id_or_key}),
        data,
    )
}

pub(super) fn space_delete(c: &Client, a: ConfirmedSpace) -> Result<Value> {
    require_yes(a.yes, "deleting the space")?;
    let data = admin::space_delete(c, &a.id_or_key)?;
    out(
        c,
        "space.delete",
        "space",
        json!({"idOrKey": a.id_or_key}),
        data,
    )
}

pub(super) fn space_archive(c: &Client, a: ConfirmedSpace) -> Result<Value> {
    require_yes(a.yes, "archiving the space")?;
    let data = admin::space_archive(c, &a.id_or_key)?;
    out(
        c,
        "space.archive",
        "space",
        json!({"idOrKey": a.id_or_key}),
        data,
    )
}

pub(super) fn space_restore(c: &Client, a: ConfirmedSpace) -> Result<Value> {
    require_yes(a.yes, "restoring the space")?;
    let data = admin::space_restore(c, &a.id_or_key)?;
    out(
        c,
        "space.restore",
        "space",
        json!({"idOrKey": a.id_or_key}),
        data,
    )
}

pub(super) fn issue_types(c: &Client, a: IssueTypesArgs) -> Result<Value> {
    let (space, types) = admin::space_issue_types(c, &space_ref(&a.id_or_key), a.first)?;
    let key = space["key"].clone();
    let data = json!({"space": space, "issueTypes": types});
    out(
        c,
        "space.issue-types",
        "issue-type",
        json!({"idOrKey": key, "first": a.first}),
        data,
    )
}

pub(super) fn status(c: &Client, cmd: StatusCmd) -> Result<Value> {
    let StatusCmd::Query(t) = cmd;
    let data = admin::space_statuses(c, &t.id_or_key)?;
    out(
        c,
        "space.statuses",
        "status",
        json!({"idOrKey": t.id_or_key}),
        data,
    )
}

pub(super) fn types(c: &Client, cmd: SpaceTypesCmd) -> Result<Value> {
    let SpaceTypesCmd::Query { id } = cmd;
    let data = admin::space_types(c, &id)?;
    out(
        c,
        "space.types.query",
        "space-type",
        json!({"ids": id}),
        data,
    )
}

pub(super) fn notification_scheme(c: &Client, cmd: NotificationSchemeCmd) -> Result<Value> {
    let NotificationSchemeCmd::Get { key, expand } = cmd;
    let data = admin::notification_scheme(c, &key, expand.as_deref())?;
    out(
        c,
        "space.notification-scheme.get",
        "space-notification-scheme",
        json!({"key": key, "projectKeyOrId": key}),
        data,
    )
}

pub(super) fn versions(c: &Client, cmd: VersionsCmd) -> Result<Value> {
    let VersionsCmd::Query {
        key,
        search_string,
        filter,
        sort_by,
        after,
        limit,
    } = cmd;
    let mut page = admin::versions(
        c,
        &key,
        &admin::VersionQuery {
            search: search_string.as_deref(),
            status: filter.as_deref(),
            order_by: sort_by.as_deref(),
            start: jira::offset_cursor(after.as_deref())?,
            limit,
        },
    )?;
    let request = json!({
        "site": jira::site(c),
        "projectIdOrKey": key,
        "key": key,
        "maxResults": limit,
        "limit": limit,
    });
    page.data = page.named("versions");
    Ok(offset_envelope(
        "jira.space.versions.query",
        "jira.space-version",
        request,
        page,
    ))
}

fn version_body(name: Option<String>, f: VersionFields) -> Map<String, Value> {
    body([
        ("name", name.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("startDate", f.start_date.map(Value::from)),
        ("releaseDate", f.release_date.map(Value::from)),
        ("released", f.released.map(Value::from)),
        ("archived", f.archived.map(Value::from)),
    ])
}

pub(super) fn version(c: &Client, cmd: VersionCmd) -> Result<Value> {
    let (verb, request, data) = match cmd {
        VersionCmd::Create { key, name, fields } => {
            let data = admin::version_create(c, &key, version_body(Some(name), fields))?;
            ("create", json!({"key": key}), data)
        }
        VersionCmd::Get { id } => {
            let data = admin::version_get(c, &id)?;
            ("get", json!({"id": id}), data)
        }
        VersionCmd::Update { id, name, fields } => {
            let data = admin::version_update(c, &id, version_body(name, fields))?;
            ("update", json!({"id": id}), data)
        }
        VersionCmd::Delete {
            id,
            move_fix_issues_to: fix,
            move_affected_issues_to: affected,
            yes: _,
        } => {
            let data = admin::version_delete(c, &id, fix.as_deref(), affected.as_deref())?;
            ("delete", json!({"id": id}), data)
        }
        VersionCmd::Merge {
            id,
            move_issues_to,
            yes: _,
        } => {
            let data = admin::version_merge(c, &id, &move_issues_to)?;
            (
                "merge",
                json!({"id": id, "moveIssuesTo": move_issues_to}),
                data,
            )
        }
        VersionCmd::Move {
            id,
            after,
            position,
        } => {
            let data = admin::version_move(c, &id, after.as_deref(), position.as_deref())?;
            ("move", json!({"id": id}), data)
        }
        VersionCmd::Counts { id } => {
            let data = admin::version_counts(c, &id)?;
            ("counts", json!({"id": id}), data)
        }
    };
    out(
        c,
        &format!("space.version.{verb}"),
        "version",
        request,
        data,
    )
}

fn component_body(name: Option<String>, f: ComponentFields) -> Map<String, Value> {
    body([
        ("name", name.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("leadAccountId", f.lead_account_id.map(Value::from)),
        ("assigneeType", f.assignee_type.map(Value::from)),
    ])
}

pub(super) fn component(c: &Client, cmd: ComponentCmd) -> Result<Value> {
    let (verb, request, data) = match cmd {
        ComponentCmd::Query { key } => {
            let data = admin::components(c, &key)?;
            ("query", json!({"key": key, "projectIdOrKey": key}), data)
        }
        ComponentCmd::Get { id } => {
            let data = admin::component_get(c, &id)?;
            ("get", json!({"id": id}), data)
        }
        ComponentCmd::Create { key, name, fields } => {
            let data = admin::component_create(c, &key, component_body(Some(name), fields))?;
            ("create", json!({"key": key}), data)
        }
        ComponentCmd::Update { id, name, fields } => {
            let data = admin::component_update(c, &id, component_body(name, fields))?;
            ("update", json!({"id": id}), data)
        }
        ComponentCmd::Delete {
            id,
            move_issues_to,
            yes: _,
        } => {
            let data = admin::component_delete(c, &id, move_issues_to.as_deref())?;
            ("delete", json!({"id": id}), data)
        }
        ComponentCmd::Counts { id } => {
            let data = admin::component_counts(c, &id)?;
            ("counts", json!({"id": id}), data)
        }
    };
    out(
        c,
        &format!("space.component.{verb}"),
        if verb == "query" {
            "space-component"
        } else {
            "component"
        },
        request,
        data,
    )
}

pub(super) fn filter(c: &Client, cmd: FilterCmd) -> Result<Value> {
    let resolve = |r: &str| admin::resolve_filter(c, r);
    let (verb, id, data) = match cmd {
        FilterCmd::Query {
            filter_name,
            owner,
            owner_account_id,
            group_name,
            group_id,
            project_id,
            filter_id,
            order_by,
            expand,
            override_share_permissions,
            favourite,
            first,
            start_at,
        } => {
            if favourite {
                let data = admin::favourite_filters(c)?;
                return out(
                    c,
                    "filter.query",
                    "filter",
                    json!({"favourite": true}),
                    data,
                );
            }
            let page = admin::filters(
                c,
                &admin::FilterQuery {
                    name: filter_name.as_deref(),
                    owner: owner.as_deref(),
                    owner_account: owner_account_id.as_deref(),
                    group: group_name.as_deref(),
                    group_id: group_id.as_deref(),
                    project_id: project_id.as_deref(),
                    ids: &filter_id,
                    order_by: order_by.as_deref(),
                    expand: expand.as_deref(),
                    override_shares: override_share_permissions,
                    start: start_at.unwrap_or(0),
                    limit: first,
                },
            )?;
            let request = json!({"site": jira::site(c), "filterName": filter_name, "first": first});
            let mut page = page;
            page.data = page.named("values");
            return Ok(offset_envelope(
                "jira.filter.query",
                "jira.filter-search-result",
                request,
                page,
            ));
        }
        FilterCmd::Create {
            name,
            jql,
            description,
            favourite,
            share_permissions_json,
            edit_permissions_json,
            expand,
            override_share_permissions,
        } => {
            let fields = body([
                ("name", Some(name.as_str().into())),
                ("jql", jql.map(Value::from)),
                ("description", description.map(Value::from)),
                ("favourite", favourite.map(Value::from)),
                (
                    "sharePermissions",
                    json_arg(
                        "--share-permissions-json",
                        share_permissions_json.as_deref(),
                    )?,
                ),
                (
                    "editPermissions",
                    json_arg("--edit-permissions-json", edit_permissions_json.as_deref())?,
                ),
            ]);
            let data =
                admin::filter_create(c, fields, expand.as_deref(), override_share_permissions)?;
            return out(c, "filter.create", "filter", json!({"name": name}), data);
        }
        FilterCmd::Get { id, expand } => {
            let id = resolve(&id)?;
            let data = admin::filter_get(c, &id, expand.as_deref())?;
            ("get", id, data)
        }
        FilterCmd::Update {
            id,
            name,
            jql,
            description,
            favourite,
            share_permissions_json,
            edit_permissions_json,
        } => {
            let fields = body([
                ("name", name.map(Value::from)),
                ("jql", jql.map(Value::from)),
                ("description", description.map(Value::from)),
                (
                    "sharePermissions",
                    json_arg(
                        "--share-permissions-json",
                        share_permissions_json.as_deref(),
                    )?,
                ),
                (
                    "editPermissions",
                    json_arg("--edit-permissions-json", edit_permissions_json.as_deref())?,
                ),
            ]);
            let id = resolve(&id)?;
            let data = admin::filter_update(c, &id, fields, favourite)?;
            ("update", id, data)
        }
        FilterCmd::Delete {
            id,
            override_share_permissions,
            yes,
        } => {
            require_yes(yes, "deleting the filter")?;
            let id = resolve(&id)?;
            let data = admin::filter_delete(c, &id, override_share_permissions)?;
            ("delete", id, data)
        }
        FilterCmd::AddFavourite { id } => {
            let id = resolve(&id)?;
            let data = admin::filter_favourite(c, &id, true, None)?;
            ("add-favourite", id, data)
        }
        FilterCmd::RemoveFavourite { id, expand } => {
            let id = resolve(&id)?;
            let data = admin::filter_favourite(c, &id, false, expand.as_deref())?;
            ("remove-favourite", id, data)
        }
        FilterCmd::ChangeOwner { id, account_id } => {
            let id = resolve(&id)?;
            let data = admin::filter_owner(c, &id, &account_id)?;
            ("change-owner", id, data)
        }
        FilterCmd::Columns { cmd } => match cmd {
            ColumnsCmd::Get { id } => {
                let id = resolve(&id)?;
                let data = admin::filter_columns(c, &id)?;
                ("columns.get", id, data)
            }
            ColumnsCmd::Set { id, column } => {
                let id = resolve(&id)?;
                let data = admin::filter_columns_set(c, &id, &column)?;
                ("columns.set", id, data)
            }
        },
        FilterCmd::ResetColumns { id, yes } => {
            require_yes(yes, "resetting the columns")?;
            let id = resolve(&id)?;
            let data = admin::filter_columns_reset(c, &id)?;
            ("reset-columns", id, data)
        }
        FilterCmd::Share { cmd } => match cmd {
            ShareCmd::Query { id } => {
                let id = resolve(&id)?;
                let data = admin::filter_shares(c, &id)?;
                ("share.query", id, data)
            }
            ShareCmd::Add {
                id,
                permission_json,
            } => {
                let permission: Value = serde_json::from_str(&permission_json)
                    .map_err(|e| usage(format!("--permission-json is not valid JSON: {e}")))?;
                let id = resolve(&id)?;
                let data = admin::filter_share_add(c, &id, &permission)?;
                ("share.add", id, data)
            }
            ShareCmd::Remove {
                id,
                permission_id,
                yes,
            } => {
                require_yes(yes, "removing the share")?;
                let id = resolve(&id)?;
                let data = admin::filter_share_remove(c, &id, &permission_id)?;
                ("share.remove", id, data)
            }
        },
        FilterCmd::Subscription {
            cmd:
                SubscriptionCmd::Query {
                    id,
                    first,
                    start_at,
                },
        } => {
            let id = resolve(&id)?;
            let data = admin::filter_subscriptions(c, &id, start_at, first)?;
            ("subscription.query", id, data)
        }
    };
    let kind = match verb {
        "share.query" => "filter-share-permission",
        "subscription.query" => "filter-subscription",
        _ => "filter",
    };
    out(c, &format!("filter.{verb}"), kind, json!({"id": id}), data)
}

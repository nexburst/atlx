//! `jira project` and its versions and components.

use super::{Confirm, Many, Paging, listed};
use crate::client::{ApiError, Client};
use crate::jira::{self, admin, agile};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ProjectCmd {
    /// Projects, by text, type, category or status; or the projects of a
    /// board
    List {
        /// Key or name contains
        #[arg(short, long)]
        query: Option<String>,
        /// software, business or service_desk
        #[arg(long = "type")]
        project_type: Option<String>,
        /// Category ID
        #[arg(long)]
        category: Option<String>,
        /// live, archived or deleted
        #[arg(long)]
        status: Option<String>,
        /// Sort, e.g. key, name, -lastIssueUpdatedDate
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
        /// The projects on this board instead (ID, URL or exact name)
        #[arg(long, conflicts_with_all = ["query", "project_type", "category", "status", "order_by"])]
        board: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Projects by key or ID
    Get {
        /// Project keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a project
    Create {
        /// Key of the new project
        #[arg(long)]
        key: String,
        /// Name
        #[arg(long)]
        name: String,
        /// business, service_desk or software
        #[arg(long = "type")]
        project_type: Option<String>,
        /// Project template key
        #[arg(long)]
        template: Option<String>,
        #[command(flatten)]
        fields: ProjectFields,
    },
    /// Change a project
    Update {
        /// Project key or ID
        key: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New key
        #[arg(long)]
        new_key: Option<String>,
        #[command(flatten)]
        fields: ProjectFields,
    },
    /// Move a project to the trash (restorable for 60 days)
    Delete {
        /// Project key or ID
        key: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Archive a project
    Archive {
        /// Project key or ID
        key: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Bring an archived or trashed project back
    Restore {
        /// Project key or ID
        key: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// A project's versions (releases)
    Version {
        #[command(subcommand)]
        cmd: VersionCmd,
    },
    /// A project's components
    Component {
        #[command(subcommand)]
        cmd: ComponentCmd,
    },
}

#[derive(Args)]
pub(crate) struct ProjectFields {
    /// Project lead's account ID
    #[arg(long)]
    lead: Option<String>,
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// A link shown with the project
    #[arg(long)]
    url: Option<String>,
    /// PROJECT_LEAD or UNASSIGNED
    #[arg(long)]
    assignee_type: Option<String>,
    /// Category ID; -1 removes it
    #[arg(long, allow_hyphen_values = true)]
    category: Option<String>,
}

impl ProjectFields {
    fn body(self, mut m: Map<String, Value>) -> Result<Map<String, Value>> {
        let category =
            self.category
                .map(|v| {
                    v.trim().parse::<i64>().map(Value::from).map_err(|_| {
                        anyhow::Error::from(ApiError::usage("--category takes a number"))
                    })
                })
                .transpose()?;
        for (k, v) in [
            ("leadAccountId", self.lead.map(Value::from)),
            ("description", self.description.map(Value::from)),
            ("url", self.url.map(Value::from)),
            ("assigneeType", self.assignee_type.map(Value::from)),
            ("categoryId", category),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v);
            }
        }
        Ok(m)
    }
}

/// The project a version or component list belongs to.
#[derive(Args)]
pub(crate) struct ProjectArg {
    /// Project key or ID
    #[arg(long)]
    project: String,
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
    /// A project's versions
    List {
        #[command(flatten)]
        project: ProjectArg,
        /// Name or description contains
        #[arg(short, long)]
        query: Option<String>,
        /// Comma-separated: released, unreleased, archived
        #[arg(long)]
        status: Option<String>,
        /// Sort, e.g. name or -releaseDate
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Versions by ID, with issue counts
    Get {
        /// Version IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a version
    Create {
        #[command(flatten)]
        project: ProjectArg,
        /// Name
        #[arg(long)]
        name: String,
        #[command(flatten)]
        fields: VersionFields,
    },
    /// Change a version
    Update {
        /// Version ID
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
        id: String,
        /// Version that takes its fix-version issues
        #[arg(long)]
        move_fix_issues_to: Option<String>,
        /// Version that takes its affected-version issues
        #[arg(long)]
        move_affected_issues_to: Option<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Delete a version, moving its issues to another
    Merge {
        /// Version to delete
        id: String,
        /// Version that receives its issues
        #[arg(long)]
        into: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Reorder a version
    Move {
        /// Version ID
        id: String,
        /// Put it after this version (ID or URL)
        #[arg(
            long,
            conflicts_with = "position",
            required_unless_present = "position"
        )]
        after_version: Option<String>,
        /// First, Last, Earlier or Later
        #[arg(long, value_parser = ["First", "Last", "Earlier", "Later"])]
        position: Option<String>,
    },
}

#[derive(Args)]
pub(crate) struct ComponentFields {
    /// Description
    #[arg(long)]
    description: Option<String>,
    /// Lead's account ID
    #[arg(long)]
    lead: Option<String>,
    /// PROJECT_DEFAULT, COMPONENT_LEAD, PROJECT_LEAD or UNASSIGNED
    #[arg(long)]
    assignee_type: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum ComponentCmd {
    /// A project's components
    List {
        #[command(flatten)]
        project: ProjectArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Components by ID
    Get {
        /// Component IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a component
    Create {
        #[command(flatten)]
        project: ProjectArg,
        /// Name
        #[arg(long)]
        name: String,
        #[command(flatten)]
        fields: ComponentFields,
    },
    /// Change a component
    Update {
        /// Component ID
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
        id: String,
        /// Component that takes its issues
        #[arg(long)]
        move_issues_to: Option<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
}

fn body<const N: usize>(fields: [(&str, Option<Value>); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
        .collect()
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

fn component_body(name: Option<String>, f: ComponentFields) -> Map<String, Value> {
    body([
        ("name", name.map(Value::from)),
        ("description", f.description.map(Value::from)),
        ("leadAccountId", f.lead.map(Value::from)),
        ("assigneeType", f.assignee_type.map(Value::from)),
    ])
}

pub(super) fn run(c: &Client, cmd: ProjectCmd) -> Result<Value> {
    match cmd {
        ProjectCmd::List {
            query,
            project_type,
            category,
            status,
            order_by,
            board,
            paging,
        } => {
            if let Some(b) = board {
                let id = jira::resolve_board(c, &b)?;
                return Ok(listed(agile::board_projects(
                    c,
                    id,
                    paging.limit(),
                    paging.cursor(),
                )?));
            }
            let q = jira::ProjectQuery {
                query: query.as_deref(),
                type_key: project_type.as_deref(),
                category_id: category.as_deref(),
                status: status.as_deref(),
                order_by: order_by.as_deref(),
            };
            Ok(listed(jira::projects(
                c,
                &q,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        ProjectCmd::Get { keys, many } => many.run(keys, |k| jira::project(c, k)),
        ProjectCmd::Create {
            key,
            name,
            project_type,
            template,
            fields,
        } => {
            let m = body([
                ("key", Some(key.into())),
                ("name", Some(name.into())),
                ("projectTypeKey", project_type.map(Value::from)),
                ("projectTemplateKey", template.map(Value::from)),
            ]);
            admin::project_create(c, fields.body(m)?)
        }
        ProjectCmd::Update {
            key,
            name,
            new_key,
            fields,
        } => {
            let m = body([
                ("key", new_key.map(Value::from)),
                ("name", name.map(Value::from)),
            ]);
            admin::project_update(c, &key, fields.body(m)?, Some("description,lead"))
        }
        ProjectCmd::Delete { key, confirm: _ } => admin::project_delete(c, &key),
        ProjectCmd::Archive { key, confirm: _ } => admin::project_archive(c, &key),
        ProjectCmd::Restore { key, confirm: _ } => admin::project_restore(c, &key),
        ProjectCmd::Version { cmd } => version(c, cmd),
        ProjectCmd::Component { cmd } => component(c, cmd),
    }
}

fn version(c: &Client, cmd: VersionCmd) -> Result<Value> {
    match cmd {
        VersionCmd::List {
            project,
            query,
            status,
            order_by,
            paging,
        } => Ok(listed(admin::versions(
            c,
            &jira::project_ref(&project.project),
            &admin::VersionQuery {
                search: query.as_deref(),
                status: status.as_deref(),
                order_by: order_by.as_deref(),
            },
            paging.limit(),
            paging.cursor(),
        )?)),
        VersionCmd::Get { ids, many } => many.run(ids, |id| admin::version_get(c, id)),
        VersionCmd::Create {
            project,
            name,
            fields,
        } => admin::version_create(
            c,
            &jira::project_ref(&project.project),
            version_body(Some(name), fields),
        ),
        VersionCmd::Update { id, name, fields } => {
            admin::version_update(c, &id, version_body(name, fields))
        }
        VersionCmd::Delete {
            id,
            move_fix_issues_to,
            move_affected_issues_to,
            confirm: _,
        } => admin::version_delete(
            c,
            &id,
            move_fix_issues_to.as_deref(),
            move_affected_issues_to.as_deref(),
        ),
        VersionCmd::Merge {
            id,
            into,
            confirm: _,
        } => admin::version_merge(c, &id, &into),
        VersionCmd::Move {
            id,
            after_version,
            position,
        } => admin::version_move(c, &id, after_version.as_deref(), position.as_deref()),
    }
}

fn component(c: &Client, cmd: ComponentCmd) -> Result<Value> {
    match cmd {
        ComponentCmd::List { project, paging } => {
            paging.local(admin::components(c, &jira::project_ref(&project.project))?)
        }
        ComponentCmd::Get { ids, many } => many.run(ids, |id| admin::component_get(c, id)),
        ComponentCmd::Create {
            project,
            name,
            fields,
        } => admin::component_create(
            c,
            &jira::project_ref(&project.project),
            component_body(Some(name), fields),
        ),
        ComponentCmd::Update { id, name, fields } => {
            admin::component_update(c, &id, component_body(name, fields))
        }
        ComponentCmd::Delete {
            id,
            move_issues_to,
            confirm: _,
        } => admin::component_delete(c, &id, move_issues_to.as_deref()),
    }
}

//! `confluence content tasks|status|watch`, `space watch|permission` and
//! `confluence templates`.

use crate::cli::util::require_yes;
use crate::client::{ApiError, Client};
use crate::confluence::{self, extras, lifecycle};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Subcommand)]
pub(crate) enum TasksCmd {
    /// Tasks on a page or blog post, or everywhere you can see
    List(TaskListArgs),
    /// One task
    Get(TaskRef),
    /// Tick a task off
    Complete(TaskRef),
    /// Untick a task
    Reopen(TaskRef),
}

#[derive(Args)]
pub(crate) struct TaskListArgs {
    /// Page or blog post ID, ARI or URL, or a title with --reference-space
    #[arg(long)]
    id: Option<String>,
    /// open, complete or all
    #[arg(long, value_enum, default_value = "all")]
    status: extras::TaskFilter,
    /// Tasks to return
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// page or blogpost (found out if left out)
    #[arg(long, value_enum, requires = "id")]
    content_type: Option<confluence::ContentType>,
    /// Space key or ID for looking up an exact title
    #[arg(long, requires = "id")]
    reference_space: Option<String>,
}

#[derive(Args)]
pub(crate) struct TaskRef {
    /// Task ID
    #[arg(long)]
    task_id: String,
}

/// Content states work by ID alone, so the type only needs to be valid.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum StatusType {
    Page,
    Blogpost,
    #[value(name = "live_doc")]
    LiveDoc,
    Whiteboard,
    Database,
    Folder,
}

#[derive(Subcommand)]
pub(crate) enum StatusCmd {
    /// The status shown on a page
    Get(StatusArgs),
    /// Set the status, which publishes a new version
    Set(StatusSetArgs),
}

#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Content ID, ARI or URL, or an exact title with --reference-space
    id: String,
    /// Accepted for twg; the ID is enough
    #[arg(long, value_enum)]
    content_type: Option<StatusType>,
    /// Use the draft
    #[arg(long)]
    draft: bool,
    /// Space key or ID for looking up an exact title
    #[arg(long)]
    reference_space: Option<String>,
}

#[derive(Args)]
pub(crate) struct StatusSetArgs {
    #[command(flatten)]
    at: StatusArgs,
    /// An existing status's ID
    #[arg(long, conflicts_with_all = ["name", "color"])]
    state_id: Option<u64>,
    /// Name of a status (20 characters at most)
    #[arg(long, requires = "color")]
    name: Option<String>,
    /// Six-digit hex color, e.g. #2684ff
    #[arg(long, requires = "name")]
    color: Option<String>,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct AvailableArgs {
    /// Content ID, ARI or URL, or an exact title with --reference-space
    id: String,
    /// Space key or ID for looking up an exact title
    #[arg(long)]
    reference_space: Option<String>,
}

#[derive(Args)]
pub(crate) struct WatchArgs {
    /// Content ID, ARI or URL, or an exact title with --reference-space
    #[arg(long)]
    id: String,
    /// Space key or ID for looking up an exact title
    #[arg(long)]
    reference_space: Option<String>,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct SpaceWatchArgs {
    /// Space key
    #[arg(
        long,
        conflicts_with = "space_id",
        required_unless_present = "space_id"
    )]
    key: Option<String>,
    /// Space ID, key, ARI or URL
    #[arg(long)]
    space_id: Option<String>,
    /// Confirm (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Subcommand)]
pub(crate) enum PermissionCmd {
    /// Who may do what in a space
    List {
        /// Space key, ID, ARI or URL
        #[arg(long)]
        space: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Every space permission there is
    Available(PageArgs),
}

#[derive(Args)]
pub(crate) struct PageArgs {
    /// Results per page (1-250)
    #[arg(long, default_value_t = 25)]
    limit: usize,
    #[arg(long)]
    cursor: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum TemplatesCmd {
    /// Page and blueprint templates
    List(TemplateListArgs),
    /// One template, with its body
    Get {
        /// Template ID
        template_id: String,
    },
}

#[derive(Args)]
pub(crate) struct TemplateListArgs {
    /// Space key, ID, ARI or URL
    #[arg(long)]
    space_id: Option<String>,
    /// Defaults to space with --space-id, else global
    #[arg(long, value_enum)]
    scope: Option<extras::Scope>,
    /// all, page or blueprint
    #[arg(long, value_enum, default_value = "all")]
    template_type: extras::TemplateType,
    /// Text in the ID, name or description
    #[arg(long)]
    query: Option<String>,
    /// `nextCursor` from the page before
    #[arg(long)]
    cursor: Option<String>,
    /// Templates per page
    #[arg(long, default_value_t = 25)]
    limit: usize,
}

pub(super) fn tasks(c: &Client, cmd: TasksCmd) -> Result<Value> {
    let (name, request, data) = match cmd {
        TasksCmd::List(a) => {
            let id =
                a.id.as_deref()
                    .map(|r| lifecycle::exact_content(c, r, a.reference_space.as_deref()))
                    .transpose()?;
            let data = extras::tasks(c, id.as_deref(), a.content_type, a.status, a.limit)?;
            return Ok(envelope("confluence.content.tasks.list", "confluence.task")
                .request(json!({"id": id, "status": a.status.name(), "limit": a.limit}))
                .next_cursor(data["nextCursor"].as_str())
                .data(data.clone()));
        }
        TasksCmd::Get(a) => {
            let data = extras::task(c, &a.task_id)?;
            ("confluence.content.tasks.get", a.task_id, data)
        }
        TasksCmd::Complete(a) => {
            let data = extras::task_set(c, &a.task_id, true)?;
            ("confluence.content.tasks.complete", a.task_id, data)
        }
        TasksCmd::Reopen(a) => {
            let data = extras::task_set(c, &a.task_id, false)?;
            ("confluence.content.tasks.reopen", a.task_id, data)
        }
    };
    Ok(envelope(name, "confluence.task")
        .request(json!({"taskId": request}))
        .data(data))
}

pub(super) fn status(c: &Client, cmd: StatusCmd) -> Result<Value> {
    let (name, at, set) = match cmd {
        StatusCmd::Get(at) => ("confluence.content.status.get", at, None),
        StatusCmd::Set(a) => {
            require_yes(a.yes, "setting the status (it publishes a new version)")?;
            let state = extras::state_input(a.state_id, a.name.as_deref(), a.color.as_deref())?;
            ("confluence.content.status.set", a.at, Some(state))
        }
    };
    let id = lifecycle::exact_content(c, &at.id, at.reference_space.as_deref())?;
    let data = match &set {
        None => extras::state(c, &id, at.draft)?,
        Some(state) => extras::state_set(c, &id, at.draft, state)?,
    };
    Ok(envelope(name, "confluence.content-state")
        .request(json!({"id": id, "draft": at.draft, "state": set}))
        .data(data))
}

pub(super) fn available(c: &Client, a: AvailableArgs) -> Result<Value> {
    let id = lifecycle::exact_content(c, &a.id, a.reference_space.as_deref())?;
    Ok(envelope(
        "confluence.content.list-available-statuses",
        "confluence.content-state",
    )
    .request(json!({"id": id}))
    .data(extras::states_available(c, &id)?))
}

pub(super) fn watch(c: &Client, a: WatchArgs, on: bool) -> Result<Value> {
    let name = if on {
        "confluence.content.watch"
    } else {
        "confluence.content.unwatch"
    };
    require_yes(a.yes, "changing the watch")?;
    let id = lifecycle::exact_content(c, &a.id, a.reference_space.as_deref())?;
    Ok(envelope(name, "confluence.content")
        .request(json!({"id": id}))
        .data(extras::watch_content(c, &id, on)?))
}

pub(super) fn space_watch(c: &Client, a: SpaceWatchArgs, on: bool) -> Result<Value> {
    let name = if on {
        "confluence.space.watch"
    } else {
        "confluence.space.unwatch"
    };
    require_yes(a.yes, "changing the watch")?;
    let Some(space) = a.key.or(a.space_id) else {
        return Err(ApiError::usage("pass --key or --space-id").into());
    };
    Ok(envelope(name, "confluence.space")
        .request(json!({"space": space}))
        .data(extras::watch_space(c, &space, on)?))
}

pub(super) fn permission(c: &Client, cmd: PermissionCmd) -> Result<Value> {
    let (name, space, page) = match cmd {
        PermissionCmd::List { space, page } => {
            ("confluence.space.permission.list", Some(space), page)
        }
        PermissionCmd::Available(page) => ("confluence.space.permission.available", None, page),
    };
    let data = extras::permissions(c, space.as_deref(), page.limit, page.cursor.as_deref())?;
    Ok(envelope(name, "confluence.space-permission")
        .request(json!({"space": space, "limit": page.limit, "cursor": page.cursor}))
        .next_cursor(data["nextCursor"].as_str())
        .data(data.clone()))
}

pub(super) fn templates(c: &Client, cmd: TemplatesCmd) -> Result<Value> {
    match cmd {
        TemplatesCmd::Get { template_id } => {
            Ok(envelope("confluence.templates.get", "confluence.template")
                .request(json!({"templateId": template_id}))
                .data(extras::template(c, &template_id)?))
        }
        TemplatesCmd::List(a) => {
            let start = match &a.cursor {
                None => 0,
                Some(cur) => cur.parse().map_err(|_| {
                    ApiError::usage(format!("'{cur}' is not a cursor from this list"))
                })?,
            };
            let data = extras::templates(
                c,
                &extras::TemplateQuery {
                    space: a.space_id.as_deref(),
                    scope: a.scope,
                    kind: a.template_type,
                    text: a.query.as_deref(),
                    start,
                    limit: a.limit,
                },
            )?;
            Ok(envelope("confluence.templates.list", "confluence.template")
                .request(json!({"spaceId": a.space_id, "query": a.query, "limit": a.limit, "cursor": a.cursor}))
                .next_cursor(data["nextCursor"].as_str())
                .data(data.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    fn parses(args: &str) -> bool {
        Cli::try_parse_from(["atlx"].into_iter().chain(args.split_whitespace())).is_ok()
    }

    #[test]
    fn parses_tasks_statuses_watches_templates_and_permissions() {
        for args in [
            "confluence content tasks list",
            "confluence content tasks list --id 12345 --status open --content-type page --limit 5",
            "confluence content tasks get --task-id 1",
            "confluence content tasks complete --task-id 1",
            "confluence content tasks reopen --task-id 1",
            "confluence content status get 12345 --draft --content-type live_doc",
            "confluence content status set 12345 --state-id 1 --yes",
            "confluence content status set 12345 --name Review --color #ffc400 --yes",
            "confluence content list-available-statuses 12345",
            "confluence content watch --id 12345 --yes",
            "confluence content unwatch --id 12345 --yes",
            "confluence space watch --key ENG --yes",
            "confluence space unwatch --space-id 55 --yes",
            "confluence space permission list --space ENG --limit 5",
            "confluence space permission available",
            "confluence templates list --space-id ENG --scope available --template-type page --query retro",
            "confluence templates get 12345",
        ] {
            assert!(parses(args), "{args}");
        }
        for args in [
            "confluence content status set 12345 --name Review",
            "confluence content status set 12345 --state-id 1 --name Review --color #ffc400",
            "confluence space watch --yes",
            "confluence space watch --key A --space-id 1 --yes",
            "confluence content tasks get",
        ] {
            assert!(!parses(args), "{args}");
        }
    }
}

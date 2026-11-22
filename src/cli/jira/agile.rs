//! `jira sprint` lifecycle, `jira board` details, and the Agile flags of
//! `workitem create|update`.

use crate::cli::util::require_yes;
use crate::client::{ApiError, Client};
use crate::jira::{self, agile};
use crate::output::envelope::envelope;
use crate::time::epoch_millis;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

/// Agile flags on `workitem create` and `workitem update`.
#[derive(Args)]
pub(crate) struct PlanningArgs {
    /// Sprint ID, ARI, URL or name (active and future sprints), or "none"
    /// for the backlog
    #[arg(long)]
    sprint: Option<String>,
    /// Story points, or "none"; the field comes from the space's boards
    #[arg(long)]
    story_points: Option<String>,
    /// Original estimate: a duration like "1d 4h", or seconds
    #[arg(long)]
    original_estimate: Option<String>,
    /// Remaining estimate: a duration like 2h, or seconds
    #[arg(long)]
    remaining_estimate: Option<String>,
    /// Team ID or ARI, or "none"
    #[arg(long)]
    team: Option<String>,
}

impl PlanningArgs {
    pub(super) fn planning(&self) -> agile::Planning<'_> {
        agile::Planning {
            sprint: self.sprint.as_deref(),
            story_points: self.story_points.as_deref(),
            original_estimate: self.original_estimate.as_deref(),
            remaining_estimate: self.remaining_estimate.as_deref(),
            team: self.team.as_deref(),
        }
    }
}

#[derive(Args)]
pub(crate) struct SprintFields {
    /// Sprint name
    #[arg(long)]
    name: Option<String>,
    /// Sprint goal
    #[arg(long)]
    goal: Option<String>,
    /// Start: date (UTC midnight), timestamp or epoch ms
    #[arg(long)]
    start_date: Option<String>,
    /// End: date (UTC midnight), timestamp or epoch ms
    #[arg(long)]
    end_date: Option<String>,
}

#[derive(Args)]
pub(crate) struct SprintTarget {
    /// Sprint ID, ARI or URL, or its exact name with --board-id
    #[arg(long)]
    id: String,
    /// The board to look a sprint name up on
    #[arg(long)]
    board_id: Option<String>,
}

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Sprint ID, ARI or URL
    #[arg(value_name = "id", conflicts_with = "id")]
    id_arg: Option<String>,
    /// Same as the positional argument
    #[arg(long)]
    id: Option<String>,
}

#[derive(Args)]
pub(crate) struct CreateArgs {
    /// Board ID, ARI, URL or exact name
    #[arg(long)]
    board_id: String,
    #[command(flatten)]
    fields: SprintFields,
}

#[derive(Args)]
pub(crate) struct UpdateSprintArgs {
    #[command(flatten)]
    target: SprintTarget,
    #[command(flatten)]
    fields: SprintFields,
    /// future, active or closed
    #[arg(long, value_parser = ["future", "active", "closed"])]
    state: Option<String>,
    /// When closing: a sprint ID, ARI or URL, or "backlog", for work that
    /// isn't done
    #[arg(long)]
    incomplete_issues_destination_id: Option<String>,
}

#[derive(Args)]
pub(crate) struct StartArgs {
    #[command(flatten)]
    target: SprintTarget,
    #[command(flatten)]
    fields: SprintFields,
    /// Length in days or weeks, e.g. 10d or 2w (atlx extra)
    #[arg(long, conflicts_with = "end_date")]
    duration: Option<String>,
}

#[derive(Args)]
pub(crate) struct CompleteArgs {
    #[command(flatten)]
    target: SprintTarget,
    #[command(flatten)]
    fields: SprintFields,
    /// A sprint ID, ARI or URL, or "backlog", for work that isn't done
    /// [default: Jira decides]
    #[arg(long, alias = "move-open-to")]
    incomplete_issues_destination_id: Option<String>,
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Sprint ID, ARI or URL, or its exact name with --board-id
    #[arg(long, conflicts_with = "sprint_id")]
    id: Option<String>,
    /// Same as --id
    #[arg(long)]
    sprint_id: Option<String>,
    /// The board to look a sprint name up on
    #[arg(long)]
    board_id: Option<String>,
    /// Accepted for compatibility; nothing asks for confirmation
    #[arg(long, short, hide = true)]
    yes: bool,
}

#[derive(Args)]
pub(crate) struct BoardGetArgs {
    /// Board ID, ARI, URL or exact name
    #[arg(long)]
    id: String,
}

#[derive(Args)]
pub(crate) struct BoardCreateArgs {
    /// Board name
    #[arg(long)]
    name: String,
    /// The filter the board shows (ID or URL)
    #[arg(long)]
    filter_id: String,
    /// Board type
    #[arg(long = "type", value_enum)]
    board_type: jira::BoardType,
}

#[derive(Args)]
pub(crate) struct BoardDeleteArgs {
    /// Board ID, ARI, URL or exact name
    #[arg(
        long,
        conflicts_with = "board_id",
        required_unless_present = "board_id"
    )]
    id: Option<String>,
    /// Same as --id
    #[arg(long)]
    board_id: Option<String>,
    /// Confirm the delete (required)
    #[arg(long, short)]
    yes: bool,
}

#[derive(Subcommand)]
pub(crate) enum BacklogCmd {
    /// The work items in a board's backlog
    Query {
        /// Board ID, ARI, URL or exact name
        #[arg(
            long,
            conflicts_with = "board_id",
            required_unless_present = "board_id"
        )]
        id: Option<String>,
        /// Same as --id
        #[arg(long)]
        board_id: Option<String>,
        /// Comma-separated IDs of quick filters to apply
        #[arg(long)]
        active_quick_filters: Option<String>,
        /// Extra JQL (atlx extra)
        #[arg(long)]
        jql: Option<String>,
        /// Most work items to return (atlx extra)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum BoardProjectsCmd {
    /// The spaces (projects) on a board
    Query {
        /// Board ID, ARI, URL or exact name
        #[arg(
            long,
            conflicts_with = "board_id",
            required_unless_present = "board_id"
        )]
        id: Option<String>,
        /// Same as --id
        #[arg(long)]
        board_id: Option<String>,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        /// Zero-based offset of the first space
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum QuickFilterCmd {
    /// A board's quick filters
    Query {
        /// Board ID, ARI, URL or exact name
        #[arg(long)]
        board_id: String,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        /// Zero-based offset of the first filter
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
    /// One quick filter
    Get {
        /// Board ID, ARI, URL or exact name
        #[arg(long)]
        board_id: String,
        /// Quick filter ID
        #[arg(long)]
        quick_filter_id: String,
    },
}

fn usage(message: &str) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// A sprint date. Ages like `2w` are refused: they would mean the past.
fn date(flag: &str, value: Option<&str>) -> Result<Option<i64>> {
    let Some(v) = value.map(str::trim) else {
        return Ok(None);
    };
    if v.ends_with(['h', 'd', 'w', 'm', 'y']) {
        return Err(usage(&format!(
            "{flag} takes a date or timestamp, not '{v}'; use `sprint start --duration` for a length"
        )));
    }
    epoch_millis(v).map(Some)
}

fn duration(value: &str) -> Result<i64> {
    let v = value.trim();
    let days = if let Some(n) = v.strip_suffix('w') {
        n.parse::<i64>().ok().and_then(|n| n.checked_mul(7))
    } else {
        v.strip_suffix('d').and_then(|n| n.parse::<i64>().ok())
    };
    days.filter(|d| (1..=3650).contains(d))
        .map(|d| d * 86_400_000)
        .ok_or_else(|| usage("--duration takes days or weeks, like 10d or 2w"))
}

impl SprintFields {
    fn sprint(&self) -> Result<agile::Sprint<'_>> {
        Ok(agile::Sprint {
            name: self.name.as_deref(),
            goal: self.goal.as_deref(),
            start: date("--start-date", self.start_date.as_deref())?,
            end: date("--end-date", self.end_date.as_deref())?,
            state: None,
        })
    }
}

fn destination(value: Option<&str>) -> Result<Option<agile::Destination>> {
    value
        .map(|v| {
            if v.trim().eq_ignore_ascii_case("backlog") {
                Ok(agile::Destination::Backlog)
            } else {
                agile::sprint_id(v)
                    .map(agile::Destination::Sprint)
                    .ok_or_else(|| {
                        usage(&format!(
                            "destination '{v}' is not a sprint ID, ARI, URL or backlog"
                        ))
                    })
            }
        })
        .transpose()
}

fn sprint_out(c: &Client, verb: &str, request: Value, data: Value) -> Result<Value> {
    let mut request = request;
    request["site"] = jira::site(c).into();
    Ok(envelope(&format!("jira.sprint.{verb}"), "jira.sprint")
        .request(request)
        .data(data))
}

pub(super) fn sprint_get(c: &Client, a: GetArgs) -> Result<Value> {
    let id = a
        .id_arg
        .or(a.id)
        .ok_or_else(|| usage("pass the sprint as an argument or with --id"))?;
    let id = agile::resolve_sprint(c, &id, None)?;
    sprint_out(c, "get", json!({"id": id}), agile::sprint_get(c, id)?)
}

pub(super) fn sprint_create(c: &Client, a: CreateArgs) -> Result<Value> {
    let sprint = a.fields.sprint()?;
    if sprint.name.is_none() {
        return Err(usage("pass --name"));
    }
    let board = jira::resolve_board(c, &a.board_id)?;
    let data = agile::sprint_create(c, board, &sprint)?;
    sprint_out(c, "create", json!({"boardId": board}), data)
}

pub(super) fn sprint_update(c: &Client, a: UpdateSprintArgs) -> Result<Value> {
    let sprint = agile::Sprint {
        state: a.state.as_deref(),
        ..a.fields.sprint()?
    };
    let to = destination(a.incomplete_issues_destination_id.as_deref())?;
    if to.is_some() && sprint.state != Some("closed") {
        return Err(usage(
            "--incomplete-issues-destination-id needs --state closed",
        ));
    }
    let id = agile::resolve_sprint(c, &a.target.id, a.target.board_id.as_deref())?;
    let data = match to {
        Some(_) => agile::sprint_complete(c, id, &sprint, to)?,
        None => agile::sprint_update(c, id, &sprint)?,
    };
    sprint_out(c, "update", json!({"id": id}), data)
}

pub(super) fn sprint_start(c: &Client, a: StartArgs) -> Result<Value> {
    let sprint = a.fields.sprint()?;
    let length = a.duration.as_deref().map(duration).transpose()?;
    let id = agile::resolve_sprint(c, &a.target.id, a.target.board_id.as_deref())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    let data = agile::sprint_start(c, id, &sprint, length, now)?;
    sprint_out(c, "start", json!({"id": id}), data)
}

pub(super) fn sprint_complete(c: &Client, a: CompleteArgs) -> Result<Value> {
    let sprint = a.fields.sprint()?;
    let to = destination(a.incomplete_issues_destination_id.as_deref())?;
    let id = agile::resolve_sprint(c, &a.target.id, a.target.board_id.as_deref())?;
    let data = agile::sprint_complete(c, id, &sprint, to)?;
    sprint_out(c, "complete", json!({"id": id}), data)
}

pub(super) fn sprint_delete(c: &Client, a: DeleteArgs) -> Result<Value> {
    let id =
        a.id.or(a.sprint_id)
            .ok_or_else(|| usage("pass --id or --sprint-id"))?;
    let id = agile::resolve_sprint(c, &id, a.board_id.as_deref())?;
    sprint_out(c, "delete", json!({"id": id}), agile::sprint_delete(c, id)?)
}

fn board_out(c: &Client, verb: &str, kind: &str, request: Value, data: Value) -> Result<Value> {
    let mut request = request;
    request["site"] = jira::site(c).into();
    Ok(envelope(&format!("jira.board.{verb}"), kind)
        .request(request)
        .data(data))
}

/// `--id` or `--board-id`.
fn board(c: &Client, id: Option<String>, board_id: Option<String>) -> Result<u64> {
    let r = id
        .or(board_id)
        .ok_or_else(|| usage("pass --id or --board-id"))?;
    jira::resolve_board(c, &r)
}

pub(super) fn board_get(c: &Client, a: BoardGetArgs) -> Result<Value> {
    let id = jira::resolve_board(c, &a.id)?;
    let data = agile::board_get(c, id)?;
    board_out(c, "get", "jira.board", json!({"id": id}), data)
}

pub(super) fn board_create(c: &Client, a: BoardCreateArgs) -> Result<Value> {
    let filter: u64 = jira::filter_ref(&a.filter_id)
        .parse()
        .map_err(|_| usage("--filter-id must be a filter ID or URL"))?;
    let data = agile::board_create(c, &a.name, a.board_type, filter)?;
    let request = json!({"name": a.name, "filterId": filter});
    board_out(c, "create", "jira.board", request, data)
}

pub(super) fn board_delete(c: &Client, a: BoardDeleteArgs) -> Result<Value> {
    require_yes(a.yes, "deleting the board")?;
    let id = board(c, a.id, a.board_id)?;
    let data = agile::board_delete(c, id)?;
    board_out(c, "delete", "jira.board", json!({"boardId": id}), data)
}

pub(super) fn backlog(c: &Client, cmd: BacklogCmd) -> Result<Value> {
    let BacklogCmd::Query {
        id,
        board_id,
        active_quick_filters,
        jql,
        limit,
    } = cmd;
    let id = board(c, id, board_id)?;
    let filters = active_quick_filters
        .as_deref()
        .map(jira::csv)
        .unwrap_or_default();
    let data = agile::backlog(c, id, &filters, jql.as_deref(), 0, limit)?;
    let request = json!({"boardId": id, "jql": jql});
    board_out(c, "backlog.query", "jira.workitem", request, data)
}

pub(super) fn projects(c: &Client, cmd: BoardProjectsCmd) -> Result<Value> {
    let BoardProjectsCmd::Query {
        id,
        board_id,
        max_results,
        start_at,
    } = cmd;
    let id = board(c, id, board_id)?;
    let data = agile::board_projects(c, id, start_at, max_results)?.to_value("projects");
    board_out(
        c,
        "projects.query",
        "jira.space",
        json!({"boardId": id}),
        data,
    )
}

pub(super) fn quick_filter(c: &Client, cmd: QuickFilterCmd) -> Result<Value> {
    const KIND: &str = "jira.quick-filter";
    match cmd {
        QuickFilterCmd::Query {
            board_id,
            max_results,
            start_at,
        } => {
            let id = jira::resolve_board(c, &board_id)?;
            let data = agile::quick_filters(c, id, start_at, max_results)?.to_value("quickFilters");
            board_out(c, "quick-filter.query", KIND, json!({"boardId": id}), data)
        }
        QuickFilterCmd::Get {
            board_id,
            quick_filter_id,
        } => {
            let id = jira::resolve_board(c, &board_id)?;
            let filter = jira::entity_id(&quick_filter_id);
            let data = agile::quick_filter(c, id, &filter)?;
            let request = json!({"boardId": id, "quickFilterId": filter});
            board_out(c, "quick-filter.get", KIND, request, data)
        }
    }
}

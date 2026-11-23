//! `jira board`, `jira board quick-filter` and `jira sprint`.

use super::{Confirm, Many, Paging, listed};
use crate::client::{ApiError, Client};
use crate::jira::{self, agile};
use crate::time::epoch_millis;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Subcommand)]
pub(crate) enum BoardCmd {
    /// Agile boards, by project, type, name or filter
    List {
        /// Project key or ID
        #[arg(long)]
        project: Option<String>,
        /// Board type
        #[arg(long = "type", value_enum)]
        board_type: Option<jira::BoardType>,
        /// Name contains
        #[arg(short, long)]
        query: Option<String>,
        /// Boards showing this filter (ID or URL)
        #[arg(long)]
        filter: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Boards by ID, URL or exact name, with columns and estimation
    Get {
        /// Board IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a board on a filter
    Create {
        /// Board name
        #[arg(long)]
        name: String,
        /// Board type
        #[arg(long = "type", value_enum)]
        board_type: jira::BoardType,
        /// The filter it shows (ID or URL)
        #[arg(long)]
        filter: String,
    },
    /// Delete a board
    Delete {
        /// Board ID, URL or exact name
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// A board's quick filters
    QuickFilter {
        #[command(subcommand)]
        cmd: QuickFilterCmd,
    },
}

#[derive(Args)]
pub(crate) struct BoardArg {
    /// Board ID, URL or exact name
    #[arg(long)]
    board: String,
}

#[derive(Subcommand)]
pub(crate) enum QuickFilterCmd {
    /// A board's quick filters
    List {
        #[command(flatten)]
        board: BoardArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Quick filters by ID
    Get {
        /// Quick filter IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        board: BoardArg,
        #[command(flatten)]
        many: Many,
    },
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

/// A sprint given by ID, ARI or URL, or by exact name with --board.
#[derive(Args)]
pub(crate) struct SprintTarget {
    /// Sprint ID, ARI or URL, or its exact name with --board
    id: String,
    /// The board to look a sprint name up on
    #[arg(long)]
    board: Option<String>,
}

impl SprintTarget {
    fn resolve(&self, c: &Client) -> Result<u64> {
        agile::resolve_sprint(c, &self.id, self.board.as_deref())
    }
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum SprintCmd {
    /// A board's sprints
    List {
        /// Board ID, URL or exact name
        #[arg(long, conflicts_with_all = ["project", "issue"])]
        board: Option<String>,
        /// Use the only board of this project
        #[arg(long, conflicts_with = "issue")]
        project: Option<String>,
        /// Use the only board of this issue's project
        #[arg(long)]
        issue: Option<String>,
        /// Comma-separated states: active, future, closed
        #[arg(long, conflicts_with = "current")]
        state: Option<String>,
        /// Only the active sprint
        #[arg(long)]
        current: bool,
        #[command(flatten)]
        paging: Paging,
    },
    /// Sprints by ID, ARI or URL
    Get {
        /// Sprint IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Create a future sprint on a board
    Create {
        /// Board ID, URL or exact name
        #[arg(long)]
        board: String,
        #[command(flatten)]
        fields: SprintFields,
    },
    /// Change some of a sprint's fields
    Update {
        #[command(flatten)]
        target: SprintTarget,
        #[command(flatten)]
        fields: SprintFields,
    },
    /// Start a sprint; it needs --end-date or --duration unless it has an end
    Start {
        #[command(flatten)]
        target: SprintTarget,
        #[command(flatten)]
        fields: SprintFields,
        /// Length in days or weeks, e.g. 10d or 2w
        #[arg(long, conflicts_with = "end_date")]
        duration: Option<String>,
    },
    /// Complete an active sprint
    Complete {
        #[command(flatten)]
        target: SprintTarget,
        #[command(flatten)]
        fields: SprintFields,
        /// Where unfinished issues go: a sprint ID, ARI or URL, or backlog
        /// [default: Jira decides]
        #[arg(long)]
        move_open_to: Option<String>,
    },
    /// Delete a sprint
    Delete {
        #[command(flatten)]
        target: SprintTarget,
        #[command(flatten)]
        confirm: Confirm,
    },
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// A sprint date. Ages like `2w` are refused: they would mean the past.
fn date(flag: &str, value: Option<&str>) -> Result<Option<i64>> {
    let Some(v) = value.map(str::trim) else {
        return Ok(None);
    };
    if v.ends_with(['h', 'd', 'w', 'm', 'y']) {
        return Err(usage(format!(
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
                        usage(format!(
                            "destination '{v}' is not a sprint ID, ARI, URL or backlog"
                        ))
                    })
            }
        })
        .transpose()
}

pub(super) fn board(c: &Client, cmd: BoardCmd) -> Result<Value> {
    match cmd {
        BoardCmd::List {
            project,
            board_type,
            query,
            filter,
            paging,
        } => {
            let project = project.map(|p| jira::project_ref(&p));
            let filter = filter.as_deref().map(jira::filter_ref);
            let q = jira::BoardQuery {
                project: project.as_deref(),
                board_type,
                name: query.as_deref(),
                filter_id: filter.as_deref(),
            };
            Ok(listed(jira::boards(
                c,
                &q,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        BoardCmd::Get { ids, many } => {
            many.run(ids, |id| agile::board_get(c, jira::resolve_board(c, id)?))
        }
        BoardCmd::Create {
            name,
            board_type,
            filter,
        } => {
            let filter: u64 = jira::filter_ref(&filter)
                .parse()
                .map_err(|_| usage("--filter must be a filter ID or URL"))?;
            agile::board_create(c, &name, board_type, filter)
        }
        BoardCmd::Delete { id, confirm: _ } => agile::board_delete(c, jira::resolve_board(c, &id)?),
        BoardCmd::QuickFilter { cmd } => match cmd {
            QuickFilterCmd::List { board, paging } => {
                let id = jira::resolve_board(c, &board.board)?;
                Ok(listed(agile::quick_filters(
                    c,
                    id,
                    paging.limit(),
                    paging.cursor(),
                )?))
            }
            QuickFilterCmd::Get { ids, board, many } => {
                let id = jira::resolve_board(c, &board.board)?;
                many.run(ids, |q| agile::quick_filter(c, id, &jira::entity_id(q)))
            }
        },
    }
}

pub(super) fn sprint(c: &Client, cmd: SprintCmd) -> Result<Value> {
    match cmd {
        SprintCmd::List {
            board,
            project,
            issue,
            state,
            current,
            paging,
        } => {
            let board = match (board, project, issue) {
                (Some(b), _, _) => jira::resolve_board(c, &b)?,
                (None, Some(p), _) => jira::project_board(c, &jira::project_ref(&p))?,
                (None, None, Some(i)) => {
                    jira::project_board(c, &jira::issue_project(c, &jira::issue_ref(&i))?)?
                }
                (None, None, None) => {
                    return Err(ApiError::usage("pass --board, --project or --issue").into());
                }
            };
            let state = if current {
                Some("active".into())
            } else {
                state
            };
            Ok(listed(jira::sprints(
                c,
                board,
                state.as_deref(),
                paging.limit(),
                paging.cursor(),
            )?))
        }
        SprintCmd::Get { ids, many } => {
            many.run(ids, |id| agile::sprint_get(c, jira::sprint_ref(id)?))
        }
        SprintCmd::Create { board, fields } => {
            let sprint = fields.sprint()?;
            if sprint.name.is_none() {
                return Err(usage("pass --name"));
            }
            let board = jira::resolve_board(c, &board)?;
            agile::sprint_create(c, board, &sprint)
        }
        SprintCmd::Update { target, fields } => {
            let sprint = fields.sprint()?;
            agile::sprint_update(c, target.resolve(c)?, &sprint)
        }
        SprintCmd::Start {
            target,
            fields,
            duration: length,
        } => {
            let sprint = fields.sprint()?;
            let length = length.as_deref().map(duration).transpose()?;
            let id = target.resolve(c)?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
            agile::sprint_start(c, id, &sprint, length, now)
        }
        SprintCmd::Complete {
            target,
            fields,
            move_open_to,
        } => {
            let sprint = fields.sprint()?;
            let to = destination(move_open_to.as_deref())?;
            agile::sprint_complete(c, target.resolve(c)?, &sprint, to)
        }
        SprintCmd::Delete { target, confirm: _ } => agile::sprint_delete(c, target.resolve(c)?),
    }
}

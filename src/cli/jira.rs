//! `atlx jira`: clap definitions and dispatch.

use super::util::{DEFAULT_CONCURRENCY, batch, concurrency_arg, deprecated, text_arg};
use crate::client::{self, ApiError};
use crate::jira;
use crate::markup::TextFormat;
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde_json::Value;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum JiraCmd {
    /// Jira work items (issues)
    Workitem {
        #[command(subcommand)]
        cmd: WorkitemCmd,
    },
    /// Jira fields
    Field {
        #[command(subcommand)]
        cmd: FieldCmd,
    },
    /// Jira spaces (projects)
    Space {
        #[command(subcommand)]
        cmd: SpaceCmd,
    },
    /// Old name of `jira space`
    #[command(hide = true)]
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Jira Agile boards and their sprints
    Board {
        #[command(subcommand)]
        cmd: BoardCmd,
    },
    /// Jira Agile sprints
    Sprint {
        #[command(subcommand)]
        cmd: SprintCmd,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
pub(crate) enum SpaceCmd {
    /// Find spaces by key, ID, name or type
    #[command(visible_alias = "list")]
    Query(SpaceQueryArgs),
    /// One space by key or ID
    Get {
        /// Space key or ID
        #[arg(value_name = "id-or-key")]
        identifier: String,
    },
}

#[derive(Args)]
pub(crate) struct SpaceQueryArgs {
    /// Text matched against space keys and names
    #[arg(long)]
    query: Option<String>,
    /// Space key; repeat for several
    #[arg(long)]
    key: Vec<String>,
    /// Space ID; repeat for several
    #[arg(long)]
    id: Vec<String>,
    /// Space type, e.g. software, business, service_desk
    #[arg(long)]
    type_key: Option<String>,
    #[arg(long)]
    category_id: Option<String>,
    /// Permission filter: view, browse or edit
    #[arg(long)]
    action: Option<String>,
    /// Comma-separated Jira expansions
    #[arg(long)]
    expand: Option<String>,
    /// live, archived or deleted
    #[arg(long)]
    status: Option<String>,
    /// Space property to include; repeatable
    #[arg(long)]
    property: Vec<String>,
    #[arg(long)]
    property_query: Option<String>,
    /// Sort order, e.g. key, name, -lastIssueUpdatedDate
    #[arg(long)]
    sort_by: Option<String>,
    /// Zero-based offset of the first space
    #[arg(long, conflicts_with = "after")]
    start_at: Option<usize>,
    /// Offset to resume from (same as --start-at)
    #[arg(long)]
    after: Option<String>,
    /// Maximum number of spaces [default: 25]
    #[arg(long, conflicts_with = "first")]
    limit: Option<usize>,
    /// Same as --limit
    #[arg(long)]
    first: Option<usize>,
}

#[derive(Subcommand)]
pub(crate) enum FieldCmd {
    /// List Jira fields (including custom fields)
    List {
        /// Case-insensitive substring filter on field name or ID
        #[arg(long)]
        query: Option<String>,
        /// Only include custom fields
        #[arg(long)]
        custom_only: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProjectCmd {
    /// Old name of `jira space query`
    #[command(hide = true)]
    List {
        /// Filter by project key or name
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Old name of `jira space get`
    #[command(hide = true)]
    Get {
        /// Project key or ID
        identifier: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum BoardCmd {
    /// Find Agile boards
    Query {
        /// Space (project) key or ID
        #[arg(long, conflicts_with = "project_ari")]
        project: Option<String>,
        /// Same as --project; also takes a project ARI
        #[arg(long)]
        project_ari: Option<String>,
        /// Use the space of this work item
        #[arg(long, conflicts_with_all = ["project", "project_ari"])]
        issue: Option<String>,
        /// Filter by board type
        #[arg(long = "type", value_enum)]
        board_type: Option<jira::BoardType>,
        /// Board name substring
        #[arg(long)]
        name: Option<String>,
        /// Boards built on this filter (ID or filter URL)
        #[arg(long)]
        filter_id: Option<String>,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        /// Zero-based offset of the first board
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
    /// Sprints on a board
    Sprints {
        #[command(subcommand)]
        cmd: BoardSprintsCmd,
    },
    /// Old name of `jira board query`
    #[command(hide = true)]
    List {
        /// Filter by project key or ID
        #[arg(long)]
        project: Option<String>,
        /// Filter by board type
        #[arg(long = "type", value_enum)]
        board_type: Option<jira::BoardType>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum BoardSprintsCmd {
    /// List the sprints of a board
    Query {
        /// Board ID, ARI, board URL or exact board name
        #[arg(long, conflicts_with = "board_id")]
        id: Option<String>,
        /// Same as --id
        #[arg(long)]
        board_id: Option<String>,
        /// Use the only board of this space (project)
        #[arg(long, conflicts_with = "project_ari")]
        project: Option<String>,
        /// Same as --project; also takes a project ARI
        #[arg(long)]
        project_ari: Option<String>,
        /// Use the only board of this work item's space
        #[arg(long)]
        issue: Option<String>,
        /// Comma-separated states: active, future, closed
        #[arg(long, conflicts_with = "current")]
        state: Option<String>,
        /// Only the active sprint (same as --state active)
        #[arg(long)]
        current: bool,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        /// Zero-based offset of the first sprint
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum SprintCmd {
    /// Work items in a sprint
    Workitems {
        #[command(subcommand)]
        cmd: SprintWorkitemsCmd,
    },
    /// Old name of `jira board sprints query`
    #[command(hide = true)]
    List {
        /// Board ID
        #[arg(long)]
        board: u64,
        /// Comma-separated states: active, future, closed
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Old name of `jira sprint workitems query`
    #[command(hide = true)]
    Issues {
        /// Sprint ID
        #[arg(long)]
        sprint: u64,
        /// Additional JQL filter
        #[arg(long)]
        jql: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum SprintWorkitemsCmd {
    /// List the work items in a sprint
    Query {
        /// Sprint ID, ARI or sprint URL
        #[arg(long, conflicts_with = "sprint_id")]
        id: Option<String>,
        /// Same as --id
        #[arg(long)]
        sprint_id: Option<String>,
        /// Extra JQL filter
        #[arg(long)]
        jql: Option<String>,
        /// Comma-separated extra fields to include
        #[arg(long)]
        fields: Option<String>,
        /// Comma-separated Jira expansions
        #[arg(long)]
        expand: Option<String>,
        #[arg(long, default_value_t = 50)]
        max_results: usize,
        /// Zero-based offset of the first work item
        #[arg(long, default_value_t = 0)]
        start_at: usize,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum WorkitemCmd {
    /// Get one or more work items by key, ID, ARI or URL
    Get {
        ids: Vec<String>,
        /// Issue key, ID, or ARI; repeat to batch
        #[arg(long = "id")]
        id_flags: Vec<String>,
        #[arg(long)]
        issue_id: Option<String>,
        #[arg(long)]
        key: Option<String>,
        /// Comma-separated Jira REST fields to include
        #[arg(long)]
        fields: Option<String>,
        /// Jira REST field or customfield_* ID to include; repeatable
        #[arg(long)]
        field: Vec<String>,
        /// Fetch complete comments
        #[arg(long)]
        comments: bool,
        /// Fetch remote links
        #[arg(long)]
        remote_links: bool,
        /// All fields, comments and remote links
        #[arg(long, conflicts_with_all = ["comments", "remote_links"])]
        full: bool,
        /// Fail the batch if any input fails
        #[arg(long)]
        strict: bool,
        /// How many work items to fetch at once (1 to 20)
        #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY, value_parser = concurrency_arg)]
        batch_concurrency: usize,
    },
    /// Find work items by fuzzy text (text ~ "…" ORDER BY updated DESC)
    Search {
        #[arg(required = true)]
        text: Vec<String>,
        #[arg(long)]
        limit: Option<usize>,
        /// Page size; alias for --limit
        #[arg(long)]
        first: Option<usize>,
        /// Pagination cursor (next_cursor from a previous response)
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        fields: Option<String>,
    },
    /// Run a JQL query
    Query {
        jql_arg: Option<String>,
        #[arg(long)]
        jql: Option<String>,
        #[arg(long, short = 'n')]
        limit: Option<usize>,
        #[arg(long)]
        first: Option<usize>,
        #[arg(long)]
        after: Option<String>,
        /// Backward paging; Jira REST search only pages forward, so this
        /// returns a usage error
        #[arg(long)]
        last: Option<usize>,
        /// Backward paging cursor; see --last
        #[arg(long)]
        before: Option<String>,
        #[arg(long)]
        fields: Option<String>,
    },
    /// Create a work item
    Create {
        /// Jira space (project) key or ID
        #[arg(long)]
        space: String,
        /// Issue type name or ID
        #[arg(long = "type", default_value = "Task")]
        issue_type: String,
        #[arg(long)]
        summary: String,
        /// `-` reads stdin
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        description_format: TextFormat,
        /// Account ID, "me", or a name/email
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        reporter: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        /// Comma-separated
        #[arg(long)]
        labels: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        /// KEY=VALUE (value parsed as JSON when valid); repeatable
        #[arg(long)]
        field: Vec<String>,
        #[arg(long)]
        fields_json: Option<String>,
        /// Accepted for twg compatibility
        #[arg(long, short)]
        yes: bool,
    },
    /// Update fields, optionally transition and comment
    Update(UpdateArgs),
    /// Comments on work items
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },
    /// Transition a work item; omit --transition-id to list transitions
    Transition {
        #[arg(long)]
        id: String,
        /// Transition ID, transition name, or target status name
        #[arg(long)]
        transition_id: Option<String>,
        #[arg(long)]
        field: Vec<String>,
        #[arg(long)]
        fields_json: Option<String>,
        #[arg(long)]
        transition_comment: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        transition_comment_format: TextFormat,
    },
    /// Available transitions
    Transitions {
        #[command(subcommand)]
        cmd: TransitionsCmd,
    },
    /// Field metadata for work items
    #[command(visible_alias = "custom-fields")]
    Field {
        #[command(subcommand)]
        cmd: WorkitemFieldCmd,
    },
    /// Old name of `jira workitem field create-metadata`
    #[command(hide = true)]
    CreateMeta {
        /// Jira space (project) key or ID
        #[arg(long)]
        space: String,
        /// Issue type name or ID
        #[arg(long = "type")]
        issue_type: Option<String>,
    },
    /// Issue links and remote web links
    Link {
        #[command(subcommand)]
        cmd: LinkCmd,
    },
    /// Issue link types
    #[command(name = "link-types")]
    LinkTypes {
        #[command(subcommand)]
        cmd: LinkTypesCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkitemFieldCmd {
    /// Issue types of a space, or the fields needed to create one type
    CreateMetadata {
        /// Jira space (project) key or ID
        #[arg(long)]
        space: String,
        /// Issue type name or ID; omit to list the space's issue types
        #[arg(long = "type")]
        issue_type: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum LinkTypesCmd {
    /// List the issue link types on the site
    Query,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    summary: Option<String>,
    /// `-` reads stdin
    #[arg(long)]
    description: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    description_format: TextFormat,
    /// "me", "none", an account ID, or a name/email
    #[arg(long)]
    assignee: Option<String>,
    #[arg(long = "type")]
    issue_type: Option<String>,
    #[arg(long)]
    priority: Option<String>,
    /// Comma-separated; replaces existing
    #[arg(long, conflicts_with_all = ["add_labels", "remove_labels"])]
    labels: Option<String>,
    #[arg(long)]
    add_labels: Option<String>,
    #[arg(long)]
    remove_labels: Option<String>,
    /// Parent key, or "none"
    #[arg(long)]
    parent: Option<String>,
    /// YYYY-MM-DD, or "none"
    #[arg(long)]
    due_date: Option<String>,
    #[arg(long, conflicts_with_all = ["add_fix_versions", "remove_fix_versions"])]
    fix_versions: Option<String>,
    #[arg(long)]
    add_fix_versions: Option<String>,
    #[arg(long)]
    remove_fix_versions: Option<String>,
    #[arg(long, conflicts_with_all = ["add_components", "remove_components"])]
    components: Option<String>,
    #[arg(long)]
    add_components: Option<String>,
    #[arg(long)]
    remove_components: Option<String>,
    /// Resolution name or ID, or "none"
    #[arg(long)]
    resolution: Option<String>,
    #[arg(long)]
    field: Vec<String>,
    #[arg(long)]
    fields_json: Option<String>,
    /// Transition by target status or transition name
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    transition_comment: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    transition_comment_format: TextFormat,
    /// Add a comment after the update
    #[arg(long)]
    comment: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    comment_format: TextFormat,
}

#[derive(Subcommand)]
pub(crate) enum CommentCmd {
    /// Add a comment
    Create {
        #[arg(long)]
        issue_id: String,
        /// `-` reads stdin
        #[arg(long)]
        body: String,
        #[arg(long, value_enum, default_value_t)]
        body_format: TextFormat,
        /// JSM internal comment
        #[arg(long)]
        internal: bool,
        #[arg(long, conflicts_with = "visibility_group")]
        visibility_role: Option<String>,
        #[arg(long)]
        visibility_group: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TransitionsCmd {
    /// List transitions and the fields they require
    Query {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum LinkCmd {
    /// Link a work item to one or more others
    ///
    /// The source reads with the link type's outward wording: `--id PROJ-1
    /// --target-id PROJ-2 --link-type-id Blocks` means PROJ-1 blocks PROJ-2.
    Workitem {
        /// Source work item key, ID, ARI or URL
        #[arg(long)]
        id: String,
        /// Target work item key, ID, ARI or URL; repeat for several
        #[arg(long, required = true)]
        target_id: Vec<String>,
        /// Link type ID or name (see `jira workitem link-types query`)
        #[arg(long)]
        link_type_id: String,
    },
    /// Add a remote web link to a work item
    Weblink {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        id: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        title: String,
        /// Stable ID; a second link with the same ID updates the first
        #[arg(long)]
        global_id: Option<String>,
        /// Relationship text, e.g. "mentioned in"
        #[arg(long)]
        relationship: Option<String>,
        #[arg(long)]
        application_name: Option<String>,
        #[arg(long)]
        application_type: Option<String>,
        /// Short description shown with the link
        #[arg(long)]
        summary: Option<String>,
        /// 16x16 icon URL
        #[arg(long)]
        icon_url: Option<String>,
        /// Show the linked item as resolved
        #[arg(long)]
        resolved: bool,
    },
    /// Old name of `jira workitem link-types query`
    #[command(hide = true)]
    Types,
    /// Old name of `jira workitem link workitem`
    #[command(hide = true)]
    Create {
        /// Inward issue key, ID, ARI, or URL
        #[arg(long)]
        inward: String,
        /// Outward issue key, ID, ARI, or URL
        #[arg(long)]
        outward: String,
        /// Link type name or ID
        #[arg(long = "type")]
        link_type: String,
        /// Optional comment (`-` reads stdin)
        #[arg(long)]
        comment: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        comment_format: TextFormat,
    },
    /// Old name of `jira workitem link weblink`
    #[command(hide = true)]
    Remote {
        /// Issue key, ID, ARI, or URL
        #[arg(long)]
        id: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        relationship: Option<String>,
    },
}

/// Issue key from a key, numeric ID, ARI, or `/browse/KEY` URL.
fn issue_ref(s: &str) -> String {
    let s = s.trim();
    if let Some(i) = s.find("/browse/") {
        return s[i + 8..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string();
    }
    if s.starts_with("ari:") {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    s.to_string()
}

fn fields_arg(fields: Option<&str>, field: &[String]) -> Vec<String> {
    let mut all = fields.map(jira::csv).unwrap_or_default();
    all.extend(field.iter().cloned());
    all
}

fn list_edit(
    set: Option<String>,
    add: Option<String>,
    remove: Option<String>,
) -> Option<jira::ListEdit> {
    if set.is_none() && add.is_none() && remove.is_none() {
        return None;
    }
    Some(jira::ListEdit {
        set: set.as_deref().map(jira::csv),
        add: add.as_deref().map(jira::csv).unwrap_or_default(),
        remove: remove.as_deref().map(jira::csv).unwrap_or_default(),
    })
}

pub(crate) fn jira_cmd(c: &client::Client, cmd: JiraCmd) -> Result<Value> {
    match cmd {
        JiraCmd::Workitem { cmd } => workitem(c, cmd),
        JiraCmd::Field {
            cmd: FieldCmd::List { query, custom_only },
        } => jira::fields(c, query.as_deref(), custom_only),
        JiraCmd::Space {
            cmd: SpaceCmd::Query(a),
        } => space_query(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Get { identifier },
        } => jira::project(c, &identifier),
        JiraCmd::Project {
            cmd: ProjectCmd::List { query, limit },
        } => {
            deprecated("jira project list", "jira space query");
            jira::space_query(
                c,
                &jira::SpaceQuery {
                    query: query.as_deref(),
                    limit,
                    ..Default::default()
                },
            )
        }
        JiraCmd::Project {
            cmd: ProjectCmd::Get { identifier },
        } => {
            deprecated("jira project get", "jira space get");
            jira::project(c, &identifier)
        }
        JiraCmd::Board {
            cmd:
                BoardCmd::Query {
                    project,
                    project_ari,
                    issue,
                    board_type,
                    name,
                    filter_id,
                    max_results,
                    start_at,
                },
        } => {
            let project = match issue {
                Some(i) => Some(jira::issue_project(c, &issue_ref(&i))?),
                None => project.or(project_ari).map(|p| project_ref(&p)),
            };
            let filter_id = filter_id.as_deref().map(jira::filter_ref);
            jira::board_query(
                c,
                &jira::BoardQuery {
                    project: project.as_deref(),
                    board_type,
                    name: name.as_deref(),
                    filter_id: filter_id.as_deref(),
                    start: start_at,
                    limit: max_results,
                },
            )
        }
        JiraCmd::Board {
            cmd:
                BoardCmd::Sprints {
                    cmd:
                        BoardSprintsCmd::Query {
                            id,
                            board_id,
                            project,
                            project_ari,
                            issue,
                            state,
                            current,
                            max_results,
                            start_at,
                        },
                },
        } => {
            let board = match (id.or(board_id), project.or(project_ari), issue) {
                (Some(b), _, _) => jira::resolve_board(c, &b)?,
                (None, Some(p), _) => jira::project_board(c, &project_ref(&p))?,
                (None, None, Some(i)) => {
                    jira::project_board(c, &jira::issue_project(c, &issue_ref(&i))?)?
                }
                (None, None, None) => {
                    return Err(ApiError::usage("pass --id, --project or --issue").into());
                }
            };
            let state = if current {
                Some("active".into())
            } else {
                state
            };
            jira::sprints(c, board, state.as_deref(), start_at, max_results)
        }
        JiraCmd::Board {
            cmd:
                BoardCmd::List {
                    project,
                    board_type,
                    limit,
                },
        } => {
            deprecated("jira board list", "jira board query");
            jira::board_query(
                c,
                &jira::BoardQuery {
                    project: project.as_deref(),
                    board_type,
                    limit,
                    ..Default::default()
                },
            )
        }
        JiraCmd::Sprint {
            cmd:
                SprintCmd::Workitems {
                    cmd:
                        SprintWorkitemsCmd::Query {
                            id,
                            sprint_id,
                            jql,
                            fields,
                            expand,
                            max_results,
                            start_at,
                        },
                },
        } => {
            let Some(sprint) = id.or(sprint_id) else {
                return Err(ApiError::usage("pass --id or --sprint-id").into());
            };
            let fields = fields_arg(fields.as_deref(), &[]);
            jira::sprint_issues(
                c,
                jira::sprint_ref(&sprint)?,
                &jira::SprintIssues {
                    jql: jql.as_deref(),
                    fields: &fields,
                    expand: expand.as_deref(),
                    start: start_at,
                    limit: max_results,
                },
            )
        }
        JiraCmd::Sprint {
            cmd:
                SprintCmd::List {
                    board,
                    state,
                    limit,
                },
        } => {
            deprecated("jira sprint list", "jira board sprints query");
            jira::sprints(c, board, state.as_deref(), 0, limit)
        }
        JiraCmd::Sprint {
            cmd: SprintCmd::Issues { sprint, jql, limit },
        } => {
            deprecated("jira sprint issues", "jira sprint workitems query");
            jira::sprint_issues(
                c,
                sprint,
                &jira::SprintIssues {
                    jql: jql.as_deref(),
                    limit,
                    ..Default::default()
                },
            )
        }
    }
}

/// A project key or ID from a key, ID or project ARI.
fn project_ref(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("ari:") {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    s.to_string()
}

fn space_query(c: &client::Client, a: SpaceQueryArgs) -> Result<Value> {
    let start = match (a.start_at, a.after.as_deref()) {
        (Some(n), _) => n,
        (None, Some(after)) => after.trim().parse().map_err(|_| {
            ApiError::usage(format!(
                "--after '{after}' is not an offset; Jira REST pages spaces by number"
            ))
        })?,
        (None, None) => 0,
    };
    jira::space_query(
        c,
        &jira::SpaceQuery {
            query: a.query.as_deref(),
            keys: &a.key,
            ids: &a.id,
            type_key: a.type_key.as_deref(),
            category_id: a.category_id.as_deref(),
            action: a.action.as_deref(),
            expand: a.expand.as_deref(),
            status: a.status.as_deref(),
            properties: &a.property,
            property_query: a.property_query.as_deref(),
            order_by: a.sort_by.as_deref(),
            start,
            limit: a.limit.or(a.first).unwrap_or(25),
        },
    )
}

fn workitem(c: &client::Client, cmd: WorkitemCmd) -> Result<Value> {
    match cmd {
        WorkitemCmd::Get {
            ids,
            id_flags,
            issue_id,
            key,
            fields,
            field,
            comments,
            remote_links,
            full,
            strict,
            batch_concurrency,
        } => {
            let inputs: Vec<String> = ids
                .into_iter()
                .chain(id_flags)
                .chain(issue_id)
                .chain(key)
                .map(|s| issue_ref(&s))
                .collect();
            let opts = jira::GetOpts {
                fields: fields_arg(fields.as_deref(), &field),
                comments,
                remote_links,
                full,
            };
            batch(inputs, strict, batch_concurrency, |k| {
                jira::get(c, k, &opts)
            })
        }
        WorkitemCmd::Search {
            text,
            limit,
            first,
            after,
            fields,
        } => {
            let fields = fields_arg(fields.as_deref(), &[]);
            jira::search(
                c,
                &text.join(" "),
                limit.or(first).unwrap_or(25),
                &fields,
                after.as_deref(),
            )
        }
        WorkitemCmd::Query {
            jql_arg,
            jql,
            limit,
            first,
            after,
            last,
            before,
            fields,
        } => {
            if last.is_some() || before.is_some() {
                return Err(ApiError::usage(
                    "--last and --before page backward, which Jira REST search can't do; \
                     page forward with --limit and --after",
                )
                .into());
            }
            let Some(jql) = jql.or(jql_arg) else {
                bail!("pass --jql or a JQL argument")
            };
            let fields = fields_arg(fields.as_deref(), &[]);
            jira::query(
                c,
                &jql,
                limit.or(first).unwrap_or(25),
                &fields,
                after.as_deref(),
            )
        }
        WorkitemCmd::Create {
            space,
            issue_type,
            summary,
            description,
            description_format,
            assignee,
            reporter,
            priority,
            labels,
            parent,
            field,
            fields_json,
            yes: _,
        } => {
            let description = description.map(text_arg).transpose()?;
            jira::create(
                c,
                jira::NewWorkitem {
                    space: &space,
                    issue_type: &issue_type,
                    summary: &summary,
                    description: description.as_deref().map(|d| (d, description_format)),
                    assignee: assignee.as_deref(),
                    reporter: reporter.as_deref(),
                    priority: priority.as_deref(),
                    labels: labels.as_deref().map(jira::csv).unwrap_or_default(),
                    parent: parent.as_deref(),
                    extra: jira::extra_fields(fields_json.as_deref(), &field)?,
                },
            )
        }
        WorkitemCmd::Update(u) => {
            let description = u.description.map(text_arg).transpose()?;
            let comment = u.comment.map(text_arg).transpose()?;
            let key = issue_ref(&u.id);
            jira::update(
                c,
                &key,
                jira::Update {
                    summary: u.summary.as_deref(),
                    description: description.as_deref().map(|d| (d, u.description_format)),
                    assignee: u.assignee.as_deref(),
                    issue_type: u.issue_type.as_deref(),
                    priority: u.priority.as_deref(),
                    labels: list_edit(u.labels, u.add_labels, u.remove_labels),
                    components: list_edit(u.components, u.add_components, u.remove_components),
                    fix_versions: list_edit(
                        u.fix_versions,
                        u.add_fix_versions,
                        u.remove_fix_versions,
                    ),
                    parent: u.parent.as_deref(),
                    due_date: u.due_date.as_deref(),
                    resolution: u.resolution.as_deref(),
                    extra: jira::extra_fields(u.fields_json.as_deref(), &u.field)?,
                    status: u.status.as_deref(),
                    transition_comment: u
                        .transition_comment
                        .as_deref()
                        .map(|t| (t, u.transition_comment_format)),
                    comment: comment.as_deref().map(|t| (t, u.comment_format)),
                },
            )
        }
        WorkitemCmd::Comment {
            cmd:
                CommentCmd::Create {
                    issue_id,
                    body,
                    body_format,
                    internal,
                    visibility_role,
                    visibility_group,
                },
        } => {
            let visibility = match (&visibility_role, &visibility_group) {
                (Some(r), _) => Some(("role", r.as_str())),
                (None, Some(g)) => Some(("group", g.as_str())),
                _ => None,
            };
            jira::comment(
                c,
                &issue_ref(&issue_id),
                &text_arg(body)?,
                body_format,
                visibility,
                internal,
            )
        }
        WorkitemCmd::Transition {
            id,
            transition_id,
            field,
            fields_json,
            transition_comment,
            transition_comment_format,
        } => jira::transition(
            c,
            &issue_ref(&id),
            transition_id.as_deref(),
            jira::extra_fields(fields_json.as_deref(), &field)?,
            transition_comment
                .as_deref()
                .map(|t| (t, transition_comment_format)),
        ),
        WorkitemCmd::Transitions {
            cmd: TransitionsCmd::Query { id },
        } => jira::transitions(c, &issue_ref(&id)),
        WorkitemCmd::Field {
            cmd: WorkitemFieldCmd::CreateMetadata { space, issue_type },
        } => jira::create_meta(c, &space, issue_type.as_deref()),
        WorkitemCmd::CreateMeta { space, issue_type } => {
            deprecated(
                "jira workitem create-meta",
                "jira workitem field create-metadata",
            );
            jira::create_meta(c, &space, issue_type.as_deref())
        }
        WorkitemCmd::LinkTypes {
            cmd: LinkTypesCmd::Query,
        } => jira::link_types(c),
        WorkitemCmd::Link { cmd } => match cmd {
            LinkCmd::Workitem {
                id,
                target_id,
                link_type_id,
            } => {
                let targets: Vec<String> = target_id.iter().map(|t| issue_ref(t)).collect();
                jira::link_workitem(c, &issue_ref(&id), &targets, &link_type_id)
            }
            LinkCmd::Weblink {
                id,
                url,
                title,
                global_id,
                relationship,
                application_name,
                application_type,
                summary,
                icon_url,
                resolved,
            } => jira::link_remote(
                c,
                &issue_ref(&id),
                &jira::Weblink {
                    url: &url,
                    title: &title,
                    relationship: relationship.as_deref(),
                    global_id: global_id.as_deref(),
                    application_name: application_name.as_deref(),
                    application_type: application_type.as_deref(),
                    summary: summary.as_deref(),
                    icon_url: icon_url.as_deref(),
                    resolved,
                },
            ),
            LinkCmd::Types => {
                deprecated("jira workitem link types", "jira workitem link-types query");
                jira::link_types(c)
            }
            LinkCmd::Create {
                inward,
                outward,
                link_type,
                comment,
                comment_format,
            } => {
                deprecated("jira workitem link create", "jira workitem link workitem");
                let comment = comment.map(text_arg).transpose()?;
                jira::link_create(
                    c,
                    &issue_ref(&inward),
                    &issue_ref(&outward),
                    &link_type,
                    comment.as_deref().map(|t| (t, comment_format)),
                )
            }
            LinkCmd::Remote {
                id,
                url,
                title,
                relationship,
            } => {
                deprecated("jira workitem link remote", "jira workitem link weblink");
                jira::link_remote(
                    c,
                    &issue_ref(&id),
                    &jira::Weblink {
                        url: &url,
                        title: &title,
                        relationship: relationship.as_deref(),
                        ..Default::default()
                    },
                )
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    #[test]
    fn issue_refs() {
        assert_eq!(issue_ref("PROJ-1"), "PROJ-1");
        assert_eq!(
            issue_ref("https://a.atlassian.net/browse/PROJ-9?focusedCommentId=1"),
            "PROJ-9"
        );
        assert_eq!(issue_ref("ari:cloud:jira:abc:issue/10001"), "10001");
    }

    #[test]
    fn parses_twg_names_and_old_aliases() {
        for ok in [
            "atlx jira space query --query PROJ --key A --key B --id 1 --type-key software --start-at 5 --first 10",
            "atlx jira space list --after 25 --limit 5",
            "atlx jira space get PROJ",
            "atlx jira project list --query PROJ --limit 3",
            "atlx jira project get PROJ",
            "atlx jira board query --issue PROJ-1 --name team --filter-id 10100 --max-results 5 --start-at 5",
            "atlx jira board query --project-ari ari:cloud:jira:abc:project/10000",
            "atlx jira board list --project PROJ --type kanban",
            "atlx jira board sprints query --id 12 --current",
            "atlx jira board sprints query --board-id 'Team Board' --state active,future",
            "atlx jira board sprints query --project PROJ",
            "atlx jira sprint list --board 12",
            "atlx jira sprint workitems query --id 99 --jql 'status = Done' --fields labels --expand names",
            "atlx jira sprint issues --sprint 99",
            "atlx jira workitem field create-metadata --space PROJ --type Bug",
            "atlx jira workitem custom-fields create-metadata --space PROJ",
            "atlx jira workitem create-meta --space PROJ",
            "atlx jira workitem link-types query",
            "atlx jira workitem link types",
            "atlx jira workitem link workitem --id PROJ-1 --target-id PROJ-2 --target-id PROJ-3 --link-type-id Blocks",
            "atlx jira workitem link create --inward PROJ-1 --outward PROJ-2 --type Blocks",
            "atlx jira workitem link weblink --id PROJ-1 --url https://e.x --title T --global-id g --application-name A --application-type t --summary s --icon-url https://e.x/i.png --resolved",
            "atlx jira workitem link remote --id PROJ-1 --url https://e.x --title T",
            "atlx jira workitem get PROJ-1 PROJ-2 --batch-concurrency 20",
            "atlx jira workitem query --jql 'project = P' -n 5 --last 3 --before c",
            "atlx user search --email a@example.com -n 3",
            "atlx user search --account-id abc",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jira workitem link workitem --id PROJ-1 --link-type-id Blocks",
            "atlx jira workitem get PROJ-1 --batch-concurrency 0",
            "atlx jira board sprints query --id 1 --state active --current",
            "atlx jira board query --project P --issue PROJ-1",
            "atlx jira sprint workitems query --id 1 --sprint-id 2",
            "atlx jira space query --start-at 1 --after 2",
            "atlx user search alice --account-id abc",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn project_and_filter_refs() {
        assert_eq!(project_ref("ari:cloud:jira:abc:project/10000"), "10000");
        assert_eq!(project_ref("PROJ"), "PROJ");
        assert_eq!(
            jira::filter_ref("https://a.atlassian.net/issues/?filter=10100"),
            "10100"
        );
        assert_eq!(jira::filter_ref("ari:cloud:jira:abc:filter/7"), "7");
        assert_eq!(jira::filter_ref("42"), "42");
    }

    #[test]
    fn rejects_conflicting_jira_update_list_flags() {
        for bad in [
            "atlx jira workitem update --id PROJ-1 --labels a --add-labels b",
            "atlx jira workitem update --id PROJ-1 --labels a --remove-labels b",
            "atlx jira workitem update --id PROJ-1 --components a --add-components b",
            "atlx jira workitem update --id PROJ-1 --components a --remove-components b",
            "atlx jira workitem update --id PROJ-1 --fix-versions a --add-fix-versions b",
            "atlx jira workitem update --id PROJ-1 --fix-versions a --remove-fix-versions b",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
        let ok = "atlx jira workitem update --id PROJ-1 --add-labels a --remove-labels b";
        assert!(Cli::try_parse_from(shell_words(ok)).is_ok());
    }
}

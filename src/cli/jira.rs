//! `atlx jira`: clap definitions and dispatch.

use super::util::{DEFAULT_CONCURRENCY, batch, concurrency_arg, deprecated, text_arg};
use crate::client::{self, ApiError};
use crate::jira;
use crate::markup::TextFormat;
use crate::output::envelope::envelope;
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

mod admin;
mod agile;
mod collab;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum JiraCmd {
    /// Jira work items (issues)
    Workitem {
        #[command(subcommand)]
        cmd: WorkitemCmd,
    },
    /// Saved filters
    Filter {
        #[command(subcommand)]
        cmd: admin::FilterCmd,
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
    /// Create a space
    Create(admin::SpaceCreateArgs),
    /// Change a space
    Update(admin::SpaceUpdateArgs),
    /// Move a space to the trash
    Delete(admin::ConfirmedSpace),
    /// Archive a space
    Archive(admin::ConfirmedSpace),
    /// Bring an archived or trashed space back
    Restore(admin::ConfirmedSpace),
    /// The issue types of a space
    IssueTypes(admin::IssueTypesArgs),
    /// The statuses used in a space
    Status {
        #[command(subcommand)]
        cmd: admin::StatusCmd,
    },
    /// Space (project) types
    Types {
        #[command(subcommand)]
        cmd: admin::SpaceTypesCmd,
    },
    /// Notification schemes
    NotificationScheme {
        #[command(subcommand)]
        cmd: admin::NotificationSchemeCmd,
    },
    /// A space's versions
    Versions {
        #[command(subcommand)]
        cmd: admin::VersionsCmd,
    },
    /// One version
    Version {
        #[command(subcommand)]
        cmd: admin::VersionCmd,
    },
    /// Components
    Component {
        #[command(subcommand)]
        cmd: admin::ComponentCmd,
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
    #[arg(long, allow_hyphen_values = true)]
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
    /// One board, with its columns and estimation
    Get(agile::BoardGetArgs),
    /// Create a board from a filter
    Create(agile::BoardCreateArgs),
    /// Delete a board
    Delete(agile::BoardDeleteArgs),
    /// A board's backlog
    Backlog {
        #[command(subcommand)]
        cmd: agile::BacklogCmd,
    },
    /// The spaces (projects) on a board
    Projects {
        #[command(subcommand)]
        cmd: agile::BoardProjectsCmd,
    },
    /// A board's quick filters
    QuickFilter {
        #[command(subcommand)]
        cmd: agile::QuickFilterCmd,
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
    /// One sprint
    Get(agile::GetArgs),
    /// Create a future sprint on a board
    Create(agile::CreateArgs),
    /// Change some of a sprint's fields
    Update(agile::UpdateSprintArgs),
    /// Start a sprint; it needs --end-date or --duration unless it has an end
    Start(agile::StartArgs),
    /// Complete a sprint
    Complete(agile::CompleteArgs),
    /// Delete a sprint
    Delete(agile::DeleteArgs),
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
        /// Run a saved filter's JQL: ID, URL or exact name (atlx extra)
        #[arg(long, conflicts_with_all = ["jql", "jql_arg"])]
        filter: Option<String>,
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
        #[command(flatten)]
        planning: agile::PlanningArgs,
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
    /// Remove issue links and remote web links
    Unlink {
        #[command(subcommand)]
        cmd: UnlinkCmd,
    },
    /// Delete a work item
    Delete {
        /// Work item key, ID, ARI or URL
        #[arg(value_name = "id", conflicts_with = "id")]
        id_arg: Option<String>,
        /// Same as the positional argument
        #[arg(long)]
        id: Option<String>,
        /// Also delete the work item's subtasks (true or false; Jira refuses
        /// to delete a work item with subtasks otherwise)
        #[arg(long, value_name = "BOOL")]
        delete_subtasks: Option<bool>,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
    },
    /// Change history of a work item
    Changelog {
        #[command(subcommand)]
        cmd: ChangelogCmd,
    },
    /// Issue types
    Types {
        #[command(subcommand)]
        cmd: TypesCmd,
    },
    /// Work item statuses
    Statuses {
        #[command(subcommand)]
        cmd: StatusesCmd,
    },
    /// Work item priorities
    Priorities {
        #[command(subcommand)]
        cmd: PrioritiesCmd,
    },
    /// Files attached to work items
    Attachment {
        #[command(subcommand)]
        cmd: collab::AttachmentCmd,
    },
    /// Time logged on work items
    Worklog {
        #[command(subcommand)]
        cmd: collab::WorklogCmd,
    },
    /// Who watches a work item
    Watcher {
        #[command(subcommand)]
        cmd: collab::WatcherCmd,
    },
    /// Votes on a work item
    Vote {
        #[command(subcommand)]
        cmd: collab::VoteCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChangelogCmd {
    /// One page of a work item's change history, oldest first
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Page size [default: 100]
        #[arg(long)]
        first: Option<usize>,
        /// Cursor from the previous page's pageInfo.nextCursor
        #[arg(long)]
        after: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum TypesCmd {
    /// The issue types of a space (project)
    Query {
        /// Space key, ID, ARI or URL
        #[arg(long)]
        project: Option<String>,
        /// Use the space of this work item
        #[arg(long)]
        issue: Option<String>,
        /// Same as --project
        #[arg(long)]
        project_id: Option<String>,
        /// Same as --project
        #[arg(long)]
        project_key: Option<String>,
        /// Hierarchy level: -1 subtask, 0 standard, 1 epic
        #[arg(long, allow_hyphen_values = true)]
        level: Option<i64>,
        /// Maximum number of types [default: 50]
        #[arg(long, short = 'n', conflicts_with = "first")]
        limit: Option<usize>,
        /// Same as --limit
        #[arg(long)]
        first: Option<usize>,
        /// Look up these type IDs instead (prefer `types get`); repeatable
        #[arg(long)]
        id: Vec<String>,
    },
    /// Issue types by ID or ARI
    Get {
        /// Issue type ID or ARI; repeat for several
        #[arg(long, required = true)]
        id: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum StatusesCmd {
    /// Every status on the site, or the given ones
    Query {
        /// Status ID or ARI; repeat for several
        #[arg(long)]
        id: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum PrioritiesCmd {
    /// Every priority on the site, or the given ones
    Query {
        /// Priority ID or ARI; repeat for several
        #[arg(long)]
        id: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum UnlinkCmd {
    /// Delete a link between two work items
    Workitem {
        /// Issue link ID (see `jira workitem link query`)
        #[arg(long)]
        link_id: String,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
    },
    /// Delete a remote web link from a work item
    #[command(group(clap::ArgGroup::new("which").required(true).args(["link_id", "global_id", "url"])))]
    Weblink {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        id: String,
        /// Remote link ID
        #[arg(long)]
        link_id: Option<String>,
        /// The link's global ID
        #[arg(long)]
        global_id: Option<String>,
        /// The linked URL; the one remote link pointing there is deleted
        #[arg(long)]
        url: Option<String>,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
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
    /// Fields that can be edited on a work item, with allowed values
    UpdateMetadata {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        id: String,
        /// List editable built-in fields too, not only custom fields
        #[arg(long)]
        include_system_fields: bool,
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
    #[command(flatten)]
    planning: agile::PlanningArgs,
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
        /// Reply to this comment (ID) on the same work item
        #[arg(long)]
        parent_id: Option<String>,
    },
    /// One page of a work item's comments
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
        /// Page size [default: 100]
        #[arg(long)]
        first: Option<usize>,
        /// Cursor from the previous page's pageInfo.nextCursor
        #[arg(long)]
        after: Option<String>,
        /// created (oldest first) or -created (newest first)
        #[arg(long, allow_hyphen_values = true)]
        order_by: Option<String>,
    },
    /// Replace the body of a comment
    Update {
        /// Comment ID
        #[arg(long)]
        id: String,
        /// Work item the comment is on
        #[arg(long)]
        issue_id: String,
        /// New body; `-` reads stdin
        #[arg(long)]
        body: String,
        #[arg(long, value_enum, default_value_t)]
        body_format: TextFormat,
    },
    /// Delete a comment
    Delete {
        /// Comment ID
        #[arg(long)]
        id: String,
        /// Work item the comment is on
        #[arg(long)]
        issue_id: String,
        /// Accepted for compatibility; nothing asks for confirmation
        #[arg(long, short, hide = true)]
        yes: bool,
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
    /// The issue links of a work item
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
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

/// `--fields` for query-style commands: `None` keeps twg's compact rows.
fn fields_opt(fields: Option<&str>) -> Option<Vec<String>> {
    fields.map(jira::csv).filter(|f| !f.is_empty())
}

/// twg's envelope around a work item query or search page.
fn found(
    c: &client::Client,
    command: &str,
    jql: &str,
    limit: usize,
    fields: Option<&[String]>,
    f: jira::Found,
) -> Value {
    let mut request = json!({"jql": jql, "site": jira::site(c), "maxResults": limit});
    if let Some(fields) = fields {
        request["fields"] = json!(fields);
    }
    envelope(command, "jira.workitem")
        .request(request)
        .next_cursor(f.next)
        .data(f.data)
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
        JiraCmd::Space {
            cmd: SpaceCmd::Create(a),
        } => admin::space_create(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Update(a),
        } => admin::space_update(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Delete(a),
        } => admin::space_delete(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Archive(a),
        } => admin::space_archive(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Restore(a),
        } => admin::space_restore(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::IssueTypes(a),
        } => admin::issue_types(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Status { cmd },
        } => admin::status(c, cmd),
        JiraCmd::Space {
            cmd: SpaceCmd::Types { cmd },
        } => admin::types(c, cmd),
        JiraCmd::Space {
            cmd: SpaceCmd::NotificationScheme { cmd },
        } => admin::notification_scheme(c, cmd),
        JiraCmd::Space {
            cmd: SpaceCmd::Versions { cmd },
        } => admin::versions(c, cmd),
        JiraCmd::Space {
            cmd: SpaceCmd::Version { cmd },
        } => admin::version(c, cmd),
        JiraCmd::Space {
            cmd: SpaceCmd::Component { cmd },
        } => admin::component(c, cmd),
        JiraCmd::Filter { cmd } => admin::filter(c, cmd),
        JiraCmd::Field {
            cmd: FieldCmd::List { query, custom_only },
        } => Ok(envelope("jira.field.list", "jira.field")
            .request(json!({"site": jira::site(c), "query": query, "customOnly": custom_only}))
            .data(jira::fields(c, query.as_deref(), custom_only)?)),
        JiraCmd::Space {
            cmd: SpaceCmd::Query(a),
        } => space_query(c, a),
        JiraCmd::Space {
            cmd: SpaceCmd::Get { identifier },
        } => space_get(c, &identifier),
        JiraCmd::Project {
            cmd: ProjectCmd::List { query, limit },
        } => {
            deprecated("jira project list", "jira space query");
            space_envelope(
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
            space_get(c, &identifier)
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
            board_envelope(
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
            sprints_envelope(c, board, state.as_deref(), start_at, max_results)
        }
        JiraCmd::Board {
            cmd: BoardCmd::Get(a),
        } => agile::board_get(c, a),
        JiraCmd::Board {
            cmd: BoardCmd::Create(a),
        } => agile::board_create(c, a),
        JiraCmd::Board {
            cmd: BoardCmd::Delete(a),
        } => agile::board_delete(c, a),
        JiraCmd::Board {
            cmd: BoardCmd::Backlog { cmd },
        } => agile::backlog(c, cmd),
        JiraCmd::Board {
            cmd: BoardCmd::Projects { cmd },
        } => agile::projects(c, cmd),
        JiraCmd::Board {
            cmd: BoardCmd::QuickFilter { cmd },
        } => agile::quick_filter(c, cmd),
        JiraCmd::Sprint {
            cmd: SprintCmd::Get(a),
        } => agile::sprint_get(c, a),
        JiraCmd::Sprint {
            cmd: SprintCmd::Create(a),
        } => agile::sprint_create(c, a),
        JiraCmd::Sprint {
            cmd: SprintCmd::Update(a),
        } => agile::sprint_update(c, a),
        JiraCmd::Sprint {
            cmd: SprintCmd::Start(a),
        } => agile::sprint_start(c, a),
        JiraCmd::Sprint {
            cmd: SprintCmd::Complete(a),
        } => agile::sprint_complete(c, a),
        JiraCmd::Sprint {
            cmd: SprintCmd::Delete(a),
        } => agile::sprint_delete(c, a),
        JiraCmd::Board {
            cmd:
                BoardCmd::List {
                    project,
                    board_type,
                    limit,
                },
        } => {
            deprecated("jira board list", "jira board query");
            board_envelope(
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
            sprint_issues_envelope(
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
            sprints_envelope(c, board, state.as_deref(), 0, limit)
        }
        JiraCmd::Sprint {
            cmd: SprintCmd::Issues { sprint, jql, limit },
        } => {
            deprecated("jira sprint issues", "jira sprint workitems query");
            sprint_issues_envelope(
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

fn space_envelope(c: &client::Client, q: &jira::SpaceQuery) -> Result<Value> {
    Ok(envelope("jira.space.query", "jira.space")
        .request(json!({
            "site": jira::site(c),
            "maxResults": q.limit,
            "orderBy": q.order_by.unwrap_or("key"),
        }))
        .data(jira::space_query(c, q)?))
}

fn space_get(c: &client::Client, identifier: &str) -> Result<Value> {
    Ok(envelope("jira.space.get", "jira.space")
        .request(json!({"site": jira::site(c), "projectIdOrKey": identifier}))
        .data(jira::project(c, identifier)?))
}

fn board_envelope(c: &client::Client, q: &jira::BoardQuery) -> Result<Value> {
    Ok(envelope("jira.board.query", "jira.board-search-result")
        .request(json!({"site": jira::site(c)}))
        .data(jira::board_query(c, q)?))
}

fn sprints_envelope(
    c: &client::Client,
    board: u64,
    state: Option<&str>,
    start: usize,
    limit: usize,
) -> Result<Value> {
    Ok(envelope("jira.board.sprints.query", "jira.sprint")
        .request(json!({"site": jira::site(c), "boardId": board, "state": state}))
        .data(jira::sprints(c, board, state, start, limit)?))
}

fn sprint_issues_envelope(
    c: &client::Client,
    sprint: u64,
    o: &jira::SprintIssues,
) -> Result<Value> {
    Ok(envelope("jira.sprint.workitems.query", "jira.workitem")
        .request(json!({"site": jira::site(c), "sprintId": sprint, "jql": o.jql}))
        .data(jira::sprint_issues(c, sprint, o)?))
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
    space_envelope(
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

fn create_meta(c: &client::Client, space: &str, issue_type: Option<&str>) -> Result<Value> {
    Ok(
        envelope("jira.workitem.field.create-metadata", "jira.workitem.field")
            .request(json!({"site": jira::site(c), "space": space, "type": issue_type}))
            .data(jira::create_meta(c, space, issue_type)?),
    )
}

fn link_types(c: &client::Client) -> Result<Value> {
    Ok(
        envelope("jira.workitem.link-types.query", "jira.issue-link-type")
            .request(json!({"site": jira::site(c)}))
            .data(jira::link_types(c)?),
    )
}

fn weblink(c: &client::Client, key: &str, w: &jira::Weblink) -> Result<Value> {
    Ok(envelope("jira.workitem.link.weblink", "jira.remote-link")
        .request(json!({"site": jira::site(c), "id": key}))
        .data(jira::link_remote(c, key, w)?))
}

/// Default page size of the offset-paged work item lists, as in twg.
const PAGE: usize = 100;

/// `request` for an offset-paged work item list. A numeric cursor is
/// echoed as a number, as twg does.
fn page_request(c: &client::Client, key: &str, first: Option<usize>, after: Option<&str>) -> Value {
    let mut r = json!({"site": jira::site(c), "issueId": key});
    if let Some(f) = first {
        r["first"] = f.into();
    }
    if let Some(a) = after {
        r["after"] = a
            .trim()
            .parse::<u64>()
            .map_or_else(|_| Value::from(a), Value::from);
    }
    r
}

fn offset_envelope(
    command: &str,
    resource_type: &str,
    request: Value,
    page: jira::OffsetPage,
) -> Value {
    envelope(command, resource_type)
        .request(request)
        .page_info(page.page_info())
        .meta("pagination", page.pagination())
        .data(page.data)
}

/// Add a top-level `warnings` list after `meta`, as twg prints it.
fn with_warning(mut out: Value, warning: &str) -> Value {
    out["warnings"] = json!([warning]);
    out
}

/// `request` of the lookups that take repeated `--id`s.
fn ids_request(c: &client::Client, ids: &[String]) -> Value {
    let mut r = json!({"site": jira::site(c)});
    if !ids.is_empty() {
        r["ids"] = json!(ids);
    }
    r
}

/// A space key or ID from a key, ID, ARI, or a `/projects/KEY` or
/// `/browse/KEY-1` URL.
fn space_ref(s: &str) -> String {
    let s = s.trim();
    for marker in ["/projects/", "/browse/"] {
        if let Some(i) = s.find(marker) {
            let seg = s[i + marker.len()..]
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            let seg = if marker == "/browse/" {
                seg.rsplit_once('-').map_or(seg, |(k, _)| k)
            } else {
                seg
            };
            if !seg.is_empty() {
                return seg.to_string();
            }
        }
    }
    project_ref(s)
}

fn types(c: &client::Client, cmd: TypesCmd) -> Result<Value> {
    match cmd {
        TypesCmd::Get { id } => {
            let ids: Vec<String> = id.iter().map(|i| jira::entity_id(i)).collect();
            Ok(envelope("jira.workitem.types.get", "jira.workitem-type")
                .request(ids_request(c, &ids))
                .data(jira::issue_types_get(c, &ids)?))
        }
        TypesCmd::Query { id, .. } if !id.is_empty() => {
            let ids: Vec<String> = id.iter().map(|i| jira::entity_id(i)).collect();
            let out = envelope("jira.workitem.types.query", "jira.workitem-type")
                .request(ids_request(c, &ids))
                .data(jira::issue_types_get(c, &ids)?);
            Ok(with_warning(
                out,
                "`jira workitem types query --id` is an older form; use `jira workitem types get --id`.",
            ))
        }
        TypesCmd::Query {
            project,
            issue,
            project_id,
            project_key,
            level,
            limit,
            first,
            id: _,
        } => {
            let project = match (project.or(project_id).or(project_key), issue) {
                (Some(p), _) => space_ref(&p),
                (None, Some(i)) => jira::issue_project(c, &issue_ref(&i))?,
                (None, None) => {
                    return Err(ApiError::usage(
                        "pass --project (or --project-id, --project-key or --issue)",
                    )
                    .into());
                }
            };
            let (key, project_id) = jira::project_key_id(c, &project)?;
            let limit = limit.or(first).unwrap_or(50);
            let mut request =
                json!({"site": jira::site(c), "project": key, "projectId": project_id});
            if let Some(l) = level {
                request["level"] = l.into();
            }
            request["first"] = limit.into();
            let (data, found) = jira::issue_types(c, &project_id, level, limit)?;
            let out = envelope("jira.workitem.types.query", "jira.workitem-type")
                .request(request)
                .data(data);
            Ok(if found > limit {
                with_warning(
                    out,
                    &format!(
                        "Only {limit} of the {found} issue types of {key} are shown; \
                         raise --limit to {found} to see them all."
                    ),
                )
            } else {
                out
            })
        }
    }
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
                fields: fields_opt(fields.as_deref()),
                extra: field,
                comments,
                remote_links,
                full,
            };
            let mut request = json!({
                "issueIdOrKey": if inputs.len() == 1 { json!(inputs[0]) } else { json!(inputs) },
                "site": jira::site(c),
            });
            if full {
                request["fields"] = json!(["*all"]);
            } else if let Some(f) = &opts.fields {
                request["fields"] = json!(f);
            }
            for (k, on) in [
                ("comments", comments || full),
                ("remoteLinks", remote_links || full),
                ("full", full),
            ] {
                if on {
                    request[k] = true.into();
                }
            }
            let single = inputs.len() == 1;
            let data = batch(inputs, strict, batch_concurrency, |k| {
                jira::get(c, k, &opts)
            })?;
            // One work item prints as a one-element list, as in twg.
            let data = if single { json!([data]) } else { data };
            Ok(envelope("jira.workitem.get", "jira.workitem")
                .request(request)
                .data(data))
        }
        WorkitemCmd::Search {
            text,
            limit,
            first,
            after,
            fields,
        } => {
            let fields = fields_opt(fields.as_deref());
            let text = text.join(" ");
            let limit = limit.or(first).unwrap_or(25);
            let f = jira::search(c, &text, limit, fields.as_deref(), after.as_deref())?;
            let jql = jira::search_jql(&text);
            Ok(found(
                c,
                "jira.workitem.search",
                &jql,
                limit,
                fields.as_deref(),
                f,
            ))
        }
        WorkitemCmd::Query {
            jql_arg,
            jql,
            filter,
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
            let saved = filter
                .as_deref()
                .map(|f| jira::admin::filter_jql(c, f))
                .transpose()?;
            let Some(jql) = saved.or(jql).or(jql_arg) else {
                bail!("pass --jql, a JQL argument or --filter")
            };
            let fields = fields_opt(fields.as_deref());
            let limit = limit.or(first).unwrap_or(25);
            let f = jira::query(c, &jql, limit, fields.as_deref(), after.as_deref())?;
            Ok(found(
                c,
                "jira.workitem.query",
                &jql,
                limit,
                fields.as_deref(),
                f,
            ))
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
            planning,
            yes: _,
        } => {
            let description = description.map(text_arg).transpose()?;
            let mut extra = jira::extra_fields(fields_json.as_deref(), &field)?;
            let planned =
                jira::agile::plan(c, jira::agile::On::Space(&space), &planning.planning())?;
            extra.extend(planned.fields);
            let mut data = jira::create(
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
                    extra,
                },
            )?;
            // A new work item is already in the backlog, so only sprints need a move.
            if let Some(to @ jira::agile::Destination::Sprint(id)) = planned.sprint {
                let key = data["key"].as_str().unwrap_or_default().to_string();
                // The work item exists by now: say so, or a retry makes a second one.
                jira::agile::move_issues(c, to, std::slice::from_ref(&key)).map_err(|e| {
                    let mut err = ApiError::from_anyhow(&e);
                    err.message = format!(
                        "created {key}, but could not add it to sprint {id} (don't create it again): {}",
                        err.message
                    );
                    anyhow::Error::from(err)
                })?;
                data["sprint"] = id.into();
            }
            Ok(envelope("jira.workitem.create", "jira.workitem")
                .request(json!({"site": jira::site(c), "space": space, "type": issue_type}))
                .data(data))
        }
        WorkitemCmd::Update(u) => {
            let description = u.description.map(text_arg).transpose()?;
            let comment = u.comment.map(text_arg).transpose()?;
            let key = issue_ref(&u.id);
            let mut extra = jira::extra_fields(u.fields_json.as_deref(), &u.field)?;
            let planned =
                jira::agile::plan(c, jira::agile::On::Issue(&key), &u.planning.planning())?;
            extra.extend(planned.fields);
            let data = jira::update(
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
                    extra,
                    sprint: planned.sprint,
                    status: u.status.as_deref(),
                    transition_comment: u
                        .transition_comment
                        .as_deref()
                        .map(|t| (t, u.transition_comment_format)),
                    comment: comment.as_deref().map(|t| (t, u.comment_format)),
                },
            )?;
            Ok(envelope("jira.workitem.update", "jira.workitem")
                .request(json!({"site": jira::site(c), "idOrKey": key}))
                .data(data))
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
                    parent_id,
                },
        } => {
            let visibility = match (&visibility_role, &visibility_group) {
                (Some(r), _) => Some(("role", r.as_str())),
                (None, Some(g)) => Some(("group", g.as_str())),
                _ => None,
            };
            let key = issue_ref(&issue_id);
            let parent = parent_id.as_deref().map(jira::entity_id);
            let data = jira::comment(
                c,
                &key,
                &text_arg(body)?,
                body_format,
                visibility,
                internal,
                parent.as_deref(),
            )?;
            let mut request = json!({"site": jira::site(c), "issueIdOrKey": key});
            if let Some(p) = &parent {
                request["parentId"] = p.as_str().into();
            }
            Ok(envelope("jira.workitem.comment.create", "jira.comment")
                .request(request)
                .data(data))
        }
        WorkitemCmd::Comment {
            cmd:
                CommentCmd::Query {
                    issue_id,
                    first,
                    after,
                    order_by,
                },
        } => {
            let key = issue_ref(&issue_id);
            let start = jira::offset_cursor(after.as_deref())?;
            let page =
                jira::comment_query(c, &key, start, first.unwrap_or(PAGE), order_by.as_deref())?;
            let mut request = page_request(c, &key, first, after.as_deref());
            if let Some(o) = &order_by {
                request["orderBy"] = o.as_str().into();
            }
            Ok(offset_envelope(
                "jira.workitem.comment.query",
                "jira.comment",
                request,
                page,
            ))
        }
        WorkitemCmd::Comment {
            cmd:
                CommentCmd::Update {
                    id,
                    issue_id,
                    body,
                    body_format,
                },
        } => {
            let key = issue_ref(&issue_id);
            let id = jira::entity_id(&id);
            let data = jira::comment_update(c, &key, &id, &text_arg(body)?, body_format)?;
            Ok(envelope("jira.workitem.comment.update", "jira.comment")
                .request(json!({"site": jira::site(c), "id": id, "issueId": key}))
                .data(data))
        }
        WorkitemCmd::Comment {
            cmd:
                CommentCmd::Delete {
                    id,
                    issue_id,
                    yes: _,
                },
        } => {
            let key = issue_ref(&issue_id);
            let id = jira::entity_id(&id);
            Ok(envelope("jira.workitem.comment.delete", "jira.comment")
                .request(json!({"site": jira::site(c), "id": id, "issueId": key}))
                .data(jira::comment_delete(c, &key, &id)?))
        }
        WorkitemCmd::Changelog {
            cmd:
                ChangelogCmd::Query {
                    issue_id,
                    first,
                    after,
                },
        } => {
            let key = issue_ref(&issue_id);
            let start = jira::offset_cursor(after.as_deref())?;
            let page = jira::changelog_query(c, &key, start, first.unwrap_or(PAGE))?;
            Ok(offset_envelope(
                "jira.workitem.changelog.query",
                "jira.changelog",
                page_request(c, &key, first, after.as_deref()),
                page,
            ))
        }
        WorkitemCmd::Delete {
            id_arg,
            id,
            delete_subtasks,
            yes: _,
        } => {
            let Some(id) = id_arg.or(id) else {
                return Err(
                    ApiError::usage("pass the work item as an argument or with --id").into(),
                );
            };
            let key = issue_ref(&id);
            let subtasks = delete_subtasks.unwrap_or(false);
            Ok(envelope("jira.workitem.delete", "jira.workitem")
                .request(json!({"site": jira::site(c), "id": key, "deleteSubtasks": subtasks}))
                .data(jira::delete(c, &key, subtasks)?))
        }
        WorkitemCmd::Unlink { cmd } => match cmd {
            UnlinkCmd::Workitem { link_id, yes: _ } => {
                let link_id = jira::entity_id(&link_id);
                Ok(envelope("jira.workitem.unlink.workitem", "jira.issue-link")
                    .request(json!({"site": jira::site(c), "linkId": link_id}))
                    .data(jira::unlink_workitem(c, &link_id)?))
            }
            UnlinkCmd::Weblink {
                id,
                link_id,
                global_id,
                url,
                yes: _,
            } => {
                let key = issue_ref(&id);
                let which = match (&link_id, &global_id, &url) {
                    (Some(l), _, _) => jira::WeblinkRef::Id(l),
                    (None, Some(g), _) => jira::WeblinkRef::GlobalId(g),
                    (None, None, Some(u)) => jira::WeblinkRef::Url(u),
                    (None, None, None) => {
                        return Err(ApiError::usage("pass --link-id, --global-id or --url").into());
                    }
                };
                Ok(envelope("jira.workitem.unlink.weblink", "jira.remote-link")
                    .request(json!({
                        "site": jira::site(c),
                        "id": key,
                        "linkId": link_id,
                        "globalId": global_id,
                        "url": url,
                    }))
                    .data(jira::unlink_weblink(c, &key, which)?))
            }
        },
        WorkitemCmd::Types { cmd } => types(c, cmd),
        WorkitemCmd::Statuses {
            cmd: StatusesCmd::Query { id },
        } => {
            let ids: Vec<String> = id.iter().map(|i| jira::entity_id(i)).collect();
            Ok(envelope("jira.workitem.statuses.query", "jira.status")
                .request(ids_request(c, &ids))
                .data(jira::statuses(c, &ids)?))
        }
        WorkitemCmd::Attachment { cmd } => collab::attachment(c, cmd),
        WorkitemCmd::Worklog { cmd } => collab::worklog(c, cmd),
        WorkitemCmd::Watcher { cmd } => collab::watcher(c, cmd),
        WorkitemCmd::Vote { cmd } => collab::vote(c, cmd),
        WorkitemCmd::Priorities {
            cmd: PrioritiesCmd::Query { id },
        } => {
            let ids: Vec<String> = id.iter().map(|i| jira::entity_id(i)).collect();
            Ok(envelope("jira.workitem.priorities.query", "jira.priority")
                .request(ids_request(c, &ids))
                .data(jira::priorities(c, &ids)?))
        }
        WorkitemCmd::Transition {
            id,
            transition_id,
            field,
            fields_json,
            transition_comment,
            transition_comment_format,
        } => {
            let key = issue_ref(&id);
            let data = jira::transition(
                c,
                &key,
                transition_id.as_deref(),
                jira::extra_fields(fields_json.as_deref(), &field)?,
                transition_comment
                    .as_deref()
                    .map(|t| (t, transition_comment_format)),
            )?;
            let (command, kind) = if transition_id.is_some() {
                ("jira.workitem.transition", "jira.workitem")
            } else {
                ("jira.workitem.transitions", "jira.transition")
            };
            Ok(envelope(command, kind)
                .request(json!({"idOrKey": key, "transitionId": transition_id}))
                .data(data))
        }
        WorkitemCmd::Transitions {
            cmd: TransitionsCmd::Query { id },
        } => {
            let key = issue_ref(&id);
            Ok(envelope("jira.workitem.transitions", "jira.transition")
                .request(json!({"idOrKey": key}))
                .data(jira::transitions(c, &key)?))
        }
        WorkitemCmd::Field {
            cmd: WorkitemFieldCmd::CreateMetadata { space, issue_type },
        } => create_meta(c, &space, issue_type.as_deref()),
        WorkitemCmd::Field {
            cmd:
                WorkitemFieldCmd::UpdateMetadata {
                    id,
                    include_system_fields,
                },
        } => {
            let key = issue_ref(&id);
            let mut request = json!({"site": jira::site(c), "id": key});
            if include_system_fields {
                request["includeSystemFields"] = true.into();
            }
            Ok(
                envelope("jira.workitem.field.update-metadata", "jira.workitem.field")
                    .request(request)
                    .meta("resourceClass", "collection")
                    .data(jira::update_meta(c, &key, include_system_fields)?),
            )
        }
        WorkitemCmd::CreateMeta { space, issue_type } => {
            deprecated(
                "jira workitem create-meta",
                "jira workitem field create-metadata",
            );
            create_meta(c, &space, issue_type.as_deref())
        }
        WorkitemCmd::LinkTypes {
            cmd: LinkTypesCmd::Query,
        } => link_types(c),
        WorkitemCmd::Link { cmd } => match cmd {
            LinkCmd::Workitem {
                id,
                target_id,
                link_type_id,
            } => {
                let targets: Vec<String> = target_id.iter().map(|t| issue_ref(t)).collect();
                let source = issue_ref(&id);
                Ok(envelope("jira.workitem.link.workitem", "jira.issue-link")
                    .request(json!({"id": source, "targetId": targets, "linkTypeId": link_type_id}))
                    .data(jira::link_workitem(c, &source, &targets, &link_type_id)?))
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
            } => weblink(
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
            LinkCmd::Query { issue_id } => {
                let key = issue_ref(&issue_id);
                Ok(envelope("jira.workitem.link.query", "jira.issue-link")
                    .request(json!({"site": jira::site(c), "issueId": key}))
                    .data(jira::link_query(c, &key)?))
            }
            LinkCmd::Types => {
                deprecated("jira workitem link types", "jira workitem link-types query");
                link_types(c)
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
                let (inward, outward) = (issue_ref(&inward), issue_ref(&outward));
                Ok(envelope("jira.workitem.link.workitem", "jira.issue-link")
                    .request(json!({"id": inward, "targetId": [outward], "linkTypeId": link_type}))
                    .data(jira::link_create(
                        c,
                        &inward,
                        &outward,
                        &link_type,
                        comment.as_deref().map(|t| (t, comment_format)),
                    )?))
            }
            LinkCmd::Remote {
                id,
                url,
                title,
                relationship,
            } => {
                deprecated("jira workitem link remote", "jira workitem link weblink");
                weblink(
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
    fn parses_triage_commands() {
        for ok in [
            "atlx jira workitem comment create --issue-id PROJ-1 --body hi --parent-id 10001",
            "atlx jira workitem comment query --issue-id PROJ-1 --first 20 --after 20 --order-by -created",
            "atlx jira workitem comment query --issue-id PROJ-1 --order-by=-created",
            "atlx jira workitem comment update --issue-id PROJ-1 --id 10001 --body x --body-format markdown",
            "atlx jira workitem comment delete --issue-id PROJ-1 --id 10001",
            "atlx jira workitem comment delete --issue-id PROJ-1 --id 10001 --yes",
            "atlx jira workitem changelog query --issue-id PROJ-1 --first 5 --after 5",
            "atlx jira workitem field update-metadata --id PROJ-1 --include-system-fields",
            "atlx jira workitem link query --issue-id PROJ-1",
            "atlx jira workitem unlink workitem --link-id 10500",
            "atlx jira workitem unlink weblink --id PROJ-1 --link-id 7",
            "atlx jira workitem unlink weblink --id PROJ-1 --global-id g",
            "atlx jira workitem unlink weblink --id PROJ-1 --url https://e.x",
            "atlx jira workitem delete PROJ-1",
            "atlx jira workitem delete --id PROJ-1 --delete-subtasks true",
            "atlx jira workitem types query",
            "atlx jira workitem types query --project PROJ --level -1 -n 5",
            "atlx jira workitem types query --issue PROJ-1 --first 3",
            "atlx jira workitem types query --id 10001 --id 10002",
            "atlx jira workitem types get --id 10001",
            "atlx jira workitem statuses query",
            "atlx jira workitem statuses query --id 3 --id 10000",
            "atlx jira workitem priorities query --id 1",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jira workitem comment query",
            "atlx jira workitem unlink weblink --id PROJ-1",
            "atlx jira workitem unlink weblink --id PROJ-1 --link-id 7 --url https://e.x",
            "atlx jira workitem delete PROJ-1 --id PROJ-2",
            "atlx jira workitem delete PROJ-1 --delete-subtasks maybe",
            "atlx jira workitem types get",
            "atlx jira workitem types query -n 1 --first 2",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_attachments_worklogs_watchers_and_votes() {
        for ok in [
            "atlx jira workitem attachment query --issue-id PROJ-1",
            "atlx jira workitem attachment get --attachment-id 10001",
            "atlx jira workitem attachment download --attachment-id 10001 --output-path a.bin --force",
            "atlx jira workitem attachment thumbnail --attachment-id 10001 --out a.png",
            "atlx jira workitem attachment upload --issue-id PROJ-1 --file a --file b",
            "atlx jira workitem attachment delete --attachment-id 10001 --yes",
            "atlx jira workitem worklog add --issue-id PROJ-1 --time-spent '1h 30m' --started 2026-01-05 --comment x --comment-format markdown",
            "atlx jira workitem worklog update --issue-id PROJ-1 --id 7 --time-spent-seconds 60 --adjust-estimate leave --notify-users false",
            "atlx jira workitem worklog query --issue-id PROJ-1 --first 5 --after 5 --started-after 7d",
            "atlx jira workitem worklog get --issue-id PROJ-1 --id 7 --expand properties",
            "atlx jira workitem worklog delete --issue-id PROJ-1 --id 7 --adjust-estimate manual --increase-by 1h --yes",
            "atlx jira workitem worklog changed --since 1763000000000",
            "atlx jira workitem worklog deleted --since 2w",
            "atlx jira workitem watcher query --issue-id PROJ-1",
            "atlx jira workitem watcher add --id PROJ-1",
            "atlx jira workitem watcher add --issue-id PROJ-1 --watcher-id me",
            "atlx jira workitem watcher remove --id PROJ-1 --watcher-id abc",
            "atlx jira workitem vote query --issue-id PROJ-1",
            "atlx jira workitem vote add --issue-id PROJ-1",
            "atlx jira workitem vote remove --issue-id PROJ-1",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jira workitem attachment upload --issue-id PROJ-1",
            "atlx jira workitem worklog add --issue-id PROJ-1 --time-spent 1h --time-spent-seconds 60",
            "atlx jira workitem worklog get --issue-id PROJ-1",
            "atlx jira workitem watcher add",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_agile_commands_and_flags() {
        for ok in [
            "atlx jira sprint get 99",
            "atlx jira sprint get --id 99",
            "atlx jira sprint create --board-id 12 --name S5 --goal ship",
            "atlx jira sprint update --id 99 --state closed --incomplete-issues-destination-id backlog",
            "atlx jira sprint start --id 99 --board-id 12 --start-date 2026-12-01 --end-date 2026-12-14",
            "atlx jira sprint start --id 99 --duration 2w",
            "atlx jira sprint complete --id 99 --move-open-to 100",
            "atlx jira sprint delete --sprint-id 99",
            "atlx jira board get --id 12",
            "atlx jira board create --name Team --type scrum --filter-id 100",
            "atlx jira board delete --board-id 12 --yes",
            "atlx jira board backlog query --board-id 12 --active-quick-filters 1,2",
            "atlx jira board projects query --id 12",
            "atlx jira board quick-filter query --board-id 12",
            "atlx jira board quick-filter get --board-id 12 --quick-filter-id 1",
            "atlx jira workitem update --id PROJ-1 --sprint none --story-points 5 --team none",
            "atlx jira workitem update --id PROJ-1 --original-estimate 2d --remaining-estimate 1d",
            "atlx jira workitem create --space PROJ --summary x --sprint 42 --story-points 3",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jira sprint start --id 99 --duration 2w --end-date 2026-12-14",
            "atlx jira sprint update --id 99 --state open",
            "atlx jira board delete --yes",
            "atlx jira board backlog query",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn parses_space_admin_and_filters() {
        for ok in [
            "atlx jira space create --key NEW --name New --project-type-key software",
            "atlx jira space update --id-or-key PROJ --category-id -1",
            "atlx jira space delete --id-or-key PROJ --yes",
            "atlx jira space archive --id-or-key PROJ --yes",
            "atlx jira space restore --id-or-key PROJ --yes",
            "atlx jira space issue-types --id-or-key PROJ --first 5",
            "atlx jira space status query --id-or-key PROJ",
            "atlx jira space types query --id software",
            "atlx jira space notification-scheme get --key PROJ",
            "atlx jira space versions query --key PROJ --status released --sort-by -releaseDate",
            "atlx jira space version create --key PROJ --name 1.0 --released true",
            "atlx jira space version get 5",
            "atlx jira space version update --id 5 --archived false",
            "atlx jira space version delete --id 5 --move-fix-issues-to 6",
            "atlx jira space version merge --id 5 --move-issues-to 6",
            "atlx jira space version move --id 5 --position First",
            "atlx jira space version counts --id 5",
            "atlx jira space component query --key PROJ",
            "atlx jira space component get 7",
            "atlx jira space component create --key PROJ --name API",
            "atlx jira space component update --id 7 --description d",
            "atlx jira space component delete --id 7",
            "atlx jira space component counts --id 7",
            "atlx jira filter query --filter-name open --order-by -id",
            "atlx jira filter query --favourite",
            "atlx jira filter get 9",
            "atlx jira filter create --name F --jql 'a = 1' --favourite true",
            "atlx jira filter update --id 9 --jql 'a = 2'",
            "atlx jira filter delete --id 9 --yes",
            "atlx jira filter add-favourite --id 9",
            "atlx jira filter remove-favourite --id 9",
            "atlx jira filter change-owner --id 9 --account-id a",
            "atlx jira filter columns get 9",
            "atlx jira filter columns set --id 9 --column summary --column status",
            "atlx jira filter reset-columns --id 9 --yes",
            "atlx jira filter share query --id 9",
            "atlx jira filter share add --id 9 --permission-json {}",
            "atlx jira filter share remove --id 9 --permission-id 3 --yes",
            "atlx jira filter subscription query --id 9",
            "atlx jira workitem query --filter 9",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jira space delete --yes",
            "atlx jira space version move --id 5",
            "atlx jira filter query --favourite --owner me",
            "atlx jira workitem query --filter 9 --jql x",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }

    #[test]
    fn space_refs_from_urls() {
        assert_eq!(space_ref("PROJ"), "PROJ");
        assert_eq!(
            space_ref("https://a.atlassian.net/jira/software/projects/PROJ/boards/1"),
            "PROJ"
        );
        assert_eq!(space_ref("https://a.atlassian.net/browse/PROJ-12"), "PROJ");
        assert_eq!(space_ref("ari:cloud:jira:abc:project/10000"), "10000");
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

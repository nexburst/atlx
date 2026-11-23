//! `jira issue` and its sub-resources.

use super::{
    Confirm, InFormat, Many, OutFormat, Paging, adf_input, default_project, issue_keys, listed,
    not_found, outcome, print_bodies_as,
};
use crate::cache::Cache;
use crate::cli::util::{self, text_arg};
use crate::client::{ApiError, Client};
use crate::jira::{self, agile, attachments, bulk, people, worklogs};

use crate::time::epoch_millis;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Map, Value};
use std::path::PathBuf;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum IssueCmd {
    /// Issues matching JQL or filters, newest change first; or the issues of
    /// a sprint or a board's backlog
    List(ListArgs),
    /// Issues whose text matches, newest change first
    Search {
        /// Words to look for
        #[arg(required = true)]
        text: Vec<String>,
        /// Only this project
        #[arg(long)]
        project: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Issues by key, ID or URL, with the description as Markdown
    Get {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        body: BodyOut,
        #[command(flatten)]
        many: Many,
    },
    /// Create an issue, or up to 50 from a file
    Create(CreateArgs),
    /// Change an issue's fields
    Update(UpdateArgs),
    /// Delete issues
    Delete {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        /// Also delete their subtasks (Jira refuses otherwise)
        #[arg(long)]
        delete_subtasks: bool,
        #[command(flatten)]
        many: Many,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Move issues through their workflow
    Transition {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        /// Target status name, transition name or transition ID
        #[arg(long)]
        to: String,
        /// NAME=VALUE for a field the transition sets; repeatable
        #[arg(long)]
        field: Vec<String>,
        /// JSON object of field IDs to REST values
        #[arg(long)]
        fields_json: Option<String>,
        /// Comment to add with the transition (`-` reads stdin)
        #[arg(long)]
        comment: Option<String>,
        /// Format of --comment
        #[arg(long, value_enum, default_value_t)]
        body_format: InFormat,
        #[command(flatten)]
        many: Many,
    },
    /// Assign issues
    Assign {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        /// me, none (unassign), an account ID, or a name or email
        #[arg(long)]
        to: String,
        #[command(flatten)]
        many: Many,
    },
    /// Archive issues (Premium and Enterprise, admins)
    Archive {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Bring archived issues back
    Restore {
        /// Issue keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Comments on an issue
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },
    /// Time logged on an issue
    Worklog {
        #[command(subcommand)]
        cmd: WorklogCmd,
    },
    /// Files attached to an issue
    Attachment {
        #[command(subcommand)]
        cmd: AttachmentCmd,
    },
    /// Links between issues
    Link {
        #[command(subcommand)]
        cmd: LinkCmd,
    },
    /// Remote (web) links on an issue
    Weblink {
        #[command(subcommand)]
        cmd: WeblinkCmd,
    },
    /// Who watches an issue
    Watcher {
        #[command(subcommand)]
        cmd: WatcherCmd,
    },
    /// Votes on an issue
    Vote {
        #[command(subcommand)]
        cmd: VoteCmd,
    },
    /// Entity properties on an issue
    Property {
        #[command(subcommand)]
        cmd: PropertyCmd,
    },
    /// An issue's change history
    Changelog {
        #[command(subcommand)]
        cmd: ChangelogCmd,
    },
}

/// `--body-format` and `--body-offset` on commands that print bodies.
#[derive(Args, Debug)]
#[allow(dead_code)]
pub(crate) struct BodyOut {
    /// How bodies are printed: markdown, or adf (the native document)
    #[arg(long, value_enum, default_value_t)]
    body_format: OutFormat,
    /// Continue a cut body from this character (see `truncated` in the result)
    #[arg(long, value_name = "N")]
    body_offset: Option<usize>,
}

impl BodyOut {
    fn apply(&self) {
        print_bodies_as(self.body_format);
    }
}

#[derive(Args)]
pub(crate) struct ListArgs {
    /// JQL; filter flags are added to it with AND
    #[arg(long)]
    jql: Option<String>,
    /// Use a saved filter's JQL: ID, URL or exact name
    #[arg(long, conflicts_with = "jql")]
    filter: Option<String>,
    /// Project key
    #[arg(long)]
    project: Option<String>,
    /// Status name; repeat for any of several
    #[arg(long)]
    status: Vec<String>,
    /// me, none, or an account ID
    #[arg(long)]
    assignee: Option<String>,
    /// Issue type name; repeat for any of several
    #[arg(long = "type")]
    issue_type: Vec<String>,
    /// Label; repeat for any of several
    #[arg(long)]
    label: Vec<String>,
    /// JQL ORDER BY clause without the keywords, e.g. "priority DESC"
    #[arg(long)]
    order_by: Option<String>,
    /// The issues of this sprint (ID, ARI or URL)
    #[arg(long, conflicts_with_all = ["filter", "backlog"])]
    sprint: Option<String>,
    /// The backlog of this board (ID, URL or exact name)
    #[arg(long, conflicts_with = "filter")]
    backlog: Option<String>,
    /// A quick filter of the --backlog board to apply; repeatable
    #[arg(long, requires = "backlog")]
    quick_filter: Vec<String>,
    #[command(flatten)]
    paging: Paging,
}

/// Agile fields on `issue create` and `issue update`.
#[derive(Args)]
pub(crate) struct PlanningArgs {
    /// Sprint ID, ARI, URL or name (active and future sprints), or "none"
    /// for the backlog
    #[arg(long)]
    sprint: Option<String>,
    /// Story points, or "none"; the field comes from the project's boards
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
    fn planning(&self) -> agile::Planning<'_> {
        agile::Planning {
            sprint: self.sprint.as_deref(),
            story_points: self.story_points.as_deref(),
            original_estimate: self.original_estimate.as_deref(),
            remaining_estimate: self.remaining_estimate.as_deref(),
            team: self.team.as_deref(),
        }
    }
}

/// `--description`, `--description-file`, `--body-format`.
#[derive(Args)]
pub(crate) struct DescriptionArgs {
    /// Description (`-` reads stdin)
    #[arg(long, conflicts_with = "description_file")]
    description: Option<String>,
    /// Read the description from this file
    #[arg(long, value_name = "PATH")]
    description_file: Option<PathBuf>,
    /// Format of the description: markdown, adf or plain
    #[arg(long, value_enum, default_value_t)]
    body_format: InFormat,
}

#[derive(Args)]
pub(crate) struct CreateArgs {
    /// Project key or ID [default: jira.project in .atlx.toml]
    #[arg(long)]
    project: Option<String>,
    /// Issue type name or ID
    #[arg(long = "type", default_value = "Task")]
    issue_type: String,
    /// Summary (title)
    #[arg(long, required_unless_present = "bulk_file")]
    summary: Option<String>,
    #[command(flatten)]
    description: DescriptionArgs,
    /// me, an account ID, or a name or email
    #[arg(long)]
    assignee: Option<String>,
    /// me, an account ID, or a name or email
    #[arg(long)]
    reporter: Option<String>,
    /// Priority name or ID
    #[arg(long)]
    priority: Option<String>,
    /// Label; repeatable
    #[arg(long)]
    label: Vec<String>,
    /// Parent issue key
    #[arg(long)]
    parent: Option<String>,
    /// NAME=VALUE by field ID or display name (value parsed as JSON when
    /// valid); repeatable. `atlx jira field list --project P --type T` lists them
    #[arg(long)]
    field: Vec<String>,
    /// JSON object of field IDs to REST values
    #[arg(long)]
    fields_json: Option<String>,
    #[command(flatten)]
    planning: PlanningArgs,
    /// Create up to 50 issues from a JSON file: an array of {"fields": {...}}
    /// or an {"issueUpdates": [...]} object (`-` reads stdin)
    #[arg(long, value_name = "PATH", conflicts_with_all = ["summary", "description", "description_file", "assignee", "reporter", "priority", "label", "parent", "field", "fields_json", "sprint", "story_points", "original_estimate", "remaining_estimate", "team"])]
    bulk_file: Option<String>,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// Issue key, ID or URL
    key: String,
    /// New summary
    #[arg(long)]
    summary: Option<String>,
    #[command(flatten)]
    description: DescriptionArgs,
    /// Replace a description that has content Markdown cannot express
    /// (destroys it; needs --yes)
    #[arg(long)]
    allow_lossy: bool,
    /// Confirm --allow-lossy
    #[arg(long, short, requires = "allow_lossy")]
    yes: bool,
    /// me, none, an account ID, or a name or email
    #[arg(long)]
    assignee: Option<String>,
    /// Issue type name or ID
    #[arg(long = "type")]
    issue_type: Option<String>,
    /// Priority name or ID
    #[arg(long)]
    priority: Option<String>,
    /// Replace the labels; repeatable
    #[arg(long, conflicts_with_all = ["add_label", "remove_label"])]
    label: Vec<String>,
    /// Add a label; repeatable
    #[arg(long)]
    add_label: Vec<String>,
    /// Remove a label; repeatable
    #[arg(long)]
    remove_label: Vec<String>,
    /// Parent key, or none
    #[arg(long)]
    parent: Option<String>,
    /// Due date YYYY-MM-DD, or none
    #[arg(long)]
    due: Option<String>,
    /// Replace the fix versions; repeatable
    #[arg(long, conflicts_with_all = ["add_fix_version", "remove_fix_version"])]
    fix_version: Vec<String>,
    /// Add a fix version; repeatable
    #[arg(long)]
    add_fix_version: Vec<String>,
    /// Remove a fix version; repeatable
    #[arg(long)]
    remove_fix_version: Vec<String>,
    /// Replace the components; repeatable
    #[arg(long, conflicts_with_all = ["add_component", "remove_component"])]
    component: Vec<String>,
    /// Add a component; repeatable
    #[arg(long)]
    add_component: Vec<String>,
    /// Remove a component; repeatable
    #[arg(long)]
    remove_component: Vec<String>,
    /// Resolution name or ID, or none
    #[arg(long)]
    resolution: Option<String>,
    /// NAME=VALUE by field ID or display name; repeatable
    #[arg(long)]
    field: Vec<String>,
    /// JSON object of field IDs to REST values
    #[arg(long)]
    fields_json: Option<String>,
    #[command(flatten)]
    planning: PlanningArgs,
}

/// The issue a sub-resource belongs to.
#[derive(Args, Debug)]
pub(crate) struct IssueArg {
    /// Issue key, ID or URL
    #[arg(long)]
    issue: String,
}

impl IssueArg {
    fn key(&self) -> String {
        jira::issue_ref(&self.issue)
    }
}

#[derive(Subcommand)]
pub(crate) enum CommentCmd {
    /// An issue's comments, oldest first
    List {
        #[command(flatten)]
        issue: IssueArg,
        /// Newest first
        #[arg(long)]
        newest_first: bool,
        #[command(flatten)]
        body: BodyOut,
        #[command(flatten)]
        paging: Paging,
    },
    /// Comments by ID
    Get {
        /// Comment IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        body: BodyOut,
        #[command(flatten)]
        many: Many,
    },
    /// Add a comment
    Create {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        text: BodyIn,
        /// JSM: an internal note, hidden from customers
        #[arg(long)]
        internal: bool,
        /// Only this project role sees it
        #[arg(long, conflicts_with = "visibility_group")]
        visibility_role: Option<String>,
        /// Only this group sees it
        #[arg(long)]
        visibility_group: Option<String>,
        /// Reply to this comment ID
        #[arg(long)]
        parent: Option<String>,
    },
    /// Replace a comment's body
    Update {
        /// Comment ID
        id: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        text: BodyIn,
        /// Replace a body that has content Markdown cannot express (destroys
        /// it; needs --yes)
        #[arg(long)]
        allow_lossy: bool,
        /// Confirm --allow-lossy
        #[arg(long, short, requires = "allow_lossy")]
        yes: bool,
    },
    /// Delete a comment
    Delete {
        /// Comment ID
        id: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

/// `--body`, `--body-file`, `--body-format` (one of the first two required).
#[derive(Args, Debug)]
pub(crate) struct BodyIn {
    /// Body text (`-` reads stdin)
    #[arg(
        long,
        required_unless_present = "body_file",
        conflicts_with = "body_file"
    )]
    body: Option<String>,
    /// Read the body from this file
    #[arg(long, value_name = "PATH")]
    body_file: Option<PathBuf>,
    /// Format of the body: markdown, adf or plain
    #[arg(long, value_enum, default_value_t)]
    body_format: InFormat,
}

impl BodyIn {
    fn adf(self) -> Result<Value> {
        adf_input(self.body, self.body_file, self.body_format)?
            .ok_or_else(|| ApiError::usage("pass --body or --body-file").into())
    }
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum WorklogCmd {
    /// An issue's worklogs; or, site-wide, the worklogs changed or deleted
    /// since a time
    List {
        /// Issue key, ID or URL
        #[arg(long, required_unless_present_any = ["changed_since", "deleted_since"])]
        issue: Option<String>,
        /// Only work started after this (epoch ms, date, timestamp or 7d)
        #[arg(long, requires = "issue")]
        started_after: Option<String>,
        /// Only work started before this
        #[arg(long, requires = "issue")]
        started_before: Option<String>,
        /// Site-wide: worklogs changed since this time
        #[arg(long, conflicts_with_all = ["issue", "deleted_since"])]
        changed_since: Option<String>,
        /// Site-wide: worklogs deleted since this time
        #[arg(long, conflicts_with = "issue")]
        deleted_since: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Worklogs by ID
    Get {
        /// Worklog IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        many: Many,
    },
    /// Log time on an issue
    Create {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        entry: EntryArgs,
    },
    /// Change a worklog
    Update {
        /// Worklog ID
        id: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        entry: EntryArgs,
    },
    /// Delete a worklog
    Delete {
        /// Worklog ID
        id: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        options: WriteArgs,
        /// Put this much back on the estimate (with manual adjustment)
        #[arg(long)]
        increase_by: Option<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
}

/// Flags shared by every worklog write.
#[derive(Args)]
pub(crate) struct WriteArgs {
    /// How the remaining estimate changes
    #[arg(long, value_enum)]
    adjust_estimate: Option<worklogs::Adjust>,
    /// The remaining estimate to set (with --adjust-estimate new)
    #[arg(long)]
    new_estimate: Option<String>,
    /// Notify watchers (true or false)
    #[arg(long, value_name = "BOOL")]
    notify_users: Option<bool>,
    /// Write even if the issue isn't editable (admins)
    #[arg(long)]
    override_editable: bool,
}

#[derive(Args)]
pub(crate) struct EntryArgs {
    /// Time spent, e.g. 2h or "1d 4h"
    #[arg(long, conflicts_with = "time_spent_seconds")]
    time_spent: Option<String>,
    /// Time spent, in seconds
    #[arg(long)]
    time_spent_seconds: Option<u64>,
    /// When the work started: timestamp, date (UTC), epoch ms, or an age
    /// like 3h
    #[arg(long)]
    started: Option<String>,
    /// What was done (`-` reads stdin)
    #[arg(long)]
    comment: Option<String>,
    /// Format of --comment
    #[arg(long, value_enum, default_value_t)]
    body_format: InFormat,
    /// REST visibility object
    #[arg(long, value_name = "JSON")]
    visibility_json: Option<String>,
    #[command(flatten)]
    options: WriteArgs,
    /// Take this much off the estimate (with manual adjustment)
    #[arg(long)]
    reduce_by: Option<String>,
}

impl WriteArgs {
    fn opts(&self) -> worklogs::WriteOpts<'_> {
        worklogs::WriteOpts {
            adjust: self.adjust_estimate,
            new_estimate: self.new_estimate.as_deref(),
            notify_users: self.notify_users,
            override_editable: self.override_editable,
            ..Default::default()
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum AttachmentCmd {
    /// The files attached to an issue
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Attachments' metadata by ID
    Get {
        /// Attachment IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Save an attachment (or its thumbnail) to a local file
    Download {
        /// Attachment ID
        id: String,
        /// File to write [default: the attachment's name, here]
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
        /// Replace the file if it exists
        #[arg(long)]
        force: bool,
        /// Save the thumbnail instead
        #[arg(long)]
        thumbnail: bool,
    },
    /// Attach local files to an issue
    Upload {
        #[command(flatten)]
        issue: IssueArg,
        /// File to attach; repeatable
        #[arg(long, required = true)]
        file: Vec<PathBuf>,
    },
    /// Delete an attachment
    Delete {
        /// Attachment ID
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum LinkCmd {
    /// An issue's links to other issues
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Link an issue to others: `--issue P-1 --target P-2 --type Blocks`
    /// means P-1 blocks P-2
    Create {
        #[command(flatten)]
        issue: IssueArg,
        /// Issue it points at; repeatable
        #[arg(long, required = true)]
        target: Vec<String>,
        /// Link type name or ID (`atlx jira link-type list`)
        #[arg(long = "type")]
        link_type: String,
    },
    /// Delete a link (its ID is in `issue link list`)
    Delete {
        /// Link ID
        id: String,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum WeblinkCmd {
    /// An issue's remote links
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add a remote link
    Create {
        #[command(flatten)]
        issue: IssueArg,
        /// Target URL
        #[arg(long)]
        url: String,
        /// Link text
        #[arg(long)]
        title: String,
        /// Stable ID; a second link with the same ID replaces the first
        #[arg(long)]
        global_id: Option<String>,
        /// How it relates, e.g. "mentioned in"
        #[arg(long)]
        relationship: Option<String>,
        /// Application name
        #[arg(long)]
        application_name: Option<String>,
        /// Application type
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
    /// Delete a remote link by ID, global ID or URL
    #[command(group(clap::ArgGroup::new("which").required(true).args(["id", "global_id", "url"])))]
    Delete {
        /// Remote link ID
        id: Option<String>,
        #[command(flatten)]
        issue: IssueArg,
        /// The link's global ID
        #[arg(long)]
        global_id: Option<String>,
        /// The linked URL; the one link pointing there is deleted
        #[arg(long)]
        url: Option<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum WatcherCmd {
    /// Who watches an issue
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add a watcher
    Create {
        /// me, an account ID, or a name or email
        #[arg(default_value = "me")]
        user: String,
        #[command(flatten)]
        issue: IssueArg,
    },
    /// Remove a watcher
    Delete {
        /// me, an account ID, or a name or email
        #[arg(default_value = "me")]
        user: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum VoteCmd {
    /// The votes on an issue
    Get {
        #[command(flatten)]
        issue: IssueArg,
    },
    /// Vote for an issue as yourself
    Create {
        #[command(flatten)]
        issue: IssueArg,
    },
    /// Take your vote back
    Delete {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum PropertyCmd {
    /// The property keys on an issue
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// One property's value
    Get {
        /// Property key
        key: String,
        #[command(flatten)]
        issue: IssueArg,
    },
    /// Create or replace a property
    Update {
        /// Property key
        key: String,
        #[command(flatten)]
        issue: IssueArg,
        /// The value as JSON (`-` reads stdin)
        #[arg(long, allow_hyphen_values = true)]
        value: String,
    },
    /// Delete a property
    Delete {
        /// Property key
        key: String,
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChangelogCmd {
    /// An issue's change history, oldest first
    List {
        #[command(flatten)]
        issue: IssueArg,
        #[command(flatten)]
        paging: Paging,
    },
}

fn list_edit(set: Vec<String>, add: Vec<String>, remove: Vec<String>) -> Option<jira::ListEdit> {
    if set.is_empty() && add.is_empty() && remove.is_empty() {
        return None;
    }
    Some(jira::ListEdit {
        set: (!set.is_empty()).then_some(set),
        add,
        remove,
    })
}

fn millis(value: Option<&str>) -> Result<Option<i64>> {
    value.map(epoch_millis).transpose()
}

/// `--field` and `--fields-json`, with display names resolved to IDs.
fn field_map(c: &Client, json: Option<&str>, field: &[String]) -> Result<Map<String, Value>> {
    let raw = jira::extra_fields(json, field)?;
    if raw.is_empty() {
        return Ok(raw);
    }
    jira::resolve_field_names(c, &Cache::global(), raw)
}

/// Issues by key: one with `GET`, several with `bulkfetch`. Inputs Jira did
/// not return go to the envelope's `errors`.
/// Up to this many issues are read one by one (each failure keeps its own
/// status); more go through `bulkfetch`, 100 per request.
const BULK_FROM: usize = 21;

fn get(c: &Client, keys: Vec<String>, many: &Many) -> Result<Value> {
    let keys = issue_keys(keys)?;
    if keys.len() < BULK_FROM {
        return many.run(keys, |k| jira::issue(c, k));
    }
    let (found, missing) = jira::issues(c, &keys)?;
    let failed: Vec<(String, ApiError)> = missing
        .into_iter()
        .map(|k| {
            let e = ApiError::new(
                crate::client::ErrorKind::NotFound,
                format!("{k} does not exist or you cannot see it"),
            );
            (k, e)
        })
        .collect();
    if !failed.is_empty() && many.strict {
        let (k, mut e) = failed[0].clone();
        e.message = format!(
            "{} of {} inputs failed; first: {k}: {}",
            failed.len(),
            keys.len(),
            e.message
        );
        return Err(e.into());
    }
    outcome((found, failed))
}

fn list(c: &Client, a: ListArgs) -> Result<Value> {
    let saved = a
        .filter
        .as_deref()
        .map(|f| jira::admin::filter_jql(c, f))
        .transpose()?;
    let filter = jira::IssueFilter {
        jql: saved.as_deref().or(a.jql.as_deref()),
        project: a.project.as_deref(),
        status: &a.status,
        assignee: a.assignee.as_deref(),
        issue_type: &a.issue_type,
        label: &a.label,
        order_by: a.order_by.as_deref(),
    };
    let (limit, cursor) = (a.paging.limit(), a.paging.cursor());
    if let Some(s) = &a.sprint {
        let jql = filter.jql();
        return Ok(listed(agile::sprint_issues(
            c,
            jira::sprint_ref(s)?,
            jql.as_deref(),
            limit,
            cursor,
        )?));
    }
    if let Some(b) = &a.backlog {
        let board = jira::resolve_board(c, b)?;
        let jql = filter.jql();
        let quick: Vec<String> = a.quick_filter.iter().map(|q| jira::entity_id(q)).collect();
        return Ok(listed(agile::backlog(
            c,
            board,
            &quick,
            jql.as_deref(),
            limit,
            cursor,
        )?));
    }
    let Some(jql) = filter.jql() else {
        return Err(ApiError::usage(
            "pass --jql, --filter, or a filter flag such as --project or --assignee",
        )
        .with_suggestions([
            "atlx jira issue list --assignee me --status 'In Progress'".to_string(),
            "atlx jira issue list --jql 'project = PROJ ORDER BY updated DESC'".to_string(),
        ])
        .into());
    };
    Ok(listed(jira::issue_list(c, &jql, limit, cursor)?))
}

fn create(c: &Client, a: CreateArgs) -> Result<Value> {
    if let Some(file) = a.bulk_file {
        let text = if file == "-" {
            text_arg(file)?
        } else {
            std::fs::read_to_string(&file)
                .map_err(|e| ApiError::usage(format!("reading {file}: {e}")))?
        };
        let items = bulk::create_input(jira::json_value("--bulk-file", &text)?)?;
        return outcome(bulk::create(c, &items)?);
    }
    let project = a
        .project
        .or_else(default_project)
        .map(|p| jira::project_ref(&p))
        .ok_or_else(|| ApiError::usage("pass --project (or set jira.project in .atlx.toml)"))?;
    let d = a.description;
    let description = adf_input(d.description, d.description_file, d.body_format)?;
    let mut extra = field_map(c, a.fields_json.as_deref(), &a.field)?;
    let planned = agile::plan(c, agile::On::Project(&project), &a.planning.planning())?;
    extra.extend(planned.fields);
    let summary = a.summary.unwrap_or_default();
    let fields = jira::new_issue_fields(
        c,
        jira::NewIssue {
            project: &project,
            issue_type: &a.issue_type,
            summary: &summary,
            description,
            assignee: a.assignee.as_deref(),
            reporter: a.reporter.as_deref(),
            priority: a.priority.as_deref(),
            labels: a.label,
            parent: a.parent.as_deref(),
            extra,
        },
    )?;
    let mut data = jira::create(c, fields).map_err(|e| {
        if crate::client::DryRun::find(&e).is_some() {
            return e;
        }
        let mut err = ApiError::from_anyhow(&e);
        if err.status == Some(400) && err.hint.is_none() {
            err.hint = Some(format!(
                "list the fields this issue type takes with `atlx jira field list --project {project} --type '{}'`",
                a.issue_type
            ));
        }
        anyhow::Error::from(err)
    })?;
    // A new issue is already in the backlog, so only sprints need a move.
    if let Some(to @ agile::Destination::Sprint(id)) = planned.sprint {
        let key = data["key"].as_str().unwrap_or_default().to_string();
        // The issue exists by now: say so, or a retry makes a second one.
        agile::move_issues(c, to, std::slice::from_ref(&key)).map_err(|e| {
            if crate::client::DryRun::find(&e).is_some() {
                return e;
            }
            let mut err = ApiError::from_anyhow(&e);
            err.message = format!(
                "created {key}, but could not add it to sprint {id} (do not create it again): {}",
                err.message
            );
            anyhow::Error::from(err)
        })?;
        data["sprint"] = id.to_string().into();
    }
    Ok(data)
}

fn update(c: &Client, u: UpdateArgs) -> Result<Value> {
    let key = jira::issue_ref(&u.key);
    let d = u.description;
    let description = adf_input(d.description, d.description_file, d.body_format)?;
    if u.allow_lossy && !u.yes && !crate::cli::dry_run_requested() {
        return Err(util::needs_yes("`--allow-lossy`"));
    }
    if description.is_some() {
        let current = jira::current_description(c, &key)?;
        jira::guard_replace(
            &current,
            d.body_format.into(),
            &format!("the description of {key}"),
            u.allow_lossy,
        )?;
    }
    let mut extra = field_map(c, u.fields_json.as_deref(), &u.field)?;
    let planned = agile::plan(c, agile::On::Issue(&key), &u.planning.planning())?;
    extra.extend(planned.fields);
    jira::update(
        c,
        &key,
        jira::Update {
            summary: u.summary.as_deref(),
            description,
            assignee: u.assignee.as_deref(),
            issue_type: u.issue_type.as_deref(),
            priority: u.priority.as_deref(),
            labels: list_edit(u.label, u.add_label, u.remove_label),
            components: list_edit(u.component, u.add_component, u.remove_component),
            fix_versions: list_edit(u.fix_version, u.add_fix_version, u.remove_fix_version),
            parent: u.parent.as_deref(),
            due_date: u.due.as_deref(),
            resolution: u.resolution.as_deref(),
            extra,
            sprint: planned.sprint,
        },
    )
}

pub(super) fn run(c: &Client, cmd: IssueCmd) -> Result<Value> {
    match cmd {
        IssueCmd::List(a) => list(c, a),
        IssueCmd::Search {
            text,
            project,
            paging,
        } => {
            let jql = jira::search_jql(&text.join(" "), project.as_deref());
            Ok(listed(jira::issue_list(
                c,
                &jql,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        IssueCmd::Get { keys, body, many } => {
            body.apply();
            get(c, keys, &many)
        }
        IssueCmd::Create(a) => create(c, a),
        IssueCmd::Update(u) => update(c, u),
        IssueCmd::Delete {
            keys,
            delete_subtasks,
            many,
            confirm: _,
        } => many.run(issue_keys(keys)?, |k| jira::delete(c, k, delete_subtasks)),
        IssueCmd::Transition {
            keys,
            to,
            field,
            fields_json,
            comment,
            body_format,
            many,
        } => {
            let fields = field_map(c, fields_json.as_deref(), &field)?;
            let comment = adf_input(comment, None, body_format)?;
            many.run(issue_keys(keys)?, |k| {
                jira::transition(c, k, &to, fields.clone(), comment.clone())
            })
        }
        IssueCmd::Assign { keys, to, many } => {
            let who = match to.trim().to_ascii_lowercase().as_str() {
                "none" | "unassigned" => None,
                _ => Some(jira::resolve_account(c, &to)?),
            };
            many.run(issue_keys(keys)?, |k| jira::assign(c, k, who.as_deref()))
        }
        IssueCmd::Archive { keys, confirm: _ } => {
            outcome(bulk::archive(c, &issue_keys(keys)?, true)?)
        }
        IssueCmd::Restore { keys, confirm: _ } => {
            outcome(bulk::archive(c, &issue_keys(keys)?, false)?)
        }
        IssueCmd::Comment { cmd } => comment(c, cmd),
        IssueCmd::Worklog { cmd } => worklog(c, cmd),
        IssueCmd::Attachment { cmd } => attachment(c, cmd),
        IssueCmd::Link { cmd } => link(c, cmd),
        IssueCmd::Weblink { cmd } => weblink(c, cmd),
        IssueCmd::Watcher { cmd } => match cmd {
            WatcherCmd::List { issue, paging } => paging.local(people::watchers(c, &issue.key())?),
            WatcherCmd::Create { user, issue } => people::watcher_add(c, &issue.key(), &user),
            WatcherCmd::Delete {
                user,
                issue,
                confirm: _,
            } => people::watcher_remove(c, &issue.key(), &user),
        },
        IssueCmd::Vote { cmd } => match cmd {
            VoteCmd::Get { issue } => people::votes(c, &issue.key()),
            VoteCmd::Create { issue } => people::vote(c, &issue.key()),
            VoteCmd::Delete { issue, confirm: _ } => people::unvote(c, &issue.key()),
        },
        IssueCmd::Property { cmd } => match cmd {
            PropertyCmd::List { issue, paging } => paging.local(bulk::properties(c, &issue.key())?),
            PropertyCmd::Get { key, issue } => bulk::property(c, &issue.key(), &key),
            PropertyCmd::Update { key, issue, value } => {
                let value = jira::json_value("--value", &text_arg(value)?)?;
                bulk::property_set(c, &issue.key(), &key, &value)
            }
            PropertyCmd::Delete {
                key,
                issue,
                confirm: _,
            } => bulk::property_delete(c, &issue.key(), &key),
        },
        IssueCmd::Changelog {
            cmd: ChangelogCmd::List { issue, paging },
        } => Ok(listed(jira::changelog(
            c,
            &issue.key(),
            paging.limit(),
            paging.cursor(),
        )?)),
    }
}

fn comment(c: &Client, cmd: CommentCmd) -> Result<Value> {
    match cmd {
        CommentCmd::List {
            issue,
            newest_first,
            body,
            paging,
        } => {
            body.apply();
            Ok(listed(jira::comments(
                c,
                &issue.key(),
                newest_first,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        CommentCmd::Get {
            ids,
            issue,
            body,
            many,
        } => {
            body.apply();
            let key = issue.key();
            many.run(ids, |id| jira::comment_get(c, &key, &jira::entity_id(id)))
        }
        CommentCmd::Create {
            issue,
            text,
            internal,
            visibility_role,
            visibility_group,
            parent,
        } => {
            let visibility = match (&visibility_role, &visibility_group) {
                (Some(r), _) => Some(jira::Visibility::Role(r)),
                (None, Some(g)) => Some(jira::Visibility::Group(g)),
                _ => None,
            };
            let parent = parent.as_deref().map(jira::entity_id);
            jira::comment(
                c,
                &issue.key(),
                text.adf()?,
                visibility,
                internal,
                parent.as_deref(),
            )
        }
        CommentCmd::Update {
            id,
            issue,
            text,
            allow_lossy,
            yes,
        } => {
            let (key, id) = (issue.key(), jira::entity_id(&id));
            if allow_lossy && !yes && !crate::cli::dry_run_requested() {
                return Err(util::needs_yes("`--allow-lossy`"));
            }
            let format = text.body_format.into();
            let body = text.adf()?;
            let current = jira::comment_get(c, &key, &id)?;
            jira::guard_replace(
                &current["body"],
                format,
                &format!("comment {id}"),
                allow_lossy,
            )?;
            jira::comment_update(c, &key, &id, body)
        }
        CommentCmd::Delete {
            id,
            issue,
            confirm: _,
        } => jira::comment_delete(c, &issue.key(), &jira::entity_id(&id)),
    }
}

fn worklog(c: &Client, cmd: WorklogCmd) -> Result<Value> {
    match cmd {
        WorklogCmd::List {
            issue,
            started_after,
            started_before,
            changed_since,
            deleted_since,
            paging,
        } => {
            let (limit, cursor) = (paging.limit(), paging.cursor());
            let feed = match (&changed_since, &deleted_since) {
                (Some(s), _) => Some((worklogs::Feed::Changed, s)),
                (None, Some(s)) => Some((worklogs::Feed::Deleted, s)),
                _ => None,
            };
            if let Some((kind, since)) = feed {
                return Ok(listed(worklogs::feed(
                    c,
                    kind,
                    epoch_millis(since)?,
                    limit,
                    cursor,
                )?));
            }
            let key = jira::issue_ref(issue.as_deref().unwrap_or_default());
            Ok(listed(worklogs::list(
                c,
                &key,
                millis(started_after.as_deref())?,
                millis(started_before.as_deref())?,
                limit,
                cursor,
            )?))
        }
        WorklogCmd::Get { ids, issue, many } => {
            let key = issue.key();
            many.run(ids, |id| worklogs::get(c, &key, &jira::entity_id(id), None))
        }
        WorklogCmd::Create { issue, entry } => write_worklog(c, &issue.key(), None, entry),
        WorklogCmd::Update { id, issue, entry } => {
            write_worklog(c, &issue.key(), Some(&jira::entity_id(&id)), entry)
        }
        WorklogCmd::Delete {
            id,
            issue,
            options,
            increase_by,
            confirm: _,
        } => {
            let opts = worklogs::WriteOpts {
                increase_by: increase_by.as_deref(),
                ..options.opts()
            };
            worklogs::delete(c, &issue.key(), &jira::entity_id(&id), &opts)
        }
    }
}

fn write_worklog(c: &Client, key: &str, id: Option<&str>, a: EntryArgs) -> Result<Value> {
    let comment = adf_input(a.comment.clone(), None, a.body_format)?;
    let visibility = a
        .visibility_json
        .as_deref()
        .map(|v| jira::json_value("--visibility-json", v))
        .transpose()?;
    let entry = worklogs::Entry {
        time_spent: a.time_spent.as_deref(),
        seconds: a.time_spent_seconds,
        started: millis(a.started.as_deref())?,
        comment,
        visibility,
        properties: None,
    };
    let opts = worklogs::WriteOpts {
        reduce_by: a.reduce_by.as_deref(),
        ..a.options.opts()
    };
    match id {
        Some(id) => worklogs::update(c, key, id, &entry, &opts),
        None => worklogs::add(c, key, &entry, &opts),
    }
}

fn attachment(c: &Client, cmd: AttachmentCmd) -> Result<Value> {
    match cmd {
        AttachmentCmd::List { issue, paging } => paging.local(attachments::list(c, &issue.key())?),
        AttachmentCmd::Get { ids, many } => {
            many.run(ids, |id| attachments::get(c, &jira::entity_id(id)))
        }
        AttachmentCmd::Download {
            id,
            path,
            force,
            thumbnail,
        } => {
            let what = if thumbnail {
                attachments::Bytes::Thumbnail
            } else {
                attachments::Bytes::Content
            };
            attachments::download(c, &jira::entity_id(&id), what, path.as_deref(), force)
        }
        AttachmentCmd::Upload { issue, file } => {
            Ok(Value::Array(attachments::upload(c, &issue.key(), &file)?))
        }
        AttachmentCmd::Delete { id, confirm: _ } => attachments::delete(c, &jira::entity_id(&id)),
    }
}

fn link(c: &Client, cmd: LinkCmd) -> Result<Value> {
    match cmd {
        LinkCmd::List { issue, paging } => paging.local(jira::links(c, &issue.key())?),
        LinkCmd::Create {
            issue,
            target,
            link_type,
        } => {
            let targets: Vec<String> = target.iter().map(|t| jira::issue_ref(t)).collect();
            Ok(Value::Array(jira::link_create(
                c,
                &issue.key(),
                &targets,
                &link_type,
                None,
            )?))
        }
        LinkCmd::Delete { id, confirm: _ } => jira::link_delete(c, &jira::entity_id(&id)),
    }
}

fn weblink(c: &Client, cmd: WeblinkCmd) -> Result<Value> {
    match cmd {
        WeblinkCmd::List { issue, paging } => paging.local(jira::weblinks(c, &issue.key())?),
        WeblinkCmd::Create {
            issue,
            url,
            title,
            global_id,
            relationship,
            application_name,
            application_type,
            summary,
            icon_url,
            resolved,
        } => jira::weblink_create(
            c,
            &issue.key(),
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
        WeblinkCmd::Delete {
            id,
            issue,
            global_id,
            url,
            confirm: _,
        } => {
            let which = match (&id, &global_id, &url) {
                (Some(l), _, _) => jira::WeblinkRef::Id(l),
                (None, Some(g), _) => jira::WeblinkRef::GlobalId(g),
                (None, None, Some(u)) => jira::WeblinkRef::Url(u),
                (None, None, None) => {
                    return Err(not_found("pass a link ID, --global-id or --url".into()));
                }
            };
            jira::weblink_delete(c, &issue.key(), which)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_edits() {
        assert!(list_edit(vec![], vec![], vec![]).is_none());
        let e = list_edit(vec![], vec!["a".into()], vec![]).unwrap();
        assert!(e.set.is_none());
        assert_eq!(e.add, ["a"]);
    }
}

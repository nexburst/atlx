//! `atlx jira`: clap definitions and dispatch.

use super::util::{batch, text_arg};
use crate::client;
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
    /// Jira projects
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Jira Agile boards
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
    /// List Jira projects
    List {
        /// Filter by project key or name
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Get a Jira project by key or ID
    Get {
        /// Project key or ID
        identifier: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum BoardCmd {
    /// List Agile boards
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
pub(crate) enum SprintCmd {
    /// List sprints on a board
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
    /// List issues in a sprint
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
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        first: Option<usize>,
        #[arg(long)]
        after: Option<String>,
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
    /// Available issue types and fields for creating a work item
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
    /// List available issue link types
    Types,
    /// Link two work items
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
    /// Add a remote web link to a work item
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
        JiraCmd::Project {
            cmd: ProjectCmd::List { query, limit },
        } => jira::projects(c, query.as_deref(), limit),
        JiraCmd::Project {
            cmd: ProjectCmd::Get { identifier },
        } => jira::project(c, &identifier),
        JiraCmd::Board {
            cmd:
                BoardCmd::List {
                    project,
                    board_type,
                    limit,
                },
        } => jira::boards(c, project.as_deref(), board_type, limit),
        JiraCmd::Sprint {
            cmd:
                SprintCmd::List {
                    board,
                    state,
                    limit,
                },
        } => jira::sprints(c, board, state.as_deref(), limit),
        JiraCmd::Sprint {
            cmd: SprintCmd::Issues { sprint, jql, limit },
        } => jira::sprint_issues(c, sprint, jql.as_deref(), limit),
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
            batch(inputs, strict, |k| jira::get(c, k, &opts))
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
            fields,
        } => {
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
        WorkitemCmd::CreateMeta { space, issue_type } => {
            jira::create_meta(c, &space, issue_type.as_deref())
        }
        WorkitemCmd::Link { cmd } => match cmd {
            LinkCmd::Types => jira::link_types(c),
            LinkCmd::Create {
                inward,
                outward,
                link_type,
                comment,
                comment_format,
            } => {
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
            } => jira::link_remote(c, &issue_ref(&id), &url, &title, relationship.as_deref()),
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

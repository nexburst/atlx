//! The records Jira commands return: what each field is called, what it
//! means, and where it comes from in the REST payload. API functions return
//! REST objects as Jira sends them; the command line projects them onto
//! these records (`--full`, `--fields`, `--raw` choose how).
//!
//! Rich-text fields (issue descriptions, comment and worklog bodies) are
//! read as ADF and printed as Markdown, unless `--body-format adf` asks for
//! the native document. A body Markdown cannot fully show adds a
//! `lossy_body` warning naming the issue or comment it came from.

use crate::markup::{BodyFormat, render_markdown};
use crate::output::notes;
use crate::schema::truncate::BODY;
use crate::schema::{Field, Record, Source, path};
use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicU8, Ordering};

static FORMAT: AtomicU8 = AtomicU8::new(0);

/// How rich-text fields are printed for this run: Markdown (the default) or
/// ADF. Plain and storage print as Markdown.
pub fn set_body_format(f: BodyFormat) {
    FORMAT.store(u8::from(f == BodyFormat::Adf), Ordering::SeqCst);
}

fn adf_output() -> bool {
    FORMAT.load(Ordering::SeqCst) == 1
}

/// A rich-text value as printed: Markdown with a `lossy_body` warning when
/// something could not be shown, or the ADF itself.
pub fn body_value(adf: &Value, owner: &str) -> Value {
    match adf {
        Value::Null => Value::Null,
        Value::String(s) => s.clone().into(),
        _ if adf_output() => adf.clone(),
        _ => {
            let r = render_markdown(adf);
            if let Some(mut w) = r.warning() {
                w["at"] = owner.into();
                notes::warn_value(w);
            }
            r.text.trim_end().to_string().into()
        }
    }
}

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// The site root of a REST object's `self` link.
fn site_of(v: &Value) -> Option<String> {
    let s = v["self"].as_str()?;
    s.find("/rest/").map(|i| s[..i].to_string())
}

fn issue_url(v: &Value) -> Value {
    match (site_of(v), v["key"].as_str()) {
        (Some(site), Some(key)) => format!("{site}/browse/{key}").into(),
        _ => Value::Null,
    }
}

fn description(v: &Value) -> Value {
    body_value(&v["fields"]["description"], &str_of(&v["key"]))
}

fn names_of(items: &Value, key: &str) -> Value {
    match items.as_array() {
        Some(a) => a.iter().map(|i| i[key].clone()).collect(),
        None => Value::Null,
    }
}

fn components(v: &Value) -> Value {
    names_of(&v["fields"]["components"], "name")
}

fn fix_versions(v: &Value) -> Value {
    names_of(&v["fields"]["fixVersions"], "name")
}

fn subtasks(v: &Value) -> Value {
    names_of(&v["fields"]["subtasks"], "key")
}

fn user_of(u: &Value) -> Value {
    if u.is_null() {
        return Value::Null;
    }
    json!({"id": u["accountId"], "name": u["displayName"]})
}

/// An issue link from the point of view of the issue it is listed on.
pub fn link_row(l: &Value) -> Value {
    let (direction, other, phrase) = if l["outwardIssue"].is_object() {
        ("outward", &l["outwardIssue"], &l["type"]["outward"])
    } else {
        ("inward", &l["inwardIssue"], &l["type"]["inward"])
    };
    json!({
        "id": l["id"],
        "type": l["type"]["name"],
        "direction": direction,
        "relation": phrase,
        "issue": other["key"],
        "title": other["fields"]["summary"],
        "status": other["fields"]["status"]["name"],
    })
}

fn links(v: &Value) -> Value {
    match v["fields"]["issuelinks"].as_array() {
        Some(a) => a.iter().map(link_row).collect(),
        None => Value::Null,
    }
}

fn comments(v: &Value) -> Value {
    let key = str_of(&v["key"]);
    match v["fields"]["comment"]["comments"].as_array() {
        Some(a) => a
            .iter()
            .map(|c| {
                json!({
                    "id": c["id"],
                    "author": user_of(&c["author"]),
                    "created": c["created"],
                    "body": body_value(&c["body"], &format!("{key} comment {}", str_of(&c["id"]))),
                })
            })
            .collect(),
        None => Value::Null,
    }
}

fn attachments(v: &Value) -> Value {
    match v["fields"]["attachment"].as_array() {
        Some(a) => a
            .iter()
            .map(|x| json!({"id": x["id"], "filename": x["filename"], "size": x["size"], "mimeType": x["mimeType"]}))
            .collect(),
        None => Value::Null,
    }
}

/// A custom field value in its simplest readable form: option values,
/// names, sprint names, account names, numbers and strings as they are.
fn simple(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.iter().map(simple).collect(),
        Value::Object(m) => {
            if m.get("type").and_then(Value::as_str) == Some("doc") {
                return body_value(v, "custom field");
            }
            for k in ["value", "name", "displayName", "key"] {
                if let Some(x) = m.get(k).filter(|x| !x.is_null()) {
                    return x.clone();
                }
            }
            v.clone()
        }
        _ => v.clone(),
    }
}

/// Custom fields that have a value, under their display names (from the
/// `names` Jira adds with `expand=names`).
fn custom(v: &Value) -> Value {
    let Some(fields) = v["fields"].as_object() else {
        return Value::Null;
    };
    let out: Map<String, Value> = fields
        .iter()
        .filter(|(k, x)| k.starts_with("customfield_") && !x.is_null())
        .map(|(k, x)| {
            let name = v["names"][k.as_str()]
                .as_str()
                .map_or_else(|| k.clone(), String::from);
            (name, simple(x))
        })
        .collect();
    Value::Object(out)
}

const ASSIGNEE: Source = Source::Object(&[
    ("id", "fields.assignee.accountId"),
    ("name", "fields.assignee.displayName"),
]);
const REPORTER: Source = Source::Object(&[
    ("id", "fields.reporter.accountId"),
    ("name", "fields.reporter.displayName"),
]);

/// The REST fields an issue list asks for (what [`ISSUE_ROW`] reads).
pub const ROW_FIELDS: &[&str] = &[
    "summary",
    "issuetype",
    "status",
    "priority",
    "assignee",
    "reporter",
    "project",
    "parent",
    "labels",
    "created",
    "updated",
];

/// The REST fields `issue get` asks for (what [`ISSUE`] reads), besides
/// custom fields.
pub const GET_FIELDS: &[&str] = &[
    "summary",
    "issuetype",
    "status",
    "priority",
    "assignee",
    "reporter",
    "project",
    "parent",
    "labels",
    "created",
    "updated",
    "description",
    "components",
    "fixVersions",
    "duedate",
    "resolution",
    "resolutiondate",
    "subtasks",
    "issuelinks",
    "comment",
    "attachment",
    "timetracking",
    "watches",
    "votes",
    "*navigable",
];

pub const ISSUE: Record = Record {
    name: "jira.issue",
    description: "a Jira issue with its description as Markdown",
    primary: "key",
    fields: &[
        Field::new("key", "string", "issue key", "key"),
        Field::new("id", "string", "numeric issue ID", "id").full(),
        Field::new("title", "string", "summary", "fields.summary"),
        Field::new("type", "string", "issue type name", "fields.issuetype.name"),
        Field::new("status", "string", "status name", "fields.status.name"),
        Field::new(
            "statusCategory",
            "string",
            "new, indeterminate or done",
            "fields.status.statusCategory.key",
        )
        .full(),
        Field::new(
            "priority",
            "string",
            "priority name",
            "fields.priority.name",
        ),
        Field::new("assignee", "user", "assignee ({id, name})", "").source(ASSIGNEE),
        Field::new("reporter", "user", "reporter ({id, name})", "").source(REPORTER),
        Field::new("project", "string", "project key", "fields.project.key"),
        Field::new("parent", "string", "parent issue key", "fields.parent.key"),
        Field::new("labels", "string[]", "labels", "fields.labels"),
        Field::new(
            "created",
            "datetime",
            "when it was created",
            "fields.created",
        ),
        Field::new(
            "updated",
            "datetime",
            "when it last changed",
            "fields.updated",
        ),
        Field::new("url", "string", "browse URL", "").source(Source::Fn(issue_url)),
        Field::new(
            "description",
            "markdown",
            "description (Markdown; ADF with --body-format adf)",
            "",
        )
        .source(Source::Fn(description))
        .truncated(BODY),
        Field::new("components", "string[]", "component names", "")
            .source(Source::Fn(components))
            .full(),
        Field::new("fixVersions", "string[]", "fix version names", "")
            .source(Source::Fn(fix_versions))
            .full(),
        Field::new("due", "date", "due date", "fields.duedate").full(),
        Field::new(
            "resolution",
            "string",
            "resolution name",
            "fields.resolution.name",
        )
        .full(),
        Field::new(
            "resolved",
            "datetime",
            "when it was resolved",
            "fields.resolutiondate",
        )
        .full(),
        Field::new("subtasks", "string[]", "subtask keys", "")
            .source(Source::Fn(subtasks))
            .full(),
        Field::new(
            "links",
            "object[]",
            "issue links: {id, type, direction, relation, issue, title, status}",
            "",
        )
        .source(Source::Fn(links))
        .full(),
        Field::new(
            "comments",
            "object[]",
            "the first page of comments: {id, author, created, body}",
            "",
        )
        .source(Source::Fn(comments))
        .full(),
        Field::new(
            "attachments",
            "object[]",
            "attachments: {id, filename, size, mimeType}",
            "",
        )
        .source(Source::Fn(attachments))
        .full(),
        Field::new(
            "timeTracking",
            "object",
            "estimates and time spent",
            "fields.timetracking",
        )
        .full(),
        Field::new(
            "watchers",
            "integer",
            "watcher count",
            "fields.watches.watchCount",
        )
        .full(),
        Field::new("votes", "integer", "vote count", "fields.votes.votes").full(),
        Field::new(
            "custom",
            "object",
            "custom fields with a value, by display name",
            "",
        )
        .source(Source::Fn(custom))
        .full(),
    ],
};

/// One row of an issue list: no description.
pub const ISSUE_ROW: Record = Record {
    name: "jira.issue-row",
    description: "a Jira issue in a list (no description; `issue get` has it)",
    primary: "key",
    fields: &[
        Field::new("key", "string", "issue key", "key"),
        Field::new("title", "string", "summary", "fields.summary"),
        Field::new("type", "string", "issue type name", "fields.issuetype.name"),
        Field::new("status", "string", "status name", "fields.status.name"),
        Field::new(
            "priority",
            "string",
            "priority name",
            "fields.priority.name",
        )
        .full(),
        Field::new("assignee", "user", "assignee ({id, name})", "").source(ASSIGNEE),
        Field::new(
            "updated",
            "datetime",
            "when it last changed",
            "fields.updated",
        ),
        Field::new("id", "string", "numeric issue ID", "id").full(),
        Field::new(
            "statusCategory",
            "string",
            "new, indeterminate or done",
            "fields.status.statusCategory.key",
        )
        .full(),
        Field::new("reporter", "user", "reporter ({id, name})", "")
            .source(REPORTER)
            .full(),
        Field::new("project", "string", "project key", "fields.project.key").full(),
        Field::new("parent", "string", "parent issue key", "fields.parent.key").full(),
        Field::new("labels", "string[]", "labels", "fields.labels").full(),
        Field::new(
            "created",
            "datetime",
            "when it was created",
            "fields.created",
        )
        .full(),
        Field::new("url", "string", "browse URL", "")
            .source(Source::Fn(issue_url))
            .full(),
    ],
};

/// What a create or update wrote.
pub const ISSUE_CHANGE: Record = Record {
    name: "jira.issue-change",
    description: "the issue a write created or changed",
    primary: "key",
    fields: &[
        Field::new("key", "string", "issue key", "key"),
        Field::new("id", "string", "numeric issue ID", "id"),
        Field::new("url", "string", "browse URL", "url"),
        Field::new("updated", "string[]", "the fields an update set", "updated"),
        Field::new(
            "sprint",
            "string",
            "sprint ID it was moved to, or backlog",
            "sprint",
        ),
    ],
};

/// The outcome of a transition, assignment, archive or similar action on
/// one issue.
pub const ISSUE_ACTION: Record = Record {
    name: "jira.issue-action",
    description: "what an action did to one issue",
    primary: "key",
    fields: &[
        Field::new("key", "string", "issue key", "key"),
        Field::new("status", "string", "status after a transition", "status"),
        Field::new("transition", "string", "the transition made", "transition"),
        Field::new(
            "assignee",
            "string",
            "account ID assigned (null: unassigned)",
            "assignee",
        ),
        Field::new(
            "archived",
            "boolean",
            "archived (true) or restored (false)",
            "archived",
        ),
        Field::new(
            "commentId",
            "string",
            "comment added with the transition",
            "commentId",
        )
        .full(),
    ],
};

/// What a delete removed.
pub const DELETED: Record = Record {
    name: "jira.deleted",
    description: "what a delete removed",
    primary: "id",
    fields: &[
        Field::new("id", "string", "the ID or key removed", "id"),
        Field::new("deleted", "boolean", "always true", "deleted"),
        Field::new("issue", "string", "the issue it belonged to", "issue"),
    ],
};

fn comment_body(v: &Value) -> Value {
    let owner = match v["issue"].as_str() {
        Some(k) => format!("{k} comment {}", str_of(&v["id"])),
        None => format!("comment {}", str_of(&v["id"])),
    };
    body_value(&v["body"], &owner)
}

const AUTHOR: Source =
    Source::Object(&[("id", "author.accountId"), ("name", "author.displayName")]);

pub const COMMENT: Record = Record {
    name: "jira.comment",
    description: "a comment on an issue, its body as Markdown",
    primary: "id",
    fields: &[
        Field::new("id", "string", "comment ID", "id"),
        Field::new("author", "user", "author ({id, name})", "").source(AUTHOR),
        Field::new("created", "datetime", "when it was written", "created"),
        Field::new("updated", "datetime", "when it last changed", "updated").full(),
        Field::new(
            "body",
            "markdown",
            "body (Markdown; ADF with --body-format adf)",
            "",
        )
        .source(Source::Fn(comment_body))
        .truncated(BODY),
        Field::new(
            "visibility",
            "object",
            "role or group it is restricted to",
            "visibility",
        )
        .full(),
        Field::new(
            "public",
            "boolean",
            "JSM: visible to customers",
            "jsdPublic",
        )
        .full(),
        Field::new("parent", "string", "the comment it replies to", "parentId").full(),
    ],
};

fn worklog_comment(v: &Value) -> Value {
    body_value(&v["comment"], &format!("worklog {}", str_of(&v["id"])))
}

pub const WORKLOG: Record = Record {
    name: "jira.worklog",
    description: "time logged on an issue",
    primary: "id",
    fields: &[
        Field::new("id", "string", "worklog ID", "").source(Source::First(&["id", "worklogId"])),
        Field::new("author", "user", "who logged it ({id, name})", "").source(AUTHOR),
        Field::new("started", "datetime", "when the work started", "started"),
        Field::new(
            "timeSpent",
            "string",
            "time spent, e.g. 1h 30m",
            "timeSpent",
        ),
        Field::new(
            "seconds",
            "integer",
            "time spent in seconds",
            "timeSpentSeconds",
        ),
        Field::new("comment", "markdown", "what was done (Markdown)", "")
            .source(Source::Fn(worklog_comment)),
        Field::new("issue", "string", "issue ID", "issueId").full(),
        Field::new("updated", "datetime", "when it last changed", "")
            .source(Source::First(&["updated", "updatedTime"]))
            .full(),
        Field::new(
            "visibility",
            "object",
            "role or group it is restricted to",
            "visibility",
        )
        .full(),
    ],
};

pub const ATTACHMENT: Record = Record {
    name: "jira.attachment",
    description: "a file attached to an issue",
    primary: "id",
    fields: &[
        Field::new("id", "string", "attachment ID", "id"),
        Field::new("filename", "string", "file name", "filename"),
        Field::new("size", "integer", "size in bytes", "size"),
        Field::new("mimeType", "string", "media type", "mimeType"),
        Field::new("author", "user", "uploader ({id, name})", "").source(AUTHOR),
        Field::new("created", "datetime", "when it was attached", "created"),
        Field::new("url", "string", "content URL (needs auth)", "content").full(),
    ],
};

pub const DOWNLOAD: Record = Record {
    name: "jira.download",
    description: "a file saved to disk",
    primary: "path",
    fields: &[
        Field::new("id", "string", "attachment ID", "attachmentId"),
        Field::new("path", "string", "the file written", "path"),
        Field::new("bytes", "integer", "bytes written", "bytes"),
    ],
};

pub const LINK: Record = Record {
    name: "jira.link",
    description: "a link between two issues, seen from one of them",
    primary: "id",
    fields: &[
        Field::new("id", "string", "link ID", "id"),
        Field::new("type", "string", "link type name", "type"),
        Field::new("direction", "string", "inward or outward", "direction"),
        Field::new(
            "relation",
            "string",
            "how it reads, e.g. blocks",
            "relation",
        ),
        Field::new("issue", "string", "the other issue's key", "issue"),
        Field::new("title", "string", "the other issue's summary", "title"),
        Field::new("status", "string", "the other issue's status", "status"),
    ],
};

pub const LINK_CREATED: Record = Record {
    name: "jira.link-created",
    description: "a link a command created: source <relation> target",
    primary: "target",
    fields: &[
        Field::new("source", "string", "issue the link reads from", "source"),
        Field::new("target", "string", "issue it points at", "target"),
        Field::new("type", "string", "link type name or ID", "type"),
    ],
};

pub const WEBLINK: Record = Record {
    name: "jira.weblink",
    description: "a remote (web) link on an issue",
    primary: "id",
    fields: &[
        Field::new("id", "string", "remote link ID", "id"),
        Field::new("title", "string", "link text", "object.title"),
        Field::new("url", "string", "target URL", "object.url"),
        Field::new(
            "relationship",
            "string",
            "e.g. mentioned in",
            "relationship",
        ),
        Field::new("globalId", "string", "stable ID", "globalId").full(),
        Field::new(
            "application",
            "string",
            "application name",
            "application.name",
        )
        .full(),
        Field::new(
            "resolved",
            "boolean",
            "shown as resolved",
            "object.status.resolved",
        )
        .full(),
    ],
};

pub const USER: Record = Record {
    name: "jira.user",
    description: "an Atlassian account",
    primary: "id",
    fields: &[
        Field::new("id", "string", "account ID", "accountId"),
        Field::new("name", "string", "display name", "displayName"),
        Field::new("email", "string", "email, when visible", "emailAddress"),
        Field::new(
            "active",
            "boolean",
            "whether the account is active",
            "active",
        ),
        Field::new(
            "type",
            "string",
            "atlassian, app or customer",
            "accountType",
        )
        .full(),
        Field::new("timeZone", "string", "time zone", "timeZone").full(),
        Field::new("locale", "string", "locale", "locale").full(),
    ],
};

fn voters(v: &Value) -> Value {
    match v["voters"].as_array() {
        Some(a) => a.iter().map(user_of).collect(),
        None => Value::Null,
    }
}

pub const VOTES: Record = Record {
    name: "jira.votes",
    description: "the votes on an issue",
    primary: "votes",
    fields: &[
        Field::new("votes", "integer", "vote count", "votes"),
        Field::new("hasVoted", "boolean", "whether you voted", "hasVoted"),
        Field::new("voters", "user[]", "who voted, when visible", "").source(Source::Fn(voters)),
    ],
};

pub const PROPERTY: Record = Record {
    name: "jira.property",
    description: "an entity property (key and JSON value)",
    primary: "key",
    fields: &[
        Field::new("key", "string", "property key", "key"),
        Field::new("value", "any", "the JSON value (not in lists)", "value"),
    ],
};

fn change_items(v: &Value) -> Value {
    match v["items"].as_array() {
        Some(a) => a
            .iter()
            .map(|i| json!({"field": i["field"], "from": i["fromString"], "to": i["toString"]}))
            .collect(),
        None => Value::Null,
    }
}

pub const CHANGE: Record = Record {
    name: "jira.change",
    description: "one entry of an issue's change history",
    primary: "id",
    fields: &[
        Field::new("id", "string", "history ID", "id"),
        Field::new("author", "user", "who changed it ({id, name})", "").source(AUTHOR),
        Field::new("created", "datetime", "when", "created"),
        Field::new("items", "object[]", "what changed: {field, from, to}", "")
            .source(Source::Fn(change_items)),
    ],
};

fn required_fields(v: &Value) -> Value {
    match v["fields"].as_object() {
        Some(m) => m
            .iter()
            .filter(|(_, f)| f["required"] == true)
            .map(|(k, _)| Value::from(k.as_str()))
            .collect(),
        None => json!([]),
    }
}

pub const TRANSITION: Record = Record {
    name: "jira.transition",
    description: "a transition an issue can make now",
    primary: "id",
    fields: &[
        Field::new("id", "string", "transition ID", "id"),
        Field::new("name", "string", "transition name", "name"),
        Field::new("to", "string", "status it leads to", "to.name"),
        Field::new(
            "category",
            "string",
            "that status's category key",
            "to.statusCategory.key",
        ),
        Field::new(
            "hasScreen",
            "boolean",
            "whether it shows a screen",
            "hasScreen",
        )
        .full(),
        Field::new("required", "string[]", "fields it requires", "")
            .source(Source::Fn(required_fields))
            .full(),
    ],
};

fn lead(v: &Value) -> Value {
    user_of(&v["lead"])
}

fn issue_type_names(v: &Value) -> Value {
    names_of(&v["issueTypes"], "name")
}

fn project_url(v: &Value) -> Value {
    match (site_of(v), v["key"].as_str()) {
        (Some(site), Some(key)) => format!("{site}/browse/{key}").into(),
        _ => Value::Null,
    }
}

pub const PROJECT: Record = Record {
    name: "jira.project",
    description: "a Jira project",
    primary: "key",
    fields: &[
        Field::new("key", "string", "project key", "key"),
        Field::new("id", "string", "numeric project ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new(
            "type",
            "string",
            "software, business or service_desk",
            "projectTypeKey",
        ),
        Field::new("lead", "user", "project lead ({id, name})", "").source(Source::Fn(lead)),
        Field::new(
            "category",
            "string",
            "category name",
            "projectCategory.name",
        )
        .full(),
        Field::new("style", "string", "classic or next-gen", "style").full(),
        Field::new("description", "string", "description", "description").full(),
        Field::new("archived", "boolean", "whether it is archived", "archived").full(),
        Field::new("issueTypes", "string[]", "issue type names", "")
            .source(Source::Fn(issue_type_names))
            .full(),
        Field::new("url", "string", "browse URL", "")
            .source(Source::Fn(project_url))
            .full(),
    ],
};

pub const VERSION: Record = Record {
    name: "jira.version",
    description: "a project version (release)",
    primary: "id",
    fields: &[
        Field::new("id", "string", "version ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("released", "boolean", "released", "released"),
        Field::new("archived", "boolean", "archived", "archived"),
        Field::new("releaseDate", "date", "release date", "releaseDate"),
        Field::new("startDate", "date", "start date", "startDate").full(),
        Field::new("description", "string", "description", "description").full(),
        Field::new("overdue", "boolean", "past its release date", "overdue").full(),
        Field::new("project", "string", "project ID", "projectId").full(),
        Field::new(
            "issueStatus",
            "object",
            "fix-version issue counts by status category (`get` only)",
            "issuesStatusForFixVersion",
        )
        .full(),
    ],
};

pub const COMPONENT: Record = Record {
    name: "jira.component",
    description: "a project component",
    primary: "id",
    fields: &[
        Field::new("id", "string", "component ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("lead", "user", "component lead ({id, name})", "").source(Source::Fn(lead)),
        Field::new(
            "assigneeType",
            "string",
            "who new issues go to",
            "assigneeType",
        ),
        Field::new("project", "string", "project key", "project").full(),
        Field::new("description", "string", "description", "description").full(),
    ],
};

pub const BOARD: Record = Record {
    name: "jira.board",
    description: "an Agile board",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "board ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("type", "string", "scrum, kanban or simple", "type"),
        Field::new("project", "string", "project key", "location.projectKey"),
        Field::new(
            "filter",
            "string",
            "the filter it shows (`get` only)",
            "configuration.filter.id",
        )
        .full(),
        Field::new(
            "columns",
            "object[]",
            "columns and their statuses (`get` only)",
            "configuration.columnConfig.columns",
        )
        .full(),
        Field::new(
            "estimation",
            "object",
            "estimation field (`get` only)",
            "configuration.estimation",
        )
        .full(),
    ],
};

pub const QUICK_FILTER: Record = Record {
    name: "jira.quick-filter",
    description: "a board quick filter",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "quick filter ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("jql", "string", "the JQL it adds", "jql"),
        Field::new("description", "string", "description", "description").full(),
        Field::new("board", "integer", "board ID", "boardId").full(),
    ],
};

pub const SPRINT: Record = Record {
    name: "jira.sprint",
    description: "an Agile sprint",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "sprint ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("state", "string", "future, active or closed", "state"),
        Field::new("start", "datetime", "start", "startDate"),
        Field::new("end", "datetime", "planned end", "endDate"),
        Field::new("goal", "string", "goal", "goal"),
        Field::new("board", "integer", "board it belongs to", "originBoardId").full(),
        Field::new(
            "completed",
            "datetime",
            "when it was completed",
            "completeDate",
        )
        .full(),
        Field::new(
            "movedIssues",
            "string[]",
            "issues moved out when completing",
            "movedIssues",
        )
        .full(),
    ],
};

const OWNER: Source = Source::Object(&[("id", "owner.accountId"), ("name", "owner.displayName")]);

pub const FILTER: Record = Record {
    name: "jira.filter",
    description: "a saved filter",
    primary: "id",
    fields: &[
        Field::new("id", "string", "filter ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("jql", "string", "its JQL", "jql"),
        Field::new("owner", "user", "owner ({id, name})", "").source(OWNER),
        Field::new(
            "favourite",
            "boolean",
            "one of your favourites",
            "favourite",
        ),
        Field::new("description", "string", "description", "description").full(),
        Field::new("url", "string", "where to view it", "viewUrl").full(),
        Field::new(
            "sharePermissions",
            "object[]",
            "who can see it",
            "sharePermissions",
        )
        .full(),
        Field::new(
            "editPermissions",
            "object[]",
            "who can edit it",
            "editPermissions",
        )
        .full(),
    ],
};

pub const SHARE: Record = Record {
    name: "jira.share",
    description: "a share permission on a filter or dashboard",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "share permission ID", "id"),
        Field::new(
            "type",
            "string",
            "group, project, projectRole, user, global, ...",
            "type",
        ),
        Field::new("project", "string", "project key", "project.key"),
        Field::new("role", "string", "project role name", "role.name"),
        Field::new("group", "string", "group name", "group.name"),
        Field::new("user", "string", "account ID", "user.accountId"),
    ],
};

pub const COLUMN: Record = Record {
    name: "jira.column",
    description: "a column of a filter's issue navigator view",
    primary: "value",
    fields: &[
        Field::new("label", "string", "column title", "label"),
        Field::new("value", "string", "field ID", "value"),
    ],
};

const SUBSCRIBER: Source =
    Source::Object(&[("id", "user.accountId"), ("name", "user.displayName")]);

pub const SUBSCRIPTION: Record = Record {
    name: "jira.subscription",
    description: "an email subscription to a filter",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "subscription ID", "id"),
        Field::new("user", "user", "subscribed user ({id, name})", "").source(SUBSCRIBER),
        Field::new("group", "string", "subscribed group", "group.name"),
    ],
};

pub const DASHBOARD: Record = Record {
    name: "jira.dashboard",
    description: "a dashboard",
    primary: "id",
    fields: &[
        Field::new("id", "string", "dashboard ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("owner", "user", "owner ({id, name})", "").source(OWNER),
        Field::new(
            "favourite",
            "boolean",
            "one of your favourites",
            "isFavourite",
        ),
        Field::new("description", "string", "description", "description").full(),
        Field::new("url", "string", "where to view it", "view").full(),
        Field::new(
            "sharePermissions",
            "object[]",
            "who can see it",
            "sharePermissions",
        )
        .full(),
        Field::new(
            "editPermissions",
            "object[]",
            "who can edit it",
            "editPermissions",
        )
        .full(),
    ],
};

pub const GADGET: Record = Record {
    name: "jira.gadget",
    description: "a gadget on a dashboard, or one the catalog offers",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "gadget ID (none in the catalog)", "id"),
        Field::new("title", "string", "title", "title"),
        Field::new("moduleKey", "string", "app module key", "moduleKey"),
        Field::new("uri", "string", "gadget spec URI", "uri"),
        Field::new("color", "string", "colour", "color"),
        Field::new("position", "object", "{row, column}", "position"),
    ],
};

fn field_type(v: &Value) -> Value {
    let s = &v["schema"];
    match (s["type"].as_str(), s["items"].as_str()) {
        (Some("array"), Some(items)) => format!("{items}[]").into(),
        (Some(t), _) => t.into(),
        _ => Value::Null,
    }
}

pub const FIELD: Record = Record {
    name: "jira.field",
    description: "a Jira field, or one field of a create or edit screen",
    primary: "id",
    fields: &[
        Field::new(
            "id",
            "string",
            "field ID (customfield_N for custom fields)",
            "",
        )
        .source(Source::First(&["fieldId", "id", "key"])),
        Field::new("name", "string", "display name", "name"),
        Field::new(
            "type",
            "string",
            "value type, e.g. string, option, user[]",
            "",
        )
        .source(Source::Fn(field_type)),
        Field::new("custom", "boolean", "a custom field (site list)", "custom"),
        Field::new("required", "boolean", "required on the screen", "required"),
        Field::new(
            "acceptedInput",
            "string",
            "what --field NAME=VALUE takes: text, option, multi-option, user, date, ...",
            "acceptedInput",
        ),
        Field::new(
            "allowedValues",
            "object[]",
            "values it accepts",
            "allowedValues",
        )
        .full(),
        Field::new(
            "example",
            "string",
            "an example --field argument",
            "example",
        )
        .full(),
        Field::new("operations", "string[]", "edit operations", "operations").full(),
    ],
};

pub const ISSUE_TYPE: Record = Record {
    name: "jira.issue-type",
    description: "an issue type",
    primary: "id",
    fields: &[
        Field::new("id", "string", "issue type ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("subtask", "boolean", "a subtask type", "subtask"),
        Field::new(
            "level",
            "integer",
            "hierarchy: -1 subtask, 0 standard, 1 epic",
            "hierarchyLevel",
        ),
        Field::new("description", "string", "description", "description").full(),
    ],
};

pub const STATUS: Record = Record {
    name: "jira.status",
    description: "an issue status",
    primary: "id",
    fields: &[
        Field::new("id", "string", "status ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new(
            "category",
            "string",
            "new, indeterminate or done",
            "statusCategory.key",
        ),
        Field::new("description", "string", "description", "description").full(),
    ],
};

pub const PRIORITY: Record = Record {
    name: "jira.priority",
    description: "an issue priority",
    primary: "id",
    fields: &[
        Field::new("id", "string", "priority ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("description", "string", "description", "description").full(),
        Field::new("color", "string", "colour", "statusColor").full(),
    ],
};

pub const LINK_TYPE: Record = Record {
    name: "jira.link-type",
    description: "an issue link type",
    primary: "id",
    fields: &[
        Field::new("id", "string", "link type ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("inward", "string", "how the inward side reads", "inward"),
        Field::new("outward", "string", "how the outward side reads", "outward"),
    ],
};

/// The value at `dotted` as a string (IDs come as numbers or strings).
pub fn id_at(v: &Value, dotted: &str) -> String {
    path(v, dotted).map(str_of).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Selection, project};

    #[test]
    fn issue_reads_rest_shapes() {
        let v = json!({
            "id": "10001", "key": "P-1", "self": "https://acme.atlassian.net/rest/api/3/issue/10001",
            "names": {"customfield_10016": "Story Points"},
            "fields": {
                "summary": "S", "status": {"name": "Done", "statusCategory": {"key": "done"}},
                "assignee": {"accountId": "a1", "displayName": "Ann", "avatarUrls": {}},
                "customfield_10016": 3, "customfield_1": null,
                "description": {"type": "doc", "version": 1, "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "Hi"}]}]},
                "issuelinks": [{"id": "9", "type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
                    "outwardIssue": {"key": "P-2", "fields": {"summary": "T", "status": {"name": "Open"}}}}],
            }
        });
        let d = project(&v, &ISSUE, &Selection::Default).unwrap();
        assert_eq!(d["title"], "S");
        assert_eq!(d["description"], "Hi");
        assert_eq!(d["url"], "https://acme.atlassian.net/browse/P-1");
        assert_eq!(d["assignee"], json!({"id": "a1", "name": "Ann"}));
        let f = project(&v, &ISSUE, &Selection::Full).unwrap();
        assert_eq!(f["custom"], json!({"Story Points": 3}));
        assert_eq!(f["links"][0]["relation"], "blocks");
        assert_eq!(f["links"][0]["issue"], "P-2");
    }
}

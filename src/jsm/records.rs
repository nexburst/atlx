//! The records Jira Service Management commands return.

use crate::schema::{Field, Record, Source};
use serde_json::{Value, json};

fn summary(v: &Value) -> Value {
    field_value(v, "summary")
}

fn description(v: &Value) -> Value {
    field_value(v, "description")
}

/// A request field's value by field ID, from `requestFieldValues`.
fn field_value(v: &Value, id: &str) -> Value {
    v["requestFieldValues"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f["fieldId"] == id)
        .map(|f| f["value"].clone())
        .unwrap_or(Value::Null)
}

/// Every request field as `{label: value}`.
fn fields(v: &Value) -> Value {
    let Some(list) = v["requestFieldValues"].as_array() else {
        return Value::Null;
    };
    Value::Object(
        list.iter()
            .map(|f| {
                let label = f["label"]
                    .as_str()
                    .or(f["fieldId"].as_str())
                    .unwrap_or_default()
                    .to_string();
                (label, f["value"].clone())
            })
            .collect(),
    )
}

const REPORTER: Source = Source::Object(&[
    ("id", "reporter.accountId"),
    ("name", "reporter.displayName"),
]);
const AUTHOR: Source =
    Source::Object(&[("id", "author.accountId"), ("name", "author.displayName")]);

pub const REQUEST: Record = Record {
    name: "jsm.request",
    description: "a customer request",
    primary: "key",
    fields: &[
        Field::new("key", "string", "issue key", "issueKey"),
        Field::new("id", "string", "issue ID", "issueId").full(),
        Field::new("title", "string", "summary", "").source(Source::Fn(summary)),
        Field::new("status", "string", "current status", "currentStatus.status"),
        Field::new("type", "string", "request type ID", "requestTypeId"),
        Field::new("desk", "string", "service desk ID", "serviceDeskId"),
        Field::new("reporter", "user", "who raised it ({id, name})", "").source(REPORTER),
        Field::new(
            "created",
            "datetime",
            "when it was raised",
            "createdDate.iso8601",
        ),
        Field::new("url", "string", "portal URL", "_links.web"),
        Field::new("description", "string", "description", "")
            .source(Source::Fn(description))
            .full(),
        Field::new("fields", "object", "every request field, by label", "")
            .source(Source::Fn(fields))
            .full(),
    ],
};

pub const COMMENT: Record = Record {
    name: "jsm.comment",
    description: "a comment on a request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "comment ID", "id"),
        Field::new("author", "user", "author ({id, name})", "").source(AUTHOR),
        Field::new(
            "created",
            "datetime",
            "when it was written",
            "created.iso8601",
        ),
        Field::new("public", "boolean", "visible to customers", "public"),
        Field::new("body", "string", "body text", "body"),
    ],
};

fn approvers(v: &Value) -> Value {
    match v["approvers"].as_array() {
        Some(a) => a
            .iter()
            .map(|x| {
                json!({
                    "id": x["approver"]["accountId"],
                    "name": x["approver"]["displayName"],
                    "decision": x["approverDecision"],
                })
            })
            .collect(),
        None => Value::Null,
    }
}

pub const APPROVAL: Record = Record {
    name: "jsm.approval",
    description: "an approval on a request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "approval ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new(
            "decision",
            "string",
            "pending, approved or declined",
            "finalDecision",
        ),
        Field::new(
            "canAnswer",
            "boolean",
            "whether you can answer it",
            "canAnswerApproval",
        ),
        Field::new("approvers", "object[]", "{id, name, decision}", "")
            .source(Source::Fn(approvers)),
        Field::new(
            "created",
            "datetime",
            "when it was requested",
            "createdDate.iso8601",
        ),
        Field::new(
            "completed",
            "datetime",
            "when it was decided",
            "completedDate.iso8601",
        )
        .full(),
    ],
};

pub const TRANSITION: Record = Record {
    name: "jsm.transition",
    description: "a transition a customer can make on a request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "transition ID", "id"),
        Field::new("name", "string", "name", "name"),
    ],
};

pub const TRANSITIONED: Record = Record {
    name: "jsm.transitioned",
    description: "the transition a command made",
    primary: "request",
    fields: &[
        Field::new("request", "string", "request key", "request"),
        Field::new("transition", "string", "transition name", "transition"),
        Field::new("transitionId", "string", "transition ID", "transitionId"),
    ],
};

pub const SLA: Record = Record {
    name: "jsm.sla",
    description: "a service level agreement on a request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "SLA ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new(
            "breached",
            "boolean",
            "the running cycle is breached",
            "ongoingCycle.breached",
        ),
        Field::new(
            "paused",
            "boolean",
            "the running cycle is paused",
            "ongoingCycle.paused",
        ),
        Field::new(
            "remaining",
            "string",
            "time left",
            "ongoingCycle.remainingTime.friendly",
        ),
        Field::new(
            "goal",
            "string",
            "goal duration",
            "ongoingCycle.goalDuration.friendly",
        ),
        Field::new(
            "breachTime",
            "datetime",
            "when it breaches",
            "ongoingCycle.breachTime.iso8601",
        ),
        Field::new(
            "completed",
            "object[]",
            "completed cycles",
            "completedCycles",
        )
        .full(),
    ],
};

pub const DESK: Record = Record {
    name: "jsm.desk",
    description: "a service desk (one per service project)",
    primary: "id",
    fields: &[
        Field::new("id", "string", "service desk ID", "id"),
        Field::new("key", "string", "project key", "projectKey"),
        Field::new("name", "string", "project name", "projectName"),
        Field::new("project", "string", "project ID", "projectId").full(),
    ],
};

pub const REQUEST_TYPE: Record = Record {
    name: "jsm.request-type",
    description: "a request type of a service desk",
    primary: "id",
    fields: &[
        Field::new("id", "string", "request type ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("description", "string", "description", "description"),
        Field::new("desk", "string", "service desk ID", "serviceDeskId").full(),
        Field::new("groups", "string[]", "group IDs", "groupIds").full(),
        Field::new(
            "fields",
            "object[]",
            "its fields, with valid values (`get` only)",
            "fields",
        )
        .full(),
        Field::new(
            "example",
            "object",
            "an example --answers-json document (`get` only)",
            "example",
        )
        .full(),
    ],
};

pub const QUEUE: Record = Record {
    name: "jsm.queue",
    description: "an agent queue",
    primary: "id",
    fields: &[
        Field::new("id", "string", "queue ID", "id"),
        Field::new("name", "string", "name", "name"),
        Field::new("count", "integer", "issues in it", "issueCount"),
        Field::new("jql", "string", "its JQL", "jql").full(),
    ],
};

pub const ARTICLE: Record = Record {
    name: "jsm.article",
    description: "a knowledge base article",
    primary: "title",
    fields: &[
        Field::new("title", "string", "title", "title"),
        Field::new("excerpt", "string", "matching excerpt", "excerpt"),
        Field::new("url", "string", "where to read it", "content.iframeSrc"),
        Field::new(
            "source",
            "object",
            "where it lives (Confluence page)",
            "source",
        )
        .full(),
    ],
};

pub const PARTICIPANT: Record = Record {
    name: "jsm.participant",
    description: "a person sharing a request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "account ID", "").source(Source::First(&["accountId", "id"])),
        Field::new("name", "string", "display name", "displayName"),
        Field::new("email", "string", "email, when visible", "emailAddress"),
        Field::new("deleted", "boolean", "removed by this command", "deleted"),
    ],
};

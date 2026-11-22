//! `jira workitem bulk-*|create-bulk|archive|unarchive|property`.

use super::issue_ref;
use crate::cli::util::{mark_partial, require_yes, text_arg};
use crate::client::{ApiError, Client};
use crate::jira::{self, bulk};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Work item keys, IDs, ARIs or URLs
    keys: Vec<String>,
    /// Another work item; repeatable
    #[arg(long)]
    id: Vec<String>,
    /// Field IDs to read, comma-separated
    #[arg(long, allow_hyphen_values = true)]
    fields: Option<String>,
    /// Another field ID; repeatable
    #[arg(long, allow_hyphen_values = true)]
    field: Vec<String>,
    /// REST expands, comma-separated
    #[arg(long)]
    expand: Option<String>,
    /// Entity property keys, comma-separated
    #[arg(long)]
    properties: Option<String>,
    /// Read every field
    #[arg(long)]
    full: bool,
    /// Exit with an error unless every work item comes back
    #[arg(long)]
    strict: bool,
}

#[derive(Args)]
pub(crate) struct CreateArgs {
    /// REST body (`issueUpdates` object or array), or - for stdin
    #[arg(long)]
    input_json: String,
}

#[derive(Args)]
pub(crate) struct TransitionArgs {
    /// Work item key, ID, ARI or URL; repeatable
    #[arg(long)]
    id: Vec<String>,
    /// Several work items, comma-separated
    #[arg(long)]
    ids: Option<String>,
    /// The transition's ID in the workflow these work items share
    #[arg(long)]
    transition_id: String,
    /// Show the request and send nothing
    #[arg(long)]
    dry_run: bool,
    /// Confirm the transition (required)
    #[arg(long, short)]
    yes: bool,
    /// Email watchers about the change
    #[arg(long)]
    send_notifications: bool,
}

#[derive(Args)]
pub(crate) struct ArchiveArgs {
    /// Work item key, ID, ARI or URL; repeatable
    #[arg(long, required = true)]
    id: Vec<String>,
}

#[derive(Subcommand)]
pub(crate) enum PropertyCmd {
    /// The property keys on a work item
    Query {
        /// Work item key, ID, ARI or URL
        #[arg(long)]
        issue_id: String,
    },
    /// One property's value
    Get(PropertyRef),
    /// Create or replace a property
    Set {
        #[command(flatten)]
        at: PropertyRef,
        /// The value as JSON, or - for stdin
        #[arg(long, allow_hyphen_values = true)]
        value_json: String,
    },
    /// Delete a property
    Delete(PropertyRef),
}

#[derive(Args)]
pub(crate) struct PropertyRef {
    /// Work item key, ID, ARI or URL
    #[arg(long)]
    issue_id: String,
    /// Property key
    #[arg(long)]
    property_key: String,
}

fn parse(flag: &str, text: String) -> Result<Value> {
    serde_json::from_str(&text_arg(text)?)
        .map_err(|e| ApiError::usage(format!("{flag} is not valid JSON: {e}")).into())
}

fn refs<'a>(ids: impl Iterator<Item = &'a String>) -> Vec<String> {
    ids.map(|s| issue_ref(s)).collect()
}

fn non_empty(v: &Value) -> bool {
    match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        _ => false,
    }
}

pub(super) fn get(c: &Client, a: GetArgs) -> Result<Value> {
    let ids = refs(a.keys.iter().chain(&a.id));
    let mut fields = a.fields.as_deref().map(jira::csv).unwrap_or_default();
    fields.extend(a.field);
    let given_fields = a.full || !fields.is_empty();
    if a.full {
        fields = vec!["*all".into()];
    } else if fields.is_empty() {
        fields = jira::GET_FIELDS.iter().map(|f| f.to_string()).collect();
    }
    let expand = a.expand.as_deref().map(jira::csv).unwrap_or_default();
    let properties = a.properties.as_deref().map(jira::csv).unwrap_or_default();
    let o = bulk::Fetch {
        fields: &fields,
        expand: &expand,
        properties: &properties,
    };
    let data = bulk::fetch(c, &ids, &o)?;
    let failed = non_empty(&data["missing"]) || non_empty(&data["errors"]);
    let none = !non_empty(&data["issues"]);
    let rows = batch_rows(&ids, &data);
    if failed && (a.strict || none) {
        let mut why = data["missing"].to_string();
        if non_empty(&data["errors"]) {
            why = format!("{why}; {}", data["errors"]);
        }
        return Err(ApiError::new(
            crate::client::ErrorKind::NotFound,
            format!("not every work item came back: {why}"),
        )
        .into());
    }
    if failed {
        mark_partial();
    }
    let mut request = json!({"site": jira::site(c), "issueIdsOrKeys": ids});
    if given_fields {
        request["fields"] = json!(fields);
    }
    let mut env = envelope("jira.workitem.bulk-get", "jira.workitem").request(request);
    if failed {
        env = env.meta("partial", true);
    }
    Ok(env.data(rows))
}

/// twg's `{items, summary}` of a bulk read: an item per ID, in the order
/// given. A moved work item answers under its new key, so it can only be
/// tied to its old one when it's the only one unaccounted for.
fn batch_rows(ids: &[String], data: &Value) -> Value {
    let list = |k: &str| data[k].as_array().cloned().unwrap_or_default();
    let (mut issues, missing) = (list("issues"), list("missing"));
    let mut seen = std::collections::HashSet::new();
    let mut items: Vec<Value> = Vec::new();
    let mut unmatched = Vec::new();
    for id in ids.iter().filter(|id| seen.insert(id.to_ascii_uppercase())) {
        let at = issues.iter().position(|issue| {
            [&issue["key"], &issue["id"]]
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(id)))
        });
        items.push(match at {
            Some(i) => json!({"input": id, "ok": true, "data": issues.remove(i)}),
            None => {
                unmatched.push(items.len());
                json!({"input": id, "ok": false, "error": {"message": "Jira didn't return it"}})
            }
        });
    }
    if missing.is_empty() && unmatched.len() == issues.len() {
        for (at, issue) in unmatched.iter().zip(issues) {
            let input = if unmatched.len() == 1 {
                items[*at]["input"].clone()
            } else {
                issue["key"].clone()
            };
            items[*at] = json!({"input": input, "ok": true, "data": issue});
        }
    } else {
        // Still work items that were asked for: don't drop them.
        for issue in issues {
            items.push(json!({"input": issue["key"], "ok": true, "data": issue}));
        }
    }
    let good = items.iter().filter(|i| i["ok"] == true).count();
    let summary = json!({
        "requested": seen.len(),
        "returned": good,
        "succeeded": good,
        "failed": items.len() - good,
        "omitted": 0,
        "truncated": 0,
    });
    json!({"items": items, "summary": summary})
}

pub(super) fn create(c: &Client, a: CreateArgs) -> Result<Value> {
    let items = bulk::create_input(parse("--input-json", a.input_json)?)?;
    let data = bulk::create(c, &items)?;
    if non_empty(&data["errors"]) {
        mark_partial();
    }
    Ok(envelope("jira.workitem.create-bulk", "jira.issue")
        .request(json!({"site": jira::site(c), "count": items.len()}))
        .data(data))
}

pub(super) fn transition(c: &Client, a: TransitionArgs) -> Result<Value> {
    if !a.dry_run {
        require_yes(a.yes, "a bulk transition")?;
    }
    let listed = a.ids.as_deref().map(jira::csv).unwrap_or_default();
    let ids = refs(a.id.iter().chain(&listed));
    let data = bulk::transition(c, &ids, &a.transition_id, a.send_notifications, a.dry_run)?;
    Ok(envelope("jira.workitem.bulk-transition", "jira.issue")
        .request(json!({"site": jira::site(c), "ids": ids, "transitionId": a.transition_id}))
        .data(data))
}

pub(super) fn archive(c: &Client, a: ArchiveArgs, on: bool) -> Result<Value> {
    let ids = refs(a.id.iter());
    let data = bulk::archive(c, &ids, on)?;
    if non_empty(&data["errors"]) {
        mark_partial();
    }
    let name = if on {
        "jira.workitem.archive"
    } else {
        "jira.workitem.unarchive"
    };
    Ok(envelope(name, "jira.issue")
        .request(json!({"site": jira::site(c), "ids": ids}))
        .data(data))
}

pub(super) fn property(c: &Client, cmd: PropertyCmd) -> Result<Value> {
    let (name, issue, key, data) = match cmd {
        PropertyCmd::Query { issue_id } => {
            let issue = issue_ref(&issue_id);
            let data = bulk::properties(c, &issue)?;
            let keys = data["keys"].as_array().cloned().unwrap_or_default();
            ("jira.workitem.property.query", issue, None, keys.into())
        }
        PropertyCmd::Get(at) => {
            let issue = issue_ref(&at.issue_id);
            let data = bulk::property(c, &issue, &at.property_key)?;
            (
                "jira.workitem.property.get",
                issue,
                Some(at.property_key),
                data,
            )
        }
        PropertyCmd::Set { at, value_json } => {
            let issue = issue_ref(&at.issue_id);
            let value = parse("--value-json", value_json)?;
            let data = bulk::property_set(c, &issue, &at.property_key, &value)?;
            (
                "jira.workitem.property.set",
                issue,
                Some(at.property_key),
                data,
            )
        }
        PropertyCmd::Delete(at) => {
            let issue = issue_ref(&at.issue_id);
            let data = bulk::property_delete(c, &issue, &at.property_key)?;
            (
                "jira.workitem.property.delete",
                issue,
                Some(at.property_key),
                data,
            )
        }
    };
    Ok(envelope(name, "jira.issue-property")
        .request(json!({"site": jira::site(c), "issueId": issue, "propertyKey": key}))
        .data(data))
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    fn parses(args: &str) -> bool {
        Cli::try_parse_from(["atlx"].into_iter().chain(args.split_whitespace())).is_ok()
    }

    #[test]
    fn parses_bulk_property_and_user_commands() {
        for args in [
            "jira workitem bulk-get P-1 P-2 --id P-3 --fields summary,-comment --field status --strict",
            "jira workitem bulk-get P-1 --full --expand names --properties a,b",
            "jira workitem create-bulk --input-json -",
            "jira workitem bulk-transition --ids P-1,P-2 --transition-id 31 --dry-run",
            "jira workitem bulk-transition --id P-1 --transition-id 31 --send-notifications --yes",
            "jira workitem archive --id P-1 --id P-2",
            "jira workitem unarchive --id P-1",
            "jira workitem property query --issue-id P-1",
            "jira workitem property get --issue-id P-1 --property-key k",
            "jira workitem property set --issue-id P-1 --property-key k --value-json -1",
            "jira workitem property delete --issue-id P-1 --property-key k",
            "user get",
            "user get abc",
            "user get --account-id abc",
            "user bulk-lookup abc a@example.com --input-type auto --strict",
        ] {
            assert!(parses(args), "{args}");
        }
        for args in [
            "jira workitem archive",
            "jira workitem create-bulk",
            "jira workitem bulk-transition --id P-1",
            "jira workitem property delete --issue-id P-1",
        ] {
            assert!(!parses(args), "{args}");
        }
    }
}

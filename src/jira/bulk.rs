//! Many work items at once, their entity properties, and user lookups.

use super::{enc, fetch_users, flatten, myself, profile, strip_media, user_row};
use crate::client::{ApiError, Client};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// `bulkfetch` takes 100 work items whatever the fields.
const FETCH_BATCH: usize = 100;
/// `issue/bulk` creates at most 50 work items.
const CREATE_MAX: usize = 50;
/// Bulk transition and archive take at most 1000 work items.
const ACTION_MAX: usize = 1000;
/// `user/bulk` is asked for this many accounts at a time.
const USER_BATCH: usize = 50;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// The IDs without repeats, in their order.
fn unique(ids: &[String], max: usize) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids.iter().map(|s| s.trim()) {
        if id.is_empty() {
            return Err(usage("a work item ID is empty"));
        }
        if seen.insert(id.to_ascii_uppercase()) {
            out.push(id.to_string());
        }
    }
    if out.is_empty() || out.len() > max {
        return Err(usage(format!(
            "pass 1 to {max} work items (got {})",
            out.len()
        )));
    }
    Ok(out)
}

// --- bulk reads and writes ---------------------------------------------------

#[derive(Debug, Default)]
pub struct Fetch<'a> {
    pub fields: &'a [String],
    pub expand: &'a [String],
    pub properties: &'a [String],
}

/// `{issues, missing, errors}`. Jira leaves out what it can't show without
/// saying so, and answers for a moved work item under its new key, so
/// `missing` is only filled in when fewer came back than were asked for.
pub fn fetch(c: &Client, ids: &[String], o: &Fetch) -> Result<Value> {
    let ids = unique(ids, usize::MAX)?;
    let mut found: Vec<Value> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    for chunk in ids.chunks(FETCH_BATCH) {
        let mut body = json!({"issueIdsOrKeys": chunk, "fields": o.fields});
        if !o.expand.is_empty() {
            body["expand"] = json!(o.expand);
        }
        if !o.properties.is_empty() {
            body["properties"] = json!(o.properties);
        }
        let v = c.post("/rest/api/3/issue/bulkfetch", &body)?;
        let issues = v["issues"]
            .as_array()
            .ok_or_else(|| anyhow!("Jira returned no issue list"))?;
        found.extend(issues.iter().cloned());
        errors.extend(v["issueErrors"].as_array().into_iter().flatten().cloned());
    }
    let position = |issue: &Value| {
        ids.iter().position(|id| {
            [&issue["key"], &issue["id"]]
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(id)))
        })
    };
    // Jira sorts by ID; give them back as asked, with moved ones last.
    found.sort_by_key(|issue| position(issue).unwrap_or(usize::MAX));
    let matched: HashSet<usize> = found.iter().filter_map(position).collect();
    let missing: Vec<&String> = if found.len() < ids.len() {
        ids.iter()
            .enumerate()
            .filter(|(i, _)| !matched.contains(i))
            .map(|(_, id)| id)
            .collect()
    } else {
        Vec::new()
    };
    let issues: Vec<Value> = found
        .iter()
        .map(|issue| {
            let mut out = flatten(c, issue);
            // What `--expand` and `--properties` add sits beside `fields`.
            for (k, v) in issue.as_object().into_iter().flatten() {
                if k != "fields" && out.get(k).is_none() {
                    out[k.as_str()] = v.clone();
                }
            }
            out
        })
        .collect();
    Ok(json!({"issues": issues, "missing": missing, "errors": errors}))
}

/// The `issueUpdates` in an `issueUpdates` object or a bare array of them.
pub fn create_input(v: Value) -> Result<Vec<Value>> {
    let items = match v {
        Value::Array(items) => items,
        Value::Object(mut m) => match m.shift_remove("issueUpdates") {
            Some(Value::Array(items)) => items,
            _ => return Err(usage("the JSON object needs an issueUpdates array")),
        },
        _ => return Err(usage("pass a JSON array or an issueUpdates object")),
    };
    if items.is_empty() || items.len() > CREATE_MAX {
        return Err(usage(format!(
            "pass 1 to {CREATE_MAX} work items (got {})",
            items.len()
        )));
    }
    if let Some(i) = items.iter().position(|i| !i["fields"].is_object()) {
        return Err(usage(format!("item {} needs a \"fields\" object", i + 1)));
    }
    Ok(items)
}

/// Some can fail while the others are created: `errors` names those.
pub fn create(c: &Client, items: &[Value]) -> Result<Value> {
    let v = c.post("/rest/api/3/issue/bulk", &json!({"issueUpdates": items}))?;
    Ok(json!({
        "issues": v["issues"].as_array().cloned().unwrap_or_default(),
        "errors": v["errors"].as_array().cloned().unwrap_or_default(),
    }))
}

/// Queues the transition and returns its task.
pub fn transition(
    c: &Client,
    ids: &[String],
    transition: &str,
    notify: bool,
    dry_run: bool,
) -> Result<Value> {
    let ids = unique(ids, ACTION_MAX)?;
    let transition = transition.trim();
    if transition.is_empty() || !transition.bytes().all(|b| b.is_ascii_digit()) {
        return Err(usage(format!("'{transition}' is not a transition ID")));
    }
    let body = json!({
        "bulkTransitionInputs": [{"selectedIssueIdsOrKeys": ids, "transitionId": transition}],
        "sendBulkNotification": notify,
    });
    if dry_run {
        return Ok(json!({"dryRun": true, "request": body}));
    }
    let v = c.post("/rest/api/3/bulk/issues/transition", &body)?;
    let task = v["taskId"]
        .as_str()
        .ok_or_else(|| anyhow!("Jira didn't return a task for the transition"))?;
    Ok(json!({
        "taskId": task,
        "requested": ids.len(),
        "statusPath": format!("/rest/api/3/bulk/queue/{}", enc(task)),
    }))
}

pub fn archive(c: &Client, ids: &[String], on: bool) -> Result<Value> {
    let ids = unique(ids, ACTION_MAX)?;
    let action = if on { "archive" } else { "unarchive" };
    let v = c.put(
        &format!("/rest/api/3/issue/{action}"),
        &json!({"issueIdsOrKeys": ids}),
    )?;
    Ok(json!({
        "requested": ids.len(),
        "numberOfIssuesUpdated": v["numberOfIssuesUpdated"],
        "errors": v["errors"],
    }))
}

// --- properties --------------------------------------------------------------

fn properties_path(issue: &str) -> Result<String> {
    if issue.trim().is_empty() {
        return Err(usage("the work item ID is empty"));
    }
    Ok(format!("/rest/api/3/issue/{}/properties", enc(issue)))
}

fn property_path(issue: &str, key: &str) -> Result<String> {
    if key.trim().is_empty() || key.chars().count() > 255 {
        return Err(usage("a property key is 1 to 255 characters"));
    }
    Ok(format!("{}/{}", properties_path(issue)?, enc(key)))
}

pub fn properties(c: &Client, issue: &str) -> Result<Value> {
    c.get(&properties_path(issue)?, &[])
}

pub fn property(c: &Client, issue: &str, key: &str) -> Result<Value> {
    c.get(&property_path(issue, key)?, &[])
}

pub fn property_set(c: &Client, issue: &str, key: &str, value: &Value) -> Result<Value> {
    c.put(&property_path(issue, key)?, value)?;
    Ok(json!({"issueId": issue, "key": key, "value": value}))
}

pub fn property_delete(c: &Client, issue: &str, key: &str) -> Result<Value> {
    c.delete(&property_path(issue, key)?, &[])?;
    Ok(json!({"issueId": issue, "key": key, "deleted": true}))
}

// --- users -------------------------------------------------------------------

fn account_id(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix("ari:cloud:identity::user/").unwrap_or(s)
}

/// One user as a profile; the caller without an account.
pub fn user(c: &Client, account: Option<&str>) -> Result<Value> {
    let v = match account.map(account_id) {
        Some("") => return Err(usage("the account ID is empty")),
        None | Some("me") => myself(c)?,
        Some(id) => c.get("/rest/api/3/user", &[("accountId", id)])?,
    };
    Ok(profile(&v))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Account(String),
    Email(String),
}

/// `{items: [{input, found, user|reason}]}`. An email only matches
/// the one user who shows exactly that address: search matches prefixes, and
/// a hidden address can't be confirmed.
pub fn lookup(c: &Client, who: &[Who]) -> Result<Value> {
    if who.is_empty() {
        return Err(usage("pass at least one account ID or email"));
    }
    if who
        .iter()
        .any(|w| matches!(w, Who::Account(s) | Who::Email(s) if account_id(s).is_empty()))
    {
        return Err(usage("an identifier is empty"));
    }
    let ids: Vec<&str> = who
        .iter()
        .filter_map(|w| match w {
            Who::Account(id) => Some(account_id(id)),
            Who::Email(_) => None,
        })
        .collect();
    let mut accounts = Map::new();
    for chunk in ids.chunks(USER_BATCH) {
        let max = USER_BATCH.to_string();
        let mut q = vec![("maxResults", max.as_str())];
        q.extend(chunk.iter().map(|id| ("accountId", *id)));
        let v = c.get("/rest/api/3/user/bulk", &q)?;
        for u in v["values"].as_array().into_iter().flatten() {
            if let Some(id) = u["accountId"].as_str() {
                accounts.insert(id.to_string(), u.clone());
            }
        }
    }
    let mut items = Vec::new();
    for w in who {
        let (input, found) = match w {
            Who::Account(id) => (
                id,
                accounts
                    .get(account_id(id))
                    .cloned()
                    .ok_or("no user has this account ID"),
            ),
            Who::Email(email) => {
                let users = fetch_users(c, email, None)?;
                let exact: Vec<&Value> = users
                    .iter()
                    .filter(|u| {
                        u["emailAddress"]
                            .as_str()
                            .is_some_and(|e| e.eq_ignore_ascii_case(email.trim()))
                    })
                    .collect();
                let found = match exact.as_slice() {
                    [one] => Ok((*one).clone()),
                    [] if users.is_empty() => Err("no user matches this email"),
                    [] => Err("no visible email address matches exactly"),
                    _ => Err("several users have this email"),
                };
                (email, found)
            }
        };
        items.push(match found {
            Ok(u) => json!({"input": input, "found": true, "user": user_row(&u)}),
            Err(reason) => json!({"input": input, "found": false, "reason": reason}),
        });
    }
    let mut out = json!({"items": items});
    strip_media(&mut out);
    Ok(out)
}

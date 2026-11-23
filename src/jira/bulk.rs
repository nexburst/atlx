//! Many issues at once (bulk create, archive and restore) and issue entity
//! properties.

use super::{browse_url, enc};
use crate::client::{ApiError, Client, ErrorKind};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashSet;

/// `issue/bulk` creates at most 50 issues.
const CREATE_MAX: usize = 50;
/// Archive and restore take at most 1000 issues.
const ACTION_MAX: usize = 1000;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// The IDs without repeats, in their order.
fn unique(ids: &[String], max: usize) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids.iter().map(|s| s.trim()) {
        if id.is_empty() {
            return Err(usage("an issue key is empty"));
        }
        if seen.insert(id.to_ascii_uppercase()) {
            out.push(id.to_string());
        }
    }
    if out.is_empty() || out.len() > max {
        return Err(usage(format!("pass 1 to {max} issues (got {})", out.len())));
    }
    Ok(out)
}

/// What a multi-item write did: the items it changed and, per failed input,
/// the error.
pub type Outcome = (Vec<Value>, Vec<(String, ApiError)>);

/// The `issueUpdates` in an `{"issueUpdates": [...]}` object or a bare array
/// of `{"fields": {...}}` items.
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
            "pass 1 to {CREATE_MAX} issues (got {})",
            items.len()
        )));
    }
    if let Some(i) = items.iter().position(|i| !i["fields"].is_object()) {
        return Err(usage(format!("item {} needs a \"fields\" object", i + 1)));
    }
    Ok(items)
}

/// Create up to 50 issues in one request. Some can fail while the others
/// are created; each failure is named `#N` (its 1-based position).
pub fn create(c: &Client, items: &[Value]) -> Result<Outcome> {
    let v = c.post("/rest/api/3/issue/bulk", &json!({"issueUpdates": items}))?;
    let created = v["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let key = i["key"].as_str().unwrap_or_default();
            json!({"id": i["id"], "key": key, "url": browse_url(c, key)})
        })
        .collect();
    let failed = v["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let n = e["failedElementNumber"].as_u64().map_or(0, |n| n + 1);
            let errs = &e["elementErrors"];
            let mut msgs: Vec<String> = errs["errorMessages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str().map(String::from))
                .collect();
            if let Some(m) = errs["errors"].as_object() {
                msgs.extend(
                    m.iter()
                        .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default())),
                );
            }
            let status = e["status"].as_u64().map(|s| s as u16);
            let kind = status.map_or(ErrorKind::Validation, ErrorKind::from_status);
            let mut err = ApiError::new(kind, msgs.join("; "));
            err.status = status;
            if errs["errors"].is_object() {
                err.fields = Some(errs["errors"].clone());
            }
            (format!("#{n}"), err)
        })
        .collect();
    Ok((created, failed))
}

/// Archive (`on`) or restore issues. Jira reports failures grouped by
/// reason; each failed key gets its group's message.
pub fn archive(c: &Client, ids: &[String], on: bool) -> Result<Outcome> {
    let ids = unique(ids, ACTION_MAX)?;
    let action = if on { "archive" } else { "unarchive" };
    let v = c.put(
        &format!("/rest/api/3/issue/{action}"),
        &json!({"issueIdsOrKeys": ids}),
    )?;
    let mut failed: Vec<(String, ApiError)> = Vec::new();
    for (reason, group) in v["errors"].as_object().into_iter().flatten() {
        let message = group["message"].as_str().unwrap_or(reason).to_string();
        for key in group["issueIdsOrKeys"].as_array().into_iter().flatten() {
            if let Some(k) = key.as_str() {
                failed.push((
                    k.to_string(),
                    ApiError::new(ErrorKind::Validation, message.clone()),
                ));
            }
        }
    }
    let done = ids
        .iter()
        .filter(|k| !failed.iter().any(|(f, _)| f.eq_ignore_ascii_case(k)))
        .map(|k| json!({"key": k, "archived": on}))
        .collect();
    Ok((done, failed))
}

// --- properties --------------------------------------------------------------

fn properties_path(issue: &str) -> Result<String> {
    if issue.trim().is_empty() {
        return Err(usage("the issue key is empty"));
    }
    Ok(format!("/rest/api/3/issue/{}/properties", enc(issue)))
}

fn property_path(issue: &str, key: &str) -> Result<String> {
    if key.trim().is_empty() || key.chars().count() > 255 {
        return Err(usage("a property key is 1 to 255 characters"));
    }
    Ok(format!("{}/{}", properties_path(issue)?, enc(key)))
}

/// The property keys on an issue.
pub fn properties(c: &Client, issue: &str) -> Result<Vec<Value>> {
    let v = c.get(&properties_path(issue)?, &[])?;
    Ok(v["keys"].as_array().cloned().unwrap_or_default())
}

pub fn property(c: &Client, issue: &str, key: &str) -> Result<Value> {
    c.get(&property_path(issue, key)?, &[])
}

pub fn property_set(c: &Client, issue: &str, key: &str, value: &Value) -> Result<Value> {
    c.put(&property_path(issue, key)?, value)?;
    Ok(json!({"key": key, "value": value}))
}

pub fn property_delete(c: &Client, issue: &str, key: &str) -> Result<Value> {
    c.delete(&property_path(issue, key)?, &[])?;
    Ok(json!({"id": key, "issue": issue, "deleted": true}))
}

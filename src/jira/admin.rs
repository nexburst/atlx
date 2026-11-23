//! Space (project) lifecycle and lookups, versions, components and filters.

use super::{Listed, Offsets, enc, offset_list, resolve_account, strip_media, trailing_number};
use crate::client::{ApiError, Client};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn clean(mut v: Value) -> Value {
    strip_media(&mut v);
    v
}

/// The numeric ID in a number, or in an ARI or URL of that kind of thing.
/// Writes use this, so a board's URL must not pass for a component's ID.
pub fn numeric_id(what: &str, s: &str) -> Result<String> {
    let s = s.trim();
    let shaped = if s.starts_with("ari:") {
        s.contains(&format!("{what}/"))
    } else {
        (s.starts_with("https://") || s.starts_with("http://"))
            && [format!("/{what}/"), format!("/{what}s/")]
                .iter()
                .any(|m| s.contains(m))
    };
    let id = if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        Some(s.to_string())
    } else if shaped {
        trailing_number(s)
    } else {
        None
    };
    id.ok_or_else(|| usage(format!("'{s}' is not a {what} ID")))
}

/// A project key or ID, given plainly, as a project ARI or as a `/projects/`
/// URL. Stricter than the read commands: an issue key or URL names a
/// project too, and must never be enough to delete or archive one.
pub fn exact_project(s: &str) -> Result<String> {
    let s = s.trim();
    let candidate = if s.starts_with("ari:") {
        s.rsplit_once(":project/").map(|(_, id)| id)
    } else if s.starts_with("https://") || s.starts_with("http://") {
        s.split_once("/projects/")
            .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
    } else {
        Some(s)
    };
    candidate
        .filter(|k| {
            let numeric = !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit());
            let key = k.starts_with(|c: char| c.is_ascii_alphabetic())
                && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            numeric || key
        })
        .map(String::from)
        .ok_or_else(|| {
            usage(format!(
                "'{s}' is not a project key, ID, ARI or /projects/ URL"
            ))
        })
}

fn non_empty(body: &Map<String, Value>) -> Result<()> {
    if body.is_empty() {
        return Err(usage("nothing to change: pass at least one field"));
    }
    Ok(())
}

// --- projects ------------------------------------------------------------------

fn project_path(project: &str) -> Result<String> {
    Ok(format!(
        "/rest/api/3/project/{}",
        enc(&exact_project(project)?)
    ))
}

pub fn project_create(c: &Client, body: Map<String, Value>) -> Result<Value> {
    Ok(clean(c.post("/rest/api/3/project", &Value::Object(body))?))
}

pub fn project_update(
    c: &Client,
    project: &str,
    body: Map<String, Value>,
    expand: Option<&str>,
) -> Result<Value> {
    non_empty(&body)?;
    let q: Vec<(&str, &str)> = expand.iter().map(|e| ("expand", *e)).collect();
    Ok(clean(c.request(
        "PUT",
        &project_path(project)?,
        &q,
        Some(&Value::Object(body)),
    )?))
}

/// Moves the project to the trash, from where it can be restored.
pub fn project_delete(c: &Client, project: &str) -> Result<Value> {
    c.delete(&project_path(project)?, &[("enableUndo", "true")])?;
    Ok(json!({"id": exact_project(project)?, "deleted": true}))
}

pub fn project_archive(c: &Client, project: &str) -> Result<Value> {
    c.request(
        "POST",
        &format!("{}/archive", project_path(project)?),
        &[],
        None,
    )?;
    Ok(json!({"key": exact_project(project)?, "archived": true}))
}

pub fn project_restore(c: &Client, project: &str) -> Result<Value> {
    Ok(clean(c.request(
        "POST",
        &format!("{}/restore", project_path(project)?),
        &[],
        None,
    )?))
}

// --- versions ----------------------------------------------------------------

fn version_path(id: &str) -> Result<String> {
    Ok(format!(
        "/rest/api/3/version/{}",
        numeric_id("version", id)?
    ))
}

#[derive(Debug, Default)]
pub struct VersionQuery<'a> {
    pub search: Option<&'a str>,
    /// Comma-separated `released`, `unreleased`, `archived`.
    pub status: Option<&'a str>,
    pub order_by: Option<&'a str>,
}

pub fn versions(
    c: &Client,
    project: &str,
    o: &VersionQuery,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let q: Vec<(&str, &str)> = [
        ("query", o.search),
        ("status", o.status),
        ("orderBy", o.order_by),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .collect();
    offset_list(
        c,
        &format!("{}/version", project_path(project)?),
        &q,
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

pub fn version_create(c: &Client, project: &str, mut body: Map<String, Value>) -> Result<Value> {
    let (_, id) = super::project_key_id(c, &exact_project(project)?)?;
    let id: u64 = id
        .parse()
        .map_err(|_| anyhow!("project ID '{id}' is not a number"))?;
    body.insert("projectId".into(), id.into());
    c.post("/rest/api/3/version", &Value::Object(body))
}

pub fn version_get(c: &Client, id: &str) -> Result<Value> {
    c.get(&version_path(id)?, &[("expand", "issuesstatus")])
}

pub fn version_update(c: &Client, id: &str, body: Map<String, Value>) -> Result<Value> {
    non_empty(&body)?;
    c.put(&version_path(id)?, &Value::Object(body))
}

/// Uses `removeAndSwap`; the plain DELETE is deprecated.
pub fn version_delete(
    c: &Client,
    id: &str,
    fix_to: Option<&str>,
    affected_to: Option<&str>,
) -> Result<Value> {
    let path = version_path(id)?;
    let this = numeric_id("version", id)?;
    let mut body = Map::new();
    for (key, to) in [
        ("moveFixIssuesTo", fix_to),
        ("moveAffectedIssuesTo", affected_to),
    ] {
        if let Some(to) = to {
            let to = numeric_id("version", to)?;
            if to == this {
                return Err(usage(
                    "a version's issues can't move to the version being deleted",
                ));
            }
            let to: u64 = to
                .parse()
                .map_err(|_| usage("that version ID is too large"))?;
            body.insert(key.into(), to.into());
        }
    }
    c.post(&format!("{path}/removeAndSwap"), &Value::Object(body))?;
    Ok(json!({"id": this, "deleted": true}))
}

/// Deletes `id`, moving its fix versions to `into`.
pub fn version_merge(c: &Client, id: &str, into: &str) -> Result<Value> {
    let (from, into) = (numeric_id("version", id)?, numeric_id("version", into)?);
    if from == into {
        return Err(usage("a version can't be merged into itself"));
    }
    c.request(
        "PUT",
        &format!("/rest/api/3/version/{from}/mergeto/{into}"),
        &[],
        None,
    )?;
    Ok(json!({"id": from, "issue": into, "deleted": true}))
}

pub fn version_move(
    c: &Client,
    id: &str,
    after: Option<&str>,
    position: Option<&str>,
) -> Result<Value> {
    let body = match (after, position) {
        (Some(a), None) if a.starts_with("https://") || a.starts_with("http://") => {
            json!({"after": a})
        }
        (Some(a), None) => json!({"after": format!("{}{}", c.base(), version_path(a)?)}),
        (None, Some(p)) => json!({"position": p}),
        _ => return Err(usage("pass either --after or --position")),
    };
    c.post(&format!("{}/move", version_path(id)?), &body)
}

// --- components --------------------------------------------------------------

fn component_path(id: &str) -> Result<String> {
    Ok(format!(
        "/rest/api/3/component/{}",
        numeric_id("component", id)?
    ))
}

pub fn components(c: &Client, project: &str) -> Result<Vec<Value>> {
    let v = c.get(&format!("{}/components", project_path(project)?), &[])?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

pub fn component_create(c: &Client, project: &str, mut body: Map<String, Value>) -> Result<Value> {
    // The endpoint takes a key, never an ID.
    let (key, _) = super::project_key_id(c, &exact_project(project)?)?;
    body.insert("project".into(), key.into());
    Ok(clean(
        c.post("/rest/api/3/component", &Value::Object(body))?,
    ))
}

pub fn component_get(c: &Client, id: &str) -> Result<Value> {
    Ok(clean(c.get(&component_path(id)?, &[])?))
}

pub fn component_update(c: &Client, id: &str, body: Map<String, Value>) -> Result<Value> {
    non_empty(&body)?;
    Ok(clean(c.put(&component_path(id)?, &Value::Object(body))?))
}

pub fn component_delete(c: &Client, id: &str, move_to: Option<&str>) -> Result<Value> {
    let path = component_path(id)?;
    let this = numeric_id("component", id)?;
    let to = move_to.map(|t| numeric_id("component", t)).transpose()?;
    if to.as_deref() == Some(this.as_str()) {
        return Err(usage(
            "a component's issues can't move to the component being deleted",
        ));
    }
    let q: Vec<(&str, &str)> = to.iter().map(|t| ("moveIssuesTo", t.as_str())).collect();
    c.delete(&path, &q)?;
    Ok(json!({"id": this, "deleted": true}))
}

// --- filters -----------------------------------------------------------------

/// A filter ID from an ID, ARI or filter URL, else the one filter with
/// exactly that name (ignoring case).
pub fn resolve_filter(c: &Client, r: &str) -> Result<String> {
    let r = r.trim();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(r) {
        return Ok(r.to_string());
    }
    if r.starts_with("ari:") {
        return numeric_id("filter", r);
    }
    if r.starts_with("https://") || r.starts_with("http://") {
        for marker in ["filter=", "/filter/", "/filters/"] {
            if let Some((_, rest)) = r.split_once(marker) {
                let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if digits(&id) {
                    return Ok(id);
                }
            }
        }
        return Err(usage(format!("'{r}' is not a filter URL")));
    }
    if r.is_empty() {
        return Err(usage("pass a filter ID or name"));
    }
    let (found, _) = offset_list(
        c,
        "/rest/api/3/filter/search",
        &[("filterName", r)],
        &[],
        Offsets::values(),
        crate::paging::HARD_CAP,
        None,
    )?;
    let ids: Vec<String> = found
        .iter()
        .filter(|f| {
            f["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(r))
        })
        .filter_map(|f| match &f["id"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect();
    match ids.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(usage(format!("no filter is named '{r}'"))),
        _ => Err(usage(format!(
            "several filters are named '{r}' ({}); pass the ID",
            ids.join(", ")
        ))),
    }
}

fn filter_path(id: &str) -> String {
    format!("/rest/api/3/filter/{}", enc(id))
}

#[derive(Debug, Default)]
pub struct FilterQuery<'a> {
    pub name: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub owner_account: Option<&'a str>,
    pub group: Option<&'a str>,
    pub group_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub ids: &'a [String],
    pub order_by: Option<&'a str>,
    pub expand: Option<&'a str>,
    pub override_shares: Option<bool>,
}

pub fn filters(c: &Client, o: &FilterQuery, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    let owner = match (o.owner_account, o.owner) {
        (Some(id), _) => Some(id.to_string()),
        (None, Some(who)) => Some(resolve_account(c, who)?),
        (None, None) => None,
    };
    let ids = o
        .ids
        .iter()
        .map(|i| numeric_id("filter", i))
        .collect::<Result<Vec<_>>>()?;
    let flag = o.override_shares.map(|b| b.to_string());
    let mut q: Vec<(&str, &str)> = [
        ("filterName", o.name),
        ("accountId", owner.as_deref()),
        ("groupname", o.group),
        ("groupId", o.group_id),
        ("projectId", o.project_id),
        ("orderBy", o.order_by),
        (
            "expand",
            Some(o.expand.unwrap_or("description,jql,owner,favourite")),
        ),
        ("overrideSharePermissions", flag.as_deref()),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .collect();
    q.extend(ids.iter().map(|i| ("id", i.as_str())));
    offset_list(
        c,
        "/rest/api/3/filter/search",
        &q,
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

pub fn favourite_filters(c: &Client) -> Result<Vec<Value>> {
    let v = c.get("/rest/api/3/filter/favourite", &[])?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

pub fn filter_get(c: &Client, id: &str, expand: Option<&str>) -> Result<Value> {
    let q: Vec<(&str, &str)> = expand.iter().map(|e| ("expand", *e)).collect();
    Ok(clean(c.get(&filter_path(id), &q)?))
}

pub fn filter_create(
    c: &Client,
    body: Map<String, Value>,
    expand: Option<&str>,
    override_shares: Option<bool>,
) -> Result<Value> {
    let flag = override_shares.map(|b| b.to_string());
    let q: Vec<(&str, &str)> = [
        ("expand", expand),
        ("overrideSharePermissions", flag.as_deref()),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .collect();
    Ok(clean(c.request(
        "POST",
        "/rest/api/3/filter",
        &q,
        Some(&Value::Object(body)),
    )?))
}

/// PUT replaces the filter, so what isn't changed is sent back as it was.
/// Being a favourite has its own endpoint.
pub fn filter_update(
    c: &Client,
    id: &str,
    changes: Map<String, Value>,
    favourite: Option<bool>,
) -> Result<Value> {
    if changes.is_empty() && favourite.is_none() {
        return Err(usage("nothing to change: pass at least one field"));
    }
    let mut out = Value::Null;
    if !changes.is_empty() {
        let now = c.get(&filter_path(id), &[])?;
        if !now["name"].is_string() {
            return Err(anyhow!(
                "filter {id} came back without its name; not changing it"
            ));
        }
        let mut body = Map::new();
        for key in [
            "name",
            "jql",
            "description",
            "sharePermissions",
            "editPermissions",
        ] {
            if let Some(v) = now.get(key).filter(|v| !v.is_null()) {
                body.insert(key.into(), v.clone());
            }
        }
        body.extend(changes);
        out = c.put(&filter_path(id), &Value::Object(body))?;
    }
    if let Some(on) = favourite {
        out = filter_favourite(c, id, on, None)?;
    }
    Ok(clean(out))
}

pub fn filter_favourite(c: &Client, id: &str, on: bool, expand: Option<&str>) -> Result<Value> {
    let q: Vec<(&str, &str)> = expand.iter().map(|e| ("expand", *e)).collect();
    let method = if on { "PUT" } else { "DELETE" };
    Ok(clean(c.request(
        method,
        &format!("{}/favourite", filter_path(id)),
        &q,
        None,
    )?))
}

pub fn filter_delete(c: &Client, id: &str, override_shares: Option<bool>) -> Result<Value> {
    let flag = override_shares.map(|b| b.to_string());
    let q: Vec<(&str, &str)> = flag
        .iter()
        .map(|f| ("overrideSharePermissions", f.as_str()))
        .collect();
    c.delete(&filter_path(id), &q)?;
    Ok(json!({"id": id, "deleted": true}))
}

pub fn filter_owner(c: &Client, id: &str, account: &str) -> Result<Value> {
    c.put(
        &format!("{}/owner", filter_path(id)),
        &json!({"accountId": account}),
    )?;
    Ok(json!({"id": id, "accountId": account}))
}

pub fn filter_columns(c: &Client, id: &str) -> Result<Vec<Value>> {
    let v = c.get(&format!("{}/columns", filter_path(id)), &[])?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Sets the columns; returns them as `column list` shows them (without
/// labels, which Jira does not echo).
pub fn filter_columns_set(c: &Client, id: &str, columns: &[String]) -> Result<Vec<Value>> {
    c.put(
        &format!("{}/columns", filter_path(id)),
        &json!({"columns": columns}),
    )?;
    Ok(columns
        .iter()
        .map(|c| json!({"label": null, "value": c}))
        .collect())
}

pub fn filter_columns_reset(c: &Client, id: &str) -> Result<Value> {
    c.delete(&format!("{}/columns", filter_path(id)), &[])?;
    Ok(json!({"id": id, "deleted": true}))
}

pub fn filter_shares(c: &Client, id: &str) -> Result<Vec<Value>> {
    let v = c.get(&format!("{}/permission", filter_path(id)), &[])?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

pub fn filter_share_add(c: &Client, id: &str, permission: &Value) -> Result<Value> {
    if !permission["type"].is_string() {
        return Err(usage("the share permission needs a \"type\""));
    }
    let mut permission = permission.clone();
    // Jira prints `loggedin`, but this endpoint only takes `authenticated`.
    if permission["type"] == "loggedin" {
        permission["type"] = "authenticated".into();
    }
    Ok(clean(c.post(
        &format!("{}/permission", filter_path(id)),
        &permission,
    )?))
}

pub fn filter_share_remove(c: &Client, id: &str, permission: &str) -> Result<Value> {
    let p = numeric_id("share permission", permission)?;
    c.delete(&format!("{}/permission/{p}", filter_path(id)), &[])?;
    Ok(json!({"id": p, "filter": id, "deleted": true}))
}

/// A filter's subscriptions (Jira returns them with the filter).
pub fn filter_subscriptions(c: &Client, id: &str) -> Result<Vec<Value>> {
    let v = c.get(&filter_path(id), &[("expand", "subscriptions")])?;
    Ok(v["subscriptions"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// `issue list --filter`: the filter's JQL.
pub fn filter_jql(c: &Client, r: &str) -> Result<String> {
    let id = resolve_filter(c, r)?;
    c.get(&filter_path(&id), &[])?["jql"]
        .as_str()
        .filter(|j| !j.trim().is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("filter {id} has no JQL"))
}

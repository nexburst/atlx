//! Dashboards, their gadgets and gadget item properties.

use super::admin::numeric_id;
use super::{enc, strip_media};
use crate::client::{ApiError, Client};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

const BASE: &str = "/rest/api/3/dashboard";
/// The most dashboards one bulk edit takes.
const EDIT_MAX: usize = 100;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn clean(mut v: Value) -> Value {
    strip_media(&mut v);
    v
}

fn path(id: &str) -> Result<String> {
    Ok(format!("{BASE}/{}", numeric_id("dashboard", id)?))
}

#[derive(Debug, Default)]
pub struct Search<'a> {
    pub name: Option<&'a str>,
    pub account_id: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub group_name: Option<&'a str>,
    pub group_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub order_by: Option<&'a str>,
    pub expand: Option<&'a str>,
    pub start: usize,
    pub limit: usize,
}

/// One REST page, or as many as `limit` needs, in REST's page shape.
pub fn query(c: &Client, o: &Search) -> Result<Value> {
    if o.limit == 0 {
        return Err(usage("--first must be at least 1"));
    }
    let mut all: Vec<Value> = Vec::new();
    let (mut total, mut last) = (None, true);
    while all.len() < o.limit {
        let start = (o.start + all.len()).to_string();
        let size = (o.limit - all.len()).min(100).to_string();
        let mut q = vec![("startAt", start.as_str()), ("maxResults", size.as_str())];
        for (k, v) in [
            ("dashboardName", o.name),
            ("accountId", o.account_id),
            ("owner", o.owner),
            ("groupname", o.group_name),
            ("groupId", o.group_id),
            ("projectId", o.project_id),
            ("orderBy", o.order_by),
            ("expand", o.expand),
        ] {
            if let Some(v) = v {
                q.push((k, v));
            }
        }
        let v = c.get(&format!("{BASE}/search"), &q)?;
        let page = v["values"]
            .as_array()
            .ok_or_else(|| anyhow!("Jira returned no dashboard list"))?;
        all.extend(page.iter().cloned());
        total = v["total"].as_u64();
        last = page.is_empty() || v["isLast"].as_bool() != Some(false);
        if last {
            break;
        }
    }
    all.truncate(o.limit);
    Ok(clean(json!({
        "values": all,
        "startAt": o.start,
        "maxResults": o.limit,
        "total": total,
        "isLast": last,
    })))
}

pub fn get(c: &Client, id: &str) -> Result<Value> {
    Ok(clean(c.get(&path(id)?, &[])?))
}

/// What a create, update or copy sets; `None` is left to the default.
#[derive(Debug, Default)]
pub struct Edit<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub share: Option<Value>,
    pub edit: Option<Value>,
    pub admin: Option<bool>,
}

impl Edit<'_> {
    fn empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.share.is_none()
            && self.edit.is_none()
    }

    fn query(&self) -> Vec<(&'static str, &'static str)> {
        match self.admin {
            Some(true) => vec![("extendAdminPermissions", "true")],
            Some(false) => vec![("extendAdminPermissions", "false")],
            None => Vec::new(),
        }
    }
}

/// Permissions as a write takes them. `loggedin` is how reads print
/// `authenticated`, and a share with a space the caller can't see comes back
/// as `project-unknown`, which can't be written.
fn permissions(what: &str, v: Value) -> Result<Value> {
    let Value::Array(mut items) = v else {
        return Err(usage(format!("{what} must be a JSON array")));
    };
    for p in &mut items {
        match p["type"].as_str() {
            Some("project-unknown") => {
                return Err(usage(format!(
                    "{what} include a space you can't see, so writing them back would lose it"
                )));
            }
            Some("loggedin") => p["type"] = "authenticated".into(),
            Some(_) => {}
            None => return Err(usage(format!("each of the {what} needs a \"type\""))),
        }
    }
    Ok(Value::Array(items))
}

fn named(name: Option<&str>) -> Result<&str> {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| usage("pass --name"))
}

/// The body of a create or copy: private unless permissions are given.
fn new_body(e: &Edit, description: Option<&str>) -> Result<Value> {
    let list = |v: &Option<Value>| v.clone().unwrap_or_else(|| json!([]));
    let mut body = json!({
        "name": named(e.name)?,
        "sharePermissions": permissions("share permissions", list(&e.share))?,
        "editPermissions": permissions("edit permissions", list(&e.edit))?,
    });
    if let Some(d) = description {
        body["description"] = d.into();
    }
    Ok(body)
}

pub fn create(c: &Client, e: &Edit) -> Result<Value> {
    let body = new_body(e, e.description)?;
    Ok(clean(c.request("POST", BASE, &e.query(), Some(&body))?))
}

pub fn copy(c: &Client, id: &str, e: &Edit) -> Result<Value> {
    let url = format!("{}/copy", path(id)?);
    let body = new_body(e, e.description)?;
    Ok(clean(c.request("POST", &url, &e.query(), Some(&body))?))
}

/// The PUT replaces everything, so what isn't given is read and sent back.
/// A part that can't be read stops the update: it would be cleared.
pub fn update(c: &Client, id: &str, e: &Edit) -> Result<Value> {
    if e.empty() {
        return Err(usage("nothing to update"));
    }
    let path = path(id)?;
    let now = c.get(&path, &[])?;
    let kept = |given: &Option<Value>, key: &str, what: &str| -> Result<Value> {
        let v = match given {
            Some(v) => v.clone(),
            None if now[key].is_array() => now[key].clone(),
            None => {
                return Err(anyhow!(
                    "Jira didn't show the current {what}, so the update would clear them; pass them"
                ));
            }
        };
        permissions(what, v)
    };
    let mut body = json!({
        "name": named(e.name.or(now["name"].as_str()))?,
        "sharePermissions": kept(&e.share, "sharePermissions", "share permissions")?,
        "editPermissions": kept(&e.edit, "editPermissions", "edit permissions")?,
    });
    if let Some(d) = e.description.or(now["description"].as_str()) {
        body["description"] = d.into();
    }
    Ok(clean(c.request("PUT", &path, &e.query(), Some(&body))?))
}

pub fn delete(c: &Client, id: &str) -> Result<Value> {
    let id = numeric_id("dashboard", id)?;
    c.delete(&format!("{BASE}/{id}"), &[])?;
    Ok(json!({"id": id, "deleted": true}))
}

pub fn bulk_edit(
    c: &Client,
    ids: &[String],
    action: &str,
    owner: Option<Value>,
    permission: Option<Value>,
    admin: Option<bool>,
) -> Result<Value> {
    let mut entities: Vec<u64> = Vec::new();
    for id in ids {
        let id: u64 = numeric_id("dashboard", id)?
            .parse()
            .map_err(|_| usage(format!("'{id}' is out of range")))?;
        if !entities.contains(&id) {
            entities.push(id);
        }
    }
    if entities.is_empty() || entities.len() > EDIT_MAX {
        return Err(usage(format!("pass 1 to {EDIT_MAX} dashboards")));
    }
    let (key, details) = match (action, owner, permission) {
        ("changeOwner", Some(o), None) => ("changeOwnerDetails", o),
        ("changeOwner", ..) => {
            return Err(usage(
                "changeOwner takes --change-owner-details-json and no permission details",
            ));
        }
        (_, None, Some(p)) => ("permissionDetails", p),
        _ => {
            return Err(usage(
                "this action takes --permission-details-json and no owner details",
            ));
        }
    };
    if !details.is_object() {
        return Err(usage("the details must be a JSON object"));
    }
    let mut body = json!({"action": action, "entityIds": entities});
    body[key] = details;
    if let Some(admin) = admin {
        body["extendAdminPermissions"] = admin.into();
    }
    Ok(clean(c.put(&format!("{BASE}/bulk/edit"), &body)?))
}

// --- gadgets -----------------------------------------------------------------

pub fn catalog(c: &Client) -> Result<Value> {
    c.get(&format!("{BASE}/gadgets"), &[])
}

pub fn gadgets(
    c: &Client,
    id: &str,
    module_keys: &[String],
    uris: &[String],
    gadget_ids: &[String],
) -> Result<Value> {
    let url = format!("{}/gadget", path(id)?);
    let mut q: Vec<(&str, &str)> = Vec::new();
    q.extend(module_keys.iter().map(|v| ("moduleKey", v.as_str())));
    q.extend(uris.iter().map(|v| ("uri", v.as_str())));
    q.extend(gadget_ids.iter().map(|v| ("gadgetId", v.as_str())));
    c.get(&url, &q)
}

#[derive(Debug, Default)]
pub struct Gadget<'a> {
    pub json: Option<Value>,
    pub title: Option<&'a str>,
    pub color: Option<&'a str>,
    pub row: Option<u32>,
    pub column: Option<u32>,
}

impl Gadget<'_> {
    /// `--gadget-json` with the flags laid over it.
    fn body(&self) -> Result<Map<String, Value>> {
        let mut m = match &self.json {
            None => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(usage("--gadget-json must be a JSON object")),
        };
        for (k, v) in [("title", self.title), ("color", self.color)] {
            if let Some(v) = v {
                m.insert(k.into(), v.into());
            }
        }
        match (self.row, self.column) {
            (Some(row), Some(column)) => {
                m.insert("position".into(), json!({"row": row, "column": column}));
            }
            (None, None) => {}
            _ => return Err(usage("--row and --column go together")),
        }
        Ok(m)
    }
}

pub fn gadget_add(
    c: &Client,
    id: &str,
    g: &Gadget,
    module_key: Option<&str>,
    uri: Option<&str>,
    skip_validation: Option<bool>,
) -> Result<Value> {
    let url = format!("{}/gadget", path(id)?);
    let mut body = g.body()?;
    for (k, v) in [("moduleKey", module_key), ("uri", uri)] {
        if let Some(v) = v {
            body.insert(k.into(), v.into());
        }
    }
    if body.contains_key("moduleKey") == body.contains_key("uri") {
        return Err(usage(
            "a gadget needs exactly one of a module key and a URI",
        ));
    }
    if let Some(skip) = skip_validation {
        body.insert("ignoreUriAndModuleKeyValidation".into(), skip.into());
    }
    c.post(&url, &Value::Object(body))
}

fn gadget_path(id: &str, gadget: &str) -> Result<String> {
    Ok(format!(
        "{}/gadget/{}",
        path(id)?,
        numeric_id("gadget", gadget)?
    ))
}

pub fn gadget_update(c: &Client, id: &str, gadget: &str, g: &Gadget) -> Result<Value> {
    let url = gadget_path(id, gadget)?;
    let body = g.body()?;
    if body.is_empty() {
        return Err(usage("nothing to update"));
    }
    c.put(&url, &Value::Object(body))?;
    Ok(json!({"dashboardId": id, "gadgetId": gadget, "updated": true}))
}

pub fn gadget_delete(c: &Client, id: &str, gadget: &str) -> Result<Value> {
    c.delete(&gadget_path(id, gadget)?, &[])?;
    Ok(json!({"dashboardId": id, "gadgetId": gadget, "deleted": true}))
}

// --- item properties ---------------------------------------------------------

fn properties_path(id: &str, item: &str) -> Result<String> {
    if item.trim().is_empty() {
        return Err(usage("the item ID is empty"));
    }
    Ok(format!("{}/items/{}/properties", path(id)?, enc(item)))
}

fn property_path(id: &str, item: &str, key: &str) -> Result<String> {
    if key.trim().is_empty() {
        return Err(usage("the property key is empty"));
    }
    Ok(format!("{}/{}", properties_path(id, item)?, enc(key)))
}

pub fn properties(c: &Client, id: &str, item: &str) -> Result<Value> {
    c.get(&properties_path(id, item)?, &[])
}

pub fn property(c: &Client, id: &str, item: &str, key: &str) -> Result<Value> {
    c.get(&property_path(id, item, key)?, &[])
}

pub fn property_set(c: &Client, id: &str, item: &str, key: &str, value: &Value) -> Result<Value> {
    c.put(&property_path(id, item, key)?, value)?;
    Ok(json!({"dashboardId": id, "itemId": item, "key": key, "value": value}))
}

pub fn property_delete(c: &Client, id: &str, item: &str, key: &str) -> Result<Value> {
    c.delete(&property_path(id, item, key)?, &[])?;
    Ok(json!({"dashboardId": id, "itemId": item, "key": key, "deleted": true}))
}

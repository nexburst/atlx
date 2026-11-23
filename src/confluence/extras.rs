//! Tasks, content states, watches, templates and space permissions.

use super::lifecycle::exact_space;
use super::{ContentType, enc, listing, rendered_body, space_id, try_content};
use crate::client::{ApiError, Client};
use crate::markup::BodyFormat;
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Map, Value, json};

/// v1 lists of templates are read this many at a time.
const TEMPLATE_PAGE: usize = 100;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The key v1 wants for a space key, ID, ARI or URL.
fn space_key(c: &Client, space: &str) -> Result<String> {
    let space = exact_space(space)?;
    if !digits(&space) {
        return Ok(space);
    }
    c.get(&format!("/api/v2/spaces/{}", enc(&space)), &[])?["key"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("space {space} came back without its key"))
}

// --- tasks -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum TaskFilter {
    Open,
    Complete,
    #[default]
    All,
}

impl TaskFilter {
    pub fn name(self) -> &'static str {
        match self {
            TaskFilter::Open => "open",
            TaskFilter::Complete => "complete",
            TaskFilter::All => "all",
        }
    }
}

/// A task with its body as Markdown.
fn shown_task(v: &Value) -> Value {
    let mut t = v.clone();
    if t.is_object() {
        let id = v["id"].as_str().unwrap_or_default();
        t["body"] = rendered_body(v, BodyFormat::Markdown, &format!("task {id}"));
    }
    t
}

/// One page of the tasks on one page or blog post, or with no ID of every
/// task the caller can see: `{results, nextCursor}`.
pub fn tasks(
    c: &Client,
    id: Option<&str>,
    kind: Option<ContentType>,
    status: TaskFilter,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let limit = limit.clamp(1, 250).to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("body-format", "atlas_doc_format"),
        ("limit", limit.as_str()),
    ];
    match status {
        TaskFilter::Open => q.push(("status", "incomplete")),
        TaskFilter::Complete => q.push(("status", "complete")),
        TaskFilter::All => {}
    }
    if let Some(id) = id {
        // The filter differs by type, and the wrong one just finds nothing.
        let kind = match kind {
            Some(k) => k,
            None => {
                try_content(|k| c.get(&format!("/api/v2/{}/{}", k.collection(), enc(id)), &[]))?.0
            }
        };
        let name = match kind {
            ContentType::Page => "page-id",
            ContentType::Blogpost => "blogpost-id",
        };
        q.push((name, id));
    }
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    Ok(listing(&c.get("/api/v2/tasks", &q)?, shown_task))
}

fn task_path(id: &str) -> Result<String> {
    if !digits(id) {
        return Err(usage(format!("'{id}' is not a task ID")));
    }
    Ok(format!("/api/v2/tasks/{id}"))
}

/// `page task get`
pub fn task(c: &Client, id: &str) -> Result<Value> {
    Ok(shown_task(&c.get(
        &task_path(id)?,
        &[("body-format", "atlas_doc_format")],
    )?))
}

/// `page task update --status`
pub fn task_set(c: &Client, id: &str, complete: bool) -> Result<Value> {
    let status = if complete { "complete" } else { "incomplete" };
    Ok(shown_task(
        &c.put(&task_path(id)?, &json!({"status": status}))?,
    ))
}

// --- content states ----------------------------------------------------------

fn version_of(draft: bool) -> [(&'static str, &'static str); 1] {
    [("status", if draft { "draft" } else { "current" })]
}

pub fn state(c: &Client, id: &str, draft: bool) -> Result<Value> {
    c.get(
        &format!("/rest/api/content/{}/state", enc(id)),
        &version_of(draft),
    )
}

/// A state by ID, or by name and color. The REST request body has no room
/// for both.
pub fn state_input(id: Option<u64>, name: Option<&str>, color: Option<&str>) -> Result<Value> {
    let name = name.map(str::trim);
    match (id, name, color) {
        (Some(id), None, None) => Ok(json!({"id": id})),
        (None, Some(name), Some(color)) => {
            if name.is_empty() || name.chars().count() > 20 {
                return Err(usage("--name is 1 to 20 characters"));
            }
            let hex = color.strip_prefix('#').unwrap_or(color);
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(usage(format!("'{color}' is not a color like #2684ff")));
            }
            Ok(json!({"name": name, "color": format!("#{hex}")}))
        }
        _ => Err(usage("pass --state-id, or both --name and --color")),
    }
}

pub fn state_set(c: &Client, id: &str, draft: bool, state: &Value) -> Result<Value> {
    c.request(
        "PUT",
        &format!("/rest/api/content/{}/state", enc(id)),
        &version_of(draft),
        Some(state),
    )
}

pub fn states_available(c: &Client, id: &str) -> Result<Value> {
    c.get(
        &format!("/rest/api/content/{}/state/available", enc(id)),
        &[],
    )
}

// --- watches -----------------------------------------------------------------

fn set_watch(c: &Client, path: &str, on: bool) -> Result<()> {
    if on {
        // Confluence's XSRF check rejects this POST without the header.
        c.request_with(
            "POST",
            path,
            &[],
            Some(&json!({})),
            &[("X-Atlassian-Token", "no-check")],
        )?;
    } else {
        c.delete(path, &[])?;
    }
    Ok(())
}

pub fn watch_content(c: &Client, id: &str, on: bool) -> Result<Value> {
    set_watch(c, &format!("/rest/api/user/watch/content/{}", enc(id)), on)?;
    Ok(json!({"id": id, "watching": on}))
}

pub fn watch_space(c: &Client, space: &str, on: bool) -> Result<Value> {
    let key = space_key(c, space)?;
    set_watch(c, &format!("/rest/api/user/watch/space/{}", enc(&key)), on)?;
    Ok(json!({"key": key, "watching": on}))
}

// --- templates ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    Global,
    Space,
    /// The space's and the global ones
    Available,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum TemplateType {
    #[default]
    All,
    Page,
    Blueprint,
}

#[derive(Debug, Default)]
pub struct TemplateQuery<'a> {
    pub space: Option<&'a str>,
    pub scope: Option<Scope>,
    pub kind: TemplateType,
    pub text: Option<&'a str>,
    /// How many matches to skip; REST can't search, so this pages locally.
    pub start: usize,
    pub limit: usize,
}

pub fn templates(c: &Client, o: &TemplateQuery) -> Result<Value> {
    if o.limit == 0 {
        return Err(usage("--limit must be at least 1"));
    }
    let scope = o.scope.unwrap_or(match o.space {
        Some(_) => Scope::Space,
        None => Scope::Global,
    });
    if scope == Scope::Space && o.space.is_none() {
        return Err(usage("--scope space needs --space"));
    }
    let key = match (scope, o.space) {
        (Scope::Global, _) | (_, None) => None,
        (_, Some(space)) => Some(space_key(c, space)?),
    };
    let mut places: Vec<Option<&str>> = vec![key.as_deref()];
    if scope == Scope::Available && key.is_some() {
        places.push(None);
    }
    let kinds: &[&str] = match o.kind {
        TemplateType::All => &["page", "blueprint"],
        TemplateType::Page => &["page"],
        TemplateType::Blueprint => &["blueprint"],
    };
    let mut all = Map::new();
    for kind in kinds {
        for place in &places {
            let mut start = 0;
            loop {
                let (at, max) = (start.to_string(), TEMPLATE_PAGE.to_string());
                let mut q = vec![("start", at.as_str()), ("limit", max.as_str())];
                if let Some(key) = place {
                    q.push(("spaceKey", key));
                }
                let v = c.get(&format!("/rest/api/template/{kind}"), &q)?;
                let page = v["results"]
                    .as_array()
                    .ok_or_else(|| anyhow!("Confluence returned no template list"))?;
                for t in page {
                    // Space and global lists overlap for blueprints.
                    let id = match &t["templateId"] {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    all.entry(id).or_insert_with(|| t.clone());
                }
                start += page.len();
                // The server may cap the page below what was asked.
                let more = v["_links"]["next"].is_string() || page.len() >= TEMPLATE_PAGE;
                if page.is_empty() || !more {
                    break;
                }
            }
        }
    }
    let wanted = o.text.map(str::to_lowercase);
    let found: Vec<&Value> = all
        .values()
        .filter(|t| match &wanted {
            None => true,
            Some(w) => ["templateId", "name", "description"]
                .iter()
                .any(|k| t[*k].as_str().is_some_and(|s| s.to_lowercase().contains(w))),
        })
        .collect();
    let end = o.start.saturating_add(o.limit);
    let results: Vec<&Value> = found.iter().copied().skip(o.start).take(o.limit).collect();
    Ok(json!({
        "results": results,
        "nextCursor": (end < found.len()).then(|| end.to_string()),
    }))
}

pub fn template(c: &Client, id: &str, format: BodyFormat) -> Result<Value> {
    let id = id.trim();
    if id.is_empty() {
        return Err(usage("the template ID is empty"));
    }
    let expand = match format {
        BodyFormat::Storage => "body.storage",
        _ => "body.atlas_doc_format",
    };
    let mut v = c.get(
        &format!("/rest/api/template/{}", enc(id)),
        &[("expand", expand)],
    )?;
    let body = match format {
        BodyFormat::Storage => v["body"]["storage"]["value"].clone(),
        _ if v["body"]["atlas_doc_format"]["value"].is_null() => {
            crate::output::notes::warn(
                "no_adf_body",
                format!(
                    "template {id} came back without an ADF body; read it with --body-format storage"
                ),
            );
            Value::Null
        }
        _ => rendered_body(&v, format, &format!("template {id}")),
    };
    v["body"] = body;
    Ok(v)
}

// --- space permissions -------------------------------------------------------

/// A space's assignments, or with no space every permission there is.
pub fn permissions(
    c: &Client,
    space: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let path = match space {
        Some(space) => {
            let id = space_id(c, &exact_space(space)?)?;
            format!("/api/v2/spaces/{}/permissions", enc(&id))
        }
        None => "/api/v2/space-permissions".to_string(),
    };
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    Ok(listing(&c.get(&path, &q)?, Value::clone))
}

//! Moving, copying, trashing and archiving content, draft actions, space
//! lifecycle, and the appearance extras of `content create`.

use super::{ContentType, enc, space_id, try_content, with_links};
use crate::client::{ApiError, Client, ErrorKind};
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn not_found(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::NotFound)
}

fn kind_name(kind: ContentType) -> &'static str {
    match kind {
        ContentType::Page => "page",
        ContentType::Blogpost => "blogpost",
    }
}

fn kind_path(kind: ContentType, id: &str) -> String {
    format!("/api/v2/{}/{}", kind.collection(), enc(id))
}

// --- references --------------------------------------------------------------

/// The content ID a number, ARI or content URL names. Stricter than
/// [`super::content_id`], which takes the first long number after `/pages/`:
/// commands that purge can't guess.
fn id_in(r: &str) -> Option<String> {
    if digits(r) {
        return Some(r.to_string());
    }
    if r.starts_with("ari:") {
        let (head, id) = r.rsplit_once('/')?;
        let content = [":page", ":blogpost"].iter().any(|k| head.ends_with(k));
        return (content && digits(id)).then(|| id.to_string());
    }
    if !r.starts_with("https://") && !r.starts_with("http://") {
        return None;
    }
    let (path, rest) = r.split_at(r.find(['?', '#']).unwrap_or(r.len()));
    if path.ends_with("/viewpage.action") {
        return rest
            .trim_start_matches('?')
            .split(['&', '#'])
            .find_map(|p| p.strip_prefix("pageId="))
            .filter(|id| digits(id))
            .map(String::from);
    }
    let parts: Vec<&str> = path.split('/').collect();
    let at = parts.iter().position(|p| *p == "pages" || *p == "blog")?;
    let mut after = &parts[at + 1..];
    if matches!(after.first(), Some(&("edit-v2" | "edit"))) {
        after = &after[1..];
    } else if parts[at] == "blog" {
        // /blog/YYYY/MM/DD/ID/Title
        after = after.get(3..)?;
    }
    after
        .first()
        .filter(|id| digits(id))
        .map(|id| id.to_string())
}

/// The content a write acts on: an ID, ARI or URL, or a title that exactly
/// one page or blog post in `space` has.
pub fn exact_content(c: &Client, reference: &str, space: Option<&str>) -> Result<String> {
    let r = reference.trim();
    if let Some(id) = id_in(r) {
        return Ok(id);
    }
    let looks_like_link = r.starts_with("ari:") || r.contains("://");
    let Some(space) = space.filter(|_| !looks_like_link && !r.is_empty()) else {
        return Err(usage(format!(
            "'{r}' is not a content ID, ARI or URL (pass --reference-space to look up an exact title)"
        )));
    };
    let sid = space_id(c, space)?;
    let q = [("space-id", sid.as_str()), ("title", r)];
    let mut found = Vec::new();
    for kind in [ContentType::Page, ContentType::Blogpost] {
        let v = c.get(&format!("/api/v2/{}", kind.collection()), &q)?;
        for item in v["results"].as_array().into_iter().flatten() {
            if let Some(id) = item["id"].as_str() {
                found.push(id.to_string());
            }
        }
    }
    match found.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(usage(format!("nothing in space '{space}' is titled '{r}'"))),
        _ => Err(usage(format!(
            "several items are titled '{r}' ({}); pass the ID",
            found.join(", ")
        ))),
    }
}

/// The space a key, ID, space ARI or space URL names. A URL of something in
/// the space is refused: it must never be enough to delete the space.
pub fn exact_space(s: &str) -> Result<String> {
    let s = s.trim();
    let candidate = if s.starts_with("ari:") {
        s.rsplit_once(":space/")
            .map(|(_, id)| id)
            .filter(|id| digits(id))
    } else if s.starts_with("https://") || s.starts_with("http://") {
        let path = &s[..s.find(['?', '#']).unwrap_or(s.len())];
        path.split_once("/spaces/").and_then(|(_, rest)| {
            match rest.trim_end_matches('/').split_once('/') {
                None => Some(rest.trim_end_matches('/')),
                Some((key, "overview")) => Some(key),
                Some(_) => None,
            }
        })
    } else {
        Some(s)
    };
    candidate
        .filter(|k| {
            let bare = k.strip_prefix('~').unwrap_or(k);
            let personal = k.starts_with('~');
            !bare.is_empty()
                && bare.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || (personal && matches!(b, b'-' | b':' | b'_'))
                })
        })
        .map(String::from)
        .ok_or_else(|| usage(format!("'{s}' is not a space key, ID, ARI or space URL")))
}

fn space_by(c: &Client, s: &str) -> Result<Value> {
    let exact = exact_space(s)?;
    c.get(
        &format!("/api/v2/spaces/{}", enc(&space_id(c, &exact)?)),
        &[("description-format", "plain")],
    )
}

// --- content -----------------------------------------------------------------

/// Where `content move` puts the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Position {
    Append,
    Before,
    After,
}

impl Position {
    fn name(self) -> &'static str {
        match self {
            Position::Append => "append",
            Position::Before => "before",
            Position::After => "after",
        }
    }
}

pub fn move_content(c: &Client, id: &str, position: Position, target: &str) -> Result<Value> {
    if id == target {
        return Err(usage("content can't be moved relative to itself"));
    }
    let path = format!(
        "/rest/api/content/{}/move/{}/{}",
        enc(id),
        position.name(),
        enc(target)
    );
    c.request("PUT", &path, &[], None)?;
    Ok(json!({"id": id, "position": position.name(), "targetId": target, "moved": true}))
}

/// Where `content copy` puts the copy.
#[derive(Clone, Copy, Debug)]
pub enum CopyTo<'a> {
    Parent(&'a str),
    Space(&'a str),
}

#[derive(Clone, Copy, Debug)]
pub struct CopyOpts<'a> {
    pub to: CopyTo<'a>,
    pub title: Option<&'a str>,
    pub attachments: bool,
    pub labels: bool,
    pub permissions: bool,
}

pub fn copy(c: &Client, id: &str, o: &CopyOpts) -> Result<Value> {
    let destination = match o.to {
        CopyTo::Parent(p) => json!({"type": "parent_page", "value": p}),
        CopyTo::Space(s) => {
            // The endpoint takes a key, never an ID.
            let exact = exact_space(s)?;
            let key = if digits(&exact) {
                space_by(c, &exact)?["key"]
                    .as_str()
                    .ok_or_else(|| anyhow!("space {exact} has no key"))?
                    .to_string()
            } else {
                exact
            };
            json!({"type": "space", "value": key})
        }
    };
    let mut req = json!({
        "destination": destination,
        "copyAttachments": o.attachments,
        "copyLabels": o.labels,
        "copyPermissions": o.permissions,
    });
    if let Some(t) = o.title {
        req["pageTitle"] = t.into();
    }
    let v = c.post(&format!("/rest/api/content/{}/copy", enc(id)), &req)?;
    Ok(json!({
        "id": v["id"],
        "title": v["title"],
        "type": v["type"],
        "status": v["status"],
        "copiedFrom": id,
        "externalUrl": super::optional_content_url(c, &v),
    }))
}

pub fn trash(c: &Client, id: &str) -> Result<Value> {
    let (kind, _) = try_content(|k| c.delete(&kind_path(k, id), &[]))?;
    Ok(json!({"id": id, "type": kind_name(kind), "trashed": true}))
}

/// Deletes for good, and only what is in the trash already.
pub fn purge(c: &Client, id: &str) -> Result<Value> {
    let (kind, now) = try_content(|k| c.get(&kind_path(k, id), &[("status", "trashed")]))?;
    if now["status"] != "trashed" {
        return Err(usage(format!(
            "content {id} isn't in the trash; run `content trash` first"
        )));
    }
    c.delete(&kind_path(kind, id), &[("purge", "true")])?;
    Ok(json!({"id": id, "type": kind_name(kind), "purged": true}))
}

/// v2 restores through an update. Its schema requires a body, though a
/// restore doesn't change it, so the trashed one is sent back: never an empty
/// body made up here.
pub fn untrash(c: &Client, id: &str) -> Result<Value> {
    let q = [("status", "trashed"), ("body-format", "storage")];
    let (kind, now) = try_content(|k| c.get(&kind_path(k, id), &q))?;
    if now["status"] != "trashed" {
        return Err(usage(format!("content {id} isn't in the trash")));
    }
    let n = now["version"]["number"]
        .as_u64()
        .ok_or_else(|| anyhow!("content {id} came back without its version"))?;
    let storage = now["body"]["storage"]["value"]
        .as_str()
        .ok_or_else(|| anyhow!("content {id} came back without its body; not restoring it"))?;
    let body = json!({
        "id": id,
        "title": now["title"],
        "status": "current",
        "body": {"representation": "storage", "value": storage},
        "version": {"number": n + 1},
    });
    let v = c.put(&kind_path(kind, id), &body)?;
    Ok(json!({
        "id": id,
        "type": kind_name(kind),
        "title": now["title"],
        "status": v["status"],
        "restored": true,
    }))
}

/// How long and how often a long-running task is polled.
#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub timeout: Duration,
    pub interval: Duration,
}

/// Archiving runs as a task on the server; `wait` polls it.
pub fn archive(c: &Client, id: &str, wait: Option<Wait>) -> Result<Value> {
    let n: u64 = id
        .parse()
        .map_err(|_| usage(format!("'{id}' is not a content ID")))?;
    let started = c.post("/rest/api/content/archive", &json!({"pages": [{"id": n}]}))?;
    let mut out = json!({"id": id, "taskId": started["id"], "archived": false});
    let Some(w) = wait else {
        return Ok(out);
    };
    let task = started["id"]
        .as_str()
        .ok_or_else(|| anyhow!("Confluence started archiving but gave no task to follow"))?;
    let begun = Instant::now();
    loop {
        let t = c.get(&format!("/rest/api/longtask/{}", enc(task)), &[])?;
        if t["finished"] == true {
            if t["successful"] != true {
                let why = t["messages"][0]["translation"]
                    .as_str()
                    .unwrap_or("no reason given");
                return Err(anyhow!("archiving {id} failed: {why}"));
            }
            out["archived"] = true.into();
            return Ok(out);
        }
        let left = w.timeout.saturating_sub(begun.elapsed());
        if left.is_zero() {
            return Err(ApiError::new(
                ErrorKind::Timeout,
                format!(
                    "{id} was still being archived after {}s (task {task})",
                    w.timeout.as_secs()
                ),
            )
            .into());
        }
        std::thread::sleep(w.interval.min(left));
    }
}

// --- drafts ------------------------------------------------------------------

/// The draft, and the version number of the published content if there is
/// one.
fn draft_of(
    c: &Client,
    id: &str,
    kind: Option<ContentType>,
) -> Result<(ContentType, Value, Option<u64>)> {
    let read = |k: ContentType| {
        c.get(
            &kind_path(k, id),
            &[("get-draft", "true"), ("body-format", "storage")],
        )
    };
    let (kind, draft) = match kind {
        Some(k) => (k, read(k)?),
        None => try_content(read)?,
    };
    if draft["status"] != "draft" {
        return Err(usage(format!("content {id} has no draft")));
    }
    let published = match c.get(&kind_path(kind, id), &[("status", "current")]) {
        Ok(v) if v["status"] == "current" => Some(
            v["version"]["number"]
                .as_u64()
                .ok_or_else(|| anyhow!("content {id} came back without its version"))?,
        ),
        Ok(_) => None,
        Err(e) if not_found(&e) => None,
        Err(e) => return Err(e),
    };
    Ok((kind, draft, published))
}

pub fn publish(
    c: &Client,
    id: &str,
    kind: Option<ContentType>,
    title: Option<&str>,
) -> Result<Value> {
    let (kind, draft, published) = draft_of(c, id, kind)?;
    let body = draft["body"]["storage"]["value"]
        .as_str()
        .ok_or_else(|| anyhow!("the draft came back without its body; not publishing"))?;
    let title = title
        .or(draft["title"].as_str())
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| usage("the draft has no title; pass --title"))?;
    let v = c.put(
        &kind_path(kind, id),
        &json!({
            "id": id,
            "status": "current",
            "title": title,
            "body": {"representation": "storage", "value": body},
            "version": {"number": published.map_or(1, |n| n + 1)},
        }),
    )?;
    Ok(with_links(c, v))
}

/// Only a draft that was never published: REST documents `draft=true` for
/// those, and a mistake here would trash the published page.
pub fn delete_draft(c: &Client, id: &str, kind: Option<ContentType>) -> Result<Value> {
    let (kind, _, published) = draft_of(c, id, kind)?;
    if published.is_some() {
        return Err(usage(format!(
            "content {id} is published; the REST API can only delete drafts that never were, so discard this one in the editor"
        )));
    }
    c.delete(&kind_path(kind, id), &[("draft", "true")])?;
    Ok(json!({"id": id, "type": kind_name(kind), "draftDeleted": true}))
}

// --- appearance --------------------------------------------------------------

/// `--content-width`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Width {
    #[value(alias = "fixed")]
    Narrow,
    #[value(alias = "full")]
    Wide,
    Max,
}

/// A title emoji as the hex code point Confluence stores.
pub fn emoji_code(s: &str) -> Result<String> {
    let s = s.trim();
    let chars: Vec<char> = s.chars().filter(|c| *c != '\u{fe0f}').collect();
    let symbol = |c: &char| *c as u32 >= 0x2000;
    let found = match chars.as_slice() {
        [c] => Some(*c).filter(symbol),
        _ => u32::from_str_radix(s, 16)
            .ok()
            .and_then(char::from_u32)
            .filter(symbol),
    };
    found.map(|c| format!("{:x}", c as u32)).ok_or_else(|| {
        usage(format!(
            "'{s}' is not a single emoji or its hex code point, like 1f44d"
        ))
    })
}

/// Set the emoji and width of content that was just created.
pub fn set_appearance(
    c: &Client,
    kind: ContentType,
    id: &str,
    emoji: Option<&str>,
    width: Option<Width>,
) -> Result<()> {
    let width = width.map(|w| match w {
        Width::Narrow => "fixed-width",
        Width::Wide => "full-width",
        Width::Max => "max",
    });
    let path = format!("{}/properties", kind_path(kind, id));
    for (prefix, value) in [("emoji-title", emoji), ("content-appearance", width)] {
        let Some(value) = value else { continue };
        for state in ["published", "draft"] {
            c.post(
                &path,
                &json!({"key": format!("{prefix}-{state}"), "value": value}),
            )?;
        }
    }
    Ok(())
}

// --- spaces ------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct NewSpace<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub private: bool,
    pub dry_run: bool,
}

fn plain(text: &str) -> Value {
    json!({"plain": {"value": text, "representation": "plain"}})
}

pub fn space_create(c: &Client, n: &NewSpace) -> Result<Value> {
    if n.key.is_empty() || !n.key.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(usage("--key takes letters and digits only"));
    }
    let mut req = json!({"key": n.key, "name": n.name});
    if let Some(d) = n.description {
        req["description"] = plain(d);
    }
    let path = if n.private {
        "/rest/api/space/_private"
    } else {
        "/rest/api/space"
    };
    if n.dry_run {
        return Ok(json!({"dryRun": true, "method": "POST", "path": path, "body": req}));
    }
    c.post(path, &req)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SpaceChange<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub homepage: Option<&'a str>,
}

/// v1's PUT replaces, so the current name and description go back with it.
pub fn space_update(c: &Client, space: &str, s: &SpaceChange) -> Result<Value> {
    if [s.name, s.description, s.kind, s.status, s.homepage]
        .iter()
        .all(Option::is_none)
    {
        return Err(usage(
            "nothing to change: pass --name, --description, --type, --status or --homepage",
        ));
    }
    let now = space_by(c, space)?;
    let (Some(key), Some(name)) = (now["key"].as_str(), s.name.or(now["name"].as_str())) else {
        return Err(anyhow!(
            "the space came back without its key or name; not changing it"
        ));
    };
    let mut req = Map::new();
    req.insert("name".into(), name.into());
    if let Some(d) = s
        .description
        .or(now["description"]["plain"]["value"].as_str())
    {
        req.insert("description".into(), plain(d));
    }
    for (k, v) in [("type", s.kind), ("status", s.status)] {
        if let Some(v) = v {
            req.insert(k.into(), v.into());
        }
    }
    if let Some(h) = s.homepage {
        let id = id_in(h).ok_or_else(|| usage("--homepage takes a page ID, ARI or URL"))?;
        req.insert("homepage".into(), json!({"id": id}));
    }
    c.put(
        &format!("/rest/api/space/{}", enc(key)),
        &Value::Object(req),
    )
}

/// Permanent. Confluence does it as a long-running task.
pub fn space_delete(c: &Client, space: &str) -> Result<Value> {
    let now = space_by(c, space)?;
    let key = now["key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow!("the space came back without its key; not deleting it"))?;
    let task = c.delete(&format!("/rest/api/space/{}", enc(key)), &[])?;
    Ok(json!({"id": now["id"], "key": key, "deleting": true, "taskId": task["id"]}))
}

//! Confluence Cloud (v1 CQL search, v2 content/spaces), shaped after twg's
//! `confluence search|content|space` commands.

use crate::client::Client;
use crate::markup::{TextFormat, quote, storage_to_text, strip_highlight, to_storage};
use crate::time;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ContentType {
    #[default]
    Page,
    Blogpost,
}

impl ContentType {
    fn collection(self) -> &'static str {
        match self {
            ContentType::Page => "pages",
            ContentType::Blogpost => "blogposts",
        }
    }
}

/// `--detail` for `content get` (twg: summary by default, full for editing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Detail {
    #[default]
    Summary,
    Full,
}

/// `--format` for reading bodies: raw storage HTML, or markdown-ish text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ReadFormat {
    #[default]
    Html,
    #[value(alias = "markdown")]
    Md,
}

/// A content ID, or a Confluence URL containing one.
pub fn content_id(r: &str) -> Result<String> {
    let r = r.trim();
    if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(r.to_string());
    }
    if let Some(i) = r.find("pageId=") {
        let id: String = r[i + 7..].chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    // .../pages/123/Title, .../blog/2026/01/02/123/Title, .../pages/edit-v2/123
    let segments: Vec<&str> = r.split(['/', '?', '#']).collect();
    if let Some(i) = segments.iter().position(|s| *s == "pages" || *s == "blog")
        && let Some(id) = segments[i + 1..].iter().find(|s| s.len() >= 5 && s.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(id.to_string());
        }
    bail!("'{r}' is not a Confluence content ID or URL (title lookup isn't supported yet)")
}

fn next_cursor(v: &Value) -> Value {
    v["_links"]["next"]
        .as_str()
        .and_then(|n| n.split(['?', '&']).find_map(|p| p.strip_prefix("cursor=")))
        .map(|c| urlencoding::decode(c).map(|c| c.into_owned()).unwrap_or_else(|_| c.to_string()))
        .map_or(Value::Null, Value::from)
}

// --- search ------------------------------------------------------------------

/// `confluence search query --cql`
pub fn search_query(c: &Client, cql: &str, limit: usize, excerpt: bool, cursor: Option<&str>) -> Result<Value> {
    let limit = limit.clamp(1, 100).to_string();
    let mut q = vec![("cql", cql), ("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let v = c.get("/rest/api/search", &q)?;
    let results: Vec<Value> = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let mut o = json!({
                "id": r["content"]["id"],
                "type": r["content"]["type"],
                "title": strip_highlight(r["title"].as_str().unwrap_or_default()),
                "space": r["resultGlobalContainer"]["title"],
                "updated": r["lastModified"],
                "url": format!("{}{}", c.base(), r["url"].as_str().unwrap_or_default()),
            });
            if excerpt {
                o["excerpt"] = strip_highlight(r["excerpt"].as_str().unwrap_or_default()).trim().into();
            }
            o
        })
        .collect();
    Ok(json!({"results": results, "next_cursor": next_cursor(&v)}))
}

/// The CQL that `confluence search text` runs: recent pages and blog posts.
pub fn text_cql(text: &str, updated_since: Option<&str>, created_since: Option<&str>) -> Result<String> {
    let mut cql = format!("siteSearch ~ {} AND type in (page, blogpost)", quote(text));
    if let Some(s) = updated_since {
        cql.push_str(&format!(" AND lastmodified >= \"{}\"", time::to_date(s)?));
    }
    if let Some(s) = created_since {
        cql.push_str(&format!(" AND created >= \"{}\"", time::to_date(s)?));
    }
    cql.push_str(" ORDER BY lastmodified DESC");
    Ok(cql)
}

// --- content -----------------------------------------------------------------

/// Fetch a page, falling back to blog posts. Returns the content type too.
fn fetch(c: &Client, id: &str, draft: bool) -> Result<(ContentType, Value)> {
    let mut q = vec![("body-format", "storage")];
    if draft {
        q.push(("get-draft", "true"));
    }
    match c.get(&format!("/api/v2/pages/{}", enc(id)), &q) {
        Ok(v) => Ok((ContentType::Page, v)),
        Err(e) if e.to_string().contains("HTTP 404") => {
            Ok((ContentType::Blogpost, c.get(&format!("/api/v2/blogposts/{}", enc(id)), &q)?))
        }
        Err(e) => Err(e),
    }
}

fn content_url(c: &Client, v: &Value) -> String {
    let base = v["_links"]["base"].as_str().unwrap_or(c.base());
    format!("{base}{}", v["_links"]["webui"].as_str().unwrap_or_default())
}

/// The version number doubles as atlx's snapshot token for optimistic updates.
fn snapshot(v: &Value) -> Value {
    v["version"]["number"].as_u64().map_or(Value::Null, |n| n.to_string().into())
}

pub struct GetOpts {
    pub detail: Detail,
    pub format: ReadFormat,
    pub draft: bool,
}

/// `confluence content get`
pub fn get(c: &Client, reference: &str, o: &GetOpts) -> Result<Value> {
    let id = content_id(reference)?;
    let (kind, v) = fetch(c, &id, o.draft)?;
    let storage = v["body"]["storage"]["value"].as_str().unwrap_or_default();
    let mut out = json!({
        "id": v["id"],
        "type": format!("{kind:?}").to_lowercase(),
        "title": v["title"],
        "status": v["status"],
        "space_id": v["spaceId"],
        "parent_id": v["parentId"],
        "version": v["version"]["number"],
        "snapshot_token": snapshot(&v),
        "updated": v["version"]["createdAt"],
        "url": content_url(c, &v),
    });
    match o.detail {
        Detail::Summary => {
            let text = storage_to_text(storage);
            let excerpt: String = text.chars().take(400).collect();
            out["excerpt"] = if excerpt.len() < text.len() { format!("{excerpt}…") } else { excerpt }.into();
        }
        Detail::Full => {
            out["format"] = format!("{:?}", o.format).to_lowercase().into();
            out["body"] = match o.format {
                ReadFormat::Html => storage.into(),
                ReadFormat::Md => storage_to_text(storage).into(),
            };
        }
    }
    Ok(out)
}

/// Space key or numeric ID -> numeric ID.
fn space_id(c: &Client, space: &str) -> Result<String> {
    if space.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(space.to_string());
    }
    let v = c.get("/api/v2/spaces", &[("keys", space)])?;
    v["results"][0]["id"].as_str().map(String::from).ok_or_else(|| anyhow!("no Confluence space with key '{space}'"))
}

pub struct NewContent<'a> {
    pub space: &'a str,
    pub content_type: ContentType,
    pub title: &'a str,
    pub body: &'a str,
    pub format: TextFormat,
    pub parent: Option<&'a str>,
    pub draft: bool,
}

/// `confluence content create`
pub fn create(c: &Client, n: NewContent) -> Result<Value> {
    let mut req = json!({
        "spaceId": space_id(c, n.space)?,
        "status": if n.draft { "draft" } else { "current" },
        "title": n.title,
        "body": {"representation": "storage", "value": to_storage(n.body, n.format)},
    });
    if let Some(p) = n.parent {
        if n.content_type == ContentType::Blogpost {
            bail!("blog posts can't have a parent");
        }
        req["parentId"] = content_id(p)?.into();
    }
    let v = c.post(&format!("/api/v2/{}", n.content_type.collection()), &req)?;
    Ok(json!({"id": v["id"], "title": v["title"], "status": v["status"], "version": v["version"]["number"],
              "snapshot_token": snapshot(&v), "url": content_url(c, &v)}))
}

pub struct Edit<'a> {
    pub title: Option<&'a str>,
    pub body: Option<(&'a str, TextFormat)>,
    pub version_message: Option<&'a str>,
    pub minor_edit: bool,
    pub snapshot_token: Option<&'a str>,
    pub dry_run: bool,
    pub draft: bool,
}

/// `confluence content update`: bumps the version, keeping whatever isn't given.
/// A `--snapshot-token` that doesn't match the current version is rejected.
pub fn update(c: &Client, reference: &str, e: Edit) -> Result<Value> {
    if e.title.is_none() && e.body.is_none() {
        bail!("nothing to update; pass --title and/or --body/--body-file");
    }
    let id = content_id(reference)?;
    let (kind, cur) = fetch(c, &id, e.draft)?;
    let version = cur["version"]["number"].as_u64().ok_or_else(|| anyhow!("content {id} has no version"))?;
    if let Some(tok) = e.snapshot_token
        && tok != version.to_string() {
            bail!("content {id} changed since your snapshot (token {tok}, current {version}); re-read it and retry");
        }
    let body = match e.body {
        Some((b, f)) => to_storage(b, f),
        None => cur["body"]["storage"]["value"].as_str().unwrap_or_default().to_string(),
    };
    let req = json!({
        "id": id,
        "status": if e.draft { "draft" } else { "current" },
        "title": e.title.unwrap_or(cur["title"].as_str().unwrap_or_default()),
        "body": {"representation": "storage", "value": body},
        "version": {"number": version + 1, "message": e.version_message.unwrap_or(""), "minorEdit": e.minor_edit},
    });
    if e.dry_run {
        return Ok(json!({"dry_run": true, "id": id, "from_version": version, "request": req}));
    }
    let v = c.put(&format!("/api/v2/{}/{}", kind.collection(), enc(&id)), &req)?;
    Ok(json!({"id": v["id"], "title": v["title"], "version": v["version"]["number"],
              "snapshot_token": snapshot(&v), "url": content_url(c, &v)}))
}

/// `confluence content list --space`
pub fn list(
    c: &Client,
    space: &str,
    kind: ContentType,
    status: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("status", status), ("limit", limit.as_str()), ("sort", "-modified-date")];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let v = c.get(&format!("/api/v2/spaces/{}/{}", enc(&space_id(c, space)?), kind.collection()), &q)?;
    let results: Vec<Value> = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| json!({"id": p["id"], "title": p["title"], "status": p["status"], "parent_id": p["parentId"],
                        "version": p["version"]["number"], "updated": p["version"]["createdAt"], "url": content_url(c, p)}))
        .collect();
    Ok(json!({"results": results, "next_cursor": next_cursor(&v)}))
}

// --- spaces ------------------------------------------------------------------

fn space_summary(c: &Client, s: &Value) -> Value {
    json!({
        "id": s["id"],
        "key": s["key"],
        "name": s["name"],
        "type": s["type"],
        "status": s["status"],
        "homepage_id": s["homepageId"],
        "url": format!("{}{}", c.base(), s["_links"]["webui"].as_str().unwrap_or_default()),
    })
}

/// `confluence space list`
pub fn spaces(c: &Client, kind: Option<&str>, status: Option<&str>, keys: Option<&str>, limit: usize) -> Result<Value> {
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    for (k, v) in [("type", kind), ("status", status), ("keys", keys)] {
        if let Some(v) = v {
            q.push((k, v));
        }
    }
    let v = c.get("/api/v2/spaces", &q)?;
    Ok(v["results"].as_array().into_iter().flatten().map(|s| space_summary(c, s)).collect())
}

/// `confluence space get <key-or-id>`
pub fn space(c: &Client, identifier: &str) -> Result<Value> {
    let v = c.get(&format!("/api/v2/spaces/{}", enc(&space_id(c, identifier)?)), &[("description-format", "plain")])?;
    let mut o = space_summary(c, &v);
    o["description"] = v["description"]["plain"]["value"].clone();
    o["created"] = v["createdAt"].clone();
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_ids_from_urls() {
        assert_eq!(content_id("123456").unwrap(), "123456");
        assert_eq!(content_id("https://a.atlassian.net/wiki/spaces/ENG/pages/98765/Runbook").unwrap(), "98765");
        assert_eq!(content_id("https://a.atlassian.net/wiki/spaces/ENG/pages/edit-v2/98765").unwrap(), "98765");
        assert_eq!(content_id("https://a.atlassian.net/wiki/spaces/ENG/blog/2026/01/02/55555/Post").unwrap(), "55555");
        assert_eq!(content_id("https://a.atlassian.net/wiki/pages/viewpage.action?pageId=4242").unwrap(), "4242");
        assert!(content_id("Some title").is_err());
    }

    #[test]
    fn cursors_and_cql() {
        let v = json!({"_links": {"next": "/wiki/api/v2/spaces/1/pages?limit=5&cursor=abc%3D%3D"}});
        assert_eq!(next_cursor(&v), "abc==");
        assert_eq!(next_cursor(&json!({})), Value::Null);
        assert_eq!(
            text_cql("launch \"plan\"", Some("2026-01-01"), None).unwrap(),
            r#"siteSearch ~ "launch \"plan\"" AND type in (page, blogpost) AND lastmodified >= "2026-01-01" ORDER BY lastmodified DESC"#
        );
    }
}

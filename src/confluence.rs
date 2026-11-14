//! Confluence Cloud (v1 CQL search, v2 content/spaces), shaped after twg's
//! `confluence search|content|space` commands.

use crate::client::Client;
use crate::markup::{
    TextFormat, adf_to_storage, quote, storage_to_text, strip_highlight, to_storage,
};
use crate::time;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

pub mod edits;
pub mod sections;

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
    /// Headings only, with their ADF `localId`s and word counts
    Outline,
    Full,
}

/// `--format` for reading bodies: raw storage HTML, or markdown-ish text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ReadFormat {
    #[default]
    Html,
    #[value(alias = "markdown")]
    Md,
    /// The ADF document as JSON, with node `localId`s for `--edits`
    /// (atlx extra; hidden from the value list because only page and blog
    /// post reads have it)
    #[value(hide = true)]
    Adf,
}

/// The `body-format` a read asks Confluence v2 for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyFormat {
    Storage,
    Adf,
}

impl BodyFormat {
    fn param(self) -> &'static str {
        match self {
            BodyFormat::Storage => "storage",
            BodyFormat::Adf => "atlas_doc_format",
        }
    }
}

/// The ADF document in a v2 response fetched with `atlas_doc_format`.
fn adf_body(v: &Value) -> Value {
    let raw = &v["body"]["atlas_doc_format"]["value"];
    let doc = match raw {
        Value::String(s) => serde_json::from_str(s).ok(),
        Value::Object(_) => Some(raw.clone()),
        _ => None,
    };
    doc.filter(Value::is_object)
        .unwrap_or_else(|| json!({"type": "doc", "version": 1, "content": []}))
}

/// A content ID, or a Confluence URL containing one.
pub fn content_id(r: &str) -> Result<String> {
    let r = r.trim();
    if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(r.to_string());
    }
    if let Some(i) = r.find("pageId=") {
        let id: String = r[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    // .../pages/123/Title, .../blog/2026/01/02/123/Title, .../pages/edit-v2/123
    let segments: Vec<&str> = r.split(['/', '?', '#']).collect();
    if let Some(i) = segments.iter().position(|s| *s == "pages" || *s == "blog")
        && let Some(id) = segments[i + 1..]
            .iter()
            .find(|s| s.len() >= 5 && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Ok(id.to_string());
    }
    bail!("'{r}' is not a Confluence content ID or URL (title lookup isn't supported yet)")
}

fn next_cursor(v: &Value) -> Value {
    v["_links"]["next"]
        .as_str()
        .and_then(|n| n.split(['?', '&']).find_map(|p| p.strip_prefix("cursor=")))
        .map(|c| {
            urlencoding::decode(c)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| c.to_string())
        })
        .map_or(Value::Null, Value::from)
}

// --- search ------------------------------------------------------------------

/// `confluence search query --cql`
pub fn search_query(
    c: &Client,
    cql: &str,
    limit: usize,
    excerpt: bool,
    cursor: Option<&str>,
) -> Result<Value> {
    let limit = limit.clamp(1, 100).to_string();
    let mut q = vec![("cql", cql), ("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let mut v = c.get("/rest/api/search", &q)?;
    // twg passes REST's search results through; highlight markers are
    // removed so titles and excerpts read as plain text.
    if let Some(results) = v["results"].as_array_mut() {
        for r in results {
            for k in ["title", "excerpt"] {
                if let Some(t) = r[k].as_str() {
                    r[k] = strip_highlight(t).trim().into();
                }
            }
            if !excerpt && let Some(m) = r.as_object_mut() {
                m.shift_remove("excerpt");
            }
        }
    }
    let next = next_cursor(&v);
    if let Some(m) = v.as_object_mut() {
        m.shift_remove("_links");
        m.insert("nextCursor".into(), next);
    }
    Ok(v)
}

/// The CQL that `confluence search text` runs: recent pages and blog posts.
pub fn text_cql(
    text: &str,
    updated_since: Option<&str>,
    created_since: Option<&str>,
) -> Result<String> {
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

/// Try a Confluence v2 operation on a page, falling back to a blog post on 404.
fn try_content<T>(mut f: impl FnMut(ContentType) -> Result<T>) -> Result<(ContentType, T)> {
    match f(ContentType::Page) {
        Ok(v) => Ok((ContentType::Page, v)),
        Err(e) if e.to_string().contains("HTTP 404") => {
            Ok((ContentType::Blogpost, f(ContentType::Blogpost)?))
        }
        Err(e) => Err(e),
    }
}

/// Fetch a page, falling back to blog posts. Returns the content type too.
fn fetch(c: &Client, id: &str, draft: bool, bf: BodyFormat) -> Result<(ContentType, Value)> {
    let mut q = vec![("body-format", bf.param())];
    if draft {
        q.push(("get-draft", "true"));
    }
    try_content(|kind| c.get(&format!("/api/v2/{}/{}", kind.collection(), enc(id)), &q))
}

/// `snapshotToken` and `externalUrl` added to a REST v2 content object.
fn with_links(c: &Client, mut v: Value) -> Value {
    if v.is_object() {
        v["snapshotToken"] = snapshot(&v);
        v["externalUrl"] = optional_content_url(c, &v);
    }
    v
}

/// `{results, nextCursor}` from a REST v2 list response, each result passed
/// through `f`.
fn listing(v: &Value, f: impl Fn(&Value) -> Value) -> Value {
    let results: Vec<Value> = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(f)
        .collect();
    json!({"results": results, "nextCursor": next_cursor(v)})
}

fn content_url(c: &Client, v: &Value) -> String {
    let base = v["_links"]["base"].as_str().unwrap_or(c.base());
    format!(
        "{base}{}",
        v["_links"]["webui"].as_str().unwrap_or_default()
    )
}

fn optional_content_url(c: &Client, v: &Value) -> Value {
    match v["_links"]["webui"].as_str().filter(|s| !s.is_empty()) {
        Some(webui) => format!(
            "{}{webui}",
            v["_links"]["base"].as_str().unwrap_or(c.base())
        )
        .into(),
        None => Value::Null,
    }
}

fn format_body(storage: &str, format: ReadFormat) -> Value {
    match format {
        // Storage-only bodies (comments) have no ADF; `adf` gives the HTML.
        ReadFormat::Html | ReadFormat::Adf => storage.into(),
        ReadFormat::Md => storage_to_text(storage).into(),
    }
}

/// The version number doubles as atlx's snapshot token for optimistic updates.
fn snapshot(v: &Value) -> Value {
    v["version"]["number"]
        .as_u64()
        .map_or(Value::Null, |n| n.to_string().into())
}

pub struct GetOpts<'a> {
    pub detail: Detail,
    pub format: ReadFormat,
    pub draft: bool,
    pub reference_space: Option<&'a str>,
    /// `--section` headings; when non-empty only those sections are returned.
    pub sections: &'a [String],
}

impl GetOpts<'_> {
    fn body_format(&self) -> BodyFormat {
        match (self.detail, self.format) {
            (Detail::Outline, _) => BodyFormat::Adf,
            (Detail::Full, ReadFormat::Adf) if self.sections.is_empty() => BodyFormat::Adf,
            _ => BodyFormat::Storage,
        }
    }
}

fn resolve_and_fetch(c: &Client, reference: &str, o: &GetOpts) -> Result<(ContentType, Value)> {
    let bf = o.body_format();
    if let Ok(id) = content_id(reference) {
        return fetch(c, &id, o.draft, bf);
    }
    let Some(ref_space) = o.reference_space else {
        bail!(
            "'{reference}' is not a Confluence content ID or URL (pass --reference-space to look up by title)"
        );
    };
    let sid = space_id(c, ref_space)?;
    let mut q = vec![
        ("space-id", sid.as_str()),
        ("title", reference),
        ("body-format", bf.param()),
    ];
    if o.draft {
        q.push(("status", "draft"));
    }
    for kind in [ContentType::Page, ContentType::Blogpost] {
        let v = c.get(&format!("/api/v2/{}", kind.collection()), &q)?;
        if let Some(item) = v["results"].as_array().and_then(|a| a.first()) {
            return Ok((kind, item.clone()));
        }
    }
    bail!("no Confluence page or blogpost titled '{reference}' in space '{ref_space}'")
}

/// `confluence content get`
pub fn get(c: &Client, reference: &str, o: &GetOpts) -> Result<Value> {
    let (kind, v) = resolve_and_fetch(c, reference, o)?;
    let storage = v["body"]["storage"]["value"].as_str().unwrap_or_default();
    let mut out = json!({
        "id": v["id"],
        "type": format!("{kind:?}").to_lowercase(),
        "title": v["title"],
        "status": v["status"],
        "spaceId": v["spaceId"],
        "parentId": v["parentId"],
        "externalUrl": content_url(c, &v),
        "detail": format!("{:?}", o.detail).to_lowercase(),
        "snapshotToken": snapshot(&v),
        "version": {"number": v["version"]["number"], "createdAt": v["version"]["createdAt"]},
    });
    match o.detail {
        Detail::Summary => {
            let text = storage_to_text(storage);
            let excerpt: String = text.chars().take(400).collect();
            let excerpt = if excerpt.len() < text.len() {
                format!("{excerpt}…")
            } else {
                excerpt
            };
            out["summary"] = json!({
                "excerpt": excerpt,
                "wordCount": text.split_whitespace().count(),
                "sectionCount": section_count(storage),
            });
        }
        Detail::Outline => {
            out["outline"] = sections::outline(&adf_body(&v)).into();
        }
        Detail::Full if !o.sections.is_empty() => {
            let picked = sections::select_storage(storage, o.sections)?;
            let (format, value) = match o.format {
                ReadFormat::Md => (
                    "md",
                    picked
                        .iter()
                        .map(|s| storage_to_text(&storage[s.start..s.end]))
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                ),
                _ => (
                    "html",
                    picked.iter().map(|s| &storage[s.start..s.end]).collect(),
                ),
            };
            let picked: Vec<Value> = picked
                .iter()
                .map(|s| {
                    json!({
                        "level": s.heading.level,
                        "text": s.heading.text,
                        "anchor": sections::anchor(&s.heading.text),
                    })
                })
                .collect();
            out["body"] = json!({"format": format, "value": value, "sections": picked});
        }
        Detail::Full if o.format == ReadFormat::Adf => {
            let doc = adf_body(&v);
            out["body"] = json!({
                "format": "adf",
                "value": serde_json::to_string_pretty(&doc).unwrap_or_default(),
            });
        }
        Detail::Full => {
            out["body"] = json!({
                "format": format!("{:?}", o.format).to_lowercase(),
                "value": format_body(storage, o.format),
            });
        }
    }
    Ok(out)
}

/// Number of headings (`<h1>` to `<h6>`) in a storage body.
fn section_count(storage: &str) -> usize {
    let lower = storage.to_ascii_lowercase();
    (1..=6)
        .map(|n| {
            let tag = format!("<h{n}");
            lower
                .match_indices(&tag)
                .filter(|(i, _)| matches!(lower.as_bytes().get(i + tag.len()), Some(b'>' | b' ')))
                .count()
        })
        .sum()
}

/// Space key or numeric ID -> numeric ID.
fn space_id(c: &Client, space: &str) -> Result<String> {
    if !space.is_empty() && space.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(space.to_string());
    }
    let v = c.get("/api/v2/spaces", &[("keys", space)])?;
    let results = v["results"].as_array().into_iter().flatten();
    results
        .clone()
        .find(|s| s["key"].as_str() == Some(space))
        .or_else(|| {
            results.clone().find(|s| {
                s["key"]
                    .as_str()
                    .is_some_and(|k| k.eq_ignore_ascii_case(space))
            })
        })
        .and_then(|s| s["id"].as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("no Confluence space with key '{space}'"))
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
    Ok(with_links(c, v))
}

pub struct Edit<'a> {
    pub title: Option<&'a str>,
    pub body: Option<(&'a str, TextFormat)>,
    pub version_message: Option<&'a str>,
    pub minor_edit: bool,
    pub snapshot_token: Option<&'a str>,
    pub dry_run: bool,
    pub draft: bool,
    /// `--edits`/`--edits-file` operations, applied to the page's ADF body.
    pub edits: Option<&'a [edits::Op]>,
}

/// `confluence content update`: bumps the version, keeping whatever isn't given.
/// A `--snapshot-token` that doesn't match the current version is rejected.
pub fn update(c: &Client, reference: &str, e: Edit) -> Result<Value> {
    if e.title.is_none() && e.body.is_none() && e.edits.is_none() {
        bail!("nothing to update; pass --title, --body/--body-file or --edits/--edits-file");
    }
    if e.body.is_some() && e.edits.is_some() {
        bail!("pass either a full body or --edits, not both");
    }
    let id = content_id(reference)?;
    let bf = if e.edits.is_some() {
        BodyFormat::Adf
    } else {
        BodyFormat::Storage
    };
    let (kind, cur) = fetch(c, &id, e.draft, bf)?;
    let version = cur["version"]["number"]
        .as_u64()
        .ok_or_else(|| anyhow!("content {id} has no version"))?;
    if let Some(tok) = e.snapshot_token
        && tok != version.to_string()
    {
        bail!(
            "content {id} changed since your snapshot (token {tok}, current {version}); re-read it and retry"
        );
    }
    // Targeted edits: apply them to the current ADF, all or nothing.
    let mut preview = None;
    let body = match (e.body, e.edits) {
        (Some((b, f)), _) => json!({"representation": "storage", "value": to_storage(b, f)}),
        (None, Some(ops)) => {
            let before = adf_body(&cur);
            let after = edits::apply(&before, ops)?;
            preview = Some((before, after.clone()));
            json!({
                "representation": "atlas_doc_format",
                "value": serde_json::to_string(&after)?,
            })
        }
        (None, None) => json!({
            "representation": "storage",
            "value": cur["body"]["storage"]["value"].as_str().unwrap_or_default(),
        }),
    };
    let req = json!({
        "id": id,
        "status": if e.draft { "draft" } else { "current" },
        "title": e.title.unwrap_or(cur["title"].as_str().unwrap_or_default()),
        "body": body,
        "version": {"number": version + 1, "message": e.version_message.unwrap_or(""), "minorEdit": e.minor_edit},
    });
    if e.dry_run {
        let mut out = json!({"dryRun": true, "id": id, "fromVersion": version, "request": req});
        if let Some((before, after)) = preview {
            out["body"] = json!({"format": "html", "value": adf_to_storage(&after)});
            out["diff"] = edits::unified_diff(
                &edits::render_lines(&before),
                &edits::render_lines(&after),
                &format!("{id} version {version}"),
                &format!("{id} after edits"),
            )
            .into();
        }
        return Ok(out);
    }
    let v = c.put(&format!("/api/v2/{}/{}", kind.collection(), enc(&id)), &req)?;
    Ok(with_links(c, v))
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
    let mut q = vec![
        ("status", status),
        ("limit", limit.as_str()),
        ("sort", "-modified-date"),
    ];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let v = c.get(
        &format!(
            "/api/v2/spaces/{}/{}",
            enc(&space_id(c, space)?),
            kind.collection()
        ),
        &q,
    )?;
    Ok(listing(&v, |p| with_links(c, p.clone())))
}

/// A REST v2 comment with its body in the requested format as
/// `body: {format, value}` and its `externalUrl`.
fn format_comment(c: &Client, cm: &Value, format: ReadFormat) -> Value {
    let storage = cm["body"]["storage"]["value"].as_str().unwrap_or_default();
    let mut o = cm.clone();
    o["body"] = json!({
        "format": format!("{format:?}").to_lowercase(),
        "value": format_body(storage, format),
    });
    o["externalUrl"] = optional_content_url(c, cm);
    o
}

/// `confluence content comments list`
pub fn comments_list(
    c: &Client,
    reference: &str,
    inline: bool,
    format: ReadFormat,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let id = content_id(reference)?;
    let endpoint = if inline {
        "inline-comments"
    } else {
        "footer-comments"
    };
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("body-format", "storage"), ("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let (_, v) = try_content(|kind| {
        c.get(
            &format!("/api/v2/{}/{}/{}", kind.collection(), enc(&id), endpoint),
            &q,
        )
    })?;
    Ok(listing(&v, |cm| format_comment(c, cm, format)))
}

/// `confluence content comments create`
pub fn comment_create(
    c: &Client,
    reference: &str,
    body: &str,
    format: TextFormat,
) -> Result<Value> {
    let id = content_id(reference)?;
    let (kind, _) =
        try_content(|k| c.get(&format!("/api/v2/{}/{}", k.collection(), enc(&id)), &[]))?;
    let id_field = match kind {
        ContentType::Page => "pageId",
        ContentType::Blogpost => "blogPostId",
    };
    let req = json!({
        id_field: id,
        "body": {"representation": "storage", "value": to_storage(body, format)},
    });
    let mut v = c.post("/api/v2/footer-comments", &req)?;
    if v.is_object() {
        v["externalUrl"] = optional_content_url(c, &v);
    }
    Ok(v)
}

/// `confluence content children`
pub fn children(c: &Client, reference: &str, limit: usize, cursor: Option<&str>) -> Result<Value> {
    let id = content_id(reference)?;
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let v = c.get(&format!("/api/v2/pages/{}/children", enc(&id)), &q)?;
    Ok(listing(&v, |p| {
        let mut p = p.clone();
        p["externalUrl"] = optional_content_url(c, &p);
        p
    }))
}

const MAX_TREE_DEPTH: usize = 5;

fn fetch_children_all(c: &Client, page_id: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut q = vec![("limit", "250")];
        if let Some(ref cur) = cursor {
            q.push(("cursor", cur.as_str()));
        }
        let v = match c.get(&format!("/api/v2/pages/{}/children", enc(page_id)), &q) {
            Ok(v) => v,
            Err(e) if e.to_string().contains("HTTP 404") => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        out.extend(v["results"].as_array().into_iter().flatten().cloned());
        match next_cursor(&v) {
            Value::String(next) => cursor = Some(next),
            _ => break,
        }
    }
    Ok(out)
}

fn build_tree_nodes(c: &Client, page_id: &str, depth: usize) -> Result<Vec<Value>> {
    if depth == 0 {
        return Ok(Vec::new());
    }
    let items = fetch_children_all(c, page_id)?;
    let mut nodes = Vec::with_capacity(items.len());
    for item in items {
        let child_id = item["id"].as_str().unwrap_or_default();
        let is_page = item["type"].as_str().is_none_or(|t| t == "page");
        let grandchildren = if depth > 1 && is_page && !child_id.is_empty() {
            build_tree_nodes(c, child_id, depth - 1)?
        } else {
            Vec::new()
        };
        nodes.push(json!({
            "id": item["id"],
            "title": item["title"],
            "type": item["type"],
            "status": item["status"],
            "children": grandchildren,
        }));
    }
    Ok(nodes)
}

/// `confluence tree`
pub fn tree(c: &Client, reference: &str, depth: usize) -> Result<Value> {
    let id = content_id(reference)?;
    let depth = depth.min(MAX_TREE_DEPTH);
    let page = c.get(&format!("/api/v2/pages/{}", enc(&id)), &[])?;
    let anc = c.get(
        &format!("/api/v2/pages/{}/ancestors", enc(&id)),
        &[("limit", "250")],
    )?;
    let ancestors: Vec<Value> = anc["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| json!({"id": a["id"], "type": a["type"], "title": a["title"]}))
        .collect();
    Ok(json!({
        "id": page["id"],
        "title": page["title"],
        "status": page["status"],
        "spaceId": page["spaceId"],
        "externalUrl": content_url(c, &page),
        "depth": depth,
        "ancestors": ancestors,
        "children": build_tree_nodes(c, &id, depth)?,
    }))
}

/// `confluence content labels`
pub fn labels(c: &Client, reference: &str, limit: usize) -> Result<Value> {
    let id = content_id(reference)?;
    let limit = limit.clamp(1, 250).to_string();
    let q = [("limit", limit.as_str())];
    let (_, v) = try_content(|kind| {
        c.get(
            &format!("/api/v2/{}/{}/labels", kind.collection(), enc(&id)),
            &q,
        )
    })?;
    Ok(listing(
        &v,
        |l| json!({"id": l["id"], "name": l["name"], "prefix": l["prefix"]}),
    ))
}

/// `confluence content delete`
pub fn delete(c: &Client, reference: &str, purge: bool) -> Result<Value> {
    let id = content_id(reference)?;
    let q: Vec<(&str, &str)> = if purge {
        vec![("purge", "true")]
    } else {
        Vec::new()
    };
    let (kind, _) =
        try_content(|k| c.delete(&format!("/api/v2/{}/{}", k.collection(), enc(&id)), &q))?;
    Ok(json!({
        "id": id,
        "type": format!("{kind:?}").to_lowercase(),
        "deleted": true,
        "purged": purge,
    }))
}

// --- spaces ------------------------------------------------------------------

/// A REST v2 space with its `externalUrl`.
fn space_summary(c: &Client, s: &Value) -> Value {
    let mut o = s.clone();
    if o.is_object() {
        o["externalUrl"] = format!(
            "{}{}",
            c.base(),
            s["_links"]["webui"].as_str().unwrap_or_default()
        )
        .into();
    }
    o
}

/// `confluence space list`
pub fn spaces(
    c: &Client,
    kind: Option<&str>,
    status: Option<&str>,
    keys: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    for (k, v) in [
        ("type", kind),
        ("status", status),
        ("keys", keys),
        ("cursor", cursor),
    ] {
        if let Some(v) = v {
            q.push((k, v));
        }
    }
    let v = c.get("/api/v2/spaces", &q)?;
    Ok(listing(&v, |s| space_summary(c, s)))
}

/// `confluence space get <key-or-id>`
pub fn space(c: &Client, identifier: &str) -> Result<Value> {
    let v = c.get(
        &format!("/api/v2/spaces/{}", enc(&space_id(c, identifier)?)),
        &[("description-format", "plain")],
    )?;
    Ok(space_summary(c, &v))
}

/// Labels of one space, as `[{id, name, prefix}]`.
pub fn space_labels(c: &Client, space_id: &str) -> Result<Value> {
    let v = c.get(
        &format!("/api/v2/spaces/{}/labels", enc(space_id)),
        &[("limit", "250")],
    )?;
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| json!({"id": l["id"], "name": l["name"], "prefix": l["prefix"]}))
        .collect())
}

/// `--labels` on `space get` / `space list`: add a `labels` array to one
/// space summary, or to each space in a list's `results`.
pub fn add_space_labels(c: &Client, v: &mut Value) -> Result<()> {
    if let Some(results) = v["results"].as_array_mut() {
        for s in results {
            add_space_labels(c, s)?;
        }
        return Ok(());
    }
    let id = v["id"]
        .as_str()
        .map(String::from)
        .or_else(|| v["id"].as_u64().map(|n| n.to_string()));
    if let Some(id) = id {
        v["labels"] = space_labels(c, &id)?;
    }
    Ok(())
}

fn match_personal_space<'a>(user: &Value, spaces: &'a [Value]) -> Option<&'a Value> {
    let account_id = user["accountId"].as_str();
    let personal_key = user["personalSpace"]["key"].as_str();
    if let Some(aid) = account_id {
        if let Some(s) = spaces
            .iter()
            .find(|s| s["authorId"].as_str() == Some(aid) || s["ownerId"].as_str() == Some(aid))
        {
            return Some(s);
        }
        let tilde_aid = format!("~{aid}");
        let tilde_compact = format!("~{}", aid.replace(':', ""));
        if let Some(s) = spaces
            .iter()
            .find(|s| matches!(s["key"].as_str(), Some(k) if k == tilde_aid || k == tilde_compact))
        {
            return Some(s);
        }
    }
    if let Some(pk) = personal_key {
        return spaces.iter().find(|s| s["key"].as_str() == Some(pk));
    }
    None
}

/// `confluence space me`
pub fn space_me(c: &Client) -> Result<Value> {
    let user = c.get("/rest/api/user/current", &[])?;
    let mut cursor: Option<String> = None;
    loop {
        let mut q = vec![("type", "personal"), ("limit", "250")];
        if let Some(ref cur) = cursor {
            q.push(("cursor", cur.as_str()));
        }
        let v = c.get("/api/v2/spaces", &q)?;
        let list = v["results"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if let Some(matched) = match_personal_space(&user, list) {
            if let Some(id) = matched["id"].as_str() {
                return space(c, id);
            }
            return Ok(space_summary(c, matched));
        }
        match next_cursor(&v) {
            Value::String(next) => cursor = Some(next),
            _ => break,
        }
    }
    let who = user["accountId"]
        .as_str()
        .or_else(|| user["displayName"].as_str())
        .unwrap_or("unknown");
    bail!("no personal Confluence space found for current user '{who}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type MockReply = (u16, Value);
    type RecordedReq = (String, String, String);

    fn mock_client(
        handler: impl Fn(&str, &str, &str) -> MockReply + Send + 'static,
    ) -> (Client, Arc<Mutex<Vec<RecordedReq>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_bg = Arc::clone(&log);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    let n = stream.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if header_end.is_none()
                        && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                    {
                        header_end = Some(pos + 4);
                        let hdrs = String::from_utf8_lossy(&buf[..pos]);
                        content_len = hdrs
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                    }
                    if let Some(end) = header_end
                        && buf.len() >= end + content_len
                    {
                        break;
                    }
                }
                let end = header_end.unwrap_or(buf.len());
                let head = String::from_utf8_lossy(&buf[..end]);
                let mut first = head.lines().next().unwrap_or_default().split_whitespace();
                let method = first.next().unwrap_or_default().to_string();
                let target = first.next().unwrap_or_default().to_string();
                let body = String::from_utf8_lossy(&buf[end..buf.len().min(end + content_len)])
                    .to_string();
                log_bg
                    .lock()
                    .unwrap()
                    .push((method.clone(), target.clone(), body.clone()));
                let (status, payload) = handler(&method, &target, &body);
                let json = if payload.is_null() {
                    String::new()
                } else {
                    payload.to_string()
                };
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                    json.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (
            Client::new(format!("http://{addr}"), "u@x.com", "tok", Some(2000)),
            log,
        )
    }

    #[test]
    fn content_ids_from_urls() {
        assert_eq!(content_id("123456").unwrap(), "123456");
        assert_eq!(
            content_id("https://a.atlassian.net/wiki/spaces/ENG/pages/98765/Runbook").unwrap(),
            "98765"
        );
        assert_eq!(
            content_id("https://a.atlassian.net/wiki/spaces/ENG/pages/edit-v2/98765").unwrap(),
            "98765"
        );
        assert_eq!(
            content_id("https://a.atlassian.net/wiki/spaces/ENG/blog/2026/01/02/55555/Post")
                .unwrap(),
            "55555"
        );
        assert_eq!(
            content_id("https://a.atlassian.net/wiki/pages/viewpage.action?pageId=4242").unwrap(),
            "4242"
        );
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

    #[test]
    fn spaces_supports_cursor_pagination() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = json!({
                "results": [{
                    "id": "101",
                    "key": "ENG",
                    "name": "Engineering",
                    "type": "global",
                    "status": "current",
                    "homepageId": "202",
                    "_links": {"webui": "/spaces/ENG"}
                }],
                "_links": {"next": "/wiki/api/v2/spaces?limit=10&cursor=page2%3D"}
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
            req
        });

        let c = Client::new(format!("http://{addr}/wiki"), "a@b.c", "tok", None);
        let out = spaces(
            &c,
            Some("global"),
            Some("current"),
            None,
            10,
            Some("page1="),
        )
        .unwrap();
        let req = handle.join().unwrap();
        let request_line = req.lines().next().unwrap_or_default();
        assert!(
            request_line.contains("/wiki/api/v2/spaces?"),
            "{request_line}"
        );
        assert!(request_line.contains("limit=10"), "{request_line}");
        assert!(request_line.contains("cursor=page1%3D"), "{request_line}");
        assert_eq!(out["results"][0]["key"], "ENG");
        assert_eq!(out["nextCursor"], "page2=");
    }

    #[test]
    fn personal_space_matching() {
        let user = json!({"accountId": "557058:abc-def", "personalSpace": {"key": "~legacy"}});
        let spaces = vec![
            json!({"id": "1", "key": "~other", "authorId": "other"}),
            json!({"id": "2", "key": "~nick", "authorId": "557058:abc-def"}),
        ];
        assert_eq!(match_personal_space(&user, &spaces).unwrap()["id"], "2");

        let spaces_by_key = vec![json!({"id": "3", "key": "~557058abc-def"})];
        assert_eq!(
            match_personal_space(&user, &spaces_by_key).unwrap()["id"],
            "3"
        );

        let spaces_legacy = vec![json!({"id": "4", "key": "~legacy"})];
        assert_eq!(
            match_personal_space(&user, &spaces_legacy).unwrap()["id"],
            "4"
        );

        let unrelated = vec![json!({"id": "5", "key": "~someone-else", "authorId": "999"})];
        assert!(match_personal_space(&user, &unrelated).is_none());
    }

    #[test]
    fn comments_list_and_create_endpoints() {
        let (c, log) = mock_client(|method, target, _| match (method, target) {
            ("GET", "/api/v2/pages/101/footer-comments?body-format=storage&limit=10") => (
                200,
                json!({
                    "results": [{
                        "id": "c1",
                        "version": {"authorId": "u1", "createdAt": "2026-01-01T00:00:00Z"},
                        "body": {"storage": {"value": "<p>Hello <strong>world</strong></p>"}},
                        "_links": {"webui": "/spaces/ENG/pages/101?focusedCommentId=c1"}
                    }],
                    "_links": {"next": "/wiki/api/v2/pages/101/footer-comments?cursor=c2"}
                }),
            ),
            ("GET", "/api/v2/pages/202/inline-comments?body-format=storage&limit=25") => {
                (404, json!({"message": "not found"}))
            }
            ("GET", "/api/v2/blogposts/202/inline-comments?body-format=storage&limit=25") => (
                200,
                json!({
                    "results": [{
                        "id": "ic1",
                        "resolutionStatus": "open",
                        "version": {"authorId": "u2", "createdAt": "2026-01-02T00:00:00Z"},
                        "body": {"storage": {"value": "<p>Fix typo</p>"}}
                    }]
                }),
            ),
            ("GET", "/api/v2/pages/202") => (404, json!({"message": "not found"})),
            ("GET", "/api/v2/blogposts/202") => (200, json!({"id": "202"})),
            ("POST", "/api/v2/footer-comments") => (
                201,
                json!({
                    "id": "fc9",
                    "status": "current",
                    "version": {"number": 1, "createdAt": "2026-01-03T00:00:00Z"},
                    "_links": {"webui": "/spaces/ENG/blog/202?focusedCommentId=fc9"}
                }),
            ),
            _ => (
                404,
                json!({"message": format!("unexpected {method} {target}")}),
            ),
        });

        let md = comments_list(&c, "101", false, ReadFormat::Md, 10, None).unwrap();
        assert_eq!(
            md["results"][0]["body"],
            json!({"format": "md", "value": "Hello **world**"})
        );
        assert_eq!(md["nextCursor"], "c2");

        let html = comments_list(&c, "101", false, ReadFormat::Html, 10, None).unwrap();
        assert_eq!(
            html["results"][0]["body"]["value"],
            "<p>Hello <strong>world</strong></p>"
        );

        let inline = comments_list(&c, "202", true, ReadFormat::Md, 25, None).unwrap();
        assert_eq!(inline["results"][0]["id"], "ic1");
        assert_eq!(inline["results"][0]["resolutionStatus"], "open");

        let created = comment_create(&c, "202", "Nice **post**", TextFormat::Markdown).unwrap();
        assert_eq!(created["id"], "fc9");
        let reqs = log.lock().unwrap();
        let post_body: Value = serde_json::from_str(&reqs.last().unwrap().2).unwrap();
        assert_eq!(post_body["blogPostId"], "202");
        assert_eq!(
            post_body["body"]["value"],
            "<p>Nice <strong>post</strong></p>"
        );
    }

    #[test]
    fn children_and_tree_hierarchy() {
        let (c, _) = mock_client(|_, target, _| match target {
            "/api/v2/pages/10/children?limit=5&cursor=cur1" => (
                200,
                json!({"results": [{"id": "11", "type": "page", "title": "Child 1", "status": "current", "spaceId": "1", "childPosition": 1}]}),
            ),
            "/api/v2/pages/10" => (
                200,
                json!({"id": "10", "title": "Root", "status": "current", "spaceId": "1", "_links": {"webui": "/spaces/S/pages/10"}}),
            ),
            "/api/v2/pages/10/ancestors?limit=250" => (
                200,
                json!({"results": [{"id": "1", "type": "page", "title": "Home"}]}),
            ),
            "/api/v2/pages/10/children?limit=250" => (
                200,
                json!({"results": [{"id": "11", "type": "page", "title": "Child 1", "status": "current"}]}),
            ),
            "/api/v2/pages/11/children?limit=250" => (
                200,
                json!({"results": [{"id": "12", "type": "page", "title": "Grandchild", "status": "current"}]}),
            ),
            "/api/v2/pages/12/children?limit=250" => (
                200,
                json!({"results": [{"id": "13", "type": "page", "title": "Too Deep", "status": "current"}]}),
            ),
            _ => (404, json!({"message": format!("unexpected {target}")})),
        });

        let ch = children(&c, "10", 5, Some("cur1")).unwrap();
        assert_eq!(ch["results"][0]["id"], "11");
        assert_eq!(ch["results"][0]["childPosition"], 1);

        let tr = tree(&c, "10", 2).unwrap();
        assert_eq!(tr["depth"], 2);
        assert_eq!(tr["ancestors"][0]["id"], "1");
        assert_eq!(tr["children"][0]["id"], "11");
        assert_eq!(tr["children"][0]["children"][0]["id"], "12");
        assert_eq!(tr["children"][0]["children"][0]["children"], json!([]));

        let capped = tree(&c, "10", 99).unwrap();
        assert_eq!(capped["depth"], MAX_TREE_DEPTH);
    }

    #[test]
    fn space_me_title_lookup_labels_and_delete() {
        let (c, log) = mock_client(|method, target, _| match (method, target) {
            ("GET", "/rest/api/user/current") => {
                (200, json!({"accountId": "acc-42", "displayName": "Ada"}))
            }
            ("GET", "/api/v2/spaces?type=personal&limit=250") => (
                200,
                json!({"results": [{"id": "77", "key": "~acc-42", "name": "Ada", "type": "personal", "status": "current", "authorId": "acc-42"}]}),
            ),
            ("GET", "/api/v2/spaces/77?description-format=plain") => (
                200,
                json!({"id": "77", "key": "~acc-42", "name": "Ada", "type": "personal", "status": "current", "homepageId": "78", "_links": {"webui": "/spaces/~acc-42"}, "description": {"plain": {"value": "My space"}}, "createdAt": "2026-01-01T00:00:00Z"}),
            ),
            ("GET", "/api/v2/spaces?keys=ENG") => {
                (200, json!({"results": [{"id": "55", "key": "ENG"}]}))
            }
            ("GET", "/api/v2/pages?space-id=55&title=Runbook&body-format=storage") => {
                (200, json!({"results": []}))
            }
            ("GET", "/api/v2/blogposts?space-id=55&title=Runbook&body-format=storage") => (
                200,
                json!({"results": [{"id": "901", "title": "Runbook", "status": "current", "spaceId": "55", "version": {"number": 2, "createdAt": "2026-01-01T00:00:00Z"}, "body": {"storage": {"value": "<p>Post</p>"}}, "_links": {"webui": "/spaces/ENG/blog/901"}}]}),
            ),
            ("GET", "/api/v2/pages/901/labels?limit=25") => (404, json!({})),
            ("GET", "/api/v2/blogposts/901/labels?limit=25") => (
                200,
                json!({"results": [{"id": "l1", "name": "ops", "prefix": "global"}]}),
            ),
            ("DELETE", "/api/v2/pages/901?purge=true") => (404, json!({})),
            ("DELETE", "/api/v2/blogposts/901?purge=true") => (204, Value::Null),
            _ => (
                404,
                json!({"message": format!("unexpected {method} {target}")}),
            ),
        });

        let me = space_me(&c).unwrap();
        assert_eq!(me["id"], "77");
        assert_eq!(me["description"]["plain"]["value"], "My space");

        let got = get(
            &c,
            "Runbook",
            &GetOpts {
                detail: Detail::Full,
                format: ReadFormat::Md,
                draft: false,
                reference_space: Some("ENG"),
                sections: &[],
            },
        )
        .unwrap();
        assert_eq!(got["id"], "901");
        assert_eq!(got["type"], "blogpost");
        assert_eq!(got["body"], json!({"format": "md", "value": "Post"}));
        assert_eq!(got["detail"], "full");
        assert_eq!(got["spaceId"], "55");

        let lbls = labels(&c, "901", 25).unwrap();
        assert_eq!(
            lbls["results"],
            json!([{"id": "l1", "name": "ops", "prefix": "global"}])
        );

        let del = delete(&c, "901", true).unwrap();
        assert_eq!(
            del,
            json!({"id": "901", "type": "blogpost", "deleted": true, "purged": true})
        );
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|(m, t, _)| m == "DELETE" && t == "/api/v2/blogposts/901?purge=true")
        );
    }
}

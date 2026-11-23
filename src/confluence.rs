//! Confluence Cloud: REST v2 for content and spaces, v1 where v2 has no
//! endpoint (CQL search, labels on content, watches, templates, restore).
//!
//! Bodies are read and written as ADF (`atlas_doc_format`) and shown as
//! Markdown through [`crate::markup`]. Storage format is only ever sent
//! unchanged: either a body the caller passed as `--body-format storage`,
//! after [`crate::markup::storage::check`], or the server's own storage
//! body sent back as it was read (title changes, restores, publishing a
//! draft). Nothing is converted to storage.

use crate::client::{ApiError, Client, ErrorKind};
use crate::markup::{
    Body, BodyFormat, Loss, decode_entities, quote, render_markdown, strip_highlight,
    unrepresentable,
};
use crate::output::notes;
use crate::time;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

pub mod attachments;
pub mod comments;
pub mod edits;
pub mod extras;
pub mod history;
pub mod labels;
pub mod lifecycle;
pub mod records;
pub mod sections;
pub mod tree;

pub(crate) fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ContentType {
    #[default]
    Page,
    Blogpost,
}

impl ContentType {
    pub(crate) fn collection(self) -> &'static str {
        match self {
            ContentType::Page => "pages",
            ContentType::Blogpost => "blogposts",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ContentType::Page => "page",
            ContentType::Blogpost => "blogpost",
        }
    }
}

/// The `body-format` query value a read in `format` asks for: storage for
/// storage, ADF for everything else.
pub(crate) fn body_param(format: BodyFormat) -> &'static str {
    match format {
        BodyFormat::Storage => "storage",
        _ => "atlas_doc_format",
    }
}

/// The ADF document in a v2 response fetched with `atlas_doc_format`.
pub(crate) fn adf_body(v: &Value) -> Value {
    let raw = &v["body"]["atlas_doc_format"]["value"];
    let doc = match raw {
        Value::String(s) => serde_json::from_str(s).ok(),
        Value::Object(_) => Some(raw.clone()),
        _ => None,
    };
    doc.filter(Value::is_object)
        .unwrap_or_else(|| json!({"type": "doc", "version": 1, "content": []}))
}

/// Plain text of an ADF document: one line per top-level block.
fn adf_text(doc: &Value) -> String {
    doc["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(sections::adf_plain)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render an ADF document in a read format. Markdown that cannot show a
/// node marks it and adds a `lossy_body` warning naming `what`.
pub(crate) fn render_adf(doc: &Value, format: BodyFormat, what: &str) -> Value {
    match format {
        BodyFormat::Adf => doc.clone(),
        BodyFormat::Plain => adf_text(doc).into(),
        BodyFormat::Markdown | BodyFormat::Storage => {
            let r = render_markdown(doc);
            warn_lossy(&r.lossy, what);
            r.text.into()
        }
    }
}

/// The `lossy_body` warning for `what` (e.g. `page 123`).
pub(crate) fn warn_lossy(lossy: &[Loss], what: &str) {
    if let Some(mut w) = crate::markup::lossy_warning(lossy) {
        w["input"] = what.into();
        notes::warn_value(w);
    }
}

/// The body of a v2 object read with [`body_param`]`(format)`, rendered.
pub(crate) fn rendered_body(v: &Value, format: BodyFormat, what: &str) -> Value {
    match format {
        BodyFormat::Storage => v["body"]["storage"]["value"].clone(),
        _ => render_adf(&adf_body(v), format, what),
    }
}

/// The `body` object a v2 write sends for `b`.
pub(crate) fn body_value(b: &Body) -> Result<Value> {
    Ok(match b {
        Body::Adf(doc) => json!({
            "representation": "atlas_doc_format",
            "value": serde_json::to_string(doc)?,
        }),
        Body::Storage(s) => json!({"representation": "storage", "value": s}),
    })
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
    Err(ApiError::usage(format!(
        "'{r}' is not a Confluence content ID or URL (pass --space to look it up by exact title)"
    ))
    .into())
}

/// The `cursor` parameter of a v2 `_links.next` link.
pub(crate) fn next_cursor(v: &Value) -> Value {
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

fn is_not_found(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::NotFound)
}

// --- search ------------------------------------------------------------------

/// One page of a CQL search (v1 `/rest/api/search`): `{results,
/// nextCursor}`, highlight markers removed from titles and excerpts.
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
    let base = v["_links"]["base"].as_str().unwrap_or(c.base()).to_string();
    if let Some(results) = v["results"].as_array_mut() {
        for r in results {
            for k in ["title", "excerpt"] {
                // Search returns them HTML-escaped, with highlight markers.
                if let Some(t) = r[k].as_str() {
                    r[k] = decode_entities(&strip_highlight(t)).trim().into();
                }
            }
            if let Some(u) = r["url"].as_str().filter(|u| !u.is_empty()) {
                r["externalUrl"] = format!("{base}{u}").into();
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

/// The CQL that a free-text `confluence search` runs: pages and blog posts,
/// most recently changed first.
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

/// Run a v2 operation on a page, falling back to a blog post on 404.
pub(crate) fn try_content<T>(
    mut f: impl FnMut(ContentType) -> Result<T>,
) -> Result<(ContentType, T)> {
    match f(ContentType::Page) {
        Ok(v) => Ok((ContentType::Page, v)),
        Err(e) if is_not_found(&e) => Ok((ContentType::Blogpost, f(ContentType::Blogpost)?)),
        Err(e) => Err(e),
    }
}

/// [`try_content`], or straight to `kind` when the caller named it.
pub(crate) fn with_kind<T>(
    kind: Option<ContentType>,
    mut f: impl FnMut(ContentType) -> Result<T>,
) -> Result<(ContentType, T)> {
    match kind {
        Some(k) => Ok((k, f(k)?)),
        None => try_content(f),
    }
}

/// Fetch a page or blog post with its body in `body_format` (a v2
/// `body-format` value).
fn fetch(
    c: &Client,
    id: &str,
    kind: Option<ContentType>,
    draft: bool,
    body_format: &str,
) -> Result<(ContentType, Value)> {
    let mut q = vec![("body-format", body_format)];
    if draft {
        q.push(("get-draft", "true"));
    }
    with_kind(kind, |k| {
        c.get(&format!("/api/v2/{}/{}", k.collection(), enc(id)), &q)
    })
}

/// `type` and `externalUrl` added to a REST v2 content object.
pub(crate) fn with_links(c: &Client, mut v: Value, kind: ContentType) -> Value {
    if v.is_object() {
        if v.get("type").is_none_or(Value::is_null) {
            v["type"] = kind.name().into();
        }
        v["externalUrl"] = optional_content_url(c, &v);
    }
    v
}

/// `{results, nextCursor}` from a REST v2 list response, each result passed
/// through `f`.
pub(crate) fn listing(v: &Value, f: impl Fn(&Value) -> Value) -> Value {
    let results: Vec<Value> = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(f)
        .collect();
    json!({"results": results, "nextCursor": next_cursor(v)})
}

pub(crate) fn content_url(c: &Client, v: &Value) -> String {
    let base = v["_links"]["base"].as_str().unwrap_or(c.base());
    format!(
        "{base}{}",
        v["_links"]["webui"].as_str().unwrap_or_default()
    )
}

pub(crate) fn optional_content_url(c: &Client, v: &Value) -> Value {
    match v["_links"]["webui"].as_str().filter(|s| !s.is_empty()) {
        Some(webui) => format!(
            "{}{webui}",
            v["_links"]["base"].as_str().unwrap_or(c.base())
        )
        .into(),
        None => Value::Null,
    }
}

/// Fetch by ID or URL, or by exact title in `space`.
fn resolve_and_fetch(
    c: &Client,
    reference: &str,
    space: Option<&str>,
    kind: Option<ContentType>,
    draft: bool,
    body_format: &str,
) -> Result<(ContentType, Value)> {
    if let Ok(id) = content_id(reference) {
        return fetch(c, &id, kind, draft, body_format);
    }
    let Some(space) = space else {
        return Err(content_id(reference).unwrap_err());
    };
    let sid = space_id(c, space)?;
    let mut q = vec![
        ("space-id", sid.as_str()),
        ("title", reference),
        ("body-format", body_format),
    ];
    if draft {
        q.push(("status", "draft"));
    }
    let kinds = match kind {
        Some(k) => vec![k],
        None => vec![ContentType::Page, ContentType::Blogpost],
    };
    for k in kinds {
        let v = c.get(&format!("/api/v2/{}", k.collection()), &q)?;
        if let Some(item) = v["results"].as_array().and_then(|a| a.first()) {
            return Ok((k, item.clone()));
        }
    }
    Err(ApiError::new(
        ErrorKind::NotFound,
        format!("no page or blog post titled '{reference}' in space '{space}'"),
    )
    .into())
}

/// The content ID for a reference: an ID or URL as is, otherwise an exact
/// title looked up in `space`.
pub fn resolve_id(c: &Client, reference: &str, space: Option<&str>) -> Result<String> {
    if let Ok(id) = content_id(reference) {
        return Ok(id);
    }
    let (_, v) = resolve_and_fetch(c, reference, space, None, false, "storage")?;
    v["id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("Confluence returned no ID for '{reference}'"))
}

/// What `page get` reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct GetOpts<'a> {
    pub format: BodyFormat,
    pub draft: bool,
    /// Space key or ID for looking a reference up by exact title.
    pub space: Option<&'a str>,
    /// Skip probing page vs blog post.
    pub kind: Option<ContentType>,
    /// `--section` headings; when non-empty only those sections are returned.
    pub sections: &'a [String],
    /// Return the heading outline instead of the body.
    pub outline: bool,
}

/// `page get`: the REST v2 object with `type`, `externalUrl`, and `body`
/// replaced by the body in `o.format` (or `outline` with `o.outline`).
pub fn get(c: &Client, reference: &str, o: &GetOpts) -> Result<Value> {
    let storage = o.format == BodyFormat::Storage;
    if storage && (o.outline || !o.sections.is_empty()) {
        return Err(ApiError::usage(
            "--outline and --section read the ADF body; drop --body-format storage",
        )
        .into());
    }
    let bf = body_param(o.format);
    let (kind, v) = resolve_and_fetch(c, reference, o.space, o.kind, o.draft, bf)?;
    let id = v["id"].as_str().unwrap_or(reference).to_string();
    let mut out = with_links(c, v, kind);
    let body = if o.outline {
        out["outline"] = sections::outline(&adf_body(&out)).into();
        Value::Null
    } else if storage {
        out["body"]["storage"]["value"].clone()
    } else {
        let mut doc = adf_body(&out);
        if !o.sections.is_empty() {
            doc = sections::select(&doc, o.sections)?;
        }
        render_adf(&doc, o.format, &format!("{} {id}", kind.name()))
    };
    out["body"] = body;
    out["bodyFormat"] = if o.outline {
        Value::Null
    } else {
        format!("{:?}", o.format).to_lowercase().into()
    };
    Ok(out)
}

/// Space key or numeric ID -> numeric ID.
pub(crate) fn space_id(c: &Client, space: &str) -> Result<String> {
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
        .ok_or_else(|| {
            ApiError::new(
                ErrorKind::NotFound,
                format!("no Confluence space with key '{space}'"),
            )
            .into()
        })
}

pub struct NewContent<'a> {
    pub space: &'a str,
    pub content_type: ContentType,
    pub title: &'a str,
    pub body: &'a Body,
    pub parent: Option<&'a str>,
    pub draft: bool,
    /// Only the creator can see and edit it.
    pub private: bool,
}

/// `page create`
pub fn create(c: &Client, n: NewContent) -> Result<Value> {
    if n.parent.is_some() && n.content_type == ContentType::Blogpost {
        return Err(ApiError::usage("blog posts can't have a parent").into());
    }
    let mut req = json!({
        "spaceId": space_id(c, n.space)?,
        "status": if n.draft { "draft" } else { "current" },
        "title": n.title,
        "body": body_value(n.body)?,
    });
    if let Some(p) = n.parent {
        req["parentId"] = content_id(p)?.into();
    }
    let q: &[(&str, &str)] = if n.private {
        &[("private", "true")]
    } else {
        &[]
    };
    let path = format!("/api/v2/{}", n.content_type.collection());
    let v = c.request("POST", &path, q, Some(&req))?;
    Ok(with_links(c, v, n.content_type))
}

/// What a version write changes besides the body.
#[derive(Clone, Copy, Debug, Default)]
pub struct VersionOpts<'a> {
    /// The version the caller read and edited (`--if-version`).
    pub if_version: u64,
    pub message: Option<&'a str>,
    pub minor_edit: bool,
    pub draft: bool,
    pub kind: Option<ContentType>,
}

/// `page update`: what changes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Update<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a Body>,
    /// The body was written in Markdown or plain text, so the current body
    /// must not hold anything those cannot express.
    pub check_loss: bool,
    /// Replace it anyway (`--allow-lossy`, which also needs `--yes`).
    pub allow_lossy: bool,
}

/// A checked write, ready to send with [`send`].
#[derive(Clone, Debug)]
pub struct Prepared {
    pub kind: ContentType,
    pub id: String,
    /// The version the write replaces.
    pub version: u64,
    pub request: Value,
    /// The ADF bodies before and after, when both are ADF (for the
    /// dry-run diff).
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub title_before: Value,
}

/// The `conflict` error for a stale `--if-version`.
fn stale(kind: ContentType, id: &str, current: u64, edited: u64) -> anyhow::Error {
    ApiError::new(
        ErrorKind::Conflict,
        format!(
            "{} {id} is at version {current}, you edited version {edited}",
            kind.name()
        ),
    )
    .with_hint(format!(
        "re-read it (`atlx confluence page get {id}`), re-apply your change, pass --if-version {current}"
    ))
    .into()
}

/// The refusal for a Markdown replace that would destroy `losses`.
pub(crate) fn lossy_replace(
    what: &str,
    id: &str,
    version: u64,
    losses: Vec<Loss>,
) -> anyhow::Error {
    let nodes = crate::markup::loss_nodes(&losses);
    let mut e = ApiError::refused(format!(
        "{what} {id} has content Markdown cannot express ({}); replacing its body with Markdown would destroy it",
        nodes.join(", ")
    ))
    .with_hint(if what == "comment" {
        format!(
            "read it with `atlx confluence page comment get {id} --body-format adf` and pass the whole body with --body-format adf; --allow-lossy --yes replaces it anyway"
        )
    } else {
        format!(
            "change only what you mean to with `atlx confluence page edit {id} --if-version {version} --edits ...`, or pass the whole body with --body-format adf; --allow-lossy --yes replaces it anyway"
        )
    });
    e.losses = serde_json::to_value(&losses).ok();
    e.into()
}

fn current_version(kind: ContentType, id: &str, cur: &Value, edited: u64) -> Result<u64> {
    let version = cur["version"]["number"]
        .as_u64()
        .ok_or_else(|| anyhow!("{} {id} came back without its version", kind.name()))?;
    if version != edited {
        return Err(stale(kind, id, version, edited));
    }
    Ok(version)
}

fn version_request(id: &str, title: &str, body: Value, version: u64, o: &VersionOpts) -> Value {
    json!({
        "id": id,
        "status": if o.draft { "draft" } else { "current" },
        "title": title,
        "body": body,
        "version": {
            "number": version + 1,
            "message": o.message.unwrap_or(""),
            "minorEdit": o.minor_edit,
        },
    })
}

/// Check and build a `page update`. Nothing is sent: a stale version is a
/// `conflict`, a lossy Markdown replace is `refused`.
pub fn prepare_update(
    c: &Client,
    reference: &str,
    u: &Update,
    o: &VersionOpts,
) -> Result<Prepared> {
    if u.title.is_none() && u.body.is_none() {
        return Err(
            ApiError::usage("nothing to update; pass --title or --body/--body-file").into(),
        );
    }
    let id = content_id(reference)?;
    // A new body is checked against the current ADF; a title change sends
    // the current storage body back exactly as it was read.
    let bf = if u.body.is_some() {
        "atlas_doc_format"
    } else {
        "storage"
    };
    let (kind, cur) = fetch(c, &id, o.kind, o.draft, bf)?;
    let version = current_version(kind, &id, &cur, o.if_version)?;
    let (body, before, after) = match u.body {
        Some(b) => {
            let before = adf_body(&cur);
            if u.check_loss && !u.allow_lossy {
                let losses = unrepresentable(&before);
                if !losses.is_empty() {
                    return Err(lossy_replace(kind.name(), &id, version, losses));
                }
            }
            let after = match b {
                Body::Adf(doc) => Some(doc.clone()),
                Body::Storage(_) => None,
            };
            (body_value(b)?, Some(before), after)
        }
        None => {
            let storage = cur["body"]["storage"]["value"].as_str().ok_or_else(|| {
                anyhow!(
                    "{} {id} came back without its body; not changing it",
                    kind.name()
                )
            })?;
            (
                json!({"representation": "storage", "value": storage}),
                None,
                None,
            )
        }
    };
    let title = u.title.or(cur["title"].as_str()).unwrap_or_default();
    Ok(Prepared {
        kind,
        id: id.clone(),
        version,
        request: version_request(&id, title, body, version, o),
        before,
        after,
        title_before: cur["title"].clone(),
    })
}

/// Check and build a `page edit`: the operations applied to the current
/// ADF body. Nodes the operations do not target are sent back unchanged.
pub fn prepare_edit(
    c: &Client,
    reference: &str,
    ops: &[edits::Op],
    title: Option<&str>,
    o: &VersionOpts,
) -> Result<Prepared> {
    let id = content_id(reference)?;
    let (kind, cur) = fetch(c, &id, o.kind, o.draft, "atlas_doc_format")?;
    let version = current_version(kind, &id, &cur, o.if_version)?;
    let before = adf_body(&cur);
    let after = edits::apply(&before, ops)?;
    let body = json!({
        "representation": "atlas_doc_format",
        "value": serde_json::to_string(&after)?,
    });
    let title = title.or(cur["title"].as_str()).unwrap_or_default();
    Ok(Prepared {
        kind,
        id: id.clone(),
        version,
        request: version_request(&id, title, body, version, o),
        before: Some(before),
        after: Some(after),
        title_before: cur["title"].clone(),
    })
}

/// Send a prepared write.
pub fn send(c: &Client, p: &Prepared) -> Result<Value> {
    let v = c.put(
        &format!("/api/v2/{}/{}", p.kind.collection(), enc(&p.id)),
        &p.request,
    )?;
    Ok(with_links(c, v, p.kind))
}

/// `page update`: [`prepare_update`] then [`send`].
pub fn update(c: &Client, reference: &str, u: &Update, o: &VersionOpts) -> Result<Value> {
    send(c, &prepare_update(c, reference, u, o)?)
}

/// A unified diff of a prepared write's body, as Markdown, when both sides
/// are ADF; the title change is a header line.
pub fn diff(p: &Prepared) -> Option<String> {
    let (Some(a), Some(b)) = (&p.before, &p.after) else {
        return None;
    };
    let lines =
        |d: &Value| -> Vec<String> { render_markdown(d).text.lines().map(String::from).collect() };
    let mut out = edits::unified_diff(
        &lines(a),
        &lines(b),
        &format!("{} version {}", p.id, p.version),
        &format!("{} version {}", p.id, p.version + 1),
    );
    if p.request["title"] != p.title_before {
        out = format!("title: {} -> {}\n{out}", p.title_before, p.request["title"]);
    }
    Some(out)
}

/// One page of `page list --space`.
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
    Ok(listing(&v, |p| with_links(c, p.clone(), kind)))
}

/// One page of `page list --parent`: the page's child pages.
pub fn children(c: &Client, reference: &str, limit: usize, cursor: Option<&str>) -> Result<Value> {
    let id = content_id(reference)?;
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let mut v = c.get(&format!("/api/v2/pages/{}/direct-children", enc(&id)), &q)?;
    // The list is mixed (whiteboards, folders, ...); this one lists pages,
    // so a page of results can be shorter than the limit and still go on.
    if let Some(items) = v["results"].as_array_mut() {
        items.retain(|i| i["type"] == "page");
    }
    Ok(listing(&v, |p| {
        let mut p = with_links(c, p.clone(), ContentType::Page);
        if p["parentId"].is_null() {
            p["parentId"] = id.clone().into();
        }
        p
    }))
}

// --- spaces ------------------------------------------------------------------

/// A REST space with its `externalUrl`.
pub(crate) fn space_summary(c: &Client, s: &Value) -> Value {
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

/// One page of `space list`.
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

/// `space get <key-or-id>`
pub fn space(c: &Client, identifier: &str) -> Result<Value> {
    let v = c.get(
        &format!(
            "/api/v2/spaces/{}",
            enc(&space_id(c, &lifecycle::exact_space(identifier)?)?)
        ),
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

/// `--labels`: add a `labels` array to one space, or to each space in an
/// array or a list's `results`.
pub fn add_space_labels(c: &Client, v: &mut Value) -> Result<()> {
    let items = match v {
        Value::Array(a) => Some(a),
        Value::Object(_) => v["results"].as_array_mut(),
        _ => None,
    };
    if let Some(items) = items {
        for s in items {
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

/// `space get --personal`: the signed-in user's personal space.
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
            Value::String(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
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
        let err = content_id("Some title").unwrap_err();
        assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
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
    fn bodies_are_sent_as_adf_or_unchanged_storage() {
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let v = body_value(&Body::Adf(adf.clone())).unwrap();
        assert_eq!(v["representation"], "atlas_doc_format");
        assert_eq!(
            serde_json::from_str::<Value>(v["value"].as_str().unwrap()).unwrap(),
            adf
        );
        let s = body_value(&Body::Storage("<p>x</p>".into())).unwrap();
        assert_eq!(s, json!({"representation": "storage", "value": "<p>x</p>"}));
    }
}

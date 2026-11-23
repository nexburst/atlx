//! `page comment`: footer and inline comment threads on REST v2.
//!
//! Bodies are read as ADF and shown in the requested format; new bodies
//! are sent as ADF (or checked storage). Replacing a body that holds
//! content Markdown cannot express is refused unless `allow_lossy`.

use super::{
    ContentType, adf_body, body_param, body_value, content_id, enc, listing, lossy_replace,
    next_cursor, optional_content_url, rendered_body, resolve_id, try_content, with_kind,
};
use crate::client::{ApiError, Client, ErrorKind};
use crate::markup::{Body, BodyFormat, unrepresentable};
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Value, json};

/// The most replies one command fetches.
pub const MAX_REPLIES: usize = 500;

/// Footer or inline: `--inline` on the commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Surface {
    Footer,
    Inline,
}

impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Surface::Footer => "footer",
            Surface::Inline => "inline",
        }
    }

    fn collection(self) -> &'static str {
        match self {
            Surface::Footer => "footer-comments",
            Surface::Inline => "inline-comments",
        }
    }
}

/// `--resolution-status`. `Open` also takes in reopened comments and those
/// whose anchor text was lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Resolution {
    Open,
    Resolved,
    /// atlx extra
    Reopened,
    /// atlx extra
    Dangling,
}

/// `--sort`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Sort {
    #[value(name = "created-date")]
    Oldest,
    #[value(name = "-created-date")]
    Newest,
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// A comment ID from a number, an ARI, or a URL that focuses a comment.
pub fn comment_id(r: &str) -> Result<String> {
    let r = r.trim();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(r) {
        return Ok(r.to_string());
    }
    if r.starts_with("ari:")
        && r.contains("comment")
        && let Some(id) = r.rsplit('/').next().filter(|s| digits(s))
    {
        return Ok(id.to_string());
    }
    if r.starts_with("https://") || r.starts_with("http://") {
        // A page URL also has the page's ID, so only comment markers count.
        for marker in ["focusedCommentId=", "commentId=", "comment-", "/comments/"] {
            if let Some(i) = r.find(marker) {
                let id: String = r[i + marker.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if digits(&id) {
                    return Ok(id);
                }
            }
        }
    }
    Err(usage(format!(
        "'{r}' is not a comment ID, ARI or comment URL"
    )))
}

fn path(surface: Surface, id: &str) -> String {
    format!("/api/v2/{}/{}", surface.collection(), enc(id))
}

/// A comment and its surface, trying footer then inline when none is
/// given; its body is in `body_format` (a v2 `body-format` value).
fn find(
    c: &Client,
    id: &str,
    surface: Option<Surface>,
    body_format: &str,
) -> Result<(Surface, Value)> {
    let read = |s: Surface| c.get(&path(s, id), &[("body-format", body_format)]);
    if let Some(s) = surface {
        return Ok((s, read(s)?));
    }
    match read(Surface::Footer) {
        Ok(v) => Ok((Surface::Footer, v)),
        Err(e) if ApiError::find(&e).is_some_and(|a| a.kind == ErrorKind::NotFound) => {
            Ok((Surface::Inline, read(Surface::Inline)?))
        }
        Err(e) => Err(e),
    }
}

/// A comment with its body rendered in `format`, its `commentType` and
/// `externalUrl`.
fn shown(c: &Client, surface: Surface, v: &Value, format: BodyFormat) -> Value {
    let id = v["id"].as_str().unwrap_or_default();
    let mut o = v.clone();
    if o.is_object() {
        o["body"] = rendered_body(v, format, &format!("comment {id}"));
        o["commentType"] = surface.name().into();
        o["externalUrl"] = optional_content_url(c, v);
    }
    o
}

/// The replies under a comment, nested. Draws on `budget`; `repliesTruncated`
/// marks a comment whose thread was cut when it ran out.
fn add_replies(
    c: &Client,
    surface: Surface,
    comment: &mut Value,
    format: BodyFormat,
    budget: &mut usize,
) -> Result<()> {
    let Some(id) = comment["id"].as_str().map(String::from) else {
        return Ok(());
    };
    let path = format!("{}/children", path(surface, &id));
    let mut replies = Vec::new();
    let mut cursor: Option<String> = None;
    let mut cut;
    loop {
        if *budget == 0 {
            cut = true;
            break;
        }
        let mut q = vec![("body-format", body_param(format)), ("limit", "100")];
        if let Some(cur) = &cursor {
            q.push(("cursor", cur));
        }
        let v = c.get(&path, &q)?;
        let page = v["results"].as_array().cloned().unwrap_or_default();
        for r in page.iter().take(*budget) {
            replies.push(shown(c, surface, r, format));
        }
        cut = page.len() > *budget;
        *budget = budget.saturating_sub(page.len());
        let next = next_cursor(&v).as_str().map(String::from);
        if cut || page.is_empty() || next.is_none() || next == cursor {
            break;
        }
        cursor = next;
    }
    for reply in &mut replies {
        add_replies(c, surface, reply, format, budget)?;
    }
    comment["replies"] = replies.into();
    if cut {
        comment["repliesTruncated"] = true.into();
    }
    Ok(())
}

pub struct ListOpts<'a> {
    pub surface: Surface,
    pub format: BodyFormat,
    pub limit: usize,
    pub cursor: Option<&'a str>,
    pub sort: Option<Sort>,
    pub status: Option<&'a str>,
    pub resolution: Option<Resolution>,
    pub content_type: Option<ContentType>,
    pub replies: bool,
}

/// `page comment list`: one page of top-level comments.
pub fn list(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    o: &ListOpts,
) -> Result<Value> {
    if o.resolution.is_some() && o.surface == Surface::Footer {
        return Err(usage(
            "Confluence's REST API only tracks resolution on inline comments; add --inline",
        ));
    }
    let id = match reference_space {
        Some(_) => resolve_id(c, reference, reference_space)?,
        None => content_id(reference)?,
    };
    let limit = o.limit.clamp(1, 250).to_string();
    let mut q = vec![
        ("body-format", body_param(o.format)),
        ("limit", limit.as_str()),
    ];
    if let Some(cur) = o.cursor {
        q.push(("cursor", cur));
    }
    if let Some(s) = o.sort {
        let s = match s {
            Sort::Oldest => "created-date",
            Sort::Newest => "-created-date",
        };
        q.push(("sort", s));
    }
    if let Some(s) = o.status {
        q.push(("status", s));
    }
    let states: &[&str] = match o.resolution {
        None => &[],
        Some(Resolution::Open) => &["open", "reopened", "dangling"],
        Some(Resolution::Resolved) => &["resolved"],
        Some(Resolution::Reopened) => &["reopened"],
        Some(Resolution::Dangling) => &["dangling"],
    };
    q.extend(states.iter().map(|s| ("resolution-status", *s)));
    let read = |k: ContentType| {
        c.get(
            &format!(
                "/api/v2/{}/{}/{}",
                k.collection(),
                enc(&id),
                o.surface.collection()
            ),
            &q,
        )
    };
    let (_, v) = with_kind(o.content_type, read)?;
    let mut out = listing(&v, |cm| shown(c, o.surface, cm, o.format));
    if o.replies
        && let Some(list) = out["results"].as_array_mut()
    {
        let mut budget = MAX_REPLIES;
        for comment in list {
            add_replies(c, o.surface, comment, o.format, &mut budget)?;
        }
    }
    Ok(out)
}

/// `page comment get`.
pub fn get(
    c: &Client,
    comment: &str,
    surface: Option<Surface>,
    format: BodyFormat,
    replies: bool,
) -> Result<Value> {
    let id = comment_id(comment)?;
    let (surface, v) = find(c, &id, surface, body_param(format))?;
    let mut out = shown(c, surface, &v, format);
    if replies {
        let mut budget = MAX_REPLIES;
        add_replies(c, surface, &mut out, format, &mut budget)?;
    }
    Ok(out)
}

/// Where an inline comment attaches: the text, and which occurrence.
#[derive(Clone, Copy, Debug)]
pub struct Selection<'a> {
    pub text: &'a str,
    /// Occurrences in the body; counted when not given.
    pub count: Option<usize>,
    /// Zero-based; the first when not given.
    pub index: Option<usize>,
}

fn written(c: &Client, surface: Surface, mut v: Value) -> Value {
    if v.is_object() {
        v["commentType"] = surface.name().into();
        v["externalUrl"] = optional_content_url(c, &v);
    }
    v
}

/// `page comment create`: inline when there is a selection, else footer.
pub fn create(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    body: &Body,
    selection: Option<Selection>,
) -> Result<Value> {
    let index = selection.and_then(|s| s.index).unwrap_or(0);
    if let Some(s) = selection {
        if s.text.trim().is_empty() {
            return Err(usage("--text-selection can't be empty"));
        }
        if let Some(n) = s.count
            && index >= n
        {
            return Err(usage(format!(
                "--text-selection-match-index {index} is out of range for {n} match(es); it is zero-based"
            )));
        }
    }
    let id = match reference_space {
        Some(_) => resolve_id(c, reference, reference_space)?,
        None => content_id(reference)?,
    };
    // Counting needs the body; otherwise the read only tells page from post.
    let counting = selection.is_some_and(|s| s.count.is_none());
    let q: &[(&str, &str)] = if counting {
        &[("body-format", "atlas_doc_format")]
    } else {
        &[]
    };
    let (kind, content) =
        try_content(|k| c.get(&format!("/api/v2/{}/{}", k.collection(), enc(&id)), q))?;
    let id_field = match kind {
        ContentType::Page => "pageId",
        ContentType::Blogpost => "blogPostId",
    };
    let mut req = json!({
        id_field: id,
        "body": body_value(body)?,
    });
    let mut surface = Surface::Footer;
    if let Some(s) = selection {
        let count = match s.count {
            Some(n) => n,
            None => visible_text(&adf_body(&content)).matches(s.text).count(),
        };
        if count == 0 {
            return Err(usage(format!(
                "the text {:?} doesn't appear in the content; copy it exactly, from within one paragraph",
                s.text
            )));
        }
        if index >= count {
            return Err(usage(format!(
                "--text-selection-match-index {index} is out of range: the text appears {count} time(s) and the index is zero-based"
            )));
        }
        req["inlineCommentProperties"] = json!({
            "textSelection": s.text,
            "textSelectionMatchCount": count,
            "textSelectionMatchIndex": index,
        });
        surface = Surface::Inline;
    }
    let v = c.post(&format!("/api/v2/{}", surface.collection()), &req)?;
    Ok(written(c, surface, v))
}

/// The text a reader can select: text runs join within a block, and a block
/// boundary breaks a match.
fn visible_text(doc: &Value) -> String {
    fn walk(node: &Value, out: &mut String) {
        match node["type"].as_str().unwrap_or_default() {
            "text" => out.push_str(node["text"].as_str().unwrap_or_default()),
            "hardBreak" => out.push('\n'),
            "mention" | "emoji" | "status" => {
                out.push_str(node["attrs"]["text"].as_str().unwrap_or_default())
            }
            kind => {
                for child in node["content"].as_array().into_iter().flatten() {
                    walk(child, out);
                }
                if matches!(kind, "paragraph" | "heading" | "codeBlock") {
                    out.push('\n');
                }
            }
        }
    }
    let mut out = String::new();
    walk(doc, &mut out);
    out
}

/// `page comment create --comment`: on the same surface as the comment.
pub fn reply(c: &Client, comment: &str, body: &Body) -> Result<Value> {
    let id = comment_id(comment)?;
    let (surface, _) = find(c, &id, None, "storage")?;
    let v = c.post(
        &format!("/api/v2/{}", surface.collection()),
        &json!({
            "parentCommentId": id,
            "body": body_value(body)?,
        }),
    )?;
    Ok(written(c, surface, v))
}

fn version(v: &Value, id: &str) -> Result<u64> {
    v["version"]["number"]
        .as_u64()
        .ok_or_else(|| anyhow!("comment {id} has no version number"))
}

/// How `page comment update` replaces a body.
#[derive(Clone, Copy, Debug, Default)]
pub struct Replace {
    /// The comment version the caller read (`--if-version`), if given.
    pub if_version: Option<u64>,
    /// The new body is Markdown or plain text: refuse when the current body
    /// holds content those cannot express...
    pub check_loss: bool,
    /// ...unless this is set (`--allow-lossy`).
    pub allow_lossy: bool,
}

/// `page comment update` with a new body.
pub fn update(
    c: &Client,
    comment: &str,
    surface: Option<Surface>,
    body: &Body,
    r: Replace,
) -> Result<Value> {
    let id = comment_id(comment)?;
    let (surface, current) = find(c, &id, surface, "atlas_doc_format")?;
    let n = version(&current, &id)?;
    if let Some(expected) = r.if_version
        && expected != n
    {
        return Err(ApiError::new(
            ErrorKind::Conflict,
            format!("comment {id} is at version {n}, you edited version {expected}"),
        )
        .with_hint(format!(
            "re-read it (`atlx confluence page comment get {id}`), re-apply your change, pass --if-version {n}"
        ))
        .into());
    }
    if r.check_loss && !r.allow_lossy {
        let losses = unrepresentable(&adf_body(&current));
        if !losses.is_empty() {
            return Err(lossy_replace("comment", &id, n, losses));
        }
    }
    let v = c.put(
        &path(surface, &id),
        &json!({
            "version": {"number": n + 1},
            "body": body_value(body)?,
        }),
    )?;
    Ok(written(c, surface, v))
}

/// `page comment delete`. Confluence deletes comments for good.
pub fn delete(c: &Client, comment: &str) -> Result<Value> {
    let id = comment_id(comment)?;
    let (surface, _) = find(c, &id, None, "storage")?;
    c.delete(&path(surface, &id), &[])?;
    Ok(json!({"id": id, "commentType": surface.name(), "deleted": true}))
}

/// `page comment update --resolve|--reopen`.
pub fn set_resolved(
    c: &Client,
    comment: &str,
    surface: Option<Surface>,
    resolved: bool,
) -> Result<Value> {
    const FOOTER: &str =
        "Confluence's REST API can only resolve and reopen inline comments, not footer comments";
    let id = comment_id(comment)?;
    if surface == Some(Surface::Footer) {
        return Err(usage(FOOTER));
    }
    // The update replaces the body, so it is read as storage and sent back
    // exactly as it was.
    let (surface, current) = find(c, &id, surface, "storage")?;
    if surface == Surface::Footer {
        return Err(usage(FOOTER));
    }
    let body = current["body"]["storage"]["value"]
        .as_str()
        .ok_or_else(|| anyhow!("comment {id} came back without its body; not changing it"))?;
    let v = c.put(
        &path(surface, &id),
        &json!({
            "version": {"number": version(&current, &id)? + 1},
            "body": {"representation": "storage", "value": body},
            "resolved": resolved,
        }),
    )?;
    Ok(written(c, surface, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_text_joins_marks_and_breaks_blocks() {
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Run the "},
                {"type": "text", "text": "deploy", "marks": [{"type": "strong"}]}
            ]},
            {"type": "heading", "content": [{"type": "text", "text": "now"}]}
        ]});
        let text = visible_text(&doc);
        assert!(text.contains("the deploy"));
        assert!(!text.contains("deploy now") && !text.contains("deploynow"));
    }
}

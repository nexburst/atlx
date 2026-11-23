//! What a Markdown round trip would destroy.
//!
//! [`unrepresentable`] is the check behind "a full-body Markdown replace is
//! refused when the current body has nodes Markdown cannot express": it
//! reports what the renderer cannot show, and, when the renderer reports
//! nothing, renders the document, parses it back and compares the two trees
//! structurally ([`canonical`]), so a mapping that is not truly two-way is
//! caught too.

use serde_json::{Map, Value, json};

use super::adf::{finalize, kind};
use super::{Loss, markdown_to_adf, render_markdown};

/// Nodes of `adf` (a `doc`, or a single node) that Markdown cannot express.
/// Empty means `adf -> Markdown -> adf` gives the same document, up to
/// [`canonical`].
pub fn unrepresentable(adf: &Value) -> Vec<Loss> {
    let rendered = render_markdown(adf);
    if !rendered.lossy.is_empty() {
        return rendered.lossy;
    }
    let doc = if kind(adf) == "doc" {
        adf.clone()
    } else {
        json!({"type": "doc", "version": 1, "content": [adf]})
    };
    let mut losses = Vec::new();
    let original = finalize(doc, &mut losses);
    if !losses.is_empty() {
        return losses;
    }
    let back = markdown_to_adf(&rendered.text);
    if !back.lost.is_empty() {
        return back.lost;
    }
    match first_difference(&canonical(&original), &canonical(&back.adf), "") {
        None => Vec::new(),
        Some((at, node)) => vec![Loss::new(
            node,
            at,
            "this part does not survive a Markdown round trip",
        )],
    }
}

/// Whether two ADF documents are the same up to [`canonical`].
pub fn structurally_equal(a: &Value, b: &Value) -> bool {
    canonical(a) == canonical(b)
}

/// The parts of an ADF tree that carry content: `localId`s, internal
/// `__`-prefixed attributes, layout-only attributes (table and image
/// widths, column widths, cell backgrounds, card sizes), layout-only marks
/// (`breakout`, `border`) and empty `content` arrays are removed; defaults
/// (`order: 1`, `colspan: 1`) are dropped; mention text always starts with
/// `@`.
pub fn canonical(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        Value::Object(o) => {
            let ty = o.get("type").and_then(Value::as_str).unwrap_or("");
            let mut out = Map::new();
            for (k, val) in o {
                match k.as_str() {
                    "attrs" => {
                        if let Some(a) = val.as_object().map(|a| attrs(ty, a))
                            && !a.is_empty()
                        {
                            out.insert(k.clone(), Value::Object(a));
                        }
                    }
                    "marks" => {
                        let ms: Vec<Value> = val
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|m| !matches!(kind(m), "breakout" | "border"))
                            .map(canonical)
                            .collect();
                        if !ms.is_empty() {
                            out.insert(k.clone(), Value::Array(ms));
                        }
                    }
                    "content" if val.as_array().is_some_and(Vec::is_empty) => {}
                    _ => {
                        out.insert(k.clone(), canonical(val));
                    }
                }
            }
            Value::Object(out)
        }
        _ => v.clone(),
    }
}

fn attrs(ty: &str, a: &Map<String, Value>) -> Map<String, Value> {
    let keep: Option<&[&str]> = match ty {
        "table" | "mediaSingle" | "taskList" => Some(&[]),
        "tableCell" | "tableHeader" => Some(&["colspan", "rowspan"]),
        "media" => Some(&["type", "url", "alt", "id", "collection"]),
        "emoji" => Some(&["shortName"]),
        "mention" => Some(&["id", "text"]),
        "status" => Some(&["text", "color"]),
        "inlineCard" | "blockCard" | "embedCard" => Some(&["url", "data"]),
        "link" => Some(&["href", "title"]),
        _ => None,
    };
    let mut out = Map::new();
    for (k, v) in a {
        if k == "localId"
            || k.starts_with("__")
            || keep.is_some_and(|keep| !keep.contains(&k.as_str()))
        {
            continue;
        }
        let default = match (ty, k.as_str()) {
            ("orderedList", "order") => v == &json!(1),
            ("tableCell" | "tableHeader", "colspan" | "rowspan") => v == &json!(1),
            _ => false,
        };
        if default || v.is_null() {
            continue;
        }
        let v = match (ty, k.as_str(), v.as_str()) {
            ("mention", "text", Some(t)) if !t.is_empty() && !t.starts_with('@') => {
                Value::String(format!("@{t}"))
            }
            ("status", "color", Some(c)) => Value::String(c.to_ascii_lowercase()),
            _ => v.clone(),
        };
        out.insert(k.clone(), v);
    }
    out
}

/// The JSON pointer and node type of the first place `a` and `b` differ.
fn first_difference(a: &Value, b: &Value, at: &str) -> Option<(String, String)> {
    if a == b {
        return None;
    }
    let here = || {
        let ty = a["type"].as_str().or(b["type"].as_str()).unwrap_or("node");
        Some((
            if at.is_empty() { "/".into() } else { at.into() },
            ty.to_string(),
        ))
    };
    let (Some(ka), Some(kb)) = (a["content"].as_array(), b["content"].as_array()) else {
        return here();
    };
    let mut sa = a.clone();
    let mut sb = b.clone();
    sa.as_object_mut().map(|o| o.remove("content"));
    sb.as_object_mut().map(|o| o.remove("content"));
    if sa != sb {
        return here();
    }
    for i in 0..ka.len().max(kb.len()) {
        let p = format!("{at}/content/{i}");
        match (ka.get(i), kb.get(i)) {
            (Some(x), Some(y)) => {
                if let Some(d) = first_difference(x, y, &p) {
                    return Some(d);
                }
            }
            (Some(x), None) | (None, Some(x)) => {
                return Some((p, x["type"].as_str().unwrap_or("node").to_string()));
            }
            (None, None) => {}
        }
    }
    here()
}

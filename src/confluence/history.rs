//! `page version list|get|diff|restore`: a page's or blog post's version
//! history.
//!
//! Lists and version details come from REST v2; the body at a version is a
//! v2 read with `version=N`, as ADF. Diffs are computed locally with the
//! line diff from [`super::edits`]. Restoring uses v1, the only public
//! endpoint for it.

use super::ContentType;
use super::{adf_body, body_param, enc, listing, rendered_body, resolve_id, try_content};
use crate::client::{ApiError, Client};
use crate::markup::{BodyFormat, render_markdown};
use anyhow::Result;
use serde_json::{Value, json};

/// One page of `page version list`, newest first: `{results, nextCursor}`.
pub fn list(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str()), ("sort", "-modified-date")];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let (_, v) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/versions", k.collection(), enc(&id)),
            &q,
        )
    })?;
    Ok(listing(&v, Value::clone))
}

/// The content at one version, with its body in `body-format`.
fn fetch_at(
    c: &Client,
    kind: ContentType,
    id: &str,
    version: u64,
    body_format: &str,
) -> Result<Value> {
    let n = version.to_string();
    c.get(
        &format!("/api/v2/{}/{}", kind.collection(), enc(id)),
        &[("version", n.as_str()), ("body-format", body_format)],
    )
}

/// `page version get N`: the version's details, with the title and body
/// (in `format`) it had.
pub fn get(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    version: u64,
    format: BodyFormat,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let (kind, meta) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/versions/{version}", k.collection(), enc(&id)),
            &[],
        )
    })?;
    let v = fetch_at(c, kind, &id, version, body_param(format))?;
    let mut out = meta;
    if !out.is_object() {
        out = json!({"number": version});
    }
    out["contentId"] = id.clone().into();
    out["type"] = kind.name().into();
    out["title"] = v["title"].clone();
    out["body"] = rendered_body(
        &v,
        format,
        &format!("{} {id} version {version}", kind.name()),
    );
    Ok(out)
}

/// The lines a diff compares: the Markdown text's lines, or one top-level
/// ADF block (as compact JSON) per line.
fn diff_lines(c: &Client, kind: ContentType, id: &str, version: u64, adf: bool) -> Result<Value> {
    let v = fetch_at(c, kind, id, version, "atlas_doc_format")?;
    let doc = adf_body(&v);
    let lines: Vec<String> = if adf {
        doc["content"]
            .as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string)
            .collect()
    } else {
        render_markdown(&doc)
            .text
            .lines()
            .map(String::from)
            .collect()
    };
    Ok(json!({"title": v["title"], "lines": lines}))
}

/// `page version diff --from N --to M`: a unified diff of the two bodies,
/// computed locally, as Markdown (default) or ADF blocks.
pub fn diff(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    from: u64,
    to: u64,
    format: BodyFormat,
) -> Result<Value> {
    if from == 0 || to == 0 {
        return Err(ApiError::usage("version numbers start at 1").into());
    }
    let adf = match format {
        BodyFormat::Markdown => false,
        BodyFormat::Adf => true,
        _ => {
            return Err(
                ApiError::usage("version diff compares markdown (the default) or adf").into(),
            );
        }
    };
    let id = resolve_id(c, reference, reference_space)?;
    let (kind, a) = try_content(|k| diff_lines(c, k, &id, from, adf))?;
    let b = diff_lines(c, kind, &id, to, adf)?;
    let strings = |v: &Value| -> Vec<String> {
        v["lines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().map(String::from))
            .collect()
    };
    let (la, lb) = (strings(&a), strings(&b));
    let diff = super::edits::unified_diff(
        &la,
        &lb,
        &format!("{id} version {from}"),
        &format!("{id} version {to}"),
    );
    Ok(json!({
        "id": id,
        "type": kind.name(),
        "from": from,
        "to": to,
        "format": if adf { "adf" } else { "markdown" },
        "titleChanged": a["title"] != b["title"],
        "changed": !diff.is_empty() || a["title"] != b["title"],
        "diff": diff,
    }))
}

/// `page version restore N`: v1 restore, which adds a new latest version
/// with the old body (and the old title with `restore_title`).
pub fn restore(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    version: u64,
    message: Option<&str>,
    restore_title: bool,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let req = json!({
        "operationKey": "restore",
        "params": {
            "versionNumber": version,
            "message": message.unwrap_or(""),
            "restoreTitle": restore_title,
        },
    });
    let v = c.post(&format!("/rest/api/content/{}/version", enc(&id)), &req)?;
    Ok(json!({
        "contentId": id,
        "restoredFrom": version,
        "number": v["number"],
        "message": v["message"],
        "createdAt": v["when"],
        "authorId": v["by"]["accountId"],
        "minorEdit": v["minorEdit"],
    }))
}

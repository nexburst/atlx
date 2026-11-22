//! `content versions list|get|diff|restore` (also reachable as
//! `content history list|get|diff`): a page's or blog post's version history.
//!
//! Lists and version details come from REST v2; the historical body is a v2
//! read with `version=N`. Diffs are computed locally with the line diff from
//! [`super::edits`]. Restoring uses v1, which is the only public endpoint for it.

use super::{ContentType, ReadFormat, adf_body, enc, listing, resolve_id, try_content};
use crate::client::{ApiError, Client};
use crate::markup::{adf_to_storage, storage_to_text};
use anyhow::Result;
use serde_json::{Value, json};

fn kind_name(kind: ContentType) -> String {
    format!("{kind:?}").to_lowercase()
}

/// `content versions list`: newest first, `{id, type, results, nextCursor}`.
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
    let (kind, v) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/versions", k.collection(), enc(&id)),
            &q,
        )
    })?;
    let mut out = listing(&v, Value::clone);
    out["id"] = id.into();
    out["type"] = kind_name(kind).into();
    Ok(out)
}

/// The content at one version, with its body in `body-format`.
fn fetch_at(c: &Client, kind: ContentType, id: &str, version: u64, adf: bool) -> Result<Value> {
    let n = version.to_string();
    let bf = if adf { "atlas_doc_format" } else { "storage" };
    c.get(
        &format!("/api/v2/{}/{}", kind.collection(), enc(id)),
        &[("version", n.as_str()), ("body-format", bf)],
    )
}

/// `content versions get --at-version N [--format md|html]`. Without a
/// format only the version's metadata is returned.
pub fn get(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    version: u64,
    format: Option<ReadFormat>,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let (kind, meta) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/versions/{version}", k.collection(), enc(&id)),
            &[],
        )
    })?;
    let mut out = json!({"id": id, "type": kind_name(kind), "version": meta});
    if let Some(format) = format {
        let v = fetch_at(c, kind, &id, version, false)?;
        let storage = v["body"]["storage"]["value"].as_str().unwrap_or_default();
        let (name, value) = match format {
            ReadFormat::Md => ("md", storage_to_text(storage)),
            _ => ("html", storage.to_string()),
        };
        out["title"] = v["title"].clone();
        out["body"] = json!({"format": name, "value": value});
    }
    Ok(out)
}

/// The lines a diff compares: one top-level block per line for HTML, the
/// Markdown text's own lines for `md`.
fn diff_lines(c: &Client, kind: ContentType, id: &str, version: u64, md: bool) -> Result<Value> {
    let v = fetch_at(c, kind, id, version, !md)?;
    let lines: Vec<String> = if md {
        let storage = v["body"]["storage"]["value"].as_str().unwrap_or_default();
        storage_to_text(storage).lines().map(String::from).collect()
    } else {
        adf_body(&v)["content"]
            .as_array()
            .into_iter()
            .flatten()
            .map(adf_to_storage)
            .collect()
    };
    Ok(json!({"title": v["title"], "lines": lines}))
}

/// `content versions diff --from N --to M [--format md|html]`: a unified
/// diff of the two bodies, computed locally.
pub fn diff(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    from: u64,
    to: u64,
    format: ReadFormat,
) -> Result<Value> {
    if from == 0 || to == 0 {
        return Err(ApiError::usage("version numbers start at 1").into());
    }
    let id = resolve_id(c, reference, reference_space)?;
    let md = format == ReadFormat::Md;
    let (kind, a) = try_content(|k| diff_lines(c, k, &id, from, md))?;
    let b = diff_lines(c, kind, &id, to, md)?;
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
        "type": kind_name(kind),
        "from": from,
        "to": to,
        "format": if md { "md" } else { "html" },
        "titleChanged": a["title"] != b["title"],
        "changed": !diff.is_empty() || a["title"] != b["title"],
        "diff": diff,
    }))
}

/// `content versions restore --at-version N`: v1 restore, which adds a new
/// latest version with the old body (and the old title with
/// `--restore-title`).
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
        "id": id,
        "restoredFrom": version,
        "version": {
            "number": v["number"],
            "message": v["message"],
            "createdAt": v["when"],
            "authorId": v["by"]["accountId"],
            "minorEdit": v["minorEdit"],
        },
    }))
}

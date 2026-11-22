//! `content attachments list|get|download|upload|delete`.
//!
//! Reads and deletes use REST v2. Uploads go through v1, the only public
//! endpoint that accepts file data: a new file name creates an attachment, an
//! existing one adds a version to it.

use super::{enc, listing, optional_content_url, resolve_id, try_content};
use crate::client::{ApiError, Client, Part};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Confluence's XSRF opt-out for v1 multipart uploads.
const NO_CHECK: (&str, &str) = ("X-Atlassian-Token", "nocheck");

/// An attachment ID: `att123`, a bare number (taken as `att123`), or a URL
/// carrying either.
pub fn attachment_id(r: &str) -> Result<String> {
    let r = r.trim();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let pick = |s: &str| -> Option<String> {
        if digits(s) {
            Some(format!("att{s}"))
        } else {
            s.strip_prefix("att")
                .filter(|n| digits(n))
                .map(|_| s.to_string())
        }
    };
    if let Some(id) = pick(r) {
        return Ok(id);
    }
    // ...?preview=/123/att456/file.png, .../attachments/att456, ?attachmentId=456
    if let Some(i) = r.find("attachmentId=") {
        let n: String = r[i + 13..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Some(id) = pick(&n) {
            return Ok(id);
        }
    }
    if let Some(id) = r
        .split(['/', '?', '#', '&', '='])
        .find(|s| s.starts_with("att"))
        .and_then(pick)
    {
        return Ok(id);
    }
    Err(ApiError::usage(format!(
        "'{r}' is not a Confluence attachment ID (like att12345) or attachment URL"
    ))
    .into())
}

/// A v2 attachment with `externalUrl` (its web page) added.
fn with_url(c: &Client, a: &Value) -> Value {
    let mut a = a.clone();
    if a.is_object() {
        a["externalUrl"] = optional_content_url(c, &a);
    }
    a
}

/// The local name for a downloaded attachment: the last component of the
/// name on the server, which anyone who can attach a file chooses. Hidden
/// names are refused, so a download can't plant a `.bashrc` or `.envrc`.
fn safe_file_name(title: &str) -> Option<&str> {
    let name = title.rsplit(['/', '\\']).next()?;
    let unsafe_name = name.trim().is_empty()
        || name.starts_with('.')
        || name.contains(':')
        || name.chars().any(char::is_control);
    (!unsafe_name).then_some(name)
}

pub struct ListOpts<'a> {
    pub limit: usize,
    pub cursor: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub status: Option<&'a str>,
    pub media_type: Option<&'a str>,
    pub filename: Option<&'a str>,
}

/// `content attachments list --id`
pub fn list(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    o: &ListOpts,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let limit = o.limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    for (k, v) in [
        ("cursor", o.cursor),
        ("sort", o.sort),
        ("status", o.status),
        ("mediaType", o.media_type),
        ("filename", o.filename),
    ] {
        if let Some(v) = v {
            q.push((k, v));
        }
    }
    let (_, v) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/attachments", k.collection(), enc(&id)),
            &q,
        )
    })?;
    Ok(listing(&v, |a| with_url(c, a)))
}

/// `content attachments get --attachment-id [--at-version]`
pub fn get(c: &Client, attachment: &str, version: Option<u64>) -> Result<Value> {
    let id = attachment_id(attachment)?;
    let n = version.map(|v| v.to_string());
    let q: Vec<(&str, &str)> = n.iter().map(|n| ("version", n.as_str())).collect();
    let v = c.get(&format!("/api/v2/attachments/{}", enc(&id)), &q)?;
    Ok(with_url(c, &v))
}

/// `content attachments download --attachment-id [--out] [--at-version]
/// [--force]`. Without `--out` the file is saved under its own name in the
/// current directory.
pub fn download(
    c: &Client,
    attachment: &str,
    out: Option<&Path>,
    version: Option<u64>,
    force: bool,
) -> Result<Value> {
    let meta = get(c, attachment, version)?;
    let link = meta["downloadLink"]
        .as_str()
        .or_else(|| meta["_links"]["download"].as_str())
        .filter(|l| !l.is_empty())
        .ok_or_else(|| anyhow!("attachment {} has no download link", meta["id"]))?;
    let dest: PathBuf = match out {
        Some(p) => p.to_path_buf(),
        None => {
            let title = meta["title"].as_str().unwrap_or_default();
            PathBuf::from(safe_file_name(title).ok_or_else(|| {
                ApiError::usage(format!(
                    "the attachment's name {title:?} is not safe to use as a file name; pass --out"
                ))
            })?)
        }
    };
    let bytes = c.download_to(link, &[], &dest, force)?;
    Ok(json!({
        "attachmentId": meta["id"],
        "title": meta["title"],
        "mediaType": meta["mediaType"],
        "version": meta["version"]["number"],
        "path": dest.display().to_string(),
        "bytes": bytes,
    }))
}

/// `content attachments upload --id --file [--comment --minor-edit]`. Returns the
/// attachment with `action: "created"` or `"updated"`.
pub fn upload(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    file: &Path,
    comment: Option<&str>,
    minor_edit: bool,
) -> Result<Value> {
    if !file.is_file() {
        return Err(ApiError::usage(format!("{} is not a readable file", file.display())).into());
    }
    let filename = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| ApiError::usage(format!("{} has no file name", file.display())))?;
    let id = resolve_id(c, reference, reference_space)?;
    // An attachment with this name already on the content gets a new version.
    let (_, found) = try_content(|k| {
        c.get(
            &format!("/api/v2/{}/{}/attachments", k.collection(), enc(&id)),
            &[("filename", filename.as_str()), ("limit", "1")],
        )
    })?;
    let existing = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["title"].as_str() == Some(filename.as_str()))
        .and_then(|a| a["id"].as_str())
        .map(String::from);
    let mut parts = vec![Part::file("file", file)];
    if let Some(cm) = comment {
        parts.push(Part::text("comment", cm));
    }
    parts.push(Part::text("minorEdit", minor_edit.to_string()));
    let (path, action) = match &existing {
        Some(att) => (
            format!(
                "/rest/api/content/{}/child/attachment/{}/data",
                enc(&id),
                enc(att)
            ),
            "updated",
        ),
        None => (
            format!("/rest/api/content/{}/child/attachment", enc(&id)),
            "created",
        ),
    };
    let v = c.post_multipart(&path, &parts, &[NO_CHECK])?;
    // Creating returns a page of results; adding a version returns one object.
    let a = v["results"]
        .as_array()
        .and_then(|r| r.first())
        .cloned()
        .unwrap_or(v);
    let base = a["_links"]["base"].as_str().unwrap_or(c.base());
    let webui = a["_links"]["webui"].as_str().filter(|s| !s.is_empty());
    Ok(json!({
        "id": a["id"],
        "title": a["title"],
        "mediaType": a["extensions"]["mediaType"],
        "fileSize": a["extensions"]["fileSize"],
        "comment": a["extensions"]["comment"],
        "version": a["version"]["number"],
        "contentId": id,
        "action": action,
        "externalUrl": webui.map(|w| format!("{base}{w}")),
    }))
}

/// `content attachments delete --attachment-id`: moves it to the trash, or
/// removes an already trashed one for good with `purge`.
pub fn delete(c: &Client, attachment: &str, purge: bool) -> Result<Value> {
    let id = attachment_id(attachment)?;
    let q: &[(&str, &str)] = if purge { &[("purge", "true")] } else { &[] };
    c.delete(&format!("/api/v2/attachments/{}", enc(&id)), q)?;
    let mut out = json!({"id": id, "deleted": true});
    if purge {
        out["purged"] = true.into();
    }
    Ok(out)
}

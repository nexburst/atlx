//! `jira workitem attachment query|get|download|thumbnail|upload|delete`.

use super::{enc, strip_media};
use crate::client::{ApiError, Client, Part};
use anyhow::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Jira's XSRF opt-out for multipart uploads.
const NO_CHECK: (&str, &str) = ("X-Atlassian-Token", "no-check");

/// The issue's `attachment` field.
pub fn query(c: &Client, key: &str) -> Result<Value> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[("fields", "attachment")],
    )?;
    let mut list = match &v["fields"]["attachment"] {
        Value::Array(a) => Value::Array(a.clone()),
        _ => json!([]),
    };
    strip_media(&mut list);
    Ok(list)
}

pub fn get(c: &Client, id: &str) -> Result<Value> {
    let mut v = c.get(&format!("/rest/api/3/attachment/{}", enc(id)), &[])?;
    strip_media(&mut v);
    Ok(v)
}

/// Which bytes of an attachment to save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bytes {
    Content,
    Thumbnail,
}

/// Save an attachment or its thumbnail. Jira redirects to the media service;
/// the client drops the credentials when that leaves the site. Without `out`
/// the file takes the attachment's own name, if that name is safe.
pub fn download(
    c: &Client,
    id: &str,
    what: Bytes,
    out: Option<&Path>,
    force: bool,
) -> Result<Value> {
    let (segment, prefix) = match what {
        Bytes::Content => ("content", ""),
        Bytes::Thumbnail => ("thumbnail", "thumbnail-"),
    };
    let dest = match out {
        Some(p) => p.to_path_buf(),
        None => {
            let meta = get(c, id)?;
            let title = meta["filename"].as_str().unwrap_or_default();
            let name = crate::files::safe_file_name(title).ok_or_else(|| {
                ApiError::usage(format!(
                    "the attachment's name {title:?} is not safe to use as a file name; pass --output-path"
                ))
            })?;
            PathBuf::from(format!("{prefix}{name}"))
        }
    };
    let bytes = c.download_to(
        &format!("/rest/api/3/attachment/{segment}/{}", enc(id)),
        &[],
        &dest,
        force,
    )?;
    Ok(json!({"attachmentId": id, "path": dest.display().to_string(), "bytes": bytes}))
}

/// Upload all files in one multipart request, after checking each exists.
pub fn upload(c: &Client, key: &str, files: &[PathBuf]) -> Result<Value> {
    if files.is_empty() {
        return Err(ApiError::usage("pass at least one --file").into());
    }
    if let Some(bad) = files.iter().find(|f| !f.is_file()) {
        return Err(ApiError::usage(format!("{} is not a readable file", bad.display())).into());
    }
    let parts: Vec<Part> = files.iter().map(|f| Part::file("file", f)).collect();
    let mut v = c.post_multipart(
        &format!("/rest/api/3/issue/{}/attachments", enc(key)),
        &parts,
        &[NO_CHECK],
    )?;
    strip_media(&mut v);
    Ok(v)
}

pub fn delete(c: &Client, id: &str) -> Result<Value> {
    c.delete(&format!("/rest/api/3/attachment/{}", enc(id)), &[])?;
    Ok(json!({"id": id, "deleted": true}))
}

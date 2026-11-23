//! `page label list|create|delete` and `label watch|unwatch`.
//!
//! Listing uses REST v2 where it has a labels endpoint for the content type
//! and v1 otherwise. Adding, removing and watching labels only exist in v1.

use super::{ContentType, enc, listing, resolve_id, try_content};
use crate::client::{ApiError, Client};
use anyhow::Result;
use clap::ValueEnum;
use serde_json::{Value, json};

/// `--type` on `page label list`. (The public REST API lists no labels
/// of whiteboards, databases or folders.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LabelTarget {
    Page,
    Blogpost,
    CustomContent,
}

fn label(l: &Value) -> Value {
    json!({"id": l["id"], "name": l["name"], "prefix": l["prefix"]})
}

/// `page label list`. Without `--type` the page endpoint is
/// tried first, then the blog post one.
pub fn list(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    content_type: Option<LabelTarget>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    let id = resolve_id(c, reference, reference_space)?;
    let limit = limit.clamp(1, 250).to_string();
    let mut q = vec![("limit", limit.as_str())];
    if let Some(cur) = cursor {
        q.push(("cursor", cur));
    }
    let v2 = |collection: &str| c.get(&format!("/api/v2/{collection}/{}/labels", enc(&id)), &q);
    let v = match content_type {
        None => try_content(|k| v2(k.collection()))?.1,
        Some(LabelTarget::Page) => v2(ContentType::Page.collection())?,
        Some(LabelTarget::Blogpost) => v2(ContentType::Blogpost.collection())?,
        Some(LabelTarget::CustomContent) => v2("custom-content")?,
    };
    Ok(listing(&v, label))
}

/// Label names from repeatable, comma-separated `--label` values.
fn names(labels: &[String]) -> Result<Vec<String>> {
    let out: Vec<String> = labels
        .iter()
        .flat_map(|l| l.split(','))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if out.is_empty() {
        return Err(ApiError::usage("pass at least one --label").into());
    }
    Ok(out)
}

/// `page label create`: v1 `POST /rest/api/content/{id}/label`. Returns the
/// content's labels after the change.
pub fn add(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    labels: &[String],
    prefix: &str,
) -> Result<Value> {
    let names = names(labels)?;
    let id = resolve_id(c, reference, reference_space)?;
    let req: Vec<Value> = names
        .iter()
        .map(|n| json!({"prefix": prefix, "name": n}))
        .collect();
    let v = c.post(
        &format!("/rest/api/content/{}/label", enc(&id)),
        &Value::Array(req),
    )?;
    let results: Vec<Value> = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(label)
        .collect();
    Ok(Value::Array(results))
}

/// `page label delete`: one v1 `DELETE` per label. A label with a
/// prefix other than `global` is addressed as `prefix:name`.
pub fn remove(
    c: &Client,
    reference: &str,
    reference_space: Option<&str>,
    labels: &[String],
    prefix: &str,
) -> Result<Value> {
    let names = names(labels)?;
    let id = resolve_id(c, reference, reference_space)?;
    for n in &names {
        let name = if prefix == "global" {
            n.clone()
        } else {
            format!("{prefix}:{n}")
        };
        c.delete(
            &format!("/rest/api/content/{}/label", enc(&id)),
            &[("name", name.as_str())],
        )?;
    }
    Ok(json!({"id": id, "removed": names}))
}

/// `label watch|unwatch`: v1 label watches for the current
/// user.
pub fn watch(c: &Client, name: &str, on: bool) -> Result<Value> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::usage("pass --name with a label name").into());
    }
    let path = format!("/rest/api/user/watch/label/{}", enc(name));
    if on {
        // Confluence's XSRF check rejects this POST without the header.
        c.request_with(
            "POST",
            &path,
            &[],
            Some(&json!({})),
            &[("X-Atlassian-Token", "no-check")],
        )?;
    } else {
        c.delete(&path, &[])?;
    }
    Ok(json!({"name": name, "watching": on}))
}

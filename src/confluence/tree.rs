//! `confluence tree`: the descendants of a page, or a whole space.

use super::{content_id, content_url, enc, next_cursor, space_id};
use crate::client::{ApiError, Client, ErrorKind};
use anyhow::Result;
use clap::ValueEnum;
use serde_json::{Value, json};

pub const MAX_DEPTH: usize = 10;
/// The most nodes one tree holds.
pub const MAX_NODES: usize = 500;

/// `--type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum NodeType {
    Page,
    Whiteboard,
    Database,
}

impl NodeType {
    fn name(self) -> &'static str {
        match self {
            NodeType::Page => "page",
            NodeType::Whiteboard => "whiteboard",
            NodeType::Database => "database",
        }
    }
}

fn not_found(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::NotFound)
}

struct Walk<'a> {
    c: &'a Client,
    /// With a type, mixed children are read so other types can be found.
    only: Option<NodeType>,
    left: usize,
    truncated: bool,
}

impl Walk<'_> {
    /// Every result of a list, up to what the node budget has left.
    fn list(&mut self, path: &str, extra: &[(&str, &str)], pages_only: bool) -> Result<Vec<Value>> {
        let mut out: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut q = extra.to_vec();
            q.push(("limit", "250"));
            if let Some(cur) = &cursor {
                q.push(("cursor", cur));
            }
            let v = match self.c.get(path, &q) {
                Ok(v) => v,
                // Gone since its parent was listed, or a leaf type.
                Err(e) if not_found(&e) && extra.is_empty() => return Ok(out),
                Err(e) => return Err(e),
            };
            // Filtered here, so other types don't use up the node budget.
            out.extend(
                v["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|i| !pages_only || i["type"] == "page")
                    .cloned(),
            );
            match next_cursor(&v) {
                Value::String(next)
                    if out.len() <= self.left && cursor.as_deref() != Some(&next) =>
                {
                    cursor = Some(next)
                }
                _ => return Ok(out),
            }
        }
    }

    fn children(&mut self, id: &str, kind: &str) -> Result<Vec<Value>> {
        let collection = match (self.only, kind) {
            (_, "page") => "pages",
            (None, _) => return Ok(Vec::new()),
            (Some(_), "whiteboard") => "whiteboards",
            (Some(_), "database") => "databases",
            (Some(_), "folder") => "folders",
            _ => return Ok(Vec::new()),
        };
        let path = format!("/api/v2/{collection}/{}/direct-children", enc(id));
        self.list(&path, &[], self.only.is_none())
    }

    /// `depth` levels from `items` down.
    fn nodes(&mut self, items: Vec<Value>, depth: usize) -> Result<Vec<Value>> {
        // A level is counted before descending, so it is never lost to the
        // first child's subtree.
        if items.len() > self.left {
            self.truncated = true;
        }
        let kept: Vec<Value> = items.into_iter().take(self.left).collect();
        self.left -= kept.len();
        let mut out = Vec::new();
        for item in kept {
            let kind = item["type"].as_str().unwrap_or("page").to_string();
            let id = item["id"].as_str().unwrap_or_default().to_string();
            let mut children = Vec::new();
            if depth > 1 && !id.is_empty() {
                if self.left == 0 {
                    self.truncated = true;
                } else {
                    let below = self.children(&id, &kind)?;
                    children = self.nodes(below, depth - 1)?;
                }
            }
            // Containers of the wanted type stay, so the tree still connects.
            if self.only.is_some_and(|t| t.name() != kind) && children.is_empty() {
                continue;
            }
            out.push(json!({
                "id": item["id"],
                "title": item["title"],
                "type": kind,
                "status": item["status"],
                "children": children,
            }));
        }
        Ok(out)
    }
}

fn walker(c: &Client, only: Option<NodeType>) -> Walk<'_> {
    Walk {
        c,
        only,
        left: MAX_NODES,
        truncated: false,
    }
}

/// `page tree`: a page's ancestors and its descendants to `depth`.
pub fn page(c: &Client, reference: &str, depth: usize, only: Option<NodeType>) -> Result<Value> {
    let id = content_id(reference)?;
    let page = c.get(&format!("/api/v2/pages/{}", enc(&id)), &[])?;
    page_tree(&mut walker(c, only), &id, &page, depth.min(MAX_DEPTH))
}

/// `space tree`: a space's top-level pages and their descendants to
/// `depth`.
pub fn space(c: &Client, reference: &str, depth: usize, only: Option<NodeType>) -> Result<Value> {
    space_tree(&mut walker(c, only), reference.trim(), depth.min(MAX_DEPTH))
}

fn page_tree(walk: &mut Walk, id: &str, page: &Value, depth: usize) -> Result<Value> {
    let c = walk.c;
    let anc = c.get(
        &format!("/api/v2/pages/{}/ancestors", enc(id)),
        &[("limit", "250")],
    )?;
    let ancestors: Vec<Value> = anc["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| json!({"id": a["id"], "type": a["type"], "title": a["title"]}))
        .collect();
    let children = if depth == 0 {
        Vec::new()
    } else {
        let top = walk.children(id, "page")?;
        walk.nodes(top, depth)?
    };
    let mut out = json!({
        "id": page["id"],
        "title": page["title"],
        "status": page["status"],
        "spaceId": page["spaceId"],
        "externalUrl": content_url(c, page),
        "depth": depth,
        "ancestors": ancestors,
        "children": children,
    });
    if walk.truncated {
        out["truncated"] = true.into();
    }
    Ok(out)
}

fn space_tree(walk: &mut Walk, reference: &str, depth: usize) -> Result<Value> {
    let c = walk.c;
    let sid = space_id(c, &super::lifecycle::exact_space(reference)?)?;
    let space = c.get(&format!("/api/v2/spaces/{}", enc(&sid)), &[])?;
    let children = if depth == 0 {
        Vec::new()
    } else {
        let mut roots = walk.list(
            &format!("/api/v2/spaces/{}/pages", enc(&sid)),
            &[("depth", "root")],
            false,
        )?;
        for r in &mut roots {
            r["type"] = "page".into();
        }
        walk.nodes(roots, depth)?
    };
    let mut out = json!({
        "id": space["id"],
        "key": space["key"],
        "title": space["name"],
        "type": "space",
        "depth": depth,
        "children": children,
    });
    if walk.truncated {
        out["truncated"] = true.into();
    }
    Ok(out)
}

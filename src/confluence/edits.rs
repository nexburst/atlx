//! `page edit`: targeted changes to a page's ADF body.
//!
//! An operation is a JSON object with a `name` and its arguments: a node's
//! ADF `localId` (from `page get --outline` or `--body-format adf`), a
//! heading, or text to find, and a `value`. Values are Markdown by default,
//! or `"format": "adf"` (an ADF node, a list of nodes or a document) or
//! `"plain"`. A value that cannot be converted without losing something is
//! refused; HTML and storage values are not accepted.
//!
//! Every operation is applied to a copy of the document, so one failure
//! leaves nothing changed, and nodes no operation targets keep their JSON
//! exactly.

use super::sections::{Heading, adf_plain, match_headings, no_match};
use crate::client::ApiError;
use crate::markup::{BodyError, BodyFormat, markdown_to_adf, parse_adf, text_to_adf};
use anyhow::Result;
use serde_json::{Map, Value, json};

/// Where `moveNode` puts a node relative to its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Before,
    After,
}

/// Replacement count for `replaceText`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    AtMost(usize),
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    ReplaceNode {
        local_id: String,
        value: Vec<Value>,
    },
    InsertNodeAfter {
        local_id: String,
        value: Vec<Value>,
    },
    InsertNodeBefore {
        local_id: String,
        value: Vec<Value>,
    },
    DeleteNode {
        local_id: String,
    },
    RemoveNodes {
        local_ids: Vec<String>,
    },
    MoveNode {
        local_id: String,
        target: String,
        position: Position,
    },
    AppendNodeToEnd {
        value: Vec<Value>,
    },
    SetAttrs {
        local_id: String,
        attrs: Map<String, Value>,
    },
    // atlx extras
    ReplaceText {
        find: String,
        replace: String,
        count: Count,
    },
    ReplaceSection {
        heading: String,
        value: Vec<Value>,
    },
    AppendToSection {
        heading: String,
        value: Vec<Value>,
    },
    PrependToSection {
        heading: String,
        value: Vec<Value>,
    },
    DeleteSection {
        heading: String,
    },
    InsertSectionAfter {
        heading: String,
        new_heading: String,
        level: Option<u8>,
        value: Vec<Value>,
    },
}

fn usage(msg: impl Into<String>) -> anyhow::Error {
    ApiError::usage(msg).into()
}

fn str_field(o: &Map<String, Value>, args: &Map<String, Value>, k: &str) -> Option<String> {
    o.get(k)
        .or_else(|| args.get(k))
        .and_then(Value::as_str)
        .map(String::from)
}

/// Parse an `--edits` JSON array. Values are converted to ADF here, so a bad
/// operation is reported before anything is fetched.
pub fn parse(json_text: &str) -> Result<Vec<Op>> {
    let v: Value = serde_json::from_str(json_text)
        .map_err(|e| usage(format!("--edits is not valid JSON: {e}")))?;
    let list = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => return Err(usage("--edits must be a JSON array of edit operations")),
    };
    if list.is_empty() {
        return Err(usage("--edits has no operations"));
    }
    list.iter()
        .enumerate()
        .map(|(i, e)| {
            parse_one(e).map_err(|err| match err {
                Bad::Usage(m) => usage(format!("edit {}: {m}", i + 1)),
                Bad::Body(mut b) => {
                    b.message = format!("edit {}: {}", i + 1, b.message);
                    b.into()
                }
            })
        })
        .collect()
}

/// Why an operation was not accepted.
enum Bad {
    Usage(String),
    /// Its value is not valid in its format, or would lose content.
    Body(BodyError),
}

impl From<String> for Bad {
    fn from(s: String) -> Self {
        Bad::Usage(s)
    }
}

impl From<&str> for Bad {
    fn from(s: &str) -> Self {
        Bad::Usage(s.to_string())
    }
}

fn parse_one(e: &Value) -> Result<Op, Bad> {
    let Some(o) = e.as_object() else {
        return Err("each edit must be a JSON object".into());
    };
    let empty = Map::new();
    let args = o
        .get("arguments")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let name = str_field(o, &empty, "name")
        .or_else(|| str_field(o, &empty, "op"))
        .ok_or("missing \"name\"")?;
    let req = |k: &str| str_field(o, args, k).ok_or(format!("{name} needs \"{k}\""));
    let format = match str_field(o, args, "format").as_deref() {
        None | Some("md" | "markdown") => BodyFormat::Markdown,
        Some("adf") => BodyFormat::Adf,
        Some("plain") => BodyFormat::Plain,
        Some(f @ ("html" | "storage")) => {
            return Err(Bad::Usage(format!(
                "values can't be {f}: write them in Markdown (the default), or pass \"format\": \"adf\" with ADF nodes"
            )));
        }
        Some(f) => return Err(format!("unknown format '{f}' (markdown, adf or plain)").into()),
    };
    let value = |required: bool| -> Result<Vec<Value>, Bad> {
        match o.get("value").or_else(|| args.get("value")) {
            Some(v) => blocks(v, format),
            None if required => Err(format!("{name} needs \"value\"").into()),
            None => Ok(Vec::new()),
        }
    };
    Ok(match name.as_str() {
        "replaceNode" => Op::ReplaceNode {
            local_id: req("localId")?,
            value: value(true)?,
        },
        "insertNodeAfter" => Op::InsertNodeAfter {
            local_id: req("localId")?,
            value: value(true)?,
        },
        "insertNodeBefore" => Op::InsertNodeBefore {
            local_id: req("localId")?,
            value: value(true)?,
        },
        "deleteNode" => Op::DeleteNode {
            local_id: req("localId")?,
        },
        "removeNodes" => {
            let ids = o
                .get("localIds")
                .or_else(|| args.get("localIds"))
                .and_then(Value::as_array)
                .ok_or("removeNodes needs \"arguments.localIds\"")?;
            let local_ids: Vec<String> = ids
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .ok_or("localIds must be strings")
                })
                .collect::<Result<_, _>>()?;
            if local_ids.is_empty() {
                return Err("removeNodes needs at least one localId".into());
            }
            Op::RemoveNodes { local_ids }
        }
        "moveNode" => Op::MoveNode {
            local_id: req("localId")?,
            target: req("targetLocalId")?,
            position: match str_field(o, args, "position").as_deref() {
                None | Some("after") => Position::After,
                Some("before") => Position::Before,
                Some(p) => return Err(format!("unknown position '{p}' (before or after)").into()),
            },
        },
        "appendNodeToEnd" => Op::AppendNodeToEnd {
            value: value(true)?,
        },
        "setAttrs" => Op::SetAttrs {
            local_id: req("localId")?,
            attrs: o
                .get("attrs")
                .or_else(|| args.get("attrs"))
                .and_then(Value::as_object)
                .cloned()
                .ok_or("setAttrs needs \"arguments.attrs\" as an object")?,
        },
        "replaceText" => {
            let find = req("find")?;
            if find.is_empty() {
                return Err("replaceText needs a non-empty \"find\"".into());
            }
            let count = match o.get("count").or_else(|| args.get("count")) {
                None => Count::AtMost(1),
                Some(Value::String(s)) if s == "all" => Count::All,
                Some(v) => match v.as_u64() {
                    Some(n) if n > 0 => Count::AtMost(n as usize),
                    _ => return Err("count must be a positive number or \"all\"".into()),
                },
            };
            Op::ReplaceText {
                find,
                replace: str_field(o, args, "replace").unwrap_or_default(),
                count,
            }
        }
        "replaceSection" => Op::ReplaceSection {
            heading: req("heading")?,
            value: value(false)?,
        },
        "appendToSection" => Op::AppendToSection {
            heading: req("heading")?,
            value: value(true)?,
        },
        "prependToSection" => Op::PrependToSection {
            heading: req("heading")?,
            value: value(true)?,
        },
        "deleteSection" => Op::DeleteSection {
            heading: req("heading")?,
        },
        "insertSectionAfter" => Op::InsertSectionAfter {
            heading: req("heading")?,
            new_heading: req("newHeading")?,
            level: match o.get("level").or_else(|| args.get("level")) {
                None => None,
                Some(v) => match v.as_u64() {
                    Some(n @ 1..=6) => Some(n as u8),
                    _ => return Err("level must be 1 to 6".into()),
                },
            },
            value: value(false)?,
        },
        other => return Err(format!("unknown edit operation '{other}'").into()),
    })
}

/// An operation's value as ADF block nodes. Markdown that would lose
/// anything is refused.
fn blocks(v: &Value, f: BodyFormat) -> Result<Vec<Value>, Bad> {
    let doc = match (f, v) {
        (BodyFormat::Adf, Value::String(s)) => parse_adf(s).map_err(Bad::Body)?,
        (BodyFormat::Adf, Value::Array(nodes)) => {
            json!({"type": "doc", "version": 1, "content": nodes})
        }
        (BodyFormat::Adf, Value::Object(_)) => parse_adf(&v.to_string()).map_err(Bad::Body)?,
        (BodyFormat::Plain, Value::String(s)) => text_to_adf(s),
        (_, Value::String(s)) => markdown_to_adf(s).into_lossless().map_err(Bad::Body)?,
        _ => {
            return Err("\"value\" must be a string (or ADF JSON with \"format\": \"adf\")".into());
        }
    };
    let nodes = doc["content"].as_array().cloned().unwrap_or_default();
    if let Some(bad) = nodes.iter().find(|n| !n["type"].is_string()) {
        return Err(format!("an ADF value node has no \"type\": {bad}").into());
    }
    Ok(nodes)
}

const INLINE: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "inlineCard",
    "status",
    "date",
    "placeholder",
    "mediaInline",
    "inlineExtension",
];

fn kind(v: &Value) -> &str {
    v["type"].as_str().unwrap_or_default()
}

/// Shape new nodes to fit where `like` sits: inline content for an inline
/// node, list items for a list item, rows for a table row, and so on. A
/// value that would only fit by dropping structure (two paragraphs, a list
/// or a table where inline content goes) is refused.
fn fit(nodes: Vec<Value>, like: &str) -> Result<Vec<Value>, String> {
    if INLINE.contains(&like) {
        let blocks: Vec<&Value> = nodes
            .iter()
            .filter(|n| !INLINE.contains(&kind(n)))
            .collect();
        if blocks.len() > 1 || blocks.iter().any(|n| kind(n) != "paragraph") {
            return Err(format!(
                "the target is inline ({like}); its value must be one paragraph of text"
            ));
        }
        return Ok(nodes
            .into_iter()
            .flat_map(|n| {
                if INLINE.contains(&kind(&n)) {
                    vec![n]
                } else {
                    n["content"].as_array().cloned().unwrap_or_default()
                }
            })
            .collect());
    }
    if let [one] = nodes.as_slice()
        && kind(one) != like
        && let Some(children) = one["content"].as_array()
        && !children.is_empty()
        && children.iter().all(|c| kind(c) == like)
    {
        return Ok(children.clone());
    }
    Ok(nodes)
}

type Path = Vec<usize>;

fn find_paths(v: &Value, id: &str, path: &mut Path, out: &mut Vec<Path>) {
    if let Some(a) = v["content"].as_array() {
        for (i, c) in a.iter().enumerate() {
            path.push(i);
            if c["attrs"]["localId"].as_str() == Some(id) {
                out.push(path.clone());
            }
            find_paths(c, id, path, out);
            path.pop();
        }
    }
}

fn locate(doc: &Value, id: &str) -> Result<Path, String> {
    let mut out = Vec::new();
    find_paths(doc, id, &mut Vec::new(), &mut out);
    match out.len() {
        0 => Err(format!("no node with localId '{id}'")),
        1 => Ok(out.remove(0)),
        n => Err(format!("{n} nodes share localId '{id}'")),
    }
}

fn node_at<'a>(doc: &'a Value, path: &[usize]) -> &'a Value {
    path.iter().fold(doc, |v, &i| &v["content"][i])
}

/// The `content` array that holds the node at `path`.
fn siblings<'a>(doc: &'a mut Value, path: &[usize]) -> &'a mut Vec<Value> {
    let (_, parent) = path.split_last().expect("non-empty path");
    let mut v = doc;
    for &i in parent {
        v = &mut v["content"][i];
    }
    v["content"].as_array_mut().expect("parent has content")
}

fn splice(doc: &mut Value, path: &[usize], remove: usize, insert: Vec<Value>, offset: usize) {
    let idx = path[path.len() - 1] + offset;
    let sib = siblings(doc, path);
    sib.splice(idx..idx + remove, insert);
}

fn top(doc: &mut Value) -> &mut Vec<Value> {
    if !doc["content"].is_array() {
        doc["content"] = json!([]);
    }
    doc["content"].as_array_mut().unwrap()
}

/// Top-level section `[heading, end)` for a heading query; the query must
/// select exactly one heading.
fn section(doc: &Value, query: &str) -> Result<(usize, usize, u8), String> {
    let nodes = doc["content"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let heads: Vec<(usize, Heading)> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| kind(n) == "heading")
        .map(|(i, n)| {
            (
                i,
                Heading {
                    level: n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8,
                    text: adf_plain(n),
                },
            )
        })
        .collect();
    let m = match_headings(heads.iter().map(|(_, h)| h.text.as_str()), query);
    let h = match m.as_slice() {
        [one] => *one,
        [] => {
            let hs: Vec<Heading> = heads.into_iter().map(|(_, h)| h).collect();
            return Err(no_match(query, &hs).to_string());
        }
        many => {
            let names: Vec<&str> = many.iter().map(|&i| heads[i].1.text.as_str()).collect();
            return Err(format!(
                "heading '{query}' is ambiguous: {}",
                names.join("; ")
            ));
        }
    };
    let (idx, level) = (heads[h].0, heads[h].1.level);
    let end = heads[h + 1..]
        .iter()
        .find(|(_, x)| x.level <= level)
        .map_or(nodes.len(), |(i, _)| *i);
    Ok((idx, end, level))
}

/// Replace `find` in every text node; returns the number of matches.
fn replace_text(v: &mut Value, find: &str, replace: &str) -> usize {
    let mut n = 0;
    if kind(v) == "text"
        && let Some(t) = v["text"].as_str()
        && t.contains(find)
    {
        n += t.matches(find).count();
        v["text"] = t.replace(find, replace).into();
    }
    if let Some(a) = v.get_mut("content").and_then(Value::as_array_mut) {
        for c in a {
            n += replace_text(c, find, replace);
        }
    }
    n
}

fn apply_one(doc: &mut Value, op: &Op) -> Result<(), String> {
    match op {
        Op::ReplaceNode { local_id, value } => {
            let p = locate(doc, local_id)?;
            let like = kind(node_at(doc, &p)).to_string();
            splice(doc, &p, 1, fit(value.clone(), &like)?, 0);
        }
        Op::InsertNodeAfter { local_id, value } | Op::InsertNodeBefore { local_id, value } => {
            let p = locate(doc, local_id)?;
            let like = kind(node_at(doc, &p)).to_string();
            let offset = usize::from(matches!(op, Op::InsertNodeAfter { .. }));
            splice(doc, &p, 0, fit(value.clone(), &like)?, offset);
        }
        Op::DeleteNode { local_id } => {
            let p = locate(doc, local_id)?;
            splice(doc, &p, 1, Vec::new(), 0);
        }
        Op::RemoveNodes { local_ids } => {
            // Locate every node first so a missing ID removes nothing.
            for id in local_ids {
                locate(doc, id)?;
            }
            for id in local_ids {
                // A node inside an earlier removed node is already gone.
                if let Ok(p) = locate(doc, id) {
                    splice(doc, &p, 1, Vec::new(), 0);
                }
            }
        }
        Op::MoveNode {
            local_id,
            target,
            position,
        } => {
            if local_id == target {
                return Err("moveNode target is the node itself".into());
            }
            let p = locate(doc, local_id)?;
            locate(doc, target)?;
            let moved = node_at(doc, &p).clone();
            let mut inner = Vec::new();
            find_paths(&moved, target, &mut Vec::new(), &mut inner);
            if !inner.is_empty() {
                return Err("moveNode target is inside the node being moved".into());
            }
            splice(doc, &p, 1, Vec::new(), 0);
            let t = locate(doc, target)?;
            let offset = usize::from(*position == Position::After);
            splice(doc, &t, 0, vec![moved], offset);
        }
        Op::AppendNodeToEnd { value } => top(doc).extend(value.iter().cloned()),
        Op::SetAttrs { local_id, attrs } => {
            let p = locate(doc, local_id)?;
            let mut v = &mut *doc;
            for &i in &p {
                v = &mut v["content"][i];
            }
            if !v["attrs"].is_object() {
                v["attrs"] = json!({});
            }
            let a = v["attrs"].as_object_mut().unwrap();
            for (k, val) in attrs {
                a.insert(k.clone(), val.clone());
            }
        }
        Op::ReplaceText {
            find,
            replace,
            count,
        } => {
            let hits = replace_text(doc, find, replace);
            if hits == 0 {
                return Err(format!(
                    "text '{find}' not found (a match must sit inside one run of text with the same formatting)"
                ));
            }
            if let Count::AtMost(max) = count
                && hits > *max
            {
                return Err(format!(
                    "text '{find}' found {hits} times, more than count {max}; raise count or pass \"all\""
                ));
            }
        }
        Op::ReplaceSection { heading, value } => {
            let (h, end, _) = section(doc, heading)?;
            top(doc).splice(h + 1..end, value.iter().cloned());
        }
        Op::AppendToSection { heading, value } => {
            let (_, end, _) = section(doc, heading)?;
            top(doc).splice(end..end, value.iter().cloned());
        }
        Op::PrependToSection { heading, value } => {
            let (h, _, _) = section(doc, heading)?;
            top(doc).splice(h + 1..h + 1, value.iter().cloned());
        }
        Op::DeleteSection { heading } => {
            let (h, end, _) = section(doc, heading)?;
            top(doc).drain(h..end);
        }
        Op::InsertSectionAfter {
            heading,
            new_heading,
            level,
            value,
        } => {
            let (_, end, l) = section(doc, heading)?;
            let mut nodes = vec![json!({
                "type": "heading",
                "attrs": {"level": level.unwrap_or(l)},
                "content": [{"type": "text", "text": new_heading}],
            })];
            nodes.extend(value.iter().cloned());
            top(doc).splice(end..end, nodes);
        }
    }
    Ok(())
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::ReplaceNode { .. } => "replaceNode",
        Op::InsertNodeAfter { .. } => "insertNodeAfter",
        Op::InsertNodeBefore { .. } => "insertNodeBefore",
        Op::DeleteNode { .. } => "deleteNode",
        Op::RemoveNodes { .. } => "removeNodes",
        Op::MoveNode { .. } => "moveNode",
        Op::AppendNodeToEnd { .. } => "appendNodeToEnd",
        Op::SetAttrs { .. } => "setAttrs",
        Op::ReplaceText { .. } => "replaceText",
        Op::ReplaceSection { .. } => "replaceSection",
        Op::AppendToSection { .. } => "appendToSection",
        Op::PrependToSection { .. } => "prependToSection",
        Op::DeleteSection { .. } => "deleteSection",
        Op::InsertSectionAfter { .. } => "insertSectionAfter",
    }
}

/// Apply all operations in order to a copy of `doc`. Returns the new
/// document, or a usage error naming the first operation that failed (and
/// `doc` is left as it was).
pub fn apply(doc: &Value, ops: &[Op]) -> Result<Value> {
    let mut out = doc.clone();
    for (i, op) in ops.iter().enumerate() {
        apply_one(&mut out, op).map_err(|e| {
            usage(format!(
                "edit {} ({}): {e}; nothing was changed",
                i + 1,
                op_name(op)
            ))
        })?;
    }
    Ok(out)
}

/// A unified diff of two line lists, with three lines of context.
pub fn unified_diff(a: &[String], b: &[String], from: &str, to: &str) -> String {
    // Trim the common prefix and suffix, then run an LCS on the middle.
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    if am.is_empty() && bm.is_empty() {
        return String::new();
    }
    // ops: (' ' | '-' | '+', index into a or b)
    let mut ops: Vec<(char, usize)> = (0..pre).map(|i| (' ', i)).collect();
    if am.len() * bm.len() <= 4_000_000 {
        let (n, m) = (am.len(), bm.len());
        let mut l = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                l[i][j] = if am[i] == bm[j] {
                    l[i + 1][j + 1] + 1
                } else {
                    l[i + 1][j].max(l[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push((' ', pre + i));
                i += 1;
                j += 1;
            } else if i < n && (j == m || l[i + 1][j] >= l[i][j + 1]) {
                ops.push(('-', pre + i));
                i += 1;
            } else {
                ops.push(('+', pre + j));
                j += 1;
            }
        }
    } else {
        ops.extend((0..am.len()).map(|i| ('-', pre + i)));
        ops.extend((0..bm.len()).map(|j| ('+', pre + j)));
    }
    ops.extend((0..suf).map(|k| (' ', a.len() - suf + k)));

    // Line numbers in a and b for every op.
    let mut pos = Vec::with_capacity(ops.len());
    let (mut ai, mut bi) = (0, 0);
    for &(c, _) in &ops {
        pos.push((ai, bi));
        match c {
            ' ' => {
                ai += 1;
                bi += 1;
            }
            '-' => ai += 1,
            _ => bi += 1,
        }
    }
    let changed: Vec<usize> = (0..ops.len()).filter(|&k| ops[k].0 != ' ').collect();
    let mut out = format!("--- {from}\n+++ {to}\n");
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(3);
        let mut end = changed[k];
        while k < changed.len() && changed[k] <= end + 7 {
            end = changed[k];
            k += 1;
        }
        let end = (end + 4).min(ops.len());
        let hunk = &ops[start..end];
        let a_len = hunk.iter().filter(|(c, _)| *c != '+').count();
        let b_len = hunk.iter().filter(|(c, _)| *c != '-').count();
        let (a0, b0) = pos[start];
        let a_start = if a_len == 0 { a0 } else { a0 + 1 };
        let b_start = if b_len == 0 { b0 } else { b0 + 1 };
        out.push_str(&format!("@@ -{a_start},{a_len} +{b_start},{b_len} @@\n"));
        for &(c, idx) in hunk {
            let line = if c == '+' { &b[idx] } else { &a[idx] };
            out.push(c);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn diff_has_hunks_and_context() {
        let a = lines(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let mut b = a.clone();
        b[1] = "B".into();
        b.push("k".into());
        let d = unified_diff(&a, &b, "a", "b");
        assert!(
            d.starts_with("--- a\n+++ b\n@@ -1,5 +1,5 @@\n a\n-b\n+B\n c\n"),
            "{d}"
        );
        assert!(d.contains("@@ -8,3 +8,4 @@\n h\n i\n j\n+k\n"), "{d}");
        assert_eq!(unified_diff(&a, &a, "a", "b"), "");
    }

    #[test]
    fn parse_errors_are_usage_errors() {
        for (bad, want) in [
            ("nope", "not valid JSON"),
            ("[]", "no operations"),
            ("[{\"name\":\"zap\"}]", "unknown edit operation"),
            (
                "[{\"name\":\"replaceNode\",\"value\":\"x\"}]",
                "needs \"localId\"",
            ),
            (
                "[{\"name\":\"appendNodeToEnd\",\"value\":\"<p>x</p>\",\"format\":\"html\"}]",
                "can't be html",
            ),
            (
                "[{\"op\":\"replaceText\",\"find\":\"a\",\"count\":0}]",
                "count",
            ),
        ] {
            let e = parse(bad).unwrap_err();
            assert_eq!(
                ApiError::find(&e).map(|a| a.kind),
                Some(crate::client::ErrorKind::Usage),
                "{bad}"
            );
            let e = e.to_string();
            assert!(e.contains(want), "{bad}: {e}");
        }
    }

    #[test]
    fn lossy_values_are_refused_and_adf_values_are_kept() {
        let e = parse(r#"[{"name":"appendNodeToEnd","value":"<div>x</div>"}]"#).unwrap_err();
        let b = e.downcast_ref::<BodyError>().expect("a body error");
        assert!(b.is_refused());
        assert!(b.message.starts_with("edit 1:"), "{}", b.message);
        let node = json!({"type":"extension","attrs":{"extensionKey":"x"}});
        let ops =
            parse(&json!([{"name":"appendNodeToEnd","format":"adf","value":node}]).to_string())
                .unwrap();
        assert_eq!(ops, [Op::AppendNodeToEnd { value: vec![node] }]);
    }

    #[test]
    fn inline_targets_take_one_paragraph() {
        let doc = json!({"type":"doc","content":[{"type":"paragraph","content":[
            {"type":"text","text":"a"},{"type":"status","attrs":{"text":"X","localId":"s1"}}]}]});
        let two = parse(r#"[{"name":"replaceNode","localId":"s1","value":"one\n\ntwo"}]"#).unwrap();
        assert!(
            apply(&doc, &two)
                .unwrap_err()
                .to_string()
                .contains("one paragraph")
        );
        let one = parse(r#"[{"name":"replaceNode","localId":"s1","value":"**b**"}]"#).unwrap();
        let out = apply(&doc, &one).unwrap();
        assert_eq!(out["content"][0]["content"][1]["text"], "b");
    }
}

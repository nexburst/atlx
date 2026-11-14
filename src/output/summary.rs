//! `--output-summary`: write the full payload to a file and print a short
//! YAML envelope that describes it.
//!
//! The envelope has `output_files` (path, line and byte counts), the command,
//! and either the payload inline (`stdout_inline`, when it is small or
//! `inline` was asked for) or its statistics (`stdout_stats`) and a type
//! sketch (`stdout_shape`). It ends with a `---END---` line.

use super::select::{self, ENVELOPE_KEYS};
use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// `--output-summary` levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Level {
    /// No envelope: print the payload as usual
    None,
    /// Only file details, statistics and shape
    Stats,
    /// Inline small payloads, describe large ones
    Auto,
    /// Always inline the payload
    Inline,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::None => "none",
            Level::Stats => "stats",
            Level::Auto => "auto",
            Level::Inline => "inline",
        }
    }
}

/// Payloads whose pretty JSON is smaller than this are inlined by `auto`.
/// `ATLX_INLINE_LIMIT` overrides it.
pub const INLINE_LIMIT: usize = 10 * 1024;
/// At most this many collections are listed in `stdout_stats`.
const MAX_COLLECTIONS: usize = 4;
/// At most this many leaves are shown in `stdout_shape`.
const MAX_SHAPE_LEAVES: usize = 120;
/// Deepest object nesting tried before cutting the shape further.
const MAX_SHAPE_DEPTH: usize = 8;
/// Example strings per path, and their length cap.
const EXAMPLES: usize = 2;
const EXAMPLE_CHARS: usize = 100;

fn inline_limit() -> usize {
    std::env::var("ATLX_INLINE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(INLINE_LIMIT)
}

/// Everything [`envelope`] needs.
pub struct Input<'a> {
    pub payload: &'a Value,
    /// The text written to the output file.
    pub text: &'a str,
    pub path: &'a str,
    pub level: Level,
    /// `--agent-fields` paths (already resolved).
    pub fields: &'a [Vec<String>],
    /// Where the `--agent-fields` projection was written, and its size.
    pub compact: Option<(&'a str, usize)>,
    /// Used when the payload carries no `command` of its own.
    pub command: &'a str,
}

/// The `--agent-fields` projection that goes into the compact file.
pub fn compact_payload(payload: &Value, fields: &[Vec<String>]) -> Value {
    let projected = select::agent_fields(payload, fields);
    match (projected, paging(payload)) {
        (Value::Object(mut p), Value::Object(extra)) => {
            for (k, v) in extra {
                p.entry(k).or_insert(v);
            }
            Value::Object(p)
        }
        (p, _) => p,
    }
}

/// Paging hints kept from an envelope: `pageInfo.{hasNextPage,nextCursor}`.
fn paging(payload: &Value) -> Value {
    let mut out = Map::new();
    if payload.get("command").is_some_and(Value::is_string)
        && let Some(p) = payload.get("pageInfo")
        && let Some(more) = p.get("hasNextPage").and_then(Value::as_bool)
    {
        let mut info = Map::new();
        info.insert("hasNextPage".into(), more.into());
        if let Some(c) = p.get("nextCursor").filter(|c| c.is_string() || c.is_null()) {
            info.insert("nextCursor".into(), c.clone());
        }
        out.insert("pageInfo".into(), Value::Object(info));
    }
    Value::Object(out)
}

/// Build the YAML envelope, `---END---` line included.
pub fn envelope(i: &Input) -> String {
    let payload = i.payload;
    let data = select::is_error_envelope(payload)
        .then(|| payload.clone())
        .or_else(|| strip_envelope(payload))
        .unwrap_or_else(|| payload.clone());
    let has_fields = !i.fields.is_empty();
    let summary_payload = if has_fields {
        select::agent_fields(payload, i.fields)
    } else {
        data.clone()
    };
    let inline_payload = if has_fields {
        summary_payload.clone()
    } else {
        payload.clone()
    };
    let inline_bytes = serde_json::to_string_pretty(&inline_payload)
        .unwrap_or_default()
        .len();
    let inline = i.level == Level::Inline
        || (inline_bytes < inline_limit() && (i.level != Level::Stats || has_fields));

    let mut top = Map::new();
    let failure = select::is_error_envelope(payload);
    if failure {
        let mut err = Map::new();
        if let Some(e) = payload.get("error").and_then(Value::as_object) {
            for k in ["code", "kind", "message", "repair", "retry", "retryable"] {
                if let Some(v) = e.get(k) {
                    err.insert(k.into(), v.clone());
                }
            }
        }
        top.insert("ok".into(), false.into());
        top.insert("error".into(), Value::Object(err));
    }
    if let Value::Object(p) = paging(payload) {
        top.extend(p);
    }
    let mut files = Map::new();
    files.insert("stdout".into(), i.path.into());
    files.insert("stdout_lines".into(), i.text.split('\n').count().into());
    files.insert("stdout_bytes".into(), i.text.len().into());
    if let Some((path, bytes)) = i.compact {
        files.insert("compact".into(), path.into());
        files.insert("compact_bytes".into(), bytes.into());
    }
    top.insert("output_files".into(), Value::Object(files));
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or(i.command);
    if !command.is_empty() {
        top.insert("command".into(), command.into());
    }
    let resource = payload
        .get("resourceType")
        .or_else(|| payload.get("meta").and_then(|m| m.get("resourceClass")))
        .filter(|v| v.is_string());
    if let Some(r) = resource {
        top.insert("resource_type".into(), r.clone());
    }
    if i.level != Level::Auto || has_fields {
        let mut ao = Map::new();
        ao.insert("summary".into(), i.level.as_str().into());
        if has_fields {
            let dotted: Vec<Value> = i.fields.iter().map(|p| p.join(".").into()).collect();
            ao.insert("fields".into(), Value::Array(dotted));
            ao.insert("field_source".into(), "explicit".into());
            if !is_empty_payload(&data) {
                let missing = select::unresolved_fields(payload, i.fields);
                if !missing.is_empty() {
                    ao.insert("fields_unresolved".into(), json!(missing));
                }
            }
        }
        top.insert("agent_output".into(), Value::Object(ao));
    }
    if inline {
        top.insert("stdout_inline".into(), inline_payload);
    } else {
        let keys: Vec<Value> = payload
            .as_object()
            .map(|m| m.keys().map(|k| k.as_str().into()).collect())
            .unwrap_or_default();
        let shape = shape_of(&summary_payload);
        let mut stats = Map::new();
        stats.insert("top_level_keys".into(), Value::Array(keys));
        let mut counts = Vec::new();
        collections(&shape, &mut Vec::new(), &mut counts);
        if !counts.is_empty() {
            let total = counts.len();
            counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_cmp(&a.0, &b.0)));
            counts.truncate(MAX_COLLECTIONS);
            let listed = counts.len();
            stats.insert(
                "collections".into(),
                Value::Object(counts.into_iter().map(|(k, n)| (k, n.into())).collect()),
            );
            if total > listed {
                stats.insert("collections_omitted".into(), (total - listed).into());
            }
        }
        top.insert("stdout_stats".into(), Value::Object(stats));
        if !failure {
            let (shape, truncated) = bound(shape);
            top.insert("stdout_shape".into(), shape.to_value());
            if truncated {
                top.insert("stdout_shape_truncated".into(), true.into());
            }
        }
    }
    format!("{}\n---END---\n", yaml(&Value::Object(top), 0))
}

fn strip_envelope(v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    let rest: Map<String, Value> = m
        .iter()
        .filter(|(k, _)| !ENVELOPE_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (!rest.is_empty()).then_some(Value::Object(rest))
}

fn is_empty_payload(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => match m.get("data") {
            Some(d) => is_empty_payload(d),
            None => m.values().all(|v| v.as_array().is_some_and(Vec::is_empty)),
        },
        _ => false,
    }
}

/// Order keys like a case-insensitive locale sort: letters compare without
/// case first, then lowercase before uppercase.
fn locale_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| {
        a.chars()
            .zip(b.chars())
            .find(|(x, y)| x != y)
            .map(|(x, _)| {
                if x.is_lowercase() {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_or(a.len().cmp(&b.len()))
    })
}

// --- shape ----------------------------------------------------------------------

/// A type sketch of a JSON value.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    /// `S`, `N`, `B`, `Z` (null), `O` or `A`, or several joined by ` | `.
    Leaf(String),
    /// A string leaf (or a union with one) with its statistics:
    /// average length, count and example values.
    Strings(String, f64, usize, Vec<String>),
    /// An array: its length (or `min..max` across merged arrays) and the
    /// merged shape of its items.
    Array(Len, Option<Box<Shape>>),
    Object(Vec<(String, Shape)>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Len(usize, usize);

impl Len {
    fn to_value(self) -> Value {
        if self.0 == self.1 {
            self.0.into()
        } else {
            format!("{}..{}", self.0, self.1).into()
        }
    }
}

#[derive(Default)]
struct StrStats {
    count: usize,
    chars: usize,
    examples: Vec<String>,
}

fn shape_of(v: &Value) -> Shape {
    let mut stats = BTreeMap::new();
    let s = build(v, &mut Vec::new(), &mut stats);
    attach(s, &mut Vec::new(), &stats)
}

fn build(v: &Value, path: &mut Vec<String>, stats: &mut BTreeMap<Vec<String>, StrStats>) -> Shape {
    match v {
        Value::Null => Shape::Leaf("Z".into()),
        Value::Bool(_) => Shape::Leaf("B".into()),
        Value::Number(_) => Shape::Leaf("N".into()),
        Value::String(s) => {
            let st = stats.entry(path.clone()).or_default();
            st.count += 1;
            st.chars += s.encode_utf16().count();
            let ex = if s.chars().count() <= EXAMPLE_CHARS {
                s.clone()
            } else {
                s.chars().take(EXAMPLE_CHARS - 3).collect::<String>() + "..."
            };
            if st.examples.len() < EXAMPLES && !st.examples.contains(&ex) {
                st.examples.push(ex);
            }
            Shape::Leaf("S".into())
        }
        Value::Array(items) => {
            let item = items
                .iter()
                .map(|i| build(i, path, stats))
                .reduce(merge_shape);
            Shape::Array(Len(items.len(), items.len()), item.map(Box::new))
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| locale_cmp(a, b));
            Shape::Object(
                keys.into_iter()
                    .map(|k| {
                        path.push(k.clone());
                        let s = build(&m[k], path, stats);
                        path.pop();
                        (k.clone(), s)
                    })
                    .collect(),
            )
        }
    }
}

fn tag(s: &Shape) -> String {
    match s {
        Shape::Leaf(t) | Shape::Strings(t, ..) => t.clone(),
        Shape::Array(..) => "A".into(),
        Shape::Object(_) => "O".into(),
    }
}

fn union(a: &str, b: &str) -> String {
    let mut parts: Vec<&str> = a.split(" | ").chain(b.split(" | ")).collect();
    parts.sort_by(|x, y| locale_cmp(x, y));
    parts.dedup();
    parts.join(" | ")
}

fn merge_shape(a: Shape, b: Shape) -> Shape {
    match (a, b) {
        (Shape::Leaf(x), Shape::Leaf(y)) => Shape::Leaf(union(&x, &y)),
        (Shape::Array(la, ia), Shape::Array(lb, ib)) => {
            let item = match (ia, ib) {
                (Some(x), Some(y)) => Some(Box::new(merge_shape(*x, *y))),
                (x, y) => x.or(y),
            };
            Shape::Array(Len(la.0.min(lb.0), la.1.max(lb.1)), item)
        }
        (Shape::Object(x), Shape::Object(y)) => {
            let mut keys: Vec<String> = x.iter().map(|(k, _)| k.clone()).collect();
            for (k, _) in &y {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
            keys.sort_by(|a, b| locale_cmp(a, b));
            let mut x: BTreeMap<String, Shape> = x.into_iter().collect();
            let mut y: BTreeMap<String, Shape> = y.into_iter().collect();
            Shape::Object(
                keys.into_iter()
                    .map(|k| {
                        let s = match (x.remove(&k), y.remove(&k)) {
                            (Some(p), Some(q)) => merge_shape(p, q),
                            (Some(p), None) | (None, Some(p)) => p,
                            (None, None) => unreachable!(),
                        };
                        (k, s)
                    })
                    .collect(),
            )
        }
        (a, b) => Shape::Leaf(union(&tag(&a), &tag(&b))),
    }
}

fn attach(s: Shape, path: &mut Vec<String>, stats: &BTreeMap<Vec<String>, StrStats>) -> Shape {
    match s {
        Shape::Leaf(t) if t.split(" | ").any(|p| p == "S") => match stats.get(path) {
            Some(st) => {
                let avg = (st.chars as f64 / st.count as f64 * 10.0).round() / 10.0;
                Shape::Strings(t, avg, st.count, st.examples.clone())
            }
            None => Shape::Leaf(t),
        },
        Shape::Array(l, Some(i)) => Shape::Array(l, Some(Box::new(attach(*i, path, stats)))),
        Shape::Object(fields) => Shape::Object(
            fields
                .into_iter()
                .map(|(k, v)| {
                    path.push(k.clone());
                    let v = attach(v, path, stats);
                    path.pop();
                    (k, v)
                })
                .collect(),
        ),
        other => other,
    }
}

/// Arrays found in the shape, by dotted path (`[]` for a top-level array).
fn collections(s: &Shape, path: &mut Vec<String>, out: &mut Vec<(String, usize)>) {
    match s {
        Shape::Array(l, item) => {
            let key = if path.is_empty() {
                "[]".to_string()
            } else {
                path.join(".")
            };
            if !out.iter().any(|(k, _)| *k == key) {
                out.push((key, l.1));
            }
            if let Some(i) = item {
                collections(i, path, out);
            }
        }
        Shape::Object(fields) => {
            for (k, v) in fields {
                path.push(k.clone());
                collections(v, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn leaves(s: &Shape) -> usize {
    match s {
        Shape::Array(_, Some(i)) => leaves(i),
        Shape::Object(f) => f.iter().map(|(_, v)| leaves(v)).sum(),
        _ => 1,
    }
}

fn cut_depth(s: &Shape, depth: usize) -> Shape {
    match s {
        Shape::Array(l, i) => Shape::Array(*l, i.as_ref().map(|i| Box::new(cut_depth(i, depth)))),
        Shape::Object(_) if depth == 0 => Shape::Leaf("...".into()),
        Shape::Object(f) => Shape::Object(
            f.iter()
                .map(|(k, v)| (k.clone(), cut_depth(v, depth - 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn cut_leaves(s: &Shape, left: &mut usize) -> Shape {
    match s {
        Shape::Array(l, Some(i)) => Shape::Array(*l, Some(Box::new(cut_leaves(i, left)))),
        Shape::Object(f) => {
            let mut out = Vec::new();
            for (k, v) in f {
                if *left == 0 {
                    break;
                }
                out.push((k.clone(), cut_leaves(v, left)));
            }
            Shape::Object(out)
        }
        other => {
            *left = left.saturating_sub(1);
            other.clone()
        }
    }
}

/// Keep the shape under [`MAX_SHAPE_LEAVES`] leaves: first by limiting depth,
/// then by dropping keys. The flag says whether anything was cut.
fn bound(s: Shape) -> (Shape, bool) {
    if leaves(&s) <= MAX_SHAPE_LEAVES {
        return (s, false);
    }
    for depth in (1..=MAX_SHAPE_DEPTH).rev() {
        let cut = cut_depth(&s, depth);
        if leaves(&cut) <= MAX_SHAPE_LEAVES {
            return (cut, true);
        }
    }
    let mut left = MAX_SHAPE_LEAVES;
    (cut_leaves(&cut_depth(&s, 1), &mut left), true)
}

impl Shape {
    fn to_value(&self) -> Value {
        match self {
            Shape::Leaf(t) => t.as_str().into(),
            Shape::Strings(t, avg, n, ex) => json!([t, avg, n, ex]),
            Shape::Array(l, i) => {
                let mut m = Map::new();
                m.insert("T".into(), "A".into());
                m.insert("L".into(), l.to_value());
                if let Some(i) = i {
                    m.insert("I".into(), i.to_value());
                }
                Value::Object(m)
            }
            Shape::Object(f) => {
                Value::Object(f.iter().map(|(k, v)| (k.clone(), v.to_value())).collect())
            }
        }
    }
}

// --- YAML -----------------------------------------------------------------------

fn is_scalar(v: &Value) -> bool {
    !v.is_array() && !v.is_object()
}

/// A string-stats tuple: `[tag, avg, count, [examples]]`.
fn is_stats_tuple(a: &[Value]) -> bool {
    a.len() == 4
        && a[0].is_string()
        && a[1].is_number()
        && a[2].is_number()
        && a[3].as_array().is_some_and(|e| e.iter().all(is_scalar))
}

fn number(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        _ => n.to_string(),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number(n),
        Value::String(s) if s.len() == 1 && s.chars().all(|c| c.is_ascii_uppercase()) => s.clone(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        _ => unreachable!(),
    }
}

fn key(k: &str) -> String {
    let mut chars = k.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain {
        k.to_string()
    } else {
        serde_json::to_string(k).unwrap_or_default()
    }
}

fn flow(a: &[Value]) -> String {
    if is_stats_tuple(a) {
        let ex: Vec<String> = a[3].as_array().unwrap().iter().map(scalar).collect();
        return format!(
            "[{}, {}, {}, [{}]]",
            scalar(&a[0]),
            scalar(&a[1]),
            scalar(&a[2]),
            ex.join(", ")
        );
    }
    let items: Vec<String> = a.iter().map(scalar).collect();
    format!("[{}]", items.join(", "))
}

/// A small YAML writer: block mappings and sequences, flow sequences for
/// scalar lists, and JSON-quoted strings so any value reads back safely.
pub fn yaml(v: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match v {
        Value::Array(a) if a.is_empty() => format!("{pad}[]"),
        Value::Array(a) if a.iter().all(is_scalar) || is_stats_tuple(a) => {
            format!("{pad}{}", flow(a))
        }
        Value::Array(a) => a
            .iter()
            .map(|item| {
                if is_scalar(item) {
                    format!("{pad}- {}", scalar(item))
                } else {
                    format!("{pad}-\n{}", yaml(item, indent + 2))
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(m) if m.is_empty() => format!("{pad}{{}}"),
        Value::Object(m) => m
            .iter()
            .map(|(k, val)| {
                let k = key(k);
                match val {
                    v if is_scalar(v) => format!("{pad}{k}: {}", scalar(v)),
                    Value::Array(a) if a.is_empty() => format!("{pad}{k}: []"),
                    Value::Array(a) if a.iter().all(is_scalar) || is_stats_tuple(a) => {
                        format!("{pad}{k}: {}", flow(a))
                    }
                    Value::Object(o) if o.is_empty() => format!("{pad}{k}: {{}}"),
                    other => format!("{pad}{k}:\n{}", yaml(other, indent + 2)),
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => format!("{pad}{}", scalar(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(payload: &'a Value, text: &'a str, level: Level) -> Input<'a> {
        Input {
            payload,
            text,
            path: "/tmp/out.json",
            level,
            fields: &[],
            compact: None,
            command: "jira.workitem.query",
        }
    }

    #[test]
    fn yaml_writer() {
        let v = json!({"a": "x", "b": [1, 2], "c": [{"d": null}], "e": {}, "f.g": "S", "h": []});
        assert_eq!(
            yaml(&v, 0),
            "a: \"x\"\nb: [1, 2]\nc:\n  -\n    d: null\ne: {}\n\"f.g\": S\nh: []"
        );
    }

    #[test]
    fn small_payloads_are_inlined() {
        let v = json!({"issues": [{"key": "A-1"}]});
        let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
        let out = envelope(&input(&v, &text, Level::Auto));
        assert_eq!(
            out,
            format!(
                "output_files:\n  stdout: \"/tmp/out.json\"\n  stdout_lines: 8\n  stdout_bytes: {}\ncommand: \"jira.workitem.query\"\nstdout_inline:\n  issues:\n    -\n      key: \"A-1\"\n---END---\n",
                text.len()
            )
        );
    }

    #[test]
    fn stats_describe_the_payload() {
        let v = json!({"issues": [
            {"key": "A-1", "n": 1, "tags": ["x"]},
            {"key": "A-22", "n": null, "tags": []}
        ]});
        let out = envelope(&input(&v, "{}", Level::Stats));
        assert!(
            out.contains("agent_output:\n  summary: \"stats\"\n"),
            "{out}"
        );
        assert!(out.contains("  top_level_keys: [\"issues\"]\n  collections:\n    issues: 2\n    \"issues.tags\": 1\n"), "{out}");
        assert!(out.contains("      key: [S, 3.5, 2, [\"A-1\", \"A-22\"]]\n      n: \"N | Z\"\n      tags:\n        T: A\n        L: \"0..1\"\n        I: [S, 1, 1, [\"x\"]]\n"), "{out}");
        assert!(!out.contains("stdout_inline"));
    }

    #[test]
    fn agent_fields_shape_the_inline_payload() {
        let v = json!({"issues": [{"key": "A-1", "summary": "s"}]});
        let fields = select::parse_paths(["data.issues.key", "data.issues.nope", "x.y"]);
        let i = Input {
            fields: &fields,
            compact: Some(("/tmp/out.compact.json", 10)),
            ..input(&v, "{}", Level::Auto)
        };
        let out = envelope(&i);
        assert!(out.contains("  compact: \"/tmp/out.compact.json\"\n  compact_bytes: 10\n"));
        assert!(out.contains("  fields: [\"data.issues.key\", \"data.issues.nope\", \"x.y\"]\n  field_source: \"explicit\"\n  fields_unresolved: [\"data.issues.nope\"]\n"), "{out}");
        assert!(
            out.ends_with("stdout_inline:\n  issues:\n    -\n      key: \"A-1\"\n---END---\n"),
            "{out}"
        );
    }

    #[test]
    fn errors_keep_their_code() {
        let v =
            json!({"ok": false, "error": {"code": "NOT_FOUND", "message": "gone", "status": 404}});
        let out = envelope(&input(&v, "{}", Level::Auto));
        assert!(
            out.starts_with(
                "ok: false\nerror:\n  code: \"NOT_FOUND\"\n  message: \"gone\"\noutput_files:"
            ),
            "{out}"
        );
    }

    #[test]
    fn large_shapes_are_bounded() {
        let wide: Map<String, Value> = (0..200).map(|i| (format!("k{i}"), json!(i))).collect();
        let (s, cut) = bound(shape_of(&Value::Object(wide)));
        assert!(cut);
        assert_eq!(leaves(&s), MAX_SHAPE_LEAVES);
    }
}

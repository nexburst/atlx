//! Output: compact JSON for agents and scripts, aligned text for humans.
//!
//! Every command's result is one [`envelope`]. In JSON modes stdout carries
//! exactly that one document; text mode prints its `data` as a table or a
//! list of fields. [`notes`] collects the warnings and truncation markers a
//! command reports while it runs.

pub mod envelope;
pub mod select;

use clap::ValueEnum;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Tables and fields for people (default on a terminal).
    Text,
    /// One line of compact JSON: the envelope (default when piped).
    Json,
    /// One record per line, then a trailer line
    /// `{"_end":{"next":…,"truncated":…,"errors":…}}`.
    Jsonl,
    /// One primary ID per line, for piping into another atlx command.
    Ids,
}

impl Format {
    /// JSON and JSONL.
    pub fn is_json(self) -> bool {
        matches!(self, Format::Json | Format::Jsonl)
    }
}

/// Warnings and truncation markers reported while a command runs; `main`
/// adds them to the envelope.
pub mod notes {
    use crate::schema::Truncation;
    use serde_json::{Value, json};
    use std::sync::Mutex;

    static WARNINGS: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    static TRUNCATED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

    /// Add `{"code": code, "message": message}` to `warnings`.
    pub fn warn(code: &str, message: impl Into<String>) {
        warn_value(json!({"code": code, "message": message.into()}));
    }

    /// Add any warning object (e.g. `{"code":"lossy_body","nodes":[…]}`).
    pub fn warn_value(w: Value) {
        WARNINGS.lock().unwrap_or_else(|e| e.into_inner()).push(w);
    }

    /// Add a `truncated` entry.
    pub fn truncated(t: &Truncation) {
        TRUNCATED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(t.to_json());
    }

    /// Take everything reported so far: `(warnings, truncated)`.
    pub fn take() -> (Vec<Value>, Vec<Value>) {
        let w = std::mem::take(&mut *WARNINGS.lock().unwrap_or_else(|e| e.into_inner()));
        let t = std::mem::take(&mut *TRUNCATED.lock().unwrap_or_else(|e| e.into_inner()));
        (w, t)
    }
}

/// `v` as it is printed in format `f`, trailing newline included.
/// `primary` names the ID field for `-o ids` (from the command's record).
pub fn render(v: &Value, f: Format, primary: Option<&str>) -> String {
    match f {
        Format::Json => serde_json::to_string(v).unwrap_or_default() + "\n",
        // One record reads better as fields than as a one-row table.
        Format::Text if envelope::is_envelope(v) => match v["data"].as_array() {
            Some(one) if one.len() == 1 && one[0].is_object() => to_text(&one[0]),
            _ => to_text(&v["data"]),
        },
        Format::Text => to_text(v),
        Format::Jsonl => jsonl(v),
        Format::Ids => ids(v, primary),
    }
}

/// The records of an envelope's `data`: its elements, or itself.
fn records(v: &Value) -> Vec<&Value> {
    match &v["data"] {
        Value::Array(a) => a.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn jsonl(v: &Value) -> String {
    if envelope::is_error(v) || !envelope::is_envelope(v) {
        return serde_json::to_string(v).unwrap_or_default() + "\n";
    }
    let mut out = String::new();
    for r in records(v) {
        out.push_str(&serde_json::to_string(r).unwrap_or_default());
        out.push('\n');
    }
    let end = json!({"_end": {
        "next": v.get("next").cloned().unwrap_or(Value::Null),
        "truncated": v.get("truncated").cloned().unwrap_or(json!([])),
        "errors": v.get("errors").cloned().unwrap_or(json!([])),
    }});
    out.push_str(&end.to_string());
    out.push('\n');
    out
}

/// The ID fields tried, in order, when a command has no record yet.
const ID_KEYS: &[&str] = &["key", "id", "accountId", "uuid", "slug", "name"];

fn ids(v: &Value, primary: Option<&str>) -> String {
    let mut out = String::new();
    for r in records(v) {
        let id = match primary {
            Some(p) => crate::schema::path(r, p).cloned(),
            None => ID_KEYS.iter().find_map(|k| r.get(*k).cloned()),
        };
        match id {
            Some(Value::String(s)) => out.push_str(&s),
            Some(Value::Null) | None => continue,
            Some(other) => out.push_str(&other.to_string()),
        }
        out.push('\n');
    }
    out
}

const MAX_CELL: usize = 60;

pub fn to_text(v: &Value) -> String {
    let mut out = String::new();
    render_text(v, &mut out, 0);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// A table cell: objects show their most telling name-like field (a
/// status's `name`, a user's `displayName`), else compact JSON.
fn cell_text(v: &Value) -> String {
    match v {
        Value::Object(m) => ["displayName", "name", "key", "value", "title", "id"]
            .iter()
            .find_map(|k| m.get(*k).filter(|x| !x.is_null() && !x.is_object()))
            .map(scalar)
            .unwrap_or_else(|| v.to_string()),
        Value::Array(a) => a.iter().map(cell_text).collect::<Vec<_>>().join(", "),
        other => scalar(other),
    }
}

fn cell(v: &Value) -> String {
    let s = cell_text(v).replace(['\n', '\r', '\t'], " ");
    if s.chars().count() > MAX_CELL {
        s.chars().take(MAX_CELL - 1).collect::<String>() + "…"
    } else {
        s
    }
}

fn is_table(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_object))
}

fn render_text(v: &Value, out: &mut String, indent: usize) {
    let pad = "  ".repeat(indent);
    match v {
        Value::Array(a) if a.is_empty() => out.push_str(&format!("{pad}(none)\n")),
        Value::Array(_) if is_table(v) => table(v.as_array().unwrap(), out, &pad),
        Value::Array(a) => {
            for item in a {
                out.push_str(&format!("{pad}{}\n", scalar(item)));
            }
        }
        Value::Object(m) => {
            let width = m.keys().map(|k| k.len()).max().unwrap_or(0);
            for (k, val) in m {
                match val {
                    Value::Array(a) if a.is_empty() => {
                        out.push_str(&format!("{pad}{k:<width$}  (none)\n"))
                    }
                    Value::Object(_) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        render_text(val, out, indent + 1);
                    }
                    Value::Array(_) if is_table(val) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        render_text(val, out, indent + 1);
                    }
                    Value::String(s) if s.contains('\n') => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        for line in s.lines() {
                            out.push_str(&format!("{pad}  {line}\n"));
                        }
                    }
                    _ => out.push_str(&format!("{pad}{k:<width$}  {}\n", scalar(val))),
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", scalar(other))),
    }
}

fn table(rows: &[Value], out: &mut String, pad: &str) {
    let mut cols: Vec<&String> = Vec::new();
    for r in rows {
        for k in r.as_object().unwrap().keys() {
            if !cols.contains(&k) {
                cols.push(k);
            }
        }
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cols.iter().map(|c| cell(&r[c.as_str()])).collect())
        .collect();
    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0)
                .max(c.len())
        })
        .collect();
    let line = |vals: Vec<String>| -> String {
        let mut s = pad.to_string();
        for (i, v) in vals.iter().enumerate() {
            if i + 1 == vals.len() {
                s.push_str(v);
            } else {
                s.push_str(v);
                s.push_str(&" ".repeat(widths[i] - v.chars().count() + 2));
            }
        }
        s.trim_end().to_string() + "\n"
    };
    out.push_str(&line(cols.iter().map(|c| c.to_uppercase()).collect()));
    for r in cells {
        out.push_str(&line(r));
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, render, to_text};
    use serde_json::json;

    #[test]
    fn json_is_one_compact_line_and_jsonl_ends_with_a_trailer() {
        let v = json!({"ok": true, "data": [{"key": "A-1"}, {"key": "A-2"}], "next": "o:2"});
        assert_eq!(
            render(&v, Format::Json, None),
            "{\"ok\":true,\"data\":[{\"key\":\"A-1\"},{\"key\":\"A-2\"}],\"next\":\"o:2\"}\n"
        );
        let lines: Vec<String> = render(&v, Format::Jsonl, None)
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], r#"{"key":"A-1"}"#);
        assert_eq!(
            lines[2],
            r#"{"_end":{"next":"o:2","truncated":[],"errors":[]}}"#
        );
        let err = json!({"ok": false, "error": {"code": "usage"}});
        assert_eq!(render(&err, Format::Jsonl, None).lines().count(), 1);
    }

    #[test]
    fn ids_print_one_primary_id_per_line() {
        let v = json!({"ok": true, "data": [{"key": "A-1", "id": "1"}, {"id": 7}, {"x": 1}]});
        assert_eq!(render(&v, Format::Ids, None), "A-1\n7\n");
        assert_eq!(render(&v, Format::Ids, Some("id")), "1\n7\n");
        let one = json!({"ok": true, "data": {"accountId": "abc"}});
        assert_eq!(render(&one, Format::Ids, None), "abc\n");
    }

    #[test]
    fn renders_tables_and_objects() {
        let v = json!([{"key": "A-1", "summary": "First"}, {"key": "A-22", "summary": null}]);
        assert_eq!(to_text(&v), "KEY   SUMMARY\nA-1   First\nA-22\n");
        let o = json!({"key": "A-1", "description": "line1\nline2", "jira": []});
        assert_eq!(
            to_text(&o),
            "key          A-1\ndescription:\n  line1\n  line2\njira         (none)\n"
        );
    }
}

//! Output: JSON for agents and scripts, aligned text for humans.
//!
//! The global output controls live in the submodules: `--select` and
//! `--agent-fields` in [`select`], `--output-summary` in [`summary`].

pub mod select;
pub mod summary;

use clap::ValueEnum;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Text,
    Json,
    /// One JSON record per line: the elements of the main list (the payload
    /// itself, or its `issues`/`results` array), else the whole payload.
    Jsonl,
}

pub fn print(v: &Value, f: Format) {
    print!("{}", render(v, f));
}

/// `v` as it is printed in format `f`, trailing newline included.
pub fn render(v: &Value, f: Format) -> String {
    match f {
        Format::Json => serde_json::to_string_pretty(v).unwrap_or_default() + "\n",
        Format::Text => to_text(v),
        Format::Jsonl => {
            let rows = v
                .as_array()
                .or_else(|| v["issues"].as_array())
                .or_else(|| v["results"].as_array());
            match rows {
                Some(rows) => rows.iter().map(|r| format!("{r}\n")).collect(),
                None => format!("{v}\n"),
            }
        }
    }
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

fn cell(v: &Value) -> String {
    let s = scalar(v).replace(['\n', '\r', '\t'], " ");
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
    use super::to_text;
    use serde_json::json;

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

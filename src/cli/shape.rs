//! Global output controls (`--fields`, `--full`, `--raw`, `--max-chars`,
//! `--output-file`) and the code that turns a command's result into the
//! printed envelope: projection onto the command's record, truncation
//! markers, warnings, partial errors, and `ok` matching the exit code.

use super::{Out, util};
use crate::client::{ApiError, DryRun};
use crate::output::envelope::{self, is_envelope};
use crate::output::{self, Format, notes, select};
use crate::schema::{self, Record, Selection, Unit};
use clap::Args;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Args, Debug, Default)]
pub(crate) struct OutputArgs {
    /// Print only these comma-separated fields of each record (`key,status`,
    /// `assignee.name`); an unknown name is a usage error listing the valid
    /// ones
    #[arg(long, global = true, value_name = "FIELDS", conflicts_with_all = ["full", "raw"])]
    pub fields: Option<String>,
    /// Print every field of the record, not just the default view
    #[arg(long, global = true, conflicts_with = "raw")]
    pub full: bool,
    /// Print the REST payload unprojected
    #[arg(long, global = true)]
    pub raw: bool,
    /// Character budget for large text fields (bodies, diffs); 0 turns
    /// truncation off. Cuts are listed in the envelope's `truncated`
    #[arg(long, global = true, value_name = "N")]
    pub max_chars: Option<usize>,
    /// Write `data` (or the body, with `--body-only`) to this file and print
    /// `{"file","bytes","lines"}` instead
    #[arg(long, global = true, value_name = "PATH")]
    pub output_file: Option<PathBuf>,
}

/// The resolved output controls for one run.
pub(crate) struct Shaping {
    fmt: Format,
    path: String,
    record: Option<&'static Record>,
    selection: Selection,
    max_chars: Option<usize>,
    /// Values of the command's own offset flags (`body-offset`, ...).
    offsets: HashMap<String, usize>,
    output_file: Option<PathBuf>,
}

impl Shaping {
    /// Resolve the flags for the command at `path`. `--fields` is checked
    /// against the command's record here, before any request is sent.
    pub(crate) fn new(
        a: OutputArgs,
        fmt: Format,
        path: &str,
        leaf: Option<&clap::ArgMatches>,
    ) -> anyhow::Result<Self> {
        let record = schema::registry::record(path);
        let selection = if a.raw {
            Selection::Raw
        } else if a.full {
            Selection::Full
        } else if let Some(f) = &a.fields {
            let names: Vec<String> = f
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if names.is_empty() {
                return Err(ApiError::usage("--fields needs at least one field name").into());
            }
            Selection::Fields(names)
        } else {
            Selection::Default
        };
        if let Some(r) = record {
            selection.validate(r)?;
        }
        let mut offsets = HashMap::new();
        if let (Some(r), Some(m)) = (record, leaf) {
            for f in r.fields.iter().filter_map(|f| f.truncate) {
                if let Some(n) = arg_number(m, f.offset_flag) {
                    offsets.insert(f.offset_flag.to_string(), n);
                }
            }
        }
        Ok(Shaping {
            fmt,
            path: path.to_string(),
            record,
            selection,
            max_chars: a.max_chars,
            offsets,
            output_file: a.output_file,
        })
    }

    /// No output controls: print results as they are.
    pub(crate) fn plain(fmt: Format, path: &str) -> Self {
        Shaping {
            fmt,
            path: path.to_string(),
            record: None,
            selection: Selection::Default,
            max_chars: None,
            offsets: HashMap::new(),
            output_file: None,
        }
    }

    /// Print a command's result and return the exit code.
    pub(crate) fn emit(&self, out: Out, code: i32) -> i32 {
        match out {
            Out::Data(v) | Out::Exit(v, _) => self.emit_value(v, code),
            Out::Doc { text, .. } if self.fmt == Format::Text => {
                println!("{text}");
                code
            }
            Out::Doc { full, .. } => self.emit_value(full, code),
            Out::Raw(s) => {
                println!("{s}");
                code
            }
            Out::Body { body, .. } => match &self.output_file {
                None => {
                    println!("{body}");
                    code
                }
                Some(p) => self.emit_file(p, &body, code),
            },
            Out::Streamed(c) => c,
        }
    }

    fn emit_value(&self, v: Value, code: i32) -> i32 {
        let env = match self.finish(v, code) {
            Ok(env) => env,
            Err(e) => return self.emit_error(&ApiError::from_anyhow(&e)),
        };
        if let Some(p) = &self.output_file {
            let text = match &env["data"] {
                Value::String(s) => s.clone(),
                d => serde_json::to_string_pretty(d).unwrap_or_default(),
            };
            return self.emit_file(p, &text, code);
        }
        if self.fmt == Format::Text {
            self.text_notes(&env);
        }
        print!(
            "{}",
            output::render(&env, self.fmt, self.record.map(|r| r.primary))
        );
        code
    }

    /// The printed envelope for a result `v` that exits with `code`.
    pub(crate) fn finish(&self, v: Value, code: i32) -> anyhow::Result<Value> {
        let mut env = if is_envelope(&v) {
            v
        } else {
            envelope::success(v)
        };
        let data = env["data"].take();
        env["data"] = self.shape_data(data)?;
        let (mut warnings, mut truncated) = notes::take();
        if let Some(r) = self.record {
            self.truncate(&mut env["data"], r, &mut truncated);
        }
        extend(&mut env, "warnings", warnings.drain(..));
        extend(&mut env, "truncated", truncated.drain(..));
        let errors = util::take_errors();
        if !errors.is_empty() {
            env["partial"] = true.into();
            extend(&mut env, "errors", errors.into_iter());
        }
        env["ok"] = (code == 0).into();
        Ok(envelope::ordered(env))
    }

    /// Apply the record view or `--fields` to `data`.
    fn shape_data(&self, data: Value) -> anyhow::Result<Value> {
        if data.is_null() {
            return Ok(data);
        }
        match (self.record, &self.selection) {
            (_, Selection::Raw) => Ok(data),
            (Some(r), sel) => schema::project(&data, r, sel),
            (None, Selection::Fields(names)) => {
                let paths = select::parse_paths(names.iter().map(String::as_str));
                select::project(&data, &paths).ok_or_else(|| {
                    let valid = select::keys(&data);
                    let unknown: Vec<&str> = names.iter().map(String::as_str).collect();
                    let valid: Vec<&str> = valid.iter().map(String::as_str).collect();
                    schema::unknown_fields(&unknown, &valid)
                })
            }
            (None, _) => Ok(data),
        }
    }

    /// Cut the record's truncatable fields and note each cut.
    fn truncate(&self, data: &mut Value, r: &Record, out: &mut Vec<Value>) {
        let mut cut_one = |obj: &mut Value, prefix: &str| {
            for f in r.fields {
                let Some(b) = f.truncate else { continue };
                let Some(text) = obj.get(f.name).and_then(Value::as_str) else {
                    continue;
                };
                let limit = match b.unit {
                    Unit::Chars => self.max_chars.unwrap_or(b.limit),
                    Unit::Lines => b.limit,
                };
                let offset = self.offsets.get(b.offset_flag).copied().unwrap_or(0);
                let field = format!("{prefix}.{}", f.name);
                let (kept, t) = schema::truncate::cut(text, &b, limit, offset, &field);
                if let Some(t) = t {
                    obj[f.name] = kept.into();
                    out.push(t.to_json());
                }
            }
        };
        match data {
            Value::Array(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    cut_one(item, &format!("data.{i}"));
                }
            }
            obj => cut_one(obj, "data"),
        }
    }

    /// Text mode: warnings and cuts go to stderr as notes.
    fn text_notes(&self, env: &Value) {
        for w in env["warnings"].as_array().into_iter().flatten() {
            let msg = w["message"]
                .as_str()
                .map_or_else(|| w.to_string(), String::from);
            eprintln!("warning: {msg}");
        }
        for t in env["truncated"].as_array().into_iter().flatten() {
            let more = t["more"]
                .as_str()
                .map_or(String::new(), |m| format!("; continue with {m}"));
            eprintln!(
                "note: {} shows {} of {} {}{more}",
                t["field"].as_str().unwrap_or_default(),
                t["shown"],
                t["total"],
                t["unit"].as_str().unwrap_or_default()
            );
        }
    }

    /// `--output-file`: write `text`, print what was written.
    fn emit_file(&self, path: &Path, text: &str, code: i32) -> i32 {
        if let Err(e) = write_file(path, text) {
            let err = ApiError::new(
                crate::client::ErrorKind::Other,
                format!("could not write {}: {e}", path.display()),
            );
            return self.emit_error(&err);
        }
        let info = json!({
            "file": path.display().to_string(),
            "bytes": text.len(),
            "lines": text.lines().count(),
        });
        let mut env = envelope::success(info);
        env["ok"] = (code == 0).into();
        if self.fmt == Format::Text {
            println!("{}", path.display());
        } else {
            print!("{}", output::render(&env, self.fmt, Some("file")));
        }
        code
    }

    /// Print a failure and return its exit code. JSON modes print the error
    /// envelope on stdout and nothing on stderr; text mode prints a readable
    /// report on stderr; `-o ids` prints the envelope on stderr so stdout
    /// stays a list of IDs.
    pub(crate) fn emit_error(&self, api: &ApiError) -> i32 {
        notes::take();
        util::take_errors();
        let env = api.envelope();
        match self.fmt {
            Format::Text => {
                if self.path.is_empty() {
                    eprintln!("✗ {}", api.message);
                } else {
                    eprintln!("✗ Failed {}.", self.path.replace('.', " "));
                }
                eprint!("{}", api.report());
            }
            Format::Ids => eprint!("{}", output::render(&env, Format::Json, None)),
            _ => print!("{}", output::render(&env, self.fmt, None)),
        }
        api.exit_code()
    }

    /// Print the request a `--dry-run` stopped. Exit 0.
    pub(crate) fn emit_dry_run(&self, d: &DryRun) -> i32 {
        notes::take();
        util::take_errors();
        let env = json!({"ok": true, "dryRun": true, "data": {"request": d.to_json()}});
        if self.fmt == Format::Text {
            println!("dry run: would send {} {}", d.method, d.url);
            if let Some(b) = &d.body {
                println!("{}", serde_json::to_string_pretty(b).unwrap_or_default());
            }
            if std::io::stderr().is_terminal() {
                eprintln!("note: nothing was sent");
            }
        } else {
            print!("{}", output::render(&env, Format::Json, None));
        }
        crate::client::exit::OK
    }
}

/// Append `items` to the array at `key`, creating it.
fn extend(env: &mut Value, key: &str, items: impl Iterator<Item = Value>) {
    let items: Vec<Value> = items.collect();
    if items.is_empty() {
        return;
    }
    let Some(m) = env.as_object_mut() else { return };
    let slot = m.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(a) = slot {
        a.extend(items);
    }
}

/// A number-valued argument of the leaf command, whatever integer type the
/// command declares it with.
fn arg_number(m: &clap::ArgMatches, id: &str) -> Option<usize> {
    let id = id.replace('-', "_");
    if !m.ids().any(|i| i.as_str() == id) {
        return None;
    }
    if let Ok(Some(n)) = m.try_get_one::<usize>(&id) {
        return Some(*n);
    }
    if let Ok(Some(n)) = m.try_get_one::<u64>(&id) {
        return usize::try_from(*n).ok();
    }
    if let Ok(Some(n)) = m.try_get_one::<u32>(&id) {
        return Some(*n as usize);
    }
    if let Ok(Some(s)) = m.try_get_one::<String>(&id) {
        return s.parse().ok();
    }
    None
}

/// Write `text` to `path`, creating missing parent directories.
fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_follows_the_exit_code_and_lists_are_added() {
        let s = Shaping::plain(Format::Json, "x");
        let v = s.finish(json!({"a": 1}), 0).unwrap();
        assert_eq!(v, json!({"ok": true, "data": {"a": 1}}));
        let v = s.finish(json!({"a": 1}), 1).unwrap();
        assert_eq!(v["ok"], false);
        notes::warn("w", "careful");
        util::push_error("P-9", &ApiError::usage("bad"));
        let v = s.finish(json!([{"a": 1}]), 3).unwrap();
        assert_eq!(v["partial"], true);
        assert_eq!(v["errors"][0]["input"], "P-9");
        assert_eq!(v["warnings"][0]["code"], "w");
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["ok", "partial", "data", "warnings", "errors"]);
    }

    #[test]
    fn fields_on_unregistered_commands_select_paths_or_fail() {
        let mut s = Shaping::plain(Format::Json, "x");
        s.selection = Selection::Fields(vec!["key".into()]);
        let v = s.finish(json!([{"key": "A", "x": 1}]), 0).unwrap();
        assert_eq!(v["data"], json!([{"key": "A"}]));
        s.selection = Selection::Fields(vec!["nope".into()]);
        let err = s.finish(json!([{"key": "A", "x": 1}]), 0).unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.exit_code(), 2);
        assert!(api.hint.as_deref().unwrap().contains("key, x"));
    }
}

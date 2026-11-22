//! Global output controls: `--select`, `--agent-fields`, `--output-file`,
//! `--output-summary` and `--api-version`, and the code that applies them
//! to a command's result before it reaches stdout.

use super::Out;
use crate::output::select::{self, Fields, Selected};
use crate::output::summary::{self, Level};
use crate::output::{self, Format};
use clap::Args;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Args, Debug)]
pub(crate) struct OutputArgs {
    /// Keep only these comma-separated paths of the JSON output, e.g.
    /// `issues.key,issues.status`. `@compact`, `@evidence` and `@rows` expand
    /// to per-command presets. JSON and JSONL only
    #[arg(long, global = true, value_name = "PATHS")]
    pub select: Option<String>,
    /// Comma-separated paths (or presets) to show in the `--output-summary`
    /// envelope, which also writes them to a `.compact.json` file next to the
    /// full output. Has no effect without `--output-summary`
    #[arg(long, global = true, value_name = "PATHS")]
    pub agent_fields: Option<String>,
    /// Write stdout to this file instead and print `stdout=<path>`. With
    /// `--body-only -o json`, the body goes to the file and the metadata
    /// (with `body.outputFile`) is printed
    #[arg(long, global = true, value_name = "PATH")]
    pub output_file: Option<PathBuf>,
    /// Write the full JSON output to a file (`--output-file`, or a temporary
    /// one) and print a YAML envelope describing it [default level: auto]
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "LEVEL",
        num_args = 0..=1,
        default_missing_value = "auto"
    )]
    pub output_summary: Option<Level>,
    /// Output contract version; only `v2` exists
    #[arg(long, global = true, value_name = "VERSION", value_parser = ["v2"])]
    pub api_version: Option<String>,
}

/// The resolved output controls for one run.
pub(crate) struct Shaping {
    fmt: Format,
    command: String,
    select: Fields,
    agent_fields: Fields,
    output_file: Option<PathBuf>,
    summary: Option<Level>,
}

impl Shaping {
    /// Resolve presets for `command` (a dotted command path) and warn about
    /// flags that do nothing in text mode.
    pub(crate) fn new(a: OutputArgs, fmt: Format, command: &str) -> anyhow::Result<Self> {
        let structured = fmt != Format::Text;
        let resolve = |flag: &str, spec: &Option<String>| match spec {
            Some(s) => select::resolve(flag, s, command),
            None => Ok(Fields::default()),
        };
        let select = resolve("--select", &a.select)?;
        let agent_fields = resolve("--agent-fields", &a.agent_fields)?;
        if !structured && a.select.is_some() {
            eprintln!("warning: --select only applies to -o json and -o jsonl; ignoring it");
        }
        if !structured && a.output_summary.is_some() {
            eprintln!(
                "warning: --output-summary only applies to -o json and -o jsonl; ignoring it"
            );
        }
        Ok(Shaping {
            fmt,
            command: command.to_string(),
            select: if structured {
                select
            } else {
                Fields::default()
            },
            agent_fields,
            output_file: a.output_file,
            summary: a.output_summary.filter(|l| structured && *l != Level::None),
        })
    }

    /// No output controls: print results as they are.
    pub(crate) fn plain(fmt: Format, command: &str) -> Self {
        Shaping {
            fmt,
            command: command.to_string(),
            select: Fields::default(),
            agent_fields: Fields::default(),
            output_file: None,
            summary: None,
        }
    }

    /// Apply `--select` to a structured result, warning when it matched
    /// nothing or named presets the command lacks.
    fn select(&self, v: Value) -> Value {
        let f = &self.select;
        if f.paths.is_empty() && f.unresolved.is_empty() {
            return v;
        }
        let all = || {
            let mut names = f.unresolved.clone();
            names.extend(f.dotted());
            names.join(", ")
        };
        if f.paths.is_empty() {
            eprintln!(
                "warning: --select matched nothing ({}); printing the full output",
                all()
            );
            return v;
        }
        if !f.unresolved.is_empty() {
            eprintln!(
                "warning: --select ignored {} (no presets for this command); kept {}",
                f.unresolved.join(", "),
                f.dotted().join(", ")
            );
        }
        let paths = f.paths.clone();
        match select::select(v, &paths) {
            Selected::Matched(v) | Selected::Unchanged(v) => v,
            Selected::Unmatched(v) => {
                eprintln!(
                    "warning: --select matched nothing ({}); printing the full output",
                    all()
                );
                v
            }
        }
    }

    /// Print a successful result. Returns `code` unchanged.
    pub(crate) fn emit(&self, out: Out, code: i32) -> i32 {
        match out {
            Out::Data(v) | Out::Exit(v, _) => {
                let v = self.select(v);
                self.deliver(&output::render(&v, self.fmt), Some(&v));
            }
            Out::Raw(s) => self.deliver(&format!("{s}\n"), None),
            Out::Doc { text, .. } if self.fmt == Format::Text => {
                self.deliver(&format!("{text}\n"), None)
            }
            Out::Doc { full, .. } => {
                let v = self.select(full);
                self.deliver(&output::render(&v, self.fmt), Some(&v));
            }
            Out::Body { body, full } => self.emit_body(body, full),
        }
        code
    }

    /// Print a failure's JSON envelope (JSON modes only; text mode reports
    /// on stderr alone).
    pub(crate) fn emit_error(&self, envelope: &Value) {
        if self.fmt == Format::Text {
            if let Some(path) = &self.output_file {
                // The redirected stdout is empty, but the file still exists.
                self.write_or_print(path, "");
            }
            return;
        }
        self.deliver(&output::render(envelope, self.fmt), Some(envelope));
    }

    /// `--body-only`: the body alone, or with `--output-file`, the body in
    /// the file and (in JSON modes) the rest of the result on stdout.
    fn emit_body(&self, body: String, full: Value) {
        let Some(path) = &self.output_file else {
            println!("{body}");
            return;
        };
        if let Err(e) = write_file(path, &body) {
            eprintln!(
                "warning: could not write {}: {e}; printing to stdout",
                path.display()
            );
            println!("{body}");
            return;
        }
        if self.fmt == Format::Text {
            println!("stdout={}", path.display());
            return;
        }
        let v = self.select(externalize_body(full, &path.display().to_string()));
        print!("{}", output::render(&v, self.fmt));
    }

    fn deliver(&self, text: &str, payload: Option<&Value>) {
        // Like twg, a JSON document written to a file has no final newline.
        let text = match (self.fmt, payload) {
            (Format::Json, Some(_)) if self.output_file.is_some() || self.summary.is_some() => {
                text.strip_suffix('\n').unwrap_or(text)
            }
            _ => text,
        };
        if let (Some(level), Some(payload)) = (self.summary, payload) {
            self.summarize(level, text, payload);
        } else if let Some(path) = &self.output_file {
            self.write_or_print(path, text);
        } else {
            print!("{text}");
        }
    }

    fn write_or_print(&self, path: &Path, text: &str) {
        match write_file(path, text) {
            Ok(()) => println!("stdout={}", path.display()),
            Err(e) => {
                eprintln!(
                    "warning: could not write {}: {e}; printing to stdout",
                    path.display()
                );
                print!("{text}");
            }
        }
    }

    fn summarize(&self, level: Level, text: &str, payload: &Value) {
        let path = self
            .output_file
            .clone()
            .unwrap_or_else(|| temp_output(self.fmt));
        if let Err(e) = write_file(&path, text) {
            eprintln!(
                "warning: could not write {}: {e}; printing to stdout",
                path.display()
            );
            print!("{text}");
            return;
        }
        let fields = &self.agent_fields.paths;
        let compact_path = compact_path(&path);
        let compact = if fields.is_empty() {
            None
        } else {
            let c = serde_json::to_string(&summary::compact_payload(payload, fields))
                .unwrap_or_default();
            write_file(&compact_path, &c).ok().map(|_| c.len())
        };
        let path_s = path.display().to_string();
        let compact_s = compact_path.display().to_string();
        print!(
            "{}",
            summary::envelope(&summary::Input {
                payload,
                text,
                path: &path_s,
                level,
                fields,
                compact: compact.map(|n| (compact_s.as_str(), n)),
                command: &self.command,
            })
        );
    }
}

/// Write `text` to `path`, creating missing parent directories.
fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// `<dir>/<stem>.compact.json` next to the full output.
fn compact_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stdout".into());
    path.with_file_name(format!("{stem}.compact.json"))
}

/// A fresh `$TMPDIR/atlx/<epoch>/run.<id>/stdout.<ext>` path.
fn temp_output(fmt: Format) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let id = format!("{:x}{:x}", std::process::id(), now.subsec_nanos());
    let ext = if fmt == Format::Jsonl {
        "jsonl"
    } else {
        "json"
    };
    std::env::temp_dir()
        .join("atlx")
        .join(now.as_secs().to_string())
        .join(format!("run.{id}"))
        .join(format!("stdout.{ext}"))
}

/// Replace the body text in a `--body-only` result with `body.outputFile`.
/// Handles atlx's `body: "<text>"` and twg's `body: {value, ...}`, at the
/// top level or under `data`.
pub(crate) fn externalize_body(mut full: Value, path: &str) -> Value {
    let target = if full.get("data").is_some_and(Value::is_object) {
        &mut full["data"]
    } else {
        &mut full
    };
    if let Some(obj) = target.as_object_mut() {
        let mut body = match obj.remove("body") {
            Some(Value::Object(b)) => b,
            _ => Map::new(),
        };
        body.remove("value");
        body.insert("outputFile".into(), path.into());
        obj.insert("body".into(), Value::Object(body));
    }
    full
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn externalizes_both_body_shapes() {
        assert_eq!(
            externalize_body(
                json!({"id": "1", "snapshot_token": "t", "body": "<p>x</p>"}),
                "/f"
            ),
            json!({"id": "1", "snapshot_token": "t", "body": {"outputFile": "/f"}})
        );
        assert_eq!(
            externalize_body(
                json!({"data": {"snapshotToken": "t", "body": {"format": "html", "value": "x"}}}),
                "/f"
            ),
            json!({"data": {"snapshotToken": "t", "body": {"format": "html", "outputFile": "/f"}}})
        );
    }

    #[test]
    fn compact_file_sits_next_to_the_output() {
        assert_eq!(
            compact_path(Path::new("/tmp/a/stdout.json")),
            PathBuf::from("/tmp/a/stdout.compact.json")
        );
    }
}

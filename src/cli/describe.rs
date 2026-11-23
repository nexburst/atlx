//! `atlx describe`: the command line documents itself as JSON.
//!
//! Built from the clap tree joined with the command registry
//! ([`crate::schema::registry`]), so it cannot drift from what the binary
//! accepts.
//!
//! - `atlx describe` — compact index: every command's path, summary, effect.
//! - `atlx describe jira` — the index narrowed to a product or group.
//! - `atlx describe jira issue get` — one command's full contract.
//! - `atlx describe jira --all` — the full contract of every command under
//!   a path.
//! - `atlx describe --records` / `--errors` — record types; error codes and
//!   exit codes.

use super::{Cli, Out};
use crate::client::{ApiError, ErrorKind, exit};
use crate::schema::{Effect, registry};
use anyhow::Result;
use clap::{ArgAction, Args, CommandFactory};
use serde_json::{Map, Value, json};
use std::any::TypeId;

/// Bumped on any breaking change to records, the envelope or exit codes.
pub const CONTRACT: u32 = 1;

#[derive(Args)]
pub(crate) struct DescribeArgs {
    /// A command path (`jira issue get`) or a group (`jira`); none for the
    /// whole index
    path: Vec<String>,
    /// List the record types commands return, with their fields
    #[arg(long, conflicts_with_all = ["errors", "all"])]
    records: bool,
    /// List error codes and exit codes
    #[arg(long, conflicts_with = "all")]
    errors: bool,
    /// Print the full contract of every command under the path
    #[arg(long)]
    all: bool,
}

pub(crate) fn run(a: DescribeArgs) -> Result<Out> {
    let data = if a.records {
        json!({"contract": CONTRACT, "records": records()})
    } else if a.errors {
        errors()
    } else {
        describe(&a.path, a.all)?
    };
    Ok(Out::Data(data))
}

/// A leaf command of the tree with its path.
pub(crate) struct Leaf {
    pub path: String,
    pub cmd: clap::Command,
}

/// Every visible runnable command, in tree order.
pub(crate) fn leaves() -> Vec<Leaf> {
    let mut out = Vec::new();
    let root = Cli::command();
    walk(&root, "", &mut out);
    out
}

fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<Leaf>) {
    for sc in cmd.get_subcommands() {
        if sc.is_hide_set() || sc.get_name() == "help" {
            continue;
        }
        let path = if prefix.is_empty() {
            sc.get_name().to_string()
        } else {
            format!("{prefix} {}", sc.get_name())
        };
        if sc.has_subcommands() {
            walk(sc, &path, out);
        } else {
            out.push(Leaf {
                path,
                cmd: sc.clone(),
            });
        }
    }
}

fn describe(path: &[String], all: bool) -> Result<Value> {
    let prefix = path.join(" ");
    let leaves = leaves();
    if let Some(leaf) = leaves.iter().find(|l| l.path == prefix) {
        return Ok(contract(leaf));
    }
    let under: Vec<&Leaf> = leaves
        .iter()
        .filter(|l| prefix.is_empty() || l.path.starts_with(&format!("{prefix} ")))
        .collect();
    if under.is_empty() {
        let first = path.first().map(String::as_str).unwrap_or_default();
        let near: Vec<String> = leaves
            .iter()
            .filter(|l| l.path.split(' ').next() == Some(first))
            .take(10)
            .map(|l| format!("atlx describe {}", l.path))
            .collect();
        return Err(ApiError::usage(format!("no command `atlx {prefix}`"))
            .with_hint("run `atlx describe` for the list of commands")
            .with_suggestions(near)
            .into());
    }
    let mut out = Map::new();
    out.insert("contract".into(), CONTRACT.into());
    if all {
        out.insert(
            "commands".into(),
            under.iter().map(|l| contract(l)).collect::<Vec<_>>().into(),
        );
        return Ok(Value::Object(out));
    }
    if prefix.is_empty() {
        let root = Cli::command();
        out.insert(
            "globals".into(),
            root.get_arguments()
                .filter(|a| !a.is_hide_set())
                .map(arg)
                .collect::<Vec<_>>()
                .into(),
        );
    }
    out.insert(
        "commands".into(),
        under
            .iter()
            .map(|l| {
                json!({
                    "path": l.path,
                    "summary": about(&l.cmd),
                    "effect": registry::effect(&l.path).as_str(),
                })
            })
            .collect::<Vec<_>>()
            .into(),
    );
    Ok(Value::Object(out))
}

fn about(cmd: &clap::Command) -> String {
    cmd.get_about().map(|s| s.to_string()).unwrap_or_default()
}

/// The full contract of one command.
pub(crate) fn contract(leaf: &Leaf) -> Value {
    let meta = registry::lookup(&leaf.path);
    let effect = registry::effect(&leaf.path);
    let args: Vec<&clap::Arg> = leaf
        .cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_global_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .collect();
    let positionals: Vec<Value> = args
        .iter()
        .filter(|a| a.is_positional())
        .map(|a| arg(a))
        .collect();
    let flags: Vec<Value> = args
        .iter()
        .filter(|a| !a.is_positional())
        .map(|a| arg(a))
        .collect();
    // Multi-ID reads can end partial (exit 3).
    let multi = leaf.path.ends_with(" get")
        && args
            .iter()
            .any(|a| a.is_positional() && matches!(a.get_action(), ArgAction::Append));
    let mut codes = vec![
        exit::OK,
        exit::FAILURE,
        exit::USAGE,
        exit::ASK,
        exit::NOT_FOUND,
        exit::RETRY,
    ];
    if multi {
        codes.push(exit::PARTIAL);
    }
    if effect != Effect::Read {
        codes.extend([exit::CONFLICT, exit::REFUSED]);
    }
    codes.sort();
    let mut m = Map::new();
    m.insert("contract".into(), CONTRACT.into());
    m.insert("path".into(), leaf.path.clone().into());
    m.insert("summary".into(), about(&leaf.cmd).into());
    if let Some(long) = leaf.cmd.get_long_about() {
        m.insert("description".into(), long.to_string().into());
    }
    m.insert("effect".into(), effect.as_str().into());
    if effect == Effect::ByMethod {
        m.insert(
            "effectByMethod".into(),
            json!({"GET": "read", "HEAD": "read", "OPTIONS": "read",
                   "POST": "write", "PUT": "write", "PATCH": "write",
                   "DELETE": "destructive"}),
        );
    }
    m.insert("confirm".into(), meta.is_some_and(|m| m.confirm()).into());
    m.insert("positionals".into(), positionals.into());
    m.insert("flags".into(), flags.into());
    m.insert(
        "record".into(),
        meta.and_then(|m| m.record)
            .map_or(Value::Null, |r| r.describe()),
    );
    m.insert(
        "examples".into(),
        json!(meta.map_or(&[][..], |m| m.examples)),
    );
    m.insert("exitCodes".into(), json!(codes));
    if meta.is_none() {
        // Transitional: the effect above is guessed from the command name.
        m.insert("registered".into(), false.into());
    }
    Value::Object(m)
}

/// One argument's description.
fn arg(a: &clap::Arg) -> Value {
    let mut m = Map::new();
    let name = if a.is_positional() {
        a.get_value_names()
            .and_then(|v| v.first())
            .map_or_else(|| a.get_id().as_str().to_uppercase(), |s| s.to_string())
    } else {
        a.get_long().map_or_else(
            || format!("-{}", a.get_short().unwrap_or('?')),
            |l| format!("--{l}"),
        )
    };
    m.insert("name".into(), name.into());
    if let Some(s) = a.get_short() {
        m.insert("short".into(), format!("-{s}").into());
    }
    let values: Vec<String> = a
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    let ty = match a.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse => "boolean",
        ArgAction::Count => "count",
        _ if !values.is_empty() => "enum",
        _ => value_type(a),
    };
    m.insert("type".into(), ty.into());
    if !values.is_empty() && ty == "enum" {
        m.insert("values".into(), json!(values));
    }
    m.insert("required".into(), a.is_required_set().into());
    if matches!(a.get_action(), ArgAction::Append) {
        m.insert("repeatable".into(), true.into());
    }
    let defaults: Vec<String> = a
        .get_default_values()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    if !defaults.is_empty() && ty != "boolean" {
        m.insert("default".into(), defaults.join(",").into());
    }
    if let Some(env) = a.get_env() {
        m.insert("env".into(), env.to_string_lossy().into_owned().into());
    }
    if let Some(h) = a.get_help() {
        m.insert("description".into(), h.to_string().into());
    }
    Value::Object(m)
}

fn value_type(a: &clap::Arg) -> &'static str {
    let t = a.get_value_parser().type_id();
    let ints = [
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
    ];
    if ints.iter().any(|i| t == *i) {
        "integer"
    } else if t == TypeId::of::<f64>() {
        "number"
    } else if t == TypeId::of::<std::path::PathBuf>() {
        "path"
    } else {
        "string"
    }
}

fn records() -> Vec<Value> {
    registry::records().iter().map(|r| r.describe()).collect()
}

fn errors() -> Value {
    let codes: Vec<Value> = ErrorKind::ALL
        .iter()
        .map(|k| {
            json!({
                "code": k.as_str(), "exit": k.exit_code(), "meaning": k.meaning(),
                "hint": k.hint(), "retryable": k.retryable(),
            })
        })
        .collect();
    let exits = json!([
        {"exit": exit::OK, "meaning": "success"},
        {"exit": exit::FAILURE, "meaning": "failed: validation, server after retries, other; read `error`"},
        {"exit": exit::USAGE, "meaning": "usage: fix the command; see `suggestions` or `atlx describe`"},
        {"exit": exit::PARTIAL, "meaning": "partial: some inputs failed; use `data`, inspect `errors`"},
        {"exit": exit::ASK, "meaning": "config, auth, forbidden: stop and ask the person"},
        {"exit": exit::NOT_FOUND, "meaning": "not_found: check the ID"},
        {"exit": exit::CONFLICT, "meaning": "conflict: re-read, then retry"},
        {"exit": exit::RETRY, "meaning": "rate_limited, network, timeout: retry later"},
        {"exit": exit::REFUSED, "meaning": "refused: a safety guard stopped the write; do not work around it"},
    ]);
    json!({"contract": CONTRACT, "errors": codes, "exitCodes": exits})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    #[test]
    fn every_registered_command_exists_and_its_examples_parse() {
        let paths: Vec<String> = leaves().into_iter().map(|l| l.path).collect();
        for m in registry::all() {
            assert!(
                paths.iter().any(|p| p == m.path),
                "{} is not a command",
                m.path
            );
            for ex in m.examples {
                let argv = shell_words(ex);
                assert_eq!(argv[0], "atlx", "{ex}");
                if let Err(e) = Cli::try_parse_from(&argv) {
                    panic!("example `{ex}` does not parse: {e}");
                }
                let words = argv[1..]
                    .iter()
                    .take_while(|w| !w.starts_with('-'))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(
                    words.starts_with(m.path),
                    "example `{ex}` is not for `{}`",
                    m.path
                );
            }
        }
    }

    #[test]
    fn contracts_name_flags_types_and_exit_codes() {
        let leaves = leaves();
        let l = leaves.iter().find(|l| l.path == "describe").unwrap();
        let c = contract(l);
        assert_eq!(c["effect"], "read");
        let records = c["flags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "--records")
            .unwrap();
        assert_eq!(records["type"], "boolean");
        assert_eq!(c["positionals"][0]["repeatable"], true);
        assert_eq!(c["exitCodes"], json!([0, 1, 2, 4, 5, 7]));
        let idx = describe(&[], false).unwrap();
        assert_eq!(idx["contract"], CONTRACT);
        assert!(
            idx["globals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["name"] == "--dry-run")
        );
        let err = describe(&["nope".into()], false).unwrap_err();
        assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
    }
}

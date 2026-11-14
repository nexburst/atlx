//! Snapshot of the whole command tree: one line per leaf command with its
//! positional args and flags, global flags listed once at the top.
//!
//! Any change to commands, flags, aliases, defaults or value sets shows up as a
//! diff of `tests/snapshots/cli-tree.txt`. Regenerate it with:
//!
//! ```sh
//! UPDATE_SNAPSHOTS=1 cargo test --test cli_tree
//! ```

use atlx::cli::Cli;
use clap::CommandFactory;
use std::path::Path;

#[test]
fn cli_tree_matches_snapshot() {
    let actual = render_tree(&Cli::command());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/cli-tree.txt");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some_and(|v| v != "0" && !v.is_empty()) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    // Normalize line endings so a CRLF checkout on Windows still matches.
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n");
    if expected != actual {
        let mut diff = String::new();
        let exp: Vec<_> = expected.lines().collect();
        let act: Vec<_> = actual.lines().collect();
        for l in &exp {
            if !act.contains(l) {
                diff.push_str(&format!("- {l}\n"));
            }
        }
        for l in &act {
            if !exp.contains(l) {
                diff.push_str(&format!("+ {l}\n"));
            }
        }
        panic!(
            "CLI tree differs from {} (rerun with UPDATE_SNAPSHOTS=1 to accept):\n{diff}",
            path.display()
        );
    }
}

/// Render the CLI as one sorted line per leaf command, with global flags
/// listed once at the top.
fn render_tree(root: &clap::Command) -> String {
    let mut globals = Vec::new();
    for a in root.get_arguments() {
        if a.is_global_set() && !is_builtin(a) {
            globals.push(format!("global: {}", render_arg(a)));
        }
    }
    let mut aliases = Vec::new();
    let mut leaves = Vec::new();
    walk(root, root.get_name(), &mut leaves, &mut aliases);
    leaves.sort();
    aliases.sort();
    let mut out = String::new();
    for l in globals.iter().chain(&aliases).chain(&leaves) {
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn is_builtin(a: &clap::Arg) -> bool {
    matches!(a.get_id().as_str(), "help" | "version")
}

fn walk(cmd: &clap::Command, path: &str, leaves: &mut Vec<String>, aliases: &mut Vec<String>) {
    let subs: Vec<_> = cmd.get_subcommands().collect();
    let own: Vec<_> = cmd
        .get_arguments()
        .filter(|a| !a.is_global_set() && !is_builtin(a))
        .collect();
    // A command with subcommands is also listed when it takes arguments of
    // its own (e.g. `confluence content labels <ref>`), since it then runs
    // by itself too.
    if subs.is_empty() || !own.is_empty() {
        let mut line = path.to_string();
        if cmd.is_hide_set() {
            line.push_str(" (hidden)");
        }
        for a in own {
            line.push(' ');
            line.push_str(&render_arg(a));
        }
        leaves.push(line);
    }
    if subs.is_empty() {
        return;
    }
    for s in subs {
        let sub_path = format!("{path} {}", s.get_name());
        let visible: Vec<_> = s.get_visible_aliases().collect();
        for al in s.get_all_aliases() {
            let kind = if visible.contains(&al) {
                "alias"
            } else {
                "hidden-alias"
            };
            aliases.push(format!("{kind}: {path} {al} -> {sub_path}"));
        }
        walk(s, &sub_path, leaves, aliases);
    }
}

fn render_arg(a: &clap::Arg) -> String {
    let takes_value = matches!(
        a.get_action(),
        clap::ArgAction::Set | clap::ArgAction::Append
    );
    let multiple = matches!(a.get_action(), clap::ArgAction::Append)
        || a.get_num_args().is_some_and(|n| n.max_values() > 1);
    let value_name = a
        .get_value_names()
        .map(|v| v.join(" "))
        .unwrap_or_else(|| a.get_id().as_str().to_uppercase());
    let mut s = if a.is_positional() {
        format!("<{value_name}>")
    } else {
        let mut names = Vec::new();
        if let Some(c) = a.get_short() {
            names.push(format!("-{c}"));
        }
        if let Some(l) = a.get_long() {
            names.push(format!("--{l}"));
        }
        for al in a.get_all_aliases().unwrap_or_default() {
            names.push(format!("--{al}"));
        }
        for al in a.get_all_short_aliases().unwrap_or_default() {
            names.push(format!("-{al}"));
        }
        let mut s = names.join("/");
        if takes_value {
            s.push_str(&format!(" <{value_name}>"));
        }
        s
    };
    if let Some(d) = a.get_value_delimiter() {
        s.push_str(&format!("{d}"));
    }
    if multiple {
        s.push_str("...");
    }
    let pv = a.get_possible_values();
    if takes_value && !pv.is_empty() {
        let names: Vec<_> = pv
            .iter()
            .filter(|p| !p.is_hide_set())
            .map(|p| p.get_name().to_string())
            .collect();
        s.push_str(&format!(" {{{}}}", names.join("|")));
    }
    let defaults: Vec<_> = a
        .get_default_values()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    if !defaults.is_empty() {
        s.push_str(&format!(" [default: {}]", defaults.join(",")));
    }
    if let Some(e) = a.get_env() {
        s.push_str(&format!(" [env: {}]", e.to_string_lossy()));
    }
    if a.is_hide_set() {
        s.push_str(" (hidden)");
    }
    if a.is_required_set() {
        s.push_str(" (required)");
    } else if a.is_positional() || takes_value {
        s = format!("[{s}]");
    }
    s
}

//! twg compatibility checks, all offline:
//!
//! - `compat/corpus.txt`: twg-style invocations tagged `ok` (must parse) or
//!   `gap #<issue>` (must fail to parse until that issue lands).
//! - `COMPAT.md`: one row per twg leaf command in the inventory, with a status
//!   that agrees with the current command tree.
//! - `compat/coverage.txt`: share of twg leaf commands atlx accepts. The test
//!   fails if coverage drops; rerun with `UPDATE_COVERAGE=1` to record a rise.
//!
//! The twg inventory (`compat/twg-<version>.txt`) is produced by
//! `scripts/twg-inventory.sh` and the atlx tree by `tests/cli_tree.rs`.

use atlx::cli::Cli;
use clap::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const INVENTORY: &str = "compat/twg-1.4.4.txt";
/// Coverage buckets: products, plus `account` for auth, login, setup and the like.
const PRODUCTS: [&str; 6] = ["jira", "confluence", "bitbucket", "user", "jsm", "account"];
const MIN_CORPUS_LINES: usize = 100;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Split a corpus line into argv. Single and double quotes group words;
/// there are no escapes.
fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    assert!(quote.is_none(), "unbalanced quote in: {s}");
    if in_word {
        out.push(cur);
    }
    out
}

enum Tag {
    Ok,
    Gap(u32),
}

/// Parse `ok <invocation>` or `gap #<issue> <invocation>`.
fn parse_corpus_line(line: &str) -> (Tag, &str) {
    if let Some(rest) = line.strip_prefix("ok ") {
        return (Tag::Ok, rest.trim());
    }
    if let Some(rest) = line.strip_prefix("gap #") {
        let (num, inv) = rest
            .split_once(char::is_whitespace)
            .unwrap_or_else(|| panic!("corpus line without invocation: {line}"));
        let n = num
            .parse()
            .unwrap_or_else(|_| panic!("corpus gap needs an issue number: {line}"));
        return (Tag::Gap(n), inv.trim());
    }
    panic!("corpus line must start with `ok ` or `gap #<issue> `: {line}");
}

#[test]
fn corpus_tags_match_parser() {
    let corpus = read("compat/corpus.txt");
    let mut problems = Vec::new();
    let mut count = 0;
    let mut seen = BTreeSet::new();
    for (i, raw) in corpus.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        count += 1;
        let (tag, inv) = parse_corpus_line(line);
        if !seen.insert(inv.to_string()) {
            problems.push(format!("line {}: duplicate invocation: {inv}", i + 1));
        }
        let mut argv = vec!["atlx".to_string()];
        argv.extend(shell_words(inv));
        let result = Cli::try_parse_from(&argv);
        match (tag, result) {
            (Tag::Ok, Err(e)) => {
                let first = e.to_string().lines().next().unwrap_or("").to_string();
                problems.push(format!(
                    "line {}: tagged ok but does not parse: {inv}\n    {first}",
                    i + 1
                ));
            }
            (Tag::Gap(n), Ok(_)) => problems.push(format!(
                "line {}: tagged gap #{n} but now parses: {inv}\n    \
                 flip it to `ok` and update the COMPAT.md row",
                i + 1
            )),
            _ => {}
        }
    }
    assert!(
        count >= MIN_CORPUS_LINES,
        "compat/corpus.txt has {count} invocations, expected at least {MIN_CORPUS_LINES}"
    );
    assert!(
        problems.is_empty(),
        "compat/corpus.txt is out of date:\n{}",
        problems.join("\n")
    );
}

/// twg's global flags (from `twg --help`), which the inventory does not repeat
/// per command.
const TWG_GLOBAL_FLAGS: [&str; 11] = [
    "--user",
    "--site",
    "--profile",
    "--output",
    "--api-version",
    "--timeout-ms",
    "--output-summary",
    "--agent-fields",
    "--select",
    "--usage-counts",
    "--output-shape",
];

/// twg's leaf commands with their long flags, plus its aliases as
/// (parent path, alias) -> canonical name.
#[allow(clippy::type_complexity)]
fn twg_tree() -> (
    BTreeMap<String, BTreeSet<String>>,
    BTreeMap<(String, String), String>,
) {
    let inv = read(INVENTORY);
    let mut leaves: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in inv.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("alias: ") {
            let (from, to) = rest.split_once(" -> ").expect("alias line has ->");
            let (parent, alias) = from.rsplit_once(' ').unwrap_or(("", from));
            aliases.insert((parent.to_string(), alias.to_string()), to.to_string());
        } else if let Some(rest) = line.strip_prefix("group: ") {
            let path = command_path(rest);
            let flags = rest.split_whitespace().filter(|w| w.starts_with("--"));
            groups
                .entry(path)
                .or_default()
                .extend(flags.map(String::from));
        } else {
            let flags = line.split_whitespace().filter(|w| w.starts_with("--"));
            leaves.insert(command_path(line), flags.map(String::from).collect());
        }
    }
    // Group flags (e.g. Bitbucket's --workspace/--repo) apply to every leaf
    // below the group.
    for (group, flags) in &groups {
        let prefix = format!("{group} ");
        for (leaf, lf) in leaves.iter_mut() {
            if leaf.starts_with(&prefix) {
                lf.extend(flags.iter().cloned());
            }
        }
    }
    (leaves, aliases)
}

#[test]
fn corpus_is_valid_twg() {
    let (leaves, aliases) = twg_tree();
    let is_prefix = |p: &str| {
        let pre = format!("{p} ");
        leaves.keys().any(|l| l == p || l.starts_with(&pre))
    };
    let corpus = read("compat/corpus.txt");
    let mut problems = Vec::new();
    for (i, raw) in corpus.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (_, inv) = parse_corpus_line(line);
        let words = shell_words(inv);
        let mut path = String::new();
        let mut rest = words.as_slice();
        while let Some((w, tail)) = rest.split_first() {
            let name = aliases
                .get(&(path.clone(), w.clone()))
                .cloned()
                .unwrap_or_else(|| w.clone());
            let next = if path.is_empty() {
                name
            } else {
                format!("{path} {name}")
            };
            if !is_prefix(&next) {
                break;
            }
            path = next;
            rest = tail;
        }
        let Some(flags) = leaves.get(&path) else {
            problems.push(format!(
                "line {}: `{inv}` is not a twg leaf command (stopped at `{path}`)",
                i + 1
            ));
            continue;
        };
        for w in rest.iter().filter(|w| w.starts_with("--")) {
            let flag = w.split('=').next().unwrap_or(w);
            if !flags.contains(flag) && !TWG_GLOBAL_FLAGS.contains(&flag) {
                problems.push(format!("line {}: twg `{path}` has no flag {flag}", i + 1));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "compat/corpus.txt has invocations twg 1.4.4 would reject:\n{}",
        problems.join("\n")
    );
}

// --- command trees -------------------------------------------------------------

/// twg leaf command paths from the inventory, without positionals or flags.
fn twg_leaves() -> BTreeSet<String> {
    twg_tree().0.into_keys().collect()
}

/// Leading words of a tree line, up to the first positional, flag or marker.
fn command_path(line: &str) -> String {
    line.split_whitespace()
        .take_while(|w| !w.starts_with(['<', '[', '-', '(']))
        .collect::<Vec<_>>()
        .join(" ")
}

/// atlx command paths from the CLI tree snapshot (without the leading
/// `atlx`). Returns (canonical leaves, every accepted spelling including
/// aliases).
fn atlx_paths() -> (BTreeSet<String>, BTreeSet<String>) {
    let snapshot = read("tests/snapshots/cli-tree.txt");
    let strip = |s: &str| s.strip_prefix("atlx ").unwrap_or(s).trim().to_string();
    let mut canonical = BTreeSet::new();
    let mut aliases = Vec::new();
    for line in snapshot.lines() {
        if line.starts_with("global:") {
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("alias: ")
            .or_else(|| line.strip_prefix("hidden-alias: "))
        {
            let (from, to) = rest.split_once(" -> ").expect("alias line has ->");
            aliases.push((strip(from), strip(to)));
        } else if line.starts_with("atlx ") {
            canonical.insert(strip(&command_path(line)));
        }
    }
    // Expand aliases until nothing new appears (aliases can nest, e.g.
    // `bb prs`).
    let mut all = canonical.clone();
    loop {
        let mut added = false;
        for (from, to) in &aliases {
            let prefix = format!("{to} ");
            let new: Vec<String> = all
                .iter()
                .filter_map(|p| {
                    if p == to {
                        Some(from.clone())
                    } else {
                        p.strip_prefix(&prefix).map(|r| format!("{from} {r}"))
                    }
                })
                .collect();
            for p in new {
                added |= all.insert(p);
            }
        }
        if !added {
            break;
        }
    }
    (canonical, all)
}

fn product_of(path: &str) -> &str {
    match path.split_whitespace().next().unwrap_or("") {
        "auth" | "login" | "logout" | "whoami" | "setup" | "doctor" => "account",
        p => p,
    }
}

// --- COMPAT.md -------------------------------------------------------------------

const STATUSES: [&str; 6] = ["✅", "🟡", "🔁", "➕", "📋", "⛔"];

struct Row {
    line: usize,
    status: String,
    notes: String,
}

/// Rows of every COMPAT.md table whose first cell is one backticked command.
fn compat_rows() -> BTreeMap<String, Row> {
    let doc = read("COMPAT.md");
    let mut rows = BTreeMap::new();
    for (i, line) in doc.lines().enumerate() {
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
        if cells.len() < 3 {
            continue;
        }
        let cmd = cells[0].trim();
        let Some(cmd) = cmd.strip_prefix('`').and_then(|c| c.strip_suffix('`')) else {
            continue;
        };
        // The global flags table lists flags, not commands.
        if cmd.starts_with('-') || cmd.contains('`') {
            continue;
        }
        let status = cells[1].trim().to_string();
        let notes = cells[2..].join("|").trim().to_string();
        let prev = rows.insert(
            cmd.to_string(),
            Row {
                line: i + 1,
                status,
                notes,
            },
        );
        assert!(prev.is_none(), "COMPAT.md lists `{cmd}` twice");
    }
    rows
}

#[test]
fn compat_matrix_matches_inventory_and_cli() {
    let twg = twg_leaves();
    let (canonical, all) = atlx_paths();
    let rows = compat_rows();
    let mut problems = Vec::new();

    for cmd in &twg {
        if !rows.contains_key(cmd) {
            problems.push(format!("missing row for twg command `{cmd}`"));
        }
    }
    for (cmd, row) in &rows {
        let at = format!("COMPAT.md:{} `{cmd}`", row.line);
        let s = row.status.as_str();
        if !STATUSES.contains(&s) {
            problems.push(format!("{at}: unknown status {s:?}"));
            continue;
        }
        let in_twg = twg.contains(cmd);
        if s == "➕" {
            if in_twg {
                problems.push(format!("{at}: ➕ but twg has this command"));
            }
            if !all.contains(cmd) {
                problems.push(format!("{at}: ➕ but atlx has no such command"));
            }
            continue;
        }
        if !in_twg {
            problems.push(format!("{at}: not in {INVENTORY}"));
            continue;
        }
        let ok = match s {
            "✅" | "🟡" => canonical.contains(cmd),
            "🔁" => all.contains(cmd) && !canonical.contains(cmd),
            _ => !all.contains(cmd),
        };
        if !ok {
            let has = if canonical.contains(cmd) {
                "as a command"
            } else if all.contains(cmd) {
                "through an alias"
            } else {
                "not at all"
            };
            problems.push(format!("{at}: status {s} but atlx accepts it {has}"));
        }
        let needs_issue = matches!(s, "📋" | "🟡");
        if needs_issue && !row.notes.contains('#') {
            problems.push(format!("{at}: {s} rows must link an issue (#N)"));
        }
        if row.notes.is_empty() && matches!(s, "⛔" | "🔁") {
            problems.push(format!("{at}: {s} rows need a note"));
        }
    }
    assert!(
        problems.is_empty(),
        "COMPAT.md is out of date:\n{}",
        problems.join("\n")
    );
}

// --- coverage --------------------------------------------------------------------

fn render_coverage() -> String {
    let twg = twg_leaves();
    let (_, all) = atlx_paths();
    let mut out = format!(
        "# Share of twg leaf commands (from {INVENTORY}) that atlx accepts under\n\
         # the same path, directly or through an alias. Flags are not counted.\n\
         # Generated by tests/twg_compat.rs; refresh with UPDATE_COVERAGE=1.\n"
    );
    let mut line = |name: &str, hit: usize, total: usize| {
        let pct = if total == 0 {
            0.0
        } else {
            100.0 * hit as f64 / total as f64
        };
        out.push_str(&format!("{name:<11} {hit:>3}/{total:<4} {pct:5.1}%\n"));
    };
    let hits = |p: Option<&str>| {
        let in_scope: Vec<_> = twg
            .iter()
            .filter(|c| p.is_none_or(|p| product_of(c) == p))
            .collect();
        let hit = in_scope.iter().filter(|c| all.contains(**c)).count();
        (hit, in_scope.len())
    };
    let (h, t) = hits(None);
    line("overall", h, t);
    for p in PRODUCTS {
        let (h, t) = hits(Some(p));
        line(p, h, t);
    }
    out
}

/// (name, hit) pairs from a coverage report.
fn coverage_counts(report: &str) -> BTreeMap<String, usize> {
    report
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let name = w.next()?;
            let hit = w.next()?.split('/').next()?.parse().ok()?;
            Some((name.to_string(), hit))
        })
        .collect()
}

#[test]
fn coverage_does_not_drop() {
    let actual = render_coverage();
    println!("{actual}");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("compat/coverage.txt");
    if std::env::var_os("UPDATE_COVERAGE").is_some_and(|v| v != "0" && !v.is_empty()) {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let stored = read("compat/coverage.txt");
    let before = coverage_counts(&stored);
    let now = coverage_counts(&actual);
    let mut dropped = Vec::new();
    for (name, old) in &before {
        let new = now.get(name).copied().unwrap_or(0);
        if new < *old {
            dropped.push(format!("{name}: {old} -> {new}"));
        }
    }
    assert!(
        dropped.is_empty(),
        "twg coverage dropped ({}):\n{}\ncurrent report:\n{actual}",
        "commands that used to parse no longer do",
        dropped.join("\n")
    );
    if stored != actual {
        println!("coverage changed; run UPDATE_COVERAGE=1 cargo test --test twg_compat");
    }
}

#[test]
fn shell_words_handles_quotes() {
    assert_eq!(
        shell_words(r#"a 'b c' "d e" f''"#),
        vec!["a", "b c", "d e", "f"]
    );
    assert_eq!(shell_words("x ''"), vec!["x", ""]);
}

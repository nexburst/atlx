//! Clear errors for twg commands atlx doesn't have.
//!
//! When the command line doesn't parse, `main` asks [`explain`] whether it
//! names a twg command. Commands that need Teamwork Graph, the GraphQL
//! gateway or Rovo (⛔ in COMPAT.md, and twg's graph-only top-level groups)
//! fail with "not supported by atlx" and exit code 2; commands that are
//! planned (📋) fail with a pointer to their issue. Anything else falls
//! through to clap's own message.
//!
//! COMPAT.md is compiled in, so this list can't drift from the matrix that
//! `tests/twg_compat.rs` checks.

use crate::client::ApiError;

const COMPAT: &str = include_str!("../../COMPAT.md");

/// twg top-level groups built on Teamwork Graph, Rovo or the GraphQL
/// gateway. atlx has no counterpart for any of them.
const GRAPH_GROUPS: &[&str] = &[
    "admin",
    "artifacts",
    "assets",
    "capabilities",
    "collaborators",
    "commits",
    "consent",
    "context",
    "csm",
    "deployments",
    "docs",
    "env",
    "feedback",
    "focus-areas",
    "focus-areas-tree",
    "goals",
    "insights",
    "issue-tree",
    "loom",
    "meetings",
    "notifications",
    "org-tree",
    "ownership",
    "people",
    "pr-tree",
    "projects",
    "prs",
    "pull-requests",
    "pull-requests-tree",
    "recently-viewed",
    "resolve",
    "responsibility",
    "rovo",
    "search-code",
    "spaces",
    "status-rollup",
    "subgraph",
    "talent",
    "teams",
    "trello",
    "user-search",
    "videos",
    "work",
    "work-tree",
    "workitem-tree",
];

/// Global flags that only make sense on top of Teamwork Graph.
const GRAPH_FLAGS: &[&str] = &["--usage-counts", "--output-shape"];

/// Global flags that take a value, so the value is not a command word.
const VALUE_FLAGS: &[&str] = &[
    "-o",
    "--output",
    "-s",
    "--site",
    "-u",
    "--user",
    "--timeout-ms",
    "-p",
    "--profile",
    "--select",
    "--agent-fields",
    "--output-file",
    "--api-version",
];

/// twg's command aliases, mapped to the names COMPAT.md uses.
fn canonical(word: &str) -> &str {
    match word {
        "bb" => "bitbucket",
        "prs" => "pull-requests",
        "page" => "content",
        "comment" => "comments",
        "label" => "labels",
        "custom-fields" => "field",
        "help-centre" => "help-center",
        other => other,
    }
}

/// The leading command words of `args` (no program name), skipping flags.
fn command_words(args: &[String]) -> Vec<&str> {
    let mut words = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a.starts_with('-') {
            if VALUE_FLAGS.contains(&a.as_str()) {
                it.next();
            }
            continue;
        }
        words.push(a.as_str());
    }
    words
}

/// One COMPAT.md row: status symbol and note for a command path.
fn row(path: &str) -> Option<(&'static str, &'static str)> {
    let needle = format!("| `{path}` |");
    let line = COMPAT.lines().find(|l| l.starts_with(&needle))?;
    let mut cells = line.split('|').map(str::trim).skip(2);
    Some((cells.next()?, cells.next().unwrap_or_default()))
}

/// The error for a command line that clap rejected, when it names a twg
/// command atlx doesn't offer.
pub(crate) fn explain(args: &[String]) -> Option<ApiError> {
    if let Some(flag) = args
        .iter()
        .map(|a| a.split('=').next().unwrap_or(a))
        .find(|a| GRAPH_FLAGS.contains(a))
    {
        return Some(ApiError::unsupported(format!(
            "`{flag}` is not supported by atlx (requires Teamwork Graph)"
        )));
    }
    let words = command_words(args);
    let first = *words.first()?;
    if GRAPH_GROUPS.contains(&first) {
        return Some(ApiError::unsupported(format!(
            "`twg {first}` is not supported by atlx (requires Teamwork Graph)"
        )));
    }
    // The longest command path that has a row, e.g. `jira board cells query`.
    let names: Vec<&str> = words.iter().map(|w| canonical(w)).collect();
    for n in (1..=names.len().min(6)).rev() {
        let path = names[..n].join(" ");
        match row(&path) {
            Some(("⛔", note)) => {
                let graph = ["Graph", "gateway", "Rovo", "AI"]
                    .iter()
                    .any(|w| note.contains(w));
                let why = match (graph, note.is_empty()) {
                    (true, _) | (_, true) => format!("requires Teamwork Graph; {note}"),
                    (false, false) => note.to_string(),
                };
                let why = why.trim_end_matches("; ");
                return Some(ApiError::unsupported(format!(
                    "`twg {path}` is not supported by atlx ({why})"
                )));
            }
            Some(("📋", note)) => {
                return Some(ApiError::usage(format!(
                    "`{path}` is not in atlx yet (planned: {note})"
                )));
            }
            Some(_) => return None,
            None => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ErrorKind, exit};

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn explains_graph_and_planned_commands() {
        let e = explain(&args("-o json rovo search launch")).unwrap();
        assert_eq!(e.kind, ErrorKind::Unsupported);
        assert_eq!(e.exit_code(), exit::UNSUPPORTED);
        assert!(
            e.message.contains("requires Teamwork Graph"),
            "{}",
            e.message
        );

        let e = explain(&args("jira board cells query --board-id 1")).unwrap();
        assert_eq!(e.kind, ErrorKind::Unsupported);
        assert!(
            e.message.contains("`twg jira board cells query`"),
            "{}",
            e.message
        );
        assert!(e.message.contains("GraphQL gateway"), "{}", e.message);

        let e = explain(&args("--site acme jira dashboard query")).unwrap();
        assert_eq!(e.kind, ErrorKind::Usage);
        assert!(e.message.contains("planned"), "{}", e.message);

        let e = explain(&args("jira workitem get PROJ-1 --usage-counts")).unwrap();
        assert_eq!(e.kind, ErrorKind::Unsupported);

        // Supported commands and unknown words fall through to clap.
        assert!(explain(&args("jira workitem get --bogus")).is_none());
        assert!(explain(&args("nonsense")).is_none());
        let e = explain(&args("auth storage status")).unwrap();
        assert!(!e.message.contains("Teamwork Graph"), "{}", e.message);
    }
}

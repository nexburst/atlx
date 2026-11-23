//! The skill's command references are generated from `atlx describe`, so
//! they cannot drift from the binary: `UPDATE_SKILL=1 cargo test --test
//! skill` rewrites them, and the test fails when a file is stale. Also
//! checks that the Markdown docs have no broken links inside the repository.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn describe(args: &[&str]) -> Value {
    let o = Command::new(env!("CARGO_BIN_EXE_atlx"))
        .arg("describe")
        .args(args)
        .args(["-o", "json"])
        .env_remove("ATLX_OUTPUT")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    v["data"].clone()
}

/// The reference file each command path belongs to, with its title.
const FILES: [(&str, &str, &str); 5] = [
    (
        "core",
        "Core commands",
        "`auth`, `doctor`, `search`, `api`, `describe`, `cache`, `completion`, `skill`",
    ),
    (
        "jira",
        "Jira commands",
        "`jira`: issues, projects, boards, sprints, filters, dashboards, metadata, users",
    ),
    (
        "jsm",
        "Jira Service Management commands",
        "`jsm`: requests, approvals, SLAs, desks, request types, queues, articles",
    ),
    (
        "confluence",
        "Confluence commands",
        "`confluence`: search, pages, comments, attachments, labels, versions, spaces",
    ),
    (
        "bitbucket",
        "Bitbucket commands",
        "`bitbucket` (alias `bb`): repositories, pull requests, pipelines, deployments",
    ),
];

fn file_of(path: &str) -> &'static str {
    match path.split(' ').next().unwrap() {
        "jira" => "jira",
        "jsm" => "jsm",
        "confluence" => "confluence",
        "bitbucket" => "bitbucket",
        _ => "core",
    }
}

/// Flags every list or multi-ID get has; the file header names them once.
const COMMON: [&str; 5] = ["--limit", "--cursor", "--all", "--strict", "--concurrency"];

fn arg(a: &Value, positional: bool) -> String {
    let name = a["name"].as_str().unwrap();
    let mut s = match a["short"].as_str() {
        Some(short) => format!("{short}/{name}"),
        None => name.to_string(),
    };
    if let Some(values) = a["values"].as_array() {
        let values: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
        if positional {
            s = values.join("|");
        } else {
            s = format!("{s} {}", values.join("|"));
        }
    }
    if a["repeatable"] == true {
        s.push_str("...");
    }
    if positional {
        if a["required"] == true {
            s
        } else {
            format!("[{s}]")
        }
    } else if a["required"] == true {
        format!("{s} (required)")
    } else {
        s
    }
}

fn command(c: &Value) -> String {
    let path = c["path"].as_str().unwrap();
    let flags: Vec<&Value> = c["flags"].as_array().unwrap().iter().collect();
    let has = |n: &str| flags.iter().any(|f| f["name"] == n);
    let mut kind = vec![c["effect"].as_str().unwrap().to_string()];
    if c["confirm"] == true {
        kind.push("needs --yes".into());
    }
    if has("--cursor") {
        kind.push("paged".into());
    }
    if has("--strict") {
        kind.push("multi-ID".into());
    }
    let summary = c["summary"].as_str().unwrap().trim_end_matches('.');
    let mut out = format!("- **`{path}`** ({}). {summary}.\n", kind.join(", "));

    let positionals: Vec<String> = c["positionals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| format!("`{}`", arg(a, true)))
        .collect();
    let named: Vec<String> = flags
        .iter()
        .filter(|f| !COMMON.contains(&f["name"].as_str().unwrap()))
        .map(|f| format!("`{}`", arg(f, false)))
        .collect();
    let mut takes = Vec::new();
    if !positionals.is_empty() {
        takes.push(format!("args {}", positionals.join(" ")));
    }
    if !named.is_empty() {
        takes.push(format!("flags {}", named.join(", ")));
    }
    if !takes.is_empty() {
        out.push_str(&format!("  Takes {}.\n", takes.join("; ")));
    }
    if let Some(r) = c["record"].as_object() {
        let fields: Vec<&str> = r["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["default"] == true)
            .filter_map(|f| f["name"].as_str())
            .collect();
        out.push_str(&format!(
            "  Returns `{}`: {}.\n",
            r["name"].as_str().unwrap(),
            fields.join(", ")
        ));
    }
    for e in c["examples"].as_array().unwrap() {
        out.push_str(&format!("  - `{}`\n", e.as_str().unwrap()));
    }
    out
}

/// The text of every generated reference file, by file name.
fn generate() -> Vec<(String, String)> {
    let index = describe(&["--all"]);
    let contract = index["contract"].as_u64().unwrap();
    let commands = index["commands"].as_array().unwrap();
    FILES
        .iter()
        .map(|(name, title, covers)| {
            let mut text = format!(
                "# {title}\n\n\
                 <!-- Generated from `atlx describe` (contract {contract}) by \
                 `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->\n\n\
                 Covers: {covers}. Every command also takes the global flags \
                 (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, \
                 `--read-only`, `-p`, ...). \"paged\" commands take `--limit` \
                 (default 25, at most 1000), `--cursor` and `--all`; \"multi-ID\" \
                 commands take several IDs (or `-` for stdin) plus `--strict` and \
                 `--concurrency`. `...` marks a repeatable value. \"Returns\" lists \
                 the record's default fields; `atlx describe <path>` has every \
                 field, flag type and default.\n"
            );
            let mut group = String::new();
            for c in commands
                .iter()
                .filter(|c| file_of(c["path"].as_str().unwrap()) == *name)
            {
                let path = c["path"].as_str().unwrap();
                let words: Vec<&str> = path.split(' ').collect();
                let g = if words.len() > 2 {
                    words[..words.len() - 1].join(" ")
                } else {
                    words[0].to_string()
                };
                if g != group {
                    text.push_str(&format!("\n## {g}\n\n"));
                    group = g;
                }
                text.push_str(&command(c));
            }
            (format!("{name}.md"), text)
        })
        .collect()
}

#[test]
fn skill_references_match_describe() {
    let dir = root().join("skills/atlx/references");
    let update = std::env::var_os("UPDATE_SKILL").is_some();
    let mut stale = Vec::new();
    for (name, text) in generate() {
        let file = dir.join(&name);
        if update {
            std::fs::write(&file, &text).unwrap();
        } else if std::fs::read_to_string(&file)
            .ok()
            .map(|s| s.replace("\r\n", "\n"))
            .as_deref()
            != Some(text.as_str())
        {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "skill references are stale; run UPDATE_SKILL=1 cargo test --test skill: {stale:?}"
    );
}

#[test]
fn every_command_is_in_a_reference() {
    let mut text = String::new();
    for (name, _, _) in FILES {
        text.push_str(
            &std::fs::read_to_string(root().join(format!("skills/atlx/references/{name}.md")))
                .unwrap_or_default(),
        );
    }
    for c in describe(&[])["commands"].as_array().unwrap() {
        let path = c["path"].as_str().unwrap();
        assert!(
            text.contains(&format!("**`{path}`**")),
            "{path} is missing from the skill references"
        );
    }
}

/// Relative link targets in a Markdown file: `[text](target)`.
fn links(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Code spans are not links.
        let prose: String = line.split('`').step_by(2).collect();
        let mut rest = prose.as_str();
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            let Some(end) = rest.find(')') else { break };
            let target = &rest[..end];
            rest = &rest[end..];
            let target = target.split_whitespace().next().unwrap_or_default();
            // Anchors and URLs (`https:`, `mailto:`) are not repo files.
            if target.is_empty() || target.starts_with('#') || target.contains(':') {
                continue;
            }
            out.push(target.split('#').next().unwrap().to_string());
        }
    }
    out
}

#[test]
fn docs_have_no_broken_links() {
    let mut docs: Vec<PathBuf> = ["README.md", "CONTRIBUTING.md", "AGENTS.md", "CHANGELOG.md"]
        .iter()
        .map(|f| root().join(f))
        .collect();
    for dir in ["skills/atlx", "skills/atlx/references"] {
        for e in std::fs::read_dir(root().join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "md") {
                docs.push(p);
            }
        }
    }
    let mut broken = Vec::new();
    for doc in &docs {
        let text = std::fs::read_to_string(doc).unwrap();
        let base = doc.parent().unwrap_or(Path::new("."));
        for target in links(&text) {
            if !base.join(&target).exists() {
                broken.push(format!("{}: {target}", doc.display()));
            }
        }
    }
    assert!(broken.is_empty(), "broken links: {broken:?}");
}

//! `atlx describe` is the contract: its output per product is snapshotted
//! (`UPDATE_SNAPSHOTS=1 cargo test --test describe` regenerates), and the
//! list of commands without a registry entry may only shrink.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn atlx(args: &[&str]) -> Value {
    let o = Command::new(env!("CARGO_BIN_EXE_atlx"))
        .args(args)
        .args(["-o", "json"])
        .env_remove("ATLX_OUTPUT")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true);
    v["data"].clone()
}

/// The snapshot each command belongs to.
fn product(path: &str) -> &'static str {
    match path.split(' ').next().unwrap() {
        "jira" | "user" => "jira",
        "jsm" => "jsm",
        "confluence" => "confluence",
        "bitbucket" => "bitbucket",
        _ => "core",
    }
}

const PRODUCTS: [&str; 5] = ["core", "jira", "jsm", "confluence", "bitbucket"];

fn snapshots() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/describe")
}

fn all_commands() -> Vec<Value> {
    atlx(&["describe", "--all"])["commands"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn describe_snapshots_match() {
    let commands = all_commands();
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut stale = Vec::new();
    for p in PRODUCTS {
        let mine: Vec<&Value> = commands
            .iter()
            .filter(|c| product(c["path"].as_str().unwrap()) == p)
            .collect();
        let text = serde_json::to_string_pretty(&mine).unwrap() + "\n";
        let file = snapshots().join(format!("{p}.json"));
        if update {
            std::fs::create_dir_all(snapshots()).unwrap();
            std::fs::write(&file, &text).unwrap();
        } else if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            stale.push(file.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "describe output changed; check the diff and run UPDATE_SNAPSHOTS=1 cargo test --test describe: {stale:?}"
    );
}

#[test]
fn unregistered_commands_only_shrink() {
    let unregistered: BTreeSet<String> = all_commands()
        .iter()
        .filter(|c| c["registered"] == false)
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect();
    let mut problems = Vec::new();
    for p in PRODUCTS {
        let file = snapshots().join(format!("{p}.unregistered.txt"));
        let allowed: BTreeSet<String> = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        for c in unregistered.iter().filter(|c| product(c) == p) {
            if !allowed.contains(c) {
                problems.push(format!(
                    "`{c}` has no CommandMeta (add one to its META table)"
                ));
            }
        }
        for a in &allowed {
            if !unregistered.contains(a) {
                problems.push(format!(
                    "`{a}` is registered or gone; remove it from {}",
                    file.display()
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn index_path_records_and_errors_work() {
    let idx = atlx(&["describe"]);
    assert_eq!(idx["contract"], 1);
    let cmds = idx["commands"].as_array().unwrap();
    assert!(
        cmds.iter()
            .any(|c| c["path"] == "auth status" && c["effect"] == "read")
    );
    assert!(
        idx["globals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["name"] == "--read-only")
    );

    let jira = atlx(&["describe", "jira"]);
    assert!(jira.get("globals").is_none());
    assert!(
        jira["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["path"].as_str().unwrap().starts_with("jira "))
    );

    let one = atlx(&["describe", "auth", "status"]);
    assert_eq!(one["path"], "auth status");
    assert_eq!(one["record"]["name"], "auth.status");
    assert!(one["examples"].as_array().unwrap().len() >= 2);
    assert!(one.get("registered").is_none());

    let migrate = atlx(&["describe", "auth", "profile", "migrate"]);
    assert_eq!(migrate["effect"], "destructive");
    assert_eq!(migrate["confirm"], true);
    assert!(migrate["exitCodes"].as_array().unwrap().contains(&8.into()));

    let records = atlx(&["describe", "--records"]);
    assert!(
        records["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "auth.profile")
    );
    let errors = atlx(&["describe", "--errors"]);
    assert_eq!(errors["exitCodes"].as_array().unwrap().len(), 9);
}

#[test]
fn cache_path_reports_no_cache() {
    let dir = std::env::temp_dir().join(format!("atlx-describe-cache-{}", std::process::id()));
    let run = |extra: &[&str]| -> Value {
        let o = Command::new(env!("CARGO_BIN_EXE_atlx"))
            .args(["cache", "path", "-o", "json"])
            .args(extra)
            .env("XDG_CACHE_HOME", &dir)
            .env_remove("ATLX_NO_CACHE")
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["data"].clone()
    };
    let on = run(&[]);
    assert_eq!(on["enabled"], true);
    assert_eq!(on["path"], dir.join("atlx").display().to_string());
    assert_eq!(run(&["--no-cache"])["enabled"], false);
}

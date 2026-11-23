//! The output contract: every command prints one envelope whose `ok` is
//! true exactly when it exits 0, checked against mock responses.

mod common;

use common::{MockResponse, MockServer, RecordedRequest};
use serde_json::{Value, json};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn command(site: &str, args: &[String]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    let home = std::env::temp_dir().join(format!("atlx-contract-{}", std::process::id()));
    cmd.args(args)
        .env("ATLX_CONFIG", home.join("config.toml"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("ATLX_SITE", site)
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .env("ATLX_BB_WORKSPACE", "acme")
        .env("ATLX_BITBUCKET_API", site)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn run_with(body: Value, args: &str) -> (Option<i32>, String, Vec<RecordedRequest>) {
    let srv = MockServer::with_handler(move |_| MockResponse::json(200, body.clone()));
    let argv: Vec<String> = args.split_whitespace().map(String::from).collect();
    let out = command(&srv.base_url(), &argv).output().unwrap();
    (
        out.status.code(),
        String::from_utf8(out.stdout).unwrap(),
        srv.requests(),
    )
}

fn json_run(body: Value, args: &str) -> (Option<i32>, Value) {
    let (code, out, _) = run_with(body, &format!("{args} -o json"));
    (
        code,
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}")),
    )
}

#[test]
fn jsonl_prints_each_record_then_a_trailer() {
    let page = json!({"isLast": true, "values": [{"id": "7", "name": "F"}]});
    let (_, out, _) = run_with(page, "jira filter list -o jsonl");
    let lines: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert_eq!(lines[0]["id"], "7");
    assert_eq!(lines[1]["_end"]["next"], Value::Null);
    assert_eq!(lines[1]["_end"]["errors"], json!([]));
}

#[test]
fn lists_print_records_and_next() {
    let page = json!({"startAt": 0, "maxResults": 50, "total": 1, "isLast": true, "values": [{"id": "7"}]});
    let (_, v) = json_run(page.clone(), "jira filter list");
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"][0]["id"], "7");
    assert_eq!(v["next"], Value::Null);
    let (_, v) = json_run(page, "jira project version list --project P");
    assert_eq!(v["data"][0]["id"], "7");
}

#[test]
fn project_list_defaults_to_25() {
    let (_, _, reqs) = run_with(
        json!({"values": [], "total": 0, "isLast": true}),
        "jira project list -o json",
    );
    assert_eq!(reqs[0].query_param("maxResults").as_deref(), Some("25"));
}

#[test]
fn project_statuses_are_listed_once() {
    let s = json!({"id": "1", "name": "To Do", "statusCategory": {"key": "new", "name": "To Do"}});
    let d = json!({"id": "2", "name": "Done", "statusCategory": {"key": "done", "name": "Done"}});
    let kinds = json!([{"name": "Task", "statuses": [d, s]}, {"name": "Bug", "statuses": [s]}]);
    let (_, v) = json_run(kinds, "jira status list --project P");
    assert_eq!(
        v["data"],
        json!([{"id": "2", "name": "Done", "category": "done"}, {"id": "1", "name": "To Do", "category": "new"}])
    );
}

#[test]
fn bitbucket_output_is_enveloped_too() {
    let repo = json!({"full_name": "team/api", "name": "api", "slug": "api", "is_private": true,
        "mainbranch": {"name": "main"}, "links": {"html": {"href": "https://bitbucket.org/team/api"}}});
    let (code, out, reqs) = run_with(repo, "bitbucket repo get team/api -o json");
    let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["ok"], true);
    assert!(v["data"].is_object(), "{v}");
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert!(
        reqs[0].path.contains("/repositories/team/api"),
        "{}",
        reqs[0].path
    );

    let (code, v) = json_run(json!({}), "bitbucket pr merge 1 --repo team/api");
    assert_eq!(code, Some(8));
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "refused");
}

#[test]
fn bitbucket_api_override_must_be_loopback() {
    let mut cmd = command(
        "http://127.0.0.1:9",
        &[
            "bitbucket".into(),
            "repo".into(),
            "get".into(),
            "team/api".into(),
            "-o".into(),
            "json".into(),
        ],
    );
    cmd.env("ATLX_BITBUCKET_API", "https://evil.example/2.0");
    let out = cmd.output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(4), "{v}");
    assert_eq!(v["error"]["code"], "config", "{v}");
}

#[test]
fn board_get_is_a_list() {
    let (_, v) = json_run(json!({"id": 3, "name": "B"}), "jira board get 3 3");
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
}

#[test]
fn property_and_worklog_feeds_are_bare_lists() {
    let (_, v) = json_run(json!({"keys": []}), "jira issue property list --issue P-1");
    assert_eq!(v["data"], json!([]));
    let page = json!({"values": [], "since": 1, "until": 2, "lastPage": true});
    let (_, v) = json_run(page, "jira issue worklog list --changed-since 1");
    assert_eq!(v["data"], json!([]));
    assert_eq!(v["next"], Value::Null);
}

#[test]
fn many_issue_get_partial_exits_3_with_ok_false() {
    // 21 or more issues are read with bulkfetch; Jira returns only P-1.
    let got = json!({"issues": [{"id": "1", "key": "P-1", "fields": {"summary": "S"}}]});
    let keys: Vec<String> = (1..=21).map(|i| format!("P-{i}")).collect();
    let (code, v) = json_run(got, &format!("jira issue get {}", keys.join(" ")));
    assert_eq!(code, Some(3));
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"][0]["key"], "P-1");
    assert_eq!(v["errors"].as_array().unwrap().len(), 20);

    let issue = json!({"id": "1", "key": "P-1", "fields": {"summary": "S"}});
    let (code, v) = json_run(issue, "jira issue get P-1");
    assert_eq!(code, Some(0));
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["key"], "P-1");
}

#[test]
fn user_get_takes_several_users() {
    let found = json!({"accountId": "a", "displayName": "Ann", "active": true});
    let (code, v) = json_run(found.clone(), "jira user get a b");
    assert_eq!(code, Some(0));
    assert_eq!(
        v["data"][1],
        json!({"id": "a", "name": "Ann", "email": null, "active": true})
    );
    let (code, v) = json_run(found, "jira user get a");
    assert_eq!(code, Some(0));
    assert_eq!(v["data"]["id"], "a");
}

/// Commands whose output is not JSON by design (shell scripts).
const RAW_OUTPUT: &[&str] = &["completion script", "skill print"];

/// Commands that wait, poll or need a terminal; not run by the walk.
const SKIP: &[&str] = &["skill install"];

/// A value for an argument the walk has to fill in.
fn sample(arg: &Value) -> String {
    if let Some(v) = arg["values"].as_array().and_then(|v| v.first()) {
        return v.as_str().unwrap_or("x").to_string();
    }
    match arg["type"].as_str() {
        Some("integer") | Some("number") => "1".into(),
        _ => "1".into(),
    }
}

/// Every command, with its required arguments filled in, against a mock
/// that answers `{}` to everything: stdout is exactly one envelope and
/// `ok` is true exactly when the exit code is 0.
#[test]
fn every_command_prints_one_envelope_and_ok_matches_the_exit_code() {
    let srv = MockServer::with_handler(|_| MockResponse::json(200, json!({})));
    let site = srv.base_url();
    let out = command(
        &site,
        &[
            "describe".into(),
            "--all".into(),
            "-o".into(),
            "json".into(),
        ],
    )
    .output()
    .unwrap();
    let all: Value = serde_json::from_slice(&out.stdout).unwrap();
    let commands = all["data"]["commands"].as_array().unwrap().clone();
    assert!(commands.len() > 250, "{}", commands.len());
    let jobs: Vec<Vec<String>> = commands
        .iter()
        .filter(|c| {
            let p = c["path"].as_str().unwrap();
            !RAW_OUTPUT.iter().chain(SKIP).any(|r| p == *r)
        })
        .map(|c| {
            let mut argv: Vec<String> = c["path"]
                .as_str()
                .unwrap()
                .split(' ')
                .map(String::from)
                .collect();
            for p in c["positionals"].as_array().unwrap() {
                if p["required"] == true {
                    argv.push(sample(p));
                }
            }
            for f in c["flags"].as_array().unwrap() {
                if f["required"] == true {
                    argv.push(f["name"].as_str().unwrap().to_string());
                    if f["type"] != "boolean" {
                        argv.push(sample(f));
                    }
                }
            }
            argv.extend(["-o".into(), "json".into()]);
            argv
        })
        .collect();
    let failures = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(argv) = jobs.get(i) else { break };
                    if let Err(why) = check(&site, argv) {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("atlx {}: {why}", argv.join(" ")));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn check(site: &str, argv: &[String]) -> Result<(), String> {
    let mut child = command(site, argv).spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let o: Output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&o.stdout);
    let code = o.status.code();
    if stdout.lines().count() != 1 {
        return Err(format!("exit {code:?}, not one line: {stdout}"));
    }
    let v: Value = serde_json::from_str(&stdout).map_err(|e| format!("{e}: {stdout}"))?;
    let keys = [
        "ok",
        "partial",
        "dryRun",
        "data",
        "next",
        "truncated",
        "warnings",
        "errors",
        "error",
    ];
    let obj = v.as_object().ok_or("not an object")?;
    if let Some(k) = obj.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("unexpected key {k}: {stdout}"));
    }
    if v["ok"] != json!(code == Some(0)) {
        return Err(format!("ok={} but exit {code:?}", v["ok"]));
    }
    if v["ok"] == false && v.get("error").is_none() && v.get("partial").is_none() {
        // A failure that still prints data (doctor, user lookup) is fine.
        if v.get("data").is_none() {
            return Err(format!("failure without error: {stdout}"));
        }
    }
    if !o.stderr.is_empty() {
        return Err(format!(
            "stderr in JSON mode: {}",
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    Ok(())
}

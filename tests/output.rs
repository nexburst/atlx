//! Global output controls on the built binary: `--select`, `--output-file`,
//! `--output-summary`, `--agent-fields` and `--api-version`.

mod common;

use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn server() -> MockServer {
    MockServer::with_handler(|req| {
        if req.path.contains("/NOPE-1") {
            MockResponse::json(404, json!({"errorMessages": ["Issue does not exist"]}))
        } else if req.path.contains("/api/v2/pages/") {
            MockResponse::json(
                200,
                json!({
                    "id": "123", "title": "Runbook", "status": "current", "spaceId": "9",
                    "version": {"number": 7, "createdAt": "2026-01-01T00:00:00Z"},
                    "body": {"storage": {"value": "<p>Hello</p>"}},
                    "_links": {"base": "https://x.atlassian.net/wiki", "webui": "/spaces/X/pages/123"}
                }),
            )
        } else if req.path.contains("/api/v2/blogposts/") {
            MockResponse::json(404, json!({"message": "not found"}))
        } else {
            MockResponse::json(
                200,
                json!({"key": "OK-1", "fields": {
                    "summary": "Fine", "status": {"name": "Done"},
                    "issuetype": {"name": "Task"}, "updated": "2026-01-01"
                }}),
            )
        }
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlx-output-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn atlx(srv: &MockServer, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    cmd.args(args)
        .env("ATLX_CONFIG", scratch("cfg").join("missing.toml"))
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "t")
        .env("ATLX_SITE", srv.base_url())
        .env_remove("ATLX_OUTPUT");
    cmd.output().expect("run atlx")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn select_projects_json_output() {
    let srv = server();
    let o = atlx(
        &srv,
        &[
            "jira",
            "workitem",
            "get",
            "OK-1",
            "-o",
            "json",
            "--select",
            "data.key,status,nope",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v, json!({"key": "OK-1", "status": "Done"}));

    // A preset.
    let o = atlx(
        &srv,
        &[
            "jira", "workitem", "get", "OK-1", "-o", "json", "--select", "@compact",
        ],
    );
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["key"], "OK-1");
    assert!(v.get("type").is_none(), "{v}");

    // Nothing matched: full payload and a warning.
    let o = atlx(
        &srv,
        &[
            "jira", "workitem", "get", "OK-1", "-o", "json", "--select", "nope",
        ],
    );
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["type"], "Task");
    assert!(stderr(&o).contains("--select matched nothing"));

    // Text mode ignores it, with a warning.
    let o = atlx(
        &srv,
        &[
            "jira", "workitem", "get", "OK-1", "-o", "text", "--select", "key",
        ],
    );
    assert!(stdout(&o).contains("Task"));
    assert!(stderr(&o).contains("--select only applies"));
}

#[test]
fn unknown_preset_is_a_usage_error() {
    let srv = server();
    let o = atlx(
        &srv,
        &[
            "jira", "workitem", "get", "OK-1", "-o", "json", "--select", "@bogus",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("@compact, @evidence, @rows"), "{msg}");
    assert!(srv.requests().is_empty(), "no request before validation");
}

#[test]
fn output_file_redirects_stdout() {
    let srv = server();
    let dir = scratch("file");
    let file = dir.join("nested/out.json");
    let f = file.to_str().unwrap();
    let o = atlx(
        &srv,
        &[
            "jira",
            "workitem",
            "get",
            "OK-1",
            "-o",
            "json",
            "--output-file",
            f,
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), format!("stdout={f}\n"));
    let v = read_json(&file);
    assert_eq!(v["key"], "OK-1");
    assert_eq!(v["type"], "Task");

    // Errors go to the file too, and keep their exit code.
    let o = atlx(
        &srv,
        &[
            "jira",
            "workitem",
            "get",
            "NOPE-1",
            "-o",
            "json",
            "--output-file",
            f,
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o), format!("stdout={f}\n"));
    assert_eq!(read_json(&file)["ok"], false);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn body_only_output_file_prints_metadata() {
    let srv = server();
    let dir = scratch("body");
    let file = dir.join("page.html");
    let f = file.to_str().unwrap();
    let args = [
        "confluence",
        "content",
        "get",
        "123",
        "--body-only",
        "--output-file",
        f,
    ];
    let o = atlx(&srv, &[&args[..], &["-o", "json"]].concat());
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "<p>Hello</p>");
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["snapshot_token"], "7");
    assert_eq!(v["body"], json!({"outputFile": f}));

    let o = atlx(&srv, &[&args[..], &["-o", "text"]].concat());
    assert_eq!(stdout(&o), format!("stdout={f}\n"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn output_summary_writes_payload_and_prints_envelope() {
    let srv = server();
    let dir = scratch("summary");
    let file = dir.join("out.json");
    let f = file.to_str().unwrap();
    let base = [
        "jira",
        "workitem",
        "get",
        "OK-1",
        "-o",
        "json",
        "--output-file",
        f,
    ];

    let o = atlx(&srv, &[&base[..], &["--output-summary"]].concat());
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    let full = std::fs::read_to_string(&file).unwrap();
    assert!(
        out.starts_with(&format!(
            "output_files:\n  stdout: \"{}\"\n  stdout_lines: {}\n  stdout_bytes: {}\ncommand: \"jira.workitem.get\"\nstdout_inline:\n  key: \"OK-1\"\n",
            f.replace('\\', "\\\\"),
            full.split('\n').count(),
            full.len()
        )),
        "{out}"
    );
    assert!(out.ends_with("---END---\n"));

    let o = atlx(&srv, &[&base[..], &["--output-summary", "stats"]].concat());
    let out = stdout(&o);
    assert!(
        out.contains("agent_output:\n  summary: \"stats\"\n"),
        "{out}"
    );
    assert!(
        out.contains("stdout_shape:\n") && out.contains("\n  key: [S, 4, 1, [\"OK-1\"]]\n"),
        "{out}"
    );
    assert!(!out.contains("stdout_inline"));

    let o = atlx(
        &srv,
        &[
            &base[..],
            &[
                "--output-summary",
                "--agent-fields",
                "data.key,data.summary",
            ],
        ]
        .concat(),
    );
    let out = stdout(&o);
    assert!(out.contains("field_source: \"explicit\"\n"), "{out}");
    assert!(
        out.contains("stdout_inline:\n  key: \"OK-1\"\n  summary: \"Fine\"\n---END---\n"),
        "{out}"
    );
    let compact = read_json(&dir.join("out.compact.json"));
    assert_eq!(compact, json!({"key": "OK-1", "summary": "Fine"}));

    // `none` prints the payload as usual (here: into the file).
    let o = atlx(&srv, &[&base[..], &["--output-summary", "none"]].concat());
    assert_eq!(stdout(&o), format!("stdout={f}\n"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn api_version_accepts_only_v2() {
    let srv = server();
    let o = atlx(
        &srv,
        &[
            "jira",
            "workitem",
            "get",
            "OK-1",
            "-o",
            "json",
            "--api-version",
            "v2",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = atlx(
        &srv,
        &["jira", "workitem", "get", "OK-1", "--api-version", "v3"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("v3"));
}

//! Global output controls on the built binary: `-o json|jsonl|ids|text`,
//! `--fields`, `--full`, `--raw`, `--max-chars`, `--output-file`.

mod common;

use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn server() -> MockServer {
    MockServer::with_handler(|req| {
        if req.path.ends_with("/rest/api/3/myself") {
            MockResponse::json(
                200,
                json!({"accountId": "acc-1", "displayName": "Ann", "emailAddress": "ann@example.com",
                       "avatarUrls": {"48x48": "https://x/a.png"}}),
            )
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
        } else if req.path.contains("/api/v2/spaces") {
            MockResponse::json(200, json!({"results": []}))
        } else {
            MockResponse::json(404, json!({"message": "unexpected"}))
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
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.args(args)
        .env("ATLX_CONFIG", scratch("cfg").join("missing.toml"))
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "t")
        .env("ATLX_SITE", srv.base_url())
        .current_dir(std::env::temp_dir());
    cmd.output().expect("run atlx")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json_out(o: &Output) -> Value {
    serde_json::from_str(&stdout(o)).unwrap_or_else(|e| panic!("{e}: {}", stdout(o)))
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn registered_records_print_the_default_view_and_fields_pick() {
    let srv = server();
    let o = atlx(&srv, &["auth", "status", "-o", "json"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(stderr(&o), "");
    assert_eq!(stdout(&o).lines().count(), 1, "one compact line");
    let v = json_out(&o);
    assert_eq!(v["ok"], true);
    let d = &v["data"];
    assert_eq!(d["user"], json!({"id": "acc-1", "name": "Ann"}));
    assert_eq!(d["products"]["jira"], "ok");
    assert!(d.get("defaults").is_none(), "only in --full: {d}");

    let v = json_out(&atlx(&srv, &["auth", "status", "-o", "json", "--full"]));
    assert!(v["data"].get("defaults").is_some());

    let v = json_out(&atlx(
        &srv,
        &[
            "auth",
            "status",
            "-o",
            "json",
            "--fields",
            "user.name,readOnly",
        ],
    ));
    assert_eq!(
        v["data"],
        json!({"user": {"name": "Ann"}, "readOnly": false})
    );
}

#[test]
fn unknown_fields_exit_2_before_any_request() {
    let srv = server();
    let o = atlx(
        &srv,
        &["auth", "status", "-o", "json", "--fields", "user,nope"],
    );
    assert_eq!(o.status.code(), Some(2));
    let v = json_out(&o);
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"]["message"].as_str().unwrap().contains("nope"));
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("profile, site, email, user")
    );
    assert!(srv.requests().is_empty(), "sent {:?}", srv.requests());
}

#[test]
fn fields_on_unregistered_commands_select_paths() {
    let srv = server();
    let o = atlx(
        &srv,
        &[
            "confluence",
            "content",
            "get",
            "123",
            "-o",
            "json",
            "--fields",
            "id,title",
        ],
    );
    assert!(o.status.success(), "{}", stdout(&o));
    assert_eq!(
        json_out(&o)["data"],
        json!({"id": "123", "title": "Runbook"})
    );
    let o = atlx(
        &srv,
        &[
            "confluence",
            "content",
            "get",
            "123",
            "-o",
            "json",
            "--fields",
            "nope",
        ],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(
        json_out(&o)["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("title")
    );
}

#[test]
fn jsonl_prints_records_then_a_trailer_and_ids_print_ids() {
    let srv = server();
    let dir = scratch("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(
        &cfg,
        "default_profile = \"a\"\n[profiles.a]\nsite = \"a\"\n[profiles.b]\nsite = \"b\"\n",
    )
    .unwrap();
    let run = |fmt: &str| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
        cmd.args(["auth", "profile", "list", "-o", fmt])
            .env("ATLX_CONFIG", &cfg)
            .env_remove("ATLX_PROFILE")
            .env_remove("ATLX_OUTPUT");
        cmd.output().unwrap()
    };
    let o = run("jsonl");
    let lines: Vec<Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{}", stdout(&o));
    assert_eq!(lines[0]["name"], "a");
    assert_eq!(lines[1]["name"], "b");
    assert_eq!(
        lines[2],
        json!({"_end": {"next": null, "truncated": [], "errors": []}})
    );
    assert_eq!(stdout(&run("ids")), "a\nb\n");
    let text = stdout(&run("text"));
    assert!(text.starts_with("NAME"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
    drop(srv);
}

#[test]
fn output_file_writes_data_and_prints_what_it_wrote() {
    let srv = server();
    let dir = scratch("file");
    let file = dir.join("nested/out.json");
    let f = file.to_str().unwrap();
    let o = atlx(&srv, &["auth", "status", "-o", "json", "--output-file", f]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v = json_out(&o);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["file"], f);
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(v["data"]["bytes"], written.len());
    assert_eq!(v["data"]["lines"], written.lines().count());
    assert_eq!(read_json(&file)["user"]["name"], "Ann");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn body_only_output_file_writes_the_body() {
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
        "-o",
        "json",
    ];
    let o = atlx(&srv, &args);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "<p>Hello</p>");
    let v = json_out(&o);
    assert_eq!(v["data"], json!({"file": f, "bytes": 12, "lines": 1}));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn removed_output_flags_are_usage_errors() {
    let srv = server();
    for flag in [
        "--select",
        "--agent-fields",
        "--output-summary",
        "--api-version",
    ] {
        let o = atlx(&srv, &["auth", "status", "-o", "json", flag, "x"]);
        assert_eq!(o.status.code(), Some(2), "{flag}");
        assert_eq!(json_out(&o)["error"]["code"], "usage", "{flag}");
    }
    assert!(srv.requests().is_empty());
}

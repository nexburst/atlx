//! Error model: typed `ApiError`s from the client, and the built binary's
//! error envelope, stderr report and exit codes (which follow twg).

mod common;

use atlx::client::{ApiError, ErrorKind};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::process::{Command, Output};

const TOKEN: &str = "ATATT3xFfGF0-very-secret-token-value";

fn jira_handler() -> MockServer {
    MockServer::with_handler(|req| {
        if req.path.ends_with("/NOPE-1") || req.path.ends_with("/NOPE-2") {
            MockResponse::json(
                404,
                json!({"errorMessages": ["Issue does not exist or you do not have permission to see it."]}),
            )
            .with_header("X-ARequestId", "req-123")
        } else if req.path.ends_with("/DENIED-1") {
            MockResponse::json(401, json!({"message": "Client must be authenticated"}))
        } else {
            MockResponse::json(200, json!({"key": "OK-1", "fields": {"summary": "Fine"}}))
        }
    })
}

/// Run the binary with an isolated config pointing at `site`.
fn atlx(site: Option<&str>, token: Option<&str>, args: &[&str]) -> Output {
    let dir = std::env::temp_dir().join(format!("atlx-errors-{}", std::process::id()));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    cmd.args(args)
        .env("ATLX_CONFIG", dir.join("missing-config.toml"))
        .env("ATLX_EMAIL", "me@example.com");
    for k in [
        "ATLX_SITE",
        "ATLX_TOKEN",
        "ATLX_OUTPUT",
        "ATLX_BB_WORKSPACE",
        "ATLX_BB_EMAIL",
        "ATLX_BB_TOKEN",
    ] {
        cmd.env_remove(k);
    }
    if let Some(s) = site {
        cmd.env("ATLX_SITE", s);
    }
    if let Some(t) = token {
        cmd.env("ATLX_TOKEN", t);
    }
    cmd.output().expect("run atlx")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn assert_no_secret(o: &Output) {
    for s in [stdout(o), stderr(o)] {
        assert!(!s.contains(TOKEN), "token leaked: {s}");
        assert!(!s.contains("Basic "), "auth header leaked: {s}");
    }
}

#[test]
fn client_errors_are_typed() {
    let srv = jira_handler();
    let err = srv
        .client()
        .get("/rest/api/3/issue/NOPE-1", &[])
        .unwrap_err();
    let api = ApiError::find(&err).expect("ApiError in chain");
    assert_eq!(api.kind, ErrorKind::NotFound);
    assert_eq!(api.status, Some(404));
    assert_eq!(api.method.as_deref(), Some("GET"));
    assert_eq!(api.request_id.as_deref(), Some("req-123"));
    assert_eq!(
        api.message,
        "Issue does not exist or you do not have permission to see it."
    );
    assert!(err.to_string().contains("HTTP 404"), "{err}");

    let err = srv
        .client()
        .get("/rest/api/3/issue/DENIED-1", &[])
        .unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Auth);
    assert!(api.hint.is_some());
}

#[test]
fn bytes_expose_status() {
    let srv = MockServer::with_responses([MockResponse::text(206, "abc")]);
    let b = srv
        .client()
        .get_bytes_with("/f", &[], &[("Range", "bytes=0-2")])
        .unwrap();
    assert_eq!(b.status, 206);

    let srv = MockServer::with_responses([MockResponse::text(416, "bad range")]);
    let err = srv.client().get_bytes("/f", &[]).unwrap_err();
    assert_eq!(ApiError::find(&err).unwrap().status, Some(416));
}

#[test]
fn not_found_prints_envelope_on_stdout_and_exits_1() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "workitem", "get", "NOPE-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).expect("JSON envelope on stdout");
    assert_eq!(v["ok"], false);
    let e = &v["error"];
    assert_eq!(e["code"], "TWG_TOOL_EXECUTION_FAILED");
    assert_eq!(e["kind"], "twg_tool_execution_failed");
    assert_eq!(e["category"], "not_found");
    assert_eq!(e["statusCode"], 404);
    assert_eq!(e["requestId"], "req-123");
    assert_eq!(e["downstreamError"]["statusCode"], 404);
    assert_eq!(e["method"], "GET");
    assert_eq!(
        e["message"],
        "Issue does not exist or you do not have permission to see it."
    );
    assert_eq!(
        e["retry"],
        json!({"recommended": false, "guidance": "do_not_retry_unchanged"})
    );
    let err = stderr(&o);
    assert!(err.starts_with("Error: Issue does not exist"), "{err}");
    assert!(err.contains("HTTP status: 404"), "{err}");
    assert_no_secret(&o);

    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "workitem", "get", "NOPE-1", "-o", "jsonl"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o).lines().count(), 1);
    let v: Value = serde_json::from_str(stdout(&o).trim()).unwrap();
    assert_eq!(v["error"]["statusCode"], 404);
}

#[test]
fn unauthorized_is_category_auth() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "workitem", "get", "DENIED-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["category"], "auth");
    assert_eq!(v["error"]["statusCode"], 401);
    assert!(v["error"]["hint"].as_str().unwrap().contains("atlx whoami"));
    assert_no_secret(&o);
}

#[test]
fn missing_token_is_category_config() {
    let o = atlx(
        Some("acme"),
        None,
        &["jira", "workitem", "get", "A-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["category"], "config");
    assert_eq!(v["error"]["code"], "TWG_COMMAND_FAILED");
    assert_eq!(v["error"]["statusCode"], 401);
    assert!(v["error"].get("downstreamError").is_none());
    assert!(stderr(&o).contains("no API token configured"));
}

#[test]
fn text_mode_prints_readable_report_only_on_stderr() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "workitem", "get", "NOPE-1", "-o", "text"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o), "");
    let err = stderr(&o);
    assert!(err.starts_with("✗ Failed jira.workitem.get.\n"), "{err}");
    assert!(
        err.contains("Error: Issue does not exist or you do not have permission to see it."),
        "{err}"
    );
    assert!(
        err.contains("Code:        TWG_TOOL_EXECUTION_FAILED"),
        "{err}"
    );
    assert!(err.contains("Request ID:  req-123"), "{err}");
    assert_no_secret(&o);
}

#[test]
fn batch_partial_failure_exits_3_and_strict_exits_1() {
    let srv = jira_handler();
    let site = srv.base_url();
    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &["jira", "workitem", "get", "OK-1", "NOPE-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["items"][0]["ok"], true);
    assert_eq!(v["items"][1]["ok"], false);
    assert_eq!(v["items"][1]["error"]["status"], 404);
    assert_eq!(v["items"][1]["error"]["category"], "not_found");
    assert_eq!(v["summary"]["failed"], 1);

    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &[
            "jira", "workitem", "get", "OK-1", "NOPE-1", "--strict", "-o", "json",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        v["error"]["message"],
        "batch read failed for 1 of 2 input(s)"
    );
    assert_eq!(v["error"]["statusCode"], 404);

    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &["jira", "workitem", "get", "NOPE-1", "NOPE-2", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["category"], "not_found");
}

#[test]
fn old_names_work_without_notes_when_piped() {
    let srv = jira_handler();
    for fmt in ["text", "json"] {
        let o = atlx(
            Some(&srv.base_url()),
            Some(TOKEN),
            &["jira", "workitem", "link", "types", "-o", fmt],
        );
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        // stdout is a pipe here, so no deprecation note may appear.
        assert!(!stderr(&o).contains("0.3"), "{}", stderr(&o));
    }
}

#[test]
fn usage_errors_exit_1() {
    let o = atlx(
        Some("acme"),
        Some(TOKEN),
        &[
            "bb",
            "pull-requests",
            "merge",
            "--repo",
            "acme/api",
            "-o",
            "json",
        ],
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["category"], "usage");
    assert_eq!(v["error"]["statusCode"], 400);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--pull-request")
    );
    assert_no_secret(&o);

    // clap argument errors: stderr only, exit 1 like twg.
    let o = atlx(None, None, &["jira", "workitem", "get", "--bogus"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o), "");
    assert!(stderr(&o).contains("--bogus"));

    let o = atlx(None, None, &["--version"]);
    assert_eq!(o.status.code(), Some(0));
}

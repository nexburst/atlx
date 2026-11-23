//! Error model: typed `ApiError`s from the client, and the built binary's
//! error envelope, stderr report and exit codes.

mod common;

use atlx::client::{ApiError, ErrorKind};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::process::{Command, Output};

const TOKEN: &str = "ATATT3xFfGF0-very-secret-token-value";

/// For commands that must fail before sending anything: if one does send,
/// it goes to a closed local port and never to the internet.
const NO_SITE: &str = "http://127.0.0.1:9";

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
        } else if req.path.ends_with("/FORBID-1") {
            MockResponse::json(403, json!({"errorMessages": ["No permission"]}))
        } else if req.path.ends_with("/CONFLICT-1") {
            MockResponse::json(409, json!({"errorMessages": ["Conflict"]}))
        } else if req.path.ends_with("/BUSY-1") {
            MockResponse::json(429, json!({"message": "slow down"})).with_header("Retry-After", "0")
        } else if req.path.ends_with("/DOWN-1") {
            MockResponse::json(503, json!({"message": "down"})).with_header("Retry-After", "0")
        } else if req.path.ends_with("/BROKEN-1") {
            MockResponse::json(500, json!({"message": "boom"}))
        } else if req.path.ends_with("/BAD-1") {
            MockResponse::json(
                400,
                json!({"errorMessages": [], "errors": {"summary": "is required"}}),
            )
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

/// `(path suffix, expected code, exit code, retryable)` for each HTTP class.
const HTTP_CASES: &[(&str, &str, i32, bool)] = &[
    ("NOPE-1", "not_found", 5, false),
    ("DENIED-1", "auth", 4, false),
    ("FORBID-1", "forbidden", 4, false),
    ("CONFLICT-1", "conflict", 6, false),
    ("BUSY-1", "rate_limited", 7, true),
    ("DOWN-1", "server", 1, true),
    ("BROKEN-1", "server", 1, false),
    ("BAD-1", "validation", 1, false),
];

#[test]
fn every_http_class_has_its_code_exit_code_hint_and_retry_flag() {
    let srv = jira_handler();
    for &(id, code, exit, retryable) in HTTP_CASES {
        let o = atlx(
            Some(&srv.base_url()),
            Some(TOKEN),
            &["jira", "issue", "get", id, "-o", "json"],
        );
        assert_eq!(o.status.code(), Some(exit), "{id}: {}", stdout(&o));
        let v: Value = serde_json::from_str(&stdout(&o)).expect("one JSON document");
        assert_eq!(v["ok"], false, "{id}");
        let e = &v["error"];
        assert_eq!(e["code"], code, "{id}");
        assert_eq!(e["retryable"], retryable, "{id}");
        assert!(
            e["hint"].as_str().is_some_and(|h| !h.is_empty()),
            "{id}: {e}"
        );
        assert_eq!(e["request"]["method"], "GET", "{id}");
        assert!(e.get("kind").is_none() && e.get("category").is_none());
        // JSON modes print one document and nothing on stderr.
        assert_eq!(stderr(&o), "", "{id}");
        assert_no_secret(&o);
    }
}

#[test]
fn not_found_names_status_request_id_and_visibility() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "issue", "get", "NOPE-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(5));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    let e = &v["error"];
    assert_eq!(e["status"], 404);
    assert_eq!(e["requestId"], "req-123");
    assert_eq!(
        e["message"],
        "Issue does not exist or you do not have permission to see it."
    );
    assert!(e["hint"].as_str().unwrap().contains("invisible"), "{e}");
    assert!(stdout(&o).lines().count() == 1, "compact JSON");

    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "issue", "get", "NOPE-1", "-o", "jsonl"],
    );
    assert_eq!(o.status.code(), Some(5));
    assert_eq!(stdout(&o).lines().count(), 1);
    let v: Value = serde_json::from_str(stdout(&o).trim()).unwrap();
    assert_eq!(v["error"]["status"], 404);
}

#[test]
fn validation_errors_carry_field_messages_and_rate_limits_their_wait() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "issue", "get", "BAD-1", "-o", "json"],
    );
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["fields"], json!({"summary": "is required"}));
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "issue", "get", "BUSY-1", "-o", "json"],
    );
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["retryAfterMs"], 0);
}

#[test]
fn missing_token_is_config_and_tells_the_agent_to_ask() {
    let o = atlx(
        Some(NO_SITE),
        None,
        &["jira", "issue", "get", "A-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(4));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["code"], "config");
    assert!(v["error"].get("status").is_none());
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no API token configured")
    );
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("atlx auth login")
    );
    assert_eq!(stderr(&o), "");
}

#[test]
fn text_mode_prints_readable_report_only_on_stderr() {
    let srv = jira_handler();
    let o = atlx(
        Some(&srv.base_url()),
        Some(TOKEN),
        &["jira", "issue", "get", "NOPE-1", "-o", "text"],
    );
    assert_eq!(o.status.code(), Some(5));
    assert_eq!(stdout(&o), "");
    let err = stderr(&o);
    assert!(err.starts_with("✗ Failed jira issue get.\n"), "{err}");
    assert!(
        err.contains("Error: Issue does not exist or you do not have permission to see it."),
        "{err}"
    );
    assert!(err.contains("Code:        not_found"), "{err}");
    assert!(err.contains("Request ID:  req-123"), "{err}");
    assert!(err.contains("Hint:"), "{err}");
    assert_no_secret(&o);
}

#[test]
fn partial_batches_exit_3_and_strict_fails_whole() {
    let srv = jira_handler();
    let site = srv.base_url();
    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &["jira", "issue", "get", "OK-1", "NOPE-1", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(3), "{}", stdout(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["partial"], true);
    assert_eq!(v["data"].as_array().unwrap().len(), 1);
    assert_eq!(v["errors"][0]["input"], "NOPE-1");
    assert_eq!(v["errors"][0]["error"]["code"], "not_found");
    assert_eq!(v["errors"][0]["error"]["status"], 404);

    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &[
            "jira", "issue", "get", "OK-1", "NOPE-1", "--strict", "-o", "json",
        ],
    );
    assert_eq!(o.status.code(), Some(5));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(v.get("data").is_none());
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("1 of 2 inputs failed; first: NOPE-1")
    );

    let o = atlx(
        Some(&site),
        Some(TOKEN),
        &["jira", "issue", "get", "NOPE-1", "NOPE-2", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(5));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn clap_errors_are_structured_usage_errors_with_suggestions() {
    let o = atlx(None, None, &["jira", "issue", "gett", "P-1", "-o", "json"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stderr(&o), "");
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "usage");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("gett"),
        "{v}"
    );
    let s = v["error"]["suggestions"].as_array().unwrap();
    assert!(s.iter().any(|x| x == "get"), "{v}");

    let o = atlx(
        None,
        None,
        &["jira", "issue", "get", "P-1", "--bogus", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(2));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(v["error"]["message"].as_str().unwrap().contains("--bogus"));
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("atlx describe")
    );

    // Text mode: clap's own message on stderr, nothing on stdout.
    let o = atlx(
        None,
        None,
        &["jira", "issue", "get", "--bogus", "-o", "text"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stdout(&o), "");
    assert!(stderr(&o).contains("--bogus"));

    let o = atlx(None, None, &["--version"]);
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn usage_errors_after_parsing_exit_2() {
    let o = atlx(
        Some(NO_SITE),
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
    assert_eq!(o.status.code(), Some(2), "{}", stdout(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"].get("status").is_none());
    assert_no_secret(&o);
}

#[test]
fn old_paths_and_removed_flags_are_plain_usage_errors() {
    for args in [
        &["whoami", "-o", "json"][..],
        &["jira", "issue", "list", "--select", "key", "-o", "json"][..],
    ] {
        let o = atlx(Some(NO_SITE), Some(TOKEN), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(v["error"]["code"], "usage", "{args:?}");
    }
}

#[test]
fn describe_lists_every_code_with_its_exit_code() {
    let o = atlx(None, None, &["describe", "--errors", "-o", "json"]);
    assert_eq!(o.status.code(), Some(0));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    let codes: Vec<(&str, i64)> = v["data"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["code"].as_str().unwrap(), e["exit"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        codes,
        [
            ("usage", 2),
            ("config", 4),
            ("auth", 4),
            ("forbidden", 4),
            ("not_found", 5),
            ("conflict", 6),
            ("validation", 1),
            ("rate_limited", 7),
            ("server", 1),
            ("network", 7),
            ("timeout", 7),
            ("refused", 8),
            ("other", 1),
        ]
    );
}

#[test]
fn transport_failures_are_network_errors_and_unknown_writes_say_so() {
    // Nothing listens on port 9; no retries are slept through for POST.
    let c = atlx::client::Client::new(NO_SITE.into(), "me@example.com", TOKEN, Some(2_000))
        .with_backoff_base(std::time::Duration::from_millis(1));
    let err = c.get("/x", &[]).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Network);
    assert!(api.retryable());
    assert_eq!(api.exit_code(), 7);
    assert!(!api.write_state_unknown);
    let err = c.post("/x", &json!({})).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert!(api.write_state_unknown);
    assert_eq!(api.to_json()["writeState"], "unknown");
    assert!(api.to_json()["hint"].as_str().unwrap().contains("read"));
}

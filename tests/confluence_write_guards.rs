//! What `content create|update|delete` refuse before sending anything.
//! Each case here once wrote to Confluence.

mod common;

use common::{MockResponse, MockServer};
use serde_json::json;
use std::process::{Command, Output, Stdio};

fn server() -> MockServer {
    MockServer::with_handler(|_| {
        MockResponse::json(
            200,
            json!({
                "id": "123", "type": "page", "status": "current", "title": "T", "spaceId": "9",
                "version": {"number": 1}, "body": {"storage": {"value": "<p>old</p>"}},
            }),
        )
    })
}

/// Run `confluence content ARGS` without a terminal, against `site` only.
fn content(site: &str, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    // Not env_clear(): without SystemRoot, Windows can't open a socket, and
    // then "nothing was sent" would hold for the wrong reason.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.args(["confluence", "content"])
        .args(args)
        .env(
            "ATLX_CONFIG",
            std::env::temp_dir().join("atlx-no-config.toml"),
        )
        .env("ATLX_SITE", site)
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn refused_writes_send_nothing() {
    let create = "create --space-id 9 --content-type page --title T";
    let edits = r#"update 123 --edits [{"name":"deleteNode","localId":"n1"}]"#;
    let cases = [
        ("delete 123".to_string(), "--yes"),
        ("delete 123 --purge".to_string(), "--yes"),
        ("delete 123 -o json".to_string(), "--yes"),
        ("update 123 --body x".to_string(), "--format"),
        (
            "update 123 --body x --format html".to_string(),
            "--ack-body-formats",
        ),
        (edits.to_string(), "--ack-body-formats"),
        (
            "update 123 --body x --format html --ack-body-formats".to_string(),
            "--snapshot-token",
        ),
        ("update 123 --title T --yes".to_string(), "--snapshot-token"),
        (
            "update 123 --title T --dry-run".to_string(),
            "--snapshot-token",
        ),
        (
            "update 123 --title T --snapshot-token 1".to_string(),
            "--yes",
        ),
        (
            "update 123 --title T --snapshot-token 1 --draft".to_string(),
            "--yes",
        ),
        (
            "update 123 --body x --format html --ack-body-formats --snapshot-token 1".to_string(),
            "--yes",
        ),
        (create.to_string(), "--body"),
        (format!("{create} --body x"), "--format"),
        (
            format!("{create} --body x --format html"),
            "--ack-body-formats",
        ),
        (
            format!("{create} --body x --format html --ack-body-formats"),
            "--yes",
        ),
        (
            format!("{create} --body x --format md --ack-body-formats --draft"),
            "--yes",
        ),
    ];
    for (line, needs) in cases {
        let srv = server();
        let args: Vec<&str> = line.split(' ').collect();
        let out = content(&srv.base_url(), &args);
        let err = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // Usage errors exit 2, a missing --yes is `refused` (exit 8).
        assert!(matches!(out.status.code(), Some(2 | 8)), "{line}: {err}");
        assert!(err.contains(needs), "{line}: expected {needs} in {err}");
        assert!(
            srv.requests().is_empty(),
            "{line}: sent {:?}",
            srv.requests()
        );
    }
}

#[test]
fn confirmed_writes_are_sent() {
    for (line, method) in [
        ("delete 123 --yes", "DELETE"),
        ("delete 123 --purge -y", "DELETE"),
        ("update 123 --title New --snapshot-token 1 --yes", "PUT"),
        (
            "update 123 --body x --format html --ack-body-formats --snapshot-token 1 -y",
            "PUT",
        ),
        (
            "create --space-id 9 --content-type page --title T --body x --format html --ack-body-formats --yes",
            "POST",
        ),
    ] {
        let srv = server();
        let args: Vec<&str> = line.split(' ').collect();
        let out = content(&srv.base_url(), &args);
        assert!(
            out.status.success(),
            "{line}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            srv.requests().iter().any(|r| r.method == method),
            "{line}: no {method}"
        );
    }
}

#[test]
fn a_dry_run_needs_no_yes_and_only_reads() {
    let srv = server();
    let out = content(
        &srv.base_url(),
        &[
            "update",
            "123",
            "--title",
            "New",
            "--snapshot-token",
            "1",
            "--dry-run",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

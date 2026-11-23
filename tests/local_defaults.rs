//! Defaults from a repository's `.atlx.toml` reach the commands that use
//! them, and an explicit flag still wins.

mod common;

use common::{MockResponse, MockServer};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn repo(tag: &str, toml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlx-local-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join(".atlx.toml"), toml).unwrap();
    dir
}

fn run(dir: &Path, site: &str, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.args(args)
        .args(["-o", "json"])
        .env("ATLX_CONFIG", dir.join("config.toml"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("ATLX_SITE", site)
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        // From a subdirectory: the file is found by walking up.
        .current_dir(dir.join("sub"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn space_server() -> MockServer {
    MockServer::with_handler(|r| {
        if r.path.ends_with("/spaces") {
            MockResponse::json(
                200,
                json!({"results": [{"id": "9", "key": "ENG"}], "_links": {}}),
            )
        } else {
            MockResponse::json(200, json!({"results": [], "_links": {}}))
        }
    })
}

#[test]
fn page_list_uses_the_confluence_space_default() {
    let dir = repo("space", "[confluence]\nspace = \"ENG\"\n");
    let srv = space_server();
    let o = run(&dir, &srv.base_url(), &["confluence", "page", "list"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let reqs = srv.requests();
    assert!(
        reqs.iter()
            .any(|r| r.query.contains("ENG") || r.path.contains("/spaces/9/")),
        "space not used: {reqs:?}"
    );

    // An explicit --space wins over the file.
    let srv = space_server();
    run(
        &dir,
        &srv.base_url(),
        &["confluence", "page", "list", "--space", "OPS"],
    );
    let reqs = srv.requests();
    assert!(reqs.iter().any(|r| r.query.contains("OPS")), "{reqs:?}");
    assert!(!reqs.iter().any(|r| r.query.contains("ENG")), "{reqs:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn without_a_default_page_list_and_create_ask_for_the_space() {
    let dir = repo("nospace", "[jira]\nproject = \"PROJ\"\n");
    let srv = space_server();
    for args in [
        &["confluence", "page", "list"][..],
        &[
            "confluence",
            "page",
            "create",
            "--title",
            "T",
            "--body",
            "x",
        ][..],
    ] {
        let o = run(&dir, &srv.base_url(), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("confluence.space"),
            "{v}"
        );
    }
    assert!(srv.requests().is_empty(), "{:?}", srv.requests());
    let _ = std::fs::remove_dir_all(dir);
}

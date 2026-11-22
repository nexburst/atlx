//! Offline tests for branches, commits, workspaces and repository extras (#33).

mod common;

use atlx::bitbucket::repos::{self, Change};
use atlx::client::{ApiError, ErrorKind};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

const REPO: &str = "/repositories/acme/api";

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn page(values: Value) -> MockResponse {
    ok(json!({"values": values}))
}

fn not_found() -> MockResponse {
    MockResponse::json(404, json!({"error": {"message": "no such branch"}}))
}

fn text(p: &str) -> Change {
    Change::Text {
        repo_path: p.into(),
        text: format!("contents of {p}"),
    }
}

#[test]
fn branch_create_resolves_the_source() {
    let srv = MockServer::with_responses([
        ok(json!({"target": {"hash": "abc1234def"}})),
        ok(json!({})),
        not_found(),
        ok(json!({})),
        not_found(),
    ]);
    let c = srv.client();
    repos::branch_create(&c, REPO, "feature/x", Some("release/1")).unwrap();
    repos::branch_create(&c, REPO, "hot", Some("0123456789ab")).unwrap();
    assert!(repos::branch_create(&c, REPO, "bad", Some("nope")).is_err());
    let r = srv.requests();
    assert_eq!(
        r[0].path,
        "/repositories/acme/api/refs/branches/release%2F1"
    );
    assert_eq!(
        r[1].json_body(),
        json!({"name": "feature/x", "target": {"hash": "abc1234def"}})
    );
    assert_eq!(r[3].json_body()["target"]["hash"], "0123456789ab");
    assert_eq!(r.len(), 5);
}

#[test]
fn branch_delete_encodes_the_name() {
    let srv = MockServer::with_responses([MockResponse::empty(204)]);
    let c = srv.client();
    assert!(repos::branch_delete(&c, REPO, " ").is_err());
    assert!(repos::branch_delete(&c, REPO, "..").is_err());
    repos::branch_delete(&c, REPO, "feature/x").unwrap();
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(
        r[0].path,
        "/repositories/acme/api/refs/branches/feature%2Fx"
    );
}

#[test]
fn commit_files_cannot_pass_for_form_fields() {
    let dir = std::env::temp_dir().join(format!("atlx-commit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let local = dir.join("local.txt");
    std::fs::write(&local, "local bytes").unwrap();
    let srv = MockServer::with_responses([MockResponse::empty(201).with_header(
        "Location",
        "https://api.bitbucket.org/2.0/repositories/acme/api/commit/fff111",
    )]);
    let changes = [
        text("message"),
        text("branch"),
        Change::Local {
            repo_path: "docs/files".into(),
            local,
        },
        Change::Delete {
            repo_path: "old.txt".into(),
        },
    ];
    let new = repos::NewCommit {
        message: "Edit",
        branch: Some("dev"),
        author: None,
        parent: Some("abc123"),
        changes: &changes,
    };
    assert_eq!(
        repos::commit_create(&srv.client(), REPO, &new).unwrap()["hash"],
        "fff111"
    );
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/repositories/acme/api/src");
    for part in [
        "name=\"message\"\r\n\r\nEdit\r\n",
        "name=\"branch\"\r\n\r\ndev\r\n",
        "name=\"parents\"\r\n\r\nabc123\r\n",
        "name=\"/message\"\r\n\r\ncontents of message\r\n",
        "name=\"/branch\"\r\n\r\ncontents of branch\r\n",
        "name=\"/docs/files\"; filename=\"files\"",
        "name=\"files\"\r\n\r\n/old.txt\r\n",
    ] {
        assert!(r.body.contains(part), "missing {part:?}");
    }
    assert!(r.body.contains("local bytes"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commit_create_validates_before_sending() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    let missing = Change::Local {
        repo_path: "a".into(),
        local: "no/such/file".into(),
    };
    let cases: Vec<Vec<Change>> = vec![
        vec![],
        vec![text("../x")],
        vec![text("/etc/passwd")],
        vec![text(".git/config")],
        vec![text("a\"b")],
        vec![
            text("a"),
            Change::Delete {
                repo_path: "a".into(),
            },
        ],
        vec![missing],
    ];
    for changes in &cases {
        let new = repos::NewCommit {
            message: "m",
            branch: None,
            author: None,
            parent: None,
            changes,
        };
        let err = repos::commit_create(&c, REPO, &new).unwrap_err();
        assert_eq!(
            ApiError::find(&err).unwrap().kind,
            ErrorKind::Usage,
            "{changes:?}"
        );
    }
    let ok_change = [text("a")];
    let blank = repos::NewCommit {
        message: "m",
        branch: Some(" "),
        author: None,
        parent: None,
        changes: &ok_change,
    };
    assert!(
        repos::commit_create(&c, REPO, &blank).is_err(),
        "a blank branch must not mean the default"
    );
    assert!(srv.requests().is_empty());
}

#[test]
fn commit_query_defaults_to_the_main_branch() {
    let srv = MockServer::with_responses([
        ok(json!({"mainbranch": {"name": "trunk"}})),
        page(json!([])),
        page(json!([])),
    ]);
    let c = srv.client();
    repos::commit_query(
        &c,
        REPO,
        &repos::CommitQuery {
            limit: 20,
            ..Default::default()
        },
    )
    .unwrap();
    let inc = ["feature/x".to_string()];
    let o = repos::CommitQuery {
        include: &inc,
        file_path: Some("src"),
        limit: 20,
        ..Default::default()
    };
    repos::commit_query(&c, REPO, &o).unwrap();
    let r = srv.requests();
    assert_eq!(r[1].query_param("include").as_deref(), Some("trunk"));
    assert_eq!(r[2].query_param("include").as_deref(), Some("feature/x"));
    assert_eq!(r[2].query_param("path").as_deref(), Some("src"));
}

#[test]
fn reports_create_and_query() {
    let srv = MockServer::with_responses([ok(json!({})), page(json!([]))]);
    let c = srv.client();
    repos::report_create(&c, REPO, "abc", "lint", &json!({"title": "Lint"})).unwrap();
    repos::reports(&c, REPO, "abc", 100).unwrap();
    assert!(repos::report_create(&c, REPO, "abc", "lint", &json!([])).is_err());
    let r = srv.requests();
    assert_eq!(r[0].method, "PUT");
    assert_eq!(r[0].path, "/repositories/acme/api/commit/abc/reports/lint");
    assert_eq!(r[1].path, "/repositories/acme/api/commit/abc/reports");
}

#[test]
fn workspace_and_members() {
    let people = || {
        page(json!([
            {"user": {"uuid": "{a}", "nickname": "ann", "display_name": "Ann Lee"}},
            {"user": {"uuid": "{b}", "nickname": "ben"}}
        ]))
    };
    let srv = MockServer::with_responses([
        ok(json!({})),
        people(),
        page(json!([])),
        people(),
        ok(json!({})),
    ]);
    let c = srv.client();
    repos::workspace_get(&c, "acme").unwrap();
    assert_eq!(
        repos::members(&c, "acme", Some("lee"), false, 50)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    repos::members(&c, "acme", None, true, 50).unwrap();
    repos::member(&c, "acme", "ann").unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/workspaces/acme");
    assert_eq!(r[2].path, "/workspaces/acme/permissions");
    assert_eq!(r[4].path, "/workspaces/acme/members/%7Ba%7D");
}

#[test]
fn repo_create_sends_privacy() {
    let srv = MockServer::with_responses([ok(json!({}))]);
    let new = repos::NewRepo {
        name: None,
        description: None,
        language: None,
        project: Some("CORE"),
        private: true,
    };
    repos::repo_create(&srv.client(), REPO, &new).unwrap();
    assert_eq!(
        srv.requests()[0].json_body(),
        json!({"scm": "git", "is_private": true, "project": {"key": "CORE"}})
    );
}

#[test]
fn list_files_resolves_refs_with_slashes() {
    let srv =
        MockServer::with_responses([ok(json!({"target": {"hash": "abc1234"}})), page(json!([]))]);
    repos::list_files(
        &srv.client(),
        REPO,
        Some("feature/x"),
        Some("src"),
        Some(2),
        100,
    )
    .unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.path, "/repositories/acme/api/src/abc1234/src/");
    assert_eq!(r.query_param("max_depth").as_deref(), Some("2"));
}

#[test]
fn web_urls() {
    let url = |f, r, l| repos::web_url(REPO, f, r, l);
    assert_eq!(
        url(None, None, None).unwrap(),
        "https://bitbucket.org/acme/api"
    );
    assert_eq!(
        url(Some("src/a b.rs"), Some("main"), Some(7)).unwrap(),
        "https://bitbucket.org/acme/api/src/main/src/a%20b.rs#lines-7"
    );
    assert!(url(None, None, Some(7)).is_err());
}

#[test]
fn contributors_count_authors() {
    let commit = |raw: &str, day: &str| json!({"author": {"raw": raw}, "date": format!("{day}T00:00:00+00:00")});
    let srv = MockServer::with_responses([page(json!([
        commit("A", "2026-03-02"),
        commit("B", "2026-03-02"),
        commit("B", "2026-03-01"),
        commit("A", "2026-01-01")
    ]))]);
    let o = repos::Contributors {
        range: Some("v1..main"),
        since: Some("2026-02-01"),
        limit: 10,
        ..Default::default()
    };
    let out = repos::contributors(&srv.client(), REPO, &o).unwrap();
    assert_eq!(out["values"][0]["author"]["raw"], "B");
    assert_eq!(out["values"][0]["commits"], 2);
    assert_eq!(out["values"][1]["commits"], 1);
    assert_eq!(
        srv.requests()[0].query_param("exclude").as_deref(),
        Some("v1")
    );
}

#[test]
fn search_prs_by_field() {
    let srv = MockServer::with_responses([page(json!([]))]);
    repos::search_prs(
        &srv.client(),
        REPO,
        "login",
        repos::Field::Title,
        "open",
        25,
    )
    .unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.query_param("q").as_deref(), Some("(title ~ \"login\")"));
    assert_eq!(r.query_param("state").as_deref(), Some("OPEN"));
}

#[test]
fn code_search_follows_the_cursor_and_stops_without_one() {
    let file = |p: &str| serde_json::json!({"path": p, "matched_chunks": [{}]});
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            serde_json::json!({"id": "r1", "matched_files": [file("a.rs")],
                "pagination": {"has_more": true, "next_cursor": "c2"}}),
        ),
        MockResponse::json(
            200,
            serde_json::json!({"id": "r1", "matched_files": [file("b.rs")],
                "pagination": {"has_more": true}}),
        ),
    ]);
    assert!(atlx::bitbucket::search_code(&srv.client(), "acme", "x", None, 5).is_err());
    let r = srv.requests();
    assert_eq!(r[1].json_body()["request_id"], "r1");
    assert_eq!(r[1].json_body()["pagination"]["next_cursor"], "c2");
}

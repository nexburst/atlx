//! Offline tests for branches, commits, files, workspaces and the other
//! repository commands.

mod common;

use atlx::bitbucket::Paging;
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
fn commit_list_defaults_to_the_main_branch() {
    let srv = MockServer::with_responses([
        ok(json!({"mainbranch": {"name": "trunk"}})),
        page(json!([])),
        page(json!([])),
    ]);
    let c = srv.client();
    let p = Paging::new(20);
    repos::commit_list(&c, REPO, &repos::CommitQuery::default(), &p).unwrap();
    let inc = ["feature/x".to_string()];
    let o = repos::CommitQuery {
        include: &inc,
        file_path: Some("src"),
        ..Default::default()
    };
    repos::commit_list(&c, REPO, &o, &p).unwrap();
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
    repos::reports(&c, REPO, "abc", &Paging::new(100)).unwrap();
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
    let p = Paging::new(50);
    let (found, _) = repos::members(&c, "acme", Some("lee"), false, &p).unwrap();
    assert_eq!(found.len(), 1);
    repos::members(&c, "acme", None, true, &p).unwrap();
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
fn file_list_resolves_refs_with_slashes() {
    let srv =
        MockServer::with_responses([ok(json!({"target": {"hash": "abc1234"}})), page(json!([]))]);
    repos::file_list(
        &srv.client(),
        REPO,
        Some("feature/x"),
        Some("src"),
        Some(2),
        &Paging::new(100),
    )
    .unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.path, "/repositories/acme/api/src/abc1234/src/");
    assert_eq!(r.query_param("max_depth").as_deref(), Some("2"));
}

#[test]
fn file_get_reads_meta_then_text_and_refuses_directories() {
    let srv = MockServer::with_responses([
        ok(
            json!({"type": "commit_file", "path": "README.md", "size": 8, "commit": {"hash": "c1"}}),
        ),
        MockResponse::text(200, "# Hello\n"),
        ok(json!({"mainbranch": {"name": "develop"}})),
        ok(json!({"type": "commit_directory", "path": "src"})),
    ]);
    let c = srv.client();
    let f = repos::file_get(&c, REPO, "README.md", Some("main")).unwrap();
    assert_eq!(
        f,
        json!({"path": "README.md", "ref": "main", "commit": "c1", "size": 8,
               "type": "commit_file", "content": "# Hello\n"})
    );
    let err = repos::file_get(&c, REPO, "src", None).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Usage);
    assert!(
        api.hint
            .as_deref()
            .unwrap()
            .contains("repo file list --path src")
    );
    let r = srv.requests();
    assert_eq!(r[0].path, "/repositories/acme/api/src/main/README.md");
    assert_eq!(r[0].query_param("format").as_deref(), Some("meta"));
    assert_eq!(r[1].query_param("format"), None);
    assert_eq!(r[3].path, "/repositories/acme/api/src/develop/src");
}

#[test]
fn file_download_writes_the_bytes() {
    let dir = std::env::temp_dir().join(format!("atlx-bb-dl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("logo.bin");
    let _ = std::fs::remove_file(&dest);
    let srv = MockServer::with_responses([MockResponse::text(200, "\u{0}\u{1}binary")]);
    let out =
        repos::file_download(&srv.client(), REPO, "a/logo.bin", Some("v1"), &dest, false).unwrap();
    assert_eq!(out["bytes"], 8);
    assert_eq!(std::fs::read(&dest).unwrap(), b"\0\x01binary");
    assert_eq!(
        srv.requests()[0].path,
        "/repositories/acme/api/src/v1/a/logo.bin"
    );
    // An existing file is kept unless --overwrite.
    let srv = MockServer::with_responses([MockResponse::text(200, "new")]);
    assert!(
        repos::file_download(&srv.client(), REPO, "a/logo.bin", Some("v1"), &dest, false).is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
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
    assert_eq!(out[0]["author"]["raw"], "B");
    assert_eq!(out[0]["commits"], 2);
    assert_eq!(out[1]["commits"], 1);
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
        Some("open"),
        &Paging::new(25),
    )
    .unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.query_param("q").as_deref(), Some("(title ~ \"login\")"));
    assert_eq!(r.query_param("state").as_deref(), Some("OPEN"));
}

#[test]
fn code_search_follows_the_cursor_and_stops_without_one() {
    let file = |p: &str| json!({"path": p, "matched_chunks": [{}]});
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"id": "r1", "matched_files": [file("a.rs")],
                "pagination": {"has_more": true, "next_cursor": "c2"}}),
        ),
        MockResponse::json(
            200,
            json!({"id": "r1", "matched_files": [file("b.rs")],
                "pagination": {"has_more": true}}),
        ),
    ]);
    let c = srv.client();
    assert!(atlx::bitbucket::search_code(&c, "acme", "x", None, &Paging::new(5)).is_err());
    let r = srv.requests();
    assert_eq!(r[1].json_body()["request_id"], "r1");
    assert_eq!(r[1].json_body()["pagination"]["next_cursor"], "c2");
}

#[test]
fn code_search_is_a_read_and_hands_back_its_cursor() {
    let file = |p: &str| json!({"path": p, "matched_chunks": [{}, {}], "url": "https://bitbucket.org/acme/api/src/main/a.rs"});
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"id": "r1", "matched_files": [file("a.rs")],
                "pagination": {"has_more": true, "next_cursor": "c2"}}),
        ),
        MockResponse::json(
            200,
            json!({"id": "r1", "matched_files": [file("b.rs")], "pagination": {"has_more": false}}),
        ),
    ]);
    // Read-only mode lets the search through: it is a POST that only reads.
    let c = srv.client().with_mode(atlx::client::Mode::ReadOnly);
    let (hits, next) =
        atlx::bitbucket::search_code(&c, "acme", "x", Some("api"), &Paging::new(1)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(next.as_deref(), Some("t:r1|c2"));
    let rec = atlx::schema::project(
        &hits[0],
        &atlx::bitbucket::records::CODE_HIT,
        &atlx::schema::Selection::Default,
    )
    .unwrap();
    assert_eq!(rec["repo"], "acme/api");
    assert_eq!(rec["matches"], 2);
    let p = Paging {
        limit: 5,
        cursor: next,
    };
    let (hits, next) = atlx::bitbucket::search_code(&c, "acme", "x", None, &p).unwrap();
    assert_eq!(hits[0]["path"], "b.rs");
    assert_eq!(next, None);
    assert_eq!(
        srv.requests()[1].json_body()["pagination"]["next_cursor"],
        "c2"
    );
}

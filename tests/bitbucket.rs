//! Offline Bitbucket tests: API functions against the mock HTTP server, and
//! the binary's safety rules (no network needed: they stop before any
//! request).

mod common;

use atlx::bitbucket::{self, NewComment, NewPr, Paging, PipelineQuery, PrQuery, RepoQuery};
use atlx::schema::{Selection, project};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::process::{Command, Output, Stdio};

fn page(n: usize) -> Paging {
    Paging::new(n)
}

#[test]
fn lists_follow_pages_up_to_the_limit_and_return_a_cursor() {
    let srv = MockServer::with_handler(|req| {
        let host = req.header("host").unwrap().to_string();
        match (req.path.as_str(), req.query_param("page").as_deref()) {
            ("/repositories/acme", None) => MockResponse::json(
                200,
                json!({
                    "values": [{"full_name": "acme/r1"}, {"full_name": "acme/r2"}],
                    "next": format!("http://{host}/repositories/acme?pagelen=2&page=2"),
                }),
            ),
            ("/repositories/acme", Some("2")) => MockResponse::json(
                200,
                json!({
                    "values": [{"full_name": "acme/r3"}, {"full_name": "acme/r4"}],
                    "next": format!("http://{host}/repositories/acme?pagelen=2&page=3"),
                }),
            ),
            ("/repositories/acme/api/pullrequests", None) => {
                MockResponse::json(200, json!({"values": [{"id": 10}, {"id": 11}, {"id": 12}]}))
            }
            _ => MockResponse::text(404, "unexpected route"),
        }
    });
    let client = srv.client();

    // The shared helper keeps its shape: an array.
    let repos = bitbucket::repo_query(
        &client,
        "acme",
        &RepoQuery {
            query: Some("api"),
            project: Some("CORE"),
            language: Some("Rust"),
            sort: "-updated_on",
            role: Some("member"),
            limit: 2,
        },
    )
    .unwrap();
    assert_eq!(repos.as_array().unwrap().len(), 2);

    let (items, next) = bitbucket::repo_list(
        &client,
        "acme",
        &bitbucket::RepoList {
            name: Some("api"),
            ..Default::default()
        },
        &Paging {
            limit: 3,
            cursor: None,
        },
    )
    .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2]["full_name"], "acme/r3");
    // The third record came from page 2, so the cursor resumes inside it,
    // with the page size of the first request.
    assert_eq!(next.as_deref(), Some("t+1:3:2"));

    let (prs, next) = bitbucket::pr_list(
        &client,
        "/repositories/acme/api",
        &PrQuery {
            state: Some("open"),
            dest: Some("main"),
            ..Default::default()
        },
        &page(3),
    )
    .unwrap();
    assert_eq!(prs.len(), 3);
    assert_eq!(next, None);

    let reqs = srv.requests();
    assert_eq!(reqs[0].query_param("role").as_deref(), Some("member"));
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("2"));
    let last = reqs.last().unwrap();
    assert_eq!(last.query_param("state").as_deref(), Some("OPEN"));
    assert!(
        last.query_param("q")
            .unwrap()
            .contains("destination.branch.name = \"main\"")
    );
}

#[test]
fn every_state_and_bbql_reach_the_request() {
    let srv = MockServer::with_handler(|_| MockResponse::json(200, json!({"values": []})));
    bitbucket::pr_list(
        &srv.client(),
        "/repositories/acme/api",
        &PrQuery {
            state: Some("ALL"),
            bbql: Some("comment_count > 3"),
            ..Default::default()
        },
        &page(5),
    )
    .unwrap();
    let r = &srv.requests()[0];
    let raw = r.query.clone();
    for s in ["OPEN", "MERGED", "DECLINED", "SUPERSEDED"] {
        assert!(raw.contains(&format!("state={s}")), "{raw}");
    }
    assert_eq!(r.query_param("q").as_deref(), Some("(comment_count > 3)"));
}

fn pr_payload() -> Value {
    json!({
        "id": 42,
        "title": "Add tests",
        "state": "OPEN",
        "draft": false,
        "author": {"uuid": "{jane}", "display_name": "Jane", "links": {"avatar": {"href": "x"}}},
        "source": {"branch": {"name": "feat/tests"}, "commit": {"hash": "3f2a9c1b7e44"}},
        "destination": {"branch": {"name": "main"}, "repository": {"full_name": "acme/api"}},
        "updated_on": "2026-01-05",
        "links": {"html": {"href": "https://bitbucket.org/acme/api/pull-requests/42"}},
        "reviewers": [{"uuid": "{bob}", "display_name": "Bob"}],
        "participants": [
            {"user": {"uuid": "{bob}", "display_name": "Bob"}, "approved": true, "state": "approved"},
            {"user": {"uuid": "{carl}", "display_name": "Carl"}, "approved": false}
        ],
        "comment_count": 2,
        "task_count": 0,
        "description": "Adds integration tests"
    })
}

#[test]
fn a_pull_request_projects_onto_its_records() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, pr_payload()),
        MockResponse::json(
            201,
            json!({"id": 43, "title": "Follow-up", "state": "OPEN"}),
        ),
        MockResponse::json(
            201,
            json!({"id": 99, "content": {"raw": "Nice"}, "inline": {"path": "README.md", "to": 5}}),
        ),
    ]);
    let client = srv.client();
    let p = "/repositories/acme/api";

    let out = bitbucket::pr_get(&client, p, 42).unwrap();
    let rec = project(&out, &bitbucket::records::PR, &Selection::Default).unwrap();
    assert_eq!(rec["id"], 42);
    assert_eq!(rec["author"], json!({"id": "{jane}", "name": "Jane"}));
    assert_eq!(rec["head"], "3f2a9c1b7e44");
    assert_eq!(rec["approvals"], 1);
    assert_eq!(
        rec["reviewers"],
        json!([{"id": "{bob}", "name": "Bob", "approved": true, "state": "approved"}])
    );
    assert_eq!(rec["body"], "Adds integration tests");
    assert!(!rec.to_string().contains("avatar"), "no link noise");
    let row = project(&out, &bitbucket::records::PR_ROW, &Selection::Default).unwrap();
    assert!(row.get("body").is_none() && row.get("reviewers").is_none());
    let full = project(&out, &bitbucket::records::PR, &Selection::Full).unwrap();
    assert_eq!(full["repo"], "acme/api");

    let created = bitbucket::pr_create(
        &client,
        p,
        NewPr {
            title: "Follow-up",
            source: "feat/x",
            dest: Some("main"),
            description: Some("desc"),
            reviewer_uuids: vec!["{u1}".into()],
            reviewer_account_ids: vec![],
            close_source_branch: true,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(created["id"], 43);
    let body = srv.requests()[1].json_body();
    assert_eq!(body["reviewers"], json!([{"uuid": "{u1}"}]));
    assert_eq!(body["description"], "desc");

    let comment = bitbucket::pr_comment(
        &client,
        p,
        43,
        NewComment {
            text: "Nice",
            path: Some("README.md"),
            anchor: bitbucket::Anchor {
                line: Some(5),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let rec = project(
        &comment,
        &bitbucket::records::PR_COMMENT,
        &Selection::Default,
    )
    .unwrap();
    assert_eq!(
        (rec["path"].clone(), rec["line"].clone()),
        (json!("README.md"), json!(5))
    );
    assert_eq!(rec["body"], "Nice");
}

#[test]
fn pipelines_filter_client_side_and_carry_a_web_url() {
    let run = |n: u64, result: &str, trigger: &str| {
        json!({"build_number": n, "uuid": format!("{{p{n}}}"),
               "state": {"name": "COMPLETED", "result": {"name": result}},
               "trigger": {"name": trigger}, "repository": {"full_name": "acme/api"}})
    };
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"values": [run(1, "SUCCESSFUL", "PUSH"), run(2, "FAILED", "MANUAL_TRIGGER"), run(3, "FAILED", "PUSH")]}),
    )]);
    let (items, next) = bitbucket::pipeline_list(
        &srv.client(),
        "/repositories/acme/api",
        &PipelineQuery {
            state: Some("COMPLETED"),
            result: Some("FAILED"),
            branch: Some("main"),
            trigger: Some("MANUAL"),
            pattern: None,
        },
        &page(10),
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(next, None);
    let rec = project(
        &items[0],
        &bitbucket::records::PIPELINE,
        &Selection::Default,
    )
    .unwrap();
    assert_eq!(rec["buildNumber"], 2);
    assert_eq!(rec["result"], "FAILED");
    assert_eq!(rec["trigger"], "MANUAL");
    assert_eq!(
        rec["url"],
        "https://bitbucket.org/acme/api/pipelines/results/2"
    );
    let r = &srv.requests()[0];
    assert_eq!(r.query_param("target.branch").as_deref(), Some("main"));
    assert_eq!(r.query_param("pagelen").as_deref(), Some("50"));
}

#[test]
fn a_filter_that_matches_nothing_stops_with_a_cursor() {
    let srv = MockServer::with_handler(|req| {
        let host = req.header("host").unwrap().to_string();
        let n: u64 = req
            .query_param("page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(1);
        MockResponse::json(
            200,
            json!({
                "values": [{"build_number": n, "state": {"name": "COMPLETED", "result": {"name": "SUCCESSFUL"}}}],
                "next": format!("http://{host}/x?page={}", n + 1),
            }),
        )
    });
    let (items, next) = bitbucket::pipeline_list(
        &srv.client(),
        "/repositories/acme/api",
        &PipelineQuery {
            result: Some("FAILED"),
            ..Default::default()
        },
        &page(5),
    )
    .unwrap();
    assert!(items.is_empty());
    assert_eq!(srv.requests().len(), bitbucket::SCAN_PAGES);
    assert_eq!(next.as_deref(), Some("t:50:21"));
}

#[test]
fn branches_filter_by_author_and_keep_their_head() {
    let branch = |name: &str, who: &str| json!({"name": name, "target": {"hash": "abc", "date": "2026-01-01", "message": "First line\n\nmore", "author": {"raw": format!("{who} <x@example.com>"), "user": {"display_name": who}}}});
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"values": [branch("a", "Alice Example"), branch("b", "Bob"), branch("c", "Alice Example")]}),
    )]);
    let (out, next) = bitbucket::branch_list(
        &srv.client(),
        "/repositories/acme/api",
        &bitbucket::BranchQuery {
            name: Some("feat"),
            author: Some("alice"),
            sort: Some("-target.date"),
            bbql: None,
        },
        &page(1),
    )
    .unwrap();
    let names: Vec<_> = out.iter().map(|b| &b["name"]).collect();
    assert_eq!(names, [&json!("a")]);
    // The second match is still on this page: resume inside it.
    assert_eq!(next.as_deref(), Some("s+1:"));
    let rec = project(&out[0], &bitbucket::records::BRANCH, &Selection::Default).unwrap();
    assert_eq!(rec["title"], "First line");
    assert_eq!(rec["author"], "Alice Example");
    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/repositories/acme/api/refs/branches");
    assert_eq!(reqs[0].query_param("sort").as_deref(), Some("-target.date"));
    assert_eq!(reqs[0].query_param("q").as_deref(), Some("name ~ \"feat\""));
}

#[test]
fn workspaces_come_from_the_repository() {
    assert_eq!(
        bitbucket::workspace_of("acme/api", None, None).unwrap(),
        "acme"
    );
    assert_eq!(
        bitbucket::workspace_of("https://bitbucket.org/team/web", Some("other"), None).unwrap(),
        "team"
    );
    assert_eq!(
        bitbucket::workspace_of("api", Some("acme"), Some("dflt")).unwrap(),
        "acme"
    );
    assert_eq!(
        bitbucket::workspace_of("api", None, Some("dflt")).unwrap(),
        "dflt"
    );
}

// --- the binary --------------------------------------------------------------

fn atlx(args: &[&str]) -> Output {
    let dir = std::env::temp_dir().join(format!("atlx-bb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.args(args)
        .env("ATLX_CONFIG", dir.join("config.toml"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .env("ATLX_BB_WORKSPACE", "acme")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn json_out(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

/// Every destructive Bitbucket command, run without `--yes`, is refused
/// (exit 8) before it reads its config or sends anything.
#[test]
fn destructive_commands_refuse_without_yes() {
    let all = json_out(&atlx(&["describe", "bitbucket", "--all", "-o", "json"]));
    let destructive: Vec<&Value> = all["data"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["effect"] == "destructive")
        .collect();
    assert!(destructive.len() >= 7, "{}", destructive.len());
    for c in destructive {
        let path = c["path"].as_str().unwrap();
        assert_eq!(c["confirm"], true, "{path}");
        let mut argv: Vec<String> = path.split(' ').map(String::from).collect();
        for p in c["positionals"].as_array().unwrap() {
            if p["required"] == true {
                argv.push("1".into());
            }
        }
        for f in c["flags"].as_array().unwrap() {
            if f["required"] == true {
                argv.push(f["name"].as_str().unwrap().into());
                argv.push("1".into());
            }
        }
        argv.extend(["--repo", "acme/api", "-o", "json"].map(String::from));
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let o = atlx(&args);
        assert_eq!(
            o.status.code(),
            Some(8),
            "{path}: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        let v = json_out(&o);
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "refused", "{path}");
        assert!(
            v["error"]["hint"].as_str().unwrap().contains("--dry-run"),
            "{path}"
        );
        assert!(o.stderr.is_empty(), "{path}");
    }
}

#[test]
fn merge_and_decline_name_their_guard() {
    for args in [
        &[
            "bb", "pr", "merge", "42", "--repo", "acme/api", "-o", "json",
        ][..],
        &["bitbucket", "pr", "decline", "42", "-o", "json"][..],
    ] {
        let o = atlx(args);
        assert_eq!(o.status.code(), Some(8), "{args:?}");
        let v = json_out(&o);
        assert!(v["error"]["message"].as_str().unwrap().contains("--yes"));
    }
}

#[test]
fn bad_cursors_and_ids_are_usage_errors() {
    for args in [
        &["bb", "pr", "get", "abc", "--repo", "acme/api", "-o", "json"][..],
        &[
            "bb", "pr", "list", "--cursor", "o:5", "--repo", "acme/api", "-o", "json",
        ][..],
        &[
            "bb", "pr", "list", "--cursor", "nope", "--repo", "acme/api", "-o", "json",
        ][..],
        &["bb", "pr", "list", "--repo", "/", "-o", "json"][..],
    ] {
        let o = atlx(args);
        assert_eq!(
            o.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        assert_eq!(json_out(&o)["error"]["code"], "usage", "{args:?}");
    }
}

#[test]
fn describe_shows_records_and_budgets() {
    let d = json_out(&atlx(&[
        "describe",
        "bitbucket",
        "pr",
        "diff",
        "-o",
        "json",
    ]));
    let diff = d["data"]["record"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "diff")
        .unwrap()
        .clone();
    assert_eq!(diff["truncated"]["limit"], 40_000);
    assert_eq!(diff["truncated"]["more"], "--diff-offset");
    let log = json_out(&atlx(&[
        "describe",
        "bitbucket",
        "pipeline",
        "log",
        "-o",
        "json",
    ]));
    assert_eq!(log["data"]["effect"], "read");
    assert!(log["data"]["examples"].as_array().unwrap().len() >= 2);
    let merge = json_out(&atlx(&[
        "describe",
        "bitbucket",
        "pr",
        "merge",
        "-o",
        "json",
    ]));
    assert_eq!(merge["data"]["effect"], "destructive");
    assert!(
        merge["data"]["exitCodes"]
            .as_array()
            .unwrap()
            .contains(&6.into())
    );
}

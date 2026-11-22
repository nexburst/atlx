//! Offline Bitbucket API tests against the mock HTTP server.

mod common;

use atlx::bitbucket::{
    self, NewComment, NewPr, PipelineGet, PipelineQuery, PrGet, PrQuery, RepoQuery,
};
use common::{MockResponse, MockServer};
use serde_json::json;

#[test]
fn bitbucket_repo_query_and_pr_query_follow_next_urls_up_to_limit() {
    let srv = MockServer::with_handler(|req| {
        match (req.path.as_str(), req.query_param("page").as_deref()) {
            ("/repositories/acme", None) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"full_name": "acme/r1", "name": "r1", "is_private": true},
                        {"full_name": "acme/r2", "name": "r2", "is_private": true}
                    ],
                    "next": format!("http://{}/repositories/acme?page=2", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme", Some("2")) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"full_name": "acme/r3", "name": "r3", "is_private": false},
                        {"full_name": "acme/r4", "name": "r4", "is_private": false}
                    ],
                    "next": format!("http://{}/repositories/acme?page=3", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme/api/pullrequests", None) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"id": 10, "title": "PR 10", "state": "OPEN"},
                        {"id": 11, "title": "PR 11", "state": "OPEN"}
                    ],
                    "next": format!("http://{}/repositories/acme/api/pullrequests?page=2", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme/api/pullrequests", Some("2")) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"id": 12, "title": "PR 12", "state": "OPEN"},
                        {"id": 13, "title": "PR 13", "state": "OPEN"}
                    ]
                }),
            ),
            _ => MockResponse::text(404, "unexpected route"),
        }
    });
    let client = srv.client();

    let repos = bitbucket::repo_query(
        &client,
        "acme",
        &RepoQuery {
            query: Some("api"),
            project: Some("CORE"),
            language: Some("Rust"),
            sort: "-updated_on",
            role: Some("member"),
            limit: 3,
        },
    )
    .unwrap();
    let repo_arr = repos.as_array().unwrap();
    assert_eq!(repo_arr.len(), 3);
    assert_eq!(repo_arr[0]["full_name"], "acme/r1");
    assert_eq!(repo_arr[2]["full_name"], "acme/r3");

    let prs = bitbucket::pr_query(
        &client,
        "/repositories/acme/api",
        &PrQuery {
            state: Some("open"),
            dest: Some("main"),
            limit: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let pr_arr = prs["values"].as_array().unwrap();
    assert_eq!(pr_arr.len(), 3);
    assert_eq!(prs["returned"], 3);
    assert_eq!(prs["hasNextPage"], true);
    assert_eq!(pr_arr[0]["id"], 10);
    assert_eq!(pr_arr[2]["id"], 12);

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].query_param("role").as_deref(), Some("member"));
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("3"));
    assert_eq!(reqs[2].query_param("state").as_deref(), Some("OPEN"));
    assert!(
        reqs[2]
            .query_param("q")
            .unwrap()
            .contains("destination.branch.name = \"main\"")
    );
}

#[test]
fn bitbucket_pr_get_with_comments_diff_and_statuses() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "id": 42,
                "title": "Add tests",
                "state": "OPEN",
                "draft": false,
                "author": {"display_name": "Jane"},
                "source": {"branch": {"name": "feat/tests"}},
                "destination": {"branch": {"name": "main"}},
                "updated_on": "2026-01-05",
                "links": {"html": {"href": "https://bitbucket.org/acme/api/pull-requests/42"}},
                "reviewers": [{"display_name": "Bob"}],
                "participants": [
                    {"user": {"display_name": "Bob"}, "approved": true},
                    {"user": {"display_name": "Carl"}, "approved": false}
                ],
                "comment_count": 2,
                "task_count": 0,
                "description": "Adds integration tests"
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"id": 1, "deleted": true, "user": {"display_name": "Bob"}, "content": {"raw": "deleted"}},
                    {"id": 2, "deleted": false, "user": {"display_name": "Bob"}, "created_on": "2026-01-05", "inline": {"path": "src/lib.rs", "to": 12}, "content": {"raw": "LGTM"}}
                ]
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"status": "modified", "lines_added": 20, "lines_removed": 3, "new": {"path": "src/lib.rs"}}
                ]
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"name": "CI", "key": "ci/build", "state": "SUCCESSFUL", "url": "https://ci.example/1"}
                ]
            }),
        ),
        // Also verify pr_create & pr_comment endpoints
        MockResponse::json(
            201,
            json!({"id": 43, "title": "Follow-up", "state": "OPEN"}),
        ),
        MockResponse::json(
            201,
            json!({"id": 99, "links": {"html": {"href": "https://bb/comment/99"}}}),
        ),
    ]);
    let client = srv.client();

    let out = bitbucket::pr_get(
        &client,
        "/repositories/acme/api",
        42,
        &PrGet {
            comments: true,
            diff: true,
            statuses: true,
        },
    )
    .unwrap();

    assert_eq!(out["id"], 42);
    assert_eq!(out["reviewers"][0]["display_name"], "Bob");
    assert_eq!(out["_comments"].as_array().unwrap().len(), 1);
    assert_eq!(out["_comments"][0]["inline"]["path"], "src/lib.rs");
    assert_eq!(out["_comments"][0]["inline"]["to"], 12);
    assert_eq!(out["_comments"][0]["content"]["raw"], "LGTM");
    assert_eq!(out["_diffstat"][0]["lines_added"], 20);
    assert_eq!(out["_statuses"][0]["state"], "SUCCESSFUL");

    let created = bitbucket::pr_create(
        &client,
        "/repositories/acme/api",
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

    let comment = bitbucket::pr_comment(
        &client,
        "/repositories/acme/api",
        43,
        NewComment {
            text: "Nice",
            reply_to: None,
            path: Some("README.md"),
            anchor: bitbucket::Anchor {
                line: Some(5),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(comment["id"], 99);
}

#[test]
fn bitbucket_pipeline_get_with_logs_failed_steps_and_lines() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "build_number": 1234,
                "uuid": "{pipe-uuid}",
                "state": {"name": "COMPLETED", "result": {"name": "FAILED"}},
                "target": {"ref_name": "main", "commit": {"hash": "0123456789abcdef"}},
                "trigger": {"name": "PUSH"},
                "created_on": "2026-01-05",
                "duration_in_seconds": 45
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"name": "Lint", "uuid": "{step-1}", "state": {"name": "COMPLETED", "result": {"name": "PASSED"}}, "duration_in_seconds": 10},
                    {"name": "Test", "uuid": "{step-2}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "duration_in_seconds": 35}
                ]
            }),
        ),
        MockResponse::text(200, "line1\nline2\nline3\nline4"),
    ]);
    let client = srv.client();

    let out = bitbucket::pipeline_get(
        &client,
        "/repositories/acme/api",
        "1234",
        &PipelineGet {
            logs: true,
            failed_steps: true,
            lines: 2,
            step: None,
        },
    )
    .unwrap();

    assert_eq!(out["build_number"], 1234);
    assert_eq!(out["target"]["commit"]["hash"], "0123456789abcdef");
    let steps = out["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert!(
        steps[0].get("log").is_none(),
        "passed step log should not be fetched"
    );
    assert_eq!(steps[1]["log"], "line3\nline4");
    assert_eq!(steps[1]["log_total_lines"], 4);
    assert_eq!(steps[1]["log_truncated"], true);

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        reqs[2].path,
        "/repositories/acme/api/pipelines/%7Bpipe-uuid%7D/steps/%7Bstep-2%7D/log"
    );
}

#[test]
fn bitbucket_pipeline_query_filters_client_side() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({
            "values": [
                {"build_number": 1, "uuid": "{p1}", "state": {"name": "COMPLETED", "result": {"name": "PASSED"}}, "trigger": {"name": "PUSH"}},
                {"build_number": 2, "uuid": "{p2}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "trigger": {"name": "MANUAL_TRIGGER"}},
                {"build_number": 3, "uuid": "{p3}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "trigger": {"name": "PUSH"}}
            ]
        }),
    )]);
    let client = srv.client();

    let out = bitbucket::pipeline_query(
        &client,
        "/repositories/acme/api",
        &PipelineQuery {
            pattern: None,
            state: Some("COMPLETED"),
            status: Some("FAILED"),
            branch: Some("main"),
            trigger: Some("MANUAL"),
            limit: 10,
        },
    )
    .unwrap();
    let arr = out.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["build_number"], 2);
    assert_eq!(arr[0]["_repo"], "acme/api");
    assert_eq!(
        srv.requests()[0].query_param("target.branch").as_deref(),
        Some("main")
    );
}

#[test]
fn bitbucket_branch_query_sorts_and_filters_by_author() {
    let branch = |name: &str, who: &str| json!({"name": name, "target": {"hash": "abc", "author": {"raw": format!("{who} <x@example.com>"), "user": {"display_name": who}}}});
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"values": [branch("a", "Alice Example"), branch("b", "Bob"), branch("c", "Alice Example")]}),
    )]);
    let out = bitbucket::branch_query(
        &srv.client(),
        "/repositories/acme/api",
        &bitbucket::BranchQuery {
            query: Some("feat"),
            author: Some("alice"),
            sort: Some("-target.date"),
            limit: 1,
        },
    )
    .unwrap();
    let names: Vec<_> = out.as_array().unwrap().iter().map(|b| &b["name"]).collect();
    assert_eq!(names, [&json!("a")]);
    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/repositories/acme/api/refs/branches");
    assert_eq!(reqs[0].query_param("sort").as_deref(), Some("-target.date"));
    assert_eq!(reqs[0].query_param("q").as_deref(), Some("name ~ \"feat\""));
    // With an author filter more than `limit` branches are scanned.
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("50"));
}

#[test]
fn bitbucket_task_limit_and_workspace_from_repo() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"values": [{"id": 1, "state": "UNRESOLVED"}, {"id": 2, "state": "RESOLVED"}]}),
    )]);
    let tasks = bitbucket::pr_tasks(&srv.client(), "/repositories/acme/api", 42, 1).unwrap();
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(
        srv.requests()[0].path,
        "/repositories/acme/api/pullrequests/42/tasks"
    );
    assert_eq!(
        srv.requests()[0].query_param("pagelen").as_deref(),
        Some("1")
    );

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

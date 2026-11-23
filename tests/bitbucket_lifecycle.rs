//! Offline tests for the pull request lifecycle: update, reviewers, merge
//! (with `--if-head`), checks, and the cross-repository scans.

mod common;

use atlx::bitbucket::lifecycle::{self as lc, User};
use atlx::bitbucket::{self, Paging, PrMerge, PrQuery};
use atlx::client::{ApiError, ErrorKind};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::time::Duration;

const REPO: &str = "/repositories/acme/api";

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn page(values: Value) -> MockResponse {
    ok(json!({"values": values}))
}

fn kind(e: &anyhow::Error) -> Option<ErrorKind> {
    ApiError::find(e).map(|a| a.kind)
}

fn pr() -> MockResponse {
    ok(json!({
        "id": 4, "title": "Old", "description": "Keep me", "state": "OPEN", "draft": false,
        "close_source_branch": true,
        "destination": {"branch": {"name": "main"}},
        "author": {"uuid": "{author}"},
        "reviewers": [{"uuid": "{a}", "nickname": "ann", "account_id": "1:a"},
                      {"uuid": "{b}", "nickname": "ben"}]
    }))
}

fn plain() -> PrMerge<'static> {
    PrMerge::default()
}

fn quick(timeout_ms: u64) -> Option<lc::Wait> {
    Some(lc::Wait {
        timeout: Duration::from_millis(timeout_ms),
        interval: Duration::from_millis(1),
    })
}

fn accepted() -> MockResponse {
    MockResponse::empty(202).with_header(
        "Location",
        "https://api.bitbucket.org/2.0/repositories/acme/api/pullrequests/4/merge/task-status/t-1",
    )
}

#[test]
fn update_keeps_what_it_does_not_change() {
    let srv = MockServer::with_responses([pr(), pr()]);
    let u = lc::PrUpdate {
        title: Some("New"),
        ..Default::default()
    };
    lc::pr_update(&srv.client(), REPO, 4, &u).unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.method, "PUT");
    assert_eq!(
        r.json_body(),
        json!({"title": "New", "description": "Keep me", "draft": false, "close_source_branch": true,
               "destination": {"branch": {"name": "main"}},
               "reviewers": [{"uuid": "{a}"}, {"uuid": "{b}"}]})
    );
}

#[test]
fn update_adds_removes_and_resolves_reviewers() {
    let srv = MockServer::with_responses([
        pr(),
        page(json!([{"user": {"uuid": "{c}", "display_name": "Cy Lee"}},
                    {"user": {"uuid": "{d}", "display_name": "Cy"}}])),
        pr(),
    ]);
    let u = lc::PrUpdate {
        remove: vec![User::Any("ben")],
        add: vec![User::Any("cy lee"), User::AccountId("1:a")],
        ..Default::default()
    };
    lc::pr_update(&srv.client(), REPO, 4, &u).unwrap();
    let r = srv.requests();
    assert_eq!(r[1].path, "/workspaces/acme/members");
    assert_eq!(
        r[2].json_body()["reviewers"],
        json!([{"uuid": "{a}"}, {"uuid": "{c}"}])
    );
}

#[test]
fn update_can_clear() {
    let srv = MockServer::with_responses([pr(), pr()]);
    let u = lc::PrUpdate {
        description: Some(""),
        set: Some(vec![]),
        ..Default::default()
    };
    lc::pr_update(&srv.client(), REPO, 4, &u).unwrap();
    let body = srv.requests()[1].json_body();
    assert_eq!(body["description"], "");
    assert_eq!(body["reviewers"], json!([]));
}

#[test]
fn update_writes_nothing_when_unsure() {
    let srv = MockServer::with_responses([]);
    let none = lc::PrUpdate::default();
    assert_eq!(
        kind(&lc::pr_update(&srv.client(), REPO, 4, &none).unwrap_err()),
        Some(ErrorKind::Usage)
    );
    assert!(srv.requests().is_empty());

    let srv = MockServer::with_responses([pr()]);
    let absent = lc::PrUpdate {
        remove: vec![User::Any("zed")],
        ..Default::default()
    };
    assert!(lc::pr_update(&srv.client(), REPO, 4, &absent).is_err());
    assert_eq!(srv.requests().len(), 1);

    // No reviewer list means it can't be sent back, so it would be cleared.
    let srv = MockServer::with_responses([ok(json!({"title": "Old"}))]);
    let title = lc::PrUpdate {
        title: Some("New"),
        ..Default::default()
    };
    assert!(lc::pr_update(&srv.client(), REPO, 4, &title).is_err());
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn ambiguous_names_are_refused() {
    let srv = MockServer::with_responses([page(json!([
        {"user": {"uuid": "{1}", "nickname": "sam"}}, {"user": {"uuid": "{2}", "nickname": "Sam"}}
    ]))]);
    let err = lc::resolve_user(&srv.client(), REPO, User::Any("sam")).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Usage));
}

#[test]
fn new_reviewers_add_defaults_but_never_the_author() {
    let srv = MockServer::with_responses([
        ok(json!({"uuid": "{me}"})),
        page(
            json!([{"user": {"uuid": "{me}"}}, {"user": {"uuid": "{x}"}},
                    {"user": {"uuid": "{y}", "account_id": "1:y"}}]),
        ),
    ]);
    let given = [User::AccountId("1:y")];
    let (uuids, accounts) = lc::new_reviewers(&srv.client(), REPO, &given, true).unwrap();
    assert_eq!(uuids, ["{x}"]);
    assert_eq!(accounts, ["1:y"]);
}

#[test]
fn merge_wait_polls_the_task() {
    let srv = MockServer::with_responses([
        accepted(),
        ok(json!({"task_status": "PENDING"})),
        ok(json!({"task_status": "SUCCESS", "merge_result": {"id": 4, "state": "MERGED"}})),
    ]);
    let out = lc::merge(&srv.client(), REPO, 4, &plain(), quick(5000)).unwrap();
    assert_eq!(out["state"], "MERGED");
    let r = srv.requests();
    assert_eq!(r[0].query_param("async").as_deref(), Some("true"));
    assert_eq!(
        r[2].path,
        "/repositories/acme/api/pullrequests/4/merge/task-status/t-1"
    );
}

#[test]
fn merge_wait_times_out_without_merging_again() {
    let srv = MockServer::with_responses([accepted(), ok(json!({"task_status": "PENDING"}))]);
    let err = lc::merge(&srv.client(), REPO, 4, &plain(), quick(0)).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Timeout));
    assert!(err.to_string().contains("don't merge again"), "{err}");
    let api = ApiError::find(&err).unwrap();
    assert!(api.hint.as_deref().unwrap().contains("pr get 4"));
    assert_eq!(
        srv.requests().iter().filter(|r| r.method == "POST").count(),
        1
    );
}

#[test]
fn merge_without_wait_says_it_is_merging() {
    let srv = MockServer::with_responses([accepted()]);
    let out = lc::merge(&srv.client(), REPO, 4, &plain(), None).unwrap();
    assert_eq!(out, json!({"id": 4, "state": "MERGING"}));
    assert_eq!(srv.requests()[0].query_param("async"), None);
}

#[test]
fn merge_555_checks_before_retrying() {
    let timed_out = || MockResponse::json(555, json!({"error": {"message": "timed out"}}));
    let srv = MockServer::with_responses([timed_out(), ok(json!({"id": 4, "state": "MERGED"}))]);
    assert_eq!(
        lc::merge(&srv.client(), REPO, 4, &plain(), None).unwrap()["state"],
        "MERGED"
    );
    assert_eq!(srv.requests().len(), 2);

    let srv = MockServer::with_responses([timed_out(), pr(), accepted()]);
    assert_eq!(
        lc::merge(&srv.client(), REPO, 4, &plain(), None).unwrap()["state"],
        "MERGING"
    );
    assert_eq!(
        srv.requests()[2].query_param("async").as_deref(),
        Some("true")
    );
}

fn head(hash: &str) -> MockResponse {
    ok(json!({"id": 4, "state": "OPEN", "source": {"commit": {"hash": hash}}}))
}

#[test]
fn merge_if_head_refuses_a_moved_branch_and_sends_no_merge() {
    let srv = MockServer::with_responses([head("9999999aaaaa")]);
    let o = PrMerge {
        if_head: Some("3f2a9c1b7e44d0c1"),
        ..Default::default()
    };
    let err = lc::merge(&srv.client(), REPO, 4, &o, quick(1000)).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Conflict);
    assert_eq!(api.exit_code(), 6);
    assert!(api.message.contains("9999999aaaaa"), "{}", api.message);
    assert!(api.hint.as_deref().unwrap().contains("pr get 4"));
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].method, "GET");

    // A matching head (Bitbucket's 12-character short hash) merges.
    let srv = MockServer::with_responses([
        head("3f2a9c1b7e44"),
        ok(json!({"id": 4, "state": "MERGED"})),
    ]);
    let out = lc::merge(&srv.client(), REPO, 4, &o, None).unwrap();
    assert_eq!(out["state"], "MERGED");
    assert_eq!(srv.requests()[1].method, "POST");

    // Too short to compare: refused before any request.
    let srv = MockServer::with_responses([]);
    let short = PrMerge {
        if_head: Some("3f2a"),
        ..Default::default()
    };
    let err = lc::merge(&srv.client(), REPO, 4, &short, None).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Usage));
    assert!(srv.requests().is_empty());
}

#[test]
fn checks_list_every_blocker_and_the_statuses() {
    let srv = MockServer::with_responses([
        ok(json!({
            "state": "OPEN", "author": {"uuid": "{author}"},
            "destination": {"branch": {"name": "release/1"}},
            "participants": [
                {"approved": true, "user": {"uuid": "{author}"}},
                {"approved": true, "user": {"uuid": "{a}"}},
                {"approved": false, "state": "changes_requested", "user": {"uuid": "{b}"}}]
        })),
        page(json!([{"state": "UNRESOLVED"}, {"state": "RESOLVED"}])),
        page(json!([{"state": "FAILED", "name": "CI"}, {"state": "SUCCESSFUL"}])),
        page(json!([{"status": "merge conflict"}, {"status": "modified"}])),
        page(json!([{"pattern": "release/*", "value": 2}, {"pattern": "main", "value": 5}])),
    ]);
    let out = lc::checks(&srv.client(), REPO, 4, lc::Exclude::default()).unwrap();
    assert_eq!(out["pr"], 4);
    assert_eq!(out["statuses"].as_array().unwrap().len(), 2);
    assert_eq!(out["mergeable"], false);
    assert_eq!(out["approvals"], 1);
    let kinds: Vec<&str> = out["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "changes_requested",
            "unresolved_tasks",
            "failing_status",
            "merge_conflict",
            "approvals"
        ]
    );
}

#[test]
fn checks_say_what_they_could_not_check() {
    let srv = MockServer::with_responses([
        pr(),
        page(json!([])),
        page(json!([])),
        MockResponse::json(403, json!({"error": {"message": "admin only"}})),
    ]);
    let x = lc::Exclude { conflicts: true };
    let out = lc::checks(&srv.client(), REPO, 4, x).unwrap();
    assert_eq!(out["mergeable"], true);
    let kinds: Vec<&str> = out["unchecked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["approvals", "maximum_commits_behind", "merge_conflict"]
    );
}

#[test]
fn conflicts_keep_only_conflicting_files() {
    let srv = MockServer::with_responses([page(json!([
        {"status": "modified"}, {"status": "merge conflict"}
    ]))]);
    let (out, next) =
        bitbucket::pr_diffstat(&srv.client(), REPO, 4, true, &Paging::new(100)).unwrap();
    assert_eq!(out, [json!({"status": "merge conflict"})]);
    assert_eq!(next, None);
}

#[test]
fn for_commit_explains_indexing() {
    let srv = MockServer::with_responses([MockResponse::json(202, json!({}))]);
    let err = lc::for_commit(&srv.client(), REPO, "abc123", &Paging::new(25)).unwrap_err();
    assert!(err.to_string().contains("indexing"), "{err}");

    let srv = MockServer::with_responses([page(json!([{"id": 4}]))]);
    assert_eq!(
        lc::for_commit(&srv.client(), REPO, "abc123", &Paging::new(25))
            .unwrap()
            .0[0]["id"],
        4
    );
    assert_eq!(
        srv.requests()[0].path,
        "/repositories/acme/api/commit/abc123/pullrequests"
    );
}

#[test]
fn list_takes_reviewer_and_a_cursor() {
    let srv = MockServer::with_responses([ok(
        json!({"values": [{"id": 1}], "next": "https://api.bitbucket.org/2.0/x?pagelen=10&page=4"}),
    )]);
    let q = PrQuery {
        reviewer: Some("{a}"),
        ..Default::default()
    };
    let p = Paging {
        limit: 1,
        cursor: Some("t:10:3".into()),
    };
    let (items, next) = bitbucket::pr_list(&srv.client(), REPO, &q, &p).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(next.as_deref(), Some("t:10:4"));
    let r = &srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].query_param("page").as_deref(), Some("3"));
    assert_eq!(r[0].query_param("pagelen").as_deref(), Some("10"));
    assert_eq!(
        r[0].query_param("q").as_deref(),
        Some("reviewers.uuid = \"{a}\"")
    );
}

#[test]
fn merged_filters_use_the_merge_event() {
    let event =
        |date: &str| page(json!([{"approval": {}}, {"update": {"state": "MERGED", "date": date}}]));
    let srv = MockServer::with_responses([
        page(json!([{"id": 1}, {"id": 2}])),
        event("2026-03-05T10:00:00+00:00"),
        event("2026-01-05T10:00:00+00:00"),
    ]);
    let q = PrQuery::default();
    let (items, next) = lc::merged(
        &srv.client(),
        REPO,
        &q,
        Some("2026-03-01"),
        None,
        &Paging::new(10),
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], 1);
    assert_eq!(items[0]["merged_on"], "2026-03-05T10:00:00+00:00");
    assert_eq!(next, None);
    let r = srv.requests();
    assert_eq!(r[0].query_param("state").as_deref(), Some("MERGED"));
    assert_eq!(
        r[0].query_param("q").as_deref(),
        Some("updated_on >= 2026-03-01")
    );
}

#[test]
fn inbox_keeps_what_waits_on_me() {
    let me = json!({"uuid": "{me}"});
    let srv = MockServer::with_responses([
        ok(me.clone()),
        page(json!([{"full_name": "acme/api"}])),
        page(json!([
            {"id": 1, "reviewers": [me.clone()], "participants": []},
            {"id": 2, "reviewers": [me.clone()], "participants": [{"user": me.clone(), "approved": true}]},
            {"id": 3, "author": me.clone(), "participants": [{"state": "changes_requested"}]},
            {"id": 4, "author": me.clone(), "participants": []}
        ])),
    ]);
    let o = lc::Mine {
        workspace: "acme",
        repo: None,
        role: None,
        waiting: true,
        state: Some("open"),
        limit: 25,
        repo_limit: 25,
        concurrency: 1,
    };
    let out = lc::mine(&srv.client(), &o).unwrap();
    assert!(!out.more);
    assert_eq!(out.repos_scanned, 1);
    let mut ids: Vec<u64> = out
        .items
        .iter()
        .map(|p| p["id"].as_u64().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, [1, 3]);
    let r = srv.requests();
    assert_eq!(r[2].query_param("state").as_deref(), Some("OPEN"));
    assert_eq!(
        r[2].query_param("fields").as_deref(),
        Some("+values.participants,+values.reviewers")
    );
}

#[test]
fn default_reviewers_and_request_changes() {
    let srv = MockServer::with_responses([
        page(json!([])),
        page(json!([])),
        ok(json!({})),
        ok(json!({})),
        MockResponse::empty(204),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    lc::default_reviewers(&c, REPO, false, &Paging::new(50)).unwrap();
    lc::default_reviewers(&c, REPO, true, &Paging::new(50)).unwrap();
    lc::default_reviewer_get(&c, REPO, "abc").unwrap();
    lc::default_reviewer_add(&c, REPO, "{abc}").unwrap();
    lc::default_reviewer_remove(&c, REPO, "abc").unwrap();
    lc::remove_request_changes(&c, REPO, 4).unwrap();
    let r = srv.requests();
    assert!(r[1].path.ends_with("/effective-default-reviewers"));
    assert!(
        r[2].path.ends_with("/default-reviewers/%7Babc%7D"),
        "{}",
        r[2].path
    );
    assert_eq!(r[3].method, "PUT");
    assert_eq!(r[4].method, "DELETE");
    assert!(r[5].path.ends_with("/pullrequests/4/request-changes"));
    assert!(lc::default_reviewer_remove(&c, REPO, " ").is_err());
}

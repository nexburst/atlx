//! Offline tests for Jira bulk commands, properties and user lookups.

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::bulk::{self, Fetch, Who};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn fetch_batches_by_hundred_and_keeps_the_order_asked() {
    let ids: Vec<String> = (1..=101).rev().map(|i| i.to_string()).collect();
    let issues = |from: usize, to: usize| -> Vec<Value> {
        (from..=to)
            .map(|i| json!({"id": i.to_string(), "key": format!("P-{i}"), "fields": {}}))
            .collect()
    };
    let srv = MockServer::with_responses([
        ok(json!({"issues": issues(2, 101)})),
        ok(json!({"issues": issues(1, 1)})),
    ]);
    let out = bulk::fetch(&srv.client(), &ids, &Fetch::default()).unwrap();
    assert_eq!(out["issues"][0]["id"], "101");
    assert_eq!(out["issues"][100]["id"], "1");
    assert_eq!(out["missing"], json!([]));
    let r = srv.requests();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].path, "/rest/api/3/issue/bulkfetch");
    assert_eq!(
        r[0].json_body()["issueIdsOrKeys"].as_array().unwrap().len(),
        100
    );
}

#[test]
fn fetch_names_what_is_missing_but_not_what_moved() {
    let srv = MockServer::with_responses([
        ok(json!({"issues": [{"id": "1", "key": "P-1", "fields": {}, "properties": {"k": 2}}]})),
        ok(json!({"issues": [{"id": "9", "key": "NEW-9", "fields": {}}]})),
    ]);
    let c = srv.client();
    let out = bulk::fetch(&c, &strings(&["P-1", "P-2"]), &Fetch::default()).unwrap();
    assert_eq!(out["missing"], json!(["P-2"]));
    assert_eq!(out["issues"][0]["properties"]["k"], 2);
    let moved = bulk::fetch(&c, &strings(&["OLD-3"]), &Fetch::default()).unwrap();
    assert_eq!(moved["missing"], json!([]));
}

#[test]
fn create_checks_every_item_first() {
    let item = json!({"fields": {"summary": "s"}});
    assert!(is_usage(
        &bulk::create_input(json!([item.clone(), {"summary": "bare"}])).unwrap_err()
    ));
    assert!(bulk::create_input(Value::Array(vec![item.clone(); 51])).is_err());
    let items = bulk::create_input(json!({"issueUpdates": [item]})).unwrap();
    let srv = MockServer::with_responses([MockResponse::json(
        201,
        json!({"issues": [{"id": "1", "key": "P-1"}], "errors": []}),
    )]);
    let out = bulk::create(&srv.client(), &items).unwrap();
    assert_eq!(out["issues"][0]["key"], "P-1");
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/issue/bulk");
    assert_eq!(
        r.json_body(),
        json!({"issueUpdates": [{"fields": {"summary": "s"}}]})
    );
}

#[test]
fn transition_dry_run_sends_nothing() {
    let srv = MockServer::with_responses([MockResponse::json(201, json!({"taskId": "t-1"}))]);
    let c = srv.client();
    let ids = strings(&["P-1", "p-1", "P-2"]);
    let dry = bulk::transition(&c, &ids, "31", false, true).unwrap();
    assert!(srv.requests().is_empty());
    let out = bulk::transition(&c, &ids, "31", false, false).unwrap();
    assert_eq!(out["taskId"], "t-1");
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/bulk/issues/transition");
    assert_eq!(r.json_body(), dry["request"]);
    assert_eq!(
        r.json_body(),
        json!({"bulkTransitionInputs": [{"selectedIssueIdsOrKeys": ["P-1", "P-2"],
                "transitionId": "31"}],
               "sendBulkNotification": false})
    );
    assert!(is_usage(
        &bulk::transition(&c, &ids, "Done", false, true).unwrap_err()
    ));
}

#[test]
fn archive_and_unarchive() {
    let srv = MockServer::with_responses([
        ok(json!({"numberOfIssuesUpdated": 1})),
        ok(json!({"numberOfIssuesUpdated": 1})),
    ]);
    let c = srv.client();
    bulk::archive(&c, &strings(&["P-1"]), true).unwrap();
    bulk::archive(&c, &strings(&["P-1"]), false).unwrap();
    assert!(is_usage(
        &bulk::archive(&c, &strings(&[" "]), true).unwrap_err()
    ));
    let r = srv.requests();
    assert_eq!(r.len(), 2);
    assert_eq!(
        (r[0].method.as_str(), r[0].path.as_str()),
        ("PUT", "/rest/api/3/issue/archive")
    );
    assert_eq!(r[1].path, "/rest/api/3/issue/unarchive");
}

#[test]
fn properties_round_trip_and_cannot_reach_the_issue() {
    let srv = MockServer::with_responses([
        ok(json!({"keys": []})),
        ok(json!({"key": "a b", "value": 1})),
        MockResponse::empty(201),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    bulk::properties(&c, "P-1").unwrap();
    bulk::property(&c, "P-1", "a b").unwrap();
    bulk::property_set(&c, "P-1", "k", &json!([1, "two"])).unwrap();
    bulk::property_delete(&c, "P-1", "k").unwrap();
    for key in ["", "..", "."] {
        assert!(bulk::property_delete(&c, "P-1", key).is_err(), "{key:?}");
    }
    assert!(bulk::property_delete(&c, "", "k").is_err());
    assert!(bulk::property_delete(&c, "..", "k").is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 4);
    assert_eq!(r[1].path, "/rest/api/3/issue/P-1/properties/a%20b");
    assert_eq!(r[2].json_body(), json!([1, "two"]));
    assert_eq!(r[3].method, "DELETE");
}

#[test]
fn lookup_takes_only_exact_emails() {
    let srv = MockServer::with_responses([
        ok(json!({"values": [{"accountId": "abc", "displayName": "Ann"}]})),
        ok(json!([{"accountId": "x", "emailAddress": "ann@example.com.au"}])),
        ok(json!([{"accountId": "y", "emailAddress": "Bo@Example.com"}])),
    ]);
    let who = [
        Who::Account("ari:cloud:identity::user/abc".into()),
        Who::Email("ann@example.com".into()),
        Who::Email("bo@example.com".into()),
    ];
    let out = bulk::lookup(&srv.client(), &who).unwrap();
    assert_eq!(out["items"][0]["user"]["accountId"], "abc");
    assert_eq!(out["items"][1]["found"], false);
    assert_eq!(out["items"][2]["user"]["accountId"], "y");
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/user/bulk");
    assert_eq!(r.query_param("accountId").as_deref(), Some("abc"));
}

#[test]
fn user_defaults_to_the_caller() {
    let srv = MockServer::with_responses([
        ok(json!({"accountId": "me"})),
        ok(json!({"accountId": "abc", "displayName": "Ann"})),
    ]);
    let c = srv.client();
    assert_eq!(bulk::user(&c, None).unwrap()["accountId"], "me");
    assert_eq!(bulk::user(&c, Some("abc")).unwrap()["name"], "Ann");
    assert_eq!(srv.requests()[0].path, "/rest/api/3/myself");
}

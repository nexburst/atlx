//! Offline tests for multi-issue reads and writes, properties and user
//! lookups.

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::{self, bulk};
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
fn issues_batch_by_hundred_as_reads_and_keep_the_order_asked() {
    let ids: Vec<String> = (1..=101).rev().map(|i| format!("P-{i}")).collect();
    let issues = |from: usize, to: usize| -> Vec<Value> {
        (from..=to)
            .map(|i| json!({"id": i.to_string(), "key": format!("P-{i}"), "fields": {}}))
            .collect()
    };
    let srv = MockServer::with_responses([
        ok(json!({"issues": issues(2, 101), "names": {"customfield_1": "Points"}})),
        ok(json!({"issues": issues(1, 1)})),
    ]);
    let c = srv.client().with_mode(atlx::client::Mode::ReadOnly);
    let (found, missing) = jira::issues(&c, &ids).unwrap();
    assert_eq!(found[0]["key"], "P-101");
    assert_eq!(found[100]["key"], "P-1");
    assert_eq!(found[0]["names"]["customfield_1"], "Points");
    assert!(missing.is_empty());
    let r = srv.requests();
    assert_eq!(r.len(), 2, "bulkfetch is a read, allowed in read-only mode");
    assert_eq!(r[0].path, "/rest/api/3/issue/bulkfetch");
    assert_eq!(
        r[0].json_body()["issueIdsOrKeys"].as_array().unwrap().len(),
        100
    );
    assert_eq!(r[0].json_body()["expand"], json!(["names"]));
}

#[test]
fn issues_name_what_is_missing_and_match_one_moved_issue() {
    let srv = MockServer::with_responses([
        ok(json!({"issues": [{"id": "1", "key": "P-1", "fields": {}}]})),
        ok(json!({"issues": [{"id": "9", "key": "NEW-9", "fields": {}}]})),
    ]);
    let c = srv.client();
    let (found, missing) = jira::issues(&c, &strings(&["P-1", "P-2"])).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(missing, ["P-2"]);
    let (moved, missing) = jira::issues(&c, &strings(&["OLD-3"])).unwrap();
    assert_eq!(moved[0]["key"], "NEW-9");
    assert!(missing.is_empty());
}

#[test]
fn create_checks_every_item_first_and_names_failures() {
    let item = json!({"fields": {"summary": "s"}});
    assert!(is_usage(
        &bulk::create_input(json!([item.clone(), {"summary": "bare"}])).unwrap_err()
    ));
    assert!(bulk::create_input(Value::Array(vec![item.clone(); 51])).is_err());
    let items = bulk::create_input(json!({"issueUpdates": [item.clone(), item]})).unwrap();
    let srv = MockServer::with_responses([MockResponse::json(
        201,
        json!({"issues": [{"id": "1", "key": "P-1"}], "errors": [
            {"failedElementNumber": 1, "status": 400,
             "elementErrors": {"errorMessages": [], "errors": {"summary": "too long"}}}
        ]}),
    )]);
    let (created, failed) = bulk::create(&srv.client(), &items).unwrap();
    assert_eq!(created[0]["key"], "P-1");
    assert_eq!(failed[0].0, "#2");
    assert_eq!(failed[0].1.message, "summary: too long");
    assert_eq!(failed[0].1.fields, Some(json!({"summary": "too long"})));
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/issue/bulk");
}

#[test]
fn archive_and_restore_report_failed_keys() {
    let srv = MockServer::with_responses([
        ok(json!({"numberOfIssuesUpdated": 1, "errors": {
            "issueIsSubtask": {"count": 1, "issueIdsOrKeys": ["P-2"], "message": "subtasks can't be archived"}
        }})),
        ok(json!({"numberOfIssuesUpdated": 1})),
    ]);
    let c = srv.client();
    let (done, failed) = bulk::archive(&c, &strings(&["P-1", "P-2"]), true).unwrap();
    assert_eq!(done, [json!({"key": "P-1", "archived": true})]);
    assert_eq!(failed[0].0, "P-2");
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
        ok(json!({"keys": [{"key": "a"}]})),
        ok(json!({"key": "a b", "value": 1})),
        MockResponse::empty(201),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    assert_eq!(bulk::properties(&c, "P-1").unwrap()[0]["key"], "a");
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
fn user_get_takes_me_ids_aris_and_only_exact_emails() {
    let srv = MockServer::with_responses([
        ok(json!({"accountId": "me-1"})),
        ok(json!({"accountId": "abc", "displayName": "Ann"})),
        ok(json!([{"accountId": "x", "emailAddress": "ann@example.com.au"}])),
        ok(json!([{"accountId": "y", "emailAddress": "Bo@Example.com"}])),
    ]);
    let c = srv.client();
    assert_eq!(jira::user_get(&c, "me").unwrap()["accountId"], "me-1");
    assert_eq!(
        jira::user_get(&c, "ari:cloud:identity::user/abc").unwrap()["displayName"],
        "Ann"
    );
    let e = jira::user_get(&c, "ann@example.com").unwrap_err();
    assert_eq!(ApiError::find(&e).unwrap().kind, ErrorKind::NotFound);
    assert_eq!(
        jira::user_get(&c, "bo@example.com").unwrap()["accountId"],
        "y"
    );
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/myself");
    assert_eq!(r[1].query_param("accountId").as_deref(), Some("abc"));
    assert_eq!(r[2].path, "/rest/api/3/user/search");
}

#[test]
fn user_search_pages_by_offset() {
    let srv = MockServer::with_responses([ok(json!([{"accountId": "a"}, {"accountId": "b"}]))]);
    let (items, next) = jira::user_search(&srv.client(), "ann", 2, None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(next.as_deref(), Some("o:2"), "a full page may have more");
    let r = &srv.requests()[0];
    assert_eq!(r.query_param("query").as_deref(), Some("ann"));
    assert_eq!(r.query_param("maxResults").as_deref(), Some("2"));
    assert!(jira::user_search(&srv.client(), " ", 2, None).is_err());
}

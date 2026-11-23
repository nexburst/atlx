//! Offline tests for Jira Service Management (#37).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jsm;
use common::{MockResponse, MockServer};
use serde_json::{Map, Value, json};

const ACCOUNT: &str = "5b10ac8d82e05b22cc7d4ef5";

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn page(values: Value) -> MockResponse {
    ok(json!({"values": values, "isLastPage": true}))
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

fn fields() -> Value {
    json!({"requestTypeFields": [
        {"fieldId": "summary", "name": "Summary", "required": true, "jiraSchema": {"type": "string"}},
        {"fieldId": "customfield_1", "name": "Model", "required": true, "jiraSchema": {"type": "option"},
         "validValues": [{"value": "101", "label": "Pro"}, {"value": "102", "label": "Air"}]},
        {"fieldId": "customfield_2", "name": "Seats", "required": false, "jiraSchema": {"type": "number"}},
        {"fieldId": "customfield_3", "name": "Extras", "required": false,
         "jiraSchema": {"type": "array", "items": "option"},
         "validValues": [{"value": "201", "label": "Dock"}]},
        {"fieldId": "priority", "name": "Priority", "required": true, "defaultValues": [{"value": "3"}]}
    ]})
}

#[test]
fn request_keys_never_come_from_portal_numbers() {
    for (good, want) in [
        ("SD-5", "SD-5"),
        ("10005", "10005"),
        ("https://x.test/browse/SD-5", "SD-5"),
        (
            "https://x.test/servicedesk/customer/portal/1/SD-5?created=true",
            "SD-5",
        ),
        ("ari:cloud:jira:site:issue/10005", "10005"),
    ] {
        assert_eq!(jsm::request_key(good).unwrap(), want);
    }
    for bad in [
        "https://x.test/servicedesk/customer/portal/1",
        "https://x.test/jira/servicedesk/projects/SD/queues/custom/4",
        "https://x.test/somewhere/SD-5",
        "SD",
        "..",
        "",
    ] {
        assert!(is_usage(&jsm::request_key(bad).unwrap_err()), "{bad}");
    }
}

#[test]
fn desks_resolve_by_key_then_exact_name() {
    let list = || {
        page(json!([
            {"id": "1", "projectKey": "IT", "projectId": "10000", "projectName": "Help"},
            {"id": "2", "projectKey": "HR", "projectId": "10001", "projectName": "IT"},
            {"id": "3", "projectKey": "FAC", "projectId": "10002", "projectName": "Help"}
        ]))
    };
    let srv = MockServer::with_responses([list(), list(), list()]);
    let c = srv.client();
    assert_eq!(jsm::resolve_desk(&c, "it").unwrap(), "1");
    assert!(is_usage(&jsm::resolve_desk(&c, "Help").unwrap_err()));
    assert_eq!(jsm::desk_of_project(&c, "10001").unwrap(), "2");
    assert_eq!(
        jsm::resolve_desk(&c, "https://x.test/servicedesk/customer/portal/7").unwrap(),
        "7"
    );
    assert_eq!(srv.requests().len(), 3);
}

#[test]
fn a_missing_request_type_lists_the_choices() {
    let types = || page(json!([{"id": "10", "name": "Laptop"}, {"id": "11", "name": "VPN"}]));
    let srv = MockServer::with_responses([types(), types()]);
    let c = srv.client();
    let err = jsm::resolve_type(&c, "1", None).unwrap_err();
    assert!(err.to_string().contains("11 (VPN)"), "{err}");
    assert_eq!(jsm::resolve_type(&c, "1", Some("vpn")).unwrap(), "11");
}

#[test]
fn lists_follow_short_pages_until_the_last() {
    let srv = MockServer::with_responses([
        ok(json!({"values": [{"id": "1"}], "isLastPage": false})),
        ok(json!({"values": [{"id": "2"}], "isLastPage": true})),
    ]);
    let (items, next) = jsm::desks(&srv.client(), None, 10, None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(next, None);
    assert_eq!(srv.requests()[1].query_param("start").as_deref(), Some("1"));

    let srv =
        MockServer::with_responses([ok(json!({"values": [{"id": "1"}], "isLastPage": false}))]);
    assert_eq!(
        jsm::desks(&srv.client(), None, 1, None)
            .unwrap()
            .1
            .as_deref(),
        Some("o:1")
    );
}

#[test]
fn create_shapes_answers_by_the_fields() {
    let srv = MockServer::with_responses([ok(fields()), ok(json!({"issueKey": "SD-5"}))]);
    let v = jsm::Values {
        summary: Some("Laptop"),
        answers: vec![
            ("model".into(), "pro".into()),
            ("Seats".into(), "2".into()),
            ("Extras".into(), "Dock".into()),
        ],
        ..Default::default()
    };
    jsm::create(&srv.client(), "1", "10", &v, Some(ACCOUNT)).unwrap();
    let r = srv.requests();
    assert_eq!(
        r[0].path,
        "/rest/servicedeskapi/servicedesk/1/requesttype/10/field"
    );
    assert_eq!(r[1].path, "/rest/servicedeskapi/request");
    assert_eq!(
        r[1].json_body(),
        json!({"serviceDeskId": "1", "requestTypeId": "10", "raiseOnBehalfOf": ACCOUNT,
               "requestFieldValues": {"summary": "Laptop", "customfield_1": {"id": "101"},
                                      "customfield_2": 2, "customfield_3": [{"id": "201"}]}})
    );
}

#[test]
fn create_sends_nothing_invalid_and_validate_never_posts() {
    let mut raw = Map::new();
    raw.insert("customfield_9".into(), "x".into());
    let v = jsm::Values {
        answers: vec![("Model".into(), "Max".into()), ("Nope".into(), "x".into())],
        fields: raw,
        ..Default::default()
    };
    let srv = MockServer::with_responses([ok(fields()), ok(fields())]);
    let c = srv.client();
    assert!(is_usage(&jsm::create(&c, "1", "10", &v, None).unwrap_err()));
    let out = jsm::validate(&c, "1", "10", &v).unwrap();
    assert_eq!(out["valid"], false);
    let problems = out["problems"].to_string();
    for part in [
        "Max",
        "Nope",
        "customfield_9",
        "(summary) is required",
        "(customfield_1) is required",
    ] {
        assert!(problems.contains(part), "{part}: {problems}");
    }
    assert!(
        !problems.contains("(priority)"),
        "a default satisfies a required field"
    );
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn example_answers_use_labels() {
    let out = jsm::example_answers(&fields()).unwrap();
    assert_eq!(out["Model"], "Pro");
    assert_eq!(out["Extras"], json!(["Dock"]));
}

#[test]
fn comments_always_state_who_sees_them() {
    let srv = MockServer::with_responses([ok(json!({})), ok(json!({}))]);
    let c = srv.client();
    jsm::comment_create(&c, "SD-5", "note", true).unwrap();
    jsm::comment_create(&c, "SD-5", "hello", false).unwrap();
    assert!(jsm::comment_create(&c, "SD-5", " ", true).is_err());
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/servicedeskapi/request/SD-5/comment");
    assert_eq!(r[0].json_body(), json!({"body": "note", "public": false}));
    assert_eq!(r[1].json_body(), json!({"body": "hello", "public": true}));
}

#[test]
fn participants_send_account_ids() {
    let srv = MockServer::with_responses([ok(json!({})), ok(json!({}))]);
    let c = srv.client();
    let who = [ACCOUNT.to_string()];
    jsm::participants_change(&c, "SD-5", &who, true).unwrap();
    jsm::participants_change(&c, "SD-5", &who, false).unwrap();
    assert!(jsm::participants_change(&c, "SD-5", &[], false).is_err());
    assert!(jsm::participants_change(&c, "SD-5", &[" ".into()], false).is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 2);
    assert_eq!(r[1].method, "DELETE");
    assert_eq!(r[1].json_body(), json!({"accountIds": [ACCOUNT]}));
}

#[test]
fn approvals_pick_only_an_unambiguous_one() {
    let a = |id: &str, decision: &str, can: bool| json!({"id": id, "finalDecision": decision, "canAnswerApproval": can});
    let srv = MockServer::with_responses([
        page(json!([
            a("1", "approved", true),
            a("2", "pending", false),
            a("3", "pending", true)
        ])),
        ok(json!({})),
    ]);
    jsm::approval_decide(&srv.client(), "SD-5", None, false).unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.path, "/rest/servicedeskapi/request/SD-5/approval/3");
    assert_eq!(r.json_body(), json!({"decision": "decline"}));

    let srv = MockServer::with_responses([page(json!([
        a("3", "pending", true),
        a("4", "pending", true)
    ]))]);
    assert!(is_usage(
        &jsm::approval_decide(&srv.client(), "SD-5", None, true).unwrap_err()
    ));
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn transitions_match_exactly() {
    let list =
        || page(json!([{"id": "7", "name": "Resolve"}, {"id": "8", "name": "Resolve later"}]));
    let srv = MockServer::with_responses([list(), MockResponse::empty(204), list()]);
    let c = srv.client();
    jsm::transition(&c, "SD-5", "resolve", Some("Done")).unwrap();
    assert!(is_usage(
        &jsm::transition(&c, "SD-5", "Res", None).unwrap_err()
    ));
    assert_eq!(
        srv.requests()[1].json_body(),
        json!({"id": "7", "additionalComment": {"body": "Done"}})
    );
}

#[test]
fn request_query_maps_status() {
    let srv = MockServer::with_responses([page(json!([]))]);
    let o = jsm::RequestQuery {
        status: Some("open"),
        desk: Some("1"),
        ..Default::default()
    };
    jsm::requests(&srv.client(), &o, 10, None).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(
        r.query_param("requestStatus").as_deref(),
        Some("OPEN_REQUESTS")
    );
    assert_eq!(r.query_param("serviceDeskId").as_deref(), Some("1"));
}

#[test]
fn read_endpoints() {
    let srv = MockServer::with_responses([
        page(json!([])),
        page(json!([])),
        page(json!([])),
        page(json!([{"id": "9", "name": "Time to resolution"}])),
        page(json!([])),
        ok(json!({})),
        page(json!([])),
    ]);
    let c = srv.client();
    jsm::request_types(&c, Some("1"), Some("vpn"), 50, None).unwrap();
    jsm::queues(&c, "1", 50, None).unwrap();
    jsm::queue_issues(&c, "1", "4", 50, None).unwrap();
    assert_eq!(
        jsm::sla(&c, "SD-5", "time to resolution").unwrap()["id"],
        "9"
    );
    jsm::articles(&c, Some("1"), "vpn", 50, None).unwrap();
    jsm::get(&c, "SD-5").unwrap();
    jsm::comments(&c, "SD-5", 50, None).unwrap();
    let r = srv.requests();
    let paths: Vec<&str> = r
        .iter()
        .map(|x| x.path.trim_start_matches("/rest/servicedeskapi"))
        .collect();
    assert_eq!(
        paths,
        [
            "/servicedesk/1/requesttype",
            "/servicedesk/1/queue",
            "/servicedesk/1/queue/4/issue",
            "/request/SD-5/sla",
            "/servicedesk/1/knowledgebase/article",
            "/request/SD-5",
            "/request/SD-5/comment"
        ]
    );
    assert_eq!(r[4].header("x-experimentalapi"), Some("opt-in"));
}

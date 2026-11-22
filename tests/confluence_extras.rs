//! Offline tests for Confluence tasks, states, watches, templates and space
//! permissions.

mod common;

use atlx::confluence::extras::{self, Scope, TaskFilter, TemplateQuery, TemplateType};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

#[test]
fn tasks_find_the_type_and_follow_the_cursor() {
    let srv = MockServer::with_responses([
        MockResponse::json(404, json!({"errors": [{"status": 404}]})),
        ok(json!({"id": "7"})),
        ok(json!({"results": [{"id": "1"}], "_links": {"next": "/wiki/api/v2/tasks?cursor=c2"}})),
        ok(json!({"results": [{"id": "2"}]})),
    ]);
    let out = extras::tasks(&srv.client(), Some("7"), None, TaskFilter::Open, 10).unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 2);
    assert_eq!(out["nextCursor"], Value::Null);
    let r = srv.requests();
    assert_eq!(r[2].path, "/api/v2/tasks");
    assert_eq!(r[2].query_param("blogpost-id").as_deref(), Some("7"));
    assert_eq!(r[2].query_param("page-id"), None);
    assert_eq!(r[2].query_param("status").as_deref(), Some("incomplete"));
    assert_eq!(r[3].query_param("cursor").as_deref(), Some("c2"));
}

#[test]
fn tasks_complete_and_reopen() {
    let srv = MockServer::with_responses([ok(json!({"id": "4"})), ok(json!({"id": "4"}))]);
    let c = srv.client();
    extras::task_set(&c, "4", true).unwrap();
    extras::task_set(&c, "4", false).unwrap();
    assert!(extras::task_set(&c, "..", true).is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 2);
    assert_eq!(
        (r[0].method.as_str(), r[0].path.as_str()),
        ("PUT", "/api/v2/tasks/4")
    );
    assert_eq!(r[0].json_body(), json!({"status": "complete"}));
    assert_eq!(r[1].json_body(), json!({"status": "incomplete"}));
}

#[test]
fn states_need_an_id_or_a_name_and_color() {
    assert_eq!(
        extras::state_input(Some(2), None, None).unwrap(),
        json!({"id": 2})
    );
    let named = extras::state_input(None, Some("Review"), Some("ffc400")).unwrap();
    assert_eq!(named, json!({"name": "Review", "color": "#ffc400"}));
    assert!(extras::state_input(None, Some("Review"), Some("yellow")).is_err());
    assert!(extras::state_input(None, Some(&"x".repeat(21)), Some("#ffc400")).is_err());
    assert!(extras::state_input(None, None, None).is_err());

    let srv = MockServer::with_responses([ok(json!({})), ok(json!({})), ok(json!({}))]);
    let c = srv.client();
    extras::state(&c, "5", false).unwrap();
    extras::state_set(&c, "5", true, &named).unwrap();
    extras::states_available(&c, "5").unwrap();
    let r = srv.requests();
    assert_eq!(r[0].query_param("status").as_deref(), Some("current"));
    assert_eq!(r[1].query_param("status").as_deref(), Some("draft"));
    assert_eq!(r[1].json_body(), named);
    assert_eq!(r[2].path, "/rest/api/content/5/state/available");
}

#[test]
fn watches_send_the_xsrf_header_and_a_space_key() {
    let srv = MockServer::with_responses([
        MockResponse::empty(204),
        ok(json!({"id": "55", "key": "ENG"})),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    extras::watch_content(&c, "5", true).unwrap();
    assert_eq!(extras::watch_space(&c, "55", false).unwrap()["key"], "ENG");
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/user/watch/content/5");
    assert_eq!(r[0].header("x-atlassian-token"), Some("no-check"));
    assert_eq!(r[2].method, "DELETE");
    assert_eq!(r[2].path, "/rest/api/user/watch/space/ENG");
}

#[test]
fn templates_merge_scopes_then_filter_and_page() {
    let t = |id: &str, name: &str| json!({"templateId": id, "name": name});
    let srv = MockServer::with_responses([
        ok(json!({"results": [t("1", "Retro"), t("2", "Plan")]})),
        ok(json!({"results": [t("2", "Plan"), t("3", "Retro long")]})),
    ]);
    let out = extras::templates(
        &srv.client(),
        &TemplateQuery {
            space: Some("ENG"),
            scope: Some(Scope::Available),
            kind: TemplateType::Page,
            text: Some("retro"),
            start: 0,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(out["results"], json!([t("1", "Retro")]));
    assert_eq!(out["nextCursor"], "1");
    let r = srv.requests();
    assert_eq!(r[0].query_param("spaceKey").as_deref(), Some("ENG"));
    assert_eq!(r[1].query_param("spaceKey"), None);
}

#[test]
fn template_and_permissions() {
    let srv = MockServer::with_responses([
        ok(json!({"templateId": "9"})),
        ok(json!({"results": []})),
        ok(json!({"results": []})),
    ]);
    let c = srv.client();
    extras::template(&c, "9").unwrap();
    extras::permissions(&c, Some("55"), 25, None).unwrap();
    extras::permissions(&c, None, 25, Some("next")).unwrap();
    assert!(extras::template(&c, "..").is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].query_param("expand").as_deref(), Some("body.storage"));
    assert_eq!(r[1].path, "/api/v2/spaces/55/permissions");
    assert_eq!(r[2].path, "/api/v2/space-permissions");
}

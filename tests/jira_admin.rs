//! Offline tests for spaces, versions, components and filters (#26).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::admin;
use common::{MockResponse, MockServer};
use serde_json::{Map, Value, json};

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn empty() -> MockResponse {
    MockResponse::empty(204)
}

fn map(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

#[test]
fn destructive_space_commands_refuse_anything_but_an_exact_space() {
    for bad in [
        "PROJ-1",
        "https://x.atlassian.net/browse/PROJ-1",
        "ari:cloud:jira:site:issue/10001",
        "My Project",
        "https://x.atlassian.net/issues/?jql=project=PROJ",
        "..",
        "",
        "12abc",
    ] {
        let srv = MockServer::with_responses([]);
        let c = srv.client();
        assert!(
            is_usage(&admin::space_delete(&c, bad).unwrap_err()),
            "{bad}"
        );
        assert!(
            is_usage(&admin::space_archive(&c, bad).unwrap_err()),
            "{bad}"
        );
        assert!(srv.requests().is_empty(), "{bad} was sent");
    }
    for (good, want) in [
        ("PROJ", "PROJ"),
        ("10000", "10000"),
        ("ari:cloud:jira:site:project/10000", "10000"),
        (
            "https://x.atlassian.net/jira/software/projects/PROJ/boards/1",
            "PROJ",
        ),
    ] {
        assert_eq!(admin::exact_space(good).unwrap(), want);
    }
}

#[test]
fn space_lifecycle_endpoints() {
    let srv = MockServer::with_responses([
        ok(json!({})),
        ok(json!({})),
        empty(),
        empty(),
        ok(json!({})),
    ]);
    let c = srv.client();
    admin::space_create(&c, map(json!({"key": "NEW", "name": "New"}))).unwrap();
    admin::space_update(&c, "PROJ", map(json!({"name": "P"})), Some("lead")).unwrap();
    admin::space_delete(&c, "PROJ").unwrap();
    admin::space_archive(&c, "PROJ").unwrap();
    admin::space_restore(&c, "PROJ").unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/project");
    assert_eq!(r[1].query_param("expand").as_deref(), Some("lead"));
    assert_eq!(r[2].method, "DELETE");
    assert_eq!(r[2].query_param("enableUndo").as_deref(), Some("true"));
    assert_eq!(r[3].path, "/rest/api/3/project/PROJ/archive");
    assert_eq!(r[4].path, "/rest/api/3/project/PROJ/restore");
}

#[test]
fn space_lookups() {
    let srv =
        MockServer::with_responses([ok(json!([])), ok(json!({})), ok(json!([])), ok(json!({}))]);
    let c = srv.client();
    admin::space_statuses(&c, "PROJ").unwrap();
    admin::notification_scheme(&c, "PROJ", None).unwrap();
    admin::space_types(&c, &[]).unwrap();
    admin::space_types(&c, &["software".into()]).unwrap();
    let paths: Vec<String> = srv.requests().iter().map(|r| r.path.clone()).collect();
    assert_eq!(
        paths,
        [
            "/rest/api/3/project/PROJ/statuses",
            "/rest/api/3/project/PROJ/notificationscheme",
            "/rest/api/3/project/type",
            "/rest/api/3/project/type/software"
        ]
    );
}

#[test]
fn versions_page_with_filters() {
    let srv = MockServer::with_responses([ok(json!({
        "values": [{"id": "1"}, {"id": "2"}], "startAt": 2, "maxResults": 2, "total": 5, "isLast": false
    }))]);
    let o = admin::VersionQuery {
        search: Some("1."),
        status: Some("released"),
        start: 2,
        limit: 2,
        ..Default::default()
    };
    let page = admin::versions(&srv.client(), "PROJ", &o).unwrap();
    assert_eq!(page.next.as_deref(), Some("4"));
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/project/PROJ/version");
    assert_eq!(r.query_param("status").as_deref(), Some("released"));
    assert_eq!(r.query_param("query").as_deref(), Some("1."));
}

#[test]
fn version_writes() {
    let srv = MockServer::with_responses([
        ok(json!({"key": "PROJ", "id": "10000"})),
        ok(json!({})),
        ok(json!({})),
        empty(),
        empty(),
        ok(json!({})),
        ok(json!({})),
        ok(json!({})),
    ]);
    let c = srv.client();
    admin::version_create(&c, "PROJ", map(json!({"name": "1.0"}))).unwrap();
    admin::version_update(&c, "5", map(json!({"released": true}))).unwrap();
    admin::version_delete(&c, "5", Some("6"), None).unwrap();
    admin::version_merge(&c, "5", "6").unwrap();
    admin::version_move(&c, "5", Some("6"), None).unwrap();
    admin::version_get(&c, "5").unwrap();
    admin::version_counts(&c, "5").unwrap();
    let r = srv.requests();
    assert_eq!(r[1].json_body(), json!({"name": "1.0", "projectId": 10000}));
    assert_eq!(r[2].json_body(), json!({"released": true}));
    assert_eq!(r[3].path, "/rest/api/3/version/5/removeAndSwap");
    assert_eq!(r[3].json_body(), json!({"moveFixIssuesTo": 6}));
    assert_eq!(r[4].path, "/rest/api/3/version/5/mergeto/6");
    let after = r[5].json_body()["after"].as_str().unwrap().to_string();
    assert!(after.ends_with("/rest/api/3/version/6"), "{after}");
    assert_eq!(r[7].path, "/rest/api/3/version/5/relatedIssueCounts");
}

#[test]
fn ids_of_the_wrong_kind_are_refused() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    let board = "https://x.atlassian.net/jira/software/projects/PROJ/boards/12";
    assert!(is_usage(
        &admin::component_delete(&c, board, None).unwrap_err()
    ));
    assert!(is_usage(
        &admin::version_delete(&c, board, None, None).unwrap_err()
    ));
    assert!(is_usage(&admin::version_merge(&c, "5", "5").unwrap_err()));
    assert!(is_usage(
        &admin::component_delete(&c, "5", Some("5")).unwrap_err()
    ));
    assert!(is_usage(
        &admin::version_update(&c, "5", Map::new()).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
    let url = "https://x.atlassian.net/projects/PROJ/versions/10000";
    assert_eq!(admin::numeric_id("version", url).unwrap(), "10000");
}

#[test]
fn component_endpoints() {
    let srv = MockServer::with_responses([
        ok(json!([])),
        ok(json!({"key": "PROJ", "id": "10000"})),
        ok(json!({})),
        ok(json!({})),
        ok(json!({})),
        empty(),
        ok(json!({})),
    ]);
    let c = srv.client();
    admin::components(&c, "PROJ").unwrap();
    admin::component_create(&c, "10000", map(json!({"name": "API"}))).unwrap();
    admin::component_get(&c, "7").unwrap();
    admin::component_update(&c, "7", map(json!({"name": "Core"}))).unwrap();
    admin::component_delete(&c, "7", Some("8")).unwrap();
    admin::component_counts(&c, "7").unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/project/PROJ/components");
    assert_eq!(r[2].json_body(), json!({"name": "API", "project": "PROJ"}));
    assert_eq!(r[5].query_param("moveIssuesTo").as_deref(), Some("8"));
}

#[test]
fn filter_names_must_match_exactly_and_once() {
    let found = |names: &[&str]| {
        let list: Vec<Value> = names
            .iter()
            .enumerate()
            .map(|(i, n)| json!({"id": i.to_string(), "name": n}))
            .collect();
        ok(json!({"values": list, "isLast": true}))
    };
    let srv = MockServer::with_responses([found(&["My open bugs", "My open"])]);
    assert_eq!(
        admin::resolve_filter(&srv.client(), "my open").unwrap(),
        "1"
    );

    for names in [&["My open bugs"][..], &["My open", "my OPEN"][..]] {
        let srv = MockServer::with_responses([found(names)]);
        assert!(is_usage(
            &admin::resolve_filter(&srv.client(), "My open").unwrap_err()
        ));
    }
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    assert_eq!(
        admin::resolve_filter(&c, "https://x.test/issues/?filter=123").unwrap(),
        "123"
    );
    assert!(admin::resolve_filter(&c, "https://x.test/browse/PROJ-1").is_err());
    assert!(srv.requests().is_empty());
}

#[test]
fn filter_query_passes_search_fields() {
    let srv = MockServer::with_responses([ok(json!({"values": [], "isLast": true}))]);
    let o = admin::FilterQuery {
        name: Some("open"),
        owner_account: Some("acct"),
        limit: 10,
        ..Default::default()
    };
    admin::filters(&srv.client(), &o).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/filter/search");
    assert_eq!(r.query_param("filterName").as_deref(), Some("open"));
    assert_eq!(r.query_param("accountId").as_deref(), Some("acct"));
    assert!(r.query_param("expand").unwrap().contains("jql"));
}

#[test]
fn filter_update_keeps_what_it_does_not_change() {
    let srv = MockServer::with_responses([
        ok(
            json!({"id": "9", "name": "Open", "jql": "a = 1", "description": "d",
                  "sharePermissions": [{"id": 1, "type": "loggedin"}], "editPermissions": []}),
        ),
        ok(json!({"id": "9"})),
    ]);
    admin::filter_update(&srv.client(), "9", map(json!({"jql": "a = 2"})), None).unwrap();
    assert_eq!(
        srv.requests()[1].json_body(),
        json!({"name": "Open", "jql": "a = 2", "description": "d",
               "sharePermissions": [{"id": 1, "type": "loggedin"}], "editPermissions": []})
    );

    let srv = MockServer::with_responses([ok(json!({"id": "9"}))]);
    admin::filter_update(&srv.client(), "9", Map::new(), Some(true)).unwrap();
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].path, "/rest/api/3/filter/9/favourite");
}

#[test]
fn filter_sub_resources() {
    let srv = MockServer::with_responses([
        ok(json!({})),
        ok(json!({})),
        empty(),
        empty(),
        ok(json!([])),
        empty(),
        empty(),
        ok(json!([])),
        ok(json!([])),
        empty(),
        ok(json!({"subscriptions": {"size": 0}})),
        ok(json!([])),
    ]);
    let c = srv.client();
    admin::filter_create(&c, map(json!({"name": "F"})), None, None).unwrap();
    admin::filter_get(&c, "9", None).unwrap();
    admin::filter_delete(&c, "9", None).unwrap();
    admin::filter_owner(&c, "9", "acct").unwrap();
    admin::filter_columns(&c, "9").unwrap();
    admin::filter_columns_set(&c, "9", &["summary".into()]).unwrap();
    admin::filter_columns_reset(&c, "9").unwrap();
    admin::filter_shares(&c, "9").unwrap();
    admin::filter_share_add(&c, "9", &json!({"type": "loggedin"})).unwrap();
    admin::filter_share_remove(&c, "9", "3").unwrap();
    assert_eq!(
        admin::filter_subscriptions(&c, "9", 0, 50).unwrap()["size"],
        0
    );
    admin::favourite_filters(&c).unwrap();
    let r = srv.requests();
    assert_eq!(r[3].json_body(), json!({"accountId": "acct"}));
    assert_eq!(r[5].json_body(), json!({"columns": ["summary"]}));
    assert_eq!(r[9].path, "/rest/api/3/filter/9/permission/3");
    assert_eq!(
        r[10].query_param("expand").as_deref(),
        Some("subscriptions[0:49]")
    );
}

#[test]
fn workitem_query_filter_uses_the_saved_jql() {
    let srv = MockServer::with_responses([ok(json!({"jql": "project = P"}))]);
    assert_eq!(
        admin::filter_jql(&srv.client(), "9").unwrap(),
        "project = P"
    );
}

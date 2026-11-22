//! Offline tests for the Jira Agile commands and flags (#25).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::agile::{self, Destination, On, Planning, Sprint};
use atlx::jira::{self, BoardType, Update};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

const DAY_MS: i64 = 86_400_000;

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn empty() -> MockResponse {
    MockResponse::empty(204)
}

fn page(values: Value) -> MockResponse {
    ok(json!({"values": values, "isLast": true}))
}

fn project() -> MockResponse {
    ok(json!({"key": "PROJ-1", "fields": {"project": {"key": "PROJ", "id": "10000"}}}))
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

#[test]
fn sprint_crud_uses_partial_updates() {
    let srv = MockServer::with_responses([
        ok(json!({"id": 5})),
        ok(json!({"id": 5})),
        ok(json!({"id": 5})),
        empty(),
    ]);
    let c = srv.client();
    let new = Sprint {
        name: Some("S5"),
        start: Some(20_405 * DAY_MS),
        ..Default::default()
    };
    agile::sprint_create(&c, 12, &new).unwrap();
    agile::sprint_get(&c, 5).unwrap();
    let goal = Sprint {
        goal: Some("ship"),
        ..Default::default()
    };
    agile::sprint_update(&c, 5, &goal).unwrap();
    agile::sprint_delete(&c, 5).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/agile/1.0/sprint");
    assert_eq!(
        r[0].json_body(),
        json!({"name": "S5", "startDate": "2025-11-13T00:00:00.000Z", "originBoardId": 12})
    );
    assert_eq!(r[2].method, "POST", "PUT would clear the fields left out");
    assert_eq!(r[2].json_body(), json!({"goal": "ship"}));
    assert_eq!(r[3].method, "DELETE");
    assert!(r[1..].iter().all(|x| x.path == "/rest/agile/1.0/sprint/5"));
}

#[test]
fn sprint_writes_validate_before_sending() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    let backwards = Sprint {
        start: Some(2 * DAY_MS),
        end: Some(DAY_MS),
        ..Default::default()
    };
    assert!(is_usage(
        &agile::sprint_update(&c, 5, &backwards).unwrap_err()
    ));
    assert!(is_usage(
        &agile::sprint_update(&c, 5, &Sprint::default()).unwrap_err()
    ));
    assert!(is_usage(
        &agile::sprint_complete(&c, 5, &backwards, Some(Destination::Backlog)).unwrap_err()
    ));
    let itself = Some(Destination::Sprint(5));
    assert!(is_usage(
        &agile::sprint_complete(&c, 5, &Sprint::default(), itself).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
}

#[test]
fn sprint_start_fills_in_both_dates() {
    let now = 20_405 * DAY_MS;
    let srv = MockServer::with_responses([ok(json!({"state": "future"})), ok(json!({"id": 5}))]);
    agile::sprint_start(&srv.client(), 5, &Sprint::default(), Some(14 * DAY_MS), now).unwrap();
    assert_eq!(
        srv.requests()[1].json_body(),
        json!({"state": "active", "startDate": "2025-11-13T00:00:00.000Z",
               "endDate": "2025-11-27T00:00:00.000Z"})
    );

    let srv = MockServer::with_responses([ok(json!({"state": "future"}))]);
    let err = agile::sprint_start(&srv.client(), 5, &Sprint::default(), None, now).unwrap_err();
    assert!(is_usage(&err));
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn sprint_start_keeps_the_sprints_own_dates() {
    let srv = MockServer::with_responses([
        ok(json!({"startDate": "2025-11-13T10:00:00.000+10:00",
                  "endDate": "2025-11-20T10:00:00.000+10:00"})),
        ok(json!({"id": 5})),
    ]);
    agile::sprint_start(&srv.client(), 5, &Sprint::default(), None, 0).unwrap();
    let body = srv.requests()[1].json_body();
    assert_eq!(body["startDate"], "2025-11-13T00:00:00.000Z");
    assert_eq!(body["endDate"], "2025-11-20T00:00:00.000Z");
}

#[test]
fn sprint_complete_moves_every_page_of_open_work_then_closes() {
    let issues = |from: usize, n: usize| -> Vec<Value> {
        (from..from + n)
            .map(|i| json!({"key": format!("PROJ-{i}")}))
            .collect()
    };
    let srv = MockServer::with_responses([
        ok(json!({"id": 5, "state": "active"})),
        ok(json!({"id": 6, "state": "future"})),
        ok(json!({"total": 120, "issues": issues(0, 100)})),
        ok(json!({"total": 120, "issues": issues(100, 20)})),
        empty(),
        empty(),
        empty(),
        ok(json!({"id": 5, "state": "closed"})),
    ]);
    let to = Some(Destination::Sprint(6));
    let out = agile::sprint_complete(&srv.client(), 5, &Sprint::default(), to).unwrap();
    assert_eq!(out["movedIssues"].as_array().unwrap().len(), 120);
    let r = srv.requests();
    let jql = r[2].query_param("jql").unwrap();
    assert!(jql.contains("statusCategory != Done"), "{jql}");
    assert!(jql.contains("subTaskIssueTypes()"), "{jql}");
    assert_eq!(r[3].query_param("startAt").as_deref(), Some("100"));
    let sizes: Vec<usize> = r[4..7]
        .iter()
        .map(|m| m.json_body()["issues"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [50, 50, 20]);
    assert!(
        r[4..7]
            .iter()
            .all(|m| m.path == "/rest/agile/1.0/sprint/6/issue")
    );
    assert_eq!(r[7].json_body(), json!({"state": "closed"}));
}

#[test]
fn sprint_complete_moves_nothing_unless_it_can_close() {
    let srv = MockServer::with_responses([ok(json!({"state": "future"}))]);
    let backlog = Some(Destination::Backlog);
    assert!(agile::sprint_complete(&srv.client(), 5, &Sprint::default(), backlog).is_err());
    assert_eq!(srv.requests().len(), 1);

    let srv = MockServer::with_responses([
        ok(json!({"state": "active"})),
        ok(json!({"state": "closed"})),
    ]);
    let closed = Some(Destination::Sprint(6));
    assert!(agile::sprint_complete(&srv.client(), 5, &Sprint::default(), closed).is_err());
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn sprint_complete_does_not_close_when_the_move_fails() {
    let srv = MockServer::with_responses([
        ok(json!({"state": "active"})),
        ok(json!({"total": 1, "issues": [{"key": "PROJ-1"}]})),
        MockResponse::json(400, json!({"errorMessages": ["no"]})),
    ]);
    let backlog = Some(Destination::Backlog);
    assert!(agile::sprint_complete(&srv.client(), 5, &Sprint::default(), backlog).is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 3);
    assert_eq!(r[2].path, "/rest/agile/1.0/backlog/issue");
}

#[test]
fn sprint_names_need_a_board_and_an_exact_match() {
    let srv = MockServer::with_responses([page(json!([
        {"id": 5, "name": "Team/5"}, {"id": 9, "name": "Team/50"}
    ]))]);
    let c = srv.client();
    assert!(is_usage(
        &agile::resolve_sprint(&c, "Team/7", None).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
    assert_eq!(agile::resolve_sprint(&c, "team/50", Some("12")).unwrap(), 9);
    assert_eq!(srv.requests()[0].path, "/rest/agile/1.0/board/12/sprint");
}

#[test]
fn board_commands_use_agile_endpoints() {
    let srv = MockServer::with_responses([
        ok(json!({"id": 12})),
        ok(json!({"estimation": {"type": "none"}})),
        ok(json!({"id": 13})),
        empty(),
        page(json!([{"key": "PROJ"}])),
        page(json!([{"id": 1}])),
        ok(json!({"id": 1})),
    ]);
    let c = srv.client();
    assert_eq!(
        agile::board_get(&c, 12).unwrap()["configuration"]["estimation"]["type"],
        "none"
    );
    agile::board_create(&c, "Team", BoardType::Scrum, 100).unwrap();
    agile::board_delete(&c, 13).unwrap();
    agile::board_projects(&c, 12, 0, 50).unwrap();
    agile::quick_filters(&c, 12, 0, 50).unwrap();
    agile::quick_filter(&c, 12, "1").unwrap();
    let r = srv.requests();
    let paths: Vec<&str> = r
        .iter()
        .map(|x| x.path.trim_start_matches("/rest/agile/1.0"))
        .collect();
    assert_eq!(
        paths,
        [
            "/board/12",
            "/board/12/configuration",
            "/board",
            "/board/13",
            "/board/12/project",
            "/board/12/quickfilter",
            "/board/12/quickfilter/1"
        ]
    );
    assert_eq!(
        r[2].json_body(),
        json!({"name": "Team", "type": "scrum", "filterId": 100})
    );
}

#[test]
fn backlog_applies_quick_filters_and_jql() {
    let srv = MockServer::with_responses([
        ok(json!({"jql": "type = Bug"})),
        ok(json!({"total": 1, "issues": [{"key": "PROJ-1", "fields": {"summary": "s"}}]})),
    ]);
    let filters = ["3".to_string()];
    let out = agile::backlog(
        &srv.client(),
        12,
        &filters,
        Some("a = 1 ORDER BY rank"),
        0,
        50,
    )
    .unwrap();
    assert_eq!(out["total"], 1);
    let r = &srv.requests()[1];
    assert_eq!(r.path, "/rest/agile/1.0/board/12/backlog");
    assert_eq!(
        r.query_param("jql").as_deref(),
        Some("(type = Bug) AND (a = 1) ORDER BY rank")
    );
}

#[test]
fn plan_takes_ids_none_and_estimates_without_requests() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    let p = Planning {
        sprint: Some("42"),
        original_estimate: Some("2d"),
        remaining_estimate: Some("3600"),
        ..Default::default()
    };
    let out = agile::plan(&c, On::Issue("PROJ-1"), &p).unwrap();
    assert_eq!(out.sprint, Some(Destination::Sprint(42)));
    assert_eq!(
        Value::Object(out.fields),
        json!({"timetracking": {"originalEstimate": "2d", "remainingEstimate": "60m"}})
    );
    let none = Planning {
        sprint: Some("none"),
        ..Default::default()
    };
    assert_eq!(
        agile::plan(&c, On::Issue("P-1"), &none).unwrap().sprint,
        Some(Destination::Backlog)
    );
    let bad = Planning {
        story_points: Some("lots"),
        ..Default::default()
    };
    assert!(is_usage(
        &agile::plan(&c, On::Issue("P-1"), &bad).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
}

#[test]
fn plan_finds_sprints_by_name_across_the_spaces_boards() {
    let boards = || page(json!([{"id": 1}, {"id": 2}, {"id": 3}]));
    let srv = MockServer::with_responses([
        project(),
        boards(),
        MockResponse::json(
            400,
            json!({"errorMessages": ["The board does not support sprints"]}),
        ),
        page(json!([{"id": 7, "name": "Alpha"}])),
        page(json!([{"id": 7, "name": "Alpha"}, {"id": 8, "name": "Beta"}])),
    ]);
    let p = Planning {
        sprint: Some("alpha"),
        ..Default::default()
    };
    let out = agile::plan(&srv.client(), On::Issue("PROJ-1"), &p).unwrap();
    assert_eq!(out.sprint, Some(Destination::Sprint(7)));
    let r = srv.requests();
    assert_eq!(r[1].query_param("projectKeyOrId").as_deref(), Some("PROJ"));
    assert_eq!(r[3].query_param("state").as_deref(), Some("active,future"));

    let srv = MockServer::with_responses([
        boards(),
        page(json!([{"id": 7, "name": "Alpha"}])),
        page(json!([{"id": 9, "name": "Alpha"}])),
        page(json!([])),
    ]);
    let err = agile::plan(&srv.client(), On::Space("PROJ"), &p).unwrap_err();
    assert!(is_usage(&err));
    assert!(
        err.to_string().contains('7') && err.to_string().contains('9'),
        "{err}"
    );
}

#[test]
fn plan_takes_story_points_from_the_board() {
    let srv = MockServer::with_responses([
        project(),
        page(json!([{"id": 1}, {"id": 2}])),
        ok(json!({"estimation": {"type": "field", "field": {"fieldId": "timeoriginalestimate"}}})),
        ok(json!({"estimation": {"type": "field", "field": {"fieldId": "customfield_10016"}}})),
    ]);
    let p = Planning {
        story_points: Some("5"),
        ..Default::default()
    };
    let out = agile::plan(&srv.client(), On::Issue("PROJ-1"), &p).unwrap();
    assert_eq!(out.fields["customfield_10016"], 5);
}

#[test]
fn plan_falls_back_to_field_names_and_lists_ambiguity() {
    let fields = || {
        ok(json!([
            {"id": "customfield_1", "name": "Story Points"},
            {"id": "customfield_2", "name": "Story point estimate"}
        ]))
    };
    let srv = MockServer::with_responses([
        project(),
        page(json!([])),
        fields(),
        ok(json!({"fields": {"customfield_2": {}}})),
    ]);
    let p = Planning {
        story_points: Some("3"),
        ..Default::default()
    };
    let out = agile::plan(&srv.client(), On::Issue("PROJ-1"), &p).unwrap();
    assert_eq!(out.fields["customfield_2"], 3);

    let srv = MockServer::with_responses([page(json!([])), fields()]);
    let err = agile::plan(&srv.client(), On::Space("PROJ"), &p).unwrap_err();
    assert!(is_usage(&err));
    assert!(err.to_string().contains("customfield_1"), "{err}");
}

#[test]
fn plan_sets_team_by_schema() {
    let srv = MockServer::with_responses([ok(json!([
        {"id": "customfield_9", "name": "Team",
         "schema": {"custom": "com.atlassian.jira.plugin.system.customfieldtypes:atlassian-team"}},
        {"id": "customfield_8", "name": "Team", "schema": {"custom": "x:textfield"}}
    ]))]);
    let p = Planning {
        team: Some("ari:cloud:identity::team/abc-123"),
        ..Default::default()
    };
    let out = agile::plan(&srv.client(), On::Issue("PROJ-1"), &p).unwrap();
    assert_eq!(out.fields["customfield_9"], "abc-123");
}

#[test]
fn update_can_only_move_to_a_sprint() {
    let srv = MockServer::with_responses([empty()]);
    let u = Update {
        sprint: Some(Destination::Sprint(42)),
        ..Default::default()
    };
    let out = jira::update(&srv.client(), "PROJ-1", u).unwrap();
    assert_eq!(out["sprint"], 42);
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].path, "/rest/agile/1.0/sprint/42/issue");
    assert_eq!(r[0].json_body(), json!({"issues": ["PROJ-1"]}));
}

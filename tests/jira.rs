//! Offline Jira API tests against the mock HTTP server.

mod common;

use atlx::jira::{self, GetOpts as JiraGetOpts, ListEdit, NewWorkitem, Update};
use atlx::markup::TextFormat;
use common::{MockResponse, MockServer};
use serde_json::{Map, json};

#[test]
fn jira_get_with_comments_remote_links_and_full() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "key": "PROJ-1",
                "fields": {
                    "summary": "Fix bug",
                    "status": {"name": "In Progress"},
                    "issuetype": {"name": "Bug"},
                    "priority": {"name": "High"},
                    "assignee": {"displayName": "Alice"},
                    "reporter": {"displayName": "Bob"},
                    "created": "2026-01-01T00:00:00Z",
                    "updated": "2026-01-02T00:00:00Z",
                    "duedate": "2026-02-01",
                    "labels": ["backend"],
                    "components": [{"name": "api"}],
                    "fixVersions": [{"name": "v1.0"}],
                    "parent": {"key": "PROJ-0"},
                    "description": {
                        "type": "doc",
                        "version": 1,
                        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Details here"}]}]
                    },
                    "customfield_100": "extra-val"
                }
            }),
        ),
        // Page 1 of comments (2 of 3)
        MockResponse::json(
            200,
            json!({
                "total": 3,
                "comments": [
                    {"id": "10", "author": {"displayName": "Alice"}, "created": "2026-01-02", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "First"}]}]}},
                    {"id": "11", "author": {"displayName": "Bob"}, "created": "2026-01-03", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Second"}]}]}}
                ]
            }),
        ),
        // Page 2 of comments (1 of 3)
        MockResponse::json(
            200,
            json!({
                "total": 3,
                "comments": [
                    {"id": "12", "author": {"displayName": "Carol"}, "created": "2026-01-04", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Third"}]}]}}
                ]
            }),
        ),
        // Remote links
        MockResponse::json(
            200,
            json!([
                {"id": 99, "relationship": "links to", "object": {"title": "PR #5", "url": "https://bitbucket.org/acme/api/pull-requests/5"}}
            ]),
        ),
    ]);
    let client = srv.client();

    let out = jira::get(
        &client,
        "PROJ-1",
        &JiraGetOpts {
            fields: vec!["customfield_100".into()],
            comments: true,
            remote_links: true,
            full: true,
        },
    )
    .unwrap();

    assert_eq!(out["key"], "PROJ-1");
    assert_eq!(out["summary"], "Fix bug");
    assert_eq!(out["description"], "Details here");
    assert_eq!(out["components"], json!(["api"]));
    assert_eq!(out["fix_versions"], json!(["v1.0"]));
    assert_eq!(out["parent"], "PROJ-0");
    assert_eq!(out["customfield_100"], "extra-val");
    assert!(out["fields"].is_object());
    assert_eq!(out["url"], format!("{}/browse/PROJ-1", srv.base_url()));
    assert_eq!(out["comments"].as_array().unwrap().len(), 3);
    assert_eq!(out["comments"][2]["body"], "Third");
    assert_eq!(out["remote_links"][0]["title"], "PR #5");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].path, "/rest/api/3/issue/PROJ-1");
    assert_eq!(reqs[0].query_param("fields").as_deref(), Some("*all"));
    assert_eq!(reqs[1].path, "/rest/api/3/issue/PROJ-1/comment");
    assert_eq!(reqs[1].query_param("startAt").as_deref(), Some("0"));
    assert_eq!(reqs[2].query_param("startAt").as_deref(), Some("2"));
    assert_eq!(reqs[3].path, "/rest/api/3/issue/PROJ-1/remotelink");
}

#[test]
fn jira_query_paginates_next_page_token_and_respects_limit_and_after() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "issues": [
                    {"key": "P-1", "fields": {"summary": "One", "status": {"name": "Open"}, "customfield_1": "x1"}},
                    {"key": "P-2", "fields": {"summary": "Two", "status": {"name": "Open"}, "customfield_1": "x2"}}
                ],
                "nextPageToken": "tok-1"
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "issues": [
                    {"key": "P-3", "fields": {"summary": "Three", "status": {"name": "Done"}, "customfield_1": "x3"}},
                    {"key": "P-4", "fields": {"summary": "Four", "status": {"name": "Done"}, "customfield_1": "x4"}}
                ],
                "nextPageToken": "tok-2"
            }),
        ),
    ]);
    let client = srv.client();

    let out = jira::query(
        &client,
        "project = P",
        3,
        &["customfield_1".into()],
        Some("tok-0"),
    )
    .unwrap();
    let issues = out["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0]["key"], "P-1");
    assert_eq!(issues[2]["key"], "P-3");
    assert_eq!(issues[2]["customfield_1"], "x3");
    assert_eq!(out["next_cursor"], "tok-2");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/rest/api/3/search/jql");
    assert_eq!(reqs[0].query_param("jql").as_deref(), Some("project = P"));
    assert_eq!(reqs[0].query_param("maxResults").as_deref(), Some("3"));
    assert_eq!(
        reqs[0].query_param("nextPageToken").as_deref(),
        Some("tok-0")
    );
    assert!(
        reqs[0]
            .query_param("fields")
            .unwrap()
            .contains("customfield_1")
    );
    assert_eq!(reqs[1].query_param("maxResults").as_deref(), Some("1"));
    assert_eq!(
        reqs[1].query_param("nextPageToken").as_deref(),
        Some("tok-1")
    );
}

#[test]
fn jira_create_and_update_resolve_accounts_and_fields() {
    let srv = MockServer::with_responses([
        // create: assignee="me" -> /myself, reporter=ARI (no call), POST /issue
        MockResponse::json(200, json!({"accountId": "acc-me"})),
        MockResponse::json(201, json!({"id": "10042", "key": "PROJ-42"})),
        // update: assignee="alice" -> /user/search (1 atlassian + 1 app), PUT /issue
        MockResponse::json(
            200,
            json!([
                {"accountType": "app", "accountId": "bot-1", "displayName": "Alice Bot"},
                {"accountType": "atlassian", "accountId": "acc-alice", "displayName": "Alice"}
            ]),
        ),
        MockResponse::empty(204),
        // update ambiguous user -> /user/search returns 2 atlassian users -> errors before PUT
        MockResponse::json(
            200,
            json!([
                {"accountType": "atlassian", "accountId": "acc-a1", "displayName": "Alex A"},
                {"accountType": "atlassian", "accountId": "acc-a2", "displayName": "Alex B"}
            ]),
        ),
    ]);
    let client = srv.client();

    let created = jira::create(
        &client,
        NewWorkitem {
            space: "PROJ",
            issue_type: "Bug",
            summary: "New bug",
            description: Some(("Hello **world**", TextFormat::Markdown)),
            assignee: Some("me"),
            reporter: Some("ari:cloud:identity::user/557058:rep-1"),
            priority: Some("High"),
            labels: vec!["triage".into()],
            parent: Some("PROJ-1"),
            extra: Map::new(),
        },
    )
    .unwrap();
    assert_eq!(created["key"], "PROJ-42");
    assert_eq!(created["id"], "10042");

    let updated = jira::update(
        &client,
        "PROJ-42",
        Update {
            summary: Some("Updated bug"),
            assignee: Some("alice"),
            labels: Some(ListEdit {
                set: None,
                add: vec!["urgent".into()],
                remove: vec!["triage".into()],
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(updated["key"], "PROJ-42");
    assert_eq!(updated["updated"], json!(["summary", "assignee", "labels"]));

    let ambig_err = jira::update(
        &client,
        "PROJ-42",
        Update {
            assignee: Some("alex"),
            ..Default::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(ambig_err.contains("matches several users"), "{ambig_err}");
    assert!(
        ambig_err.contains("Alex A") && ambig_err.contains("Alex B"),
        "{ambig_err}"
    );

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 5);
    assert_eq!(reqs[0].path, "/rest/api/3/myself");
    let create_body = reqs[1].json_body();
    assert_eq!(create_body["fields"]["project"], json!({"key": "PROJ"}));
    assert_eq!(
        create_body["fields"]["assignee"],
        json!({"accountId": "acc-me"})
    );
    assert_eq!(
        create_body["fields"]["reporter"],
        json!({"accountId": "557058:rep-1"})
    );
    assert_eq!(create_body["fields"]["parent"], json!({"key": "PROJ-1"}));

    assert_eq!(reqs[2].path, "/rest/api/3/user/search");
    assert_eq!(reqs[2].query_param("query").as_deref(), Some("alice"));
    let update_body = reqs[3].json_body();
    assert_eq!(
        update_body["fields"]["assignee"],
        json!({"accountId": "acc-alice"})
    );
    assert_eq!(
        update_body["update"]["labels"],
        json!([{"add": "urgent"}, {"remove": "triage"}])
    );
}

#[test]
fn jira_transition_handles_has_screen_true_and_false() {
    let transitions_payload = json!({
        "transitions": [
            {
                "id": "21",
                "name": "Resolve",
                "to": {"name": "Resolved"},
                "hasScreen": true,
                "fields": {
                    "resolution": {"required": true, "name": "Resolution", "allowedValues": [{"name": "Done"}]}
                }
            },
            {
                "id": "31",
                "name": "Close",
                "to": {"name": "Closed"},
                "hasScreen": false,
                "fields": {}
            }
        ]
    });

    let srv = MockServer::with_responses([
        // Case 1: hasScreen = true => comment goes inside transition POST
        MockResponse::json(200, transitions_payload.clone()),
        MockResponse::empty(204),
        // Case 2: hasScreen = false => transition POST has no comment, followed by POST /comment
        MockResponse::json(200, transitions_payload),
        MockResponse::empty(204),
        MockResponse::json(201, json!({"id": "c-77"})),
    ]);
    let client = srv.client();

    let mut fields = Map::new();
    fields.insert("resolution".into(), json!({"name": "Done"}));
    let res1 = jira::transition(
        &client,
        "PROJ-1",
        Some("Resolve"),
        fields,
        Some(("Fixed it", TextFormat::Plain)),
    )
    .unwrap();
    assert_eq!(res1["status"], "Resolved");
    assert!(res1.get("comment_id").is_none());

    let res2 = jira::transition(
        &client,
        "PROJ-1",
        Some("Closed"),
        Map::new(),
        Some(("Closing note", TextFormat::Plain)),
    )
    .unwrap();
    assert_eq!(res2["status"], "Closed");
    assert_eq!(res2["comment_id"], "c-77");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 5);

    let post_screen = reqs[1].json_body();
    assert_eq!(post_screen["transition"]["id"], "21");
    assert_eq!(post_screen["fields"]["resolution"]["name"], "Done");
    assert!(post_screen["update"]["comment"][0]["add"]["body"].is_object());

    let post_noscreen = reqs[3].json_body();
    assert_eq!(post_noscreen["transition"]["id"], "31");
    assert!(post_noscreen.get("update").is_none());

    assert_eq!(reqs[4].path, "/rest/api/3/issue/PROJ-1/comment");
}

#[test]
fn jira_space_query_sends_twg_filters_and_offset() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({
            "isLast": true,
            "values": [{"id": "10000", "key": "PROJ", "name": "Project", "projectTypeKey": "software"}]
        }),
    )]);
    let client = srv.client();
    let keys = vec!["PROJ".to_string(), "OPS".to_string()];
    let out = jira::space_query(
        &client,
        &jira::SpaceQuery {
            query: Some("proj"),
            keys: &keys,
            type_key: Some("software"),
            order_by: Some("name"),
            start: 10,
            limit: 5,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(out[0]["key"], "PROJ");
    assert_eq!(out[0]["type"], "software");

    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/rest/api/3/project/search");
    assert!(
        reqs[0].query.contains("keys=PROJ&keys=OPS"),
        "{}",
        reqs[0].query
    );
    assert_eq!(reqs[0].query_param("query").as_deref(), Some("proj"));
    assert_eq!(reqs[0].query_param("typeKey").as_deref(), Some("software"));
    assert_eq!(reqs[0].query_param("orderBy").as_deref(), Some("name"));
    assert_eq!(reqs[0].query_param("startAt").as_deref(), Some("10"));
    assert_eq!(reqs[0].query_param("maxResults").as_deref(), Some("5"));
}

#[test]
fn jira_board_query_by_issue_and_sprints_for_project() {
    let srv = MockServer::with_responses([
        // issue_project
        MockResponse::json(200, json!({"fields": {"project": {"key": "PROJ"}}})),
        // board_query
        MockResponse::json(
            200,
            json!({"isLast": true, "values": [{"id": 12, "name": "PROJ board", "type": "scrum", "location": {"projectKey": "PROJ"}}]}),
        ),
        // project_board
        MockResponse::json(
            200,
            json!({"isLast": true, "values": [{"id": 12, "name": "PROJ board"}]}),
        ),
        // sprints
        MockResponse::json(
            200,
            json!({"isLast": true, "values": [{"id": 99, "name": "Sprint 9", "state": "active", "originBoardId": 12}]}),
        ),
    ]);
    let client = srv.client();
    let project = jira::issue_project(&client, "PROJ-1").unwrap();
    assert_eq!(project, "PROJ");
    let boards = jira::board_query(
        &client,
        &jira::BoardQuery {
            project: Some(&project),
            name: Some("board"),
            filter_id: Some("10100"),
            start: 0,
            limit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(boards[0]["id"], 12);
    assert_eq!(boards[0]["project_key"], "PROJ");

    let board = jira::project_board(&client, "PROJ").unwrap();
    let sprints = jira::sprints(&client, board, Some("active"), 5, 10).unwrap();
    assert_eq!(sprints[0]["id"], 99);

    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/rest/api/3/issue/PROJ-1");
    assert_eq!(reqs[0].query_param("fields").as_deref(), Some("project"));
    assert_eq!(reqs[1].path, "/rest/agile/1.0/board");
    assert_eq!(
        reqs[1].query_param("projectKeyOrId").as_deref(),
        Some("PROJ")
    );
    assert_eq!(reqs[1].query_param("name").as_deref(), Some("board"));
    assert_eq!(reqs[1].query_param("filterId").as_deref(), Some("10100"));
    assert_eq!(reqs[1].query_param("maxResults").as_deref(), Some("10"));
    assert_eq!(reqs[3].path, "/rest/agile/1.0/board/12/sprint");
    assert_eq!(reqs[3].query_param("state").as_deref(), Some("active"));
    assert_eq!(reqs[3].query_param("startAt").as_deref(), Some("5"));
}

#[test]
fn jira_resolve_board_by_id_url_and_name() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"isLast": true, "values": [
                {"id": 7, "name": "Team Board Extra"},
                {"id": 8, "name": "Team Board"}
            ]}),
        ),
        MockResponse::json(200, json!({"isLast": true, "values": []})),
    ]);
    let client = srv.client();
    assert_eq!(jira::resolve_board(&client, "12").unwrap(), 12);
    assert_eq!(
        jira::resolve_board(
            &client,
            "https://acme.atlassian.net/jira/software/projects/PROJ/boards/34"
        )
        .unwrap(),
        34
    );
    assert_eq!(
        jira::resolve_board(
            &client,
            "https://acme.atlassian.net/secure/RapidBoard.jspa?rapidView=56"
        )
        .unwrap(),
        56
    );
    assert_eq!(jira::resolve_board(&client, "team board").unwrap(), 8);
    assert!(jira::resolve_board(&client, "Nope").is_err());
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].query_param("name").as_deref(), Some("team board"));
}

#[test]
fn jira_project_board_rejects_several_boards() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"isLast": true, "values": [{"id": 1, "name": "A"}, {"id": 2, "name": "B"}]}),
    )]);
    let err = jira::project_board(&srv.client(), "PROJ").unwrap_err();
    assert!(err.to_string().contains("several boards"), "{err}");
}

#[test]
fn jira_sprint_workitems_with_fields_and_offset() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({
            "total": 3,
            "issues": [{"key": "PROJ-3", "fields": {"summary": "Three", "labels": ["x"]}}]
        }),
    )]);
    let client = srv.client();
    let fields = vec!["labels".to_string()];
    let out = jira::sprint_issues(
        &client,
        jira::sprint_ref("ari:cloud:jira-software:abc:sprint/99").unwrap(),
        &jira::SprintIssues {
            jql: Some("status != Done"),
            fields: &fields,
            expand: Some("names"),
            start: 2,
            limit: 10,
        },
    )
    .unwrap();
    assert_eq!(out[0]["key"], "PROJ-3");
    assert_eq!(out[0]["labels"], json!(["x"]));
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/rest/agile/1.0/sprint/99/issue");
    assert_eq!(reqs[0].query_param("startAt").as_deref(), Some("2"));
    assert_eq!(reqs[0].query_param("expand").as_deref(), Some("names"));
    assert_eq!(
        reqs[0].query_param("jql").as_deref(),
        Some("status != Done")
    );
    assert!(reqs[0].query_param("fields").unwrap().contains("labels"));
    assert!(jira::sprint_ref("not a sprint").is_err());
}

#[test]
fn jira_link_workitem_sends_source_as_inward_issue_per_target() {
    let srv = MockServer::with_responses([MockResponse::empty(201), MockResponse::empty(201)]);
    let client = srv.client();
    let targets = vec!["PROJ-2".to_string(), "PROJ-3".to_string()];
    let out = jira::link_workitem(&client, "PROJ-1", &targets, "Blocks").unwrap();
    assert_eq!(out.as_array().unwrap().len(), 2);
    assert_eq!(out[1]["outward"], "PROJ-3");
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    for (r, t) in reqs.iter().zip(["PROJ-2", "PROJ-3"]) {
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/rest/api/3/issueLink");
        let b = r.json_body();
        assert_eq!(b["inwardIssue"], json!({"key": "PROJ-1"}));
        assert_eq!(b["outwardIssue"], json!({"key": t}));
        assert_eq!(b["type"], json!({"name": "Blocks"}));
    }

    let one = MockServer::with_responses([MockResponse::empty(201)]);
    let v = jira::link_workitem(&one.client(), "PROJ-1", &targets[..1], "10000").unwrap();
    assert_eq!(v["outward"], "PROJ-2");
    assert_eq!(
        one.requests()[0].json_body()["type"],
        json!({"id": "10000"})
    );
    assert!(jira::link_workitem(&one.client(), "PROJ-1", &[], "Blocks").is_err());
}

#[test]
fn jira_weblink_and_user_by_account() {
    let srv = MockServer::with_responses([
        MockResponse::json(201, json!({"id": 10001})),
        MockResponse::json(
            200,
            json!({"accountId": "abc", "displayName": "Alice", "active": true, "accountType": "atlassian"}),
        ),
    ]);
    let client = srv.client();
    let out = jira::link_remote(
        &client,
        "PROJ-1",
        &jira::Weblink {
            url: "https://example.com/spec",
            title: "Spec",
            global_id: Some("spec-1"),
            resolved: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(out["id"], 10001);
    let users = jira::user_by_account(&client, "ari:cloud:identity::user/abc").unwrap();
    assert_eq!(users[0]["display_name"], "Alice");

    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/rest/api/3/issue/PROJ-1/remotelink");
    let body = reqs[0].json_body();
    assert_eq!(body["globalId"], "spec-1");
    assert_eq!(body["object"]["status"]["resolved"], true);
    assert_eq!(reqs[1].path, "/rest/api/3/user");
    assert_eq!(reqs[1].query_param("accountId").as_deref(), Some("abc"));
}

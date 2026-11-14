//! Offline Confluence API tests against the mock HTTP server.

mod common;

use atlx::confluence::{
    self, ContentType, Detail, Edit, GetOpts as ConfGetOpts, NewContent, ReadFormat,
};
use atlx::markup::TextFormat;
use common::{MockResponse, MockServer};
use serde_json::json;

#[test]
fn confluence_get_falls_back_from_page_404_to_blogpost() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            404,
            json!({"errors": [{"status": 404, "title": "Page not found"}]}),
        ),
        MockResponse::json(
            200,
            json!({
                "id": "55555",
                "title": "Launch Post",
                "status": "current",
                "spaceId": "101",
                "parentId": null,
                "version": {"number": 3, "createdAt": "2026-01-10T00:00:00Z"},
                "body": {"storage": {"value": "<h2>Release</h2><p>Shipped!</p>"}},
                "_links": {"webui": "/spaces/ENG/blog/55555"}
            }),
        ),
    ]);
    let client = srv.client();

    let out = confluence::get(
        &client,
        "55555",
        &ConfGetOpts {
            detail: Detail::Full,
            format: ReadFormat::Md,
            draft: false,
            reference_space: None,
        },
    )
    .unwrap();

    assert_eq!(out["id"], "55555");
    assert_eq!(out["type"], "blogpost");
    assert_eq!(out["snapshotToken"], "3");
    assert_eq!(out["detail"], "full");
    assert_eq!(out["body"]["format"], "md");
    let body = out["body"]["value"].as_str().unwrap();
    assert!(body.contains("## Release"));
    assert!(body.contains("Shipped!"));
    assert_eq!(
        out["externalUrl"],
        format!("{}/spaces/ENG/blog/55555", srv.base_url())
    );

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/api/v2/pages/55555");
    assert_eq!(reqs[1].path, "/api/v2/blogposts/55555");
}

#[test]
fn confluence_update_snapshot_token_validation_and_dry_run() {
    let current_page = json!({
        "id": "12345",
        "title": "Runbook",
        "status": "current",
        "spaceId": "10",
        "version": {"number": 4, "createdAt": "2026-01-10T00:00:00Z"},
        "body": {"storage": {"value": "<p>Old</p>"}}
    });

    let srv = MockServer::with_responses([
        // 1) Stale snapshot token
        MockResponse::json(200, current_page.clone()),
        // 2) Matching token + dry_run
        MockResponse::json(200, current_page.clone()),
        // 3) Matching token + real update
        MockResponse::json(200, current_page),
        MockResponse::json(
            200,
            json!({
                "id": "12345",
                "title": "Runbook",
                "version": {"number": 5},
                "_links": {"webui": "/spaces/ENG/pages/12345"}
            }),
        ),
    ]);
    let client = srv.client();

    let stale_err = confluence::update(
        &client,
        "12345",
        Edit {
            title: None,
            body: Some(("New", TextFormat::Plain)),
            version_message: None,
            minor_edit: false,
            snapshot_token: Some("3"),
            dry_run: false,
            draft: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(stale_err.contains("token 3, current 4"), "{stale_err}");

    let dry = confluence::update(
        &client,
        "12345",
        Edit {
            title: Some("Runbook v2"),
            body: Some(("# Heading", TextFormat::Markdown)),
            version_message: Some("bump"),
            minor_edit: true,
            snapshot_token: Some("4"),
            dry_run: true,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(dry["dryRun"], true);
    assert_eq!(dry["fromVersion"], 4);
    assert_eq!(dry["request"]["version"]["number"], 5);
    assert_eq!(dry["request"]["version"]["minorEdit"], true);
    assert_eq!(dry["request"]["body"]["value"], "<h1>Heading</h1>");

    let real = confluence::update(
        &client,
        "12345",
        Edit {
            title: None,
            body: Some(("Updated", TextFormat::Plain)),
            version_message: Some("edit"),
            minor_edit: false,
            snapshot_token: Some("4"),
            dry_run: false,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(real["version"]["number"], 5);
    assert_eq!(real["snapshotToken"], "5");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[3].method, "PUT");
    assert_eq!(reqs[3].path, "/api/v2/pages/12345");
    assert_eq!(reqs[3].json_body()["version"]["number"], 5);
}

#[test]
fn confluence_search_query_and_list_extract_next_cursor() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "results": [{
                    "title": "@@@hl@@@Launch@@@endhl@@@ Plan",
                    "excerpt": "  Our @@@hl@@@launch@@@endhl@@@ checklist  ",
                    "lastModified": "2026-01-05",
                    "url": "/spaces/ENG/pages/10",
                    "content": {"id": "10", "type": "page"},
                    "resultGlobalContainer": {"title": "Engineering"}
                }],
                "_links": {"next": "/wiki/rest/api/search?cql=type%3Dpage&cursor=cur%2F1%3D%3D"}
            }),
        ),
        // list: resolve space key ENG -> id 999, then list pages
        MockResponse::json(200, json!({"results": [{"id": "999", "key": "ENG"}]})),
        MockResponse::json(
            200,
            json!({
                "results": [{
                    "id": "10",
                    "title": "Runbook",
                    "status": "current",
                    "parentId": "1",
                    "version": {"number": 2, "createdAt": "2026-01-05"},
                    "_links": {"webui": "/spaces/ENG/pages/10"}
                }],
                "_links": {"next": "/wiki/api/v2/spaces/999/pages?limit=5&cursor=page%3D2"}
            }),
        ),
        // create: numeric space id avoids /spaces lookup
        MockResponse::json(
            200,
            json!({"id": "77", "title": "Created", "status": "current", "version": {"number": 1}, "_links": {"webui": "/pages/77"}}),
        ),
    ]);
    let client = srv.client();

    let searched = confluence::search_query(&client, "type=page", 10, true, Some("prev")).unwrap();
    assert_eq!(searched["nextCursor"], "cur/1==");
    assert!(searched.get("_links").is_none());
    assert_eq!(searched["results"][0]["title"], "Launch Plan");
    assert_eq!(searched["results"][0]["excerpt"], "Our launch checklist");

    let listed =
        confluence::list(&client, "ENG", ContentType::Page, "current", 5, Some("c0")).unwrap();
    assert_eq!(listed["nextCursor"], "page=2");
    assert_eq!(listed["results"][0]["id"], "10");

    let created = confluence::create(
        &client,
        NewContent {
            space: "999",
            content_type: ContentType::Page,
            title: "Created",
            body: "Hi",
            format: TextFormat::Plain,
            parent: None,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(created["id"], "77");
    assert_eq!(created["snapshotToken"], "1");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].query_param("cursor").as_deref(), Some("prev"));
    assert_eq!(reqs[1].path, "/api/v2/spaces");
    assert_eq!(reqs[1].query_param("keys").as_deref(), Some("ENG"));
    assert_eq!(reqs[2].path, "/api/v2/spaces/999/pages");
    assert_eq!(reqs[2].query_param("cursor").as_deref(), Some("c0"));
}

#[test]
fn confluence_space_labels_are_added_per_space() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"results": [{"id": "101", "key": "ENG"}, {"id": "102", "key": "OPS"}], "_links": {}}),
        ),
        MockResponse::json(
            200,
            json!({"results": [{"id": "1", "name": "team", "prefix": "global"}]}),
        ),
        MockResponse::json(200, json!({"results": []})),
    ]);
    let client = srv.client();
    let mut v = confluence::spaces(&client, None, None, None, 25, None).unwrap();
    confluence::add_space_labels(&client, &mut v).unwrap();
    assert_eq!(v["results"][0]["labels"][0]["name"], "team");
    assert_eq!(v["results"][1]["labels"], json!([]));
    let reqs = srv.requests();
    assert_eq!(reqs[1].path, "/api/v2/spaces/101/labels");
    assert_eq!(reqs[2].path, "/api/v2/spaces/102/labels");
}

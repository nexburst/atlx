//! Offline Confluence API tests against the mock HTTP server: reads,
//! creates, full-body updates, search and lists.

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{self, ContentType, GetOpts, NewContent, Update, VersionOpts};
use atlx::markup::{Body, BodyFormat, parse_body};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn doc(content: Value) -> String {
    json!({"type": "doc", "version": 1, "content": content}).to_string()
}

fn para(text: &str) -> Value {
    json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
}

/// A page whose body has a panel, a table and a Jira macro.
fn rich_page() -> Value {
    let body = doc(json!([
        {"type": "panel", "attrs": {"panelType": "info"}, "content": [para("Read me first")]},
        {"type": "table", "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [para("Step")]},
                {"type": "tableHeader", "content": [para("Owner")]}
            ]},
            {"type": "tableRow", "content": [
                {"type": "tableCell", "content": [para("Deploy")]},
                {"type": "tableCell", "content": [para("Ann")]}
            ]}
        ]},
        {"type": "extension", "attrs": {"extensionType": "com.atlassian.confluence.macro.core",
            "extensionKey": "jira", "localId": "jira-1",
            "parameters": {"macroParams": {"key": {"value": "OPS-12"}}}}},
        {"type": "paragraph", "attrs": {"localId": "p-1"}, "content": [{"type": "text", "text": "Old text"}]}
    ]));
    json!({
        "id": "12345", "title": "Runbook", "status": "current", "spaceId": "10",
        "version": {"number": 7, "createdAt": "2026-01-10T00:00:00Z"},
        "body": {"atlas_doc_format": {"value": body, "representation": "atlas_doc_format"}},
        "_links": {"webui": "/spaces/ENG/pages/12345"}
    })
}

fn plain_page() -> Value {
    json!({
        "id": "12345", "title": "Runbook", "status": "current", "spaceId": "10",
        "version": {"number": 7},
        "body": {
            "atlas_doc_format": {"value": doc(json!([para("Old")]))},
            "storage": {"value": "<p>Old</p>"}
        },
    })
}

fn kind(e: &anyhow::Error) -> Option<ErrorKind> {
    ApiError::find(e).map(|a| a.kind)
}

fn md(s: &str) -> Body {
    parse_body(s, BodyFormat::Markdown).unwrap()
}

fn version(n: u64) -> VersionOpts<'static> {
    VersionOpts {
        if_version: n,
        ..Default::default()
    }
}

#[test]
fn get_falls_back_from_page_404_to_blogpost() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            404,
            json!({"errors": [{"status": 404, "title": "Page not found"}]}),
        ),
        MockResponse::json(
            200,
            json!({
                "id": "55555", "title": "Launch Post", "status": "current", "spaceId": "101",
                "version": {"number": 3, "createdAt": "2026-01-10T00:00:00Z"},
                "body": {"atlas_doc_format": {"value": doc(json!([
                    {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Release"}]},
                    para("Shipped!")
                ]))}},
                "_links": {"webui": "/spaces/ENG/blog/55555"}
            }),
        ),
    ]);
    let out = confluence::get(&srv.client(), "55555", &GetOpts::default()).unwrap();
    assert_eq!(out["type"], "blogpost");
    assert_eq!(out["body"], "## Release\n\nShipped!");
    assert_eq!(
        out["externalUrl"],
        format!("{}/spaces/ENG/blog/55555", srv.base_url())
    );
    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/api/v2/pages/55555");
    assert_eq!(reqs[1].path, "/api/v2/blogposts/55555");
    assert_eq!(
        reqs[1].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
}

#[test]
fn a_rich_page_reads_as_markdown_with_a_marker_for_the_macro() {
    let srv = MockServer::with_responses([MockResponse::json(200, rich_page())]);
    let out = confluence::get(&srv.client(), "12345", &GetOpts::default()).unwrap();
    let body = out["body"].as_str().unwrap();
    assert!(body.starts_with("> [!NOTE]\n> Read me first"), "{body}");
    assert!(body.contains("| Step | Owner |"), "{body}");
    assert!(body.contains("| Deploy | Ann |"), "{body}");
    assert!(body.contains("<!-- atlx: extension:jira"), "{body}");
    // The lossy_body warning is in atlx::output::notes; the CLI test checks
    // that it reaches the envelope.
}

#[test]
fn storage_reads_are_passed_through() {
    let srv = MockServer::with_responses([MockResponse::json(200, plain_page())]);
    let o = GetOpts {
        format: BodyFormat::Storage,
        ..Default::default()
    };
    let out = confluence::get(&srv.client(), "12345", &o).unwrap();
    assert_eq!(out["body"], "<p>Old</p>");
    assert_eq!(out["bodyFormat"], "storage");
    assert_eq!(
        srv.requests()[0].query_param("body-format").as_deref(),
        Some("storage")
    );
}

#[test]
fn title_lookup_needs_a_space_and_finds_blog_posts() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, json!({"results": [{"id": "55", "key": "ENG"}]})),
        MockResponse::json(200, json!({"results": []})),
        MockResponse::json(
            200,
            json!({"results": [{"id": "901", "title": "Runbook", "body": {}}]}),
        ),
    ]);
    let c = srv.client();
    let err = confluence::get(&c, "Runbook", &GetOpts::default()).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Usage));
    assert!(srv.requests().is_empty());
    let o = GetOpts {
        space: Some("ENG"),
        ..Default::default()
    };
    let out = confluence::get(&c, "Runbook", &o).unwrap();
    assert_eq!(out["id"], "901");
    assert_eq!(out["type"], "blogpost");
}

#[test]
fn create_sends_adf_and_never_converts_to_storage() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({"id": "77", "title": "Created", "status": "current", "version": {"number": 1},
               "_links": {"webui": "/pages/77"}}),
    )]);
    let created = confluence::create(
        &srv.client(),
        NewContent {
            space: "999",
            content_type: ContentType::Page,
            title: "Created",
            body: &md("Hi **there**"),
            parent: Some("123"),
            draft: false,
            private: false,
        },
    )
    .unwrap();
    assert_eq!(created["id"], "77");
    assert_eq!(created["type"], "page");
    let body = srv.requests()[0].json_body();
    assert_eq!(body["spaceId"], "999");
    assert_eq!(body["parentId"], "123");
    assert_eq!(body["body"]["representation"], "atlas_doc_format");
    let sent: Value = serde_json::from_str(body["body"]["value"].as_str().unwrap()).unwrap();
    assert_eq!(
        sent["content"][0]["content"][1]["marks"][0]["type"],
        "strong"
    );
}

#[test]
fn update_checks_the_version_before_writing() {
    let srv = MockServer::with_responses([MockResponse::json(200, plain_page())]);
    let u = Update {
        title: Some("New"),
        ..Default::default()
    };
    let err = confluence::update(&srv.client(), "12345", &u, &version(6)).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Conflict);
    assert_eq!(api.exit_code(), 6);
    assert!(
        api.message
            .contains("page 12345 is at version 7, you edited version 6")
    );
    assert!(api.hint.as_deref().unwrap().contains("--if-version 7"));
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn a_title_change_sends_the_stored_body_back_unchanged() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, plain_page()),
        MockResponse::json(200, json!({"id": "12345", "version": {"number": 8}})),
    ]);
    let u = Update {
        title: Some("New"),
        ..Default::default()
    };
    confluence::update(&srv.client(), "12345", &u, &version(7)).unwrap();
    let reqs = srv.requests();
    assert_eq!(
        reqs[0].query_param("body-format").as_deref(),
        Some("storage")
    );
    let put = reqs[1].json_body();
    assert_eq!(put["title"], "New");
    assert_eq!(put["version"]["number"], 8);
    assert_eq!(
        put["body"],
        json!({"representation": "storage", "value": "<p>Old</p>"})
    );
}

#[test]
fn a_markdown_replace_of_a_rich_body_is_refused_and_sends_nothing() {
    let srv = MockServer::with_responses([MockResponse::json(200, rich_page())]);
    let body = md("All new");
    let u = Update {
        body: Some(&body),
        check_loss: true,
        ..Default::default()
    };
    let err = confluence::update(&srv.client(), "12345", &u, &version(7)).unwrap_err();
    let api = ApiError::find(&err).unwrap();
    assert_eq!(api.kind, ErrorKind::Refused);
    assert_eq!(api.exit_code(), 8);
    assert!(api.message.contains("extension:jira"), "{}", api.message);
    assert!(api.hint.as_deref().unwrap().contains("page edit"));
    assert!(api.losses.is_some());
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn an_adf_replace_or_allow_lossy_is_sent() {
    for (format, allow) in [(BodyFormat::Adf, false), (BodyFormat::Markdown, true)] {
        let srv = MockServer::with_responses([
            MockResponse::json(200, rich_page()),
            MockResponse::json(200, json!({"id": "12345"})),
        ]);
        let body = match format {
            BodyFormat::Adf => parse_body(&doc(json!([para("x")])), BodyFormat::Adf).unwrap(),
            _ => md("x"),
        };
        let u = Update {
            body: Some(&body),
            check_loss: format == BodyFormat::Markdown,
            allow_lossy: allow,
            ..Default::default()
        };
        confluence::update(&srv.client(), "12345", &u, &version(7)).unwrap();
        let put = &srv.requests()[1];
        assert_eq!(put.method, "PUT");
        assert_eq!(
            put.json_body()["body"]["representation"],
            "atlas_doc_format"
        );
    }
}

#[test]
fn search_strips_highlights_and_entities_and_adds_urls() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({
            "results": [{
                "title": "@@@hl@@@Launch@@@endhl@@@ &amp; Plan",
                "excerpt": "  Our @@@hl@@@launch@@@endhl@@@ checklist  ",
                "lastModified": "2026-01-05",
                "url": "/spaces/ENG/pages/10",
                "content": {"id": "10", "type": "page"},
                "resultGlobalContainer": {"title": "Engineering"}
            }],
            "_links": {"base": "https://x.test/wiki",
                       "next": "/wiki/rest/api/search?cql=type%3Dpage&cursor=cur%2F1%3D%3D"}
        }),
    )]);
    let out = confluence::search_query(&srv.client(), "type=page", 10, true, Some("prev")).unwrap();
    assert_eq!(out["nextCursor"], "cur/1==");
    assert!(out.get("_links").is_none());
    let hit = &out["results"][0];
    assert_eq!(hit["title"], "Launch & Plan");
    assert_eq!(hit["excerpt"], "Our launch checklist");
    assert_eq!(
        hit["externalUrl"],
        "https://x.test/wiki/spaces/ENG/pages/10"
    );
    assert_eq!(
        srv.requests()[0].query_param("cursor").as_deref(),
        Some("prev")
    );
}

#[test]
fn lists_resolve_the_space_and_pass_the_cursor() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, json!({"results": [{"id": "999", "key": "ENG"}]})),
        MockResponse::json(
            200,
            json!({
                "results": [{"id": "10", "title": "Runbook", "_links": {"webui": "/spaces/ENG/pages/10"}}],
                "_links": {"next": "/wiki/api/v2/spaces/999/pages?limit=5&cursor=page%3D2"}
            }),
        ),
        MockResponse::json(
            200,
            json!({"results": [
                {"id": "11", "type": "page", "title": "Child"},
                {"id": "12", "type": "whiteboard", "title": "Board"}
            ]}),
        ),
    ]);
    let c = srv.client();
    let listed = confluence::list(&c, "ENG", ContentType::Page, "current", 5, Some("c0")).unwrap();
    assert_eq!(listed["nextCursor"], "page=2");
    assert_eq!(listed["results"][0]["type"], "page");
    let kids = confluence::children(&c, "10", 5, None).unwrap();
    assert_eq!(kids["results"].as_array().unwrap().len(), 1);
    assert_eq!(kids["results"][0]["parentId"], "10");
    let reqs = srv.requests();
    assert_eq!(reqs[0].query_param("keys").as_deref(), Some("ENG"));
    assert_eq!(reqs[1].path, "/api/v2/spaces/999/pages");
    assert_eq!(reqs[1].query_param("cursor").as_deref(), Some("c0"));
    assert_eq!(reqs[2].path, "/api/v2/pages/10/direct-children");
}

#[test]
fn space_labels_are_added_per_space() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"results": [{"id": "101", "key": "ENG"}, {"id": "102", "key": "OPS"}]}),
        ),
        MockResponse::json(
            200,
            json!({"results": [{"id": "1", "name": "team", "prefix": "global"}]}),
        ),
        MockResponse::json(200, json!({"results": []})),
    ]);
    let c = srv.client();
    let mut v = confluence::spaces(&c, None, None, None, 25, None).unwrap();
    let mut items = v["results"].take();
    confluence::add_space_labels(&c, &mut items).unwrap();
    assert_eq!(items[0]["labels"][0]["name"], "team");
    assert_eq!(items[1]["labels"], json!([]));
    let reqs = srv.requests();
    assert_eq!(reqs[1].path, "/api/v2/spaces/101/labels");
    assert_eq!(reqs[2].path, "/api/v2/spaces/102/labels");
}

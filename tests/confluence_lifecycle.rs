//! Offline tests for the Confluence lifecycle commands and tree (#30).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{self, ContentType, lifecycle as lc, tree};
use atlx::markup::TextFormat;
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::time::Duration;

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn not_found() -> MockResponse {
    MockResponse::json(404, json!({"message": "not found"}))
}

fn empty() -> MockResponse {
    MockResponse::empty(204)
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

#[test]
fn space_writes_never_take_a_content_url() {
    for bad in [
        "https://x.test/wiki/spaces/ENG/pages/12345/Title",
        "https://x.test/wiki/spaces/ENG/blog/2026/01/02/12345/Post",
        "https://x.test/wiki/x/AbC",
        "ari:cloud:confluence:site:page/12345",
        "Engineering Space",
        "EN-G",
        "..",
        "~",
        "",
    ] {
        let srv = MockServer::with_responses([]);
        assert!(
            is_usage(&lc::space_delete(&srv.client(), bad).unwrap_err()),
            "{bad}"
        );
        assert!(srv.requests().is_empty(), "{bad} was sent");
    }
    for (good, want) in [
        ("ENG", "ENG"),
        ("12345", "12345"),
        ("~557058:ab-cd", "~557058:ab-cd"),
        ("https://x.test/wiki/spaces/ENG", "ENG"),
        (
            "https://x.test/wiki/spaces/ENG/overview?homepageId=1",
            "ENG",
        ),
        ("ari:cloud:confluence:site:space/99", "99"),
    ] {
        assert_eq!(lc::exact_space(good).unwrap(), want);
    }
}

#[test]
fn content_references_never_guess() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    for (good, want) in [
        ("12345", "12345"),
        ("https://x.test/wiki/spaces/E/pages/1234/20260105", "1234"),
        ("https://x.test/wiki/spaces/E/pages/edit-v2/12345", "12345"),
        (
            "https://x.test/wiki/spaces/E/blog/2026/01/02/55555/Post",
            "55555",
        ),
        (
            "https://x.test/wiki/pages/viewpage.action?pageId=12345",
            "12345",
        ),
        ("ari:cloud:confluence:site:page/12345", "12345"),
    ] {
        assert_eq!(lc::exact_content(&c, good, None).unwrap(), want);
    }
    for bad in [
        "https://x.test/wiki/spaces/E/pages/create?parentPageId=12345",
        "https://x.test/wiki/spaces/E/overview?homepageId=12345",
        "ari:cloud:confluence:site:space/99",
        "Some title",
    ] {
        assert!(
            is_usage(&lc::exact_content(&c, bad, None).unwrap_err()),
            "{bad}"
        );
    }
    assert!(srv.requests().is_empty());
}

#[test]
fn a_title_must_match_exactly_one_item() {
    let srv = MockServer::with_responses([
        ok(json!({"results": [{"id": "1"}]})),
        ok(json!({"results": [{"id": "2"}]})),
    ]);
    assert!(is_usage(
        &lc::exact_content(&srv.client(), "Weekly", Some("99")).unwrap_err()
    ));
}

#[test]
fn move_and_copy() {
    let srv = MockServer::with_responses([empty(), ok(json!({"id": "3"})), ok(json!({"id": "4"}))]);
    let c = srv.client();
    lc::move_content(&c, "1", lc::Position::Before, "2").unwrap();
    let to = |to| lc::CopyOpts {
        to,
        title: Some("Copy"),
        attachments: true,
        labels: false,
        permissions: false,
    };
    lc::copy(&c, "1", &to(lc::CopyTo::Parent("2"))).unwrap();
    lc::copy(&c, "1", &to(lc::CopyTo::Space("ENG"))).unwrap();
    assert!(lc::move_content(&c, "1", lc::Position::Append, "1").is_err());
    let r = srv.requests();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].path, "/rest/api/content/1/move/before/2");
    assert_eq!(
        r[1].json_body()["destination"],
        json!({"type": "parent_page", "value": "2"})
    );
    assert_eq!(r[1].json_body()["pageTitle"], "Copy");
    assert_eq!(
        r[2].json_body()["destination"],
        json!({"type": "space", "value": "ENG"})
    );
}

#[test]
fn purge_only_deletes_what_is_in_the_trash() {
    let srv = MockServer::with_responses([ok(json!({"status": "current"}))]);
    assert!(is_usage(&lc::purge(&srv.client(), "1").unwrap_err()));
    assert_eq!(srv.requests().len(), 1);

    let srv = MockServer::with_responses([ok(json!({"status": "trashed"})), empty()]);
    lc::purge(&srv.client(), "1").unwrap();
    let r = srv.requests();
    assert_eq!(r[0].query_param("status").as_deref(), Some("trashed"));
    assert_eq!(r[1].query_param("purge").as_deref(), Some("true"));
}

#[test]
fn trash_and_untrash() {
    let srv = MockServer::with_responses([
        empty(),
        ok(
            json!({"status": "trashed", "title": "T", "version": {"number": 4},
                  "body": {"storage": {"value": "<p>kept</p>"}}}),
        ),
        ok(json!({"status": "current"})),
    ]);
    let c = srv.client();
    lc::trash(&c, "1").unwrap();
    let out = lc::untrash(&c, "1").unwrap();
    assert_eq!(
        (&out["status"], &out["type"]),
        (&json!("current"), &json!("page"))
    );
    let r = srv.requests();
    assert_eq!(r[0].query, "");
    assert_eq!(r[1].path, "/api/v2/pages/1");
    assert_eq!(r[1].query_param("status").as_deref(), Some("trashed"));
    assert_eq!(r[1].query_param("body-format").as_deref(), Some("storage"));
    assert_eq!((r[2].method.as_str(), r[2].query.as_str()), ("PUT", ""));
    assert_eq!(r[2].path, "/api/v2/pages/1");
    assert_eq!(
        r[2].json_body(),
        json!({"id": "1", "title": "T", "status": "current", "version": {"number": 5},
               "body": {"representation": "storage", "value": "<p>kept</p>"}})
    );
}

#[test]
fn untrash_handles_blog_posts() {
    let srv = MockServer::with_responses([
        not_found(),
        ok(
            json!({"status": "trashed", "title": "B", "version": {"number": 1},
                  "body": {"storage": {"value": ""}}}),
        ),
        ok(json!({"status": "current"})),
    ]);
    assert_eq!(lc::untrash(&srv.client(), "1").unwrap()["type"], "blogpost");
    assert_eq!(srv.requests()[2].path, "/api/v2/blogposts/1");
}

#[test]
fn untrash_never_writes_a_body_it_did_not_read() {
    for now in [
        json!({"status": "trashed", "title": "T", "version": {"number": 4}}),
        json!({"status": "current", "title": "T", "version": {"number": 4},
               "body": {"storage": {"value": "<p>live</p>"}}}),
    ] {
        let srv = MockServer::with_responses([ok(now)]);
        assert!(lc::untrash(&srv.client(), "1").is_err());
        assert_eq!(srv.requests().len(), 1);
    }
}

#[test]
fn archive_polls_its_task() {
    let wait = Some(lc::Wait {
        timeout: Duration::from_secs(5),
        interval: Duration::from_millis(1),
    });
    let srv = MockServer::with_responses([
        MockResponse::json(202, json!({"id": "task-1"})),
        ok(json!({"finished": false})),
        ok(json!({"finished": true, "successful": true})),
    ]);
    assert_eq!(
        lc::archive(&srv.client(), "12", wait).unwrap()["archived"],
        true
    );
    let r = srv.requests();
    assert_eq!(r[0].json_body(), json!({"pages": [{"id": 12}]}));
    assert_eq!(r[2].path, "/rest/api/longtask/task-1");

    let srv = MockServer::with_responses([
        MockResponse::json(202, json!({"id": "task-1"})),
        ok(json!({"finished": true, "successful": false})),
    ]);
    assert!(lc::archive(&srv.client(), "12", wait).is_err());
}

#[test]
fn publish_numbers_the_version() {
    for (published, want) in [
        (not_found(), 1),
        (
            ok(json!({"status": "current", "version": {"number": 6}})),
            7,
        ),
    ] {
        let srv = MockServer::with_responses([
            ok(
                json!({"status": "draft", "title": "D", "body": {"storage": {"value": "<p>x</p>"}}}),
            ),
            published,
            ok(json!({"id": "1"})),
        ]);
        lc::publish(&srv.client(), "1", Some(ContentType::Page), None).unwrap();
        let body = srv.requests()[2].json_body();
        assert_eq!(body["version"]["number"], want);
        assert_eq!(body["body"]["value"], "<p>x</p>");
    }
}

#[test]
fn delete_draft_never_touches_published_content() {
    let draft = || ok(json!({"status": "draft"}));
    let srv = MockServer::with_responses([
        draft(),
        ok(json!({"status": "current", "version": {"number": 2}})),
    ]);
    assert!(is_usage(
        &lc::delete_draft(&srv.client(), "1", Some(ContentType::Page)).unwrap_err()
    ));
    assert_eq!(srv.requests().len(), 2);

    let srv = MockServer::with_responses([ok(json!({"status": "current"}))]);
    assert!(lc::delete_draft(&srv.client(), "1", Some(ContentType::Page)).is_err());
    assert_eq!(srv.requests().len(), 1);

    let srv = MockServer::with_responses([draft(), not_found(), empty()]);
    lc::delete_draft(&srv.client(), "1", Some(ContentType::Page)).unwrap();
    assert_eq!(
        srv.requests()[2].query_param("draft").as_deref(),
        Some("true")
    );
}

#[test]
fn private_create_is_one_request() {
    let srv = MockServer::with_responses([ok(json!({"id": "5"}))]);
    let new = confluence::NewContent {
        space: "99",
        content_type: ContentType::Page,
        title: "T",
        body: "x",
        format: TextFormat::Plain,
        parent: None,
        draft: false,
        private: true,
    };
    confluence::create(&srv.client(), new).unwrap();
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].query_param("private").as_deref(), Some("true"));
}

#[test]
fn appearance_sets_both_states() {
    assert_eq!(lc::emoji_code("👍").unwrap(), "1f44d");
    assert_eq!(lc::emoji_code("1F44D").unwrap(), "1f44d");
    for bad in ["", "a", "👍👍", "zz", "41"] {
        assert!(lc::emoji_code(bad).is_err(), "{bad}");
    }
    let srv =
        MockServer::with_responses([ok(json!({})), ok(json!({})), ok(json!({})), ok(json!({}))]);
    lc::set_appearance(
        &srv.client(),
        ContentType::Page,
        "5",
        Some("1f44d"),
        Some(lc::Width::Wide),
    )
    .unwrap();
    let r = srv.requests();
    assert_eq!(
        r[0].json_body(),
        json!({"key": "emoji-title-published", "value": "1f44d"})
    );
    assert_eq!(
        r[3].json_body(),
        json!({"key": "content-appearance-draft", "value": "full-width"})
    );
}

#[test]
fn space_create_and_dry_run() {
    let new = lc::NewSpace {
        key: "NEW",
        name: "New",
        description: Some("d"),
        private: true,
        dry_run: true,
    };
    let srv = MockServer::with_responses([]);
    assert_eq!(
        lc::space_create(&srv.client(), &new).unwrap()["dryRun"],
        true
    );
    assert!(srv.requests().is_empty());

    let srv = MockServer::with_responses([ok(json!({"key": "NEW"}))]);
    lc::space_create(
        &srv.client(),
        &lc::NewSpace {
            dry_run: false,
            ..new
        },
    )
    .unwrap();
    assert_eq!(srv.requests()[0].path, "/rest/api/space/_private");
}

#[test]
fn space_update_keeps_name_and_description() {
    let srv = MockServer::with_responses([
        ok(json!({"key": "ENG", "name": "Eng", "description": {"plain": {"value": "docs"}}})),
        ok(json!({})),
    ]);
    let change = lc::SpaceChange {
        status: Some("archived"),
        ..Default::default()
    };
    lc::space_update(&srv.client(), "99", &change).unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.path, "/rest/api/space/ENG");
    assert_eq!(
        r.json_body(),
        json!({"name": "Eng", "status": "archived",
               "description": {"plain": {"value": "docs", "representation": "plain"}}})
    );
}

#[test]
fn space_delete_uses_the_key_the_server_gave() {
    let srv = MockServer::with_responses([
        ok(json!({"id": "99", "key": "ENG"})),
        MockResponse::json(202, json!({"id": "task-2"})),
    ]);
    assert_eq!(
        lc::space_delete(&srv.client(), "99").unwrap()["taskId"],
        "task-2"
    );
    assert_eq!(srv.requests()[1].path, "/rest/api/space/ENG");
}

#[test]
fn tree_of_a_space_starts_at_its_root_pages() {
    let srv = MockServer::with_responses([
        not_found(),
        ok(json!({"id": "99", "key": "ENG", "name": "Eng"})),
        ok(json!({"results": [{"id": "1", "title": "Home", "status": "current"}]})),
        ok(json!({"results": [{"id": "2", "title": "Child", "type": "page"}]})),
    ]);
    let out = tree::tree(&srv.client(), "99", 2, None).unwrap();
    assert_eq!(out["type"], "space");
    assert_eq!(out["children"][0]["children"][0]["id"], "2");
    assert_eq!(
        srv.requests()[2].query_param("depth").as_deref(),
        Some("root")
    );
}

#[test]
fn tree_stops_at_the_node_cap() {
    let many: Vec<Value> = (0..=tree::MAX_NODES)
        .map(|i| json!({"id": i.to_string(), "type": "page"}))
        .collect();
    let srv = MockServer::with_responses([
        ok(json!({"id": "10"})),
        ok(json!({"results": []})),
        ok(json!({"results": many})),
    ]);
    let out = tree::tree(&srv.client(), "10", 3, None).unwrap();
    assert_eq!(out["children"].as_array().unwrap().len(), tree::MAX_NODES);
    assert_eq!(out["truncated"], true);
    assert_eq!(srv.requests().len(), 3);
}

#[test]
fn tree_lists_only_pages_without_a_type() {
    let srv = MockServer::with_responses([
        ok(json!({"id": "10"})),
        ok(json!({"results": []})),
        ok(json!({"results": [{"id": "20", "type": "folder"}, {"id": "11", "type": "page"}]})),
    ]);
    let out = tree::tree(&srv.client(), "10", 1, None).unwrap();
    assert_eq!(out["children"].as_array().unwrap().len(), 1);
    assert_eq!(out["children"][0]["id"], "11");
    assert!(
        srv.requests()[2]
            .path
            .ends_with("/pages/10/direct-children")
    );
}

#[test]
fn tree_type_keeps_the_containers() {
    let srv = MockServer::with_responses([
        ok(json!({"id": "10"})),
        ok(json!({"results": []})),
        ok(json!({"results": [{"id": "11", "type": "page"}, {"id": "12", "type": "page"}]})),
        ok(json!({"results": [{"id": "20", "type": "whiteboard"}]})),
        ok(json!({"results": []})),
    ]);
    let out = tree::tree(&srv.client(), "10", 2, Some(tree::NodeType::Whiteboard)).unwrap();
    let top = out["children"].as_array().unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0]["children"][0]["id"], "20");
    assert!(
        srv.requests()[2]
            .path
            .ends_with("/pages/10/direct-children")
    );
}

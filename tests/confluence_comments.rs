//! Offline tests for Confluence comment threads (#29).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{ContentType, ReadFormat, comments};
use atlx::markup::TextFormat;
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn not_found() -> MockResponse {
    MockResponse::json(
        404,
        json!({"errors": [{"status": 404, "title": "Not Found"}]}),
    )
}

fn comment(id: &str) -> Value {
    json!({"id": id, "version": {"number": 3}, "body": {"storage": {"value": "<p>hi</p>"}}})
}

fn kind(e: &anyhow::Error) -> Option<ErrorKind> {
    ApiError::find(e).map(|a| a.kind)
}

fn opts(surface: comments::Surface) -> comments::ListOpts<'static> {
    comments::ListOpts {
        surface,
        format: ReadFormat::Md,
        limit: 25,
        cursor: None,
        sort: None,
        status: None,
        resolution: None,
        content_type: None,
        replies: false,
    }
}

#[test]
fn comment_ids_never_come_from_the_page_part_of_a_url() {
    let page = "https://x.test/wiki/spaces/E/pages/12345/Title";
    for (input, want) in [
        ("777".to_string(), "777"),
        ("ari:cloud:confluence:site:comment/777".to_string(), "777"),
        (format!("{page}?focusedCommentId=777"), "777"),
        (format!("{page}#comment-777"), "777"),
    ] {
        assert_eq!(comments::comment_id(&input).unwrap(), want, "{input}");
    }
    for bad in [
        page,
        "ari:cloud:confluence:site:page/12345",
        "..",
        "",
        "abc",
    ] {
        let err = comments::comment_id(bad).unwrap_err();
        assert_eq!(kind(&err), Some(ErrorKind::Usage), "{bad}");
    }
}

#[test]
fn list_sends_filters_and_open_takes_in_reopened_and_dangling() {
    let srv = MockServer::with_responses([ok(json!({"results": [comment("1")]}))]);
    let o = comments::ListOpts {
        sort: Some(comments::Sort::Newest),
        status: Some("current"),
        resolution: Some(comments::Resolution::Open),
        content_type: Some(ContentType::Blogpost),
        ..opts(comments::Surface::Inline)
    };
    let out = comments::list(&srv.client(), "12345", None, &o).unwrap();
    assert_eq!(out["results"][0]["commentType"], "inline");
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/api/v2/blogposts/12345/inline-comments");
    assert_eq!(r.query_param("sort").as_deref(), Some("-created-date"));
    assert_eq!(r.query_param("status").as_deref(), Some("current"));
    for state in ["open", "reopened", "dangling"] {
        assert!(
            r.query.contains(&format!("resolution-status={state}")),
            "{}",
            r.query
        );
    }
}

#[test]
fn list_refuses_resolution_on_footer_comments() {
    let srv = MockServer::with_responses([]);
    let o = comments::ListOpts {
        resolution: Some(comments::Resolution::Resolved),
        ..opts(comments::Surface::Footer)
    };
    let err = comments::list(&srv.client(), "12345", None, &o).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Usage));
    assert!(srv.requests().is_empty());
}

#[test]
fn replies_nest_and_follow_cursors() {
    let srv = MockServer::with_responses([
        ok(json!({"results": [comment("1")]})),
        ok(json!({"results": [comment("2")], "_links": {"next": "/x?cursor=c2"}})),
        ok(json!({"results": [comment("3")]})),
        ok(json!({"results": [comment("4")]})),
        ok(json!({"results": []})),
        ok(json!({"results": []})),
    ]);
    let o = comments::ListOpts {
        replies: true,
        ..opts(comments::Surface::Footer)
    };
    let out = comments::list(&srv.client(), "12345", None, &o).unwrap();
    let replies = &out["results"][0]["replies"];
    assert_eq!(replies[0]["id"], "2");
    assert_eq!(replies[1]["id"], "3");
    assert_eq!(replies[0]["replies"][0]["id"], "4");
    let r = srv.requests();
    assert_eq!(r[1].path, "/api/v2/footer-comments/1/children");
    assert_eq!(r[2].query_param("cursor").as_deref(), Some("c2"));
    assert_eq!(r[3].path, "/api/v2/footer-comments/2/children");
}

#[test]
fn replies_stop_at_the_cap_and_say_so() {
    let many: Vec<Value> = (0..=comments::MAX_REPLIES)
        .map(|i| comment(&format!("{i}")))
        .collect();
    let srv = MockServer::with_responses([ok(comment("9999")), ok(json!({"results": many}))]);
    let out = comments::get(&srv.client(), "9999", None, ReadFormat::Md, true).unwrap();
    assert_eq!(
        out["replies"].as_array().unwrap().len(),
        comments::MAX_REPLIES
    );
    assert_eq!(out["repliesTruncated"], true);
    assert_eq!(out["replies"][0]["repliesTruncated"], true);
    assert_eq!(srv.requests().len(), 2);
}

#[test]
fn get_detects_inline_comments() {
    let srv = MockServer::with_responses([not_found(), ok(comment("777"))]);
    let out = comments::get(&srv.client(), "777", None, ReadFormat::Md, false).unwrap();
    assert_eq!(out["commentType"], "inline");
    assert_eq!(out["body"]["value"], "hi");
    assert_eq!(srv.requests()[1].path, "/api/v2/inline-comments/777");
}

#[test]
fn inline_create_counts_matches_in_the_visible_text() {
    let doc = json!({"type": "doc", "content": [
        {"type": "paragraph", "content": [
            {"type": "text", "text": "deploy the "},
            {"type": "text", "text": "service", "marks": [{"type": "strong"}]}]},
        {"type": "paragraph", "content": [{"type": "text", "text": "then test the service"}]}
    ]});
    let srv = MockServer::with_responses([
        ok(json!({"id": "12345", "body": {"atlas_doc_format": {"value": doc.to_string()}}})),
        MockResponse::json(201, json!({"id": "9"})),
    ]);
    let sel = comments::Selection {
        text: "the service",
        count: None,
        index: Some(1),
    };
    let out = comments::create(
        &srv.client(),
        "12345",
        None,
        "why?",
        TextFormat::Plain,
        Some(sel),
    )
    .unwrap();
    assert_eq!(out["commentType"], "inline");
    let r = srv.requests();
    assert_eq!(
        r[0].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
    assert_eq!(r[1].path, "/api/v2/inline-comments");
    let body = r[1].json_body();
    assert_eq!(body["pageId"], "12345");
    assert_eq!(
        body["inlineCommentProperties"],
        json!({"textSelection": "the service", "textSelectionMatchCount": 2, "textSelectionMatchIndex": 1})
    );
}

#[test]
fn inline_create_never_posts_a_bad_anchor() {
    let doc = json!({"type": "doc", "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": "once"}]}
    ]});
    for (text, index) in [("absent", None), ("once", Some(1))] {
        let srv = MockServer::with_responses([ok(
            json!({"body": {"atlas_doc_format": {"value": doc.to_string()}}}),
        )]);
        let sel = comments::Selection {
            text,
            count: None,
            index,
        };
        let err = comments::create(
            &srv.client(),
            "12345",
            None,
            "x",
            TextFormat::Plain,
            Some(sel),
        )
        .unwrap_err();
        assert_eq!(kind(&err), Some(ErrorKind::Usage), "{text}");
        assert_eq!(srv.requests().len(), 1);
    }
    let srv = MockServer::with_responses([]);
    let sel = comments::Selection {
        text: "once",
        count: Some(2),
        index: Some(2),
    };
    assert!(
        comments::create(
            &srv.client(),
            "12345",
            None,
            "x",
            TextFormat::Plain,
            Some(sel)
        )
        .is_err()
    );
    assert!(srv.requests().is_empty());
}

#[test]
fn reply_goes_to_the_parents_surface() {
    let srv = MockServer::with_responses([not_found(), ok(comment("777")), ok(json!({"id": "8"}))]);
    comments::reply(&srv.client(), "777", "Thanks", TextFormat::Plain).unwrap();
    let r = &srv.requests()[2];
    assert_eq!(r.path, "/api/v2/inline-comments");
    assert_eq!(r.json_body()["parentCommentId"], "777");
}

#[test]
fn update_bumps_the_version_and_honours_the_expected_one() {
    let srv = MockServer::with_responses([ok(comment("777"))]);
    let err = comments::update(
        &srv.client(),
        "777",
        None,
        "new",
        TextFormat::Plain,
        Some(2),
    )
    .unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Conflict));
    assert_eq!(srv.requests().len(), 1);

    let srv = MockServer::with_responses([ok(comment("777")), ok(comment("777"))]);
    comments::update(
        &srv.client(),
        "777",
        None,
        "new",
        TextFormat::Plain,
        Some(3),
    )
    .unwrap();
    let r = &srv.requests()[1];
    assert_eq!(r.method, "PUT");
    assert_eq!(r.path, "/api/v2/footer-comments/777");
    assert_eq!(r.json_body()["version"]["number"], 4);
}

#[test]
fn delete_uses_the_detected_surface() {
    let srv =
        MockServer::with_responses([not_found(), ok(comment("777")), MockResponse::empty(204)]);
    comments::delete(&srv.client(), "777").unwrap();
    let r = &srv.requests()[2];
    assert_eq!(r.method, "DELETE");
    assert_eq!(r.path, "/api/v2/inline-comments/777");
}

#[test]
fn resolve_and_reopen_send_the_body_back_unchanged() {
    for resolved in [true, false] {
        let srv = MockServer::with_responses([not_found(), ok(comment("777")), ok(comment("777"))]);
        comments::set_resolved(&srv.client(), "777", None, resolved).unwrap();
        assert_eq!(
            srv.requests()[2].json_body(),
            json!({"version": {"number": 4}, "resolved": resolved,
                   "body": {"representation": "storage", "value": "<p>hi</p>"}})
        );
    }
}

#[test]
fn resolve_refuses_footer_comments_and_missing_bodies() {
    let srv = MockServer::with_responses([ok(comment("777"))]);
    let err = comments::set_resolved(&srv.client(), "777", None, true).unwrap_err();
    assert_eq!(kind(&err), Some(ErrorKind::Usage));
    assert_eq!(srv.requests().len(), 1);

    let srv = MockServer::with_responses([ok(json!({"id": "777", "version": {"number": 1}}))]);
    let inline = Some(comments::Surface::Inline);
    assert!(comments::set_resolved(&srv.client(), "777", inline, true).is_err());
    assert_eq!(srv.requests().len(), 1, "a body must never be blanked");
}

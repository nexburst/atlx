//! Offline tests for Confluence versions, labels, attachments, the
//! body-format guide and `content get --format url` (#28).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{self, Detail, GetOpts, ReadFormat, attachments, formats, history, labels};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::path::PathBuf;

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn not_found() -> MockResponse {
    MockResponse::json(
        404,
        json!({"errors": [{"status": 404, "title": "Not Found"}]}),
    )
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn storage(title: &str, body: &str) -> MockResponse {
    ok(json!({"id": "12345", "title": title, "body": {"storage": {"value": body}}}))
}

#[test]
fn versions_list_is_newest_first_with_a_cursor() {
    let srv = MockServer::with_responses([ok(json!({
        "results": [{"number": 3}, {"number": 2}],
        "_links": {"next": "/wiki/api/v2/pages/12345/versions?cursor=abc&limit=2"}
    }))]);
    let out = history::list(&srv.client(), "12345", None, 2, Some("start")).unwrap();
    assert_eq!(out["id"], "12345");
    assert_eq!(out["type"], "page");
    assert_eq!(out["results"][0]["number"], 3);
    assert_eq!(out["nextCursor"], "abc");
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/api/v2/pages/12345/versions");
    assert_eq!(r.query_param("sort").as_deref(), Some("-modified-date"));
    assert_eq!(r.query_param("limit").as_deref(), Some("2"));
    assert_eq!(r.query_param("cursor").as_deref(), Some("start"));
}

#[test]
fn versions_get_falls_back_to_blog_posts_and_reads_that_body() {
    let srv = MockServer::with_responses([
        not_found(),
        ok(json!({"number": 2, "message": "typo"})),
        storage("Launch", "<h2>Plan</h2><p>Ship it</p>"),
    ]);
    let out = history::get(&srv.client(), "12345", None, 2, Some(ReadFormat::Md)).unwrap();
    assert_eq!(out["type"], "blogpost");
    assert_eq!(out["version"]["message"], "typo");
    assert_eq!(out["title"], "Launch");
    assert!(out["body"]["value"].as_str().unwrap().contains("## Plan"));
    let r = srv.requests();
    assert_eq!(r[1].path, "/api/v2/blogposts/12345/versions/2");
    assert_eq!(r[2].path, "/api/v2/blogposts/12345");
    assert_eq!(r[2].query_param("version").as_deref(), Some("2"));
    assert_eq!(r[2].query_param("body-format").as_deref(), Some("storage"));
}

#[test]
fn versions_diff_is_a_unified_diff_of_both_bodies() {
    let srv = MockServer::with_responses([
        storage("Runbook", "<p>same</p><p>old line</p>"),
        storage("Runbook", "<p>same</p><p>new line</p>"),
    ]);
    let out = history::diff(&srv.client(), "12345", None, 1, 2, ReadFormat::Md).unwrap();
    assert_eq!(out["changed"], true);
    assert_eq!(out["titleChanged"], false);
    let diff = out["diff"].as_str().unwrap();
    assert!(diff.contains("-old line"), "{diff}");
    assert!(diff.contains("+new line"), "{diff}");
    let r = srv.requests();
    assert_eq!(r[0].query_param("version").as_deref(), Some("1"));
    assert_eq!(r[1].query_param("version").as_deref(), Some("2"));
}

#[test]
fn versions_diff_reports_a_title_only_change_and_rejects_version_zero() {
    let srv = MockServer::with_responses([storage("A", "<p>x</p>"), storage("B", "<p>x</p>")]);
    let c = srv.client();
    assert!(is_usage(
        &history::diff(&c, "12345", None, 0, 2, ReadFormat::Md).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
    let out = history::diff(&c, "12345", None, 1, 2, ReadFormat::Md).unwrap();
    assert_eq!(out["changed"], true);
    assert_eq!(out["titleChanged"], true);
    assert_eq!(out["diff"], "");
}

#[test]
fn versions_diff_in_html_compares_adf_blocks() {
    let page = |text: &str| {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": text}]}
        ]});
        ok(json!({"title": "T", "body": {"atlas_doc_format": {"value": doc.to_string()}}}))
    };
    let srv = MockServer::with_responses([page("one"), page("two")]);
    let out = history::diff(&srv.client(), "12345", None, 1, 2, ReadFormat::Html).unwrap();
    assert!(
        out["diff"].as_str().unwrap().contains("+<p>two</p>"),
        "{out}"
    );
    let r = &srv.requests()[0];
    assert_eq!(
        r.query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
}

#[test]
fn versions_restore_posts_the_v1_operation() {
    let srv = MockServer::with_responses([ok(json!({"number": 8, "message": "undo"}))]);
    let out = history::restore(&srv.client(), "12345", None, 5, Some("undo"), true).unwrap();
    assert_eq!(out["restoredFrom"], 5);
    assert_eq!(out["version"]["number"], 8);
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/rest/api/content/12345/version");
    assert_eq!(
        r.json_body(),
        json!({"operationKey": "restore",
               "params": {"versionNumber": 5, "message": "undo", "restoreTitle": true}})
    );
}

#[test]
fn labels_list_refuses_what_rest_cannot_list() {
    let body = || ok(json!({"results": [{"id": "1", "name": "runbook", "prefix": "global"}]}));
    let srv = MockServer::with_responses([body()]);
    let c = srv.client();
    let page = labels::list(&c, "12345", None, None, 250).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/api/v2/pages/12345/labels");
    assert!(page["results"][0]["id"].is_string());
    for target in [
        labels::LabelTarget::Whiteboard,
        labels::LabelTarget::Database,
        labels::LabelTarget::Folder,
    ] {
        assert!(labels::list(&c, "12345", None, Some(target), 250).is_err());
    }
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn labels_add_splits_commas() {
    let srv = MockServer::with_responses([ok(json!({"results": [{"id": "1", "name": "a"}]}))]);
    let names = ["a, b".to_string(), "c".to_string()];
    let out = labels::add(&srv.client(), "12345", None, &names, "global").unwrap();
    assert_eq!(out["added"], json!(["a", "b", "c"]));
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/rest/api/content/12345/label");
    assert_eq!(
        r.json_body(),
        json!([{"prefix": "global", "name": "a"}, {"prefix": "global", "name": "b"},
               {"prefix": "global", "name": "c"}])
    );
}

#[test]
fn labels_remove_deletes_each_name_by_query() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    labels::remove(&c, "12345", None, &["a/b".into(), "x".into()], "my").unwrap();
    let r = srv.requests();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].method, "DELETE");
    assert_eq!(r[0].path, "/rest/api/content/12345/label");
    assert_eq!(r[0].query_param("name").as_deref(), Some("my:a/b"));
    assert!(is_usage(
        &labels::remove(&c, "12345", None, &[" , ".into()], "global").unwrap_err()
    ));
    assert_eq!(srv.requests().len(), 2);
}

#[test]
fn label_watch_sends_the_xsrf_header() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    assert_eq!(
        labels::watch(&c, "runbook", true).unwrap()["watching"],
        true
    );
    assert_eq!(
        labels::watch(&c, "runbook", false).unwrap()["watching"],
        false
    );
    let r = srv.requests();
    assert_eq!(r[0].method, "POST");
    assert_eq!(r[0].path, "/rest/api/user/watch/label/runbook");
    assert_eq!(r[0].header("x-atlassian-token"), Some("no-check"));
    assert_eq!(r[1].method, "DELETE");
}

#[test]
fn attachment_ids_come_from_ids_numbers_and_urls() {
    for (input, want) in [
        ("att456", "att456"),
        ("456", "att456"),
        (
            "https://x.test/wiki/pages/viewpageattachments.action?attachmentId=456",
            "att456",
        ),
        (
            "https://x.test/wiki/spaces/E/pages/12345?preview=/12345/att456/a.png",
            "att456",
        ),
    ] {
        assert_eq!(attachments::attachment_id(input).unwrap(), want, "{input}");
    }
    for bad in ["", "..", "att", "attx", "report.pdf"] {
        assert!(
            is_usage(&attachments::attachment_id(bad).unwrap_err()),
            "{bad}"
        );
    }
}

#[test]
fn attachments_list_passes_filters() {
    let srv = MockServer::with_responses([ok(json!({
        "results": [{"id": "att456", "title": "a.png", "_links": {"webui": "/a"}}]
    }))]);
    let c = srv.client();
    let o = attachments::ListOpts {
        limit: 10,
        cursor: None,
        sort: Some("title"),
        status: Some("current"),
        media_type: Some("image/png"),
        filename: Some("a.png"),
    };
    let out = attachments::list(&c, "12345", None, &o).unwrap();
    assert_eq!(out["results"][0]["externalUrl"], format!("{}/a", c.base()));
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/api/v2/pages/12345/attachments");
    for (k, v) in [
        ("limit", "10"),
        ("sort", "title"),
        ("mediaType", "image/png"),
        ("filename", "a.png"),
    ] {
        assert_eq!(r.query_param(k).as_deref(), Some(v), "{k}");
    }
}

#[test]
fn attachment_download_follows_the_link_and_keeps_existing_files() {
    let meta = || {
        ok(
            json!({"id": "att456", "title": "a.txt", "version": {"number": 2},
                  "downloadLink": "/download/attachments/12345/a.txt?version=2"}),
        )
    };
    let srv = MockServer::with_responses([meta(), MockResponse::text(200, "content"), meta()]);
    let dir = temp_dir("download");
    let dest = dir.join("a.txt");
    let c = srv.client();
    let out = attachments::download(&c, "att456", Some(&dest), Some(2), false).unwrap();
    assert_eq!(out["bytes"], 7);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "content");
    assert!(attachments::download(&c, "att456", Some(&dest), None, false).is_err());
    let r = srv.requests();
    assert_eq!(r[0].query_param("version").as_deref(), Some("2"));
    assert_eq!(r[1].path, "/download/attachments/12345/a.txt");
    assert_eq!(r.len(), 3, "an existing file is refused before downloading");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachment_download_never_takes_an_unsafe_name_from_the_server() {
    for title in [
        ".bashrc",
        "../.envrc",
        "..\\.profile",
        "..",
        "dir/",
        "C:evil",
        "a\nb",
        "",
    ] {
        let srv = MockServer::with_responses([ok(json!({
            "id": "att456", "title": title, "downloadLink": "/download/attachments/1/x"
        }))]);
        // Even with --force.
        let err = attachments::download(&srv.client(), "att456", None, None, true).unwrap_err();
        assert!(is_usage(&err), "{title:?}");
        assert_eq!(srv.requests().len(), 1, "{title:?} was downloaded");
    }
}

#[test]
fn attachment_upload_creates_then_updates_by_file_name() {
    let dir = temp_dir("upload");
    let file = dir.join("notes.txt");
    std::fs::write(&file, "hello upload").unwrap();
    let srv = MockServer::with_responses([
        ok(json!({"results": []})),
        ok(json!({"results": [{"id": "att9", "title": "notes.txt", "version": {"number": 1}}]})),
        ok(json!({"results": [{"id": "att9", "title": "notes.txt"}]})),
        ok(json!({"id": "att9", "title": "notes.txt", "version": {"number": 2}})),
    ]);
    let c = srv.client();
    let first = attachments::upload(&c, "12345", None, &file, Some("v1"), false).unwrap();
    assert_eq!(first["action"], "created");
    let second = attachments::upload(&c, "12345", None, &file, None, true).unwrap();
    assert_eq!(second["action"], "updated");
    assert_eq!(second["version"], 2);

    let r = srv.requests();
    assert_eq!(r[0].query_param("filename").as_deref(), Some("notes.txt"));
    assert_eq!(r[1].method, "POST");
    assert_eq!(r[1].path, "/rest/api/content/12345/child/attachment");
    assert_eq!(r[1].header("x-atlassian-token"), Some("nocheck"));
    assert!(r[1].body.contains("hello upload"));
    assert!(r[1].body.contains("name=\"comment\"\r\n\r\nv1"));
    assert_eq!(
        r[3].path,
        "/rest/api/content/12345/child/attachment/att9/data"
    );
    assert!(r[3].body.contains("name=\"minorEdit\"\r\n\r\ntrue"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachment_delete_trashes_unless_purged() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    attachments::delete(&c, "att456", false).unwrap();
    assert_eq!(
        attachments::delete(&c, "456", true).unwrap()["purged"],
        true
    );
    let r = srv.requests();
    assert_eq!(r[0].path, "/api/v2/attachments/att456");
    assert_eq!(r[0].query_param("purge"), None);
    assert_eq!(r[1].query_param("purge").as_deref(), Some("true"));
}

#[test]
fn get_url_returns_only_the_address() {
    let srv = MockServer::with_responses([ok(json!({
        "id": "12345", "title": "Runbook", "_links": {"webui": "/spaces/ENG/pages/12345"}
    }))]);
    let c = srv.client();
    let o = GetOpts {
        detail: Detail::Summary,
        format: ReadFormat::Html,
        draft: false,
        reference_space: None,
        sections: &[],
    };
    let out = confluence::get_url(&c, "12345", &o).unwrap();
    assert_eq!(out["url"], format!("{}/spaces/ENG/pages/12345", c.base()));
    assert!(out.get("body").is_none());
}

#[test]
fn body_format_guide_covers_formats_and_types() {
    assert_eq!(formats::guide(None, None).unwrap()["format"], "index");
    assert_eq!(
        formats::guide(Some("markdown"), None).unwrap()["format"],
        "md"
    );
    assert_eq!(
        formats::guide(Some("nonsense"), None).unwrap()["format"],
        "index"
    );
    let board = formats::guide(None, Some(formats::GuideType::Whiteboard)).unwrap();
    assert_eq!(board["validFormats"], json!([]));
    assert!(is_usage(
        &formats::guide(Some("md"), Some(formats::GuideType::Page)).unwrap_err()
    ));
}

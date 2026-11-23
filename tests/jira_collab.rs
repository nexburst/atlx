//! Offline tests for Jira attachments, worklogs, watchers and votes (#24).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::{attachments, people, worklogs};
use atlx::time;
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::path::PathBuf;

const ACCOUNT: &str = "5b10ac8d82e05b22cc7d4ef5";

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn is_usage(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::Usage)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlx-jira-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn attachment_list_and_get() {
    let file =
        || json!({"id": "10001", "filename": "a.txt", "author": {"avatarUrls": {"48x48": "x"}}});
    let srv =
        MockServer::with_responses([ok(json!({"fields": {"attachment": [file()]}})), ok(file())]);
    let c = srv.client();
    let list = attachments::list(&c, "PROJ-1").unwrap();
    assert_eq!(list[0]["id"], "10001");
    assert_eq!(attachments::get(&c, "10001").unwrap()["filename"], "a.txt");
    let r = srv.requests();
    assert_eq!(r[0].query_param("fields").as_deref(), Some("attachment"));
    assert_eq!(r[1].path, "/rest/api/3/attachment/10001");
}

#[test]
fn attachment_upload_sends_every_file_as_multipart() {
    let dir = temp_dir("upload");
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    std::fs::write(&a, "alpha contents").unwrap();
    std::fs::write(&b, "beta contents").unwrap();
    let srv = MockServer::with_responses([ok(json!([{"id": "1"}, {"id": "2"}]))]);
    let c = srv.client();
    let missing = dir.join("missing");
    assert!(is_usage(
        &attachments::upload(&c, "PROJ-1", &[a.clone(), missing]).unwrap_err()
    ));
    assert!(
        srv.requests().is_empty(),
        "nothing is sent when a file is missing"
    );

    attachments::upload(&c, "PROJ-1", &[a, b]).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/rest/api/3/issue/PROJ-1/attachments");
    assert_eq!(r.header("x-atlassian-token"), Some("no-check"));
    assert!(
        r.header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(r.body.contains("name=\"file\"; filename=\"a.txt\""));
    assert!(r.body.contains("name=\"file\"; filename=\"b.txt\""));
    assert!(r.body.contains("alpha contents") && r.body.contains("beta contents"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachment_downloads_drop_auth_on_the_media_origin() {
    for (what, segment) in [
        (attachments::Bytes::Content, "content"),
        (attachments::Bytes::Thumbnail, "thumbnail"),
    ] {
        let media = MockServer::with_responses([MockResponse::text(200, "bytes")]);
        let jira = MockServer::with_responses([MockResponse::empty(303)
            .with_header("Location", format!("{}/file/abc?token=t", media.base_url()))]);
        let dir = temp_dir(segment);
        let dest = dir.join("out.bin");
        let out = attachments::download(&jira.client(), "10001", what, Some(&dest), false).unwrap();
        assert_eq!(out["bytes"], 5);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "bytes");
        let (first, second) = (&jira.requests()[0], &media.requests()[0]);
        assert_eq!(
            first.path,
            format!("/rest/api/3/attachment/{segment}/10001")
        );
        assert!(first.header("authorization").is_some());
        assert_eq!(second.header("authorization"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn attachment_download_never_takes_an_unsafe_name_from_the_server() {
    for name in [".bashrc", "../.envrc", "..\\.profile", "C:evil", "", "a\nb"] {
        let srv = MockServer::with_responses([ok(json!({"filename": name}))]);
        let what = attachments::Bytes::Content;
        let err = attachments::download(&srv.client(), "1", what, None, true).unwrap_err();
        assert!(is_usage(&err), "{name:?}");
        assert_eq!(srv.requests().len(), 1, "{name:?} was downloaded");
    }
}

#[test]
fn attachment_delete_hits_the_attachment() {
    let srv = MockServer::with_responses([MockResponse::empty(204)]);
    let c = srv.client();
    assert!(attachments::delete(&c, "..").is_err());
    assert_eq!(attachments::delete(&c, "10001").unwrap()["deleted"], true);
    assert_eq!(srv.requests()[0].path, "/rest/api/3/attachment/10001");
}

#[test]
fn watcher_add_sends_the_account_as_a_json_string() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    people::watcher_add(&c, "PROJ-1", ACCOUNT).unwrap();
    let ari = format!("ari:cloud:identity::user/{ACCOUNT}");
    people::watcher_remove(&c, "PROJ-1", &ari).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issue/PROJ-1/watchers");
    assert_eq!(r[0].json_body(), json!(ACCOUNT));
    assert_eq!(r[1].method, "DELETE");
    assert_eq!(r[1].query_param("accountId").as_deref(), Some(ACCOUNT));
}

#[test]
fn watcher_defaults_resolve_me_and_never_send_an_empty_account() {
    let srv =
        MockServer::with_responses([ok(json!({"accountId": ACCOUNT})), MockResponse::empty(204)]);
    let out = people::watcher_add(&srv.client(), "PROJ-1", "me").unwrap();
    assert_eq!(out["accountId"], ACCOUNT);
    assert_eq!(srv.requests()[0].path, "/rest/api/3/myself");

    let srv = MockServer::with_responses([]);
    assert!(people::watcher_remove(&srv.client(), "PROJ-1", "ari:cloud:identity::user/").is_err());
    assert!(srv.requests().is_empty());
}

#[test]
fn watchers_and_votes_use_their_endpoints() {
    let srv = MockServer::with_responses([
        ok(json!({"watchCount": 1, "watchers": [{"accountId": "a"}]})),
        ok(json!({"votes": 2})),
        MockResponse::empty(204),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    assert_eq!(people::watchers(&c, "PROJ-1").unwrap()[0]["accountId"], "a");
    assert_eq!(people::votes(&c, "PROJ-1").unwrap()["votes"], 2);
    people::vote(&c, "PROJ-1").unwrap();
    people::unvote(&c, "PROJ-1").unwrap();
    let r = srv.requests();
    assert_eq!((r[2].method.as_str(), r[2].body.as_str()), ("POST", ""));
    assert_eq!(r[3].method, "DELETE");
    assert!(
        r[1..]
            .iter()
            .all(|x| x.path == "/rest/api/3/issue/PROJ-1/votes")
    );
}

#[test]
fn worklog_add_sends_body_and_estimate_options() {
    let srv = MockServer::with_responses([MockResponse::json(201, json!({"id": "7"}))]);
    let entry = worklogs::Entry {
        time_spent: Some("1h 30m"),
        started: Some(time::epoch_millis("2026-01-05T10:00:00+02:00").unwrap()),
        comment: Some(atlx::markup::text_to_adf("Pairing")),
        ..Default::default()
    };
    let opts = worklogs::WriteOpts {
        adjust: Some(worklogs::Adjust::Manual),
        reduce_by: Some("1h"),
        notify_users: Some(false),
        override_editable: true,
        ..Default::default()
    };
    worklogs::add(&srv.client(), "PROJ-1", &entry, &opts).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/issue/PROJ-1/worklog");
    let body = r.json_body();
    assert_eq!(body["timeSpent"], "1h 30m");
    assert_eq!(body["started"], "2026-01-05T08:00:00.000+0000");
    assert_eq!(body["comment"]["type"], "doc");
    for (k, v) in [
        ("adjustEstimate", "manual"),
        ("reduceBy", "1h"),
        ("notifyUsers", "false"),
        ("overrideEditableFlag", "true"),
    ] {
        assert_eq!(r.query_param(k).as_deref(), Some(v), "{k}");
    }
}

#[test]
fn worklog_writes_validate_before_sending() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    let none = worklogs::WriteOpts::default();
    let empty = worklogs::Entry::default();
    assert!(is_usage(
        &worklogs::add(&c, "P-1", &empty, &none).unwrap_err()
    ));
    assert!(is_usage(
        &worklogs::update(&c, "P-1", "7", &empty, &none).unwrap_err()
    ));
    for adjust in [worklogs::Adjust::New, worklogs::Adjust::Manual] {
        let o = worklogs::WriteOpts {
            adjust: Some(adjust),
            ..Default::default()
        };
        assert!(is_usage(&worklogs::delete(&c, "P-1", "7", &o).unwrap_err()));
    }
    assert!(srv.requests().is_empty());
}

#[test]
fn worklog_update_get_and_delete_address_one_worklog() {
    let srv = MockServer::with_responses([
        ok(json!({"id": "7"})),
        ok(json!({"id": "7"})),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    let entry = worklogs::Entry {
        seconds: Some(3600),
        ..Default::default()
    };
    worklogs::update(&c, "PROJ-1", "7", &entry, &Default::default()).unwrap();
    worklogs::get(&c, "PROJ-1", "7", Some("properties")).unwrap();
    let o = worklogs::WriteOpts {
        adjust: Some(worklogs::Adjust::Manual),
        increase_by: Some("1h"),
        ..Default::default()
    };
    worklogs::delete(&c, "PROJ-1", "7", &o).unwrap();
    let r = srv.requests();
    assert!(
        r.iter()
            .all(|x| x.path == "/rest/api/3/issue/PROJ-1/worklog/7")
    );
    assert_eq!(r[0].json_body(), json!({"timeSpentSeconds": 3600}));
    assert_eq!(r[1].query_param("expand").as_deref(), Some("properties"));
    assert_eq!(r[2].query_param("increaseBy").as_deref(), Some("1h"));
}

#[test]
fn worklog_list_pages_by_offset() {
    let srv = MockServer::with_responses([ok(json!({
        "startAt": 2, "maxResults": 2, "total": 5, "worklogs": [{"id": "3"}, {"id": "4"}]
    }))]);
    let (items, next) = worklogs::list(
        &srv.client(),
        "PROJ-1",
        Some(1000),
        Some(2000),
        2,
        Some("o:2"),
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(next.as_deref(), Some("o:4"));
    let r = &srv.requests()[0];
    assert_eq!(r.query_param("startedAfter").as_deref(), Some("1000"));
    assert_eq!(r.query_param("startedBefore").as_deref(), Some("2000"));
}

#[test]
fn worklog_feeds_send_since_in_epoch_millis() {
    let srv = MockServer::with_responses([
        ok(json!({"values": [{"worklogId": 1}], "until": 5, "lastPage": false})),
        ok(json!({"values": [{"worklogId": 2}], "until": 9, "lastPage": true})),
        ok(json!({"values": [{"worklogId": 1}], "until": 5, "lastPage": false})),
    ]);
    let c = srv.client();
    let since = time::epoch_millis("2025-11-13").unwrap();
    // The next cursor carries the page's `until`, so a run resumes there.
    let (items, next) = worklogs::feed(&c, worklogs::Feed::Changed, since, 25, None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(next, None);
    let (_, next) = worklogs::feed(&c, worklogs::Feed::Deleted, since, 1, None).unwrap();
    assert_eq!(next.as_deref(), Some("t:5"));
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/worklog/updated");
    assert_eq!(r[0].query_param("since").as_deref(), Some("1762992000000"));
    assert_eq!(r[1].query_param("since").as_deref(), Some("5"));
    assert_eq!(r[2].path, "/rest/api/3/worklog/deleted");
}

//! Offline tests for issue comments, changelog, screens, links, delete and
//! the type, status and priority lookups.

mod common;

use atlx::cache::Cache;
use atlx::client::{ApiError, ErrorKind};
use atlx::jira::{self, WeblinkRef};
use atlx::markup::text_to_adf;
use common::{MockResponse, MockServer};
use serde_json::{Map, json};

fn ok(v: serde_json::Value) -> MockResponse {
    MockResponse::json(200, v)
}

#[test]
fn comments_page_by_offset_and_carry_their_issue() {
    let srv = MockServer::with_responses([
        ok(
            json!({"startAt": 0, "maxResults": 2, "total": 3, "comments": [{"id": "1"}, {"id": "2"}]}),
        ),
        ok(json!({"startAt": 2, "maxResults": 2, "total": 3, "comments": [{"id": "3"}]})),
    ]);
    let c = srv.client();
    let (items, next) = jira::comments(&c, "PROJ-1", true, 2, None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["issue"], "PROJ-1");
    assert_eq!(next.as_deref(), Some("o:2"));
    let (rest, next) = jira::comments(&c, "PROJ-1", true, 25, Some("o:2")).unwrap();
    assert_eq!(rest[0]["id"], "3");
    assert_eq!(next, None);
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issue/PROJ-1/comment");
    assert_eq!(r[0].query_param("orderBy").as_deref(), Some("-created"));
    assert_eq!(r[1].query_param("startAt").as_deref(), Some("2"));
    let bad = jira::comments(&c, "PROJ-1", false, 2, Some("p:2")).unwrap_err();
    assert_eq!(ApiError::find(&bad).unwrap().kind, ErrorKind::Usage);
}

#[test]
fn changelog_passes_rest_entries_through() {
    let srv = MockServer::with_responses([ok(json!({
        "values": [{"id": "1", "items": [{"field": "status", "fromString": "Open", "toString": "Done"}]}],
        "isLast": true
    }))]);
    let (items, next) = jira::changelog(&srv.client(), "PROJ-1", 25, None).unwrap();
    assert_eq!(items[0]["items"][0]["toString"], "Done");
    assert_eq!(next, None);
    assert_eq!(srv.requests()[0].path, "/rest/api/3/issue/PROJ-1/changelog");
}

#[test]
fn comment_update_delete_and_reply() {
    let srv = MockServer::with_responses([
        ok(json!({"id": "10001"})),
        MockResponse::empty(204),
        MockResponse::json(201, json!({"id": "10009"})),
    ]);
    let c = srv.client();
    let up = jira::comment_update(&c, "PROJ-1", "10001", text_to_adf("Edited")).unwrap();
    assert_eq!(up["issue"], "PROJ-1");
    let del = jira::comment_delete(&c, "PROJ-1", "10001").unwrap();
    assert_eq!(
        del,
        json!({"id": "10001", "issue": "PROJ-1", "deleted": true})
    );
    let reply = jira::comment(
        &c,
        "PROJ-1",
        text_to_adf("Thanks"),
        None,
        false,
        Some("10001"),
    )
    .unwrap();
    assert_eq!(reply["id"], "10009");
    let reqs = srv.requests();
    assert_eq!(reqs[0].method, "PUT");
    assert_eq!(reqs[0].json_body()["body"]["type"], "doc");
    assert_eq!(reqs[1].method, "DELETE");
    assert_eq!(reqs[2].json_body()["parentId"], 10001);
}

#[test]
fn delete_sends_delete_subtasks() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    let out = jira::delete(&c, "PROJ-1", false).unwrap();
    assert_eq!(out, json!({"id": "PROJ-1", "deleted": true}));
    jira::delete(&c, "PROJ-2", true).unwrap();
    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/rest/api/3/issue/PROJ-1");
    assert_eq!(
        reqs[0].query_param("deleteSubtasks").as_deref(),
        Some("false")
    );
    assert_eq!(
        reqs[1].query_param("deleteSubtasks").as_deref(),
        Some("true")
    );
}

#[test]
fn screens_list_fields_with_inputs_and_examples() {
    let srv = MockServer::with_responses([
        ok(json!({"fields": {
            "summary": {"name": "Summary", "required": true, "schema": {"type": "string"}, "operations": ["set"]},
            "customfield_1": {"name": "Flagged", "schema": {"type": "array", "items": "option"},
                "allowedValues": [{"value": "Impediment"}], "operations": ["add", "set"]},
            "created": {"name": "Created", "operations": []}
        }})),
        ok(json!({"issueTypes": [{"id": "1", "name": "Bug"}], "total": 1})),
        ok(
            json!({"fields": [{"fieldId": "customfield_2", "name": "Start", "required": false,
            "schema": {"type": "date"}}], "total": 1}),
        ),
    ]);
    let c = srv.client();
    let edit = jira::edit_fields(&c, "PROJ-1").unwrap();
    assert_eq!(edit.len(), 2, "fields without operations are not editable");
    assert_eq!(edit[1]["acceptedInput"], "multi-option");
    assert_eq!(edit[1]["example"], "--field 'Flagged=Impediment'");
    let create = jira::create_fields(&c, "PROJ", "bug").unwrap();
    assert_eq!(create[0]["fieldId"], "customfield_2");
    assert_eq!(create[0]["acceptedInput"], "date");
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issue/PROJ-1/editmeta");
    assert_eq!(r[2].path, "/rest/api/3/issue/createmeta/PROJ/issuetypes/1");
}

#[test]
fn field_names_resolve_to_ids_through_the_cache() {
    let dir = std::env::temp_dir().join(format!("atlx-field-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cache = Cache::at(&dir, std::time::Duration::from_secs(60));
    let srv = MockServer::with_responses([
        ok(
            json!([{"id": "customfield_10016", "name": "Story Points"}, {"id": "labels", "name": "Labels"}]),
        ),
        ok(json!([{"id": "customfield_10016", "name": "Story Points"}])),
    ]);
    let c = srv.client();
    let mut m = Map::new();
    m.insert("story points".into(), json!(3));
    m.insert("customfield_1".into(), json!("x"));
    let out = jira::resolve_field_names(&c, &cache, m.clone()).unwrap();
    assert_eq!(out["customfield_10016"], 3);
    assert_eq!(out["customfield_1"], "x");
    // The second lookup is served from the cache: no request.
    jira::resolve_field_names(&c, &cache, m).unwrap();
    assert_eq!(srv.requests().len(), 1);
    // An unknown name refetches once, then fails as a usage error.
    let mut bad = Map::new();
    bad.insert("Nope".into(), json!(1));
    let e = jira::resolve_field_names(&c, &cache, bad).unwrap_err();
    assert_eq!(ApiError::find(&e).unwrap().kind, ErrorKind::Usage);
    assert_eq!(srv.requests().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn links_are_seen_from_the_issue() {
    let srv = MockServer::with_responses([ok(json!({"fields": {"issuelinks": [
        {"id": "1", "type": {"name": "Blocks", "inward": "is blocked by", "outward": "blocks"},
         "inwardIssue": {"key": "PROJ-2", "fields": {"summary": "S", "status": {"name": "Open"}}}}
    ]}}))]);
    let links = jira::links(&srv.client(), "PROJ-1").unwrap();
    assert_eq!(
        links[0],
        json!({"id": "1", "type": "Blocks", "direction": "inward", "relation": "is blocked by",
               "issue": "PROJ-2", "title": "S", "status": "Open"})
    );
}

#[test]
fn link_delete_and_weblink_delete_by_each_selector() {
    let srv = MockServer::with_responses([
        MockResponse::empty(204),
        MockResponse::empty(204),
        MockResponse::empty(204),
        ok(json!([{"id": 7, "object": {"url": "https://e.x/"}}])),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    jira::link_delete(&c, "10500").unwrap();
    jira::weblink_delete(&c, "PROJ-1", WeblinkRef::Id("3")).unwrap();
    jira::weblink_delete(&c, "PROJ-1", WeblinkRef::GlobalId("g")).unwrap();
    let out = jira::weblink_delete(&c, "PROJ-1", WeblinkRef::Url("https://e.x")).unwrap();
    assert_eq!(out["id"], "7");
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issueLink/10500");
    assert_eq!(r[1].path, "/rest/api/3/issue/PROJ-1/remotelink/3");
    assert_eq!(r[2].query_param("globalId").as_deref(), Some("g"));
    assert_eq!(r[4].path, "/rest/api/3/issue/PROJ-1/remotelink/7");
}

#[test]
fn issue_types_statuses_and_priorities() {
    let srv = MockServer::with_responses([
        ok(json!([{"id": "1"}])),
        ok(json!({"id": "10000", "key": "PROJ"})),
        ok(json!([{"id": "2", "hierarchyLevel": -1}])),
        ok(json!({"id": "3"})),
        ok(json!([{"id": "s"}])),
        ok(json!({"values": [{"id": "p"}], "isLast": true})),
    ]);
    let c = srv.client();
    assert_eq!(jira::issue_types(&c, None, None).unwrap()[0]["id"], "1");
    assert_eq!(
        jira::issue_types(&c, Some("PROJ"), Some(-1)).unwrap()[0]["id"],
        "2"
    );
    jira::issue_type(&c, "ari:cloud:jira:x:issue-type/3").unwrap();
    jira::statuses(&c, None).unwrap();
    let (p, _) = jira::priorities(&c, 25, None).unwrap();
    assert_eq!(p[0]["id"], "p");
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issuetype");
    assert_eq!(r[2].query_param("projectId").as_deref(), Some("10000"));
    assert_eq!(r[2].query_param("level").as_deref(), Some("-1"));
    assert_eq!(r[3].path, "/rest/api/3/issuetype/3");
    assert_eq!(r[5].path, "/rest/api/3/priority/search");
}

#[test]
fn deletes_refuse_ids_that_would_address_the_parent() {
    for id in ["..", ".", ""] {
        let srv = MockServer::with_responses([MockResponse::empty(204)]);
        let c = srv.client();
        for err in [
            jira::comment_delete(&c, "PROJ-1", id).unwrap_err(),
            jira::weblink_delete(&c, "PROJ-1", WeblinkRef::Id(id)).unwrap_err(),
            jira::link_delete(&c, id).unwrap_err(),
            jira::delete(&c, id, false).unwrap_err(),
        ] {
            assert_eq!(
                ApiError::find(&err).unwrap().kind,
                ErrorKind::Usage,
                "{id:?}"
            );
        }
        assert!(srv.requests().is_empty(), "{id:?} was sent");
    }
}

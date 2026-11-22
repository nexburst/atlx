//! Offline tests for Jira dashboards.

mod common;

use atlx::jira::dashboards::{self, Edit, Gadget, Search};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

#[test]
fn update_keeps_what_was_not_given() {
    let now = json!({"id": "7", "name": "Team", "description": "Old",
        "sharePermissions": [{"type": "loggedin"}],
        "editPermissions": [{"type": "user", "user": {"accountId": "a"}}]});
    let srv = MockServer::with_responses([ok(now), ok(json!({"id": "7"}))]);
    let e = Edit {
        name: Some("Team 2"),
        ..Edit::default()
    };
    dashboards::update(&srv.client(), "7", &e).unwrap();
    let r = srv.requests();
    assert_eq!(r[1].method, "PUT");
    assert_eq!(
        r[1].json_body(),
        json!({"name": "Team 2", "description": "Old",
               "sharePermissions": [{"type": "authenticated"}],
               "editPermissions": [{"type": "user", "user": {"accountId": "a"}}]})
    );
}

#[test]
fn update_never_clears_what_it_could_not_read() {
    for now in [
        json!({"name": "T", "sharePermissions": []}),
        json!({"name": "T", "editPermissions": [], "sharePermissions": [{"type": "project-unknown"}]}),
    ] {
        let srv = MockServer::with_responses([ok(now)]);
        let e = Edit {
            description: Some("New"),
            ..Edit::default()
        };
        assert!(dashboards::update(&srv.client(), "7", &e).is_err());
        assert_eq!(srv.requests().len(), 1);
    }
}

#[test]
fn create_and_copy_are_private_by_default() {
    let srv = MockServer::with_responses([ok(json!({"id": "1"})), ok(json!({"id": "2"}))]);
    let c = srv.client();
    let e = Edit {
        name: Some("Team"),
        ..Edit::default()
    };
    dashboards::create(&c, &e).unwrap();
    dashboards::copy(&c, "1", &e).unwrap();
    assert!(dashboards::create(&c, &Edit::default()).is_err());
    let r = srv.requests();
    let body = json!({"name": "Team", "sharePermissions": [], "editPermissions": []});
    assert_eq!(
        (r[0].path.as_str(), r[0].json_body()),
        ("/rest/api/3/dashboard", body.clone())
    );
    assert_eq!(
        (r[1].path.as_str(), r[1].json_body()),
        ("/rest/api/3/dashboard/1/copy", body)
    );
}

#[test]
fn query_pages_to_the_limit() {
    let srv = MockServer::with_responses([
        ok(json!({"values": [{"id": "1"}], "isLast": false, "total": 2})),
        ok(json!({"values": [{"id": "2"}], "isLast": true, "total": 2})),
    ]);
    let o = Search {
        name: Some("Team"),
        limit: 10,
        ..Search::default()
    };
    let out = dashboards::query(&srv.client(), &o).unwrap();
    assert_eq!(out["values"].as_array().unwrap().len(), 2);
    assert_eq!(out["isLast"], true);
    assert_eq!(
        srv.requests()[1].query_param("startAt").as_deref(),
        Some("1")
    );
}

#[test]
fn deletes_cannot_reach_the_parent() {
    let srv = MockServer::with_responses([]);
    let c = srv.client();
    for bad in ["", "..", "search", "1/../2"] {
        assert!(dashboards::delete(&c, bad).is_err(), "{bad:?}");
        assert!(dashboards::gadget_delete(&c, "1", bad).is_err(), "{bad:?}");
    }
    for bad in ["", ".."] {
        assert!(dashboards::property_delete(&c, "1", "2", bad).is_err());
        assert!(dashboards::property_delete(&c, "1", bad, "k").is_err());
    }
    assert!(srv.requests().is_empty());
}

#[test]
fn bulk_edit_checks_the_details_for_the_action() {
    let srv = MockServer::with_responses([ok(json!({"action": "changeOwner"}))]);
    let c = srv.client();
    let ids = ["1".to_string()];
    let owner = json!({"newOwner": "acct", "autofixName": true});
    assert!(
        dashboards::bulk_edit(&c, &ids, "addPermission", Some(owner.clone()), None, None).is_err()
    );
    dashboards::bulk_edit(&c, &ids, "changeOwner", Some(owner.clone()), None, None).unwrap();
    let r = srv.requests();
    assert_eq!(r.len(), 1);
    assert_eq!(
        r[0].json_body(),
        json!({"action": "changeOwner", "entityIds": [1], "changeOwnerDetails": owner})
    );
}

#[test]
fn gadgets_and_properties() {
    let srv = MockServer::with_responses([
        ok(json!({"gadgets": []})),
        ok(json!({"gadgets": []})),
        ok(json!({"id": 5})),
        MockResponse::empty(204),
        MockResponse::empty(204),
        ok(json!({"keys": []})),
        ok(json!({"key": "k"})),
        ok(json!({})),
        MockResponse::empty(204),
        MockResponse::empty(204),
    ]);
    let c = srv.client();
    let placed = Gadget {
        row: Some(0),
        column: Some(1),
        ..Gadget::default()
    };
    let titled = Gadget {
        title: Some("Open"),
        ..Gadget::default()
    };
    dashboards::catalog(&c).unwrap();
    dashboards::gadgets(&c, "1", &[], &[], &["5".into()]).unwrap();
    dashboards::gadget_add(&c, "1", &placed, Some("mod:key"), None, None).unwrap();
    dashboards::gadget_update(&c, "1", "5", &titled).unwrap();
    dashboards::gadget_delete(&c, "1", "5").unwrap();
    dashboards::properties(&c, "1", "5").unwrap();
    dashboards::property(&c, "1", "5", "k").unwrap();
    dashboards::property_set(&c, "1", "5", "k", &json!({"a": 1})).unwrap();
    dashboards::property_delete(&c, "1", "5", "k").unwrap();
    dashboards::delete(&c, "1").unwrap();
    let r = srv.requests();
    assert_eq!(
        r[2].json_body(),
        json!({"moduleKey": "mod:key", "position": {"row": 0, "column": 1}})
    );
    assert_eq!(r[3].json_body(), json!({"title": "Open"}));
    assert_eq!(r[8].path, "/rest/api/3/dashboard/1/items/5/properties/k");
    assert_eq!(r[9].path, "/rest/api/3/dashboard/1");
}

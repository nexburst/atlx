//! Output shapes and exit codes seen when atlx and twg 1.4.4 were run side
//! by side on a live site, replayed here with synthetic values.

mod common;

use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::process::{Command, Stdio};

fn run(body: Value, args: &str) -> (Option<i32>, String) {
    let srv = MockServer::with_handler(move |_| MockResponse::json(200, body.clone()));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    let out = cmd
        .args(args.split_whitespace())
        .env(
            "ATLX_CONFIG",
            std::env::temp_dir().join("atlx-no-config.toml"),
        )
        .env("ATLX_SITE", srv.base_url())
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code(), String::from_utf8(out.stdout).unwrap())
}

fn json_run(body: Value, args: &str) -> (Option<i32>, Value) {
    let (code, out) = run(body, &format!("{args} -o json"));
    (code, serde_json::from_str(&out).unwrap())
}

#[test]
fn empty_pages_are_one_jsonl_row_and_empty_lists_none() {
    let page = json!({"startAt": 0, "maxResults": 50, "total": 0, "isLast": true, "values": []});
    let (_, out) = run(page, "jira filter query -o jsonl");
    let row: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(row["values"], json!([]));
    assert_eq!(row["_twg"]["collection"], "root");

    let none = json!({"issues": [], "isLast": true});
    assert_eq!(run(none, "jira workitem query --jql x=1 -o jsonl").1, "");
}

#[test]
fn searches_print_the_page() {
    let page = json!({"startAt": 0, "maxResults": 50, "total": 1, "isLast": true, "values": [{"id": "7"}]});
    let (_, v) = json_run(page.clone(), "jira filter query");
    assert_eq!(v["data"]["values"][0]["id"], "7");
    assert_eq!(v["meta"]["resourceType"], "jira.filter-search-result");
    let (_, v) = json_run(page, "jira space versions query --key P");
    assert_eq!(v["data"]["versions"][0]["id"], "7");
    assert_eq!(v["request"]["projectIdOrKey"], "P");
}

#[test]
fn space_query_defaults_to_fifty() {
    let (_, v) = json_run(
        json!({"values": [], "total": 0, "isLast": true}),
        "jira space query",
    );
    assert_eq!(v["request"]["maxResults"], 50);
}

#[test]
fn space_statuses_are_listed_once() {
    let s = json!({"id": "1", "name": "To Do", "statusCategory": {"key": "new", "name": "To Do"}});
    let d = json!({"id": "2", "name": "Done", "statusCategory": {"key": "done", "name": "Done"}});
    let kinds = json!([{"name": "Task", "statuses": [d, s]}, {"name": "Bug", "statuses": [s]}]);
    let (_, v) = json_run(kinds, "jira space status query --id-or-key P");
    assert_eq!(v["command"], "jira.space.statuses");
    assert_eq!(v["data"], json!({"statuses": [s, d]}));
}

#[test]
fn repo_url_names_the_workspace_and_repo() {
    let (_, v) = json_run(json!({}), "bitbucket repo url --workspace team --repo api");
    assert_eq!(
        (&v["workspace"], &v["repo"]),
        (&json!("team"), &json!("api"))
    );
}

#[test]
fn board_get_is_a_list() {
    let (_, v) = json_run(
        json!({"id": 3, "name": "B"}),
        "jira board get --id 3 --id 3",
    );
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["request"]["ids"], json!(["3", "3"]));
}

#[test]
fn property_and_worklog_feeds_are_bare_lists() {
    let (_, v) = json_run(
        json!({"keys": []}),
        "jira workitem property query --issue-id P-1",
    );
    assert_eq!(v["data"], json!([]));
    let page = json!({"values": [], "since": 1, "until": 2, "lastPage": true});
    let (_, v) = json_run(page, "jira workitem worklog changed --since 1");
    assert_eq!(v["data"], json!([]));
    assert_eq!(v["meta"]["sync"]["until"], 2);
}

#[test]
fn bulk_get_is_a_batch_and_partial_exits_3() {
    let got = json!({"issues": [{"id": "1", "key": "P-1", "fields": {"summary": "S"}}]});
    let (code, v) = json_run(got.clone(), "jira workitem bulk-get P-1 P-2");
    assert_eq!(code, Some(3));
    assert_eq!(v["data"]["items"][0]["data"]["key"], "P-1");
    assert_eq!(v["data"]["items"][1]["ok"], false);
    assert_eq!(v["data"]["summary"]["failed"], 1);
    assert_eq!(v["meta"]["partial"], true);

    let (code, v) = json_run(got, "jira workitem bulk-get OLD-1");
    assert_eq!(code, Some(0));
    assert_eq!(v["data"]["items"][0]["input"], "OLD-1");
    assert_eq!(v["data"]["items"][0]["data"]["key"], "P-1");
}

#[test]
fn user_lookup_prints_the_batch_whatever_the_exit_code() {
    let found = json!({"values": [{"accountId": "a", "displayName": "Ann", "active": true}]});
    for (args, want) in [
        ("--account-id a", 0),
        ("--account-id a --account-id b", 0),
        ("--account-id a --account-id b --strict", 1),
        ("--account-id b", 1),
    ] {
        let (code, v) = json_run(found.clone(), &format!("user bulk-lookup {args}"));
        assert_eq!(code, Some(want), "{args}");
        assert!(v["data"]["summary"]["requested"].is_u64(), "{args}");
    }
}

//! Jira: API functions against the mock HTTP server, and the command-line
//! contract of `atlx jira` / `atlx jsm` (grammar, `--yes`, `--dry-run`,
//! record sizes, Markdown bodies) run through the binary.

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::jira::{self, ListEdit, NewIssue, Update};
use atlx::markup::{BodyFormat, text_to_adf};
use common::{MockResponse, MockServer, RecordedRequest};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn ok(v: Value) -> MockResponse {
    MockResponse::json(200, v)
}

// --- API functions -------------------------------------------------------------

#[test]
fn issue_get_asks_for_the_record_fields_and_names() {
    let srv = MockServer::with_responses([ok(json!({"key": "PROJ-1", "fields": {}}))]);
    jira::issue(&srv.client(), "PROJ-1").unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/rest/api/3/issue/PROJ-1");
    assert_eq!(r.query_param("expand").as_deref(), Some("names"));
    let fields = r.query_param("fields").unwrap();
    assert!(
        fields.contains("description") && fields.contains("comment"),
        "{fields}"
    );
}

#[test]
fn issue_list_follows_next_page_token_to_the_limit() {
    let srv = MockServer::with_responses([
        ok(json!({"issues": [{"key": "P-1"}, {"key": "P-2"}], "nextPageToken": "tok2"})),
        ok(json!({"issues": [{"key": "P-3"}, {"key": "P-4"}], "nextPageToken": "tok3"})),
    ]);
    let c = srv.client();
    let (items, next) = jira::issue_list(&c, "project = P", 3, None).unwrap();
    assert_eq!(items.len(), 3);
    // The limit fell inside the second page: the cursor resumes inside it.
    assert_eq!(next.as_deref(), Some("t+1:tok2"));
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/search/jql");
    assert_eq!(r[0].query_param("maxResults").as_deref(), Some("3"));
    assert_eq!(r[1].query_param("nextPageToken").as_deref(), Some("tok2"));
    assert!(r[0].query_param("fields").unwrap().contains("summary"));
}

#[test]
fn search_builds_text_jql() {
    assert_eq!(
        jira::search_jql("login \"fails\"", Some("P")),
        "project = \"P\" AND text ~ \"login \\\"fails\\\"\" ORDER BY updated DESC"
    );
}

#[test]
fn create_and_update_resolve_accounts_and_fields() {
    let srv = MockServer::with_responses([
        ok(json!({"accountId": "acc-me"})),
        MockResponse::json(201, json!({"id": "10042", "key": "PROJ-42"})),
        ok(json!([
            {"accountType": "app", "accountId": "bot-1", "displayName": "Alice Bot"},
            {"accountType": "atlassian", "accountId": "acc-alice", "displayName": "Alice"}
        ])),
        MockResponse::empty(204),
        ok(json!([
            {"accountType": "atlassian", "accountId": "a1", "displayName": "Alex A"},
            {"accountType": "atlassian", "accountId": "a2", "displayName": "Alex B"}
        ])),
    ]);
    let c = srv.client();
    let fields = jira::new_issue_fields(
        &c,
        NewIssue {
            project: "PROJ",
            issue_type: "Bug",
            summary: "New bug",
            description: Some(jira::adf_body("Hello **world**", BodyFormat::Markdown).unwrap()),
            assignee: Some("me"),
            reporter: Some("ari:cloud:identity::user/557058:rep-1"),
            priority: Some("High"),
            labels: vec!["triage".into()],
            parent: Some("https://x.atlassian.net/browse/PROJ-1"),
            extra: Map::new(),
        },
    )
    .unwrap();
    let created = jira::create(&c, fields).unwrap();
    assert_eq!(created["key"], "PROJ-42");
    assert!(
        created["url"]
            .as_str()
            .unwrap()
            .ends_with("/browse/PROJ-42")
    );

    let updated = jira::update(
        &c,
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
    assert_eq!(updated["updated"], json!(["summary", "assignee", "labels"]));
    let err = jira::update(
        &c,
        "PROJ-42",
        Update {
            assignee: Some("alex"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
    assert!(err.to_string().contains("Alex A"), "{err}");

    let r = srv.requests();
    assert_eq!(r.len(), 5);
    let body = r[1].json_body();
    assert_eq!(body["fields"]["project"], json!({"key": "PROJ"}));
    assert_eq!(body["fields"]["assignee"], json!({"accountId": "acc-me"}));
    assert_eq!(
        body["fields"]["reporter"],
        json!({"accountId": "557058:rep-1"})
    );
    assert_eq!(body["fields"]["parent"], json!({"key": "PROJ-1"}));
    assert_eq!(
        body["fields"]["description"]["content"][0]["content"][1]["marks"][0]["type"],
        "strong"
    );
    let update = r[3].json_body();
    assert_eq!(
        update["fields"]["assignee"],
        json!({"accountId": "acc-alice"})
    );
    assert_eq!(
        update["update"]["labels"],
        json!([{"add": "urgent"}, {"remove": "triage"}])
    );
}

#[test]
fn transition_sends_a_comment_with_a_screen_and_after_without() {
    let transitions = json!({"transitions": [
        {"id": "21", "name": "Resolve", "to": {"name": "Resolved"}, "hasScreen": true},
        {"id": "31", "name": "Close", "to": {"name": "Closed"}, "hasScreen": false}
    ]});
    let srv = MockServer::with_responses([
        ok(transitions.clone()),
        MockResponse::empty(204),
        ok(transitions),
        MockResponse::empty(204),
        MockResponse::json(201, json!({"id": "c-77"})),
    ]);
    let c = srv.client();
    let mut fields = Map::new();
    fields.insert("resolution".into(), json!({"name": "Done"}));
    let a = jira::transition(&c, "PROJ-1", "resolve", fields, Some(text_to_adf("Fixed"))).unwrap();
    assert_eq!(
        a,
        json!({"key": "PROJ-1", "transition": "Resolve", "status": "Resolved"})
    );
    let b = jira::transition(
        &c,
        "PROJ-1",
        "Closed",
        Map::new(),
        Some(text_to_adf("Note")),
    )
    .unwrap();
    assert_eq!(b["commentId"], "c-77");
    let r = srv.requests();
    assert_eq!(
        r[0].query_param("expand").as_deref(),
        Some("transitions.fields")
    );
    assert!(r[1].json_body()["update"]["comment"][0]["add"]["body"].is_object());
    assert!(r[3].json_body().get("update").is_none());
    assert_eq!(r[4].path, "/rest/api/3/issue/PROJ-1/comment");

    let srv = MockServer::with_responses([ok(
        json!({"transitions": [{"id": "1", "name": "Go", "to": {"name": "Done"}}]}),
    )]);
    let e = jira::transition(&srv.client(), "PROJ-1", "Nope", Map::new(), None).unwrap_err();
    let api = ApiError::find(&e).unwrap();
    assert_eq!(api.kind, ErrorKind::Usage);
    assert!(api.message.contains("Done"), "{}", api.message);
}

#[test]
fn assign_puts_the_account_or_null() {
    let srv = MockServer::with_responses([MockResponse::empty(204), MockResponse::empty(204)]);
    let c = srv.client();
    jira::assign(&c, "PROJ-1", Some("acc-1:x")).unwrap();
    jira::assign(&c, "PROJ-1", None).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/rest/api/3/issue/PROJ-1/assignee");
    assert_eq!(r[0].json_body(), json!({"accountId": "acc-1:x"}));
    assert_eq!(r[1].json_body(), json!({"accountId": null}));
}

#[test]
fn projects_and_boards_page_with_filters() {
    let srv = MockServer::with_responses([
        ok(json!({"isLast": false, "values": [{"key": "A"}, {"key": "B"}]})),
        ok(json!({"isLast": true, "values": [{"id": 1}]})),
    ]);
    let c = srv.client();
    let q = jira::ProjectQuery {
        query: Some("web"),
        type_key: Some("software"),
        ..Default::default()
    };
    let (items, next) = jira::projects(&c, &q, 2, Some("o:5")).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(next.as_deref(), Some("o:7"));
    let b = jira::BoardQuery {
        project: Some("A"),
        board_type: Some(jira::BoardType::Kanban),
        ..Default::default()
    };
    jira::boards(&c, &b, 25, None).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].query_param("startAt").as_deref(), Some("5"));
    assert_eq!(r[0].query_param("typeKey").as_deref(), Some("software"));
    assert_eq!(r[0].query_param("orderBy").as_deref(), Some("key"));
    assert_eq!(r[1].path, "/rest/agile/1.0/board");
    assert_eq!(r[1].query_param("type").as_deref(), Some("kanban"));
}

#[test]
fn resolve_board_by_id_url_and_exact_name() {
    let srv = MockServer::with_responses([ok(json!({"isLast": true, "values": [
        {"id": 7, "name": "Team board"}, {"id": 8, "name": "Team board 2"}
    ]}))]);
    let c = srv.client();
    assert_eq!(jira::resolve_board(&c, "12").unwrap(), 12);
    assert_eq!(
        jira::resolve_board(&c, "https://x/jira/software/projects/P/boards/34").unwrap(),
        34
    );
    assert_eq!(jira::resolve_board(&c, "team board").unwrap(), 7);
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn link_create_sends_the_source_as_inward_issue_per_target() {
    let srv = MockServer::with_responses([MockResponse::empty(201), MockResponse::empty(201)]);
    let targets = vec!["PROJ-2".to_string(), "PROJ-3".to_string()];
    let out = jira::link_create(&srv.client(), "PROJ-1", &targets, "Blocks", None).unwrap();
    assert_eq!(
        out[1],
        json!({"source": "PROJ-1", "target": "PROJ-3", "type": "Blocks"})
    );
    let r = srv.requests();
    assert_eq!(r[0].json_body()["inwardIssue"], json!({"key": "PROJ-1"}));
    assert_eq!(r[1].json_body()["outwardIssue"], json!({"key": "PROJ-3"}));
}

#[test]
fn a_lossy_description_is_not_replaced_from_markdown() {
    let lossy = json!({"type": "doc", "version": 1, "content": [
        {"type": "extension", "attrs": {"extensionKey": "jira", "extensionType": "x"}}
    ]});
    let e =
        jira::guard_replace(&lossy, BodyFormat::Markdown, "the description", false).unwrap_err();
    assert_eq!(ApiError::from_anyhow(&e).kind, ErrorKind::Refused);
    jira::guard_replace(&lossy, BodyFormat::Adf, "the description", false).unwrap();
    jira::guard_replace(&lossy, BodyFormat::Markdown, "the description", true).unwrap();
    jira::guard_replace(&text_to_adf("plain"), BodyFormat::Markdown, "d", false).unwrap();
}

// --- the command line ----------------------------------------------------------

struct Run {
    dir: PathBuf,
}

impl Run {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("atlx-jira-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.json"), "[{\"fields\": {\"summary\": \"s\"}}]").unwrap();
        Self { dir }
    }

    fn run(&self, site: &str, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("ATLX_") {
                cmd.env_remove(&key);
            }
        }
        cmd.args(args)
            .args(["-o", "json"])
            .env("ATLX_CONFIG", self.dir.join("config.toml"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("ATLX_SITE", site)
            .env("ATLX_EMAIL", "me@example.com")
            .env("ATLX_TOKEN", "fake")
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn out_json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

fn describe(product: &str) -> Vec<Value> {
    let r = Run::new(&format!("describe-{product}"));
    let o = r.run("http://127.0.0.1:9", &["describe", product, "--all"]);
    assert!(o.status.success());
    out_json(&o)["data"]["commands"].as_array().unwrap().clone()
}

fn contracts() -> Vec<Value> {
    let mut all = describe("jira");
    all.extend(describe("jsm"));
    all
}

const VERBS: &[&str] = &[
    "list",
    "search",
    "get",
    "create",
    "update",
    "delete",
    "transition",
    "assign",
    "move",
    "copy",
    "archive",
    "restore",
    "merge",
    "decline",
    "approve",
    "unapprove",
    "start",
    "complete",
    "run",
    "download",
    "upload",
    "edit",
];

#[test]
fn every_jira_and_jsm_path_follows_the_grammar() {
    let banned = [
        "--id",
        "--issue-id",
        "--max-results",
        "--start-at",
        "--first",
        "--after",
    ];
    let commands = contracts();
    assert!(commands.len() > 100, "{}", commands.len());
    for c in &commands {
        let path = c["path"].as_str().unwrap();
        let words: Vec<&str> = path.split(' ').collect();
        assert!(words.len() <= 4, "{path}: more than one sub-resource");
        assert!(VERBS.contains(words.last().unwrap()), "{path}: not a verb");
        for f in c["flags"].as_array().unwrap() {
            let name = f["name"].as_str().unwrap();
            assert!(!banned.contains(&name), "{path} has {name}");
        }
        assert!(c.get("registered").is_none(), "{path} has no CommandMeta");
        assert!(!c["record"].is_null(), "{path} has no record");
        assert!(!c["examples"].as_array().unwrap().is_empty(), "{path}");
        let lists = matches!(*words.last().unwrap(), "list" | "search");
        let has = |n: &str| {
            c["flags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == n)
        };
        if lists {
            assert!(has("--limit") && has("--cursor"), "{path}: lists page");
        }
        let destructive = c["effect"] == "destructive";
        assert_eq!(
            has("--yes"),
            destructive || has("--allow-lossy"),
            "{path}: --yes"
        );
    }
}

/// A value clap and the command accept for an argument of `path`.
fn sample(path: &str, arg: &Value) -> String {
    let name = arg["name"].as_str().unwrap();
    let v = match (arg["type"].as_str().unwrap(), name) {
        ("integer" | "count", _) => "1",
        ("enum", _) => arg["values"][0].as_str().unwrap(),
        ("path", _) => "x.json",
        (_, "--issue" | "--request") => "P-1",
        (_, "--project") => "PROJ",
        (_, "--to") if path == "jira issue assign" => "none",
        (_, "--to" | "--board" | "--filter" | "--dashboard" | "--item" | "--desk") => "1",
        (_, "--into") => "11",
        (_, "--value") => "1",
        (_, "--permission-json") => "{\"type\":\"global\"}",
        (_, n) if n.contains("KEY") => "P-1",
        (_, n) if n.contains("USER") => "me",
        (_, n) if !n.starts_with('-') => "10",
        _ => "x",
    };
    v.to_string()
}

/// A command line for a described command: every positional and every
/// required flag filled in.
fn argv(c: &Value) -> Vec<String> {
    let path = c["path"].as_str().unwrap();
    let mut out: Vec<String> = path.split(' ').map(String::from).collect();
    for p in c["positionals"].as_array().unwrap() {
        out.push(sample(path, p));
    }
    for f in c["flags"].as_array().unwrap() {
        if f["required"] == true {
            out.push(f["name"].as_str().unwrap().into());
            out.push(sample(path, f));
        }
    }
    // Flags that are required only in combination, or that the command
    // needs to have something to write.
    let extra: &[&str] = match c["path"].as_str().unwrap() {
        "jira issue create" => &["--summary", "S", "--project", "P"],
        "jira issue update" | "jira sprint update" => &["--summary", "S"],
        "jira issue comment create"
        | "jira issue comment update"
        | "jsm request comment create" => &["--body", "hi"],
        "jira issue worklog create" | "jira issue worklog update" => &["--time-spent", "1h"],
        "jira project version update"
        | "jira project component update"
        | "jira dashboard create"
        | "jira dashboard copy"
        | "jira dashboard update"
        | "jira project update" => &["--name", "N"],
        "jira dashboard gadget create" => &["--module-key", "k"],
        "jira dashboard gadget update" => &["--title", "T"],
        "jira filter update" => &["--jql", "a = 1"],
        "jira project version move" => &["--position", "First"],
        "jira sprint start" => &["--duration", "2w"],
        "jsm request create" => &["--type", "1"],
        "jira issue worklog list" => &["--issue", "P-1"],
        _ => &[],
    };
    // `sprint update` takes --name, not --summary.
    let extra: Vec<&str> = if c["path"] == "jira sprint update" {
        vec!["--name", "N"]
    } else {
        extra.to_vec()
    };
    out.extend(extra.iter().map(|s| s.to_string()));
    out
}

#[test]
fn every_destructive_command_refuses_without_yes_and_sends_nothing() {
    let srv = MockServer::with_handler(|_| ok(json!({})));
    let r = Run::new("yes");
    let mut checked = 0;
    for c in contracts().iter().filter(|c| c["effect"] == "destructive") {
        let args = argv(c);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = r.run(&srv.base_url(), &args);
        let v = out_json(&o);
        assert_eq!(o.status.code(), Some(8), "{args:?}: {v}");
        assert_eq!(v["error"]["code"], "refused", "{args:?}");
        assert!(v["error"]["hint"].as_str().unwrap().contains("--dry-run"));
        checked += 1;
    }
    assert!(checked > 30, "{checked}");
    assert!(srv.requests().is_empty(), "{:?}", srv.requests());
}

/// Mutating requests the server saw (bulkfetch is a read sent as POST).
fn writes(reqs: &[RecordedRequest]) -> Vec<String> {
    reqs.iter()
        .filter(|r| r.method != "GET" && !r.path.ends_with("/issue/bulkfetch"))
        .map(|r| format!("{} {}", r.method, r.path))
        .collect()
}

#[test]
fn every_write_honours_dry_run() {
    let srv = MockServer::with_handler(|req| {
        if req.method != "GET" {
            return MockResponse::json(201, json!({"id": "1"}));
        }
        ok(json!({
            "id": "1", "key": "P-1", "name": "P-1", "accountId": "acc", "isLast": true,
            "values": [{"id": "1", "name": "P-1", "projectKey": "P-1"}],
            "issues": [], "fields": {"summary": "s"},
            "transitions": [{"id": "1", "name": "P-1", "to": {"name": "P-1"}}],
            "requestTypeFields": [], "issueTypes": [{"id": "1", "name": "Task"}],
        }))
    });
    let r = Run::new("dry");
    let mut dry = 0;
    let mut stopped_early = Vec::new();
    for c in contracts().iter().filter(|c| c["effect"] != "read") {
        let mut args = argv(c);
        args.push("--dry-run".into());
        if c["effect"] == "destructive" {
            args.push("--yes".into());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = r.run(&srv.base_url(), &args);
        let v = out_json(&o);
        assert_ne!(o.status.code(), Some(8), "{args:?}: {v}");
        let msg = v["error"]["message"].as_str().unwrap_or_default();
        let parse = [
            "unexpected argument",
            "required arguments",
            "invalid value",
            "unrecognized",
        ];
        assert!(
            !parse.iter().any(|p| msg.contains(p)),
            "{args:?} does not parse: {v}"
        );
        assert!(
            !msg.contains("dry run"),
            "{args:?}: the dry-run stop became an error: {v}"
        );
        if v["dryRun"] == true {
            dry += 1;
            assert_ne!(v["data"]["request"]["method"], "GET", "{args:?}");
        } else {
            stopped_early.push(c["path"].as_str().unwrap().to_string());
        }
        assert!(writes(&srv.requests()).is_empty(), "{args:?} wrote");
    }
    assert!(
        dry > 50,
        "only {dry} writes reached the dry-run stop; not: {stopped_early:?}"
    );
}

fn issue_row(i: usize) -> Value {
    json!({
        "id": format!("{}", 10000 + i),
        "key": format!("PROJ-{}", 100 + i),
        "self": format!("https://acme.atlassian.net/rest/api/3/issue/{}", 10000 + i),
        "fields": {
            "summary": format!("Login fails on Safari after session expiry #{i}"),
            "issuetype": {"id": "1", "name": "Bug", "iconUrl": "https://acme.atlassian.net/i.png", "subtask": false},
            "status": {"id": "3", "name": "In Progress", "statusCategory": {"key": "indeterminate", "name": "In Progress", "colorName": "yellow"}},
            "priority": {"id": "2", "name": "High", "iconUrl": "https://acme.atlassian.net/p.png"},
            "assignee": {"accountId": "712020:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "displayName": "Alice Example",
                         "avatarUrls": {"48x48": "https://avatar/48"}, "emailAddress": "alice@example.com", "active": true},
            "reporter": {"accountId": "712020:ffffffff-bbbb-cccc-dddd-eeeeeeeeeeee", "displayName": "Bob Example"},
            "project": {"id": "10000", "key": "PROJ", "name": "Project"},
            "labels": ["frontend", "safari"],
            "created": "2026-01-01T09:00:00.000+0000",
            "updated": "2026-01-02T03:04:05.000+0000",
        }
    })
}

#[test]
fn issue_list_default_view_of_25_issues_stays_under_6_kb() {
    let issues: Vec<Value> = (0..25).map(issue_row).collect();
    let srv = MockServer::with_responses([ok(json!({"issues": issues, "nextPageToken": "n"}))]);
    let r = Run::new("budget");
    let o = r.run(
        &srv.base_url(),
        &["jira", "issue", "list", "--project", "PROJ"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let v = out_json(&o);
    assert_eq!(v["data"].as_array().unwrap().len(), 25);
    assert_eq!(v["next"], "t:n");
    assert_eq!(
        v["data"][0],
        json!({"key": "PROJ-100", "title": "Login fails on Safari after session expiry #0",
               "type": "Bug", "status": "In Progress",
               "assignee": {"id": "712020:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "name": "Alice Example"},
               "updated": "2026-01-02T03:04:05.000+0000"})
    );
    assert!(o.stdout.len() < 6 * 1024, "{} bytes", o.stdout.len());
    let jql = srv.requests()[0].query_param("jql").unwrap();
    assert_eq!(jql, "project = \"PROJ\" ORDER BY updated DESC");
}

fn lossy_issue() -> Value {
    json!({"key": "PROJ-1", "id": "10001", "self": "https://acme.atlassian.net/rest/api/3/issue/10001",
    "fields": {"summary": "S", "description": {"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": "Steps:"}]},
        {"type": "extension", "attrs": {"extensionType": "com.atlassian.jira", "extensionKey": "chart"}}
    ]}}})
}

#[test]
fn issue_get_prints_markdown_and_warns_about_what_it_cannot_show() {
    let srv = MockServer::with_handler(|_| ok(lossy_issue()));
    let r = Run::new("lossy");
    let o = r.run(&srv.base_url(), &["jira", "issue", "get", "PROJ-1"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let v = out_json(&o);
    let d = v["data"]["description"].as_str().unwrap();
    assert!(d.starts_with("Steps:"), "{d}");
    assert!(d.contains("<!-- atlx: extension"), "{d}");
    assert_eq!(v["warnings"][0]["code"], "lossy_body");
    assert_eq!(v["warnings"][0]["at"], "PROJ-1");
    assert_eq!(v["data"]["url"], "https://acme.atlassian.net/browse/PROJ-1");

    let o = r.run(
        &srv.base_url(),
        &["jira", "issue", "get", "PROJ-1", "--body-format", "adf"],
    );
    let v = out_json(&o);
    assert_eq!(v["data"]["description"]["type"], "doc");
    assert!(v.get("warnings").is_none());
}

#[test]
fn replacing_a_lossy_description_with_markdown_is_refused_before_any_write() {
    let srv = MockServer::with_handler(|_| ok(lossy_issue()));
    let r = Run::new("replace");
    let o = r.run(
        &srv.base_url(),
        &[
            "jira",
            "issue",
            "update",
            "PROJ-1",
            "--description",
            "New text",
        ],
    );
    let v = out_json(&o);
    assert_eq!(o.status.code(), Some(8), "{v}");
    assert_eq!(v["error"]["code"], "refused");
    assert!(
        v["error"]["losses"][0]["node"]
            .as_str()
            .unwrap()
            .starts_with("extension")
    );
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
    // --allow-lossy needs --yes, and is refused without it.
    let o = r.run(
        &srv.base_url(),
        &[
            "jira",
            "issue",
            "update",
            "PROJ-1",
            "--description",
            "x",
            "--allow-lossy",
        ],
    );
    assert_eq!(o.status.code(), Some(8));
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn multi_issue_get_reports_missing_inputs_as_partial() {
    let srv = MockServer::with_handler(|req| {
        if req.path.ends_with("/PROJ-1") {
            ok(json!({"key": "PROJ-1", "id": "1", "fields": {"summary": "S"}}))
        } else {
            MockResponse::json(404, json!({"errorMessages": ["Issue does not exist"]}))
        }
    });
    let r = Run::new("partial");
    let o = r.run(
        &srv.base_url(),
        &[
            "jira",
            "issue",
            "get",
            "PROJ-1",
            "PROJ-2",
            "--fields",
            "key,title",
        ],
    );
    let v = out_json(&o);
    assert_eq!(o.status.code(), Some(3), "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["partial"], true);
    assert_eq!(v["data"], json!([{"key": "PROJ-1", "title": "S"}]));
    assert_eq!(v["errors"][0]["input"], "PROJ-2");
    assert_eq!(v["errors"][0]["error"]["code"], "not_found");
    let o = r.run(
        &srv.base_url(),
        &["jira", "issue", "get", "PROJ-1", "PROJ-2", "--strict"],
    );
    assert_eq!(o.status.code(), Some(5));
    let o = r.run(
        &srv.base_url(),
        &["jira", "issue", "get", "PROJ-1", "--fields", "nope"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(
        out_json(&o)["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("title")
    );
}

#[test]
fn many_issues_are_read_with_bulkfetch_in_input_order() {
    let srv = MockServer::with_handler(|req| {
        assert_eq!(req.path, "/rest/api/3/issue/bulkfetch");
        let keys = req.json_body()["issueIdsOrKeys"].clone();
        let issues: Vec<Value> = keys
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .filter(|k| *k != "P-7")
            .map(|k| json!({"key": k, "fields": {"summary": "S"}}))
            .collect();
        ok(json!({"issues": issues}))
    });
    let r = Run::new("bulkget");
    let keys: Vec<String> = (1..=30).map(|i| format!("P-{i}")).collect();
    let mut args = vec!["jira", "issue", "get", "--fields", "key"];
    args.extend(keys.iter().map(String::as_str));
    let o = r.run(&srv.base_url(), &args);
    let v = out_json(&o);
    assert_eq!(o.status.code(), Some(3), "{v}");
    assert_eq!(v["data"][0]["key"], "P-1");
    assert_eq!(v["data"][28]["key"], "P-30");
    assert_eq!(v["errors"][0]["input"], "P-7");
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn list_without_a_filter_is_a_usage_error_with_suggestions() {
    let r = Run::new("nofilter");
    let o = r.run("http://127.0.0.1:9", &["jira", "issue", "list"]);
    assert_eq!(o.status.code(), Some(2));
    let v = out_json(&o);
    assert!(!v["error"]["suggestions"].as_array().unwrap().is_empty());
}

#[test]
fn ids_output_pipes_keys() {
    let issues: Vec<Value> = (0..3).map(issue_row).collect();
    let srv = MockServer::with_responses([ok(json!({"issues": issues}))]);
    let r = Run::new("ids");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    cmd.args(["jira", "issue", "list", "--jql", "x", "-o", "ids"])
        .env("ATLX_CONFIG", r.dir.join("config.toml"))
        .env("ATLX_SITE", srv.base_url())
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .env_remove("ATLX_OUTPUT");
    let o = cmd.output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "PROJ-100\nPROJ-101\nPROJ-102\n"
    );
}

//! The Confluence command line end to end against a mock site: what writes
//! refuse before sending anything, the version check, the lossy-body
//! guards, dry runs, and the shape of reads and lists.

mod common;

use common::{MockResponse, MockServer, RecordedRequest};
use serde_json::{Value, json};
use std::process::{Command, Output, Stdio};

fn doc(content: Value) -> String {
    json!({"type": "doc", "version": 1, "content": content}).to_string()
}

fn para(text: &str) -> Value {
    json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
}

/// The Jira macro on the rich page, exactly as the server sends it.
fn jira_macro() -> Value {
    json!({"type": "extension", "attrs": {
        "extensionType": "com.atlassian.confluence.macro.core", "extensionKey": "jira",
        "localId": "jira-1", "parameters": {"macroParams": {"key": {"value": "OPS-12"}}}}})
}

fn rich_doc() -> Value {
    json!({"type": "doc", "version": 1, "content": [
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
        jira_macro(),
        {"type": "paragraph", "attrs": {"localId": "p-1"}, "content": [{"type": "text", "text": "Old text"}]}
    ]})
}

/// The storage body the mock serves for page 123.
const STORED: &str = "<p>old</p>";

/// Page 123 is plain (version 1); page 500 has a panel, a table and a Jira
/// macro (version 4). Both answer in the body format asked for.
fn page(id: &str, req: &RecordedRequest) -> Value {
    let (version, adf) = match id {
        "500" => (4, rich_doc().to_string()),
        _ => (1, doc(json!([para("old")]))),
    };
    let body = match req.query_param("body-format").as_deref() {
        Some("storage") => json!({"storage": {"value": STORED, "representation": "storage"}}),
        Some("atlas_doc_format") => {
            json!({"atlas_doc_format": {"value": adf, "representation": "atlas_doc_format"}})
        }
        _ => json!({}),
    };
    json!({
        "id": id, "title": "Runbook", "status": "current", "spaceId": "9", "parentId": "1",
        "version": {"number": version, "createdAt": "2026-01-10T00:00:00Z", "authorId": "a1"},
        "body": body, "_links": {"webui": format!("/spaces/ENG/pages/{id}")},
    })
}

fn comment(req: &RecordedRequest) -> Value {
    let body = match req.query_param("body-format").as_deref() {
        Some("storage") => json!({"storage": {"value": STORED}}),
        _ => json!({"atlas_doc_format": {"value": doc(json!([para("hi")]))}}),
    };
    json!({"id": "777", "pageId": "123", "status": "current", "version": {"number": 2}, "body": body})
}

/// `cursor` -> (ids, next cursor) for the paged space listing.
fn listing(cursor: Option<&str>) -> Value {
    let (ids, next): (&[&str], Option<&str>) = match cursor {
        None => (&["1", "2"], Some("c2")),
        Some("c2") => (&["3", "4"], Some("c3")),
        _ => (&["5"], None),
    };
    let results: Vec<Value> = ids
        .iter()
        .map(|i| json!({"id": i, "title": format!("P{i}"), "status": "current", "version": {"number": 1}}))
        .collect();
    let mut v = json!({"results": results, "_links": {}});
    if let Some(n) = next {
        v["_links"]["next"] = format!("/wiki/api/v2/spaces/9/pages?limit=2&cursor={n}").into();
    }
    v
}

fn handler(req: &RecordedRequest) -> MockResponse {
    let path = req.path.trim_start_matches("/wiki");
    let ok = |v: Value| MockResponse::json(200, v);
    match (req.method.as_str(), path) {
        ("GET", "/api/v2/pages/123") | ("GET", "/api/v2/pages/500") => {
            ok(page(&path["/api/v2/pages/".len()..], req))
        }
        ("PUT", "/api/v2/pages/123") | ("PUT", "/api/v2/pages/500") => {
            let sent = req.json_body();
            ok(
                json!({"id": sent["id"], "title": sent["title"], "status": "current",
                      "version": sent["version"], "_links": {"webui": "/x"}}),
            )
        }
        ("POST", "/api/v2/pages") => ok(json!({"id": "900", "title": "T", "status": "current",
                                               "version": {"number": 1}})),
        ("GET", "/api/v2/spaces") => ok(json!({"results": [{"id": "9", "key": "ENG"}]})),
        ("GET", "/api/v2/spaces/9/pages") => ok(listing(req.query_param("cursor").as_deref())),
        ("GET", "/api/v2/footer-comments/777") => ok(comment(req)),
        ("PUT", "/api/v2/footer-comments/777") | ("POST", "/api/v2/footer-comments") => {
            ok(json!({"id": "777", "version": {"number": 3}}))
        }
        ("DELETE", _) => MockResponse::empty(204),
        _ => MockResponse::json(404, json!({"message": format!("no {} {path}", req.method)})),
    }
}

fn server() -> MockServer {
    MockServer::with_handler(handler)
}

/// Run `atlx confluence ARGS` without a terminal against `srv`.
fn atlx(srv: &MockServer, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
    // Not env_clear(): without SystemRoot, Windows can't open a socket, and
    // then "nothing was sent" would hold for the wrong reason.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("ATLX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.arg("confluence")
        .args(args)
        .env(
            "ATLX_CONFIG",
            std::env::temp_dir().join("atlx-no-config.toml"),
        )
        .env("ATLX_SITE", srv.base_url())
        .env("ATLX_EMAIL", "me@example.com")
        .env("ATLX_TOKEN", "fake")
        .env("ATLX_NO_CACHE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn words(line: &str) -> Vec<&str> {
    line.split(' ').collect()
}

fn json_out(o: &Output) -> Value {
    let s = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn writes(srv: &MockServer) -> Vec<RecordedRequest> {
    srv.requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect()
}

const EDIT_P1: &str = r#"[{"name":"replaceNode","localId":"p-1","value":"New text"}]"#;

#[test]
fn guarded_writes_refuse_and_send_nothing() {
    for (line, exit, needle) in [
        // Destructive commands need --yes (refused, exit 8).
        ("page delete 123", 8, "--yes"),
        ("page delete 123 --purge", 8, "--yes"),
        ("page delete 123 --draft", 8, "--yes"),
        ("page archive 123", 8, "--yes"),
        ("page version restore 1 --page 123", 8, "--yes"),
        ("page comment delete 777", 8, "--yes"),
        ("page attachment delete att1", 8, "--yes"),
        ("page label delete draft --page 123", 8, "--yes"),
        ("space delete ENG", 8, "--yes"),
        ("space archive ENG", 8, "--yes"),
        (
            "page update 123 --if-version 1 --body x --allow-lossy",
            8,
            "--yes",
        ),
        // --if-version is required (usage, exit 2).
        // clap names the missing flag on its second line, which the usage
        // error drops (a shared-code issue), so only the code is checked.
        ("page update 123 --title T", 2, "usage"),
        ("page edit 123 --edits []", 2, "usage"),
        // HTML is not a body format; old flags are gone.
        (
            "page update 123 --if-version 1 --body x --body-format html",
            2,
            "html",
        ),
        (
            "page create --space ENG --title T --body x --body-format html",
            2,
            "html",
        ),
        (
            "page comment create --page 123 --body x --body-format html",
            2,
            "html",
        ),
        (
            "page update 123 --if-version 1 --title T --snapshot-token 1",
            2,
            "snapshot-token",
        ),
        (
            "page create --space ENG --title T --body x --ack-body-formats",
            2,
            "ack-body-formats",
        ),
        (
            "page create --space ENG --title T",
            2,
            "--body or --body-file",
        ),
    ] {
        let srv = server();
        let out = atlx(&srv, &[&words(line)[..], &["-o", "json"]].concat());
        let v = json_out(&out);
        assert_eq!(out.status.code(), Some(exit), "{line}: {v}");
        assert_eq!(v["ok"], false, "{line}");
        assert!(
            v.to_string().contains(needle),
            "{line}: expected {needle} in {v}"
        );
        assert!(writes(&srv).is_empty(), "{line}: sent {:?}", writes(&srv));
    }
}

#[test]
fn confirmed_writes_are_sent() {
    for (line, method) in [
        ("page delete 123 --yes", "DELETE"),
        ("page update 123 --if-version 1 --title New", "PUT"),
        ("page update 123 --if-version 1 --body new", "PUT"),
        ("page create --space ENG --title T --body x", "POST"),
        ("page comment create --page 123 --body x", "POST"),
    ] {
        let srv = server();
        let out = atlx(&srv, &words(line));
        assert!(
            out.status.success(),
            "{line}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            srv.requests().iter().any(|r| r.method == method),
            "{line}: no {method}"
        );
    }
}

#[test]
fn a_stale_if_version_is_a_conflict_and_nothing_is_written() {
    for args in [
        vec![
            "page",
            "update",
            "123",
            "--if-version",
            "3",
            "--title",
            "New",
        ],
        vec![
            "page",
            "edit",
            "500",
            "--if-version",
            "3",
            "--edits",
            EDIT_P1,
        ],
    ] {
        let srv = server();
        let out = atlx(&srv, &args);
        let v = json_out(&out);
        assert_eq!(out.status.code(), Some(6), "{args:?}: {v}");
        assert_eq!(v["error"]["code"], "conflict");
        assert!(
            v["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("--if-version"),
            "{v}"
        );
        assert!(writes(&srv).is_empty(), "{args:?}");
    }
}

#[test]
fn a_body_that_would_lose_content_is_refused_with_exit_8() {
    let edit_html = r#"[{"name":"appendNodeToEnd","value":"<custom>x</custom>"}]"#;
    for args in [
        // Raw HTML in Markdown has no ADF mapping.
        vec![
            "page",
            "update",
            "123",
            "--if-version",
            "1",
            "--body",
            "<custom>x</custom>",
        ],
        vec![
            "page",
            "create",
            "--space",
            "ENG",
            "--title",
            "T",
            "--body",
            "<custom>x</custom>",
        ],
        vec![
            "page",
            "comment",
            "create",
            "--page",
            "123",
            "--body",
            "<div>x</div>",
        ],
        // Storage must use storage elements only.
        vec![
            "page",
            "create",
            "--space",
            "ENG",
            "--title",
            "T",
            "--body",
            "<custom>x</custom>",
            "--body-format",
            "storage",
        ],
        // The current body has a macro Markdown cannot express.
        vec![
            "page",
            "update",
            "500",
            "--if-version",
            "4",
            "--body",
            "All new",
        ],
        vec![
            "page",
            "update",
            "500",
            "--if-version",
            "4",
            "--body",
            "All new",
            "--body-format",
            "plain",
        ],
        // An edit value that would lose content.
        vec![
            "page",
            "edit",
            "500",
            "--if-version",
            "4",
            "--edits",
            edit_html,
        ],
    ] {
        let srv = server();
        let out = atlx(&srv, &[&args[..], &["-o", "json"]].concat());
        let v = json_out(&out);
        assert_eq!(out.status.code(), Some(8), "{args:?}: {v}");
        assert_eq!(v["ok"], false, "{args:?}");
        assert_eq!(v["error"]["code"], "refused", "{args:?}: {v}");
        assert!(writes(&srv).is_empty(), "{args:?}: sent {:?}", writes(&srv));
    }
}

#[test]
fn a_rich_page_reads_honestly_and_edits_leave_the_macro_untouched() {
    let srv = server();
    let out = atlx(&srv, &["page", "get", "500", "-o", "json"]);
    let v = json_out(&out);
    assert!(out.status.success(), "{v}");
    let body = v["data"]["body"].as_str().unwrap();
    assert!(body.contains("> [!NOTE]\n> Read me first"), "{body}");
    assert!(body.contains("| Step | Owner |"), "{body}");
    assert!(body.contains("<!-- atlx: extension:jira"), "{body}");
    let w = &v["warnings"][0];
    assert_eq!(w["code"], "lossy_body", "{v}");
    assert!(
        w["nodes"]
            .as_array()
            .unwrap()
            .contains(&"extension:jira".into()),
        "{w}"
    );
    assert_eq!(w["input"], "page 500");
    assert_eq!(v["data"]["version"], 4);

    let srv = server();
    let out = atlx(
        &srv,
        &[
            "page",
            "edit",
            "500",
            "--if-version",
            "4",
            "--edits",
            EDIT_P1,
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let put = writes(&srv);
    assert_eq!(put.len(), 1);
    let sent = put[0].json_body();
    assert_eq!(sent["version"]["number"], 5);
    assert_eq!(sent["body"]["representation"], "atlas_doc_format");
    let adf = sent["body"]["value"].as_str().unwrap();
    assert!(adf.contains(&jira_macro().to_string()), "{adf}");
    let sent_doc: Value = serde_json::from_str(adf).unwrap();
    let mut want = rich_doc();
    want["content"][3] = para("New text");
    assert_eq!(sent_doc, want);
}

#[test]
fn storage_is_only_sent_as_validated_input_or_the_stored_body() {
    let srv = server();
    let runs: Vec<Vec<&str>> = vec![
        vec![
            "page",
            "create",
            "--space",
            "ENG",
            "--title",
            "T",
            "--body",
            "# Hi\n\n**x**",
        ],
        vec![
            "page",
            "create",
            "--space",
            "ENG",
            "--title",
            "T",
            "--body",
            "<p>new</p>",
            "--body-format",
            "storage",
        ],
        vec![
            "page",
            "update",
            "123",
            "--if-version",
            "1",
            "--body",
            "new *text*",
        ],
        vec![
            "page",
            "update",
            "123",
            "--if-version",
            "1",
            "--body",
            "plain\ntext",
            "--body-format",
            "plain",
        ],
        vec![
            "page",
            "update",
            "123",
            "--if-version",
            "1",
            "--title",
            "Renamed",
        ],
        vec![
            "page",
            "edit",
            "500",
            "--if-version",
            "4",
            "--edits",
            EDIT_P1,
        ],
        vec![
            "page", "comment", "create", "--page", "123", "--body", "Nice",
        ],
        vec![
            "page",
            "comment",
            "update",
            "777",
            "--body",
            "Nicer",
            "--if-version",
            "2",
        ],
    ];
    for args in &runs {
        let out = atlx(&srv, args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let sent = writes(&srv);
    assert_eq!(sent.len(), runs.len());
    let mut storage = 0;
    for r in &sent {
        let body = r.json_body();
        let b = &body["body"];
        match b["representation"].as_str() {
            Some("storage") => {
                storage += 1;
                let v = b["value"].as_str().unwrap();
                assert!(
                    v == STORED || v == "<p>new</p>",
                    "{} {}: {v}",
                    r.method,
                    r.path
                );
            }
            Some("atlas_doc_format") => {
                let doc: Value = serde_json::from_str(b["value"].as_str().unwrap()).unwrap();
                assert_eq!(doc["type"], "doc");
            }
            other => panic!("{} {}: representation {other:?}", r.method, r.path),
        }
    }
    assert_eq!(storage, 2, "the storage create and the title change");
}

#[test]
fn a_dry_run_shows_the_request_and_a_diff() {
    let srv = server();
    let out = atlx(
        &srv,
        &[
            "page",
            "update",
            "123",
            "--if-version",
            "1",
            "--body",
            "new",
            "--dry-run",
            "-o",
            "json",
        ],
    );
    let v = json_out(&out);
    assert!(out.status.success(), "{v}");
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["data"]["request"]["method"], "PUT");
    let diff = v["data"]["diff"].as_str().unwrap();
    assert!(diff.contains("-old\n+new\n"), "{diff}");
    assert!(writes(&srv).is_empty());

    // Destructive commands need no --yes for a dry run.
    let srv = server();
    let out = atlx(&srv, &["page", "delete", "123", "--dry-run", "-o", "json"]);
    let v = json_out(&out);
    assert!(out.status.success(), "{v}");
    assert_eq!(v["data"]["request"]["method"], "DELETE");
    assert!(writes(&srv).is_empty());
}

#[test]
fn get_returns_the_page_record_and_fields_pick_from_it() {
    let srv = server();
    let v = json_out(&atlx(&srv, &["page", "get", "123", "-o", "json"]));
    let keys: Vec<&String> = v["data"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "id", "type", "title", "status", "spaceId", "parentId", "version", "updated", "url",
            "body", "outline"
        ]
    );
    assert_eq!(v["data"]["body"], "old");
    assert_eq!(v["data"]["version"], 1);

    let v = json_out(&atlx(
        &srv,
        &["page", "get", "123", "--fields", "id,version", "-o", "json"],
    ));
    assert_eq!(v["data"], json!({"id": "123", "version": 1}));
    let out = atlx(
        &srv,
        &["page", "get", "123", "--fields", "nope", "-o", "json"],
    );
    assert_eq!(out.status.code(), Some(2));

    // Several IDs: an array in input order; a failure makes it partial.
    let out = atlx(&srv, &["page", "get", "500", "123", "404404", "-o", "json"]);
    let v = json_out(&out);
    assert_eq!(out.status.code(), Some(3), "{v}");
    assert_eq!(v["data"][0]["id"], "500");
    assert_eq!(v["data"][1]["id"], "123");
    assert_eq!(v["errors"][0]["input"], "404404");
}

#[test]
fn lists_follow_cursors_to_the_limit() {
    let srv = server();
    let v = json_out(&atlx(
        &srv,
        &[
            "page", "list", "--space", "ENG", "--limit", "3", "-o", "json",
        ],
    ));
    let ids: Vec<&str> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["1", "2", "3"]);
    let next = v["next"].as_str().unwrap().to_string();
    assert_eq!(v["data"][0]["type"], "page");
    assert!(v["data"][0].get("body").is_none());

    let out = atlx(
        &srv,
        &[
            "page", "list", "--space", "ENG", "--cursor", &next, "-o", "ids",
        ],
    );
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "4\n5\n");

    let v = json_out(&atlx(
        &srv,
        &["page", "list", "--space", "ENG", "--all", "-o", "json"],
    ));
    assert_eq!(v["data"].as_array().unwrap().len(), 5);
    assert_eq!(v["next"], Value::Null);
    let out = atlx(
        &srv,
        &[
            "page", "list", "--space", "ENG", "--cursor", "o:5", "-o", "json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn body_only_prints_or_writes_just_the_body() {
    let srv = server();
    let out = atlx(&srv, &["page", "get", "123", "--body-only"]);
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "old\n");
    let out = atlx(
        &srv,
        &[
            "page",
            "get",
            "123",
            "--body-only",
            "--body-format",
            "storage",
        ],
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("{STORED}\n"));
    let file = std::env::temp_dir().join(format!("atlx-body-only-{}.md", std::process::id()));
    let f = file.to_str().unwrap();
    let out = atlx(
        &srv,
        &[
            "page",
            "get",
            "123",
            "--body-only",
            "--output-file",
            f,
            "-o",
            "json",
        ],
    );
    let v = json_out(&out);
    assert_eq!(v["data"]["bytes"], 3, "{v}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
    let _ = std::fs::remove_file(&file);
    let out = atlx(
        &srv,
        &["page", "get", "123", "500", "--body-only", "-o", "json"],
    );
    assert_eq!(out.status.code(), Some(2));
}

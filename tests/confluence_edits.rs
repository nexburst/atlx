//! Offline tests for Confluence outline and section reads and for
//! `content update --edits` (#27).

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{self, Detail, Edit, GetOpts, ReadFormat, edits};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn opts<'a>(detail: Detail, format: ReadFormat, sections: &'a [String]) -> GetOpts<'a> {
    GetOpts {
        detail,
        format,
        draft: false,
        reference_space: None,
        sections,
    }
}

fn edit<'a>(ops: &'a [edits::Op], token: Option<&'a str>, dry_run: bool) -> Edit<'a> {
    Edit {
        title: None,
        body: None,
        version_message: None,
        minor_edit: false,
        snapshot_token: token,
        dry_run,
        draft: false,
        edits: Some(ops),
    }
}

fn is_usage(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>()
        .is_some_and(|a| a.kind == ErrorKind::Usage)
}

const STORAGE: &str = "<h1>Runbook</h1><p>Intro text</p>\
    <h2>Deploy &amp; Verify</h2><p>Run the deploy.</p>\
    <h3>Checks</h3><ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[<h2>fake</h2> curl /health]]></ac:plain-text-body></ac:structured-macro>\
    <h2>Rollback plan</h2><p>Revert the <strong>release</strong>.</p>\
    <h2>Rollback</h2><p>Short form.</p>";

fn storage_page() -> Value {
    json!({
        "id": "12345",
        "title": "Runbook",
        "status": "current",
        "spaceId": "10",
        "version": {"number": 7, "createdAt": "2026-01-10T00:00:00Z"},
        "body": {"storage": {"value": STORAGE, "representation": "storage"}},
        "_links": {"webui": "/spaces/ENG/pages/12345"}
    })
}

fn doc() -> Value {
    json!({"type": "doc", "version": 1, "content": [
        {"type": "heading", "attrs": {"level": 1, "localId": "h-top"}, "content": [{"type": "text", "text": "Runbook"}]},
        {"type": "paragraph", "attrs": {"localId": "p-intro"}, "content": [{"type": "text", "text": "Intro text here"}]},
        {"type": "heading", "attrs": {"level": 2, "localId": "h-deploy"}, "content": [{"type": "text", "text": "Deploy"}]},
        {"type": "paragraph", "attrs": {"localId": "p-deploy"}, "content": [
            {"type": "text", "text": "Run the "},
            {"type": "text", "text": "deploy", "marks": [{"type": "strong"}]},
            {"type": "text", "text": " job, then deploy again."}
        ]},
        {"type": "bulletList", "attrs": {"localId": "list"}, "content": [
            {"type": "listItem", "attrs": {"localId": "li-1"}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]},
            {"type": "listItem", "attrs": {"localId": "li-2"}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "two"}]}]}
        ]},
        {"type": "extension", "attrs": {"extensionKey": "toc", "localId": "macro"}},
        {"type": "heading", "attrs": {"level": 2, "localId": "h-rollback"}, "content": [{"type": "text", "text": "Rollback"}]},
        {"type": "paragraph", "attrs": {"localId": "p-rollback"}, "content": [{"type": "text", "text": "Revert it."}]}
    ]})
}

fn adf_page(doc: &Value) -> Value {
    json!({
        "id": "12345",
        "title": "Runbook",
        "status": "current",
        "spaceId": "10",
        "version": {"number": 7, "createdAt": "2026-01-10T00:00:00Z"},
        "body": {"atlas_doc_format": {"value": doc.to_string(), "representation": "atlas_doc_format"}},
        "_links": {"webui": "/spaces/ENG/pages/12345"}
    })
}

fn ids(doc: &Value) -> Vec<String> {
    doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["attrs"]["localId"].as_str().unwrap_or("-").to_string())
        .collect()
}

fn run(json_ops: &str) -> Value {
    edits::apply(&doc(), &edits::parse(json_ops).unwrap()).unwrap()
}

fn run_err(json_ops: &str) -> String {
    let e = edits::apply(&doc(), &edits::parse(json_ops).unwrap()).unwrap_err();
    assert!(is_usage(&e), "{e}");
    e.to_string()
}

// --- reads ---------------------------------------------------------------------

#[test]
fn outline_reads_adf_and_lists_headings_with_local_ids() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let out = confluence::get(
        &srv.client(),
        "12345",
        &opts(Detail::Outline, ReadFormat::Html, &[]),
    )
    .unwrap();
    assert_eq!(out["detail"], "outline");
    assert!(out.get("body").is_none());
    assert_eq!(out["snapshotToken"], "7");
    let o = out["outline"].as_array().unwrap();
    assert_eq!(o.len(), 3);
    assert_eq!(
        o[0],
        json!({"level": 1, "text": "Runbook", "anchor": "Runbook", "localId": "h-top", "wordCount": 14})
    );
    assert_eq!(o[1]["localId"], "h-deploy");
    assert_eq!(o[1]["wordCount"], 9);
    assert_eq!(o[2]["text"], "Rollback");
    assert_eq!(o[2]["wordCount"], 2);
    let reqs = srv.requests();
    assert_eq!(reqs[0].path, "/api/v2/pages/12345");
    assert_eq!(
        reqs[0].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
}

#[test]
fn full_adf_read_exposes_local_ids() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let out = confluence::get(
        &srv.client(),
        "12345",
        &opts(Detail::Full, ReadFormat::Adf, &[]),
    )
    .unwrap();
    assert_eq!(out["body"]["format"], "adf");
    let parsed: Value = serde_json::from_str(out["body"]["value"].as_str().unwrap()).unwrap();
    assert_eq!(parsed, doc());
}

#[test]
fn section_reads_slice_storage() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, storage_page()),
        MockResponse::json(200, storage_page()),
        MockResponse::json(200, storage_page()),
        MockResponse::json(200, storage_page()),
    ]);
    let c = srv.client();

    // Exact storage slice in html, through the nested h3 and the code macro.
    let q = ["deploy & verify".to_string()];
    let out = confluence::get(&c, "12345", &opts(Detail::Full, ReadFormat::Html, &q)).unwrap();
    assert_eq!(out["body"]["format"], "html");
    let html = out["body"]["value"].as_str().unwrap();
    assert!(html.starts_with("<h2>Deploy &amp; Verify</h2>"), "{html}");
    assert!(html.ends_with("</ac:structured-macro>"), "{html}");
    assert!(!html.contains("Rollback"));
    assert_eq!(out["body"]["sections"][0]["text"], "Deploy & Verify");
    assert_eq!(out["body"]["sections"][0]["level"], 2);

    // Exact match wins over "contains"; Markdown output.
    let q = ["ROLLBACK".to_string()];
    let out = confluence::get(&c, "12345", &opts(Detail::Full, ReadFormat::Md, &q)).unwrap();
    assert_eq!(out["body"]["format"], "md");
    assert_eq!(out["body"]["value"], "## Rollback\n\nShort form.");

    // Repeated sections come back in document order; contained ones once.
    let q = [
        "Rollback plan".to_string(),
        "checks".to_string(),
        "Deploy".to_string(),
    ];
    let out = confluence::get(&c, "12345", &opts(Detail::Full, ReadFormat::Md, &q)).unwrap();
    let md = out["body"]["value"].as_str().unwrap();
    assert!(md.starts_with("## Deploy & Verify"), "{md}");
    assert!(md.contains("### Checks"));
    assert!(md.contains("## Rollback plan\n\nRevert the **release**."));
    assert!(!md.contains("Short form"));
    assert_eq!(out["body"]["sections"].as_array().unwrap().len(), 2);

    // No match: a usage error that lists the headings.
    let q = ["Escalation".to_string()];
    let e = confluence::get(&c, "12345", &opts(Detail::Full, ReadFormat::Md, &q)).unwrap_err();
    assert!(is_usage(&e));
    let msg = e.to_string();
    assert!(msg.contains("no heading matches 'Escalation'"), "{msg}");
    assert!(
        msg.contains("## Deploy & Verify") && msg.contains("### Checks"),
        "{msg}"
    );
    assert!(
        !msg.contains("fake"),
        "CDATA must not count as a heading: {msg}"
    );

    for r in srv.requests() {
        assert_eq!(r.query_param("body-format").as_deref(), Some("storage"));
    }
}

// --- edit operations -----------------------------------------------------------

#[test]
fn replace_node_fits_block_inline_and_list_targets() {
    let d =
        run(r#"[{"name":"replaceNode","localId":"p-intro","value":"<p>New <em>intro</em></p>"}]"#);
    assert_eq!(d["content"][1]["type"], "paragraph");
    assert_eq!(d["content"][1]["content"][0]["text"], "New ");
    assert_eq!(d["content"][1]["content"][1]["marks"][0]["type"], "em");
    // Untouched nodes are kept exactly, localIds included.
    assert_eq!(d["content"][3], doc()["content"][3]);
    assert_eq!(d["content"][5], doc()["content"][5]);

    // A list item replaced from an <li> (or a whole <ul>) stays a list item.
    let d = run(r#"[{"name":"replaceNode","localId":"li-2","value":"<ul><li>TWO</li></ul>"}]"#);
    let items = d["content"][4]["content"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["type"], "listItem");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "TWO");

    // A heading replaced by a heading; several blocks replace one node.
    let d = run(
        r#"[{"name":"replaceNode","localId":"h-rollback","value":"<h3>Undo</h3><p>extra</p>"}]"#,
    );
    assert_eq!(d["content"][6]["attrs"]["level"], 3);
    assert_eq!(d["content"][7]["content"][0]["text"], "extra");
    assert_eq!(d["content"].as_array().unwrap().len(), 9);
}

#[test]
fn insert_delete_remove_move_append_set_attrs() {
    let d = run(
        r#"[{"name":"insertNodeAfter","localId":"p-intro","value":"<p>after</p>"},
                    {"name":"insertNodeBefore","localId":"h-top","value":"<p>before</p>"}]"#,
    );
    assert_eq!(d["content"][0]["content"][0]["text"], "before");
    assert_eq!(d["content"][3]["content"][0]["text"], "after");

    let d = run(r#"[{"name":"deleteNode","localId":"macro"}]"#);
    assert!(!ids(&d).contains(&"macro".to_string()));

    let d =
        run(r#"[{"name":"removeNodes","arguments":{"localIds":["p-intro","li-1","p-rollback"]}}]"#);
    assert_eq!(
        ids(&d),
        [
            "h-top",
            "h-deploy",
            "p-deploy",
            "list",
            "macro",
            "h-rollback"
        ]
    );
    assert_eq!(d["content"][3]["content"].as_array().unwrap().len(), 1);

    let d = run(
        r#"[{"name":"moveNode","arguments":{"localId":"p-rollback","targetLocalId":"h-deploy","position":"before"}}]"#,
    );
    assert_eq!(ids(&d)[2], "p-rollback");
    let d = run(
        r#"[{"name":"moveNode","arguments":{"localId":"h-top","targetLocalId":"p-rollback","position":"after"}}]"#,
    );
    assert_eq!(ids(&d).last().unwrap(), "h-top");

    let d = run(r#"[{"name":"appendNodeToEnd","value":"<p>The end</p>"}]"#);
    assert_eq!(d["content"][8]["content"][0]["text"], "The end");

    let d = run(r#"[{"name":"setAttrs","localId":"h-deploy","arguments":{"attrs":{"level":3}}}]"#);
    assert_eq!(
        d["content"][2]["attrs"],
        json!({"level": 3, "localId": "h-deploy"})
    );
}

#[test]
fn text_and_section_extras() {
    // A match must sit inside one text node: "deploy" in bold is a separate run.
    let d = run(r#"[{"op":"replaceText","find":"deploy again","replace":"redeploy"}]"#);
    assert_eq!(
        d["content"][3]["content"][2]["text"],
        " job, then redeploy."
    );
    let e = run_err(r#"[{"op":"replaceText","find":"deploy","replace":"ship"}]"#);
    assert!(e.contains("found 2 times"), "{e}");
    let d = run(r#"[{"op":"replaceText","find":"deploy","replace":"ship","count":"all"}]"#);
    assert_eq!(d["content"][3]["content"][1]["text"], "ship");
    assert_eq!(
        d["content"][3]["content"][2]["text"],
        " job, then ship again."
    );
    assert_eq!(
        d["content"][2]["content"][0]["text"], "Deploy",
        "case-sensitive"
    );
    let e = run_err(r#"[{"op":"replaceText","find":"Run the deploy","replace":"x"}]"#);
    assert!(e.contains("not found"), "{e}");

    let d =
        run(r#"[{"op":"replaceSection","heading":"deploy","value":"New **body**","format":"md"}]"#);
    assert_eq!(
        ids(&d),
        [
            "h-top",
            "p-intro",
            "h-deploy",
            "-",
            "h-rollback",
            "p-rollback"
        ]
    );
    assert_eq!(d["content"][3]["content"][1]["marks"][0]["type"], "strong");

    let d = run(
        r#"[{"op":"appendToSection","heading":"Deploy","value":"<p>tail</p>"},
                    {"op":"prependToSection","heading":"Deploy","value":"<p>head</p>"}]"#,
    );
    assert_eq!(d["content"][3]["content"][0]["text"], "head");
    assert_eq!(d["content"][7]["content"][0]["text"], "tail");
    assert_eq!(ids(&d)[8], "h-rollback");

    let d = run(r#"[{"op":"deleteSection","heading":"Rollback"}]"#);
    assert_eq!(ids(&d).last().unwrap(), "macro");

    // The Runbook h1 section spans the whole page, so the new one goes last.
    let d = run(
        r#"[{"op":"insertSectionAfter","heading":"Deploy","newHeading":"Verify","value":"<p>check</p>"},
                    {"op":"insertSectionAfter","heading":"runbook","newHeading":"Appendix","level":1}]"#,
    );
    assert_eq!(d["content"][6]["attrs"]["level"], 2);
    assert_eq!(d["content"][6]["content"][0]["text"], "Verify");
    assert_eq!(d["content"][7]["content"][0]["text"], "check");
    assert_eq!(ids(&d)[8], "h-rollback");
    assert_eq!(d["content"][10]["content"][0]["text"], "Appendix");

    let e = run_err(r#"[{"op":"deleteSection","heading":"o"}]"#);
    assert!(e.contains("ambiguous"), "{e}");
    let e = run_err(r#"[{"op":"deleteSection","heading":"Missing"}]"#);
    assert!(e.contains("available headings"), "{e}");
}

#[test]
fn failed_operations_change_nothing() {
    let original = doc();
    for bad in [
        r#"[{"name":"deleteNode","localId":"p-intro"},{"name":"replaceNode","localId":"nope","value":"<p>x</p>"}]"#,
        r#"[{"name":"removeNodes","arguments":{"localIds":["p-intro","ghost"]}}]"#,
        r#"[{"name":"moveNode","arguments":{"localId":"list","targetLocalId":"li-1"}}]"#,
        r#"[{"name":"deleteNode","localId":"p-intro"},{"name":"deleteNode","localId":"p-intro"}]"#,
    ] {
        let ops = edits::parse(bad).unwrap();
        let e = edits::apply(&original, &ops).unwrap_err();
        assert!(is_usage(&e), "{bad}: {e}");
        assert!(e.to_string().contains("nothing was changed"), "{e}");
    }
    assert_eq!(original, doc());
}

// --- update --------------------------------------------------------------------

#[test]
fn update_with_edits_puts_adf_and_bumps_version() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, adf_page(&doc())),
        MockResponse::json(
            200,
            json!({"id": "12345", "title": "Runbook", "version": {"number": 8},
                   "_links": {"webui": "/spaces/ENG/pages/12345"}}),
        ),
    ]);
    let ops = edits::parse(
        r#"[{"name":"replaceNode","localId":"p-rollback","value":"<p>Revert <code>v2</code>.</p>"}]"#,
    )
    .unwrap();
    let mut e = edit(&ops, Some("7"), false);
    e.version_message = Some("fix rollback");
    let out = confluence::update(&srv.client(), "12345", e).unwrap();
    assert_eq!(out["snapshotToken"], "8");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(
        reqs[0].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
    assert_eq!(reqs[1].method, "PUT");
    assert_eq!(reqs[1].path, "/api/v2/pages/12345");
    let body = reqs[1].json_body();
    assert_eq!(body["version"]["number"], 8);
    assert_eq!(body["version"]["message"], "fix rollback");
    assert_eq!(body["title"], "Runbook");
    assert_eq!(body["status"], "current");
    assert_eq!(body["body"]["representation"], "atlas_doc_format");
    let sent: Value = serde_json::from_str(body["body"]["value"].as_str().unwrap()).unwrap();
    let mut want = doc();
    want["content"][7] = json!({"type": "paragraph", "content": [
        {"type": "text", "text": "Revert "},
        {"type": "text", "text": "v2", "marks": [{"type": "code"}]},
        {"type": "text", "text": "."}
    ]});
    assert_eq!(sent, want);
}

#[test]
fn edits_dry_run_shows_diff_and_makes_no_put() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let ops =
        edits::parse(r#"[{"name":"replaceNode","localId":"p-intro","value":"<p>Changed</p>"}]"#)
            .unwrap();
    let out = confluence::update(&srv.client(), "12345", edit(&ops, Some("7"), true)).unwrap();
    assert_eq!(out["dryRun"], true);
    assert_eq!(out["fromVersion"], 7);
    assert_eq!(out["request"]["version"]["number"], 8);
    assert_eq!(out["body"]["format"], "html");
    let html = out["body"]["value"].as_str().unwrap();
    assert!(
        html.contains("<p>Changed</p>") && !html.contains("Intro text"),
        "{html}"
    );
    let diff = out["diff"].as_str().unwrap();
    assert!(
        diff.starts_with("--- 12345 version 7\n+++ 12345 after edits\n@@ -1,5 +1,5 @@\n <h1>Runbook</h1>\n-<p>Intro text here</p>\n+<p>Changed</p>\n"),
        "{diff}"
    );
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
}

#[test]
fn edits_check_snapshot_and_fail_atomically_without_put() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, adf_page(&doc())),
        MockResponse::json(200, adf_page(&doc())),
    ]);
    let c = srv.client();
    let ops = edits::parse(r#"[{"name":"deleteNode","localId":"p-intro"}]"#).unwrap();
    let e = confluence::update(&c, "12345", edit(&ops, Some("6"), false)).unwrap_err();
    assert!(e.to_string().contains("token 6, current 7"), "{e}");

    let ops = edits::parse(
        r#"[{"name":"deleteNode","localId":"p-intro"},{"name":"deleteNode","localId":"ghost"}]"#,
    )
    .unwrap();
    let e = confluence::update(&c, "12345", edit(&ops, Some("7"), false)).unwrap_err();
    assert!(is_usage(&e));
    assert!(e.to_string().contains("edit 2 (deleteNode)"), "{e}");
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

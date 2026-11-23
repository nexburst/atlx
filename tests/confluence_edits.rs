//! Offline tests for Confluence outline and section reads and for `page
//! edit` operations.

mod common;

use atlx::client::{ApiError, ErrorKind};
use atlx::confluence::{self, GetOpts, VersionOpts, edits};
use atlx::markup::{BodyError, BodyFormat};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn is_usage(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>()
        .is_some_and(|a| a.kind == ErrorKind::Usage)
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
        {"type": "extension", "attrs": {"extensionType": "com.atlassian.confluence.macro.core", "extensionKey": "toc", "localId": "macro", "parameters": {"macroParams": {"maxLevel": {"value": "2"}}}}},
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

fn version(n: u64) -> VersionOpts<'static> {
    VersionOpts {
        if_version: n,
        ..Default::default()
    }
}

// --- reads ---------------------------------------------------------------------

#[test]
fn outline_reads_adf_and_lists_headings_with_local_ids() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let o = GetOpts {
        outline: true,
        ..Default::default()
    };
    let out = confluence::get(&srv.client(), "12345", &o).unwrap();
    assert_eq!(out["body"], Value::Null);
    let outline = out["outline"].as_array().unwrap();
    assert_eq!(outline.len(), 3);
    assert_eq!(outline[1]["localId"], "h-deploy");
    assert_eq!(outline[1]["text"], "Deploy");
    assert_eq!(
        srv.requests()[0].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
}

#[test]
fn reads_are_markdown_with_markers_or_adf_with_local_ids() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, adf_page(&doc())),
        MockResponse::json(200, adf_page(&doc())),
    ]);
    let c = srv.client();
    let md = confluence::get(&c, "12345", &GetOpts::default()).unwrap();
    let body = md["body"].as_str().unwrap();
    assert!(body.starts_with("# Runbook\n\nIntro text here"), "{body}");
    assert!(body.contains("<!-- atlx: extension:toc"), "{body}");
    assert_eq!(md["bodyFormat"], "markdown");
    assert_eq!(md["type"], "page");
    let adf = confluence::get(
        &c,
        "12345",
        &GetOpts {
            format: BodyFormat::Adf,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(adf["body"], doc());
}

#[test]
fn section_reads_slice_the_adf() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let sections = ["rollback".to_string()];
    let o = GetOpts {
        sections: &sections,
        ..Default::default()
    };
    let out = confluence::get(&srv.client(), "12345", &o).unwrap();
    assert_eq!(out["body"], "## Rollback\n\nRevert it.");

    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let missing = ["nope".to_string()];
    let o = GetOpts {
        sections: &missing,
        ..Default::default()
    };
    let e = confluence::get(&srv.client(), "12345", &o).unwrap_err();
    assert!(is_usage(&e));
    assert!(e.to_string().contains("## Deploy"), "{e}");

    let srv = MockServer::with_responses([]);
    let o = GetOpts {
        sections: &sections,
        format: BodyFormat::Storage,
        ..Default::default()
    };
    assert!(is_usage(
        &confluence::get(&srv.client(), "12345", &o).unwrap_err()
    ));
    assert!(srv.requests().is_empty());
}

// --- edit operations -----------------------------------------------------------

#[test]
fn replace_node_fits_block_inline_and_list_targets() {
    let d = run(r#"[{"name":"replaceNode","localId":"p-intro","value":"New *intro*"}]"#);
    assert_eq!(d["content"][1]["type"], "paragraph");
    assert_eq!(d["content"][1]["content"][0]["text"], "New ");
    assert_eq!(d["content"][1]["content"][1]["marks"][0]["type"], "em");
    // Untouched nodes are kept exactly, localIds included.
    assert_eq!(d["content"][3], doc()["content"][3]);
    assert_eq!(d["content"][5], doc()["content"][5]);

    // A list item replaced by a one-item list stays a list item.
    let d = run(r#"[{"name":"replaceNode","localId":"li-2","value":"- TWO"}]"#);
    let items = d["content"][4]["content"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["type"], "listItem");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "TWO");

    // A heading replaced by a heading; several blocks replace one node.
    let d = run(
        &json!([{"name": "replaceNode", "localId": "h-rollback", "value": "### Undo\n\nextra"}])
            .to_string(),
    );
    assert_eq!(d["content"][6]["attrs"]["level"], 3);
    assert_eq!(d["content"][7]["content"][0]["text"], "extra");
    assert_eq!(d["content"].as_array().unwrap().len(), 9);

    // ADF values go in as they are.
    let node = json!({"type": "rule"});
    let d = run(
        &json!([{"name": "replaceNode", "localId": "p-intro", "format": "adf", "value": node}])
            .to_string(),
    );
    assert_eq!(d["content"][1], node);
}

#[test]
fn insert_delete_remove_move_append_set_attrs() {
    let d = run(
        r#"[{"name":"insertNodeAfter","localId":"p-intro","value":"after"},
            {"name":"insertNodeBefore","localId":"h-top","value":"before"}]"#,
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

    let d = run(r#"[{"name":"appendNodeToEnd","value":"The end"}]"#);
    assert_eq!(d["content"][8]["content"][0]["text"], "The end");

    let d = run(r#"[{"name":"setAttrs","localId":"h-deploy","arguments":{"attrs":{"level":3}}}]"#);
    assert_eq!(
        d["content"][2]["attrs"],
        json!({"level": 3, "localId": "h-deploy"})
    );
}

#[test]
fn text_and_section_operations() {
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

    let d = run(r#"[{"op":"replaceSection","heading":"deploy","value":"New **body**"}]"#);
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
        r#"[{"op":"appendToSection","heading":"Deploy","value":"tail"},
            {"op":"prependToSection","heading":"Deploy","value":"head"}]"#,
    );
    assert_eq!(d["content"][3]["content"][0]["text"], "head");
    assert_eq!(d["content"][7]["content"][0]["text"], "tail");

    let d = run(r#"[{"op":"deleteSection","heading":"Rollback"}]"#);
    assert_eq!(ids(&d).last().unwrap(), "macro");

    let d = run(
        r#"[{"op":"insertSectionAfter","heading":"Deploy","newHeading":"Verify","value":"check"}]"#,
    );
    assert_eq!(d["content"][6]["content"][0]["text"], "Verify");
    assert_eq!(d["content"][7]["content"][0]["text"], "check");

    let e = run_err(r#"[{"op":"deleteSection","heading":"o"}]"#);
    assert!(e.contains("ambiguous"), "{e}");
}

#[test]
fn failed_operations_change_nothing() {
    let original = doc();
    for bad in [
        r#"[{"name":"deleteNode","localId":"p-intro"},{"name":"replaceNode","localId":"nope","value":"x"}]"#,
        r#"[{"name":"removeNodes","arguments":{"localIds":["p-intro","ghost"]}}]"#,
        r#"[{"name":"moveNode","arguments":{"localId":"list","targetLocalId":"li-1"}}]"#,
    ] {
        let ops = edits::parse(bad).unwrap();
        let e = edits::apply(&original, &ops).unwrap_err();
        assert!(is_usage(&e), "{bad}: {e}");
        assert!(e.to_string().contains("nothing was changed"), "{e}");
    }
    assert_eq!(original, doc());
}

#[test]
fn values_that_would_lose_content_are_refused() {
    for (ops, nodes) in [
        (
            r#"[{"name":"appendNodeToEnd","value":"<custom>x</custom>"}]"#,
            "html",
        ),
        (
            r#"[{"name":"replaceNode","localId":"p-intro","value":"a <span style=\"color:red\">b</span>"}]"#,
            "html",
        ),
    ] {
        let e = edits::parse(ops).unwrap_err();
        let b = e
            .downcast_ref::<BodyError>()
            .unwrap_or_else(|| panic!("{ops}: {e}"));
        assert!(b.is_refused(), "{ops}");
        assert!(
            b.nodes().iter().any(|n| n.contains(nodes)),
            "{ops}: {:?}",
            b.nodes()
        );
        let api = ApiError::from_anyhow(&e);
        assert_eq!(api.code(), "refused");
        assert_eq!(api.exit_code(), 8);
    }
    for f in ["html", "storage"] {
        let ops = format!(r#"[{{"name":"appendNodeToEnd","value":"<p>x</p>","format":"{f}"}}]"#);
        assert!(is_usage(&edits::parse(&ops).unwrap_err()), "{f}");
    }
}

// --- page edit ------------------------------------------------------------------

#[test]
fn edit_puts_adf_and_bumps_the_version() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, adf_page(&doc())),
        MockResponse::json(
            200,
            json!({"id": "12345", "title": "Runbook", "version": {"number": 8},
                   "_links": {"webui": "/spaces/ENG/pages/12345"}}),
        ),
    ]);
    let ops =
        edits::parse(r#"[{"name":"replaceNode","localId":"p-rollback","value":"Revert `v2`."}]"#)
            .unwrap();
    let o = VersionOpts {
        message: Some("fix rollback"),
        ..version(7)
    };
    let p = confluence::prepare_edit(&srv.client(), "12345", &ops, None, &o).unwrap();
    let out = confluence::send(&srv.client(), &p).unwrap();
    assert_eq!(out["version"]["number"], 8);
    assert_eq!(out["type"], "page");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[0].query_param("body-format").as_deref(),
        Some("atlas_doc_format")
    );
    assert_eq!(reqs[1].method, "PUT");
    let body = reqs[1].json_body();
    assert_eq!(body["version"]["number"], 8);
    assert_eq!(body["version"]["message"], "fix rollback");
    assert_eq!(body["title"], "Runbook");
    assert_eq!(body["body"]["representation"], "atlas_doc_format");
    let sent: Value = serde_json::from_str(body["body"]["value"].as_str().unwrap()).unwrap();
    let mut want = doc();
    want["content"][7] = json!({"type": "paragraph", "content": [
        {"type": "text", "text": "Revert "},
        {"type": "text", "text": "v2", "marks": [{"type": "code"}]},
        {"type": "text", "text": "."}
    ]});
    assert_eq!(sent, want);
    // The extension's JSON is byte-identical in what was sent.
    let raw = body["body"]["value"].as_str().unwrap();
    assert!(raw.contains(&doc()["content"][5].to_string()), "{raw}");
}

#[test]
fn edit_diff_is_markdown() {
    let srv = MockServer::with_responses([MockResponse::json(200, adf_page(&doc()))]);
    let ops =
        edits::parse(r#"[{"name":"replaceNode","localId":"p-intro","value":"Changed"}]"#).unwrap();
    let p = confluence::prepare_edit(&srv.client(), "12345", &ops, None, &version(7)).unwrap();
    let diff = confluence::diff(&p).unwrap();
    assert!(
        diff.starts_with("--- 12345 version 7\n+++ 12345 version 8\n"),
        "{diff}"
    );
    assert!(diff.contains("-Intro text here\n+Changed\n"), "{diff}");
    assert_eq!(srv.requests().len(), 1);
}

#[test]
fn edit_checks_the_version_and_fails_atomically_without_put() {
    let srv = MockServer::with_responses([
        MockResponse::json(200, adf_page(&doc())),
        MockResponse::json(200, adf_page(&doc())),
    ]);
    let c = srv.client();
    let ops = edits::parse(r#"[{"name":"deleteNode","localId":"p-intro"}]"#).unwrap();
    let e = confluence::prepare_edit(&c, "12345", &ops, None, &version(6)).unwrap_err();
    let api = ApiError::find(&e).unwrap();
    assert_eq!(api.kind, ErrorKind::Conflict);
    assert_eq!(api.exit_code(), 6);
    assert!(
        api.message.contains("at version 7, you edited version 6"),
        "{e}"
    );
    assert!(api.hint.as_deref().unwrap().contains("--if-version 7"));

    let ops = edits::parse(
        r#"[{"name":"deleteNode","localId":"p-intro"},{"name":"deleteNode","localId":"ghost"}]"#,
    )
    .unwrap();
    let e = confluence::prepare_edit(&c, "12345", &ops, None, &version(7)).unwrap_err();
    assert!(is_usage(&e));
    assert!(e.to_string().contains("edit 2 (deleteNode)"), "{e}");
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

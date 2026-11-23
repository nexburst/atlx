//! Offline tests for the Bitbucket pull request review loop: comments,
//! inline anchors, tasks, diffstat, commits, patch and diff --grep.

mod common;

use atlx::bitbucket::{self, Anchor, NewComment, Paging, TaskChange};
use atlx::client::ApiError;
use common::{MockResponse, MockServer, RecordedRequest};
use serde_json::{Value, json};

const REPO: &str = "/repositories/acme/api";

/// A server that echoes each request's method and path and answers 204 to
/// DELETE, so one test can drive several endpoints.
fn echo_server() -> MockServer {
    MockServer::with_handler(|req: &RecordedRequest| {
        if req.method == "DELETE" {
            return MockResponse::empty(204);
        }
        MockResponse::json(200, json!({"method": req.method, "path": req.path}))
    })
}

fn usage_message(err: anyhow::Error) -> String {
    let api = ApiError::find(&err).expect("an ApiError");
    assert_eq!(api.code(), ApiError::usage("x").code(), "{err}");
    err.to_string()
}

fn anchor_json(path: Option<&str>, a: Anchor) -> Value {
    bitbucket::inline_anchor(path, &a)
        .unwrap()
        .unwrap_or(Value::Null)
}

#[test]
fn inline_anchors_map_to_bitbucket_fields() {
    let a = Anchor::default;
    assert_eq!(anchor_json(None, a()), Value::Null);
    assert_eq!(
        anchor_json(Some("src/lib.rs"), a()),
        json!({"path": "src/lib.rs"})
    );
    assert_eq!(
        anchor_json(
            Some("src/lib.rs"),
            Anchor {
                line: Some(42),
                ..a()
            }
        ),
        json!({"path": "src/lib.rs", "to": 42})
    );
    assert_eq!(
        anchor_json(
            Some("src/lib.rs"),
            Anchor {
                from_line: Some(7),
                ..a()
            }
        ),
        json!({"path": "src/lib.rs", "from": 7})
    );
    assert_eq!(
        anchor_json(
            Some("src/lib.rs"),
            Anchor {
                line: Some(9),
                from_line: Some(8),
                ..a()
            }
        ),
        json!({"path": "src/lib.rs", "to": 9, "from": 8})
    );
    assert_eq!(
        anchor_json(
            Some("src/lib.rs"),
            Anchor {
                start_line: Some(17),
                end_line: Some(21),
                ..a()
            }
        ),
        json!({"path": "src/lib.rs", "start_to": 17, "to": 21})
    );
    assert_eq!(
        anchor_json(
            Some("src/lib.rs"),
            Anchor {
                start_from_line: Some(3),
                end_from_line: Some(5),
                ..a()
            }
        ),
        json!({"path": "src/lib.rs", "start_from": 3, "from": 5})
    );
}

#[test]
fn inline_anchor_errors_are_usage_errors() {
    let a = Anchor::default;
    let err = |path: Option<&str>, an: Anchor| {
        usage_message(bitbucket::inline_anchor(path, &an).unwrap_err())
    };
    assert!(
        err(
            None,
            Anchor {
                line: Some(1),
                ..a()
            }
        )
        .contains("--path")
    );
    assert!(
        err(
            Some("f"),
            Anchor {
                start_line: Some(1),
                ..a()
            }
        )
        .contains("--end-line")
    );
    assert!(
        err(
            Some("f"),
            Anchor {
                end_from_line: Some(1),
                ..a()
            }
        )
        .contains("--start-from-line")
    );
    assert!(
        err(
            Some("f"),
            Anchor {
                start_line: Some(5),
                end_line: Some(4),
                ..a()
            }
        )
        .contains("after")
    );
    assert!(
        err(
            Some("f"),
            Anchor {
                line: Some(2),
                start_from_line: Some(1),
                end_from_line: Some(3),
                ..a()
            }
        )
        .contains("not both")
    );
    assert!(
        err(
            Some("f"),
            Anchor {
                from_line: Some(0),
                ..a()
            }
        )
        .contains("--from-line")
    );
}

#[test]
fn comment_create_sends_reply_pending_and_range_anchor() {
    let srv = MockServer::with_responses([
        MockResponse::json(201, json!({"id": 100, "pending": true})),
        MockResponse::json(201, json!({"id": 101})),
    ]);
    let c = srv.client();
    let out = bitbucket::pr_comment(
        &c,
        REPO,
        42,
        NewComment {
            text: "Simplify this",
            reply_to: Some(7),
            pending: true,
            path: Some("src/main.rs"),
            anchor: Anchor {
                start_line: Some(10),
                end_line: Some(12),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert_eq!(out["id"], 100);
    bitbucket::pr_comment(
        &c,
        REPO,
        42,
        NewComment {
            text: "General note",
            ..Default::default()
        },
    )
    .unwrap();

    let reqs = srv.requests();
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(
        reqs[0].path,
        "/repositories/acme/api/pullrequests/42/comments"
    );
    assert_eq!(
        reqs[0].json_body(),
        json!({
            "content": {"raw": "Simplify this"},
            "parent": {"id": 7},
            "pending": true,
            "inline": {"path": "src/main.rs", "start_to": 10, "to": 12}
        })
    );
    assert_eq!(
        reqs[1].json_body(),
        json!({"content": {"raw": "General note"}})
    );
}

#[test]
fn comment_create_with_a_bad_anchor_sends_nothing() {
    let srv = MockServer::with_responses([]);
    let err = bitbucket::pr_comment(
        &srv.client(),
        REPO,
        42,
        NewComment {
            text: "x",
            anchor: Anchor {
                line: Some(3),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(usage_message(err).contains("--path"));
    assert!(srv.requests().is_empty());
}

#[test]
fn comment_get_update_delete_resolve_reopen() {
    let srv = echo_server();
    let c = srv.client();
    let base = "/repositories/acme/api/pullrequests/42/comments/9";

    let got = bitbucket::pr_comment_get(&c, REPO, 42, 9).unwrap();
    assert_eq!(got, json!({"method": "GET", "path": base}));

    let upd = bitbucket::pr_comment_update(&c, REPO, 42, 9, "Edited").unwrap();
    assert_eq!(upd["method"], "PUT");

    let del = bitbucket::pr_comment_delete(&c, REPO, 42, 9).unwrap();
    assert_eq!(del, json!({"id": 9, "deleted": true}));

    // Resolve and reopen read the comment back.
    let res = bitbucket::pr_comment_resolve(&c, REPO, 42, 9).unwrap();
    assert_eq!(res, json!({"method": "GET", "path": base}));

    let reo = bitbucket::pr_comment_reopen(&c, REPO, 42, 9).unwrap();
    assert_eq!(reo, json!({"method": "GET", "path": base}));

    let reqs = srv.requests();
    let calls: Vec<(&str, &str)> = reqs
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    let resolve = format!("{base}/resolve");
    assert_eq!(
        calls,
        [
            ("GET", base),
            ("PUT", base),
            ("DELETE", base),
            ("POST", resolve.as_str()),
            ("GET", base),
            ("DELETE", resolve.as_str()),
            ("GET", base),
        ]
    );
    assert_eq!(reqs[1].json_body(), json!({"content": {"raw": "Edited"}}));
    assert!(reqs[3].body.is_empty());
}

#[test]
fn comment_list_pages_in_order_and_drops_deleted() {
    let srv = MockServer::with_handler(|req| match req.query_param("page").as_deref() {
        None => MockResponse::json(
            200,
            json!({
                "values": [
                    {"id": 1, "content": {"raw": "a"}, "deleted": false},
                    {"id": 2, "content": {"raw": ""}, "deleted": true}
                ],
                "next": format!(
                    "http://{}/repositories/acme/api/pullrequests/42/comments?page=2",
                    req.header("host").unwrap()
                )
            }),
        ),
        _ => MockResponse::json(
            200,
            json!({"values": [{"id": 3, "inline": {"path": "f", "to": 4}}]}),
        ),
    });
    let (out, next) = bitbucket::pr_comment_list(&srv.client(), REPO, 42, &Paging::new(5)).unwrap();
    let ids: Vec<_> = out.iter().map(|c| &c["id"]).collect();
    assert_eq!(ids, [&json!(1), &json!(3)]);
    assert_eq!(next, None);
    let reqs = srv.requests();
    assert_eq!(
        reqs[0].path,
        "/repositories/acme/api/pullrequests/42/comments"
    );
    assert_eq!(reqs[0].query_param("sort").as_deref(), Some("created_on"));
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("50"));
}

#[test]
fn task_create_get_update_delete() {
    let srv = echo_server();
    let c = srv.client();
    let tasks = "/repositories/acme/api/pullrequests/42/tasks";
    let one = format!("{tasks}/6");

    bitbucket::pr_task_create(&c, REPO, 42, "Add a test", Some(12)).unwrap();
    bitbucket::pr_task_create(&c, REPO, 42, "Docs", None).unwrap();
    bitbucket::pr_task_get(&c, REPO, 42, 6).unwrap();
    bitbucket::pr_task_update(
        &c,
        REPO,
        42,
        6,
        &TaskChange {
            text: Some("Add two tests"),
            resolved: Some(true),
        },
    )
    .unwrap();
    bitbucket::pr_task_update(
        &c,
        REPO,
        42,
        6,
        &TaskChange {
            text: None,
            resolved: Some(false),
        },
    )
    .unwrap();
    let del = bitbucket::pr_task_delete(&c, REPO, 42, 6).unwrap();
    assert_eq!(del, json!({"id": 6, "deleted": true}));

    let reqs = srv.requests();
    let calls: Vec<(&str, &str)> = reqs
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("POST", tasks),
            ("POST", tasks),
            ("GET", one.as_str()),
            ("PUT", one.as_str()),
            ("PUT", one.as_str()),
            ("DELETE", one.as_str()),
        ]
    );
    assert_eq!(
        reqs[0].json_body(),
        json!({"content": {"raw": "Add a test"}, "comment": {"id": 12}})
    );
    assert_eq!(reqs[1].json_body(), json!({"content": {"raw": "Docs"}}));
    assert_eq!(
        reqs[3].json_body(),
        json!({"content": {"raw": "Add two tests"}, "state": "RESOLVED"})
    );
    assert_eq!(reqs[4].json_body(), json!({"state": "UNRESOLVED"}));
}

#[test]
fn task_update_needs_a_change() {
    let srv = MockServer::with_responses([]);
    let err =
        bitbucket::pr_task_update(&srv.client(), REPO, 42, 6, &TaskChange::default()).unwrap_err();
    assert!(usage_message(err).contains("--body"));
    assert!(srv.requests().is_empty());
}

#[test]
fn diffstat_commits_and_patch() {
    let srv = MockServer::with_handler(|req| match req.path.rsplit('/').next().unwrap() {
        "diffstat" => MockResponse::json(
            200,
            json!({"values": [
                {"status": "modified", "lines_added": 3, "lines_removed": 1,
                 "old": {"path": "a.rs"}, "new": {"path": "a.rs"}},
                {"status": "added", "lines_added": 9, "lines_removed": 0,
                 "old": null, "new": {"path": "b.rs"}}
            ]}),
        ),
        "commits" => MockResponse::json(
            200,
            json!({"values": [{"hash": "abc123", "message": "Fix\n\nbody"}]}),
        ),
        "patch" => MockResponse::text(200, "From abc123 Mon Sep 17 00:00:00 2001\n"),
        _ => MockResponse::text(404, "unexpected route"),
    });
    let c = srv.client();
    let (stat, _) = bitbucket::pr_diffstat(&c, REPO, 42, false, &Paging::new(1)).unwrap();
    assert_eq!(stat.len(), 1);
    assert_eq!(stat[0]["lines_added"], 3);
    let rec = atlx::schema::project(
        &stat[0],
        &bitbucket::records::DIFFSTAT,
        &atlx::schema::Selection::Default,
    )
    .unwrap();
    assert_eq!(
        rec,
        json!({"path": "a.rs", "status": "modified", "added": 3, "removed": 1})
    );
    let (commits, _) = bitbucket::pr_commits(&c, REPO, 42, &Paging::new(50)).unwrap();
    assert_eq!(commits[0]["hash"], "abc123");
    let patch = bitbucket::pr_diff_view(
        &c,
        REPO,
        42,
        &bitbucket::DiffOpts {
            patch: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(patch["diff"].as_str().unwrap().starts_with("From abc123"));

    let reqs = srv.requests();
    assert_eq!(
        reqs[0].path,
        "/repositories/acme/api/pullrequests/42/diffstat"
    );
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("1"));
    assert_eq!(
        reqs[1].path,
        "/repositories/acme/api/pullrequests/42/commits"
    );
    assert_eq!(reqs[2].path, "/repositories/acme/api/pullrequests/42/patch");
}

const DIFF: &str = "\
diff --git a/src/input.rs b/src/input.rs
index 1111111..2222222 100644
--- a/src/input.rs
+++ b/src/input.rs
@@ -10,3 +10,3 @@ fn main() {
 fn keep() {}
-fn validate_old() {}
+fn validate_input() {}
+// validate twice
--- not a header, a removed line
diff --git a/docs/old.md b/docs/new.md
similarity index 90%
rename from docs/old.md
rename to docs/new.md
--- a/docs/old.md
+++ b/docs/new.md
@@ -1,2 +1,2 @@
-Run validate first.
+Run validate before merging.
 The end.
\\ No newline at end of file
";

#[test]
fn diff_lines_give_new_and_old_line_numbers() {
    let all = bitbucket::diff_lines(DIFF, "validate", None);
    let picks: Vec<(String, String, Value, Value)> = all
        .iter()
        .map(|m| {
            (
                m["path"].as_str().unwrap().to_string(),
                m["type"].as_str().unwrap().to_string(),
                m["line"].clone(),
                m["fromLine"].clone(),
            )
        })
        .collect();
    let row = |p: &str, t: &str, l: Value, f: Value| (p.to_string(), t.to_string(), l, f);
    assert_eq!(
        picks,
        [
            row("src/input.rs", "removed", Value::Null, json!(11)),
            row("src/input.rs", "added", json!(11), Value::Null),
            row("src/input.rs", "added", json!(12), Value::Null),
            row("docs/new.md", "removed", Value::Null, json!(1)),
            row("docs/new.md", "added", json!(1), Value::Null),
        ]
    );
    assert_eq!(all[3]["oldPath"], "docs/old.md");
    assert_eq!(all[1]["text"], "fn validate_input() {}");
    assert!(
        all[0]["hunk"]
            .as_str()
            .unwrap()
            .starts_with("@@ -10,3 +10,3 @@")
    );

    // The removed line that looks like a `---` header is still diff content.
    let fake = bitbucket::diff_lines(DIFF, "not a header", None);
    assert_eq!(fake.len(), 1);
    assert_eq!(fake[0]["fromLine"], 12);

    let ctx = bitbucket::diff_lines(DIFF, "The end", Some("docs/old.md"));
    assert_eq!(ctx.len(), 1);
    assert_eq!(ctx[0]["type"], "context");
    assert_eq!(
        (&ctx[0]["line"], &ctx[0]["fromLine"]),
        (&json!(2), &json!(2))
    );

    assert!(bitbucket::diff_lines(DIFF, "validate", Some("README.md")).is_empty());
}

#[test]
fn pr_diff_grep_reads_the_diff_of_one_file() {
    let srv = MockServer::with_responses([MockResponse::text(200, DIFF)]);
    let o = bitbucket::DiffOpts {
        path: Some("src/input.rs"),
        grep: Some("validate_input"),
        ..Default::default()
    };
    let out = bitbucket::pr_diff_view(&srv.client(), REPO, 42, &o).unwrap();
    assert_eq!(out["diff"], Value::Null);
    assert_eq!(out["matches"].as_array().unwrap().len(), 1);
    assert_eq!(out["matches"][0]["line"], 11);
    let r = &srv.requests()[0];
    assert_eq!(r.path, "/repositories/acme/api/pullrequests/42/diff");
    assert_eq!(r.query_param("path").as_deref(), Some("src/input.rs"));

    let empty = bitbucket::DiffOpts {
        grep: Some(""),
        ..Default::default()
    };
    let err = bitbucket::pr_diff_view(&srv.client(), REPO, 42, &empty).unwrap_err();
    assert!(usage_message(err).contains("--grep"));
}

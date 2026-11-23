//! Offline tests for pipelines, steps, logs, deployments and environments.

mod common;

use atlx::bitbucket::{self, Paging, PipelineQuery, deployments, pipelines as pl};
use atlx::client::{ApiError, ErrorKind};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::time::Duration;

const REPO: &str = "/repositories/acme/api";

fn ok(body: Value) -> MockResponse {
    MockResponse::json(200, body)
}

fn page(values: Value) -> MockResponse {
    ok(json!({"values": values}))
}

fn run_state(state: &str, result: Option<&str>) -> MockResponse {
    let mut s = json!({"name": state});
    if let Some(r) = result {
        s["result"] = json!({"name": r});
    }
    ok(json!({"uuid": "{p}", "build_number": 7, "state": s}))
}

fn step(state: &str, result: Option<&str>) -> MockResponse {
    let mut s = json!({"name": state});
    if let Some(r) = result {
        s["result"] = json!({"name": r});
    }
    page(json!([{"uuid": "{s}", "name": "Test", "state": s}]))
}

fn tail_opts() -> pl::Tail<'static> {
    pl::Tail {
        step: None,
        lines: Some(10),
        follow: true,
        interval: Duration::from_millis(1),
        timeout: Duration::from_secs(5),
    }
}

fn tail(srv: &MockServer, o: &pl::Tail) -> (anyhow::Result<bool>, Vec<Value>) {
    let mut events = Vec::new();
    let out = pl::tail(&srv.client(), REPO, "7", o, &mut |e| {
        events.push(e.clone());
        Ok(())
    });
    (out, events)
}

fn texts(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["text"].as_str()).collect()
}

#[test]
fn run_builds_ref_and_commit_targets() {
    let srv = MockServer::with_responses([ok(json!({})), ok(json!({}))]);
    let c = srv.client();
    let on_branch = pl::Run {
        branch: Some("main"),
        custom: Some("nightly"),
        ..Default::default()
    };
    pl::run(&c, REPO, &on_branch).unwrap();
    let at_commit = pl::Run {
        commit: Some("abc"),
        custom: Some("nightly"),
        ..Default::default()
    };
    pl::run(&c, REPO, &at_commit).unwrap();
    let r = srv.requests();
    assert_eq!(r[0].path, "/repositories/acme/api/pipelines/");
    assert_eq!(
        r[0].json_body()["target"],
        json!({"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "main",
               "selector": {"type": "custom", "pattern": "nightly"}})
    );
    assert_eq!(
        r[1].json_body()["target"],
        json!({"type": "pipeline_commit_target", "commit": {"type": "commit", "hash": "abc"},
               "selector": {"type": "custom", "pattern": "nightly"}})
    );
}

#[test]
fn run_never_echoes_secured_values() {
    let srv = MockServer::with_responses([ok(json!({
        "variables": [{"key": "T", "value": "hunter2", "secured": true}]
    }))]);
    let o = pl::Run {
        branch: Some("main"),
        variables: vec![("T".into(), "hunter2".into(), true)],
        ..Default::default()
    };
    let out = pl::run(&srv.client(), REPO, &o).unwrap();
    assert!(!out.to_string().contains("hunter2"));
    assert_eq!(
        srv.requests()[0].json_body()["variables"][0]["secured"],
        true
    );

    let srv = MockServer::with_responses([]);
    let bare = pl::Run {
        commit: Some("abc"),
        ..Default::default()
    };
    assert!(pl::run(&srv.client(), REPO, &bare).is_err());
    assert!(srv.requests().is_empty());
}

#[test]
fn rerun_needs_a_failure_a_commit_and_readable_variables() {
    let failed = json!({"result": {"name": "FAILED"}});
    let target = json!({"commit": {"hash": "abc"}});
    for p in [
        json!({"uuid": "{p}", "target": target, "state": {"result": {"name": "SUCCESSFUL"}}}),
        json!({"uuid": "{p}", "target": target, "state": failed, "variables": [{"key": "T", "secured": true}]}),
        json!({"uuid": "{p}", "target": {"ref_name": "main"}, "state": failed}),
    ] {
        let srv = MockServer::with_responses([ok(p)]);
        assert!(pl::rerun(&srv.client(), REPO, "7").is_err());
        assert_eq!(srv.requests().len(), 1);
    }
}

#[test]
fn rerun_uses_the_same_commit() {
    let srv = MockServer::with_responses([
        ok(json!({
            "uuid": "{p}", "build_number": 7, "state": {"result": {"name": "FAILED"}},
            "target": {"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "main",
                       "commit": {"type": "commit", "hash": "abc", "links": {}}}
        })),
        ok(json!({"build_number": 8})),
    ]);
    let out = pl::rerun(&srv.client(), REPO, "7").unwrap();
    assert_eq!(out["rerunOf"], 7);
    assert_eq!(
        srv.requests()[1].json_body()["target"],
        json!({"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "main",
               "commit": {"type": "commit", "hash": "abc"}})
    );
}

#[test]
fn steps_are_numbered_and_one_comes_with_its_tests() {
    let steps = || page(json!([{"uuid": "{a}", "name": "Lint"}, {"uuid": "{b}", "name": "Test"}]));
    let srv = MockServer::with_responses([
        run_state("COMPLETED", Some("FAILED")),
        steps(),
        run_state("COMPLETED", Some("FAILED")),
        steps(),
        ok(json!({"number_of_failed_test_cases": 1})),
        page(json!([{"name": "t1", "status": "FAILED"}])),
    ]);
    let c = srv.client();
    let (list, next) = pl::step_list(&c, REPO, "7", &Paging::new(1)).unwrap();
    assert_eq!(list[0]["index"], 1);
    assert_eq!(next.as_deref(), Some("o:1"));

    let t = pl::Tests {
        cases: true,
        case_limit: 100,
    };
    let test = pl::step_get(&c, REPO, "7", "2", &t).unwrap();
    assert_eq!(test["index"], 2);
    assert_eq!(test["test_report"]["number_of_failed_test_cases"], 1);
    assert_eq!(test["test_cases"][0]["name"], "t1");
    let rec = atlx::schema::project(
        &test,
        &bitbucket::records::STEP,
        &atlx::schema::Selection::Default,
    )
    .unwrap();
    assert_eq!(
        rec["testCases"],
        json!([{"name": "t1", "status": "FAILED", "duration": null}])
    );
    assert!(
        srv.requests()[5]
            .path
            .ends_with("/steps/%7Bb%7D/test_reports/test_cases")
    );
}

#[test]
fn an_unknown_step_is_a_usage_error() {
    let srv = MockServer::with_responses([run_state("COMPLETED", None), step("COMPLETED", None)]);
    let o = pl::LogOpts {
        step: Some("Deploy"),
        ..Default::default()
    };
    let err = pl::logs(&srv.client(), REPO, Some("7"), &o).unwrap_err();
    assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
}

#[test]
fn list_matches_pattern_and_passed() {
    let runs = json!([
        {"uuid": "{1}", "state": {"result": {"name": "SUCCESSFUL"}}, "target": {"selector": {"pattern": "nightly"}}},
        {"uuid": "{2}", "state": {"result": {"name": "SUCCESSFUL"}}, "target": {}}
    ]);
    let srv = MockServer::with_responses([page(runs)]);
    let q = PipelineQuery {
        result: Some("passed"),
        pattern: Some("nightly"),
        ..Default::default()
    };
    let (out, _) = bitbucket::pipeline_list(&srv.client(), REPO, &q, &Paging::new(5)).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["uuid"], "{1}");
}

#[test]
fn logs_of_the_newest_failure_keep_only_failed_steps() {
    let srv = MockServer::with_responses([
        page(json!([
            {"uuid": "{ok}", "state": {"result": {"name": "SUCCESSFUL"}}},
            {"uuid": "{p}", "build_number": 7, "state": {"result": {"name": "FAILED"}}}
        ])),
        page(json!([
            {"uuid": "{s1}", "name": "Lint", "state": {"name": "COMPLETED", "result": {"name": "SUCCESSFUL"}}},
            {"uuid": "{s2}", "name": "Test", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}}
        ])),
        MockResponse::text(200, "123"),
    ]);
    let o = pl::LogOpts {
        failed: true,
        branch: Some("main"),
        ..Default::default()
    };
    let out = pl::logs(&srv.client(), REPO, None, &o).unwrap();
    let recs = out.as_array().unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["step"], "Test");
    assert_eq!(recs[0]["stepIndex"], 2);
    assert_eq!(recs[0]["pipeline"], 7);
    assert_eq!(recs[0]["log"], "123", "a log that is valid JSON stays text");
    let r = srv.requests();
    assert_eq!(r[0].query_param("target.branch").as_deref(), Some("main"));
    assert!(r[2].path.ends_with("/steps/%7Bs2%7D/log"), "{}", r[2].path);

    let srv = MockServer::with_responses([page(json!([]))]);
    let err = pl::logs(&srv.client(), REPO, None, &o).unwrap_err();
    assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::NotFound);
}

#[test]
fn grep_numbers_lines_and_stops_at_the_cap() {
    let srv = MockServer::with_responses([
        run_state("COMPLETED", None),
        step("COMPLETED", None),
        MockResponse::text(200, "one\nERROR a\nthree\nerror b\n"),
    ]);
    let o = pl::LogOpts {
        grep: Some(pl::Grep {
            pattern: "error",
            ignore_case: true,
            context: 1,
            max_matches: 1,
        }),
        ..Default::default()
    };
    let out = pl::logs(&srv.client(), REPO, Some("7"), &o).unwrap();
    let rec = &out[0];
    assert_eq!(rec["log"], Value::Null);
    let m = &rec["matches"][0];
    assert_eq!(m["line"], 2);
    assert_eq!(m["before"], json!(["one"]));
    assert_eq!(m["after"], json!(["three"]));
    assert_eq!(rec["matches"].as_array().unwrap().len(), 1);
}

#[test]
fn tail_follows_with_ranges_and_joins_split_lines() {
    let srv = MockServer::with_responses([
        run_state("IN_PROGRESS", None),
        step("IN_PROGRESS", None),
        MockResponse::text(200, "old\nnew\nhal"),
        run_state("COMPLETED", Some("SUCCESSFUL")),
        step("COMPLETED", Some("SUCCESSFUL")),
        MockResponse::text(206, "f\nend\n"),
    ]);
    let (out, events) = tail(
        &srv,
        &pl::Tail {
            lines: Some(1),
            ..tail_opts()
        },
    );
    assert!(out.unwrap());
    assert_eq!(texts(&events), ["new", "half", "end"]);
    assert_eq!(events.last().unwrap()["event"], "done");
    let r = srv.requests();
    assert_eq!(r[2].header("range"), None);
    assert_eq!(r[5].header("range"), Some("bytes=11-"));
}

#[test]
fn tail_copes_with_a_full_body_and_416() {
    let srv = MockServer::with_responses([
        run_state("IN_PROGRESS", None),
        step("IN_PROGRESS", None),
        MockResponse::text(200, "a\n"),
        run_state("IN_PROGRESS", None),
        step("IN_PROGRESS", None),
        MockResponse::empty(416),
        run_state("COMPLETED", Some("FAILED")),
        step("COMPLETED", Some("FAILED")),
        MockResponse::text(200, "a\nb\n"),
    ]);
    let (out, events) = tail(&srv, &tail_opts());
    assert!(!out.unwrap(), "a failed pipeline is a failure");
    assert_eq!(texts(&events), ["a", "b"]);
}

#[test]
fn tail_without_follow_and_at_the_timeout() {
    let running = || {
        MockServer::with_responses([
            run_state("IN_PROGRESS", None),
            step("IN_PROGRESS", None),
            MockResponse::text(200, "a\npartial"),
        ])
    };
    let (out, events) = tail(
        &running(),
        &pl::Tail {
            follow: false,
            ..tail_opts()
        },
    );
    assert!(out.unwrap());
    assert_eq!(texts(&events), ["a", "partial"]);

    let (out, _) = tail(
        &running(),
        &pl::Tail {
            timeout: Duration::ZERO,
            ..tail_opts()
        },
    );
    assert_eq!(
        ApiError::find(&out.unwrap_err()).unwrap().kind,
        ErrorKind::Timeout
    );
}

#[test]
fn tail_stops_at_a_paused_pipeline() {
    let srv = MockServer::with_responses([
        ok(json!({"uuid": "{p}", "state": {"name": "IN_PROGRESS", "stage": {"name": "PAUSED"}}})),
        step("PENDING", None),
    ]);
    let (out, events) = tail(&srv, &tail_opts());
    assert!(!out.unwrap());
    assert_eq!(events[0]["result"], "PAUSED");
}

#[test]
fn config_reads_and_writes() {
    let srv = MockServer::with_responses([ok(json!({})), ok(json!({}))]);
    let c = srv.client();
    pl::config_get(&c, REPO).unwrap();
    pl::config_update(&c, REPO, false).unwrap();
    let r = srv.requests();
    assert_eq!(r[1].method, "PUT");
    assert_eq!(r[1].path, "/repositories/acme/api/pipelines_config");
    assert_eq!(r[1].json_body(), json!({"enabled": false}));
}

#[test]
fn environment_variables_hide_secured_values() {
    let srv = MockServer::with_responses([
        page(json!([{"uuid": "{env}", "name": "Production", "slug": "production"}])),
        page(
            json!([{"key": "A", "value": "1", "secured": false}, {"key": "B", "value": "s3cret", "secured": true}]),
        ),
    ]);
    let (out, _) =
        deployments::variables(&srv.client(), REPO, "production", &Paging::new(25)).unwrap();
    assert_eq!(out[0]["value"], "1");
    assert!(out[1].get("value").is_none());
    assert!(!json!(out).to_string().contains("s3cret"));
    assert_eq!(
        srv.requests()[1].path,
        "/repositories/acme/api/deployments_config/environments/%7Benv%7D/variables"
    );
}

fn deployment(id: &str, env: &str, status: &str, at: &str, hash: &str) -> Value {
    json!({"uuid": id, "environment": {"uuid": env},
           "state": {"name": "COMPLETED", "status": {"name": status}, "started_on": at},
           "release": {"commit": {"hash": hash}}})
}

#[test]
fn filtered_lists_sort_newest_first_with_an_offset_cursor() {
    let env = "{11111111-1111-1111-1111-111111111111}";
    let srv = MockServer::with_responses([page(json!([
        deployment("{1}", env, "SUCCESSFUL", "2026-01-01T00:00:00Z", "a"),
        deployment("{2}", env, "FAILED", "2026-01-02T00:00:00Z", "b"),
        deployment("{3}", env, "SUCCESSFUL", "2026-01-03T00:00:00.123456Z", "c"),
        deployment("{4}", "{other}", "SUCCESSFUL", "2026-01-04T00:00:00Z", "d")
    ]))]);
    let statuses = ["successful".to_string()];
    let o = deployments::Filter {
        environment: Some(env),
        statuses: &statuses,
        ..Default::default()
    };
    let (out, next) =
        deployments::deployment_list(&srv.client(), REPO, &o, &Paging::new(1)).unwrap();
    assert_eq!(out[0]["uuid"], "{3}");
    assert_eq!(next.as_deref(), Some("o:1"));
    let srv = MockServer::with_responses([page(json!([
        deployment("{1}", env, "SUCCESSFUL", "2026-01-01T00:00:00Z", "a"),
        deployment("{3}", env, "SUCCESSFUL", "2026-01-03T00:00:00.123456Z", "c"),
    ]))]);
    let p = Paging {
        limit: 5,
        cursor: next,
    };
    let (out, next) = deployments::deployment_list(&srv.client(), REPO, &o, &p).unwrap();
    let ids: Vec<&str> = out.iter().map(|d| d["uuid"].as_str().unwrap()).collect();
    assert_eq!(ids, ["{1}"]);
    assert_eq!(next, None);
}

#[test]
fn commits_compare_with_the_previous_success() {
    let env = "{11111111-1111-1111-1111-111111111111}";
    let new = deployment("{3}", env, "SUCCESSFUL", "2026-01-03T00:00:00Z", "c");
    let srv = MockServer::with_responses([
        ok(new.clone()),
        page(json!([
            deployment("{1}", env, "SUCCESSFUL", "2026-01-01T00:00:00Z", "a"),
            deployment("{2}", env, "FAILED", "2026-01-02T00:00:00Z", "b"),
            new
        ])),
        page(json!([{"hash": "c"}])),
        page(json!([])),
    ]);
    let out = deployments::commits(&srv.client(), REPO, "{3}", Some(env), 50).unwrap();
    assert_eq!(out["previous"], "{1}");
    assert_eq!(out["environment"], env);
    assert_eq!(out["added"][0]["hash"], "c");
    let r = &srv.requests()[2];
    assert_eq!(r.query_param("include").as_deref(), Some("c"));
    assert_eq!(r.query_param("exclude").as_deref(), Some("a"));
}

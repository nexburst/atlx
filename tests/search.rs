//! Offline tests for cross-product search.

mod common;

use atlx::config::Config;
use atlx::{Product, search};
use common::{MockResponse, MockServer};
use serde_json::json;

#[test]
fn search_fans_out_in_parallel_and_isolates_product_failures() {
    let jira_srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"issues": [{"key": "J-1", "fields": {"summary": "Deploy"}}]}),
        ),
        MockResponse::json(
            200,
            json!({"issues": [{"key": "J-2", "fields": {"summary": "Still works"}}]}),
        ),
    ]);
    let conf_srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"results": [{"title": "Deploy Guide", "content": {"id": "10", "type": "page"}, "url": "/pages/10"}]}),
        ),
        MockResponse::json(500, json!({"message": "Confluence unavailable"})),
    ]);
    let bb_srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({"values": [{"full_name": "acme/deploy", "name": "deploy"}]}),
        ),
        MockResponse::json(
            200,
            json!({"values": [{"full_name": "acme/infra", "name": "infra"}]}),
        ),
    ]);

    // 1. All three products succeed in parallel
    let ok = search::run_with(
        "deploy",
        5,
        &[],
        true,
        || Ok(jira_srv.client()),
        || Ok(conf_srv.client()),
        || Ok((bb_srv.client(), Some("acme".into()))),
    );
    assert_eq!(ok["query"], "deploy");
    assert_eq!(ok["jira"][0]["key"], "J-1");
    assert_eq!(ok["confluence"][0]["title"], "Deploy Guide");
    assert_eq!(ok["bitbucket"][0]["full_name"], "acme/deploy");

    // 2. Confluence fails with HTTP 500 while Jira and Bitbucket still return results
    let partial = search::run_with(
        "deploy",
        5,
        &[],
        true,
        || Ok(jira_srv.client()),
        || Ok(conf_srv.client()),
        || Ok((bb_srv.client(), Some("acme".into()))),
    );
    assert_eq!(partial["jira"][0]["key"], "J-2");
    assert_eq!(partial["bitbucket"][0]["full_name"], "acme/infra");
    let conf_err = &partial["confluence"]["error"];
    assert_eq!(conf_err["statusCode"], 500, "{conf_err}");
    assert_eq!(conf_err["category"], "server", "{conf_err}");
    assert_eq!(conf_err["code"], "TWG_UPSTREAM_FAILURE", "{conf_err}");
    assert_eq!(conf_err["message"], "Confluence unavailable", "{conf_err}");

    // 3. Direct search::run with unconfigured Config isolates credential error & skips unconfigured Bitbucket
    let empty_cfg = Config::default();
    let cfg_err = search::run(&empty_cfg, "deploy", 5, &[]);
    assert!(
        cfg_err["jira"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no email configured")
    );
    assert!(
        cfg_err["confluence"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no email configured")
    );
    assert_eq!(cfg_err["jira"]["error"]["category"], "config");
    assert!(cfg_err.get("bitbucket").is_none());

    // Explicit `--only bitbucket` includes bitbucket error even when unconfigured
    let only_bb = search::run(&empty_cfg, "deploy", 5, &[Product::Bitbucket]);
    assert!(only_bb.get("jira").is_none());
    assert!(
        only_bb["bitbucket"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no Bitbucket email configured")
    );
}

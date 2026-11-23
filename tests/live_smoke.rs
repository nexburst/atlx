//! Opt-in read-only smoke tests against real Atlassian Cloud / Bitbucket APIs.
//!
//! Run with:
//!   ATLX_SITE=... ATLX_EMAIL=... ATLX_TOKEN=... cargo test --test live_smoke -- --ignored
//!
//! Optional variables:
//!   ATLX_TEST_ISSUE    Jira issue key to fetch (e.g. PROJ-1)
//!   ATLX_TEST_PAGE     Confluence page ID or URL to fetch
//!   ATLX_BB_WORKSPACE  Bitbucket workspace slug (enables Bitbucket checks)
//!   ATLX_BB_REPO       Bitbucket repo slug or workspace/slug
//!   ATLX_BB_EMAIL      Override Bitbucket email (defaults to ATLX_EMAIL)
//!   ATLX_BB_TOKEN      Override Bitbucket token (defaults to ATLX_TOKEN)

use atlx::bitbucket::{self, RepoQuery};
use atlx::config::{Bitbucket, Config};
use atlx::confluence::{self, GetOpts as ConfGetOpts};
use atlx::jira;

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn req_env(key: &str) -> String {
    env_opt(key).unwrap_or_else(|| panic!("set {key} to run live smoke tests"))
}

fn atlassian_config() -> Config {
    Config {
        site: Some(req_env("ATLX_SITE")),
        email: Some(req_env("ATLX_EMAIL")),
        token: Some(req_env("ATLX_TOKEN")),
        bitbucket: Bitbucket {
            workspace: env_opt("ATLX_BB_WORKSPACE"),
            email: env_opt("ATLX_BB_EMAIL"),
            token: env_opt("ATLX_BB_TOKEN"),
            ..Default::default()
        },
        timeout_ms: Some(30_000),
        ..Default::default()
    }
}

#[test]
#[ignore]
fn live_jira_myself_and_search() {
    let cfg = atlassian_config();
    let client = cfg.jira().expect("build Jira client");

    let me = jira::myself(&client).expect("jira::myself failed");
    assert!(
        me["accountId"].as_str().is_some_and(|s| !s.is_empty()),
        "expected accountId in {me}"
    );

    let results = jira::search(&client, "test", 5, None, None).expect("jira::search failed");
    assert!(
        results.data["issues"].is_array(),
        "expected issues array in {}",
        results.data
    );

    if let Some(key) = env_opt("ATLX_TEST_ISSUE") {
        let issue = jira::issue(&client, &key).expect("jira::issue failed");
        assert_eq!(issue["key"], key);
    }
}

#[test]
#[ignore]
fn live_confluence_spaces_and_optional_page() {
    let cfg = atlassian_config();
    let client = cfg.confluence().expect("build Confluence client");

    let spaces =
        confluence::spaces(&client, None, None, None, 5, None).expect("confluence::spaces failed");
    assert!(
        spaces["results"].is_array(),
        "expected spaces results array in {spaces}"
    );

    if let Some(page) = env_opt("ATLX_TEST_PAGE") {
        let content = confluence::get(&client, &page, &ConfGetOpts::default())
            .expect("confluence::get failed");
        assert!(
            content["id"].as_str().is_some(),
            "expected content id in {content}"
        );
    }
}

#[test]
#[ignore]
fn live_bitbucket_repo_query() {
    let Some(ws) = env_opt("ATLX_BB_WORKSPACE") else {
        eprintln!("skipping Bitbucket live check: ATLX_BB_WORKSPACE not set");
        return;
    };
    let cfg = Config {
        email: env_opt("ATLX_EMAIL"),
        token: env_opt("ATLX_TOKEN"),
        bitbucket: Bitbucket {
            workspace: Some(ws.clone()),
            email: env_opt("ATLX_BB_EMAIL"),
            token: env_opt("ATLX_BB_TOKEN"),
            ..Default::default()
        },
        timeout_ms: Some(30_000),
        ..Default::default()
    };
    let (client, default_ws) = cfg.bitbucket().expect("build Bitbucket client");
    let ws = bitbucket::workspace_or(None, default_ws.as_deref()).unwrap();

    let repos = bitbucket::repo_query(
        &client,
        ws,
        &RepoQuery {
            query: None,
            project: None,
            language: None,
            sort: "-updated_on",
            role: None,
            limit: 5,
        },
    )
    .expect("bitbucket::repo_query failed");
    assert!(repos.is_array(), "expected repos array in {repos}");

    if let Some(repo) = env_opt("ATLX_BB_REPO") {
        let path =
            bitbucket::repo_path(Some(&repo), Some(ws), Some(ws)).expect("resolve repo_path");
        let got = bitbucket::repo_get(&client, &path).expect("bitbucket::repo_get failed");
        assert!(
            got["repo"].as_str().is_some(),
            "expected repo full_name in {got}"
        );
    }
}

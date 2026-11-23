//! Offline tests for the HTTP client (auth, retries, error extraction).

mod common;

use atlx::client::{ApiError, ErrorKind};
use base64::Engine;
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

#[test]
fn client_sends_basic_auth_headers_and_query() {
    let srv = MockServer::with_responses([MockResponse::json(200, json!({"ok": true}))]);
    let client = srv.client();

    let v = client
        .get(
            "/rest/api/3/myself",
            &[("expand", "groups"), ("q", "hello world")],
        )
        .unwrap();
    assert_eq!(v, json!({"ok": true}));

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.path, "/rest/api/3/myself");
    assert_eq!(r.query_param("expand").as_deref(), Some("groups"));
    assert_eq!(r.query_param("q").as_deref(), Some("hello world"));
    assert_eq!(r.header("accept"), Some("application/json"));

    let expected_creds =
        base64::engine::general_purpose::STANDARD.encode("me@example.com:secret-token");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Basic {expected_creds}").as_str())
    );
}

#[test]
fn client_retries_429_honoring_retry_after_and_succeeds() {
    let srv = MockServer::with_responses([
        MockResponse::json(429, json!({"message": "slow down"})).with_header("Retry-After", "0"),
        MockResponse::json(429, json!({"message": "still busy"})).with_header("Retry-After", "0"),
        MockResponse::json(200, json!({"recovered": true})),
    ]);
    let client = srv.client();

    let v = client.get("/ping", &[]).unwrap();
    assert_eq!(v, json!({"recovered": true}));
    assert_eq!(srv.requests().len(), 3);
}

#[test]
fn client_fails_after_max_retries_on_429() {
    // MAX_RETRIES = 3 => 1 initial attempt + 3 retries = 4 total attempts.
    let srv = MockServer::with_responses([
        MockResponse::json(429, json!({"message": "rate limited"})).with_header("Retry-After", "0"),
        MockResponse::json(429, json!({"message": "rate limited"})).with_header("Retry-After", "0"),
        MockResponse::json(429, json!({"message": "rate limited"})).with_header("Retry-After", "0"),
        MockResponse::json(429, json!({"message": "rate limited"})).with_header("Retry-After", "0"),
    ]);
    let client = srv.client();

    let err = client.get("/ping", &[]).unwrap_err().to_string();
    assert!(err.contains("HTTP 429"), "unexpected error: {err}");
    assert!(err.contains("rate limited"), "unexpected error: {err}");
    assert_eq!(srv.requests().len(), 4);
}

#[test]
fn client_extracts_product_errors_html_pages_and_auth_hints() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            400,
            json!({"errorMessages": ["Bad JQL"], "errors": {"jql": "syntax error"}}),
        ),
        MockResponse::json(
            404,
            json!({"errors": [{"status": 404, "title": "Page not found"}]}),
        ),
        MockResponse::json(
            403,
            json!({"type": "error", "error": {"message": "Forbidden repo"}}),
        ),
        MockResponse::json(401, json!({"message": "Bad credentials"})),
        MockResponse::html(404, "<html><body>Not enabled</body></html>"),
    ]);
    let client = srv.client();

    let jira_err = client.get("/jira", &[]).unwrap_err().to_string();
    assert!(jira_err.contains("HTTP 400"));
    assert!(
        jira_err.contains("Bad JQL; jql: syntax error"),
        "{jira_err}"
    );

    let conf_err = client.get("/conf", &[]).unwrap_err().to_string();
    assert!(conf_err.contains("HTTP 404"));
    assert!(conf_err.contains("Page not found"), "{conf_err}");

    let bb_err = client.get("/bb", &[]).unwrap_err().to_string();
    assert!(bb_err.contains("HTTP 403"));
    assert!(bb_err.contains("Forbidden repo"), "{bb_err}");
    assert!(bb_err.contains("permission or token scope"), "{bb_err}");

    let unauth_err = client.get("/auth", &[]).unwrap_err().to_string();
    assert!(unauth_err.contains("HTTP 401"));
    assert!(unauth_err.contains("atlx auth status"), "{unauth_err}");

    let html_err = client.get("/html", &[]).unwrap_err().to_string();
    assert!(
        html_err.contains("non-JSON (HTML) error page"),
        "{html_err}"
    );
}

#[test]
fn client_handles_204_empty_and_plain_text_bodies() {
    let srv = MockServer::with_responses([
        MockResponse::empty(204),
        MockResponse::text(200, "   \n"),
        MockResponse::text(200, "line 1\nline 2"),
    ]);
    let client = srv.client();

    assert_eq!(client.put("/a", &json!({})).unwrap(), Value::Null);
    assert_eq!(client.get("/b", &[]).unwrap(), Value::Null);
    assert_eq!(
        client.get("/c", &[]).unwrap(),
        Value::String("line 1\nline 2".into())
    );
}

#[test]
fn client_refuses_dot_segments_before_sending() {
    // Percent-encoding leaves dots alone and URL parsers resolve them, so
    // each of these would reach a different resource than the one named.
    for path in [
        "/rest/api/3/issue/PROJ-1/comment/..",
        "/rest/api/3/issue/PROJ-1/comment/.",
        "/rest/api/3/issue/PROJ-1/../PROJ-2",
        "/issue/PROJ-1/comment/..?notify=false",
        "/issue/PROJ-1/comment/..#top",
        "/issue/PROJ-1/comment/%2e%2E",
        "/issue/PROJ-1/comment/.%2e",
        "/issue/PROJ-1/comment/..%2F..",
        "/issue/PROJ-1/comment\\..",
        "/issue/PROJ-1/comment/.\t.",
        "/issue/PROJ-1/comment/.. ",
    ] {
        for method in ["GET", "POST", "PUT", "DELETE"] {
            let srv = MockServer::with_responses([MockResponse::json(200, json!({}))]);
            let err = srv.client().request(method, path, &[], None).unwrap_err();
            let api = ApiError::find(&err).expect("a typed error");
            assert_eq!(api.kind, ErrorKind::Usage, "{method} {path:?}");
            assert!(srv.requests().is_empty(), "{method} {path:?} was sent");
        }
    }
}

#[test]
fn client_refuses_dot_segments_in_absolute_urls_and_downloads() {
    let srv = MockServer::with_responses([MockResponse::json(200, json!({}))]);
    let c = srv.client();
    let url = format!("{}/issue/PROJ-1/comment/..", srv.base_url());
    assert!(c.delete(&url, &[]).is_err());
    assert!(c.get_bytes("/attachment/..", &[]).is_err());
    assert!(c.post_multipart("/issue/../attachments", &[], &[]).is_err());
    assert!(srv.requests().is_empty());
}

#[test]
fn client_refuses_delete_of_an_empty_id() {
    for path in [
        "/issue/PROJ-1/comment/",
        "/branches/?name=x",
        "/issue//votes",
    ] {
        let srv = MockServer::with_responses([MockResponse::json(200, json!({}))]);
        let err = srv.client().delete(path, &[]).unwrap_err();
        assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
        assert!(srv.requests().is_empty(), "{path:?} was sent");
    }
}

#[test]
fn client_allows_dots_inside_segments_and_queries() {
    for path in [
        "/src/main/.github/a..b.rs",
        "/refs/branches/feature%2Fv1.2",
        "/repositories/acme/api/pipelines/",
        "/files/...",
        "/search?path=../x",
    ] {
        let srv = MockServer::with_responses([MockResponse::json(200, json!({}))]);
        srv.client().get(path, &[]).unwrap();
        assert_eq!(srv.requests().len(), 1, "{path:?}");
    }
}

use atlx::bitbucket::{
    self, NewComment, NewPr, PipelineGet, PipelineQuery, PrGet, PrQuery, RepoQuery,
};
use atlx::client::Client;
use atlx::config::Config;
use atlx::confluence::{
    self, ContentType, Detail, Edit, GetOpts as ConfGetOpts, NewContent, ReadFormat,
};
use atlx::jira::{self, GetOpts as JiraGetOpts, ListEdit, NewWorkitem, Update};
use atlx::markup::TextFormat;
use atlx::{Product, search};
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    target: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl RecordedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json_body(&self) -> Value {
        serde_json::from_str(&self.body).expect("request body should be valid JSON")
    }

    fn query_param(&self, key: &str) -> Option<String> {
        if self.query.is_empty() {
            return None;
        }
        for pair in self.query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if decode_query_component(k) == key {
                return Some(decode_query_component(v));
            }
        }
        None
    }
}

fn decode_query_component(s: &str) -> String {
    let plus_as_space = s.replace('+', " ");
    urlencoding::decode(&plus_as_space)
        .map(|c| c.into_owned())
        .unwrap_or(plus_as_space)
}

#[derive(Debug, Clone)]
struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    content_type: Option<String>,
}

impl MockResponse {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: serde_json::to_vec(&body).unwrap(),
            content_type: Some("application/json".into()),
        }
    }

    fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
            content_type: Some("text/plain; charset=utf-8".into()),
        }
    }

    fn html(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
            content_type: Some("text/html; charset=utf-8".into()),
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            content_type: None,
        }
    }

    fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

type Handler = Box<dyn Fn(&RecordedRequest) -> MockResponse + Send + Sync>;

struct MockServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stopped: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    fn with_responses(responses: impl IntoIterator<Item = MockResponse>) -> Self {
        let queue = Mutex::new(responses.into_iter().collect::<VecDeque<_>>());
        Self::with_handler(move |req| {
            queue.lock().unwrap().pop_front().unwrap_or_else(|| {
                MockResponse::text(
                    500,
                    format!("unexpected request: {} {}", req.method, req.target),
                )
            })
        })
    }

    fn with_handler(
        handler: impl Fn(&RecordedRequest) -> MockResponse + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock HTTP listener");
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let reqs_bg = Arc::clone(&requests);
        let stopped_bg = Arc::clone(&stopped);
        let handler: Handler = Box::new(handler);

        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped_bg.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { break };
                let Some(req) = read_http_request(&mut stream) else {
                    continue;
                };
                let resp = handler(&req);
                reqs_bg.lock().unwrap().push(req);
                write_http_response(&mut stream, &resp);
            }
        });

        Self {
            addr,
            requests,
            stopped,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn client(&self) -> Client {
        Client::new(
            self.base_url(),
            "me@example.com",
            "secret-token",
            Some(5_000),
        )
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn read_http_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = std::str::from_utf8(&buf[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let req_line = lines.next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }

    let body_start = header_end + 4;
    while buf.len() - body_start < content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let end = (body_start + content_length).min(buf.len());
    let body = String::from_utf8_lossy(&buf[body_start..end]).into_owned();

    Some(RecordedRequest {
        method,
        target,
        path,
        query,
        headers,
        body,
    })
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn write_http_response(stream: &mut TcpStream, resp: &MockResponse) {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        resp.status,
        status_reason(resp.status)
    );
    if !(resp.status == 204 && resp.body.is_empty()) {
        head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    }
    let has_ct = resp
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    if !has_ct && let Some(ct) = &resp.content_type {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    if !resp.body.is_empty() {
        let _ = stream.write_all(&resp.body);
    }
    let _ = stream.flush();
}

// --- Client tests ------------------------------------------------------------

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
    assert!(bb_err.contains("permission or scope"), "{bb_err}");

    let unauth_err = client.get("/auth", &[]).unwrap_err().to_string();
    assert!(unauth_err.contains("HTTP 401"));
    assert!(unauth_err.contains("atlx whoami"), "{unauth_err}");

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

// --- Jira tests --------------------------------------------------------------

#[test]
fn jira_get_with_comments_remote_links_and_full() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "key": "PROJ-1",
                "fields": {
                    "summary": "Fix bug",
                    "status": {"name": "In Progress"},
                    "issuetype": {"name": "Bug"},
                    "priority": {"name": "High"},
                    "assignee": {"displayName": "Alice"},
                    "reporter": {"displayName": "Bob"},
                    "created": "2026-01-01T00:00:00Z",
                    "updated": "2026-01-02T00:00:00Z",
                    "duedate": "2026-02-01",
                    "labels": ["backend"],
                    "components": [{"name": "api"}],
                    "fixVersions": [{"name": "v1.0"}],
                    "parent": {"key": "PROJ-0"},
                    "description": {
                        "type": "doc",
                        "version": 1,
                        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Details here"}]}]
                    },
                    "customfield_100": "extra-val"
                }
            }),
        ),
        // Page 1 of comments (2 of 3)
        MockResponse::json(
            200,
            json!({
                "total": 3,
                "comments": [
                    {"id": "10", "author": {"displayName": "Alice"}, "created": "2026-01-02", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "First"}]}]}},
                    {"id": "11", "author": {"displayName": "Bob"}, "created": "2026-01-03", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Second"}]}]}}
                ]
            }),
        ),
        // Page 2 of comments (1 of 3)
        MockResponse::json(
            200,
            json!({
                "total": 3,
                "comments": [
                    {"id": "12", "author": {"displayName": "Carol"}, "created": "2026-01-04", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Third"}]}]}}
                ]
            }),
        ),
        // Remote links
        MockResponse::json(
            200,
            json!([
                {"id": 99, "relationship": "links to", "object": {"title": "PR #5", "url": "https://bitbucket.org/acme/api/pull-requests/5"}}
            ]),
        ),
    ]);
    let client = srv.client();

    let out = jira::get(
        &client,
        "PROJ-1",
        &JiraGetOpts {
            fields: vec!["customfield_100".into()],
            comments: true,
            remote_links: true,
            full: true,
        },
    )
    .unwrap();

    assert_eq!(out["key"], "PROJ-1");
    assert_eq!(out["summary"], "Fix bug");
    assert_eq!(out["description"], "Details here");
    assert_eq!(out["components"], json!(["api"]));
    assert_eq!(out["fix_versions"], json!(["v1.0"]));
    assert_eq!(out["parent"], "PROJ-0");
    assert_eq!(out["customfield_100"], "extra-val");
    assert!(out["fields"].is_object());
    assert_eq!(out["url"], format!("{}/browse/PROJ-1", srv.base_url()));
    assert_eq!(out["comments"].as_array().unwrap().len(), 3);
    assert_eq!(out["comments"][2]["body"], "Third");
    assert_eq!(out["remote_links"][0]["title"], "PR #5");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].path, "/rest/api/3/issue/PROJ-1");
    assert_eq!(reqs[0].query_param("fields").as_deref(), Some("*all"));
    assert_eq!(reqs[1].path, "/rest/api/3/issue/PROJ-1/comment");
    assert_eq!(reqs[1].query_param("startAt").as_deref(), Some("0"));
    assert_eq!(reqs[2].query_param("startAt").as_deref(), Some("2"));
    assert_eq!(reqs[3].path, "/rest/api/3/issue/PROJ-1/remotelink");
}

#[test]
fn jira_query_paginates_next_page_token_and_respects_limit_and_after() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "issues": [
                    {"key": "P-1", "fields": {"summary": "One", "status": {"name": "Open"}, "customfield_1": "x1"}},
                    {"key": "P-2", "fields": {"summary": "Two", "status": {"name": "Open"}, "customfield_1": "x2"}}
                ],
                "nextPageToken": "tok-1"
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "issues": [
                    {"key": "P-3", "fields": {"summary": "Three", "status": {"name": "Done"}, "customfield_1": "x3"}},
                    {"key": "P-4", "fields": {"summary": "Four", "status": {"name": "Done"}, "customfield_1": "x4"}}
                ],
                "nextPageToken": "tok-2"
            }),
        ),
    ]);
    let client = srv.client();

    let out = jira::query(
        &client,
        "project = P",
        3,
        &["customfield_1".into()],
        Some("tok-0"),
    )
    .unwrap();
    let issues = out["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0]["key"], "P-1");
    assert_eq!(issues[2]["key"], "P-3");
    assert_eq!(issues[2]["customfield_1"], "x3");
    assert_eq!(out["next_cursor"], "tok-2");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/rest/api/3/search/jql");
    assert_eq!(reqs[0].query_param("jql").as_deref(), Some("project = P"));
    assert_eq!(reqs[0].query_param("maxResults").as_deref(), Some("3"));
    assert_eq!(
        reqs[0].query_param("nextPageToken").as_deref(),
        Some("tok-0")
    );
    assert!(
        reqs[0]
            .query_param("fields")
            .unwrap()
            .contains("customfield_1")
    );
    assert_eq!(reqs[1].query_param("maxResults").as_deref(), Some("1"));
    assert_eq!(
        reqs[1].query_param("nextPageToken").as_deref(),
        Some("tok-1")
    );
}

#[test]
fn jira_create_and_update_resolve_accounts_and_fields() {
    let srv = MockServer::with_responses([
        // create: assignee="me" -> /myself, reporter=ARI (no call), POST /issue
        MockResponse::json(200, json!({"accountId": "acc-me"})),
        MockResponse::json(201, json!({"id": "10042", "key": "PROJ-42"})),
        // update: assignee="alice" -> /user/search (1 atlassian + 1 app), PUT /issue
        MockResponse::json(
            200,
            json!([
                {"accountType": "app", "accountId": "bot-1", "displayName": "Alice Bot"},
                {"accountType": "atlassian", "accountId": "acc-alice", "displayName": "Alice"}
            ]),
        ),
        MockResponse::empty(204),
        // update ambiguous user -> /user/search returns 2 atlassian users -> errors before PUT
        MockResponse::json(
            200,
            json!([
                {"accountType": "atlassian", "accountId": "acc-a1", "displayName": "Alex A"},
                {"accountType": "atlassian", "accountId": "acc-a2", "displayName": "Alex B"}
            ]),
        ),
    ]);
    let client = srv.client();

    let created = jira::create(
        &client,
        NewWorkitem {
            space: "PROJ",
            issue_type: "Bug",
            summary: "New bug",
            description: Some(("Hello **world**", TextFormat::Markdown)),
            assignee: Some("me"),
            reporter: Some("ari:cloud:identity::user/557058:rep-1"),
            priority: Some("High"),
            labels: vec!["triage".into()],
            parent: Some("PROJ-1"),
            extra: Map::new(),
        },
    )
    .unwrap();
    assert_eq!(created["key"], "PROJ-42");
    assert_eq!(created["id"], "10042");

    let updated = jira::update(
        &client,
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
    assert_eq!(updated["key"], "PROJ-42");
    assert_eq!(updated["updated"], json!(["summary", "assignee", "labels"]));

    let ambig_err = jira::update(
        &client,
        "PROJ-42",
        Update {
            assignee: Some("alex"),
            ..Default::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(ambig_err.contains("matches several users"), "{ambig_err}");
    assert!(
        ambig_err.contains("Alex A") && ambig_err.contains("Alex B"),
        "{ambig_err}"
    );

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 5);
    assert_eq!(reqs[0].path, "/rest/api/3/myself");
    let create_body = reqs[1].json_body();
    assert_eq!(create_body["fields"]["project"], json!({"key": "PROJ"}));
    assert_eq!(
        create_body["fields"]["assignee"],
        json!({"accountId": "acc-me"})
    );
    assert_eq!(
        create_body["fields"]["reporter"],
        json!({"accountId": "557058:rep-1"})
    );
    assert_eq!(create_body["fields"]["parent"], json!({"key": "PROJ-1"}));

    assert_eq!(reqs[2].path, "/rest/api/3/user/search");
    assert_eq!(reqs[2].query_param("query").as_deref(), Some("alice"));
    let update_body = reqs[3].json_body();
    assert_eq!(
        update_body["fields"]["assignee"],
        json!({"accountId": "acc-alice"})
    );
    assert_eq!(
        update_body["update"]["labels"],
        json!([{"add": "urgent"}, {"remove": "triage"}])
    );
}

#[test]
fn jira_transition_handles_has_screen_true_and_false() {
    let transitions_payload = json!({
        "transitions": [
            {
                "id": "21",
                "name": "Resolve",
                "to": {"name": "Resolved"},
                "hasScreen": true,
                "fields": {
                    "resolution": {"required": true, "name": "Resolution", "allowedValues": [{"name": "Done"}]}
                }
            },
            {
                "id": "31",
                "name": "Close",
                "to": {"name": "Closed"},
                "hasScreen": false,
                "fields": {}
            }
        ]
    });

    let srv = MockServer::with_responses([
        // Case 1: hasScreen = true => comment goes inside transition POST
        MockResponse::json(200, transitions_payload.clone()),
        MockResponse::empty(204),
        // Case 2: hasScreen = false => transition POST has no comment, followed by POST /comment
        MockResponse::json(200, transitions_payload),
        MockResponse::empty(204),
        MockResponse::json(201, json!({"id": "c-77"})),
    ]);
    let client = srv.client();

    let mut fields = Map::new();
    fields.insert("resolution".into(), json!({"name": "Done"}));
    let res1 = jira::transition(
        &client,
        "PROJ-1",
        Some("Resolve"),
        fields,
        Some(("Fixed it", TextFormat::Plain)),
    )
    .unwrap();
    assert_eq!(res1["status"], "Resolved");
    assert!(res1.get("comment_id").is_none());

    let res2 = jira::transition(
        &client,
        "PROJ-1",
        Some("Closed"),
        Map::new(),
        Some(("Closing note", TextFormat::Plain)),
    )
    .unwrap();
    assert_eq!(res2["status"], "Closed");
    assert_eq!(res2["comment_id"], "c-77");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 5);

    let post_screen = reqs[1].json_body();
    assert_eq!(post_screen["transition"]["id"], "21");
    assert_eq!(post_screen["fields"]["resolution"]["name"], "Done");
    assert!(post_screen["update"]["comment"][0]["add"]["body"].is_object());

    let post_noscreen = reqs[3].json_body();
    assert_eq!(post_noscreen["transition"]["id"], "31");
    assert!(post_noscreen.get("update").is_none());

    assert_eq!(reqs[4].path, "/rest/api/3/issue/PROJ-1/comment");
}

// --- Confluence tests --------------------------------------------------------

#[test]
fn confluence_get_falls_back_from_page_404_to_blogpost() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            404,
            json!({"errors": [{"status": 404, "title": "Page not found"}]}),
        ),
        MockResponse::json(
            200,
            json!({
                "id": "55555",
                "title": "Launch Post",
                "status": "current",
                "spaceId": "101",
                "parentId": null,
                "version": {"number": 3, "createdAt": "2026-01-10T00:00:00Z"},
                "body": {"storage": {"value": "<h2>Release</h2><p>Shipped!</p>"}},
                "_links": {"webui": "/spaces/ENG/blog/55555"}
            }),
        ),
    ]);
    let client = srv.client();

    let out = confluence::get(
        &client,
        "55555",
        &ConfGetOpts {
            detail: Detail::Full,
            format: ReadFormat::Md,
            draft: false,
        },
    )
    .unwrap();

    assert_eq!(out["id"], "55555");
    assert_eq!(out["type"], "blogpost");
    assert_eq!(out["snapshot_token"], "3");
    assert_eq!(out["format"], "md");
    assert!(out["body"].as_str().unwrap().contains("## Release"));
    assert!(out["body"].as_str().unwrap().contains("Shipped!"));
    assert_eq!(
        out["url"],
        format!("{}/spaces/ENG/blog/55555", srv.base_url())
    );

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/api/v2/pages/55555");
    assert_eq!(reqs[1].path, "/api/v2/blogposts/55555");
}

#[test]
fn confluence_update_snapshot_token_validation_and_dry_run() {
    let current_page = json!({
        "id": "12345",
        "title": "Runbook",
        "status": "current",
        "spaceId": "10",
        "version": {"number": 4, "createdAt": "2026-01-10T00:00:00Z"},
        "body": {"storage": {"value": "<p>Old</p>"}}
    });

    let srv = MockServer::with_responses([
        // 1) Stale snapshot token
        MockResponse::json(200, current_page.clone()),
        // 2) Matching token + dry_run
        MockResponse::json(200, current_page.clone()),
        // 3) Matching token + real update
        MockResponse::json(200, current_page),
        MockResponse::json(
            200,
            json!({
                "id": "12345",
                "title": "Runbook",
                "version": {"number": 5},
                "_links": {"webui": "/spaces/ENG/pages/12345"}
            }),
        ),
    ]);
    let client = srv.client();

    let stale_err = confluence::update(
        &client,
        "12345",
        Edit {
            title: None,
            body: Some(("New", TextFormat::Plain)),
            version_message: None,
            minor_edit: false,
            snapshot_token: Some("3"),
            dry_run: false,
            draft: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(stale_err.contains("token 3, current 4"), "{stale_err}");

    let dry = confluence::update(
        &client,
        "12345",
        Edit {
            title: Some("Runbook v2"),
            body: Some(("# Heading", TextFormat::Markdown)),
            version_message: Some("bump"),
            minor_edit: true,
            snapshot_token: Some("4"),
            dry_run: true,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["from_version"], 4);
    assert_eq!(dry["request"]["version"]["number"], 5);
    assert_eq!(dry["request"]["version"]["minorEdit"], true);
    assert_eq!(dry["request"]["body"]["value"], "<h1>Heading</h1>");

    let real = confluence::update(
        &client,
        "12345",
        Edit {
            title: None,
            body: Some(("Updated", TextFormat::Plain)),
            version_message: Some("edit"),
            minor_edit: false,
            snapshot_token: Some("4"),
            dry_run: false,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(real["version"], 5);
    assert_eq!(real["snapshot_token"], "5");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[3].method, "PUT");
    assert_eq!(reqs[3].path, "/api/v2/pages/12345");
    assert_eq!(reqs[3].json_body()["version"]["number"], 5);
}

#[test]
fn confluence_search_query_and_list_extract_next_cursor() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "results": [{
                    "title": "@@@hl@@@Launch@@@endhl@@@ Plan",
                    "excerpt": "  Our @@@hl@@@launch@@@endhl@@@ checklist  ",
                    "lastModified": "2026-01-05",
                    "url": "/spaces/ENG/pages/10",
                    "content": {"id": "10", "type": "page"},
                    "resultGlobalContainer": {"title": "Engineering"}
                }],
                "_links": {"next": "/wiki/rest/api/search?cql=type%3Dpage&cursor=cur%2F1%3D%3D"}
            }),
        ),
        // list: resolve space key ENG -> id 999, then list pages
        MockResponse::json(200, json!({"results": [{"id": "999", "key": "ENG"}]})),
        MockResponse::json(
            200,
            json!({
                "results": [{
                    "id": "10",
                    "title": "Runbook",
                    "status": "current",
                    "parentId": "1",
                    "version": {"number": 2, "createdAt": "2026-01-05"},
                    "_links": {"webui": "/spaces/ENG/pages/10"}
                }],
                "_links": {"next": "/wiki/api/v2/spaces/999/pages?limit=5&cursor=page%3D2"}
            }),
        ),
        // create: numeric space id avoids /spaces lookup
        MockResponse::json(
            200,
            json!({"id": "77", "title": "Created", "status": "current", "version": {"number": 1}, "_links": {"webui": "/pages/77"}}),
        ),
    ]);
    let client = srv.client();

    let searched = confluence::search_query(&client, "type=page", 10, true, Some("prev")).unwrap();
    assert_eq!(searched["next_cursor"], "cur/1==");
    assert_eq!(searched["results"][0]["title"], "Launch Plan");
    assert_eq!(searched["results"][0]["excerpt"], "Our launch checklist");

    let listed =
        confluence::list(&client, "ENG", ContentType::Page, "current", 5, Some("c0")).unwrap();
    assert_eq!(listed["next_cursor"], "page=2");
    assert_eq!(listed["results"][0]["id"], "10");

    let created = confluence::create(
        &client,
        NewContent {
            space: "999",
            content_type: ContentType::Page,
            title: "Created",
            body: "Hi",
            format: TextFormat::Plain,
            parent: None,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(created["id"], "77");
    assert_eq!(created["snapshot_token"], "1");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].query_param("cursor").as_deref(), Some("prev"));
    assert_eq!(reqs[1].path, "/api/v2/spaces");
    assert_eq!(reqs[1].query_param("keys").as_deref(), Some("ENG"));
    assert_eq!(reqs[2].path, "/api/v2/spaces/999/pages");
    assert_eq!(reqs[2].query_param("cursor").as_deref(), Some("c0"));
}

// --- Bitbucket tests ---------------------------------------------------------

#[test]
fn bitbucket_repo_query_and_pr_query_follow_next_urls_up_to_limit() {
    let srv = MockServer::with_handler(|req| {
        match (req.path.as_str(), req.query_param("page").as_deref()) {
            ("/repositories/acme", None) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"full_name": "acme/r1", "name": "r1", "is_private": true},
                        {"full_name": "acme/r2", "name": "r2", "is_private": true}
                    ],
                    "next": format!("http://{}/repositories/acme?page=2", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme", Some("2")) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"full_name": "acme/r3", "name": "r3", "is_private": false},
                        {"full_name": "acme/r4", "name": "r4", "is_private": false}
                    ],
                    "next": format!("http://{}/repositories/acme?page=3", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme/api/pullrequests", None) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"id": 10, "title": "PR 10", "state": "OPEN"},
                        {"id": 11, "title": "PR 11", "state": "OPEN"}
                    ],
                    "next": format!("http://{}/repositories/acme/api/pullrequests?page=2", req.header("host").unwrap())
                }),
            ),
            ("/repositories/acme/api/pullrequests", Some("2")) => MockResponse::json(
                200,
                json!({
                    "values": [
                        {"id": 12, "title": "PR 12", "state": "OPEN"},
                        {"id": 13, "title": "PR 13", "state": "OPEN"}
                    ]
                }),
            ),
            _ => MockResponse::text(404, "unexpected route"),
        }
    });
    let client = srv.client();

    let repos = bitbucket::repo_query(
        &client,
        "acme",
        &RepoQuery {
            query: Some("api"),
            project: Some("CORE"),
            language: Some("Rust"),
            sort: "-updated_on",
            role: Some("member"),
            limit: 3,
        },
    )
    .unwrap();
    let repo_arr = repos.as_array().unwrap();
    assert_eq!(repo_arr.len(), 3);
    assert_eq!(repo_arr[0]["repo"], "acme/r1");
    assert_eq!(repo_arr[2]["repo"], "acme/r3");

    let prs = bitbucket::pr_query(
        &client,
        "/repositories/acme/api",
        &PrQuery {
            state: Some("open"),
            dest: Some("main"),
            limit: 3,
            ..Default::default()
        },
    )
    .unwrap();
    let pr_arr = prs.as_array().unwrap();
    assert_eq!(pr_arr.len(), 3);
    assert_eq!(pr_arr[0]["id"], 10);
    assert_eq!(pr_arr[2]["id"], 12);

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(reqs[0].query_param("role").as_deref(), Some("member"));
    assert_eq!(reqs[0].query_param("pagelen").as_deref(), Some("3"));
    assert_eq!(reqs[2].query_param("state").as_deref(), Some("OPEN"));
    assert!(
        reqs[2]
            .query_param("q")
            .unwrap()
            .contains("destination.branch.name = \"main\"")
    );
}

#[test]
fn bitbucket_pr_get_with_comments_diff_and_statuses() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "id": 42,
                "title": "Add tests",
                "state": "OPEN",
                "draft": false,
                "author": {"display_name": "Jane"},
                "source": {"branch": {"name": "feat/tests"}},
                "destination": {"branch": {"name": "main"}},
                "updated_on": "2026-01-05",
                "links": {"html": {"href": "https://bitbucket.org/acme/api/pull-requests/42"}},
                "reviewers": [{"display_name": "Bob"}],
                "participants": [
                    {"user": {"display_name": "Bob"}, "approved": true},
                    {"user": {"display_name": "Carl"}, "approved": false}
                ],
                "comment_count": 2,
                "task_count": 0,
                "description": "Adds integration tests"
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"id": 1, "deleted": true, "user": {"display_name": "Bob"}, "content": {"raw": "deleted"}},
                    {"id": 2, "deleted": false, "user": {"display_name": "Bob"}, "created_on": "2026-01-05", "inline": {"path": "src/lib.rs", "to": 12}, "content": {"raw": "LGTM"}}
                ]
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"status": "modified", "lines_added": 20, "lines_removed": 3, "new": {"path": "src/lib.rs"}}
                ]
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"name": "CI", "key": "ci/build", "state": "SUCCESSFUL", "url": "https://ci.example/1"}
                ]
            }),
        ),
        // Also verify pr_create & pr_comment endpoints
        MockResponse::json(
            201,
            json!({"id": 43, "title": "Follow-up", "state": "OPEN"}),
        ),
        MockResponse::json(
            201,
            json!({"id": 99, "links": {"html": {"href": "https://bb/comment/99"}}}),
        ),
    ]);
    let client = srv.client();

    let out = bitbucket::pr_get(
        &client,
        "/repositories/acme/api",
        42,
        &PrGet {
            comments: true,
            diff: true,
            statuses: true,
        },
    )
    .unwrap();

    assert_eq!(out["id"], 42);
    assert_eq!(out["reviewers"], json!(["Bob"]));
    assert_eq!(out["approved_by"], json!(["Bob"]));
    assert_eq!(out["comments"].as_array().unwrap().len(), 1);
    assert_eq!(out["comments"][0]["path"], "src/lib.rs");
    assert_eq!(out["comments"][0]["line"], 12);
    assert_eq!(out["comments"][0]["body"], "LGTM");
    assert_eq!(out["diffstat"][0]["added"], 20);
    assert_eq!(out["statuses"][0]["state"], "SUCCESSFUL");

    let created = bitbucket::pr_create(
        &client,
        "/repositories/acme/api",
        NewPr {
            title: "Follow-up",
            source: "feat/x",
            dest: Some("main"),
            description: Some("desc"),
            reviewer_uuids: vec!["{u1}".into()],
            reviewer_account_ids: vec![],
            close_source_branch: true,
            draft: false,
        },
    )
    .unwrap();
    assert_eq!(created["id"], 43);

    let comment = bitbucket::pr_comment(
        &client,
        "/repositories/acme/api",
        43,
        NewComment {
            text: "Nice",
            reply_to: None,
            path: Some("README.md"),
            line: Some(5),
        },
    )
    .unwrap();
    assert_eq!(comment["id"], 99);
}

#[test]
fn bitbucket_pipeline_get_with_logs_failed_steps_and_lines() {
    let srv = MockServer::with_responses([
        MockResponse::json(
            200,
            json!({
                "build_number": 1234,
                "uuid": "{pipe-uuid}",
                "state": {"name": "COMPLETED", "result": {"name": "FAILED"}},
                "target": {"ref_name": "main", "commit": {"hash": "0123456789abcdef"}},
                "trigger": {"name": "PUSH"},
                "created_on": "2026-01-05",
                "duration_in_seconds": 45
            }),
        ),
        MockResponse::json(
            200,
            json!({
                "values": [
                    {"name": "Lint", "uuid": "{step-1}", "state": {"name": "COMPLETED", "result": {"name": "PASSED"}}, "duration_in_seconds": 10},
                    {"name": "Test", "uuid": "{step-2}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "duration_in_seconds": 35}
                ]
            }),
        ),
        MockResponse::text(200, "line1\nline2\nline3\nline4"),
    ]);
    let client = srv.client();

    let out = bitbucket::pipeline_get(
        &client,
        "/repositories/acme/api",
        "1234",
        &PipelineGet {
            logs: true,
            failed_steps: true,
            lines: 2,
            step: None,
        },
    )
    .unwrap();

    assert_eq!(out["build"], 1234);
    assert_eq!(out["commit"], "0123456789ab");
    let steps = out["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert!(
        steps[0].get("log").is_none(),
        "passed step log should not be fetched"
    );
    assert_eq!(steps[1]["log"], "line3\nline4");

    let reqs = srv.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        reqs[2].path,
        "/repositories/acme/api/pipelines/%7Bpipe-uuid%7D/steps/%7Bstep-2%7D/log"
    );
}

#[test]
fn bitbucket_pipeline_query_filters_client_side() {
    let srv = MockServer::with_responses([MockResponse::json(
        200,
        json!({
            "values": [
                {"build_number": 1, "uuid": "{p1}", "state": {"name": "COMPLETED", "result": {"name": "PASSED"}}, "trigger": {"name": "PUSH"}},
                {"build_number": 2, "uuid": "{p2}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "trigger": {"name": "MANUAL_TRIGGER"}},
                {"build_number": 3, "uuid": "{p3}", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}, "trigger": {"name": "PUSH"}}
            ]
        }),
    )]);
    let client = srv.client();

    let out = bitbucket::pipeline_query(
        &client,
        "/repositories/acme/api",
        &PipelineQuery {
            state: Some("COMPLETED"),
            status: Some("FAILED"),
            branch: Some("main"),
            trigger: Some("MANUAL"),
            limit: 10,
        },
    )
    .unwrap();
    let arr = out.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["build"], 2);
    assert_eq!(
        srv.requests()[0].query_param("target.branch").as_deref(),
        Some("main")
    );
}

// --- Cross-product search tests ----------------------------------------------

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
    assert_eq!(ok["bitbucket"][0]["repo"], "acme/deploy");

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
    assert_eq!(partial["bitbucket"][0]["repo"], "acme/infra");
    let conf_err = partial["confluence"]["error"].as_str().unwrap();
    assert!(conf_err.contains("HTTP 500"), "{conf_err}");
    assert!(conf_err.contains("Confluence unavailable"), "{conf_err}");

    // 3. Direct search::run with unconfigured Config isolates credential error & skips unconfigured Bitbucket
    let empty_cfg = Config::default();
    let cfg_err = search::run(&empty_cfg, "deploy", 5, &[]);
    assert!(
        cfg_err["jira"]["error"]
            .as_str()
            .unwrap()
            .contains("no email configured")
    );
    assert!(
        cfg_err["confluence"]["error"]
            .as_str()
            .unwrap()
            .contains("no email configured")
    );
    assert!(cfg_err.get("bitbucket").is_none());

    // Explicit `--only bitbucket` includes bitbucket error even when unconfigured
    let only_bb = search::run(&empty_cfg, "deploy", 5, &[Product::Bitbucket]);
    assert!(only_bb.get("jira").is_none());
    assert!(
        only_bb["bitbucket"]["error"]
            .as_str()
            .unwrap()
            .contains("no Bitbucket email configured")
    );
}

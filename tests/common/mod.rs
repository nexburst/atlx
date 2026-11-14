//! Shared mock HTTP server for the offline integration tests.
//!
//! Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use atlx::client::Client;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json_body(&self) -> Value {
        serde_json::from_str(&self.body).expect("request body should be valid JSON")
    }

    pub fn query_param(&self, key: &str) -> Option<String> {
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

pub fn decode_query_component(s: &str) -> String {
    let plus_as_space = s.replace('+', " ");
    urlencoding::decode(&plus_as_space)
        .map(|c| c.into_owned())
        .unwrap_or(plus_as_space)
}

#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

impl MockResponse {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: serde_json::to_vec(&body).unwrap(),
            content_type: Some("application/json".into()),
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
            content_type: Some("text/plain; charset=utf-8".into()),
        }
    }

    pub fn html(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into().into_bytes(),
            content_type: Some("text/html; charset=utf-8".into()),
        }
    }

    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            content_type: None,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub type Handler = Box<dyn Fn(&RecordedRequest) -> MockResponse + Send + Sync>;

pub struct MockServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stopped: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    pub fn with_responses(responses: impl IntoIterator<Item = MockResponse>) -> Self {
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

    pub fn with_handler(
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

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn client(&self) -> Client {
        Client::new(
            self.base_url(),
            "me@example.com",
            "secret-token",
            Some(5_000),
        )
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
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

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub fn read_http_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
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

pub fn status_reason(code: u16) -> &'static str {
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

pub fn write_http_response(stream: &mut TcpStream, resp: &MockResponse) {
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

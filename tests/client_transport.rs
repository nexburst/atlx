//! Offline wire-level tests for `atlx::client::Client` transport features:
//! extra headers, multipart uploads, downloads and redirects, retries and
//! large bodies. Uses its own tiny loopback HTTP server.

use atlx::client::{Client, Part};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn header_count(&self, name: &str) -> usize {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .count()
    }
}

enum Reply {
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Read the request, then close the connection without answering.
    Hangup,
}

fn reply(status: u16, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) -> Reply {
    Reply::Respond {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.into(),
    }
}

fn json_reply(status: u16, v: Value) -> Reply {
    reply(
        status,
        &[("Content-Type", "application/json")],
        serde_json::to_vec(&v).unwrap(),
    )
}

/// Serves scripted replies in order, one per connection, and records every
/// request. Unexpected extra requests get a 599. The thread is detached.
struct Server {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Server {
    fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let mut queue: VecDeque<Reply> = replies.into();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(req) = read_request(&mut stream) else {
                    continue;
                };
                recorded.lock().unwrap().push(req);
                match queue.pop_front() {
                    Some(Reply::Hangup) => drop(stream),
                    Some(Reply::Respond {
                        status,
                        headers,
                        body,
                    }) => write_response(&mut stream, status, &headers, &body),
                    None => write_response(&mut stream, 599, &[], b"unexpected request"),
                }
            }
        });
        Self { addr, requests }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut start = lines.next()?.split_whitespace();
    let method = start.next()?.to_string();
    let target = start.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}

fn write_response(stream: &mut TcpStream, status: u16, headers: &[(String, String)], body: &[u8]) {
    let mut out = format!("HTTP/1.1 {status} Status\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    // The client may legitimately hang up early (e.g. body over the cap).
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn client(server: &Server) -> Client {
    Client::new(server.url(), "me@example.com", "tok", Some(10_000))
        .with_backoff_base(Duration::from_millis(1))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atlx-client-transport-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn request_with_sends_extra_headers_and_overrides_accept() {
    let server = Server::start(vec![reply(
        206,
        &[("Content-Type", "text/plain")],
        "log line 2\n",
    )]);
    let v = client(&server)
        .request_with(
            "GET",
            "/log",
            &[("page", "1")],
            None,
            &[("Accept", "text/plain"), ("Range", "bytes=10-")],
        )
        .unwrap();
    assert_eq!(v, Value::String("log line 2\n".into()));
    let reqs = server.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].target, "/log?page=1");
    assert_eq!(reqs[0].header("Accept"), Some("text/plain"));
    assert_eq!(reqs[0].header_count("Accept"), 1);
    assert_eq!(reqs[0].header("Range"), Some("bytes=10-"));
    assert!(reqs[0].header("Authorization").is_some());
}

#[test]
fn multipart_upload_has_boundary_part_headers_and_bytes() {
    let dir = temp_dir("multipart");
    let file = dir.join("report.bin");
    let contents: Vec<u8> = (0u8..=255)
        .chain(b"\r\n--tail\r\n".iter().copied())
        .collect();
    std::fs::write(&file, &contents).unwrap();

    let server = Server::start(vec![json_reply(200, json!([{"id": "10001"}]))]);
    let parts = [
        Part::text("comment", "hello \"world\""),
        Part::File {
            field: "file".into(),
            filename: "report.bin".into(),
            content_type: Some("application/x-test".into()),
            path: file.clone(),
        },
    ];
    let v = client(&server)
        .post_multipart(
            "/rest/api/3/issue/ABC-1/attachments",
            &parts,
            &[("X-Atlassian-Token", "no-check")],
        )
        .unwrap();
    assert_eq!(v, json!([{"id": "10001"}]));

    let reqs = server.requests();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.header("X-Atlassian-Token"), Some("no-check"));
    assert!(req.header("Authorization").is_some());
    let ct = req.header("Content-Type").unwrap();
    let boundary = ct
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap_or_else(|| panic!("bad content type {ct}"));
    assert!(
        boundary.starts_with("atlx-") && boundary.len() > 20,
        "{boundary}"
    );

    let mut expected = Vec::new();
    expected.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"comment\"\r\n\r\nhello \"world\"\r\n"
        )
        .as_bytes(),
    );
    expected.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"report.bin\"\r\nContent-Type: application/x-test\r\n\r\n"
        )
        .as_bytes(),
    );
    expected.extend_from_slice(&contents);
    expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    assert_eq!(
        req.header("Content-Length"),
        Some(expected.len().to_string().as_str())
    );
    assert_eq!(req.body, expected);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn multipart_missing_file_is_a_clear_error() {
    let server = Server::start(vec![]);
    let err = client(&server)
        .post_multipart("/up", &[Part::file("file", "/no/such/atlx-file")], &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot open"), "{err}");
    assert!(server.requests().is_empty());
}

#[test]
fn download_follows_cross_origin_redirect_without_authorization() {
    let media = Server::start(vec![reply(
        200,
        &[("Content-Type", "application/octet-stream")],
        vec![0u8, 1, 2, 255, b'\n'],
    )]);
    let site = Server::start(vec![reply(
        302,
        &[(
            "Location",
            &format!("{}/file/abc?token=signed", media.url()),
        )],
        "",
    )]);
    let dir = temp_dir("download");
    let dest = dir.join("out.bin");
    let n = client(&site)
        .download_to("/rest/api/3/attachment/content/1", &[], &dest, false)
        .unwrap();
    assert_eq!(n, 5);
    assert_eq!(std::fs::read(&dest).unwrap(), vec![0u8, 1, 2, 255, b'\n']);

    let first = site.requests();
    assert_eq!(first.len(), 1);
    assert!(first[0].header("Authorization").is_some());
    let second = media.requests();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].target, "/file/abc?token=signed");
    assert!(
        second[0].header("Authorization").is_none(),
        "Authorization leaked to another origin: {:?}",
        second[0].headers
    );

    // Refuses to overwrite unless asked, and leaves no temp files behind.
    let err = client(&site)
        .download_to("/again", &[], &dest, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("refusing to overwrite"), "{err}");
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_overwrites_when_asked() {
    let server = Server::start(vec![reply(200, &[], "new")]);
    let dir = temp_dir("overwrite");
    let dest = dir.join("f.txt");
    std::fs::write(&dest, "old contents").unwrap();
    let n = client(&server).download_to("/f", &[], &dest, true).unwrap();
    assert_eq!(n, 3);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn same_origin_redirect_keeps_authorization() {
    let server = Server::start(vec![
        reply(302, &[("Location", "/final")], ""),
        json_reply(200, json!({"ok": true})),
    ]);
    let v = client(&server).get("/start", &[]).unwrap();
    assert_eq!(v, json!({"ok": true}));
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].target, "/final");
    assert!(reqs[1].header("Authorization").is_some());
}

#[test]
fn get_bytes_returns_binary_and_content_type() {
    let server = Server::start(vec![reply(
        200,
        &[("Content-Type", "image/png")],
        vec![0x89, b'P', b'N', b'G', 0, 0xff],
    )]);
    let b = client(&server).get_bytes("/img", &[]).unwrap();
    assert_eq!(b.data, vec![0x89, b'P', b'N', b'G', 0, 0xff]);
    assert_eq!(b.content_type.as_deref(), Some("image/png"));
    assert_eq!(server.requests()[0].header("Accept"), Some("*/*"));
}

#[test]
fn get_retries_503_then_succeeds() {
    let server = Server::start(vec![
        json_reply(503, json!({"message": "unavailable"})),
        json_reply(200, json!({"ok": 1})),
    ]);
    let v = client(&server).get("/flaky", &[]).unwrap();
    assert_eq!(v, json!({"ok": 1}));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn post_503_is_not_retried() {
    let server = Server::start(vec![
        json_reply(503, json!({"message": "unavailable"})),
        json_reply(200, json!({"ok": 1})),
    ]);
    let err = client(&server)
        .post("/create", &json!({"a": 1}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("HTTP 503"), "{err}");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn post_429_is_retried() {
    let server = Server::start(vec![
        reply(429, &[("Retry-After", "0")], ""),
        json_reply(201, json!({"id": 1})),
    ]);
    let v = client(&server).post("/create", &json!({})).unwrap();
    assert_eq!(v, json!({"id": 1}));
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].body, b"{}");
}

#[test]
fn http_date_retry_after_is_honored() {
    // A date in the past means "retry now"; without parsing it the client
    // would fall back to backoff, which is also fast here, so additionally
    // check a near-future date makes the client wait.
    let server = Server::start(vec![
        reply(503, &[("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")], ""),
        json_reply(200, json!({"ok": 1})),
    ]);
    assert_eq!(client(&server).get("/x", &[]).unwrap(), json!({"ok": 1}));

    let in_two_secs = httpdate(std::time::SystemTime::now() + Duration::from_secs(2));
    let server = Server::start(vec![
        reply(503, &[("Retry-After", &in_two_secs)], ""),
        json_reply(200, json!({"ok": 2})),
    ]);
    let started = Instant::now();
    assert_eq!(client(&server).get("/x", &[]).unwrap(), json!({"ok": 2}));
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "Retry-After date ignored: {:?}",
        started.elapsed()
    );
}

fn httpdate(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let weekdays = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    format!(
        "{}, {d:02} {} {y} {:02}:{:02}:{:02} GMT",
        weekdays[days.rem_euclid(7) as usize],
        months[(m - 1) as usize],
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[test]
fn get_retries_transport_errors_but_post_does_not() {
    let server = Server::start(vec![Reply::Hangup, json_reply(200, json!({"ok": 1}))]);
    assert_eq!(client(&server).get("/x", &[]).unwrap(), json!({"ok": 1}));
    assert_eq!(server.requests().len(), 2);

    let server = Server::start(vec![Reply::Hangup, json_reply(200, json!({"ok": 1}))]);
    assert!(client(&server).post("/x", &json!({})).is_err());
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn gives_up_after_three_retries() {
    let server = Server::start(vec![
        reply(502, &[("Retry-After", "0")], ""),
        reply(502, &[("Retry-After", "0")], ""),
        reply(502, &[("Retry-After", "0")], ""),
        reply(502, &[("Retry-After", "0")], ""),
        json_reply(200, json!({})),
    ]);
    let err = client(&server).get("/x", &[]).unwrap_err().to_string();
    assert!(err.contains("HTTP 502"), "{err}");
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn text_body_over_10_mb_is_read() {
    let body = "a".repeat(11 * 1024 * 1024);
    let server = Server::start(vec![reply(
        200,
        &[("Content-Type", "text/plain")],
        body.clone(),
    )]);
    let v = client(&server)
        .request_with("GET", "/diff", &[], None, &[("Accept", "text/plain")])
        .unwrap();
    assert_eq!(v.as_str().map(str::len), Some(body.len()));
}

#[test]
fn body_over_the_cap_is_a_clear_error() {
    let server = Server::start(vec![reply(200, &[], vec![b'x'; 2 * 1024 * 1024])]);
    let err = client(&server)
        .with_max_body(1024 * 1024)
        .get("/big", &[])
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("larger than the 1 MiB limit"),
        "unexpected error: {err}"
    );
}

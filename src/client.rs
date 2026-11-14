//! Thin HTTP client for the Atlassian REST APIs: basic auth, readable errors,
//! retries (429 always; 502/503/504 and transport errors for idempotent
//! methods), manual redirect handling that never sends `Authorization` to
//! another origin, capped text/JSON bodies, binary downloads and streamed
//! `multipart/form-data` uploads.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde_json::Value;
use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{Cursor, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Client {
    agent: ureq::Agent,
    base: String,
    auth: String,
    timeout: Duration,
    max_body: u64,
    backoff_base: Duration,
}

/// A raw (possibly binary) response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes {
    pub data: Vec<u8>,
    /// The raw `Content-Type` header, if the server sent one.
    pub content_type: Option<String>,
}

/// One part of a `multipart/form-data` request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// A file part whose contents are streamed from `path`.
    File {
        field: String,
        filename: String,
        /// `application/octet-stream` when `None`.
        content_type: Option<String>,
        path: PathBuf,
    },
    /// A plain form field.
    Text { field: String, value: String },
}

impl Part {
    /// A file part named after the file's own name.
    pub fn file(field: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        Part::File {
            field: field.into(),
            filename,
            content_type: None,
            path,
        }
    }

    pub fn text(field: impl Into<String>, value: impl Into<String>) -> Self {
        Part::Text {
            field: field.into(),
            value: value.into(),
        }
    }
}

const MAX_RETRIES: u32 = 3;
const MAX_WAIT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;
/// Default cap for text/JSON/byte responses read into memory.
pub const DEFAULT_MAX_BODY: u64 = 64 * 1024 * 1024;
const ERROR_BODY_CAP: u64 = 1024 * 1024;

enum Payload<'a> {
    Empty,
    Json(&'a Value),
    Multipart { parts: &'a [Part], boundary: String },
}

/// Per-request options that differ between the public entry points.
struct Opts<'a> {
    headers: &'a [(&'a str, &'a str)],
    default_accept: &'a str,
    /// Apply the overall request deadline (JSON calls). Downloads and uploads
    /// only use the per-phase connect/read/write timeouts.
    overall_timeout: bool,
}

impl Client {
    pub fn new(base: String, email: &str, token: &str, timeout_ms: Option<u64>) -> Self {
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(60_000));
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(timeout)
            .timeout_read(timeout)
            .timeout_write(timeout)
            .redirects(0)
            .user_agent(concat!("atlx/", env!("CARGO_PKG_VERSION")))
            .build();
        let creds = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
        Self {
            agent,
            base,
            auth: format!("Basic {creds}"),
            timeout,
            max_body: DEFAULT_MAX_BODY,
            backoff_base: Duration::from_secs(1),
        }
    }

    /// Cap in bytes for responses read into memory (default 64 MiB).
    pub fn with_max_body(mut self, bytes: u64) -> Self {
        self.max_body = bytes;
        self
    }

    /// Base delay for exponential backoff when no `Retry-After` is given
    /// (default 1s).
    pub fn with_backoff_base(mut self, base: Duration) -> Self {
        self.backoff_base = base;
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.request_with("GET", path, query, None, &[])
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.request_with("POST", path, &[], Some(body), &[])
    }

    pub fn put(&self, path: &str, body: &Value) -> Result<Value> {
        self.request_with("PUT", path, &[], Some(body), &[])
    }

    pub fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.request_with("DELETE", path, query, None, &[])
    }

    /// `path` is appended to the base URL unless it is already absolute
    /// (e.g. a Bitbucket `next` pagination link).
    pub fn request(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value> {
        self.request_with(method, path, query, body, &[])
    }

    /// Like [`Client::request`] with extra request headers (e.g.
    /// `X-Atlassian-Token`, `Accept: text/plain`, `Range`). A caller-supplied
    /// `Accept` replaces the default `application/json`. The body is parsed as
    /// JSON, falling back to a JSON string for other text; empty is `null`.
    pub fn request_with(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> Result<Value> {
        let opts = Opts {
            headers,
            default_accept: "application/json",
            overall_timeout: true,
        };
        let payload = body.map_or(Payload::Empty, Payload::Json);
        let (resp, desc) = self.execute(method, path, query, payload, &opts)?;
        Ok(parse_value(self.read_body(resp, &desc)?))
    }

    /// GET a raw (binary) body into memory, subject to the body cap.
    pub fn get_bytes(&self, path: &str, query: &[(&str, &str)]) -> Result<Bytes> {
        self.get_bytes_with(path, query, &[])
    }

    /// [`Client::get_bytes`] with extra request headers (e.g. `Range`).
    pub fn get_bytes_with(
        &self,
        path: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<Bytes> {
        let opts = Opts {
            headers,
            default_accept: "*/*",
            overall_timeout: false,
        };
        let (resp, desc) = self.execute("GET", path, query, Payload::Empty, &opts)?;
        let content_type = resp.header("Content-Type").map(String::from);
        let data = self.read_body(resp, &desc)?;
        Ok(Bytes { data, content_type })
    }

    /// Stream a GET response to `dest` without a size cap. Refuses to replace
    /// an existing file unless `overwrite`; writes a temporary file in the same
    /// directory and renames it into place. Returns the number of bytes written.
    pub fn download_to(
        &self,
        path: &str,
        query: &[(&str, &str)],
        dest: &Path,
        overwrite: bool,
    ) -> Result<u64> {
        if dest.is_dir() {
            bail!("{} is a directory", dest.display());
        }
        if !overwrite && dest.exists() {
            bail!("refusing to overwrite existing file {}", dest.display());
        }
        let opts = Opts {
            headers: &[],
            default_accept: "*/*",
            overall_timeout: false,
        };
        let (resp, desc) = self.execute("GET", path, query, Payload::Empty, &opts)?;
        let dir = match dest.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());
        let tmp = dir.join(format!(
            ".{name}.{}-{:x}.atlx-part",
            std::process::id(),
            random_u64()
        ));
        let result = (|| -> Result<u64> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .with_context(|| format!("cannot create {}", tmp.display()))?;
            let n = std::io::copy(&mut resp.into_reader(), &mut file)
                .with_context(|| format!("{desc}: reading response body failed"))?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            if !overwrite && dest.exists() {
                bail!("refusing to overwrite existing file {}", dest.display());
            }
            std::fs::rename(&tmp, dest)
                .with_context(|| format!("cannot move download into {}", dest.display()))?;
            Ok(n)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// POST a streamed `multipart/form-data` body. Jira attachment uploads need
    /// `X-Atlassian-Token: no-check`, Confluence v1 `X-Atlassian-Token: nocheck`.
    /// Not retried on 5xx (POST), but 429 is.
    pub fn post_multipart(
        &self,
        path: &str,
        parts: &[Part],
        headers: &[(&str, &str)],
    ) -> Result<Value> {
        let boundary = format!("atlx-{:016x}{:08x}", random_u64(), std::process::id());
        let opts = Opts {
            headers,
            default_accept: "application/json",
            overall_timeout: false,
        };
        let payload = Payload::Multipart { parts, boundary };
        let (resp, desc) = self.execute("POST", path, &[], payload, &opts)?;
        Ok(parse_value(self.read_body(resp, &desc)?))
    }

    fn read_body(&self, resp: ureq::Response, desc: &str) -> Result<Vec<u8>> {
        match read_capped(resp.into_reader(), self.max_body) {
            Ok(Some(data)) => Ok(data),
            Ok(None) => bail!(
                "{desc}: response body is larger than the {} limit; download it to a file instead",
                human_size(self.max_body)
            ),
            Err(e) => Err(anyhow!("{desc}: reading response body failed: {e}")),
        }
    }

    /// Send the request, following redirects manually and retrying per policy.
    /// Returns the final non-error response and a `METHOD url` description.
    fn execute(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        mut payload: Payload<'_>,
        opts: &Opts<'_>,
    ) -> Result<(ureq::Response, String)> {
        let (mut url, mut auth) = self.resolve_request(path)?;
        let mut method = method.to_ascii_uppercase();
        let mut query = query;
        for _ in 0..=MAX_REDIRECTS {
            let resp = self.send_with_retry(&method, &url, query, &payload, auth, opts)?;
            let status = resp.status();
            let location = match resp.header("Location") {
                Some(l) if (300..400).contains(&status) && status != 304 => l.to_string(),
                _ => return Ok((resp, format!("{method} {url}"))),
            };
            let next = join_location(resp.get_url(), &location);
            match status {
                301..=303 => {
                    if method != "HEAD" {
                        method = "GET".into();
                    }
                    payload = Payload::Empty;
                }
                307 | 308 => {
                    if matches!(payload, Payload::Multipart { .. }) {
                        bail!(
                            "{method} {url} was redirected ({status}) to {next}; not re-sending the upload"
                        );
                    }
                }
                _ => return Ok((resp, format!("{method} {url}"))),
            }
            let (next_url, next_auth) = self.resolve_request(&next)?;
            // Authorization only ever goes to the configured base origin, and
            // never again once a hop has left it.
            auth = auth && next_auth;
            url = next_url;
            query = &[];
        }
        bail!("{method} {url} failed: too many redirects (more than {MAX_REDIRECTS})")
    }

    fn send_with_retry(
        &self,
        method: &str,
        url: &str,
        query: &[(&str, &str)],
        payload: &Payload<'_>,
        auth: bool,
        opts: &Opts<'_>,
    ) -> Result<ureq::Response> {
        let idempotent = matches!(method, "GET" | "HEAD" | "PUT" | "DELETE" | "OPTIONS");
        let mut attempt = 0;
        loop {
            let mut req = self.agent.request(method, url);
            if opts.overall_timeout {
                req = req.timeout(self.timeout);
            }
            if !opts
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("accept"))
            {
                req = req.set("Accept", opts.default_accept);
            }
            if auth {
                req = req.set("Authorization", &self.auth);
            }
            for (k, v) in opts.headers {
                if !auth && k.eq_ignore_ascii_case("authorization") {
                    continue;
                }
                req = req.set(k, v);
            }
            for (k, v) in query {
                req = req.query(k, v);
            }
            let res = match payload {
                Payload::Empty => req.call(),
                Payload::Json(b) => req.send_json(*b),
                Payload::Multipart { parts, boundary } => {
                    let (len, body) = multipart_body(parts, boundary)?;
                    req.set(
                        "Content-Type",
                        &format!("multipart/form-data; boundary={boundary}"),
                    )
                    .set("Content-Length", &len.to_string())
                    .send(body)
                }
            };
            match res {
                Ok(r) => return Ok(r),
                Err(ureq::Error::Status(code, r)) => {
                    let retryable = code == 429 || (idempotent && matches!(code, 502..=504));
                    if retryable && attempt < MAX_RETRIES {
                        let wait = r
                            .header("Retry-After")
                            .and_then(|v| parse_retry_after(v, SystemTime::now()))
                            .unwrap_or_else(|| self.backoff(attempt));
                        attempt += 1;
                        std::thread::sleep(wait.min(MAX_WAIT));
                        continue;
                    }
                    let text = read_capped(r.into_reader(), ERROR_BODY_CAP)
                        .ok()
                        .flatten()
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    bail!(
                        "{method} {url} failed: HTTP {code}: {}{}",
                        error_message(&text),
                        hint(code)
                    );
                }
                Err(ureq::Error::Transport(t)) => {
                    let transient = matches!(
                        t.kind(),
                        ureq::ErrorKind::Io | ureq::ErrorKind::ConnectionFailed
                    );
                    if idempotent && transient && attempt < MAX_RETRIES {
                        let wait = self.backoff(attempt);
                        attempt += 1;
                        std::thread::sleep(wait.min(MAX_WAIT));
                        continue;
                    }
                    bail!("{method} {url} failed: {t}");
                }
            }
        }
    }

    /// Exponential backoff with "equal jitter": a random wait in
    /// `[full/2, full]` where `full = base * 2^attempt`, capped at 30s.
    fn backoff(&self, attempt: u32) -> Duration {
        let full = self
            .backoff_base
            .saturating_mul(2u32.saturating_pow(attempt))
            .min(MAX_WAIT);
        let half = full / 2;
        let span = (full - half).as_millis() as u64;
        let jitter = if span == 0 {
            0
        } else {
            random_u64() % (span + 1)
        };
        half + Duration::from_millis(jitter)
    }

    fn resolve_request(&self, path: &str) -> Result<(String, bool)> {
        let is_abs = path
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
            || path
                .get(..8)
                .is_some_and(|p| p.eq_ignore_ascii_case("https://"));
        let url = if is_abs {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        };
        let target = parse_origin(&url).ok_or_else(|| anyhow!("invalid request URL '{url}'"))?;
        if target.scheme == "http" && !is_loopback_host(&target.host) {
            bail!(
                "refusing plaintext HTTP request to non-loopback host '{}'",
                target.host
            );
        }
        let attach_auth = if !is_abs {
            true
        } else if let Some(base_origin) = parse_origin(&self.base) {
            target == base_origin && (target.scheme == "https" || is_loopback_host(&target.host))
        } else {
            false
        };
        Ok((url, attach_auth))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

fn parse_origin(url: &str) -> Option<Origin> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return None;
    }
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = if let Some(after_open) = authority.strip_prefix('[') {
        let (ip, rest) = after_open.split_once(']')?;
        let port = if let Some(p) = rest.strip_prefix(':') {
            p.parse::<u16>().ok()?
        } else if rest.is_empty() {
            default_port
        } else {
            return None;
        };
        (format!("[{}]", ip.to_ascii_lowercase()), port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        if p.is_empty() || h.contains(':') {
            return None;
        }
        (h.to_ascii_lowercase(), p.parse::<u16>().ok()?)
    } else {
        (authority.to_ascii_lowercase(), default_port)
    };
    if host.is_empty() {
        return None;
    }
    Some(Origin { scheme, host, port })
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A random-ish `u64` without the `rand` crate: std's `RandomState` is
/// seeded from the OS, mixed with the clock and process id.
fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    h.write_u128(nanos);
    h.write_u32(std::process::id());
    h.finish()
}

/// Read at most `cap` bytes; `Ok(None)` if the body is larger than that.
fn read_capped(reader: impl Read, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    reader.take(cap.saturating_add(1)).read_to_end(&mut buf)?;
    Ok((buf.len() as u64 <= cap).then_some(buf))
}

fn parse_value(data: Vec<u8>) -> Value {
    let text = match String::from_utf8(data) {
        Ok(t) => t,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    if text.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn human_size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// Build a streamed multipart body. Returns its exact length and a reader that
/// yields the part headers and file contents in order.
fn multipart_body(parts: &[Part], boundary: &str) -> Result<(u64, ChainReader)> {
    let mut chain = ChainReader {
        readers: Vec::new(),
        idx: 0,
    };
    let mut len = 0u64;
    let mut file_bytes = 0u64;
    let mut push = |chain: &mut ChainReader, bytes: Vec<u8>| {
        len += bytes.len() as u64;
        chain.readers.push(Box::new(Cursor::new(bytes)));
    };
    for part in parts {
        match part {
            Part::Text { field, value } => {
                let head = format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n",
                    quote_param(field)
                );
                push(
                    &mut chain,
                    [head.as_bytes(), value.as_bytes(), b"\r\n"].concat(),
                );
            }
            Part::File {
                field,
                filename,
                content_type,
                path,
            } => {
                let file =
                    File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
                let size = file
                    .metadata()
                    .with_context(|| format!("cannot read {}", path.display()))?
                    .len();
                let head = format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    quote_param(field),
                    quote_param(filename),
                    content_type
                        .as_deref()
                        .unwrap_or("application/octet-stream")
                );
                push(&mut chain, head.into_bytes());
                // `take` keeps the body consistent with Content-Length even if
                // the file grows while it is being sent.
                chain.readers.push(Box::new(file.take(size)));
                push(&mut chain, b"\r\n".to_vec());
                file_bytes += size;
            }
        }
    }
    push(&mut chain, format!("--{boundary}--\r\n").into_bytes());
    Ok((len + file_bytes, chain))
}

fn quote_param(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

struct ChainReader {
    readers: Vec<Box<dyn Read + Send>>,
    idx: usize,
}

impl Read for ChainReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while let Some(r) = self.readers.get_mut(self.idx) {
            let n = r.read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            self.idx += 1;
        }
        Ok(0)
    }
}

/// Resolve a `Location` header against the URL that produced it.
fn join_location(current: &str, location: &str) -> String {
    let location = location.trim();
    let lower = location.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return location.to_string();
    }
    let (scheme, rest) = current.split_once("://").unwrap_or(("https", current));
    if let Some(net) = location.strip_prefix("//") {
        return format!("{scheme}://{net}");
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..authority_end]);
    if location.starts_with('/') {
        return format!("{origin}{location}");
    }
    let path = rest[authority_end..].split(['?', '#']).next().unwrap_or("");
    if location.starts_with('?') {
        return format!("{origin}{path}{location}");
    }
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    format!("{origin}{dir}{location}")
}

/// Parse a `Retry-After` value: delay-seconds or an HTTP-date (IMF-fixdate,
/// e.g. `Wed, 21 Oct 2015 07:28:00 GMT`, or the obsolete RFC 850 form).
/// Dates in the past yield zero. The 30s cap is applied by the caller.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim();
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse::<u64>().ok().map(Duration::from_secs);
    }
    let when = parse_http_date(v)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(Duration::from_secs((when - now).max(0) as u64))
}

/// HTTP-date -> seconds since the Unix epoch.
fn parse_http_date(s: &str) -> Option<i64> {
    // Drop the weekday ("Wed," / "Wednesday,").
    let s = s.split_once(',').map_or(s, |(_, rest)| rest).trim();
    let mut it = s.split_whitespace();
    let first = it.next()?;
    let (day, mon, year) = if first.contains('-') {
        // RFC 850: 21-Oct-15
        let mut d = first.split('-');
        let (day, mon, yy) = (d.next()?, d.next()?, d.next()?);
        let yy: i64 = yy.parse().ok()?;
        let year = match yy {
            0..=69 => 2000 + yy,
            70..=99 => 1900 + yy,
            _ => yy,
        };
        (day.parse::<i64>().ok()?, mon, year)
    } else {
        let mon = it.next()?;
        let year = it.next()?.parse::<i64>().ok()?;
        (first.parse::<i64>().ok()?, mon, year)
    };
    let time = it.next()?;
    if !it.next()?.eq_ignore_ascii_case("GMT") {
        return None;
    }
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| mon.eq_ignore_ascii_case(m))? as i64
        + 1;
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, sec) = (t.next()??, t.next()??, t.next()??);
    if !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + sec)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn hint(code: u16) -> &'static str {
    match code {
        401 => " (check your email and API token: `atlx whoami`)",
        403 => " (the token may lack the permission or scope this needs)",
        _ => "",
    }
}

/// Pull a human-readable message out of the various Atlassian error shapes.
pub fn error_message(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        let t = body.trim_start();
        if t.starts_with('<') {
            // Atlassian serves HTML error pages e.g. when a product isn't on the site.
            return "non-JSON (HTML) error page; is this product enabled on your site?".into();
        }
        return truncate(body.trim(), 500);
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(msgs) = v["errorMessages"].as_array() {
        parts.extend(msgs.iter().filter_map(|m| m.as_str()).map(String::from));
    }
    if let Some(errs) = v["errors"].as_object() {
        parts.extend(
            errs.iter()
                .map(|(k, m)| format!("{k}: {}", m.as_str().unwrap_or(&m.to_string()))),
        );
    }
    if let Some(errs) = v["errors"].as_array() {
        parts.extend(
            errs.iter()
                .filter_map(|e| e["title"].as_str().or(e["message"].as_str()))
                .map(String::from),
        );
    }
    for m in [
        &v["errorMessage"],
        &v["message"],
        &v["error"]["message"],
        &v["error"]["detail"],
    ] {
        if let Some(s) = m.as_str() {
            parts.push(s.to_string());
        }
    }
    if let Some(s) = v["error"].as_str() {
        parts.push(s.to_string());
    }
    if parts.is_empty() {
        truncate(body.trim(), 500)
    } else {
        parts.join("; ")
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::{Client, error_message, join_location, parse_retry_after};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn retry_after_seconds_and_http_dates() {
        // 2015-10-21T07:27:00Z
        let now = UNIX_EPOCH + Duration::from_secs(1_445_412_420);
        assert_eq!(parse_retry_after("7", now), Some(Duration::from_secs(7)));
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", now),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            parse_retry_after("Wednesday, 21-Oct-15 07:28:00 GMT", now),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            parse_retry_after("Tue, 20 Oct 2015 07:28:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after("Thu, 29 Feb 2024 00:00:00 GMT", UNIX_EPOCH),
            Some(Duration::from_secs(1_709_164_800))
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("", now), None);
    }

    #[test]
    fn joins_redirect_locations() {
        let cur = "https://a.example:8443/rest/api/3/attachment/content/1?redirect=true";
        assert_eq!(
            join_location(cur, "https://media.example/file/x?token=t"),
            "https://media.example/file/x?token=t"
        );
        assert_eq!(
            join_location(cur, "//cdn.example/x"),
            "https://cdn.example/x"
        );
        assert_eq!(join_location(cur, "/other"), "https://a.example:8443/other");
        assert_eq!(
            join_location(cur, "2"),
            "https://a.example:8443/rest/api/3/attachment/content/2"
        );
        assert_eq!(
            join_location(cur, "?page=2"),
            "https://a.example:8443/rest/api/3/attachment/content/1?page=2"
        );
    }

    #[test]
    fn backoff_is_bounded_and_jittered() {
        let c = Client::new("https://x.example".into(), "a", "b", None);
        for attempt in 0..3 {
            let full = Duration::from_secs(1 << attempt);
            let w = c.backoff(attempt);
            assert!(w >= full / 2 && w <= full, "{attempt}: {w:?}");
        }
        assert!(c.backoff(10) <= Duration::from_secs(30));
    }

    #[test]
    fn extracts_jira_errors() {
        let b = r#"{"errorMessages":["Issue does not exist"],"errors":{"summary":"required"}}"#;
        assert_eq!(error_message(b), "Issue does not exist; summary: required");
    }

    #[test]
    fn extracts_bitbucket_and_confluence_errors() {
        assert_eq!(
            error_message(r#"{"type":"error","error":{"message":"Repository not found"}}"#),
            "Repository not found"
        );
        assert_eq!(
            error_message(r#"{"errors":[{"status":404,"title":"Not Found"}]}"#),
            "Not Found"
        );
        assert_eq!(error_message("plain text"), "plain text");
    }

    #[test]
    fn auth_origin_and_scheme_guard() {
        let c = Client::new(
            "https://api.bitbucket.org/2.0".into(),
            "a@b.c",
            "secret",
            None,
        );
        assert_eq!(
            c.resolve_request("/repositories/acme").unwrap(),
            (
                "https://api.bitbucket.org/2.0/repositories/acme".into(),
                true
            )
        );
        assert_eq!(
            c.resolve_request("https://api.bitbucket.org/2.0/repositories/acme?page=2")
                .unwrap(),
            (
                "https://api.bitbucket.org/2.0/repositories/acme?page=2".into(),
                true
            )
        );
        assert_eq!(
            c.resolve_request("HTTPS://API.BITBUCKET.ORG:443/2.0/repositories/acme")
                .unwrap(),
            (
                "HTTPS://API.BITBUCKET.ORG:443/2.0/repositories/acme".into(),
                true
            )
        );
        // Different host must not receive Authorization.
        assert_eq!(
            c.resolve_request("https://evil.example/steal").unwrap(),
            ("https://evil.example/steal".into(), false)
        );
        assert_eq!(
            c.resolve_request("https://api.bitbucket.org.evil.example/steal")
                .unwrap(),
            ("https://api.bitbucket.org.evil.example/steal".into(), false)
        );
        // Plaintext HTTP on non-loopback hosts must be rejected.
        assert!(
            c.resolve_request("http://api.bitbucket.org/2.0/repositories/acme")
                .is_err()
        );
        assert!(c.resolve_request("http://evil.example/steal").is_err());
        let bad_base = Client::new(
            "http://api.bitbucket.org/2.0".into(),
            "a@b.c",
            "secret",
            None,
        );
        assert!(bad_base.resolve_request("/repositories/acme").is_err());

        // Loopback HTTP is allowed for local mock testing.
        let local = Client::new("http://127.0.0.1:8080".into(), "a@b.c", "secret", None);
        assert_eq!(
            local.resolve_request("/next").unwrap(),
            ("http://127.0.0.1:8080/next".into(), true)
        );
        assert_eq!(
            local
                .resolve_request("http://127.0.0.1:8080/next?page=2")
                .unwrap(),
            ("http://127.0.0.1:8080/next?page=2".into(), true)
        );
        assert_eq!(
            local.resolve_request("http://localhost:8080/next").unwrap(),
            ("http://localhost:8080/next".into(), false)
        );
        let localhost = Client::new("http://localhost:8080".into(), "a@b.c", "secret", None);
        assert_eq!(
            localhost
                .resolve_request("http://localhost:8080/next")
                .unwrap(),
            ("http://localhost:8080/next".into(), true)
        );
    }

    #[test]
    fn wire_request_omits_auth_for_different_host() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        fn serve_once(listener: TcpListener) -> std::thread::JoinHandle<String> {
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
                stream.write_all(resp.as_bytes()).unwrap();
                req
            })
        }

        let same_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let same_addr = same_listener.local_addr().unwrap();
        let same_handle = serve_once(same_listener);

        let other_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let other_addr = other_listener.local_addr().unwrap();
        let other_handle = serve_once(other_listener);

        let c = Client::new(format!("http://{same_addr}"), "a@b.c", "secret", None);
        c.get(&format!("http://{same_addr}/ok"), &[]).unwrap();
        c.get(&format!("http://{other_addr}/other"), &[]).unwrap();

        let same_req = same_handle.join().unwrap().to_ascii_lowercase();
        let other_req = other_handle.join().unwrap().to_ascii_lowercase();
        assert!(same_req.contains("authorization: basic "), "{same_req}");
        assert!(!other_req.contains("authorization:"), "{other_req}");
    }
}

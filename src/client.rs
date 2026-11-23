//! Thin HTTP client for the Atlassian REST APIs: basic auth, readable errors,
//! retries (429 always; 502/503/504 and transport errors for idempotent
//! methods), manual redirect handling that never sends `Authorization` to
//! another origin, capped text/JSON bodies, binary downloads and streamed
//! `multipart/form-data` uploads.
//!
//! Failures are [`ApiError`]s inside `anyhow::Error`, so callers can recover
//! the HTTP status and error class with [`ApiError::find`] (or
//! `err.downcast_ref::<ApiError>()`) instead of parsing message text.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde_json::{Map, Value, json};
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
    mode: Mode,
}

/// What the client does with a mutating request: any method other than
/// GET, HEAD and OPTIONS, unless sent through [`Client::post_read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Send it.
    #[default]
    Live,
    /// Do not send it: stop the command with a [`DryRun`] carrying the
    /// request. Reads still go out.
    DryRun,
    /// Refuse it with a `refused` error (`--read-only`, `ATLX_READ_ONLY`,
    /// `read_only = true`).
    ReadOnly,
}

/// A raw (possibly binary) response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes {
    /// The final HTTP status, e.g. `206` for a satisfied `Range` request.
    pub status: u16,
    pub data: Vec<u8>,
    /// The raw `Content-Type` header, if the server sent one.
    pub content_type: Option<String>,
}

/// A JSON response with the parts of its head some commands act on.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    /// The raw `Location` header, e.g. of a `202 Accepted`.
    pub location: Option<String>,
    pub body: Value,
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
    /// A POST that only reads ([`Client::post_read`]): exempt from
    /// dry-run and read-only mode.
    read: bool,
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
            mode: Mode::Live,
        }
    }

    /// How mutating requests are handled (default [`Mode::Live`]).
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn mode(&self) -> Mode {
        self.mode
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

    /// POST to an endpoint that only reads (a search or bulk fetch sent as
    /// POST). It still goes out under `--dry-run` and read-only mode, so
    /// only use it for endpoints Atlassian documents as reads.
    pub fn post_read(&self, path: &str, body: &Value) -> Result<Value> {
        let opts = Opts {
            headers: &[],
            default_accept: "application/json",
            overall_timeout: true,
            read: true,
        };
        let (resp, desc) = self.execute("POST", path, &[], Payload::Json(body), &opts)?;
        Ok(parse_value(self.read_body(resp, &desc)?))
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
            read: false,
        };
        let payload = body.map_or(Payload::Empty, Payload::Json);
        let (resp, desc) = self.execute(method, path, query, payload, &opts)?;
        Ok(parse_value(self.read_body(resp, &desc)?))
    }

    /// Like [`Client::request`], for endpoints where the status or the
    /// `Location` header matters (`202 Accepted` and its task URL).
    pub fn request_response(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Response> {
        let opts = Opts {
            headers: &[],
            default_accept: "application/json",
            overall_timeout: true,
            read: false,
        };
        let payload = body.map_or(Payload::Empty, Payload::Json);
        let (resp, desc) = self.execute(method, path, query, payload, &opts)?;
        let status = resp.status();
        let location = resp.header("Location").map(String::from);
        Ok(Response {
            status,
            location,
            body: parse_value(self.read_body(resp, &desc)?),
        })
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
            read: false,
        };
        let (resp, desc) = self.execute("GET", path, query, Payload::Empty, &opts)?;
        let status = resp.status();
        let content_type = resp.header("Content-Type").map(String::from);
        let data = self.read_body(resp, &desc)?;
        Ok(Bytes {
            status,
            data,
            content_type,
        })
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
            read: false,
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
            read: false,
        };
        let payload = Payload::Multipart { parts, boundary };
        let (resp, desc) = self.execute("POST", path, &[], payload, &opts)?;
        Ok(parse_value(self.read_body(resp, &desc)?))
    }

    /// [`Client::post_multipart`], for endpoints that answer with a `Location`.
    pub fn post_multipart_response(&self, path: &str, parts: &[Part]) -> Result<Response> {
        let boundary = format!("atlx-{:016x}{:08x}", random_u64(), std::process::id());
        let opts = Opts {
            headers: &[],
            default_accept: "application/json",
            overall_timeout: false,
            read: false,
        };
        let payload = Payload::Multipart { parts, boundary };
        let (resp, desc) = self.execute("POST", path, &[], payload, &opts)?;
        let status = resp.status();
        let location = resp.header("Location").map(String::from);
        Ok(Response {
            status,
            location,
            body: parse_value(self.read_body(resp, &desc)?),
        })
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
        check_path(method, path)?;
        let (mut url, mut auth) = self.resolve_request(path)?;
        let mut method = method.to_ascii_uppercase();
        if !opts.read && !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
            self.guard_write(&method, &url, query, &payload)?;
        }
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

    /// Apply [`Mode`] to a mutating request before anything is sent.
    fn guard_write(
        &self,
        method: &str,
        url: &str,
        query: &[(&str, &str)],
        payload: &Payload<'_>,
    ) -> Result<()> {
        let shown = redact_url(&with_query(url, query));
        match self.mode {
            Mode::Live => Ok(()),
            Mode::ReadOnly => Err(ApiError::refused(format!(
                "read-only mode: refusing to send {method} {shown}"
            ))
            .with_hint(
                "read-only mode is on (--read-only, ATLX_READ_ONLY or the profile's read_only); ask the person before turning it off",
            )
            .into()),
            Mode::DryRun => Err(DryRun {
                method: method.to_string(),
                url: shown,
                body: match payload {
                    Payload::Empty => None,
                    Payload::Json(b) => Some((*b).clone()),
                    Payload::Multipart { parts, .. } => Some(json!({
                        "multipart": parts.iter().map(|p| match p {
                            Part::File { field, filename, path, .. } => json!({
                                "field": field, "filename": filename, "path": path.display().to_string(),
                            }),
                            Part::Text { field, value } => json!({"field": field, "value": value}),
                        }).collect::<Vec<_>>(),
                    })),
                },
            }
            .into()),
        }
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
                    let header = |names: &[&str]| {
                        names
                            .iter()
                            .find_map(|n| r.header(n))
                            .map(|v| truncate(v.trim(), 200))
                            .filter(|v| !v.is_empty())
                    };
                    let request_id = header(&["X-ARequestId", "X-Request-Id"]);
                    let trace_id = header(&["Atl-TraceId"]);
                    let retry_after = r
                        .header("Retry-After")
                        .and_then(|v| parse_retry_after(v, SystemTime::now()));
                    let text = read_capped(r.into_reader(), ERROR_BODY_CAP)
                        .ok()
                        .flatten()
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    let mut err = ApiError::http(method, url, code, error_message(&text));
                    err.request_id = request_id;
                    err.trace_id = trace_id;
                    err.fields = error_fields(&text);
                    if code == 429 || matches!(code, 502..=504) {
                        err.retry_after_ms = retry_after.map(|d| d.as_millis() as u64);
                    }
                    return Err(err.into());
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
                    let timed_out = t.kind() == ureq::ErrorKind::Io
                        && t.to_string().to_ascii_lowercase().contains("timed out");
                    let kind = if timed_out {
                        ErrorKind::Timeout
                    } else {
                        ErrorKind::Network
                    };
                    let mut err = ApiError::new(kind, redact_url(&t.to_string()));
                    err.method = Some(method.to_string());
                    err.url = Some(redact_url(url));
                    if !idempotent {
                        err.write_state_unknown = true;
                        err.hint = Some(
                            "the write may or may not have happened; read the item back before retrying"
                                .into(),
                        );
                    }
                    return Err(err.into());
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

/// Refuse a path that would reach another resource than the one it names.
///
/// `urlencoding` leaves dots alone and the URL parser resolves dot segments,
/// so an ID of `..` would turn "delete this comment" into "delete its work
/// item". The parser also reads `%2e` as a dot and `\\` as `/`, and drops tabs,
/// newlines and trailing blanks. An empty ID similarly turns a DELETE of one
/// item into a DELETE of its collection.
///
/// Only paths built by atlx or given by the user are checked; redirects may
/// use relative references.
fn check_path(method: &str, path: &str) -> Result<()> {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = match path.split_once("://") {
        Some((scheme, rest)) if !scheme.contains('/') => rest.find('/').map_or("", |i| &rest[i..]),
        _ => path,
    };
    let mut segments = Vec::new();
    for raw in path.split('/') {
        let decoded = urlencoding::decode_binary(raw.as_bytes());
        let decoded = String::from_utf8_lossy(&decoded).replace(['\t', '\n', '\r'], "");
        for segment in decoded.split(['/', '\\']) {
            if matches!(segment.trim_end_matches(|c: char| c <= ' '), "." | "..") {
                return Err(ApiError::usage(format!(
                    "refusing to send {method} {path}: an ID or path segment is '.' or '..'"
                ))
                .into());
            }
        }
        segments.push(raw);
    }
    if method.eq_ignore_ascii_case("DELETE") && segments.iter().skip(1).any(|s| s.trim().is_empty())
    {
        return Err(ApiError::usage(format!(
            "refusing to send {method} {path}: an ID or path segment is empty"
        ))
        .into());
    }
    Ok(())
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

/// The hint for an HTTP status, before any endpoint-specific one.
fn status_hint(code: u16) -> Option<&'static str> {
    match code {
        401 => Some(
            "the API token was rejected; ask the person to check it with `atlx auth status` or run `atlx auth login`",
        ),
        403 => Some(
            "this account lacks the permission or token scope this needs; ask the person, do not retry",
        ),
        404 | 410 => Some("check the ID; the item may also exist but be invisible to this account"),
        409 | 412 => Some("the item changed since it was read; re-read it, then retry"),
        429 => Some("rate limited after atlx's own retries; wait, then retry"),
        502..=504 => Some("the service is unavailable after atlx's own retries; retry later"),
        400 | 422 => Some(
            "the request was rejected as invalid; check `error.fields` and `atlx describe <command>`",
        ),
        500..=599 => Some("the service failed; it may work later, report it if it persists"),
        _ => None,
    }
}

/// Process exit codes. Each class of failure gets its own code, so a caller
/// that only sees the exit status still knows whether to fix the command,
/// ask the person, retry or stop. `atlx describe --errors` lists them.
pub mod exit {
    pub const OK: i32 = 0;
    /// `validation`, `server` after retries, `other`.
    pub const FAILURE: i32 = 1;
    /// `usage`: fix the command.
    pub const USAGE: i32 = 2;
    /// Some inputs of a multi-input command failed: use `data`, inspect `errors`.
    pub const PARTIAL: i32 = 3;
    /// `config`, `auth`, `forbidden`: stop and ask the person.
    pub const ASK: i32 = 4;
    /// `not_found`: check the ID.
    pub const NOT_FOUND: i32 = 5;
    /// `conflict`: re-read, then retry.
    pub const CONFLICT: i32 = 6;
    /// `rate_limited`, `network`, `timeout`: retry later.
    pub const RETRY: i32 = 7;
    /// `refused`: a safety guard stopped the write.
    pub const REFUSED: i32 = 8;
}

/// What went wrong, independent of the message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// HTTP 401.
    Auth,
    /// HTTP 403.
    Forbidden,
    /// HTTP 404 and 410.
    NotFound,
    /// HTTP 409 and 412 (e.g. Confluence version conflicts).
    Conflict,
    /// HTTP 400, 422 and any other 4xx not listed above.
    Validation,
    /// HTTP 429, after the client's own retries.
    RateLimited,
    /// HTTP 5xx.
    Server,
    /// No HTTP response at all (DNS, connect, TLS, connection reset).
    Network,
    /// The request timed out.
    Timeout,
    /// Missing site, email or token.
    Config,
    /// Invalid arguments, caught by clap or after parsing.
    Usage,
    /// A safety guard stopped a write: missing `--yes`, read-only mode, a
    /// lossy conversion, a stale `--if-version`.
    Refused,
    /// Anything else (local I/O, oversized bodies, redirect limits, ...).
    Other,
}

impl ErrorKind {
    /// Every kind, in the order `atlx describe --errors` lists them.
    pub const ALL: [ErrorKind; 13] = [
        Self::Usage,
        Self::Config,
        Self::Auth,
        Self::Forbidden,
        Self::NotFound,
        Self::Conflict,
        Self::Validation,
        Self::RateLimited,
        Self::Server,
        Self::Network,
        Self::Timeout,
        Self::Refused,
        Self::Other,
    ];

    pub fn from_status(status: u16) -> Self {
        match status {
            401 => Self::Auth,
            403 => Self::Forbidden,
            404 | 410 => Self::NotFound,
            409 | 412 => Self::Conflict,
            429 => Self::RateLimited,
            500..=599 => Self::Server,
            400..=499 => Self::Validation,
            _ => Self::Other,
        }
    }

    /// Stable `snake_case` name, emitted as `error.code`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Validation => "validation",
            Self::RateLimited => "rate_limited",
            Self::Server => "server",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Config => "config",
            Self::Usage => "usage",
            Self::Refused => "refused",
            Self::Other => "other",
        }
    }

    /// The process exit code for this class.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Usage => exit::USAGE,
            Self::Config | Self::Auth | Self::Forbidden => exit::ASK,
            Self::NotFound => exit::NOT_FOUND,
            Self::Conflict => exit::CONFLICT,
            Self::RateLimited | Self::Network | Self::Timeout => exit::RETRY,
            Self::Refused => exit::REFUSED,
            Self::Validation | Self::Server | Self::Other => exit::FAILURE,
        }
    }

    /// One line on what the class means, for `atlx describe --errors`.
    pub fn meaning(self) -> &'static str {
        match self {
            Self::Usage => "the command line is wrong",
            Self::Config => "site, email or token is missing or unusable",
            Self::Auth => "the credentials were rejected",
            Self::Forbidden => "the account lacks permission or token scope",
            Self::NotFound => "no such item, or it is not visible to this account",
            Self::Conflict => "the item changed since it was read",
            Self::Validation => "the service rejected the request as invalid",
            Self::RateLimited => "rate limited after atlx's own retries",
            Self::Server => "the service failed after atlx's own retries",
            Self::Network => "no response from the service",
            Self::Timeout => "the request timed out",
            Self::Refused => "a safety guard stopped a write",
            Self::Other => "a local or unexpected failure",
        }
    }

    /// The default `hint` for errors of this class that have no better one.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Usage => "fix the command; see `suggestions` or run `atlx describe <command>`",
            Self::Config => {
                "ask the person to run `atlx auth login`; never pass a token on the command line"
            }
            Self::Auth => {
                "the credentials were rejected; ask the person to run `atlx auth status` or `atlx auth login`"
            }
            Self::Forbidden => "this account cannot do this; ask the person, do not retry",
            Self::NotFound => {
                "check the ID; the item may also exist but be invisible to this account"
            }
            Self::Conflict => "re-read the item, re-apply the change, then retry",
            Self::Validation => {
                "check `error.fields` and the command's flags in `atlx describe <command>`"
            }
            Self::RateLimited => "wait, then retry",
            Self::Server => "the service failed; it may work later",
            Self::Network => "check connectivity, then retry",
            Self::Timeout => "retry later, or raise --timeout-ms",
            Self::Refused => "do not work around the guard; run with --dry-run to preview, or ask",
            Self::Other => "read the message",
        }
    }

    /// Whether retrying later may succeed. atlx has already retried 429s
    /// and, for idempotent requests, 502/503/504 and transport errors.
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Network | Self::Timeout)
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A classified failure. API functions return it inside `anyhow::Error`;
/// recover it with [`ApiError::find`] or [`ApiError::from_anyhow`].
///
/// `url` never contains userinfo and no request headers are kept, so the
/// value is safe to print.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub kind: ErrorKind,
    /// The HTTP status, when the server answered.
    pub status: Option<u16>,
    pub method: Option<String>,
    pub url: Option<String>,
    /// Human-readable message (for HTTP errors, extracted from the body).
    pub message: String,
    pub hint: Option<String>,
    /// `X-ARequestId` / `X-Request-Id` response header.
    pub request_id: Option<String>,
    /// `Atl-TraceId` response header.
    pub trace_id: Option<String>,
    /// How long the server asked to wait (`Retry-After`), when known.
    pub retry_after_ms: Option<u64>,
    /// Field-level validation messages (Jira's `errors: {field: msg}`).
    pub fields: Option<Value>,
    /// Commands or flags to try instead (usage errors).
    pub suggestions: Vec<String>,
    /// Set when a non-idempotent write failed without an answer, so it may
    /// or may not have happened.
    pub write_state_unknown: bool,
    /// What a body conversion could not represent (`[{node, at, detail}]`).
    pub losses: Option<Value>,
}

impl ApiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            status: None,
            method: None,
            url: None,
            message: message.into(),
            hint: None,
            request_id: None,
            trace_id: None,
            retry_after_ms: None,
            fields: None,
            suggestions: Vec::new(),
            write_state_unknown: false,
            losses: None,
        }
    }

    /// An HTTP error response.
    pub fn http(method: &str, url: &str, status: u16, message: impl Into<String>) -> Self {
        let mut e = Self::new(ErrorKind::from_status(status), message);
        e.status = Some(status);
        e.method = Some(method.to_string());
        e.url = Some(redact_url(url));
        e.hint = status_hint(status).map(String::from);
        e
    }

    /// Missing or unusable site/credential configuration.
    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Config, message)
    }

    /// Invalid arguments detected after clap parsing.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, message)
    }

    /// A safety guard stopped a write before anything was sent.
    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Refused, message)
    }

    /// Transition shim: something the public REST API cannot do. It is a
    /// `usage` error now; product code moves to [`ApiError::usage`].
    #[deprecated(note = "use ApiError::usage")]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::usage(message)
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_suggestions(mut self, s: impl IntoIterator<Item = String>) -> Self {
        self.suggestions.extend(s);
        self
    }

    /// The `ApiError` inside an `anyhow` chain (through any `context`).
    pub fn find(err: &anyhow::Error) -> Option<&ApiError> {
        err.downcast_ref::<ApiError>()
    }

    /// Classify any error. An `ApiError` in the chain keeps its fields, with
    /// outer `context` messages prefixed to `message`; anything else becomes
    /// [`ErrorKind::Other`] with the whole chain as its message.
    pub fn from_anyhow(err: &anyhow::Error) -> ApiError {
        if let Some(e) = foreign(err) {
            return e;
        }
        let Some(api) = Self::find(err) else {
            return Self::new(ErrorKind::Other, format!("{err:#}"));
        };
        let mut parts = Vec::new();
        for cause in err.chain() {
            match cause.downcast_ref::<ApiError>() {
                Some(a) => {
                    parts.push(a.message.clone());
                    break;
                }
                None => parts.push(cause.to_string()),
            }
        }
        let mut out = api.clone();
        out.message = parts.join(": ");
        out
    }

    /// The error code: the lower-case class name.
    pub fn code(&self) -> &'static str {
        self.kind.as_str()
    }

    /// Whether retrying later may succeed (see [`ErrorKind::retryable`]);
    /// a 502/503/504 counts as well.
    pub fn retryable(&self) -> bool {
        self.kind.retryable()
            || (self.kind == ErrorKind::Server && matches!(self.status, Some(502..=504)))
    }

    /// The process exit code for this error.
    pub fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }

    /// The hint that is printed: the specific one, else the class default.
    pub fn effective_hint(&self) -> String {
        self.hint
            .clone()
            .unwrap_or_else(|| self.kind.hint().to_string())
    }

    /// The `error` object of the JSON error envelope.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("code".into(), self.code().into());
        m.insert("message".into(), self.message.clone().into());
        if let Some(s) = self.status {
            m.insert("status".into(), s.into());
        }
        m.insert("hint".into(), self.effective_hint().into());
        m.insert("retryable".into(), self.retryable().into());
        if let Some(ms) = self.retry_after_ms {
            m.insert("retryAfterMs".into(), ms.into());
        }
        if self.write_state_unknown {
            m.insert("writeState".into(), "unknown".into());
        }
        if let Some(f) = &self.fields {
            m.insert("fields".into(), f.clone());
        }
        if !self.suggestions.is_empty() {
            m.insert("suggestions".into(), json!(self.suggestions));
        }
        if let Some(l) = &self.losses {
            m.insert("losses".into(), l.clone());
        }
        if let (Some(method), Some(url)) = (&self.method, &self.url) {
            m.insert("request".into(), json!({"method": method, "url": url}));
        }
        if let Some(r) = &self.request_id {
            m.insert("requestId".into(), r.clone().into());
        }
        if let Some(t) = &self.trace_id {
            m.insert("traceId".into(), t.clone().into());
        }
        Value::Object(m)
    }

    /// `{"ok": false, "error": {...}}`, the envelope printed in JSON modes.
    pub fn envelope(&self) -> Value {
        json!({"ok": false, "error": self.to_json()})
    }

    /// The per-input error of a multi-input command (same shape as `error`).
    pub fn item_json(&self) -> Value {
        self.to_json()
    }

    /// The multi-line report printed on stderr in text mode.
    pub fn report(&self) -> String {
        let mut out = format!("Error: {}\n  Code:        {}\n", self.message, self.code());
        if let Some(s) = self.status {
            out.push_str(&format!("  HTTP status: {s}\n"));
        }
        if let Some(r) = &self.request_id {
            out.push_str(&format!("  Request ID:  {r}\n"));
        }
        if let (Some(m), Some(u)) = (&self.method, &self.url) {
            out.push_str(&format!("  Request:     {m} {u}\n"));
        }
        if let Some(Value::Object(fields)) = &self.fields {
            for (k, v) in fields {
                let v = v.as_str().map_or_else(|| v.to_string(), String::from);
                out.push_str(&format!("  Field:       {k}: {v}\n"));
            }
        }
        for s in &self.suggestions {
            out.push_str(&format!("  Try:         {s}\n"));
        }
        if self.write_state_unknown {
            out.push_str("  Write state: unknown (the write may have happened)\n");
        }
        out.push_str(&format!("  Hint:        {}\n", self.effective_hint()));
        out
    }
}

/// Classify errors other modules define, before the generic fallback.
///
/// When the markup package's `crate::markup::BodyError` lands, it maps here:
/// `Refused` -> [`ErrorKind::Refused`] (exit 8), `Invalid` ->
/// [`ErrorKind::Usage`], with its `hint` and `losses`:
///
/// ```ignore
/// if let Some(b) = err.chain().find_map(|c| c.downcast_ref::<crate::markup::BodyError>()) {
///     let kind = if b.is_refused() { ErrorKind::Refused } else { ErrorKind::Usage };
///     let mut e = ApiError::new(kind, b.message.clone()).with_hint(b.hint.clone());
///     e.losses = serde_json::to_value(&b.losses).ok();
///     return Some(e);
/// }
/// ```
fn foreign(_err: &anyhow::Error) -> Option<ApiError> {
    None
}

/// The request a `--dry-run` stopped before sending. It travels up as an
/// error so the command ends there; `main` prints it as a success.
#[derive(Debug, Clone, PartialEq)]
pub struct DryRun {
    pub method: String,
    pub url: String,
    pub body: Option<Value>,
}

impl DryRun {
    /// The `DryRun` inside an `anyhow` chain.
    pub fn find(err: &anyhow::Error) -> Option<&DryRun> {
        err.chain().find_map(|c| c.downcast_ref::<DryRun>())
    }

    pub fn to_json(&self) -> Value {
        let mut req = json!({"method": self.method, "url": self.url});
        if let Some(b) = &self.body {
            req["body"] = b.clone();
        }
        req
    }
}

impl std::fmt::Display for DryRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dry run: {} {} was not sent", self.method, self.url)
    }
}

impl std::error::Error for DryRun {}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(m), Some(u)) = (&self.method, &self.url) {
            write!(f, "{m} {u} failed: ")?;
            if let Some(s) = self.status {
                write!(f, "HTTP {s}: ")?;
            }
        }
        f.write_str(&self.message)?;
        if let Some(h) = &self.hint {
            write!(f, " ({h})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// Drop any `user:password@` from URLs inside `s` before it is shown.
pub fn redact_url(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => out.push_str(&authority[at + 1..]),
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
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

/// Field-level messages (`errors: {field: message}`, as Jira sends them).
pub fn error_fields(body: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("errors")
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())
        .map(|m| Value::Object(m.clone()))
}

/// `url` with `query` appended, for showing a request.
fn with_query(url: &str, query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return url.to_string();
    }
    let q: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect();
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{}", q.join("&"))
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
    use super::{
        ApiError, Client, ErrorKind, error_message, exit, join_location, parse_retry_after,
        redact_url,
    };

    #[test]
    fn status_maps_to_kind_code_retry_and_exit() {
        let cases: &[(u16, ErrorKind, &str, bool, i32)] = &[
            (
                400,
                ErrorKind::Validation,
                "validation",
                false,
                exit::FAILURE,
            ),
            (401, ErrorKind::Auth, "auth", false, exit::ASK),
            (403, ErrorKind::Forbidden, "forbidden", false, exit::ASK),
            (
                404,
                ErrorKind::NotFound,
                "not_found",
                false,
                exit::NOT_FOUND,
            ),
            (409, ErrorKind::Conflict, "conflict", false, exit::CONFLICT),
            (412, ErrorKind::Conflict, "conflict", false, exit::CONFLICT),
            (
                422,
                ErrorKind::Validation,
                "validation",
                false,
                exit::FAILURE,
            ),
            (
                429,
                ErrorKind::RateLimited,
                "rate_limited",
                true,
                exit::RETRY,
            ),
            (500, ErrorKind::Server, "server", false, exit::FAILURE),
            (503, ErrorKind::Server, "server", true, exit::FAILURE),
        ];
        for &(status, kind, code, retry, exit_code) in cases {
            let e = ApiError::http("GET", "https://x.test/a", status, "m");
            assert_eq!(e.kind, kind, "{status}");
            assert_eq!(e.code(), code, "{status}");
            assert_eq!(e.retryable(), retry, "{status}");
            assert_eq!(e.exit_code(), exit_code, "{status}");
            let j = e.to_json();
            assert_eq!(j["status"], status);
            assert_eq!(j["code"], code);
            assert_eq!(j["retryable"], retry);
            assert!(
                j["hint"].as_str().is_some_and(|h| !h.is_empty()),
                "{status}"
            );
            assert_eq!(j["request"]["method"], "GET");
        }
        let cfg = ApiError::config("no token").to_json();
        assert_eq!(cfg["code"], "config");
        assert!(cfg.get("status").is_none());
        assert!(cfg["hint"].as_str().unwrap().contains("atlx auth login"));
        for kind in ErrorKind::ALL {
            let j = ApiError::new(kind, "m").to_json();
            assert_eq!(j["code"], kind.as_str());
            assert!(!kind.hint().is_empty() && !kind.meaning().is_empty());
        }
        assert_eq!(ErrorKind::Refused.exit_code(), exit::REFUSED);
        assert_eq!(ErrorKind::Usage.exit_code(), exit::USAGE);
        let net = ApiError::new(ErrorKind::Network, "refused");
        assert!(net.retryable());
        assert_eq!(net.exit_code(), exit::RETRY);
    }

    #[test]
    fn from_anyhow_keeps_kind_through_context() {
        use anyhow::Context;
        let r: anyhow::Result<()> =
            Err(ApiError::http("GET", "https://x.test", 404, "gone").into());
        let e = r.context("loading PROJ-1").unwrap_err();
        let api = ApiError::from_anyhow(&e);
        assert_eq!(api.kind, ErrorKind::NotFound);
        assert_eq!(api.message, "loading PROJ-1: gone");
        let other = ApiError::from_anyhow(&anyhow::anyhow!("disk full"));
        assert_eq!(other.kind, ErrorKind::Other);
    }

    #[test]
    fn redacts_userinfo() {
        assert_eq!(
            redact_url("https://me:tok@x.test/a?b=1"),
            "https://x.test/a?b=1"
        );
        assert_eq!(
            redact_url("GET http://u:p@127.0.0.1:9/x failed"),
            "GET http://127.0.0.1:9/x failed"
        );
        assert_eq!(redact_url("no url here"), "no url here");
    }
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

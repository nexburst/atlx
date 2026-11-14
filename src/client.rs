//! Thin JSON-over-HTTP client with basic auth, readable errors and 429 retry.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;

pub struct Client {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

const MAX_RETRIES: u32 = 3;

impl Client {
    pub fn new(base: String, email: &str, token: &str, timeout_ms: Option<u64>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(timeout_ms.unwrap_or(60_000)))
            .user_agent(concat!("atlx/", env!("CARGO_PKG_VERSION")))
            .build();
        let creds = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
        Self {
            agent,
            base,
            auth: format!("Basic {creds}"),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.request("GET", path, query, None)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.request("POST", path, &[], Some(body))
    }

    pub fn put(&self, path: &str, body: &Value) -> Result<Value> {
        self.request("PUT", path, &[], Some(body))
    }

    pub fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.request("DELETE", path, query, None)
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
        let (url, attach_auth) = self.resolve_request(path)?;
        let mut attempt = 0;
        loop {
            let mut req = self
                .agent
                .request(method, &url)
                .set("Accept", "application/json");
            if attach_auth {
                req = req.set("Authorization", &self.auth);
            }
            for (k, v) in query {
                req = req.query(k, v);
            }
            let res = match body {
                Some(b) => req.send_json(b),
                None => req.call(),
            };
            match res {
                Ok(r) => {
                    let text = r.into_string()?;
                    if text.trim().is_empty() {
                        return Ok(Value::Null);
                    }
                    return Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)));
                }
                Err(ureq::Error::Status(429, r)) if attempt < MAX_RETRIES => {
                    let wait = r
                        .header("Retry-After")
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .unwrap_or(2u64.pow(attempt))
                        .min(30);
                    attempt += 1;
                    std::thread::sleep(Duration::from_secs(wait));
                }
                Err(ureq::Error::Status(code, r)) => {
                    let text = r.into_string().unwrap_or_default();
                    bail!(
                        "{method} {url} failed: HTTP {code}: {}{}",
                        error_message(&text),
                        hint(code)
                    );
                }
                Err(e) => bail!("{method} {url} failed: {e}"),
            }
        }
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
    use super::{Client, error_message};

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

//! Thin JSON-over-HTTP client with basic auth, readable errors and 429 retry.

use anyhow::{Result, bail};
use base64::Engine;
use serde_json::Value;
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
        Self { agent, base, auth: format!("Basic {creds}") }
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

    /// `path` is appended to the base URL unless it is already absolute
    /// (e.g. a Bitbucket `next` pagination link).
    pub fn request(&self, method: &str, path: &str, query: &[(&str, &str)], body: Option<&Value>) -> Result<Value> {
        let url = if path.starts_with("https://") || path.starts_with("http://") {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        };
        let mut attempt = 0;
        loop {
            let mut req = self
                .agent
                .request(method, &url)
                .set("Authorization", &self.auth)
                .set("Accept", "application/json");
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
                    bail!("{method} {url} failed: HTTP {code}: {}{}", error_message(&text), hint(code));
                }
                Err(e) => bail!("{method} {url} failed: {e}"),
            }
        }
    }
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
        parts.extend(errs.iter().map(|(k, m)| format!("{k}: {}", m.as_str().unwrap_or(&m.to_string()))));
    }
    if let Some(errs) = v["errors"].as_array() {
        parts.extend(errs.iter().filter_map(|e| e["title"].as_str().or(e["message"].as_str())).map(String::from));
    }
    for m in [&v["errorMessage"], &v["message"], &v["error"]["message"], &v["error"]["detail"]] {
        if let Some(s) = m.as_str() {
            parts.push(s.to_string());
        }
    }
    if let Some(s) = v["error"].as_str() {
        parts.push(s.to_string());
    }
    if parts.is_empty() { truncate(body.trim(), 500) } else { parts.join("; ") }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

#[cfg(test)]
mod tests {
    use super::error_message;

    #[test]
    fn extracts_jira_errors() {
        let b = r#"{"errorMessages":["Issue does not exist"],"errors":{"summary":"required"}}"#;
        assert_eq!(error_message(b), "Issue does not exist; summary: required");
    }

    #[test]
    fn extracts_bitbucket_and_confluence_errors() {
        assert_eq!(error_message(r#"{"type":"error","error":{"message":"Repository not found"}}"#), "Repository not found");
        assert_eq!(error_message(r#"{"errors":[{"status":404,"title":"Not Found"}]}"#), "Not Found");
        assert_eq!(error_message("plain text"), "plain text");
    }
}

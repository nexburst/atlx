//! `jira workitem watcher` and `jira workitem vote`.

use super::{enc, resolve_account, strip_media};
use crate::client::Client;
use anyhow::Result;
use serde_json::{Value, json};

fn path(key: &str, what: &str) -> String {
    format!("/rest/api/3/issue/{}/{what}", enc(key))
}

pub fn watchers(c: &Client, key: &str) -> Result<Value> {
    let mut v = c.get(&path(key, "watchers"), &[])?;
    strip_media(&mut v);
    Ok(v)
}

/// `who` is `me`, an account ID or ARI, or an email or name to look up.
pub fn watcher_add(c: &Client, key: &str, who: &str) -> Result<Value> {
    let account = resolve_account(c, who)?;
    // The body is the bare account ID as a JSON string.
    c.post(&path(key, "watchers"), &Value::from(account.as_str()))?;
    Ok(json!({"id": key, "accountId": account, "watching": true}))
}

pub fn watcher_remove(c: &Client, key: &str, who: &str) -> Result<Value> {
    let account = resolve_account(c, who)?;
    // Without an account Jira would pick the caller, so never send it empty.
    if account.is_empty() {
        anyhow::bail!("no account ID for '{who}'");
    }
    c.delete(&path(key, "watchers"), &[("accountId", account.as_str())])?;
    Ok(json!({"id": key, "accountId": account, "watching": false}))
}

pub fn votes(c: &Client, key: &str) -> Result<Value> {
    let mut v = c.get(&path(key, "votes"), &[])?;
    strip_media(&mut v);
    Ok(v)
}

/// Votes only exist for the calling user.
pub fn vote(c: &Client, key: &str, on: bool) -> Result<Value> {
    let path = path(key, "votes");
    if on {
        c.request("POST", &path, &[], None)?;
    } else {
        c.delete(&path, &[])?;
    }
    Ok(json!({"issueId": key, "voted": on}))
}

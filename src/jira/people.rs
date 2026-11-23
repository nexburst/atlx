//! Issue watchers and votes.

use super::{enc, resolve_account};
use crate::client::Client;
use anyhow::Result;
use serde_json::{Value, json};

fn path(key: &str, what: &str) -> String {
    format!("/rest/api/3/issue/{}/{what}", enc(key))
}

/// The users watching an issue.
pub fn watchers(c: &Client, key: &str) -> Result<Vec<Value>> {
    let v = c.get(&path(key, "watchers"), &[])?;
    Ok(v["watchers"].as_array().cloned().unwrap_or_default())
}

/// `who` is `me`, an account ID or ARI, or an email or name to look up.
pub fn watcher_add(c: &Client, key: &str, who: &str) -> Result<Value> {
    let account = resolve_account(c, who)?;
    // The body is the bare account ID as a JSON string.
    c.post(&path(key, "watchers"), &Value::from(account.as_str()))?;
    Ok(json!({"accountId": account}))
}

pub fn watcher_remove(c: &Client, key: &str, who: &str) -> Result<Value> {
    let account = resolve_account(c, who)?;
    // Without an account Jira would pick the caller, so never send it empty.
    if account.is_empty() {
        anyhow::bail!("no account ID for '{who}'");
    }
    c.delete(&path(key, "watchers"), &[("accountId", account.as_str())])?;
    Ok(json!({"id": account, "issue": key, "deleted": true}))
}

pub fn votes(c: &Client, key: &str) -> Result<Value> {
    c.get(&path(key, "votes"), &[])
}

/// Votes only exist for the calling user.
pub fn vote(c: &Client, key: &str) -> Result<Value> {
    c.request("POST", &path(key, "votes"), &[], None)?;
    Ok(json!({"hasVoted": true}))
}

pub fn unvote(c: &Client, key: &str) -> Result<Value> {
    c.delete(&path(key, "votes"), &[])?;
    Ok(json!({"id": key, "issue": key, "deleted": true}))
}

//! `jira issue worklog list|get|create|update|delete`.

use super::{Listed, Offsets, enc, offset_list, strip_media};
use crate::client::{ApiError, Client};
use crate::paging::{self, Page, Position};
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Map, Value, json};

/// `--adjust-estimate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Adjust {
    Auto,
    Leave,
    New,
    Manual,
}

impl Adjust {
    fn as_str(self) -> &'static str {
        match self {
            Adjust::Auto => "auto",
            Adjust::Leave => "leave",
            Adjust::New => "new",
            Adjust::Manual => "manual",
        }
    }
}

/// The query options that the worklog writes share.
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOpts<'a> {
    pub adjust: Option<Adjust>,
    pub new_estimate: Option<&'a str>,
    /// `--reduce-by` (add, update).
    pub reduce_by: Option<&'a str>,
    /// `--increase-by` (delete).
    pub increase_by: Option<&'a str>,
    pub notify_users: Option<bool>,
    pub override_editable: bool,
    pub expand: Option<&'a str>,
}

impl<'a> WriteOpts<'a> {
    fn query(&self) -> Result<Vec<(&'static str, &'a str)>> {
        if self.adjust == Some(Adjust::New) && self.new_estimate.is_none() {
            return Err(ApiError::usage("--adjust-estimate new needs --new-estimate").into());
        }
        if self.adjust == Some(Adjust::Manual)
            && self.reduce_by.is_none()
            && self.increase_by.is_none()
        {
            return Err(ApiError::usage(
                "--adjust-estimate manual needs --reduce-by (add, update) or --increase-by (delete)",
            )
            .into());
        }
        let notify = self.notify_users.map(|b| if b { "true" } else { "false" });
        Ok([
            ("adjustEstimate", self.adjust.map(Adjust::as_str)),
            ("newEstimate", self.new_estimate),
            ("reduceBy", self.reduce_by),
            ("increaseBy", self.increase_by),
            ("notifyUsers", notify),
            (
                "overrideEditableFlag",
                self.override_editable.then_some("true"),
            ),
            ("expand", self.expand),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect())
    }
}

/// The body of an add or update; absent fields are left out.
#[derive(Debug, Default)]
pub struct Entry<'a> {
    pub time_spent: Option<&'a str>,
    pub seconds: Option<u64>,
    /// Epoch milliseconds.
    pub started: Option<i64>,
    /// Already converted to ADF.
    pub comment: Option<Value>,
    pub visibility: Option<Value>,
    pub properties: Option<Value>,
}

impl Entry<'_> {
    fn body(&self) -> Value {
        let mut m = Map::new();
        if let Some(t) = self.time_spent {
            m.insert("timeSpent".into(), t.into());
        }
        if let Some(s) = self.seconds {
            m.insert("timeSpentSeconds".into(), s.into());
        }
        if let Some(ms) = self.started {
            m.insert("started".into(), crate::time::jira_timestamp(ms).into());
        }
        if let Some(c) = &self.comment {
            m.insert("comment".into(), c.clone());
        }
        if let Some(v) = &self.visibility {
            m.insert("visibility".into(), v.clone());
        }
        if let Some(v) = &self.properties {
            m.insert("properties".into(), v.clone());
        }
        Value::Object(m)
    }
}

fn path(key: &str) -> String {
    format!("/rest/api/3/issue/{}/worklog", enc(key))
}

fn item(key: &str, id: &str) -> String {
    format!("{}/{}", path(key), enc(id))
}

fn stripped(mut v: Value) -> Value {
    strip_media(&mut v);
    v
}

pub fn add(c: &Client, key: &str, e: &Entry, o: &WriteOpts) -> Result<Value> {
    if e.time_spent.is_none() && e.seconds.is_none() {
        return Err(ApiError::usage("pass --time-spent or --time-spent-seconds").into());
    }
    let q = o.query()?;
    Ok(stripped(c.request(
        "POST",
        &path(key),
        &q,
        Some(&e.body()),
    )?))
}

pub fn update(c: &Client, key: &str, id: &str, e: &Entry, o: &WriteOpts) -> Result<Value> {
    let body = e.body();
    if body.as_object().is_some_and(Map::is_empty) {
        return Err(ApiError::usage(
            "nothing to update: pass --time-spent, --started, --comment, --visibility-json or --properties-json",
        )
        .into());
    }
    let q = o.query()?;
    Ok(stripped(c.request(
        "PUT",
        &item(key, id),
        &q,
        Some(&body),
    )?))
}

pub fn delete(c: &Client, key: &str, id: &str, o: &WriteOpts) -> Result<Value> {
    c.delete(&item(key, id), &o.query()?)?;
    Ok(json!({"id": id, "issue": key, "deleted": true}))
}

pub fn get(c: &Client, key: &str, id: &str, expand: Option<&str>) -> Result<Value> {
    let q: Vec<(&str, &str)> = expand.iter().map(|e| ("expand", *e)).collect();
    Ok(stripped(c.get(&item(key, id), &q)?))
}

/// An issue's worklogs, with optional bounds on `started` in epoch ms.
pub fn list(
    c: &Client,
    key: &str,
    after: Option<i64>,
    before: Option<i64>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let (after, before) = (after.map(|v| v.to_string()), before.map(|v| v.to_string()));
    let q: Vec<(&str, &str)> = [
        ("startedAfter", after.as_deref()),
        ("startedBefore", before.as_deref()),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .collect();
    offset_list(
        c,
        &path(key),
        &q,
        &[],
        Offsets::values().list("worklogs").page(1000),
        limit,
        cursor,
    )
}

/// Which change feed to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Changed,
    Deleted,
}

/// Site-wide IDs of worklogs changed or deleted since `since` (epoch ms).
/// Jira pages the feed by time: the next cursor carries the page's `until`.
pub fn feed(
    c: &Client,
    feed: Feed,
    since: i64,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let segment = match feed {
        Feed::Changed => "updated",
        Feed::Deleted => "deleted",
    };
    let path = format!("/rest/api/3/worklog/{segment}");
    paging::collect(limit, cursor, |pos, _| {
        let from = match pos {
            Position::Start => since.to_string(),
            Position::Token(t) => t.clone(),
            _ => return Err(ApiError::usage("--cursor is not from this list").into()),
        };
        let v = c.get(&path, &[("since", from.as_str())])?;
        let items = v["values"]
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow!("Jira answered {path} without a list"))?;
        let next = match (v["lastPage"].as_bool(), &v["until"]) {
            (Some(false), Value::Number(n)) => Some(Position::Token(n.to_string())),
            _ => None,
        };
        Ok(Page { items, next })
    })
}

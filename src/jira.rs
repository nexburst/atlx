//! Jira Cloud REST v3 and Agile 1.0.
//!
//! API functions return REST objects as Jira sends them (lists as a
//! [`Listed`] of items plus the next cursor); the command line projects them
//! onto the records in [`records`].

use crate::cache::{Cache, Table};
use crate::client::{ApiError, Client, ErrorKind};
use crate::markup::{Body, BodyError, BodyFormat, parse_body, quote, unrepresentable};
use crate::paging::{self, Page, Position};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Map, Value, json};

pub mod admin;
pub mod agile;
pub mod attachments;
pub mod bulk;
pub mod dashboards;
pub mod people;
pub mod records;
pub mod worklogs;

/// One page of a list: the REST objects and the cursor of what follows
/// (`None` when there is no more).
pub type Listed = (Vec<Value>, Option<String>);

pub(crate) fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn browse_url(c: &Client, key: &str) -> String {
    format!("{}/browse/{key}", c.base())
}

/// Split a comma-separated list flag (`--label a,b`).
pub fn csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

/// Merge `--fields-json` and repeated `--field KEY=VALUE` (values parse as
/// JSON when valid, else stay strings). The same key twice is an error.
pub fn extra_fields(fields_json: Option<&str>, field: &[String]) -> Result<Map<String, Value>> {
    let mut out = match fields_json {
        Some(j) => match serde_json::from_str::<Value>(j)
            .map_err(|e| usage(format!("--fields-json is not valid JSON: {e}")))?
        {
            Value::Object(m) => m,
            _ => return Err(usage("--fields-json must be a JSON object")),
        },
        None => Map::new(),
    };
    for kv in field {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| usage(format!("--field '{kv}' must be NAME=VALUE")))?;
        let k = k.trim();
        if out.contains_key(k) {
            return Err(usage(format!(
                "field '{k}' given more than once (with --field or --fields-json)"
            )));
        }
        let v = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        out.insert(k.to_string(), v);
    }
    Ok(out)
}

/// Whether a `--field` key is a field ID rather than a display name.
fn is_field_id(k: &str) -> bool {
    k.starts_with("customfield_")
        || (k.starts_with(|c: char| c.is_ascii_lowercase())
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// The site's field list as `[{id, name}]`, from the metadata cache.
fn field_table(c: &Client) -> impl Fn() -> Result<Value> + '_ {
    move || {
        let v = c.get("/rest/api/3/field", &[])?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(|f| json!({"id": f["id"], "name": f["name"]}))
            .collect())
    }
}

/// Field display names (`Story Points`) in a field map replaced by their
/// IDs (`customfield_10016`). IDs pass through without a request; names are
/// looked up in the cached field list, which is refetched once on a miss.
pub fn resolve_field_names(
    c: &Client,
    cache: &Cache,
    fields: Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in fields {
        if is_field_id(&k) {
            out.insert(k, v);
            continue;
        }
        let find = |table: &Value| -> Option<Vec<String>> {
            let hits: Vec<String> = table
                .as_array()?
                .iter()
                .filter(|f| {
                    f["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&k))
                })
                .filter_map(|f| f["id"].as_str().map(String::from))
                .collect();
            (!hits.is_empty()).then_some(hits)
        };
        let hits = cache.lookup(c.base(), Table::JiraFields, field_table(c), find)?;
        match hits.as_deref() {
            Some([id]) => {
                out.insert(id.clone(), v);
            }
            Some(many) => {
                return Err(ApiError::usage(format!(
                    "several fields are named '{k}': {}; pass the field ID",
                    many.join(", ")
                ))
                .into());
            }
            None => {
                return Err(ApiError::usage(format!("no field is named '{k}'"))
                    .with_hint("list fields with `atlx jira field list -q NAME`")
                    .into());
            }
        }
    }
    Ok(out)
}

/// The host name of the client's site.
pub fn site(c: &Client) -> String {
    let b = c.base();
    b.split_once("://")
        .map_or(b, |(_, h)| h)
        .trim_end_matches('/')
        .to_string()
}

/// Remove avatar and icon URLs at any depth.
pub fn strip_media(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.shift_remove("avatarUrls");
            m.shift_remove("iconUrl");
            m.values_mut().for_each(strip_media);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_media),
        _ => {}
    }
}

/// An ID as text, whether REST sent a string or a number.
pub(crate) fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// --- paging ------------------------------------------------------------------

/// How an offset-paged endpoint names its parameters.
#[derive(Clone, Copy, Debug)]
pub struct Offsets {
    /// The list key in the response (`values`, `comments`); `None` when the
    /// response is the list itself.
    pub list: Option<&'static str>,
    pub start: &'static str,
    pub size: &'static str,
    /// The largest page the endpoint serves.
    pub max_page: usize,
}

impl Offsets {
    pub const fn values() -> Self {
        Self {
            list: Some("values"),
            start: "startAt",
            size: "maxResults",
            max_page: 50,
        }
    }

    pub const fn list(mut self, key: &'static str) -> Self {
        self.list = Some(key);
        self
    }

    pub const fn bare(mut self) -> Self {
        self.list = None;
        self
    }

    pub const fn page(mut self, n: usize) -> Self {
        self.max_page = n;
        self
    }

    /// Jira Service Management: `start` and `limit`.
    pub const fn servicedesk() -> Self {
        Self {
            list: Some("values"),
            start: "start",
            size: "limit",
            max_page: 50,
        }
    }
}

fn cursor_mismatch() -> anyhow::Error {
    usage("--cursor is not from this list; pass the `next` of a previous run of the same command")
}

/// Read up to `limit` items of an offset-paged list, from `cursor`.
pub fn offset_list(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    headers: &[(&str, &str)],
    o: Offsets,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    paging::collect(limit, cursor, |pos, wanted| {
        let start = match pos {
            Position::Start => 0,
            Position::Offset(n) => *n,
            _ => return Err(cursor_mismatch()),
        };
        let size = wanted.clamp(1, o.max_page);
        let (s, m) = (start.to_string(), size.to_string());
        let mut q = query.to_vec();
        q.push((o.start, &s));
        q.push((o.size, &m));
        let v = c.request_with("GET", path, &q, None, headers)?;
        let items = match o.list {
            Some(k) => v[k].as_array(),
            None => v.as_array(),
        }
        .cloned()
        .ok_or_else(|| anyhow!("Jira answered {path} without a list"))?;
        let end = start + items.len() as u64;
        let last = v["isLast"].as_bool().or(v["isLastPage"].as_bool());
        let more = match (last, v["total"].as_u64()) {
            (Some(last), _) => !last && !items.is_empty(),
            (None, Some(t)) => end < t,
            (None, None) => !items.is_empty() && items.len() >= size,
        };
        Ok(Page {
            items,
            next: more.then_some(Position::Offset(end)),
        })
    })
}

/// Page through a list already in memory.
pub fn local_list(items: Vec<Value>, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    paging::collect(limit, cursor, |pos, _| {
        let start = match pos {
            Position::Start => 0,
            Position::Offset(n) => *n as usize,
            _ => return Err(cursor_mismatch()),
        };
        Ok(Page {
            items: items.iter().skip(start).cloned().collect(),
            next: None,
        })
    })
}

/// Issues from an endpoint that pages with `nextPageToken`.
fn token_list(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    paging::collect(limit, cursor, |pos, wanted| {
        let token = match pos {
            Position::Start => None,
            Position::Token(t) => Some(t.as_str()),
            _ => return Err(cursor_mismatch()),
        };
        let size = wanted.clamp(1, 100).to_string();
        let mut q = query.to_vec();
        q.push(("maxResults", &size));
        if let Some(t) = token {
            q.push(("nextPageToken", t));
        }
        let v = c.get(path, &q)?;
        let items = v["issues"]
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow!("Jira answered {path} without an issue list"))?;
        let next = v["nextPageToken"]
            .as_str()
            .filter(|t| !t.is_empty() && v["isLast"] != true)
            .map(|t| Position::Token(t.to_string()));
        Ok(Page { items, next })
    })
}

// --- bodies ------------------------------------------------------------------

/// A rich-text input as ADF. Jira stores ADF, so storage format is a usage
/// error; Markdown that would lose anything is refused.
pub fn adf_body(text: &str, f: BodyFormat) -> Result<Value> {
    if f == BodyFormat::Storage {
        return Err(ApiError::usage(
            "Jira bodies are markdown, adf or plain; storage is Confluence's format",
        )
        .into());
    }
    match parse_body(text, f)? {
        Body::Adf(v) => Ok(v),
        Body::Storage(_) => Err(usage("Jira bodies are markdown, adf or plain")),
    }
}

/// Before a whole rich-text field is replaced from Markdown or plain text:
/// refuse when the current value has content Markdown cannot express, since
/// the replace would destroy it. ADF input replaces as given.
pub fn guard_replace(current: &Value, f: BodyFormat, what: &str, allow_lossy: bool) -> Result<()> {
    if f == BodyFormat::Adf || allow_lossy || current.is_null() {
        return Ok(());
    }
    let losses = unrepresentable(current);
    if losses.is_empty() {
        return Ok(());
    }
    Err(BodyError::refused(
        format!("{what} has content Markdown cannot express; replacing it with Markdown would destroy it"),
        losses,
        "read it with --body-format adf, change the JSON and write it back with --body-format adf (or pass --allow-lossy --yes to replace it anyway)",
    )
    .into())
}

// --- issues: reads -----------------------------------------------------------

/// An issue key from a key, numeric ID, ARI, or `/browse/KEY` URL.
pub fn issue_ref(s: &str) -> String {
    let s = s.trim();
    if let Some(i) = s.find("/browse/") {
        return s[i + 8..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string();
    }
    if s.starts_with("ari:") {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    s.to_string()
}

/// One issue as REST returns it, with the fields [`records::ISSUE`] reads and
/// `names` (field display names) expanded.
pub fn issue(c: &Client, key: &str) -> Result<Value> {
    c.get(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[
            ("fields", &records::GET_FIELDS.join(",")),
            ("expand", "names"),
        ],
    )
}

/// `bulkfetch` takes 100 issues per request.
const FETCH_BATCH: usize = 100;

/// Several issues in as few requests as possible (`bulkfetch`, a read sent
/// as POST). Returns the issues in input order and the inputs Jira did not
/// return. Jira answers for a moved issue under its new key, so an
/// unmatched issue is given to the only unmatched input when there is one.
pub fn issues(c: &Client, keys: &[String]) -> Result<(Vec<Value>, Vec<String>)> {
    let mut found: Vec<Value> = Vec::new();
    for chunk in keys.chunks(FETCH_BATCH) {
        let body = json!({
            "issueIdsOrKeys": chunk,
            "fields": records::GET_FIELDS,
            "expand": ["names"],
        });
        let v = c.post_read("/rest/api/3/issue/bulkfetch", &body)?;
        let names = v["names"].clone();
        for mut i in v["issues"].as_array().cloned().unwrap_or_default() {
            if !names.is_null() && i.get("names").is_none() {
                i["names"] = names.clone();
            }
            found.push(i);
        }
    }
    let matches = |i: &Value, k: &str| {
        [&i["key"], &i["id"]]
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(k)))
    };
    let mut slots: Vec<Option<Value>> = vec![None; keys.len()];
    let mut rest = Vec::new();
    for i in found {
        match keys
            .iter()
            .enumerate()
            .find(|(n, k)| slots[*n].is_none() && matches(&i, k))
        {
            Some((n, _)) => slots[n] = Some(i),
            None => rest.push(i),
        }
    }
    let open: Vec<usize> = (0..keys.len()).filter(|n| slots[*n].is_none()).collect();
    if open.len() == 1 && rest.len() == 1 {
        slots[open[0]] = rest.pop();
    }
    let mut missing = Vec::new();
    let mut out = Vec::new();
    for (k, s) in keys.iter().zip(slots) {
        match s {
            Some(i) => out.push(i),
            None => missing.push(k.clone()),
        }
    }
    out.extend(rest);
    Ok((out, missing))
}

/// Issues matching JQL, as REST returns them with the fields of
/// [`records::ISSUE_ROW`].
pub fn issue_list(c: &Client, jql: &str, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    let fields = records::ROW_FIELDS.join(",");
    token_list(
        c,
        "/rest/api/3/search/jql",
        &[("jql", jql), ("fields", &fields)],
        limit,
        cursor,
    )
}

/// The JQL `issue search` runs for free text.
pub fn search_jql(text: &str, project: Option<&str>) -> String {
    match project {
        Some(p) => format!(
            "project = {} AND text ~ {} ORDER BY updated DESC",
            quote(p),
            quote(text)
        ),
        None => format!("text ~ {} ORDER BY updated DESC", quote(text)),
    }
}

/// Structured `issue list` filters combined into JQL.
#[derive(Debug, Default)]
pub struct IssueFilter<'a> {
    pub jql: Option<&'a str>,
    pub project: Option<&'a str>,
    pub status: &'a [String],
    pub assignee: Option<&'a str>,
    pub issue_type: &'a [String],
    pub label: &'a [String],
    pub order_by: Option<&'a str>,
}

fn jql_list(field: &str, values: &[String]) -> Option<String> {
    match values {
        [] => None,
        [one] => Some(format!("{field} = {}", quote(one))),
        many => Some(format!(
            "{field} in ({})",
            many.iter().map(|v| quote(v)).collect::<Vec<_>>().join(", ")
        )),
    }
}

impl IssueFilter<'_> {
    /// The JQL, or `None` when nothing narrows the search.
    pub fn jql(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        let (base, order) = match self.jql.map(str::trim).filter(|j| !j.is_empty()) {
            Some(j) => split_order(j),
            None => (String::new(), String::new()),
        };
        if !base.trim().is_empty() {
            parts.push(format!("({})", base.trim()));
        }
        if let Some(p) = self.project {
            parts.push(format!("project = {}", quote(p)));
        }
        parts.extend(jql_list("status", self.status));
        parts.extend(jql_list("issuetype", self.issue_type));
        parts.extend(jql_list("labels", self.label));
        if let Some(a) = self.assignee {
            parts.push(match a.trim().to_ascii_lowercase().as_str() {
                "me" => "assignee = currentUser()".to_string(),
                "none" => "assignee is EMPTY".to_string(),
                _ => format!("assignee = {}", quote(a.trim())),
            });
        }
        if parts.is_empty() {
            return None;
        }
        let order = match self.order_by {
            Some(o) => format!("ORDER BY {o}"),
            None if !order.is_empty() => order,
            None => "ORDER BY updated DESC".into(),
        };
        Some(format!("{} {order}", parts.join(" AND ")))
    }
}

/// `(where, "ORDER BY ...")` of a JQL string.
fn split_order(jql: &str) -> (String, String) {
    match jql
        .as_bytes()
        .windows(8)
        .rposition(|w| w.eq_ignore_ascii_case(b"order by"))
    {
        Some(i) => (jql[..i].to_string(), jql[i..].trim().to_string()),
        None => (jql.to_string(), String::new()),
    }
}

/// Cross-product search's Jira results: compact rows for free text.
pub struct Found {
    pub data: Value,
    pub next: Option<String>,
}

/// Used by `atlx search`: the first `limit` issues matching `text`, as
/// `{"issues": [{id, key, url, summary, status, ...}]}`.
pub fn search(
    c: &Client,
    text: &str,
    limit: usize,
    _fields: Option<&[String]>,
    after: Option<&str>,
) -> Result<Found> {
    let cursor = after.map(|a| format!("t:{a}"));
    let (items, next) = issue_list(c, &search_jql(text, None), limit.max(1), cursor.as_deref())?;
    let rows: Vec<Value> = items
        .iter()
        .map(|i| {
            let key = i["key"].as_str().unwrap_or_default();
            json!({
                "id": i["id"], "key": key, "url": browse_url(c, key),
                "summary": i["fields"]["summary"],
                "status": i["fields"]["status"]["name"],
                "type": i["fields"]["issuetype"]["name"],
                "assignee": i["fields"]["assignee"]["displayName"],
                "updated": i["fields"]["updated"],
            })
        })
        .collect();
    Ok(Found {
        data: json!({"issues": rows}),
        next,
    })
}

// --- issues: writes ----------------------------------------------------------

fn id_or_name(s: &str, name_key: &str) -> Value {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        json!({"id": s})
    } else {
        json!({name_key: s})
    }
}

pub struct NewIssue<'a> {
    pub project: &'a str,
    pub issue_type: &'a str,
    pub summary: &'a str,
    /// Already converted to ADF.
    pub description: Option<Value>,
    pub assignee: Option<&'a str>,
    pub reporter: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub labels: Vec<String>,
    pub parent: Option<&'a str>,
    /// Other fields by ID.
    pub extra: Map<String, Value>,
}

/// The REST `fields` of a new issue.
pub fn new_issue_fields(c: &Client, n: NewIssue) -> Result<Map<String, Value>> {
    let mut fields = Map::new();
    fields.insert("project".into(), id_or_name(n.project, "key"));
    fields.insert("issuetype".into(), id_or_name(n.issue_type, "name"));
    fields.insert("summary".into(), n.summary.into());
    if let Some(d) = n.description {
        fields.insert("description".into(), d);
    }
    if let Some(a) = n.assignee {
        fields.insert(
            "assignee".into(),
            json!({"accountId": resolve_account(c, a)?}),
        );
    }
    if let Some(r) = n.reporter {
        fields.insert(
            "reporter".into(),
            json!({"accountId": resolve_account(c, r)?}),
        );
    }
    if let Some(p) = n.priority {
        fields.insert("priority".into(), id_or_name(p, "name"));
    }
    if !n.labels.is_empty() {
        fields.insert("labels".into(), json!(n.labels));
    }
    if let Some(p) = n.parent {
        fields.insert("parent".into(), json!({"key": issue_ref(p)}));
    }
    fields.extend(n.extra);
    Ok(fields)
}

/// `issue create`: `{id, key, url}`.
pub fn create(c: &Client, fields: Map<String, Value>) -> Result<Value> {
    let v = c.post("/rest/api/3/issue", &json!({"fields": fields}))?;
    let key = v["key"].as_str().unwrap_or_default();
    Ok(json!({"id": v["id"], "key": key, "url": browse_url(c, key)}))
}

/// Set / add / remove a multi-value field.
#[derive(Debug, Default)]
pub struct ListEdit {
    pub set: Option<Vec<String>>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Default)]
pub struct Update<'a> {
    pub summary: Option<&'a str>,
    /// Already converted to ADF (and checked with [`guard_replace`]).
    pub description: Option<Value>,
    pub assignee: Option<&'a str>,
    pub issue_type: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub labels: Option<ListEdit>,
    pub components: Option<ListEdit>,
    pub fix_versions: Option<ListEdit>,
    pub parent: Option<&'a str>,
    pub due_date: Option<&'a str>,
    pub resolution: Option<&'a str>,
    /// Other fields by ID.
    pub extra: Map<String, Value>,
    /// A sprint or backlog move, made after the field update.
    pub sprint: Option<agile::Destination>,
}

fn none_or(s: &str) -> Option<&str> {
    (!s.eq_ignore_ascii_case("none")).then_some(s)
}

/// `issue update`: one field edit, then an optional sprint move.
/// `{key, updated: [field names], url, sprint?}`.
pub fn update(c: &Client, key: &str, u: Update) -> Result<Value> {
    for (name, edit) in [
        ("labels", &u.labels),
        ("components", &u.components),
        ("fixVersions", &u.fix_versions),
    ] {
        if let Some(edit) = edit
            && edit.set.is_some()
            && (!edit.add.is_empty() || !edit.remove.is_empty())
        {
            return Err(usage(format!(
                "cannot both set and add/remove '{name}' in the same update"
            )));
        }
    }
    let mut fields = Map::new();
    let mut ops = Map::new();
    let mut changed: Vec<String> = Vec::new();
    let mut op_names: Vec<String> = Vec::new();
    let mut set = |name: &str, v: Value| {
        fields.insert(name.into(), v);
        changed.push(name.into());
    };
    if let Some(s) = u.summary {
        set("summary", s.into());
    }
    if let Some(d) = u.description {
        set("description", d);
    }
    if let Some(a) = u.assignee {
        let v = match none_or(a) {
            None => Value::Null,
            Some(a) => json!({"accountId": resolve_account(c, a)?}),
        };
        set("assignee", v);
    }
    if let Some(t) = u.issue_type {
        set("issuetype", id_or_name(t, "name"));
    }
    if let Some(p) = u.priority {
        set("priority", id_or_name(p, "name"));
    }
    if let Some(p) = u.parent {
        set(
            "parent",
            none_or(p).map_or(Value::Null, |p| json!({"key": issue_ref(p)})),
        );
    }
    if let Some(d) = u.due_date {
        set("duedate", none_or(d).map_or(Value::Null, Value::from));
    }
    if let Some(r) = u.resolution {
        set(
            "resolution",
            none_or(r).map_or(Value::Null, |r| id_or_name(r, "name")),
        );
    }
    for (name, edit, named) in [
        ("labels", u.labels, false),
        ("components", u.components, true),
        ("fixVersions", u.fix_versions, true),
    ] {
        let Some(edit) = edit else { continue };
        let wrap = |s: &String| if named { json!({"name": s}) } else { json!(s) };
        if let Some(all) = &edit.set {
            set(name, all.iter().map(wrap).collect());
        }
        let mut list: Vec<Value> = edit.add.iter().map(|x| json!({"add": wrap(x)})).collect();
        list.extend(edit.remove.iter().map(|x| json!({"remove": wrap(x)})));
        if !list.is_empty() {
            ops.insert(name.into(), list.into());
            op_names.push(name.to_string());
        }
    }
    for (k, v) in u.extra {
        set(&k, v);
    }
    changed.extend(op_names);
    if fields.is_empty() && ops.is_empty() && u.sprint.is_none() {
        return Err(usage("nothing to update; pass at least one field"));
    }
    if !fields.is_empty() || !ops.is_empty() {
        let mut body = json!({});
        if !fields.is_empty() {
            body["fields"] = fields.into();
        }
        if !ops.is_empty() {
            body["update"] = ops.into();
        }
        c.put(&format!("/rest/api/3/issue/{}", enc(key)), &body)?;
    }
    let mut out = json!({"key": key, "updated": changed, "url": browse_url(c, key)});
    if let Some(to) = u.sprint {
        agile::move_issues(c, to, &[key.to_string()])?;
        out["sprint"] = match to {
            agile::Destination::Sprint(id) => id.to_string().into(),
            agile::Destination::Backlog => "backlog".into(),
        };
    }
    Ok(out)
}

/// The issue's current description (ADF), for [`guard_replace`].
pub fn current_description(c: &Client, key: &str) -> Result<Value> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[("fields", "description")],
    )?;
    Ok(v["fields"]["description"].clone())
}

/// `issue assign`: `None` unassigns. Jira answers 204.
pub fn assign(c: &Client, key: &str, who: Option<&str>) -> Result<Value> {
    let account = who.map(|w| resolve_account(c, w)).transpose()?;
    c.put(
        &format!("/rest/api/3/issue/{}/assignee", enc(key)),
        &json!({"accountId": account}),
    )?;
    Ok(json!({"key": key, "assignee": account}))
}

/// `issue delete`
pub fn delete(c: &Client, key: &str, delete_subtasks: bool) -> Result<Value> {
    c.delete(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[(
            "deleteSubtasks",
            if delete_subtasks { "true" } else { "false" },
        )],
    )?;
    Ok(json!({"id": key, "deleted": true}))
}

// --- comments ----------------------------------------------------------------

/// Who may see a comment.
#[derive(Clone, Copy, Debug)]
pub enum Visibility<'a> {
    Role(&'a str),
    Group(&'a str),
}

/// `issue comment create`: the REST comment. `parent` makes it a reply.
pub fn comment(
    c: &Client,
    key: &str,
    body: Value,
    visibility: Option<Visibility>,
    internal: bool,
    parent: Option<&str>,
) -> Result<Value> {
    let mut req = json!({"body": body});
    if let Some(p) = parent {
        req["parentId"] = p
            .trim()
            .parse::<u64>()
            .map_or_else(|_| Value::from(p.trim()), Value::from);
    }
    match visibility {
        Some(Visibility::Role(r)) => req["visibility"] = json!({"type": "role", "value": r}),
        Some(Visibility::Group(g)) => req["visibility"] = json!({"type": "group", "value": g}),
        None => {}
    }
    if internal {
        req["properties"] = json!([{"key": "sd.public.comment", "value": {"internal": true}}]);
    }
    let mut v = c.post(&format!("/rest/api/3/issue/{}/comment", enc(key)), &req)?;
    if v.is_object() {
        v["issue"] = key.into();
    }
    Ok(v)
}

fn comment_path(key: &str, id: &str) -> String {
    format!("/rest/api/3/issue/{}/comment/{}", enc(key), enc(id))
}

/// `issue comment list`: oldest first unless `newest_first`.
pub fn comments(
    c: &Client,
    key: &str,
    newest_first: bool,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let order = if newest_first { "-created" } else { "created" };
    let (mut items, next) = offset_list(
        c,
        &format!("/rest/api/3/issue/{}/comment", enc(key)),
        &[("orderBy", order)],
        &[],
        Offsets::values().list("comments").page(100),
        limit,
        cursor,
    )?;
    for i in &mut items {
        i["issue"] = key.into();
    }
    Ok((items, next))
}

pub fn comment_get(c: &Client, key: &str, id: &str) -> Result<Value> {
    let mut v = c.get(&comment_path(key, id), &[])?;
    if v.is_object() {
        v["issue"] = key.into();
    }
    Ok(v)
}

/// `issue comment update`: the updated REST comment.
pub fn comment_update(c: &Client, key: &str, id: &str, body: Value) -> Result<Value> {
    let mut v = c.put(&comment_path(key, id), &json!({"body": body}))?;
    if v.is_object() {
        v["issue"] = key.into();
    }
    Ok(v)
}

pub fn comment_delete(c: &Client, key: &str, id: &str) -> Result<Value> {
    c.delete(&comment_path(key, id), &[])?;
    Ok(json!({"id": id, "issue": key, "deleted": true}))
}

/// `issue changelog list`: oldest first.
pub fn changelog(c: &Client, key: &str, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    offset_list(
        c,
        &format!("/rest/api/3/issue/{}/changelog", enc(key)),
        &[],
        &[],
        Offsets::values().page(100),
        limit,
        cursor,
    )
}

// --- transitions -------------------------------------------------------------

/// The transitions an issue can make now, with their fields.
pub fn transitions(c: &Client, key: &str) -> Result<Vec<Value>> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}/transitions", enc(key)),
        &[("expand", "transitions.fields")],
    )?;
    Ok(v["transitions"].as_array().cloned().unwrap_or_default())
}

/// `issue transition`: `target` matches a transition ID, transition name or
/// destination status name. A comment is sent with the transition when it
/// has a screen, else added afterwards (Jira drops it silently otherwise).
pub fn transition(
    c: &Client,
    key: &str,
    target: &str,
    fields: Map<String, Value>,
    comment_body: Option<Value>,
) -> Result<Value> {
    let list = transitions(c, key)?;
    let is = |v: &Value| {
        v.as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(target.trim()))
    };
    let Some(t) = list
        .iter()
        .find(|t| is(&t["id"]))
        .or_else(|| list.iter().find(|t| is(&t["name"]) || is(&t["to"]["name"])))
    else {
        let names: Vec<_> = list
            .iter()
            .filter_map(|t| t["to"]["name"].as_str().or(t["name"].as_str()))
            .collect();
        return Err(ApiError::usage(format!(
            "{key} has no transition to '{target}'; it can go to: {}",
            names.join(", ")
        ))
        .with_hint(format!(
            "list them with `atlx jira transition list --issue {key}`"
        ))
        .into());
    };
    let mut req = json!({"transition": {"id": t["id"]}});
    if !fields.is_empty() {
        req["fields"] = fields.into();
    }
    let has_screen = t["hasScreen"] == true;
    if let Some(body) = comment_body.clone().filter(|_| has_screen) {
        req["update"] = json!({"comment": [{"add": {"body": body}}]});
    }
    c.post(&format!("/rest/api/3/issue/{}/transitions", enc(key)), &req)?;
    let mut out = json!({
        "key": key,
        "transition": t["name"],
        "status": t["to"]["name"],
    });
    if let Some(body) = comment_body.filter(|_| !has_screen) {
        out["commentId"] = comment(c, key, body, None, false, None)?["id"].clone();
    }
    Ok(out)
}

// --- users -------------------------------------------------------------------

pub fn myself(c: &Client) -> Result<Value> {
    c.get("/rest/api/3/myself", &[])
}

pub(crate) fn fetch_users(
    c: &Client,
    query: &str,
    max_results: Option<usize>,
) -> Result<Vec<Value>> {
    let max = max_results.map(|n| n.clamp(1, 1000).to_string());
    let mut q = vec![("query", query)];
    if let Some(m) = &max {
        q.push(("maxResults", m.as_str()));
    }
    let v = c.get("/rest/api/3/user/search", &q)?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// `user search`: names and emails.
pub fn user_search(c: &Client, query: &str, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    if query.trim().is_empty() {
        return Err(usage("pass some text to search for"));
    }
    offset_list(
        c,
        "/rest/api/3/user/search",
        &[("query", query)],
        &[],
        Offsets::values().bare().page(100),
        limit,
        cursor,
    )
}

/// One user: `me`, an account ID or identity ARI, or an exact email that
/// only one visible user has.
pub fn user_get(c: &Client, who: &str) -> Result<Value> {
    let who = who.trim();
    let id = who.strip_prefix("ari:cloud:identity::user/").unwrap_or(who);
    if id.is_empty() {
        return Err(usage("the user is empty"));
    }
    if id.eq_ignore_ascii_case("me") {
        return myself(c);
    }
    if !id.contains('@') {
        return c.get("/rest/api/3/user", &[("accountId", id)]);
    }
    let users = fetch_users(c, id, None)?;
    let exact: Vec<&Value> = users
        .iter()
        .filter(|u| {
            u["emailAddress"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(id))
        })
        .collect();
    match exact.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(ApiError::new(
            ErrorKind::NotFound,
            format!("no visible user has the email {id}"),
        )
        .into()),
        _ => Err(usage(format!(
            "several users have the email {id}; pass an account ID"
        ))),
    }
}

/// `me`, a raw accountId, or a name/email that matches exactly one user.
pub(crate) fn resolve_account(c: &Client, who: &str) -> Result<String> {
    if who.eq_ignore_ascii_case("me") {
        return myself(c)?["accountId"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow!("no accountId"));
    }
    if let Some(id) = who.strip_prefix("ari:cloud:identity::user/") {
        return Ok(id.to_string());
    }
    if who.contains(':') || (who.len() >= 24 && who.chars().all(|c| c.is_ascii_alphanumeric())) {
        return Ok(who.to_string());
    }
    let users = fetch_users(c, who, None)?;
    let users: Vec<&Value> = users
        .iter()
        .filter(|u| u["accountType"] == "atlassian")
        .collect();
    match users.as_slice() {
        [u] => Ok(u["accountId"].as_str().unwrap_or_default().to_string()),
        [] => Err(usage(format!("no user matches '{who}'"))),
        many => {
            let names: Vec<_> = many
                .iter()
                .filter_map(|u| u["displayName"].as_str())
                .collect();
            Err(usage(format!(
                "'{who}' matches several users: {}; pass an account ID",
                names.join(", ")
            )))
        }
    }
}

// --- fields and screens ------------------------------------------------------

/// `field list`: the site's fields, narrowed by a name/ID substring.
pub fn fields(c: &Client, query: Option<&str>, custom_only: bool) -> Result<Vec<Value>> {
    let v = c.get("/rest/api/3/field", &[])?;
    let q = query
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter(|f| !custom_only || f["custom"] == true)
        .filter(|f| {
            let Some(q) = &q else { return true };
            let name = f["name"].as_str().unwrap_or_default().to_lowercase();
            let id = f["id"].as_str().unwrap_or_default().to_lowercase();
            name.contains(q) || id.contains(q)
        })
        .cloned()
        .collect())
}

fn paginate_createmeta(c: &Client, path: &str, key: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        let start = out.len().to_string();
        let v = c.get(path, &[("startAt", &start), ("maxResults", "50")])?;
        let page = v[key]
            .as_array()
            .or_else(|| v["values"].as_array())
            .cloned()
            .unwrap_or_default();
        let page_len = page.len();
        out.extend(page);
        let total = v["total"].as_u64();
        let is_last = v["isLast"].as_bool();
        if page_len == 0
            || is_last == Some(true)
            || total.is_some_and(|t| out.len() as u64 >= t)
            || (is_last.is_none() && total.is_none() && page_len < 50)
        {
            return Ok(out);
        }
    }
}

fn resolve_issue_type<'a>(types: &'a [Value], target: &str) -> Result<&'a Value> {
    let is = |t: &Value, k: &str| {
        t[k].as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(target))
    };
    types
        .iter()
        .find(|t| is(t, "id"))
        .or_else(|| types.iter().find(|t| is(t, "name")))
        .ok_or_else(|| {
            let names: Vec<_> = types.iter().filter_map(|t| t["name"].as_str()).collect();
            usage(format!(
                "no issue type '{target}'; available: {}",
                names.join(", ")
            ))
        })
}

/// What a `--field NAME=VALUE` value means for a field schema.
fn accepted_input(schema: &Value) -> &'static str {
    let ty = schema["type"].as_str().unwrap_or_default();
    let items = schema["items"].as_str().unwrap_or_default();
    match (ty, items) {
        ("array", "option") => "multi-option",
        ("array", "user") => "multi-user",
        ("array", "version") => "multi-version",
        ("array", "component") => "multi-component",
        ("array", "string") => "multi-text",
        ("option", _) | ("option-with-child", _) => "option",
        ("date", _) => "date",
        ("datetime", _) => "datetime",
        ("number", _) => "number",
        ("string", _) => "text",
        ("user", _) => "user",
        ("version", _) => "version",
        ("priority", _) => "option",
        _ => "value",
    }
}

/// A screen field with `acceptedInput` and an `example` argument added.
fn screen_field(id: &str, f: &Value) -> Value {
    let mut o = f.as_object().cloned().unwrap_or_default();
    o.insert("fieldId".into(), id.into());
    o.insert("acceptedInput".into(), accepted_input(&f["schema"]).into());
    let settable = f["operations"]
        .as_array()
        .is_none_or(|ops| ops.iter().any(|x| x == "set"));
    if settable {
        let name = f["name"].as_str().unwrap_or(id);
        let first = f["allowedValues"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|a| a["value"].as_str().or(a["name"].as_str()))
            .unwrap_or("VALUE");
        o.insert("example".into(), format!("--field '{name}={first}'").into());
    }
    let mut v = Value::Object(o);
    strip_media(&mut v);
    v
}

/// The fields of a project's create screen for one issue type.
pub fn create_fields(c: &Client, project: &str, issue_type: &str) -> Result<Vec<Value>> {
    let types_path = format!("/rest/api/3/issue/createmeta/{}/issuetypes", enc(project));
    let types = paginate_createmeta(c, &types_path, "issueTypes")?;
    let it = resolve_issue_type(&types, issue_type)?;
    let it_id = it["id"].as_str().unwrap_or_default();
    let raw = paginate_createmeta(c, &format!("{types_path}/{}", enc(it_id)), "fields")?;
    Ok(raw
        .iter()
        .map(|f| {
            let id = f["fieldId"]
                .as_str()
                .or(f["key"].as_str())
                .unwrap_or_default();
            screen_field(id, f)
        })
        .collect())
}

/// The fields that can be edited on an issue (REST `editmeta`).
pub fn edit_fields(c: &Client, key: &str) -> Result<Vec<Value>> {
    let v = c.get(&format!("/rest/api/3/issue/{}/editmeta", enc(key)), &[])?;
    Ok(v["fields"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, f)| f["operations"].as_array().is_some_and(|o| !o.is_empty()))
        .map(|(id, f)| screen_field(id, f))
        .collect())
}

// --- projects ----------------------------------------------------------------

/// Filters for `project list` (REST `project/search`).
#[derive(Default)]
pub struct ProjectQuery<'a> {
    pub query: Option<&'a str>,
    pub type_key: Option<&'a str>,
    pub category_id: Option<&'a str>,
    /// live, archived or deleted.
    pub status: Option<&'a str>,
    pub order_by: Option<&'a str>,
}

pub fn projects(
    c: &Client,
    o: &ProjectQuery,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let mut q = Vec::new();
    if let Some(s) = o.query.map(str::trim).filter(|s| !s.is_empty()) {
        q.push(("query", s));
    }
    for (k, v) in [
        ("typeKey", o.type_key),
        ("categoryId", o.category_id),
        ("status", o.status),
        ("orderBy", Some(o.order_by.unwrap_or("key"))),
        ("expand", Some("description,lead")),
    ] {
        if let Some(v) = v {
            q.push((k, v));
        }
    }
    offset_list(
        c,
        "/rest/api/3/project/search",
        &q,
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

/// A project key or ID from a key, ID, ARI, or a `/projects/KEY` or
/// `/browse/KEY-1` URL.
pub fn project_ref(s: &str) -> String {
    let s = s.trim();
    for marker in ["/projects/", "/browse/"] {
        if let Some(i) = s.find(marker) {
            let seg = s[i + marker.len()..]
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            let seg = if marker == "/browse/" {
                seg.rsplit_once('-').map_or(seg, |(k, _)| k)
            } else {
                seg
            };
            if !seg.is_empty() {
                return seg.to_string();
            }
        }
    }
    if s.starts_with("ari:") {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    s.to_string()
}

pub fn project(c: &Client, key_or_id: &str) -> Result<Value> {
    c.get(
        &format!("/rest/api/3/project/{}", enc(&project_ref(key_or_id))),
        &[("expand", "description,lead,issueTypes")],
    )
}

/// A project's key and numeric ID, from its key or ID.
pub fn project_key_id(c: &Client, key_or_id: &str) -> Result<(String, String)> {
    let v = c.get(&format!("/rest/api/3/project/{}", enc(key_or_id)), &[])?;
    let id = id_text(&v["id"]).ok_or_else(|| anyhow!("project {key_or_id} has no ID"))?;
    let key = v["key"].as_str().unwrap_or(key_or_id).to_string();
    Ok((key, id))
}

/// The project key of an issue.
pub fn issue_project(c: &Client, issue: &str) -> Result<String> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}", enc(issue)),
        &[("fields", "project")],
    )?;
    v["fields"]["project"]["key"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("issue {issue} has no project"))
}

// --- issue links -------------------------------------------------------------

pub fn link_types(c: &Client) -> Result<Vec<Value>> {
    let v = c.get("/rest/api/3/issueLinkType", &[])?;
    Ok(v["issueLinkTypes"].as_array().cloned().unwrap_or_default())
}

fn link_payload(source: &str, target: &str, link_type: &str, comment: Option<Value>) -> Value {
    // The source reads with the type's outward phrase ("P-1 blocks P-2");
    // REST names that side `inwardIssue`.
    let mut req = json!({
        "type": id_or_name(link_type, "name"),
        "inwardIssue": id_or_name(source, "key"),
        "outwardIssue": id_or_name(target, "key"),
    });
    if let Some(body) = comment {
        req["comment"] = json!({"body": body});
    }
    req
}

/// `issue link create`: link `source` to each target, in order.
pub fn link_create(
    c: &Client,
    source: &str,
    targets: &[String],
    link_type: &str,
    comment: Option<Value>,
) -> Result<Vec<Value>> {
    if targets.is_empty() {
        return Err(usage("pass at least one --target"));
    }
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        c.post(
            "/rest/api/3/issueLink",
            &link_payload(source, t, link_type, comment.clone()),
        )?;
        out.push(json!({"source": source, "target": t, "type": link_type}));
    }
    Ok(out)
}

/// `issue link list`: the links in the issue's `issuelinks` field, seen from
/// the issue.
pub fn links(c: &Client, key: &str) -> Result<Vec<Value>> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[("fields", "issuelinks")],
    )?;
    Ok(v["fields"]["issuelinks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(records::link_row)
        .collect())
}

pub fn link_delete(c: &Client, link_id: &str) -> Result<Value> {
    c.delete(&format!("/rest/api/3/issueLink/{}", enc(link_id)), &[])?;
    Ok(json!({"id": link_id, "deleted": true}))
}

/// A remote web link.
#[derive(Default)]
pub struct Weblink<'a> {
    pub url: &'a str,
    pub title: &'a str,
    pub relationship: Option<&'a str>,
    pub global_id: Option<&'a str>,
    pub application_name: Option<&'a str>,
    pub application_type: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub icon_url: Option<&'a str>,
    pub resolved: bool,
}

fn weblink_payload(w: &Weblink) -> Value {
    let mut req = json!({"object": {"url": w.url, "title": w.title}});
    if let Some(rel) = w.relationship {
        req["relationship"] = rel.into();
    }
    if let Some(g) = w.global_id {
        req["globalId"] = g.into();
    }
    if w.application_name.is_some() || w.application_type.is_some() {
        let mut app = json!({});
        if let Some(n) = w.application_name {
            app["name"] = n.into();
        }
        if let Some(t) = w.application_type {
            app["type"] = t.into();
        }
        req["application"] = app;
    }
    if let Some(s) = w.summary {
        req["object"]["summary"] = s.into();
    }
    if let Some(i) = w.icon_url {
        req["object"]["icon"] = json!({"url16x16": i});
    }
    if w.resolved {
        req["object"]["status"] = json!({"resolved": true});
    }
    req
}

fn weblinks_path(key: &str) -> String {
    format!("/rest/api/3/issue/{}/remotelink", enc(key))
}

/// `issue weblink create`: the new link, as `weblink list` shows it.
pub fn weblink_create(c: &Client, key: &str, w: &Weblink) -> Result<Value> {
    let req = weblink_payload(w);
    let v = c.post(&weblinks_path(key), &req)?;
    let mut out = req;
    out["id"] = v["id"].clone();
    Ok(out)
}

pub fn weblinks(c: &Client, key: &str) -> Result<Vec<Value>> {
    let v = c.get(&weblinks_path(key), &[])?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Which remote link `issue weblink delete` removes.
#[derive(Clone, Copy, Debug)]
pub enum WeblinkRef<'a> {
    Id(&'a str),
    GlobalId(&'a str),
    /// The one link whose `object.url` is this URL.
    Url(&'a str),
}

pub fn weblink_delete(c: &Client, key: &str, r: WeblinkRef) -> Result<Value> {
    let base = weblinks_path(key);
    let id = match r {
        WeblinkRef::Id(id) => {
            c.delete(&format!("{base}/{}", enc(id)), &[])?;
            id.to_string()
        }
        WeblinkRef::GlobalId(g) => {
            c.delete(&base, &[("globalId", g)])?;
            g.to_string()
        }
        WeblinkRef::Url(url) => {
            let want = url.trim_end_matches('/');
            let all = weblinks(c, key)?;
            let hits: Vec<&Value> = all
                .iter()
                .filter(|l| {
                    l["object"]["url"]
                        .as_str()
                        .is_some_and(|u| u.trim_end_matches('/') == want)
                })
                .collect();
            let id = match hits.as_slice() {
                [l] => id_text(&l["id"])
                    .ok_or_else(|| anyhow!("the remote link for {url} has no ID"))?,
                [] => {
                    return Err(ApiError::new(
                        ErrorKind::NotFound,
                        format!("{key} has no remote link to {url}"),
                    )
                    .into());
                }
                many => {
                    let ids: Vec<String> = many.iter().filter_map(|l| id_text(&l["id"])).collect();
                    return Err(usage(format!(
                        "{key} has several remote links to {url} (IDs {}); pass the link ID",
                        ids.join(", ")
                    )));
                }
            };
            c.delete(&format!("{base}/{}", enc(&id)), &[])?;
            id
        }
    };
    Ok(json!({"id": id, "issue": key, "deleted": true}))
}

// --- issue types, statuses, priorities ---------------------------------------

/// The numeric part of an ID or ARI (`ari:...:issue-type/10001`).
pub fn entity_id(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("ari:") {
        return trailing_number(s).unwrap_or_else(|| s.to_string());
    }
    s.to_string()
}

/// `issue-type list`: every issue type, or a project's (at one hierarchy
/// level).
pub fn issue_types(c: &Client, project: Option<&str>, level: Option<i64>) -> Result<Vec<Value>> {
    let Some(p) = project else {
        let v = c.get("/rest/api/3/issuetype", &[])?;
        return Ok(v.as_array().cloned().unwrap_or_default());
    };
    let (_, id) = project_key_id(c, &project_ref(p))?;
    let level = level.map(|l| l.to_string());
    let mut q = vec![("projectId", id.as_str())];
    if let Some(l) = &level {
        q.push(("level", l));
    }
    let v = c.get("/rest/api/3/issuetype/project", &q)?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

pub fn issue_type(c: &Client, id: &str) -> Result<Value> {
    c.get(
        &format!("/rest/api/3/issuetype/{}", enc(&entity_id(id))),
        &[],
    )
}

/// `status list`: every status, or each status a project uses once (Jira
/// lists them per issue type).
pub fn statuses(c: &Client, project: Option<&str>) -> Result<Vec<Value>> {
    let Some(p) = project else {
        let v = c.get("/rest/api/3/status", &[])?;
        return Ok(v.as_array().cloned().unwrap_or_default());
    };
    let v = c.get(
        &format!("/rest/api/3/project/{}/statuses", enc(&project_ref(p))),
        &[],
    )?;
    let mut seen = std::collections::HashSet::new();
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .flat_map(|kind| kind["statuses"].as_array().into_iter().flatten())
        .filter(|s| seen.insert(s["id"].to_string()))
        .cloned()
        .collect())
}

pub fn status(c: &Client, id: &str) -> Result<Value> {
    c.get(&format!("/rest/api/3/status/{}", enc(&entity_id(id))), &[])
}

pub fn priorities(c: &Client, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    offset_list(
        c,
        "/rest/api/3/priority/search",
        &[],
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

pub fn priority(c: &Client, id: &str) -> Result<Value> {
    c.get(
        &format!("/rest/api/3/priority/{}", enc(&entity_id(id))),
        &[],
    )
}

// --- agile: boards & sprints -------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BoardType {
    Scrum,
    Kanban,
}

impl BoardType {
    fn as_str(self) -> &'static str {
        match self {
            BoardType::Scrum => "scrum",
            BoardType::Kanban => "kanban",
        }
    }
}

/// Filters for `board list` (Agile REST `board`).
#[derive(Default)]
pub struct BoardQuery<'a> {
    /// Project key or ID.
    pub project: Option<&'a str>,
    pub board_type: Option<BoardType>,
    /// Board name substring.
    pub name: Option<&'a str>,
    pub filter_id: Option<&'a str>,
}

impl BoardQuery<'_> {
    fn query(&self) -> Vec<(&str, &str)> {
        let mut q = Vec::new();
        if let Some(p) = self.project {
            q.push(("projectKeyOrId", p));
        }
        if let Some(t) = self.board_type {
            q.push(("type", t.as_str()));
        }
        if let Some(n) = self.name {
            q.push(("name", n));
        }
        if let Some(f) = self.filter_id {
            q.push(("filterId", f));
        }
        q
    }
}

pub fn boards(c: &Client, o: &BoardQuery, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    offset_list(
        c,
        "/rest/agile/1.0/board",
        &o.query(),
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

/// Every board matching `o` (up to the paging cap), for name lookups.
fn all_boards(c: &Client, o: &BoardQuery) -> Result<Vec<Value>> {
    Ok(boards(c, o, paging::HARD_CAP, None)?.0)
}

/// A Jira filter ID from an ID, a `filter=N` URL or a filter ARI.
pub fn filter_ref(s: &str) -> String {
    let s = s.trim();
    if let Some(i) = s.find("filter=") {
        let id: String = s[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !id.is_empty() {
            return id;
        }
    }
    trailing_number(s).unwrap_or_else(|| s.to_string())
}

/// The last run of digits after a `/` (ARIs, `/boards/12` URLs).
pub(crate) fn trailing_number(s: &str) -> Option<String> {
    s.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .map(String::from)
}

/// A board ID from an ID, an ARI, a board URL (`/boards/12`,
/// `rapidView=12`), or an exact board name.
pub fn resolve_board(c: &Client, r: &str) -> Result<u64> {
    let r = r.trim();
    if let Ok(n) = r.parse() {
        return Ok(n);
    }
    for marker in ["rapidView=", "/boards/"] {
        if let Some(i) = r.find(marker) {
            let id: String = r[i + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(n) = id.parse() {
                return Ok(n);
            }
        }
    }
    if r.starts_with("ari:")
        && let Some(n) = trailing_number(r).and_then(|n| n.parse().ok())
    {
        return Ok(n);
    }
    let found = all_boards(
        c,
        &BoardQuery {
            name: Some(r),
            ..Default::default()
        },
    )?;
    let exact: Vec<&Value> = found
        .iter()
        .filter(|b| {
            b["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(r))
        })
        .collect();
    match exact.as_slice() {
        [b] => b["id"]
            .as_u64()
            .ok_or_else(|| anyhow!("board '{r}' has no ID")),
        [] => Err(usage(format!("no board named '{r}'"))),
        _ => Err(usage(format!(
            "several boards are named '{r}'; pass the board ID"
        ))),
    }
}

/// The only board of a project.
pub fn project_board(c: &Client, project: &str) -> Result<u64> {
    let found = all_boards(
        c,
        &BoardQuery {
            project: Some(project),
            ..Default::default()
        },
    )?;
    match found.as_slice() {
        [b] => b["id"].as_u64().ok_or_else(|| anyhow!("board has no ID")),
        [] => Err(usage(format!("project {project} has no boards"))),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|b| format!("{} ({})", b["id"], b["name"].as_str().unwrap_or_default()))
                .collect();
            Err(usage(format!(
                "project {project} has several boards: {}; pass --board",
                list.join(", ")
            )))
        }
    }
}

pub fn normalize_sprint_states(s: &str) -> Result<String> {
    let states = csv(s);
    if states.is_empty() {
        return Err(usage(
            "--state cannot be empty; expected active, future, or closed",
        ));
    }
    let mut out = Vec::with_capacity(states.len());
    for st in states {
        let lower = st.to_lowercase();
        match lower.as_str() {
            "active" | "future" | "closed" => out.push(lower),
            _ => {
                return Err(usage(format!(
                    "invalid sprint state '{st}'; expected active, future, or closed"
                )));
            }
        }
    }
    Ok(out.join(","))
}

/// `sprint list`: a board's sprints.
pub fn sprints(
    c: &Client,
    board_id: u64,
    state: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let state = state.map(normalize_sprint_states).transpose()?;
    let q: Vec<(&str, &str)> = state.iter().map(|s| ("state", s.as_str())).collect();
    offset_list(
        c,
        &format!("/rest/agile/1.0/board/{board_id}/sprint"),
        &q,
        &[],
        Offsets::values(),
        limit,
        cursor,
    )
}

/// A sprint ID from an ID, an ARI, or a URL with `sprint=N` / `/sprints/N`.
pub fn sprint_ref(s: &str) -> Result<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse() {
        return Ok(n);
    }
    for marker in ["sprint=", "/sprints/", "/sprint/"] {
        if let Some(i) = s.find(marker) {
            let id: String = s[i + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(n) = id.parse() {
                return Ok(n);
            }
        }
    }
    trailing_number(s)
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| usage(format!("'{s}' is not a sprint ID, ARI or URL")))
}

/// Issues from a Jira Software list (sprint issues, board backlog), which
/// page by token.
pub fn software_list(
    c: &Client,
    path: &str,
    jql: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let fields = records::ROW_FIELDS.join(",");
    let mut q = vec![("fields", fields.as_str())];
    if let Some(j) = jql {
        q.push(("jql", j));
    }
    token_list(c, path, &q, limit, cursor)
}

/// Every issue of a Jira Software list (for moves), refusing a list that
/// cannot be finished.
pub(crate) fn software_issues(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Vec<Value>> {
    let mut all: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut q = query.to_vec();
        q.push(("maxResults", "100"));
        if let Some(t) = &token {
            q.push(("nextPageToken", t.as_str()));
        }
        let v = c.get(path, &q)?;
        let page = v["issues"]
            .as_array()
            .ok_or_else(|| anyhow!("Jira returned no issue list for {path}"))?;
        all.extend(page.iter().cloned());
        if v["isLast"].as_bool() == Some(true) {
            return Ok(all);
        }
        match v["nextPageToken"].as_str().filter(|t| !t.is_empty()) {
            Some(next) if token.as_deref() != Some(next) => token = Some(next.to_string()),
            None if v["isLast"].as_bool().is_none() => return Ok(all),
            _ => bail!("Jira said there are more issues but gave no way to read them"),
        }
    }
}

/// Read a JSON flag value (`-` reads stdin).
pub fn json_value(flag: &str, text: &str) -> Result<Value> {
    serde_json::from_str(text)
        .with_context(|| format!("{flag} is not valid JSON"))
        .map_err(|e| ApiError::usage(format!("{e:#}")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_fields_merge() {
        let m = extra_fields(
            Some(r#"{"customfield_1": 5}"#),
            &["customfield_2=\"7\"".into(), "x=hello".into()],
        )
        .unwrap();
        assert_eq!(m["customfield_1"], 5);
        assert_eq!(m["customfield_2"], "7");
        assert_eq!(m["x"], "hello");
        assert!(extra_fields(Some(r#"{"a":1}"#), &["a=2".into()]).is_err());
        assert!(extra_fields(None, &["novalue".into()]).is_err());
        assert!(is_field_id("customfield_10016"));
        assert!(is_field_id("duedate"));
        assert!(!is_field_id("Story Points"));
    }

    #[test]
    fn helpers() {
        assert_eq!(csv("a, b,,c"), vec!["a", "b", "c"]);
        assert_eq!(id_or_name("10001", "key"), json!({"id": "10001"}));
        assert_eq!(id_or_name("PROJ", "key"), json!({"key": "PROJ"}));
        assert_eq!(
            issue_ref("https://a.atlassian.net/browse/PROJ-9?x=1"),
            "PROJ-9"
        );
        assert_eq!(issue_ref("ari:cloud:jira:abc:issue/10001"), "10001");
        assert_eq!(
            project_ref("https://a.atlassian.net/jira/software/projects/PROJ/boards/1"),
            "PROJ"
        );
        assert_eq!(
            project_ref("https://a.atlassian.net/browse/PROJ-12"),
            "PROJ"
        );
        assert_eq!(project_ref("ari:cloud:jira:abc:project/10000"), "10000");
        assert_eq!(
            filter_ref("https://a.atlassian.net/issues/?filter=10100"),
            "10100"
        );
        assert_eq!(filter_ref("ari:cloud:jira:abc:filter/7"), "7");
    }

    #[test]
    fn filters_become_jql() {
        let s = ["Open".to_string(), "In Progress".to_string()];
        let f = IssueFilter {
            jql: Some("labels = x ORDER BY created"),
            project: Some("P"),
            status: &s,
            assignee: Some("me"),
            ..Default::default()
        };
        assert_eq!(
            f.jql().unwrap(),
            "(labels = x) AND project = \"P\" AND status in (\"Open\", \"In Progress\") AND assignee = currentUser() ORDER BY created"
        );
        assert_eq!(IssueFilter::default().jql(), None);
        let plain = IssueFilter {
            project: Some("P"),
            ..Default::default()
        };
        assert_eq!(
            plain.jql().unwrap(),
            "project = \"P\" ORDER BY updated DESC"
        );
    }

    #[test]
    fn screen_fields_get_inputs_and_examples() {
        let f = screen_field(
            "customfield_1",
            &json!({"name": "Flagged", "schema": {"type": "array", "items": "option"},
                "allowedValues": [{"value": "Impediment"}], "operations": ["set"]}),
        );
        assert_eq!(f["acceptedInput"], "multi-option");
        assert_eq!(f["example"], "--field 'Flagged=Impediment'");
        assert_eq!(f["fieldId"], "customfield_1");
        let types = vec![
            json!({"id": "1", "name": "Task"}),
            json!({"id": "2", "name": "Bug"}),
        ];
        assert_eq!(resolve_issue_type(&types, "bug").unwrap()["id"], "2");
        assert!(resolve_issue_type(&types, "Epic").is_err());
    }

    #[test]
    fn builds_link_payloads() {
        let link = link_payload("PROJ-1", "10002", "Blocks", None);
        assert_eq!(link["type"], json!({"name": "Blocks"}));
        assert_eq!(link["inwardIssue"], json!({"key": "PROJ-1"}));
        assert_eq!(link["outwardIssue"], json!({"id": "10002"}));
        let w = weblink_payload(&Weblink {
            url: "https://e.x",
            title: "T",
            resolved: true,
            ..Default::default()
        });
        assert_eq!(
            w["object"],
            json!({"url": "https://e.x", "title": "T", "status": {"resolved": true}})
        );
    }

    #[test]
    fn normalizes_sprint_states() {
        assert_eq!(
            normalize_sprint_states("Active, future").unwrap(),
            "active,future"
        );
        assert!(normalize_sprint_states("").is_err());
        assert!(normalize_sprint_states("active,unknown").is_err());
    }

    #[test]
    fn storage_bodies_are_refused_for_jira() {
        let err = adf_body("<p>x</p>", BodyFormat::Storage).unwrap_err();
        assert_eq!(ApiError::find(&err).unwrap().kind, ErrorKind::Usage);
        assert_eq!(adf_body("hi", BodyFormat::Markdown).unwrap()["type"], "doc");
    }
}

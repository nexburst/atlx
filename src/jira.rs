//! Jira Cloud REST v3 and Agile 1.0, shaped after twg's `jira` commands.

use crate::client::Client;
use crate::markup::{TextFormat, adf_to_text, quote, to_adf};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Map, Value, json};

const LIST_FIELDS: &[&str] = &[
    "summary",
    "status",
    "issuetype",
    "priority",
    "assignee",
    "updated",
];
const VIEW_FIELDS: &[&str] = &[
    "summary",
    "status",
    "issuetype",
    "priority",
    "assignee",
    "reporter",
    "description",
    "labels",
    "created",
    "updated",
    "parent",
    "duedate",
    "components",
    "fixVersions",
];

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn browse_url(c: &Client, key: &str) -> String {
    format!("{}/browse/{key}", c.base())
}

/// Split twg's comma-separated list flags (`--labels a,b`).
pub fn csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

/// Merge `--fields-json` and repeated `--field KEY=VALUE` (values parse as JSON
/// when valid, else stay strings). The same key twice is an error, as in twg.
pub fn extra_fields(fields_json: Option<&str>, field: &[String]) -> Result<Map<String, Value>> {
    let mut out = match fields_json {
        Some(j) => {
            match serde_json::from_str::<Value>(j).context("--fields-json is not valid JSON")? {
                Value::Object(m) => m,
                _ => bail!("--fields-json must be a JSON object"),
            }
        }
        None => Map::new(),
    };
    for kv in field {
        let (k, v) = kv
            .split_once('=')
            .with_context(|| format!("--field '{kv}' must be KEY=VALUE"))?;
        if out.contains_key(k) {
            bail!("field '{k}' given more than once (via --field and/or --fields-json)");
        }
        let v = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()));
        out.insert(k.to_string(), v);
    }
    Ok(out)
}

fn summarize(c: &Client, issue: &Value, extra: &[String]) -> Value {
    let f = &issue["fields"];
    let key = issue["key"].as_str().unwrap_or_default();
    let mut o = json!({
        "key": key,
        "summary": f["summary"],
        "status": f["status"]["name"],
        "type": f["issuetype"]["name"],
        "priority": f["priority"]["name"],
        "assignee": f["assignee"]["displayName"],
        "updated": f["updated"],
        "url": browse_url(c, key),
    });
    for name in extra {
        if o.get(name).is_none() {
            o[name] = f[name].clone();
        }
    }
    o
}

fn field_list(base: &[&str], extra: &[String]) -> String {
    let mut all: Vec<&str> = base.to_vec();
    all.extend(
        extra
            .iter()
            .map(String::as_str)
            .filter(|e| !base.contains(e)),
    );
    all.join(",")
}

// --- reads -------------------------------------------------------------------

pub struct GetOpts {
    pub fields: Vec<String>,
    pub comments: bool,
    pub remote_links: bool,
    pub full: bool,
}

/// `jira workitem get`
pub fn get(c: &Client, key: &str, o: &GetOpts) -> Result<Value> {
    let fields = if o.full {
        "*all".to_string()
    } else {
        field_list(VIEW_FIELDS, &o.fields)
    };
    let v = c.get(
        &format!("/rest/api/3/issue/{}", enc(key)),
        &[("fields", &fields)],
    )?;
    let f = &v["fields"];
    let key = v["key"].as_str().unwrap_or(key).to_string();
    let mut out = summarize(c, &v, &o.fields);
    out["reporter"] = f["reporter"]["displayName"].clone();
    out["created"] = f["created"].clone();
    out["due_date"] = f["duedate"].clone();
    out["labels"] = f["labels"].clone();
    out["components"] = names(&f["components"]);
    out["fix_versions"] = names(&f["fixVersions"]);
    out["parent"] = f["parent"]["key"].clone();
    out["description"] = adf_to_text(&f["description"]).into();
    if o.full {
        out["fields"] = f.clone();
    }
    if o.comments || o.full {
        out["comments"] = comments(c, &key)?;
    }
    if o.remote_links || o.full {
        out["remote_links"] = remote_links(c, &key)?;
    }
    Ok(out)
}

fn names(v: &Value) -> Value {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x["name"].as_str())
        .map(Value::from)
        .collect()
}

fn comments(c: &Client, key: &str) -> Result<Value> {
    let mut out = Vec::new();
    loop {
        let start = out.len().to_string();
        let v = c.get(
            &format!("/rest/api/3/issue/{}/comment", enc(key)),
            &[
                ("startAt", &start),
                ("maxResults", "100"),
                ("orderBy", "created"),
            ],
        )?;
        let page = v["comments"].as_array().cloned().unwrap_or_default();
        out.extend(page.iter().map(|cm| {
            json!({
                "id": cm["id"],
                "author": cm["author"]["displayName"],
                "created": cm["created"],
                "body": adf_to_text(&cm["body"]),
            })
        }));
        if page.is_empty() || out.len() as u64 >= v["total"].as_u64().unwrap_or(0) {
            return Ok(Value::Array(out));
        }
    }
}

fn remote_links(c: &Client, key: &str) -> Result<Value> {
    let v = c.get(&format!("/rest/api/3/issue/{}/remotelink", enc(key)), &[])?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|l| json!({"id": l["id"], "title": l["object"]["title"], "url": l["object"]["url"], "relationship": l["relationship"]}))
        .collect())
}

/// `jira workitem query`: JQL, following `nextPageToken` up to `limit`.
/// `after` resumes from a previous `next_cursor`.
pub fn query(
    c: &Client,
    jql: &str,
    limit: usize,
    fields: &[String],
    after: Option<&str>,
) -> Result<Value> {
    let field_param = field_list(LIST_FIELDS, fields);
    let mut issues = Vec::new();
    let mut token = after.map(String::from);
    loop {
        let page = (limit - issues.len()).min(100).to_string();
        let mut q = vec![
            ("jql", jql),
            ("maxResults", page.as_str()),
            ("fields", field_param.as_str()),
        ];
        if let Some(t) = &token {
            q.push(("nextPageToken", t));
        }
        let v = c.get("/rest/api/3/search/jql", &q)?;
        let batch = v["issues"].as_array().cloned().unwrap_or_default();
        issues.extend(
            batch
                .iter()
                .take(limit - issues.len())
                .map(|i| summarize(c, i, fields)),
        );
        token = v["nextPageToken"].as_str().map(String::from);
        if batch.is_empty() || token.is_none() || issues.len() >= limit {
            break;
        }
    }
    Ok(json!({"issues": issues, "next_cursor": token}))
}

/// `jira workitem search`: fuzzy text, as twg builds it.
pub fn search(
    c: &Client,
    text: &str,
    limit: usize,
    fields: &[String],
    after: Option<&str>,
) -> Result<Value> {
    query(
        c,
        &format!("text ~ {} ORDER BY updated DESC", quote(text)),
        limit,
        fields,
        after,
    )
}

// --- writes ------------------------------------------------------------------

fn id_or_name(s: &str, name_key: &str) -> Value {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        json!({"id": s})
    } else {
        json!({name_key: s})
    }
}

pub struct NewWorkitem<'a> {
    pub space: &'a str,
    pub issue_type: &'a str,
    pub summary: &'a str,
    pub description: Option<(&'a str, TextFormat)>,
    pub assignee: Option<&'a str>,
    pub reporter: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub labels: Vec<String>,
    pub parent: Option<&'a str>,
    pub extra: Map<String, Value>,
}

/// `jira workitem create`
pub fn create(c: &Client, n: NewWorkitem) -> Result<Value> {
    let mut fields = Map::new();
    fields.insert("project".into(), id_or_name(n.space, "key"));
    fields.insert("issuetype".into(), id_or_name(n.issue_type, "name"));
    fields.insert("summary".into(), n.summary.into());
    if let Some((d, f)) = n.description {
        fields.insert("description".into(), to_adf(d, f));
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
        fields.insert("parent".into(), json!({"key": p}));
    }
    fields.extend(n.extra);
    let v = c.post("/rest/api/3/issue", &json!({"fields": fields}))?;
    let key = v["key"].as_str().unwrap_or_default();
    Ok(json!({"key": key, "id": v["id"], "url": browse_url(c, key)}))
}

/// Set / add / remove a multi-value field the way twg's `--x`, `--add-x`, `--remove-x` do.
pub struct ListEdit {
    pub set: Option<Vec<String>>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Default)]
pub struct Update<'a> {
    pub summary: Option<&'a str>,
    pub description: Option<(&'a str, TextFormat)>,
    pub assignee: Option<&'a str>,
    pub issue_type: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub labels: Option<ListEdit>,
    pub components: Option<ListEdit>,
    pub fix_versions: Option<ListEdit>,
    pub parent: Option<&'a str>,
    pub due_date: Option<&'a str>,
    pub resolution: Option<&'a str>,
    pub extra: Map<String, Value>,
    pub status: Option<&'a str>,
    pub transition_comment: Option<(&'a str, TextFormat)>,
    pub comment: Option<(&'a str, TextFormat)>,
}

fn none_or(s: &str) -> Option<&str> {
    (!s.eq_ignore_ascii_case("none")).then_some(s)
}

/// `jira workitem update`: field edits, then optional transition and comment.
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
            bail!("cannot both set and add/remove '{name}' in the same update");
        }
    }
    let mut fields = Map::new();
    let mut ops = Map::new();
    let mut changed: Vec<&str> = Vec::new();
    let set = |name: &'static str, v: Value, fields: &mut Map<String, Value>| {
        fields.insert(name.into(), v);
        name
    };
    if let Some(s) = u.summary {
        changed.push(set("summary", s.into(), &mut fields));
    }
    if let Some((d, f)) = u.description {
        changed.push(set("description", to_adf(d, f), &mut fields));
    }
    if let Some(a) = u.assignee {
        let v = match none_or(a) {
            None => Value::Null,
            Some(a) => json!({"accountId": resolve_account(c, a)?}),
        };
        changed.push(set("assignee", v, &mut fields));
    }
    if let Some(t) = u.issue_type {
        changed.push(set("issuetype", id_or_name(t, "name"), &mut fields));
    }
    if let Some(p) = u.priority {
        changed.push(set("priority", id_or_name(p, "name"), &mut fields));
    }
    if let Some(p) = u.parent {
        changed.push(set(
            "parent",
            none_or(p).map_or(Value::Null, |p| json!({"key": p})),
            &mut fields,
        ));
    }
    if let Some(d) = u.due_date {
        changed.push(set(
            "duedate",
            none_or(d).map_or(Value::Null, Value::from),
            &mut fields,
        ));
    }
    if let Some(r) = u.resolution {
        changed.push(set(
            "resolution",
            none_or(r).map_or(Value::Null, |r| id_or_name(r, "name")),
            &mut fields,
        ));
    }
    for (name, edit, named) in [
        ("labels", u.labels, false),
        ("components", u.components, true),
        ("fixVersions", u.fix_versions, true),
    ] {
        let Some(edit) = edit else { continue };
        let wrap = |s: &String| if named { json!({"name": s}) } else { json!(s) };
        if let Some(all) = &edit.set {
            changed.push(set(name, all.iter().map(wrap).collect(), &mut fields));
        }
        let mut list: Vec<Value> = edit.add.iter().map(|x| json!({"add": wrap(x)})).collect();
        list.extend(edit.remove.iter().map(|x| json!({"remove": wrap(x)})));
        if !list.is_empty() {
            ops.insert(name.into(), list.into());
            changed.push(name);
        }
    }
    let extra_keys: Vec<String> = u.extra.keys().cloned().collect();
    fields.extend(u.extra);

    if fields.is_empty() && ops.is_empty() && u.status.is_none() && u.comment.is_none() {
        bail!("nothing to update; pass at least one field, --status or --comment");
    }
    let mut out = json!({"key": key, "updated": changed});
    out["updated"]
        .as_array_mut()
        .unwrap()
        .extend(extra_keys.into_iter().map(Value::from));
    if !fields.is_empty() || !ops.is_empty() {
        c.put(
            &format!("/rest/api/3/issue/{}", enc(key)),
            &json!({"fields": fields, "update": ops}),
        )?;
    }
    if let Some(s) = u.status {
        let t = transition(c, key, Some(s), Map::new(), u.transition_comment)?;
        out["status"] = t["status"].clone();
        if !t["comment_id"].is_null() {
            out["transition_comment_id"] = t["comment_id"].clone();
        }
    }
    if let Some((body, f)) = u.comment {
        out["comment_id"] = comment(c, key, body, f, None, false)?["id"].clone();
    }
    out["url"] = browse_url(c, key).into();
    Ok(out)
}

/// `jira workitem comment create`
pub fn comment(
    c: &Client,
    key: &str,
    body: &str,
    f: TextFormat,
    visibility: Option<(&str, &str)>,
    internal: bool,
) -> Result<Value> {
    let mut req = json!({"body": to_adf(body, f)});
    if let Some((kind, value)) = visibility {
        req["visibility"] = json!({"type": kind, "value": value});
    }
    if internal {
        req["properties"] = json!([{"key": "sd.public.comment", "value": {"internal": true}}]);
    }
    let v = c.post(&format!("/rest/api/3/issue/{}/comment", enc(key)), &req)?;
    let id = v["id"].as_str().unwrap_or_default();
    Ok(
        json!({"key": key, "id": id, "url": format!("{}?focusedCommentId={id}", browse_url(c, key))}),
    )
}

/// `jira workitem transitions query`: transitions plus the screen fields each requires.
pub fn transitions(c: &Client, key: &str) -> Result<Value> {
    let v = c.get(
        &format!("/rest/api/3/issue/{}/transitions", enc(key)),
        &[("expand", "transitions.fields")],
    )?;
    Ok(v["transitions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let required: Vec<Value> = t["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, f)| f["required"] == true)
                .map(|(id, f)| {
                    let allowed: Vec<&str> = f["allowedValues"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a["name"].as_str().or(a["value"].as_str()))
                        .collect();
                    json!({"id": id, "name": f["name"], "allowed_values": allowed})
                })
                .collect();
            json!({"id": t["id"], "name": t["name"], "to": t["to"]["name"], "has_screen": t["hasScreen"], "requirements": required})
        })
        .collect())
}

/// `jira workitem transition`: without a target, list transitions (read-only).
/// The target matches a transition id, transition name, or destination status.
pub fn transition(
    c: &Client,
    key: &str,
    target: Option<&str>,
    fields: Map<String, Value>,
    transition_comment: Option<(&str, TextFormat)>,
) -> Result<Value> {
    let ts = transitions(c, key)?;
    let Some(target) = target else {
        return Ok(json!({"key": key, "transitions": ts}));
    };
    let is = |t: &Value, k: &str| {
        t[k].as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(target))
    };
    let list = ts.as_array().unwrap();
    let Some(t) = list
        .iter()
        .find(|t| is(t, "id"))
        .or_else(|| list.iter().find(|t| is(t, "name") || is(t, "to")))
    else {
        let names: Vec<_> = list.iter().filter_map(|t| t["name"].as_str()).collect();
        bail!(
            "no transition '{target}' for {key}; available: {}",
            names.join(", ")
        );
    };
    let mut req = json!({"transition": {"id": t["id"]}});
    if !fields.is_empty() {
        req["fields"] = fields.into();
    }
    // Jira silently drops a transition comment when the transition has no
    // screen, so in that case add it as a regular comment afterwards.
    let has_screen = t["has_screen"] == true;
    if let Some((body, f)) = transition_comment.filter(|_| has_screen) {
        req["update"] = json!({"comment": [{"add": {"body": to_adf(body, f)}}]});
    }
    c.post(&format!("/rest/api/3/issue/{}/transitions", enc(key)), &req)?;
    let mut out =
        json!({"key": key, "transition": t["name"], "status": t["to"], "url": browse_url(c, key)});
    if let Some((body, f)) = transition_comment.filter(|_| !has_screen) {
        out["comment_id"] = comment(c, key, body, f, None, false)?["id"].clone();
    }
    Ok(out)
}

pub fn myself(c: &Client) -> Result<Value> {
    c.get("/rest/api/3/myself", &[])
}

fn fetch_users(c: &Client, query: &str, max_results: Option<usize>) -> Result<Vec<Value>> {
    let max = max_results.map(|n| n.clamp(1, 1000).to_string());
    let mut q = vec![("query", query)];
    if let Some(m) = &max {
        q.push(("maxResults", m.as_str()));
    }
    let v = c.get("/rest/api/3/user/search", &q)?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

fn summarize_user(u: &Value) -> Value {
    let mut o = json!({
        "account_id": u["accountId"],
        "display_name": u["displayName"],
    });
    if let Some(email) = u["emailAddress"].as_str().filter(|s| !s.is_empty()) {
        o["email"] = email.into();
    }
    o["active"] = u["active"].clone();
    o["account_type"] = u["accountType"].clone();
    o
}

/// `user search`
pub fn user_search(c: &Client, query: &str, limit: usize) -> Result<Value> {
    if limit == 0 {
        return Ok(json!([]));
    }
    let users = fetch_users(c, query, Some(limit))?;
    Ok(users.iter().take(limit).map(summarize_user).collect())
}

/// `me`, a raw accountId, or a name/email that matches exactly one user.
fn resolve_account(c: &Client, who: &str) -> Result<String> {
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
        [] => bail!("no user matches '{who}'"),
        many => {
            let names: Vec<_> = many
                .iter()
                .filter_map(|u| u["displayName"].as_str())
                .collect();
            bail!(
                "'{who}' matches several users: {}; pass an account ID",
                names.join(", ")
            )
        }
    }
}

// --- metadata: fields, projects, create-meta ---------------------------------

pub fn filter_fields(fields: &[Value], query: Option<&str>, custom_only: bool) -> Vec<Value> {
    let q = query
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    fields
        .iter()
        .filter(|f| !custom_only || f["custom"].as_bool() == Some(true))
        .filter(|f| {
            let Some(q) = &q else { return true };
            let name = f["name"].as_str().unwrap_or_default().to_lowercase();
            let id = f["id"].as_str().unwrap_or_default().to_lowercase();
            name.contains(q) || id.contains(q)
        })
        .map(|f| {
            json!({
                "id": f["id"],
                "name": f["name"],
                "custom": f["custom"],
                "type": f["schema"]["type"],
            })
        })
        .collect()
}

/// `jira field list`
pub fn fields(c: &Client, query: Option<&str>, custom_only: bool) -> Result<Value> {
    let v = c.get("/rest/api/3/field", &[])?;
    let list = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
    Ok(Value::Array(filter_fields(list, query, custom_only)))
}

fn paginate_values(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    limit: usize,
) -> Result<Vec<Value>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    loop {
        let start = out.len().to_string();
        let max_page = (limit - out.len()).min(50);
        let page_size = max_page.to_string();
        let mut q = query.to_vec();
        q.push(("startAt", &start));
        q.push(("maxResults", &page_size));
        let v = c.get(path, &q)?;
        let page = v["values"].as_array().cloned().unwrap_or_default();
        let page_len = page.len();
        out.extend(page);
        let is_last = v["isLast"].as_bool();
        let total = v["total"].as_u64();
        if page_len == 0
            || is_last == Some(true)
            || total.is_some_and(|t| out.len() as u64 >= t)
            || (is_last.is_none() && total.is_none() && page_len < max_page)
            || out.len() >= limit
        {
            break;
        }
    }
    out.truncate(limit);
    Ok(out)
}

fn summarize_project(c: &Client, p: &Value) -> Value {
    let key = p["key"].as_str().unwrap_or_default();
    json!({
        "id": p["id"],
        "key": key,
        "name": p["name"],
        "type": p["projectTypeKey"],
        "style": p["style"],
        "simplified": p["simplified"],
        "url": browse_url(c, key),
    })
}

/// `jira project list`
pub fn projects(c: &Client, query: Option<&str>, limit: usize) -> Result<Value> {
    let mut q = Vec::new();
    if let Some(s) = query.map(str::trim).filter(|s| !s.is_empty()) {
        q.push(("query", s));
    }
    let items = paginate_values(c, "/rest/api/3/project/search", &q, limit)?;
    Ok(items.iter().map(|p| summarize_project(c, p)).collect())
}

/// `jira project get`
pub fn project(c: &Client, key_or_id: &str) -> Result<Value> {
    let v = c.get(&format!("/rest/api/3/project/{}", enc(key_or_id)), &[])?;
    let mut out = summarize_project(c, &v);
    out["lead"] = v["lead"]["displayName"].clone();
    out["description"] = v["description"].clone();
    out["issue_types"] = v["issueTypes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(summarize_issue_type)
        .collect();
    Ok(out)
}

fn paginate_createmeta(c: &Client, path: &str, key: &str, alt_key: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        let start = out.len().to_string();
        let v = c.get(path, &[("startAt", &start), ("maxResults", "50")])?;
        let page = v[key]
            .as_array()
            .or_else(|| v[alt_key].as_array())
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

fn summarize_issue_type(t: &Value) -> Value {
    json!({
        "id": t["id"],
        "name": t["name"],
        "subtask": t["subtask"],
        "description": t["description"],
    })
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
            anyhow!("no issue type '{target}'; available: {}", names.join(", "))
        })
}

fn summarize_create_fields(fields: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = fields
        .iter()
        .map(|f| {
            let id = f["fieldId"]
                .as_str()
                .or_else(|| f["key"].as_str())
                .unwrap_or_default();
            let allowed: Vec<&str> = f["allowedValues"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    a["name"]
                        .as_str()
                        .or(a["value"].as_str())
                        .or(a["key"].as_str())
                        .or(a["id"].as_str())
                })
                .collect();
            json!({
                "id": id,
                "name": f["name"],
                "required": f["required"],
                "type": f["schema"]["type"],
                "has_default": f["hasDefaultValue"],
                "allowed_values": allowed,
            })
        })
        .collect();
    out.sort_by_key(|f| f["required"].as_bool() != Some(true));
    out
}

/// `jira workitem create-meta`
pub fn create_meta(c: &Client, space: &str, issue_type: Option<&str>) -> Result<Value> {
    let types_path = format!("/rest/api/3/issue/createmeta/{}/issuetypes", enc(space));
    let types = paginate_createmeta(c, &types_path, "issueTypes", "values")?;
    let Some(target) = issue_type else {
        return Ok(json!({
            "space": space,
            "issue_types": types.iter().map(summarize_issue_type).collect::<Vec<_>>(),
        }));
    };
    let it = resolve_issue_type(&types, target)?;
    let it_id = it["id"].as_str().unwrap_or_default();
    let fields_path = format!("{types_path}/{}", enc(it_id));
    let raw_fields = paginate_createmeta(c, &fields_path, "fields", "values")?;
    Ok(json!({
        "space": space,
        "issue_type": summarize_issue_type(it),
        "fields": summarize_create_fields(&raw_fields),
    }))
}

// --- issue & remote links ----------------------------------------------------

/// `jira workitem link types`
pub fn link_types(c: &Client) -> Result<Value> {
    let v = c.get("/rest/api/3/issueLinkType", &[])?;
    Ok(v["issueLinkTypes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            json!({
                "id": t["id"],
                "name": t["name"],
                "inward": t["inward"],
                "outward": t["outward"],
            })
        })
        .collect())
}

fn build_issue_link_payload(
    inward: &str,
    outward: &str,
    link_type: &str,
    comment: Option<(&str, TextFormat)>,
) -> Value {
    let mut req = json!({
        "type": id_or_name(link_type, "name"),
        "inwardIssue": id_or_name(inward, "key"),
        "outwardIssue": id_or_name(outward, "key"),
    });
    if let Some((body, fmt)) = comment {
        req["comment"] = json!({"body": to_adf(body, fmt)});
    }
    req
}

/// `jira workitem link create`
pub fn link_create(
    c: &Client,
    inward: &str,
    outward: &str,
    link_type: &str,
    comment: Option<(&str, TextFormat)>,
) -> Result<Value> {
    let req = build_issue_link_payload(inward, outward, link_type, comment);
    c.post("/rest/api/3/issueLink", &req)?;
    Ok(json!({
        "inward": inward,
        "outward": outward,
        "type": link_type,
        "inward_url": browse_url(c, inward),
        "outward_url": browse_url(c, outward),
    }))
}

fn build_remote_link_payload(url: &str, title: &str, relationship: Option<&str>) -> Value {
    let mut req = json!({
        "object": {
            "url": url,
            "title": title,
        }
    });
    if let Some(rel) = relationship {
        req["relationship"] = rel.into();
    }
    req
}

/// `jira workitem link remote`
pub fn link_remote(
    c: &Client,
    key: &str,
    url: &str,
    title: &str,
    relationship: Option<&str>,
) -> Result<Value> {
    let req = build_remote_link_payload(url, title, relationship);
    let v = c.post(&format!("/rest/api/3/issue/{}/remotelink", enc(key)), &req)?;
    let mut out = json!({
        "key": key,
        "id": v["id"],
        "title": title,
        "url": url,
        "issue_url": browse_url(c, key),
    });
    if let Some(rel) = relationship {
        out["relationship"] = rel.into();
    }
    Ok(out)
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

fn summarize_board(b: &Value) -> Value {
    json!({
        "id": b["id"],
        "name": b["name"],
        "type": b["type"],
        "project_key": b["location"]["projectKey"],
        "project_name": b["location"]["projectName"],
    })
}

/// `jira board list`
pub fn boards(
    c: &Client,
    project: Option<&str>,
    board_type: Option<BoardType>,
    limit: usize,
) -> Result<Value> {
    let mut q = Vec::new();
    if let Some(p) = project {
        q.push(("projectKeyOrId", p));
    }
    if let Some(t) = board_type {
        q.push(("type", t.as_str()));
    }
    let items = paginate_values(c, "/rest/agile/1.0/board", &q, limit)?;
    Ok(items.iter().map(summarize_board).collect())
}

pub fn normalize_sprint_states(s: &str) -> Result<String> {
    let states = csv(s);
    if states.is_empty() {
        bail!("--state cannot be empty; expected active, future, or closed");
    }
    let mut out = Vec::with_capacity(states.len());
    for st in states {
        let lower = st.to_lowercase();
        match lower.as_str() {
            "active" | "future" | "closed" => out.push(lower),
            _ => bail!("invalid sprint state '{st}'; expected active, future, or closed"),
        }
    }
    Ok(out.join(","))
}

fn summarize_sprint(s: &Value) -> Value {
    json!({
        "id": s["id"],
        "name": s["name"],
        "state": s["state"],
        "start_date": s["startDate"],
        "end_date": s["endDate"],
        "complete_date": s["completeDate"],
        "board_id": s["originBoardId"],
        "goal": s["goal"],
    })
}

/// `jira sprint list`
pub fn sprints(c: &Client, board_id: u64, state: Option<&str>, limit: usize) -> Result<Value> {
    let state_param = state.map(normalize_sprint_states).transpose()?;
    let mut q = Vec::new();
    if let Some(st) = &state_param {
        q.push(("state", st.as_str()));
    }
    let items = paginate_values(
        c,
        &format!("/rest/agile/1.0/board/{board_id}/sprint"),
        &q,
        limit,
    )?;
    Ok(items.iter().map(summarize_sprint).collect())
}

/// `jira sprint issues`
pub fn sprint_issues(c: &Client, sprint_id: u64, jql: Option<&str>, limit: usize) -> Result<Value> {
    if limit == 0 {
        return Ok(json!([]));
    }
    let field_param = field_list(LIST_FIELDS, &[]);
    let mut issues = Vec::new();
    loop {
        let start = issues.len().to_string();
        let page_size = (limit - issues.len()).min(100).to_string();
        let mut q = vec![
            ("startAt", start.as_str()),
            ("maxResults", page_size.as_str()),
            ("fields", field_param.as_str()),
        ];
        if let Some(j) = jql {
            q.push(("jql", j));
        }
        let v = c.get(&format!("/rest/agile/1.0/sprint/{sprint_id}/issue"), &q)?;
        let batch = v["issues"].as_array().cloned().unwrap_or_default();
        let batch_len = batch.len();
        issues.extend(
            batch
                .iter()
                .take(limit - issues.len())
                .map(|i| summarize(c, i, &[])),
        );
        let total = v["total"].as_u64();
        if batch_len == 0
            || issues.len() >= limit
            || total.is_some_and(|t| issues.len() as u64 >= t)
            || (total.is_none() && batch_len < 100)
        {
            break;
        }
    }
    Ok(Value::Array(issues))
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
    }

    #[test]
    fn helpers() {
        assert_eq!(csv("a, b,,c"), vec!["a", "b", "c"]);
        assert_eq!(id_or_name("10001", "key"), json!({"id": "10001"}));
        assert_eq!(id_or_name("PROJ", "key"), json!({"key": "PROJ"}));
        assert_eq!(field_list(&["a", "b"], &["b".into(), "c".into()]), "a,b,c");
    }

    #[test]
    fn update_rejects_conflicting_list_edits() {
        let c = Client::new("https://example.atlassian.net".into(), "a@b.c", "tok", None);
        let err = update(
            &c,
            "PROJ-1",
            Update {
                labels: Some(ListEdit {
                    set: Some(vec!["a".into()]),
                    add: vec!["b".into()],
                    remove: vec![],
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("labels"), "{err}");

        let err = update(
            &c,
            "PROJ-1",
            Update {
                components: Some(ListEdit {
                    set: Some(vec![]),
                    add: vec![],
                    remove: vec!["api".into()],
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("components"), "{err}");

        let err = update(
            &c,
            "PROJ-1",
            Update {
                fix_versions: Some(ListEdit {
                    set: Some(vec!["1.0".into()]),
                    add: vec!["2.0".into()],
                    remove: vec![],
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("fixVersions"), "{err}");
    }

    #[test]
    fn user_summary_handles_optional_email() {
        let with_email = json!({
            "accountId": "123",
            "displayName": "Alice",
            "emailAddress": "alice@example.com",
            "active": true,
            "accountType": "atlassian"
        });
        assert_eq!(
            summarize_user(&with_email),
            json!({
                "account_id": "123",
                "display_name": "Alice",
                "email": "alice@example.com",
                "active": true,
                "account_type": "atlassian"
            })
        );
        let without_email = json!({
            "accountId": "456",
            "displayName": "Bob",
            "active": false,
            "accountType": "customer"
        });
        let s = summarize_user(&without_email);
        assert!(s.get("email").is_none());
        assert_eq!(s["account_id"], "456");
        assert_eq!(s["active"], false);
    }

    #[test]
    fn filters_fields_by_query_and_custom() {
        let raw = vec![
            json!({"id": "summary", "name": "Summary", "custom": false, "schema": {"type": "string"}}),
            json!({"id": "customfield_10010", "name": "Story Points", "custom": true, "schema": {"type": "number"}}),
            json!({"id": "customfield_10020", "name": "Sprint", "custom": true, "schema": {"type": "array"}}),
        ];
        assert_eq!(filter_fields(&raw, None, false).len(), 3);
        let custom = filter_fields(&raw, None, true);
        assert_eq!(custom.len(), 2);
        assert_eq!(custom[0]["id"], "customfield_10010");
        assert_eq!(custom[0]["type"], "number");
        let by_name = filter_fields(&raw, Some("story"), true);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0]["name"], "Story Points");
        let by_id = filter_fields(&raw, Some("10020"), false);
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0]["name"], "Sprint");
    }

    #[test]
    fn create_meta_helpers_resolve_and_sort_fields() {
        let types = vec![
            json!({"id": "10001", "name": "Task", "subtask": false, "description": "A task"}),
            json!({"id": "10002", "name": "Bug", "subtask": false, "description": "A bug"}),
        ];
        assert_eq!(resolve_issue_type(&types, "bug").unwrap()["id"], "10002");
        assert_eq!(resolve_issue_type(&types, "10001").unwrap()["name"], "Task");
        assert!(resolve_issue_type(&types, "Epic").is_err());

        let fields = vec![
            json!({"fieldId": "labels", "name": "Labels", "required": false, "schema": {"type": "array"}, "hasDefaultValue": false}),
            json!({"fieldId": "priority", "name": "Priority", "required": true, "schema": {"type": "priority"}, "hasDefaultValue": true, "allowedValues": [{"name": "High"}, {"name": "Low"}]}),
        ];
        let summarized = summarize_create_fields(&fields);
        assert_eq!(summarized[0]["id"], "priority");
        assert_eq!(summarized[0]["required"], true);
        assert_eq!(summarized[0]["allowed_values"], json!(["High", "Low"]));
        assert_eq!(summarized[1]["id"], "labels");
    }

    #[test]
    fn builds_link_payloads() {
        let link = build_issue_link_payload(
            "PROJ-1",
            "10002",
            "Blocks",
            Some(("blocked", TextFormat::Plain)),
        );
        assert_eq!(link["type"], json!({"name": "Blocks"}));
        assert_eq!(link["inwardIssue"], json!({"key": "PROJ-1"}));
        assert_eq!(link["outwardIssue"], json!({"id": "10002"}));
        assert_eq!(link["comment"]["body"]["type"], "doc");

        let link_no_comment = build_issue_link_payload("PROJ-1", "PROJ-2", "10000", None);
        assert_eq!(link_no_comment["type"], json!({"id": "10000"}));
        assert!(link_no_comment.get("comment").is_none());

        let remote =
            build_remote_link_payload("https://example.com/doc", "Design Doc", Some("Wiki Page"));
        assert_eq!(
            remote,
            json!({"relationship": "Wiki Page", "object": {"url": "https://example.com/doc", "title": "Design Doc"}})
        );
        let remote_min = build_remote_link_payload("https://example.com", "Home", None);
        assert!(remote_min.get("relationship").is_none());
    }

    #[test]
    fn normalizes_sprint_states() {
        assert_eq!(
            normalize_sprint_states("active, future").unwrap(),
            "active,future"
        );
        assert_eq!(normalize_sprint_states("CLOSED").unwrap(), "closed");
        assert!(normalize_sprint_states("").is_err());
        assert!(normalize_sprint_states("active,unknown").is_err());
    }

    #[test]
    fn summarizes_boards_sprints_and_projects() {
        let c = Client::new(
            "https://acme.atlassian.net".into(),
            "u@acme.com",
            "tok",
            None,
        );
        let proj = json!({
            "id": "10000",
            "key": "PROJ",
            "name": "Main Project",
            "projectTypeKey": "software",
            "style": "classic",
            "simplified": false
        });
        assert_eq!(
            summarize_project(&c, &proj),
            json!({
                "id": "10000",
                "key": "PROJ",
                "name": "Main Project",
                "type": "software",
                "style": "classic",
                "simplified": false,
                "url": "https://acme.atlassian.net/browse/PROJ"
            })
        );

        let board = json!({
            "id": 7,
            "name": "PROJ board",
            "type": "scrum",
            "location": {"projectKey": "PROJ", "projectName": "Main Project"}
        });
        assert_eq!(
            summarize_board(&board),
            json!({
                "id": 7,
                "name": "PROJ board",
                "type": "scrum",
                "project_key": "PROJ",
                "project_name": "Main Project"
            })
        );
        assert_eq!(BoardType::Scrum.as_str(), "scrum");
        assert_eq!(BoardType::Kanban.as_str(), "kanban");

        let sprint = json!({
            "id": 42,
            "name": "Sprint 42",
            "state": "active",
            "startDate": "2026-01-01T00:00:00Z",
            "endDate": "2026-01-14T00:00:00Z",
            "completeDate": null,
            "originBoardId": 7,
            "goal": "Ship it"
        });
        assert_eq!(
            summarize_sprint(&sprint),
            json!({
                "id": 42,
                "name": "Sprint 42",
                "state": "active",
                "start_date": "2026-01-01T00:00:00Z",
                "end_date": "2026-01-14T00:00:00Z",
                "complete_date": null,
                "board_id": 7,
                "goal": "Ship it"
            })
        );
    }
}

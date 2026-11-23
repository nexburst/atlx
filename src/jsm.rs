//! Jira Service Management over the public Service Desk REST API.
//!
//! List functions return the REST objects and the next cursor; the command
//! line projects them onto the records in [`records`].

use crate::client::{ApiError, Client, ErrorKind};
use crate::jira::{Listed, Offsets, local_list, offset_list, resolve_account};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

pub mod records;

const API: &str = "/rest/servicedeskapi";
/// The most items read to resolve a name or check every entry.
const SCAN: usize = 5000;
const EXPERIMENTAL: &[(&str, &str)] = &[("X-ExperimentalApi", "opt-in")];

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// An ID, which REST prints as a string or a number.
fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn clean(mut v: Value) -> Value {
    crate::jira::strip_media(&mut v);
    v
}

/// One collected range of a list.
#[derive(Debug)]
struct Page {
    values: Vec<Value>,
    last: bool,
}

/// A Service Desk list from `cursor`.
fn list(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    headers: &[(&str, &str)],
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    offset_list(
        c,
        path,
        query,
        headers,
        Offsets::servicedesk(),
        limit,
        cursor,
    )
}

/// Follow a paged list from `start` until `limit`.
fn page(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    headers: &[(&str, &str)],
    start: usize,
    limit: usize,
) -> Result<Page> {
    let mut out = Page {
        values: Vec::new(),
        last: false,
    };
    while out.values.len() < limit {
        let at = (start + out.values.len()).to_string();
        let size = (limit - out.values.len()).min(50);
        let max = size.to_string();
        let mut q = query.to_vec();
        q.push(("start", &at));
        q.push(("limit", &max));
        let v = c.request_with("GET", path, &q, None, headers)?;
        let values = v["values"]
            .as_array()
            .ok_or_else(|| anyhow!("{path} answered without a list"))?;
        let got = values.len();
        out.values.extend(values.iter().take(size).cloned());
        // The server may serve smaller pages than asked, so trust isLastPage.
        let last = v["isLastPage"].as_bool().unwrap_or(got < size);
        if last || got == 0 {
            out.last = got <= size;
            break;
        }
    }
    Ok(out)
}

fn all(c: &Client, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
    Ok(page(c, path, query, &[], 0, SCAN)?.values)
}

// --- references --------------------------------------------------------------

/// A request key or ID, given plainly or as a browse, portal or queue URL.
pub fn request_key(s: &str) -> Result<String> {
    let s = s.trim();
    let candidate = if s.starts_with("https://") || s.starts_with("http://") {
        let path = &s[..s.find(['?', '#']).unwrap_or(s.len())];
        let marked = ["/browse/", "/portal/", "/queues/"]
            .iter()
            .any(|m| path.contains(m));
        let last = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        // The last segment of those URLs is also a portal or queue number.
        if marked && !digits(last) { last } else { "" }
    } else if s.starts_with("ari:") {
        s.rsplit_once(":issue/").map_or("", |(_, id)| id)
    } else {
        s
    };
    let is_key = candidate.rsplit_once('-').is_some_and(|(project, n)| {
        digits(n)
            && project.starts_with(|c: char| c.is_ascii_alphabetic())
            && project
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    });
    if is_key || digits(candidate) {
        Ok(candidate.to_string())
    } else {
        Err(usage(format!("'{s}' is not a request key, ID or URL")))
    }
}

fn request_path(request: &str) -> Result<String> {
    Ok(format!("{API}/request/{}", enc(&request_key(request)?)))
}

fn list_of(items: &[Value], name: &str) -> String {
    let shown: Vec<String> = items
        .iter()
        .take(30)
        .map(|v| {
            let id = id_text(&v["id"]).unwrap_or_default();
            format!("{id} ({})", v[name].as_str().unwrap_or_default())
        })
        .collect();
    shown.join(", ")
}

/// A service desk ID from an ID, portal URL, project key or exact name.
pub fn resolve_desk(c: &Client, r: &str) -> Result<String> {
    let r = r.trim();
    if digits(r) {
        return Ok(r.to_string());
    }
    if r.starts_with("https://") || r.starts_with("http://") {
        let id: String = r
            .split_once("/portal/")
            .map(|(_, rest)| rest.chars().take_while(char::is_ascii_digit).collect())
            .unwrap_or_default();
        if digits(&id) {
            return Ok(id);
        }
        return Err(usage(format!("'{r}' is not a portal URL")));
    }
    let desks = all(c, &format!("{API}/servicedesk"), &[])?;
    let same = |d: &Value, k: &str| d[k].as_str().is_some_and(|v| v.eq_ignore_ascii_case(r));
    // Keys are unique; names only need to be.
    let mut found: Vec<&Value> = desks.iter().filter(|d| same(d, "projectKey")).collect();
    if found.is_empty() {
        found = desks.iter().filter(|d| same(d, "projectName")).collect();
    }
    match found.as_slice() {
        [d] => id_text(&d["id"]).ok_or_else(|| anyhow!("service desk '{r}' has no ID")),
        _ => Err(usage(format!(
            "'{r}' matches {} service desks; the service desks are {}",
            found.len(),
            list_of(&desks, "projectName")
        ))),
    }
}

/// The service desk of a project ID, key, ARI or URL.
pub fn desk_of_project(c: &Client, r: &str) -> Result<String> {
    let r = r.trim();
    let project = if r.starts_with("ari:") {
        r.rsplit('/').next().unwrap_or(r)
    } else if let Some((_, rest)) = r.split_once("/projects/") {
        rest.split(['/', '?', '#']).next().unwrap_or(rest)
    } else {
        r
    };
    let found: Vec<String> = all(c, &format!("{API}/servicedesk"), &[])?
        .iter()
        .filter(|d| {
            id_text(&d["projectId"]).as_deref() == Some(project)
                || d["projectKey"]
                    .as_str()
                    .is_some_and(|k| k.eq_ignore_ascii_case(project))
        })
        .filter_map(|d| id_text(&d["id"]))
        .collect();
    match found.as_slice() {
        [id] => Ok(id.clone()),
        _ => Err(usage(format!("no service desk belongs to project '{r}'"))),
    }
}

/// A request type ID from an ID or exact name; without one, the error lists
/// the desk's types.
pub fn resolve_type(c: &Client, desk: &str, r: Option<&str>) -> Result<String> {
    if let Some(id) = r.map(str::trim).filter(|r| digits(r)) {
        return Ok(id.to_string());
    }
    let types = all(
        c,
        &format!("{API}/servicedesk/{}/requesttype", enc(desk)),
        &[],
    )?;
    let listed = list_of(&types, "name");
    let Some(name) = r.map(str::trim) else {
        return Err(usage(format!(
            "pass --request-type; service desk {desk} has {listed}"
        )));
    };
    let found: Vec<String> = types
        .iter()
        .filter(|t| {
            t["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .filter_map(|t| id_text(&t["id"]))
        .collect();
    match found.as_slice() {
        [id] => Ok(id.clone()),
        _ => Err(usage(format!(
            "'{name}' matches {} request types; service desk {desk} has {listed}",
            found.len()
        ))),
    }
}

// --- service desks, types, queues --------------------------------------------

/// Service desks; `term` filters on key and name client-side.
pub fn desks(c: &Client, term: Option<&str>, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    let path = format!("{API}/servicedesk");
    let Some(term) = term else {
        return list(c, &path, &[], &[], limit, cursor);
    };
    let term = term.to_lowercase();
    let found: Vec<Value> = all(c, &path, &[])?
        .into_iter()
        .filter(|d| {
            ["projectKey", "projectName"].iter().any(|k| {
                d[*k]
                    .as_str()
                    .is_some_and(|v| v.to_lowercase().contains(&term))
            })
        })
        .collect();
    local_list(found, limit, cursor)
}

pub fn request_types(
    c: &Client,
    desk: Option<&str>,
    term: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    let path = match desk {
        Some(d) => format!("{API}/servicedesk/{}/requesttype", enc(d)),
        None => format!("{API}/requesttype"),
    };
    let q: Vec<(&str, &str)> = term.iter().map(|t| ("searchQuery", *t)).collect();
    list(c, &path, &q, &[], limit, cursor)
}

/// One request type, with its fields under `fields`.
pub fn request_type(c: &Client, desk: &str, id: &str) -> Result<Value> {
    let mut v = c.get(
        &format!("{API}/servicedesk/{}/requesttype/{}", enc(desk), enc(id)),
        &[],
    )?;
    let f = fields(c, desk, id)?;
    if v.is_object() {
        v["fields"] = f["requestTypeFields"].clone();
        v["example"] = example_answers(&f)?;
    }
    Ok(clean(v))
}

pub fn fields(c: &Client, desk: &str, request_type: &str) -> Result<Value> {
    c.get(
        &format!(
            "{API}/servicedesk/{}/requesttype/{}/field",
            enc(desk),
            enc(request_type)
        ),
        &[],
    )
}

fn field_list(v: &Value) -> Result<&Vec<Value>> {
    v["requestTypeFields"]
        .as_array()
        .ok_or_else(|| anyhow!("Jira returned no field list for the request type"))
}

/// An example `--answers-json` document for a request type's fields.
pub fn example_answers(fields: &Value) -> Result<Value> {
    let mut out = Map::new();
    for f in field_list(fields)? {
        let Some(name) = f["name"].as_str().or(f["fieldId"].as_str()) else {
            continue;
        };
        let first = f["validValues"][0]["label"].as_str();
        let one: Value = match (first, f["jiraSchema"]["type"].as_str()) {
            (Some(label), _) => label.into(),
            (None, Some("number")) => 1.into(),
            (None, Some("date")) => "2026-01-31".into(),
            _ => "VALUE".into(),
        };
        let value = if f["jiraSchema"]["type"] == "array" {
            json!([one])
        } else {
            one
        };
        out.insert(name.into(), value);
    }
    Ok(out.into())
}

pub fn queues(c: &Client, desk: &str, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    let path = format!("{API}/servicedesk/{}/queue", enc(desk));
    list(c, &path, &[("includeCount", "true")], &[], limit, cursor)
}

pub fn queue_issues(
    c: &Client,
    desk: &str,
    queue: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    if !digits(queue.trim()) {
        return Err(usage("--queue takes a queue ID"));
    }
    let path = format!(
        "{API}/servicedesk/{}/queue/{}/issue",
        enc(desk),
        enc(queue.trim())
    );
    list(c, &path, &[], &[], limit, cursor)
}

/// Knowledge base articles matching `query`, in one desk or all.
pub fn articles(
    c: &Client,
    desk: Option<&str>,
    query: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    if query.trim().is_empty() {
        return Err(usage(
            "pass some text: the REST API can only search articles",
        ));
    }
    let path = match desk {
        Some(d) => format!("{API}/servicedesk/{}/knowledgebase/article", enc(d)),
        None => format!("{API}/knowledgebase/article"),
    };
    list(c, &path, &[("query", query)], EXPERIMENTAL, limit, cursor)
}

// --- request values ----------------------------------------------------------

/// `jsm request create|validate` input.
#[derive(Debug, Default)]
pub struct Values<'a> {
    pub summary: Option<&'a str>,
    pub description: Option<&'a str>,
    /// Field ID to REST value, sent as given.
    pub fields: Map<String, Value>,
    /// Question label or field ID to a plain answer, shaped by the field.
    pub answers: Vec<(String, Value)>,
}

/// One value as the field's schema wants it.
fn shaped(field: &Value, value: &Value) -> std::result::Result<Value, String> {
    if value.is_object() || value.is_null() {
        return Ok(value.clone());
    }
    let schema = &field["jiraSchema"];
    let kind = schema["type"].as_str().unwrap_or_default();
    if kind == "array" {
        let one = |v: &Value| {
            let mut item = field.clone();
            item["jiraSchema"] = json!({"type": schema["items"]});
            shaped(&item, v)
        };
        return match value {
            Value::Array(list) => list.iter().map(one).collect(),
            v => Ok(json!([one(v)?])),
        };
    }
    let text = id_text(value).unwrap_or_else(|| value.to_string());
    let options = field["validValues"].as_array().filter(|o| !o.is_empty());
    if let Some(options) = options {
        let found = options
            .iter()
            .find(|o| o["value"].as_str() == Some(text.as_str()))
            .or_else(|| {
                let mut by_label = options.iter().filter(|o| {
                    o["label"]
                        .as_str()
                        .is_some_and(|l| l.eq_ignore_ascii_case(&text))
                });
                by_label.next().filter(|_| by_label.next().is_none())
            });
        let Some(found) = found else {
            let labels: Vec<&str> = options
                .iter()
                .take(20)
                .filter_map(|o| o["label"].as_str())
                .collect();
            return Err(format!("'{text}' is not one of {}", labels.join(", ")));
        };
        return Ok(if kind == "string" {
            found["value"].clone()
        } else {
            json!({"id": found["value"]})
        });
    }
    match kind {
        "number" => match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(text.parse::<i64>().map_or(n.into(), Value::from)),
            _ => Err(format!("'{text}' is not a number")),
        },
        "user" => Ok(json!({"accountId": text})),
        _ => Ok(value.clone()),
    }
}

/// The `requestFieldValues` to send, and what is wrong with them.
pub fn field_values(fields: &Value, v: &Values) -> Result<(Map<String, Value>, Vec<String>)> {
    let list = field_list(fields)?;
    let mut out = Map::new();
    let mut problems = Vec::new();
    if let Some(s) = v.summary {
        out.insert("summary".into(), s.into());
    }
    if let Some(d) = v.description {
        out.insert("description".into(), d.into());
    }
    for (question, answer) in &v.answers {
        let by_id = list
            .iter()
            .find(|f| f["fieldId"].as_str() == Some(question.as_str()));
        let found: Vec<&Value> = match by_id {
            Some(f) => vec![f],
            None => list
                .iter()
                .filter(|f| {
                    f["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(question))
                })
                .collect(),
        };
        match found.as_slice() {
            [f] => match (f["fieldId"].as_str(), shaped(f, answer)) {
                (Some(id), Ok(value)) => {
                    out.insert(id.into(), value);
                }
                (_, Err(why)) => problems.push(format!("{question}: {why}")),
                (None, Ok(_)) => problems.push(format!("{question}: the field has no ID")),
            },
            [] => problems.push(format!("{question}: not a question of this request type")),
            _ => problems.push(format!(
                "{question}: several questions have this label; use the field ID"
            )),
        }
    }
    out.extend(v.fields.clone());
    for id in out.keys() {
        if !list
            .iter()
            .any(|f| f["fieldId"].as_str() == Some(id.as_str()))
        {
            problems.push(format!("{id}: not a field of this request type"));
        }
    }
    for f in list {
        let has_default = f["defaultValues"].as_array().is_some_and(|d| !d.is_empty());
        if f["required"] != true || has_default {
            continue;
        }
        let Some(id) = f["fieldId"].as_str() else {
            continue;
        };
        let given = out.get(id).is_some_and(|v| match v {
            Value::Null => false,
            Value::String(s) => !s.trim().is_empty(),
            Value::Array(a) => !a.is_empty(),
            _ => true,
        });
        if !given {
            problems.push(format!(
                "{} ({id}) is required",
                f["name"].as_str().unwrap_or(id)
            ));
        }
    }
    Ok((out, problems))
}

/// Checks the values against the type's fields. Creates nothing.
pub fn validate(c: &Client, desk: &str, request_type: &str, v: &Values) -> Result<Value> {
    let (values, problems) = field_values(&fields(c, desk, request_type)?, v)?;
    Ok(json!({
        "valid": problems.is_empty(),
        "problems": problems,
        "serviceDeskId": desk,
        "requestTypeId": request_type,
        "requestFieldValues": values,
    }))
}

pub fn create(
    c: &Client,
    desk: &str,
    request_type: &str,
    v: &Values,
    on_behalf_of: Option<&str>,
) -> Result<Value> {
    let (values, problems) = field_values(&fields(c, desk, request_type)?, v)?;
    if !problems.is_empty() {
        return Err(usage(format!(
            "the request was not sent: {}",
            problems.join("; ")
        )));
    }
    let mut body = json!({
        "serviceDeskId": desk,
        "requestTypeId": request_type,
        "requestFieldValues": values,
    });
    if let Some(who) = on_behalf_of {
        body["raiseOnBehalfOf"] = account(c, who)?.into();
    }
    Ok(clean(c.post(&format!("{API}/request"), &body)?))
}

// --- requests ----------------------------------------------------------------

fn account(c: &Client, who: &str) -> Result<String> {
    let who = who.trim();
    if who.is_empty() {
        return Err(usage("a user can't be empty"));
    }
    let id = resolve_account(c, who)?;
    if id.is_empty() {
        return Err(usage(format!("'{who}' has no account ID")));
    }
    Ok(id)
}

pub fn get(c: &Client, request: &str) -> Result<Value> {
    Ok(clean(c.get(&request_path(request)?, &[])?))
}

#[derive(Debug, Default)]
pub struct RequestQuery<'a> {
    /// `open`, `closed` or `all`.
    pub status: Option<&'a str>,
    pub desk: Option<&'a str>,
    pub request_type: Option<&'a str>,
    pub search: Option<&'a str>,
}

pub fn requests(
    c: &Client,
    o: &RequestQuery,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Listed> {
    if o.request_type.is_some() && o.desk.is_none() {
        return Err(usage("--request-type needs --service-desk"));
    }
    let status = match o.status {
        None => None,
        Some("open") => Some("OPEN_REQUESTS"),
        Some("closed") => Some("CLOSED_REQUESTS"),
        Some("all") => Some("ALL_REQUESTS"),
        Some(other) => {
            return Err(usage(format!(
                "--status '{other}' is not open, closed or all"
            )));
        }
    };
    let q: Vec<(&str, &str)> = [
        ("requestStatus", status),
        ("serviceDeskId", o.desk),
        ("requestTypeId", o.request_type),
        ("searchTerm", o.search),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k, v)))
    .collect();
    list(c, &format!("{API}/request"), &q, &[], limit, cursor)
}

pub fn comments(c: &Client, request: &str, limit: usize, cursor: Option<&str>) -> Result<Listed> {
    let path = format!("{}/comment", request_path(request)?);
    list(c, &path, &[], &[], limit, cursor)
}

/// `internal` is always sent, so a note can't turn public through a default.
pub fn comment_create(c: &Client, request: &str, body: &str, internal: bool) -> Result<Value> {
    if body.trim().is_empty() {
        return Err(usage("the comment can't be empty"));
    }
    let path = format!("{}/comment", request_path(request)?);
    Ok(clean(c.post(
        &path,
        &json!({"body": body, "public": !internal}),
    )?))
}

pub fn participants(c: &Client, request: &str) -> Result<Vec<Value>> {
    let path = format!("{}/participant", request_path(request)?);
    all(c, &path, &[])
}

pub fn participants_change(c: &Client, request: &str, who: &[String], add: bool) -> Result<Value> {
    if who.is_empty() {
        return Err(usage("pass at least one --participant"));
    }
    let path = format!("{}/participant", request_path(request)?);
    let ids = who
        .iter()
        .map(|w| account(c, w))
        .collect::<Result<Vec<_>>>()?;
    let method = if add { "POST" } else { "DELETE" };
    let v = c.request(method, &path, &[], Some(&json!({"accountIds": ids})))?;
    if add {
        return Ok(clean(v)["values"].clone());
    }
    Ok(ids
        .iter()
        .map(|id| json!({"id": id, "deleted": true}))
        .collect())
}

pub fn transitions(c: &Client, request: &str) -> Result<Vec<Value>> {
    let path = format!("{}/transition", request_path(request)?);
    all(c, &path, &[])
}

/// `transition` is an ID or exact name of one the caller may make.
pub fn transition(
    c: &Client,
    request: &str,
    transition: &str,
    comment: Option<&str>,
) -> Result<Value> {
    let key = request_key(request)?;
    let path = format!("{}/transition", request_path(&key)?);
    let list = all(c, &path, &[])?;
    let wanted = transition.trim();
    let by_id: Vec<&Value> = list
        .iter()
        .filter(|t| id_text(&t["id"]).as_deref() == Some(wanted))
        .collect();
    let found = if by_id.is_empty() {
        list.iter()
            .filter(|t| {
                t["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
            })
            .collect()
    } else {
        by_id
    };
    let [t] = found.as_slice() else {
        return Err(usage(format!(
            "'{wanted}' matches {} transitions; this request allows {}",
            found.len(),
            list_of(&list, "name")
        )));
    };
    let mut body = json!({"id": t["id"]});
    if let Some(text) = comment {
        body["additionalComment"] = json!({"body": text});
    }
    c.post(&path, &body)?;
    Ok(json!({"request": key, "transitionId": t["id"], "transition": t["name"]}))
}

pub fn approvals(c: &Client, request: &str) -> Result<Vec<Value>> {
    let path = format!("{}/approval", request_path(request)?);
    Ok(all(c, &path, &[])?.into_iter().map(clean).collect())
}

fn approval_path(request: &str, approval: &str) -> Result<String> {
    let id = approval.trim();
    if !digits(id) {
        return Err(usage(format!("'{id}' is not an approval ID")));
    }
    Ok(format!("{}/approval/{id}", request_path(request)?))
}

pub fn approval_get(c: &Client, request: &str, approval: &str) -> Result<Value> {
    Ok(clean(c.get(&approval_path(request, approval)?, &[])?))
}

/// Without an ID, the one pending approval the caller can answer.
pub fn approval_decide(
    c: &Client,
    request: &str,
    approval: Option<&str>,
    approve: bool,
) -> Result<Value> {
    let id = match approval {
        Some(id) => id.to_string(),
        None => {
            let path = format!("{}/approval", request_path(request)?);
            let pending: Vec<String> = all(c, &path, &[])?
                .iter()
                .filter(|a| {
                    a["finalDecision"]
                        .as_str()
                        .is_some_and(|d| d.eq_ignore_ascii_case("pending"))
                })
                .filter(|a| a["canAnswerApproval"] != false)
                .filter_map(|a| id_text(&a["id"]))
                .collect();
            match pending.as_slice() {
                [id] => id.clone(),
                [] => return Err(usage("there is no pending approval you can answer")),
                many => {
                    return Err(usage(format!(
                        "several approvals are pending ({}); pass --approval-id",
                        many.join(", ")
                    )));
                }
            }
        }
    };
    let decision = if approve { "approve" } else { "decline" };
    Ok(clean(c.post(
        &approval_path(request, &id)?,
        &json!({"decision": decision}),
    )?))
}

/// A request's SLAs.
pub fn slas(c: &Client, request: &str) -> Result<Vec<Value>> {
    let path = format!("{}/sla", request_path(request)?);
    all(c, &path, &[])
}

/// One SLA of a request, by ID or name.
pub fn sla(c: &Client, request: &str, which: &str) -> Result<Value> {
    let which = which.trim();
    slas(c, request)?
        .into_iter()
        .find(|s| {
            id_text(&s["id"]).as_deref() == Some(which)
                || s["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(which))
        })
        .ok_or_else(|| {
            ApiError::new(
                ErrorKind::NotFound,
                format!("the request has no SLA '{which}'"),
            )
            .into()
        })
}

//! Sprint lifecycle, board details, and the Agile fields of `workitem
//! create|update` (sprint, story points, estimates, team).

use super::{
    Paged, board_values, enc, issue_project, issue_row, none_or, paginate_values, software_issues,
    trailing_number,
};
use crate::client::{ApiError, Client};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const AGILE: &str = "/rest/agile/1.0";
/// The issue lists that replace the deprecated Agile ones.
const SOFTWARE: &str = "/rest/software/1.0";
/// The most work items one Agile move call accepts.
const MOVE_BATCH: usize = 50;

fn usage(message: String) -> anyhow::Error {
    ApiError::usage(message).into()
}

/// Boards that can't answer (no sprints, a broken filter, no permission) are
/// skipped when scanning a project's boards; other failures are not.
fn board_cannot_answer(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| matches!(a.status, Some(400 | 403 | 404)))
}

fn all_boards(c: &Client, project: &str) -> Result<Vec<u64>> {
    let o = super::BoardQuery {
        project: Some(project),
        limit: usize::MAX,
        ..Default::default()
    };
    Ok(board_values(c, &o)?
        .values
        .iter()
        .filter_map(|b| b["id"].as_u64())
        .collect())
}

fn candidates(found: &BTreeMap<u64, String>) -> String {
    let list: Vec<String> = found
        .iter()
        .map(|(id, name)| format!("{id} ({name})"))
        .collect();
    list.join(", ")
}

// --- references --------------------------------------------------------------

/// A sprint ID given as a number, ARI or URL. Stricter than
/// [`super::sprint_ref`], because writes use it: a name like `Team/5` must
/// not be read as sprint 5.
pub fn sprint_id(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    if s.starts_with("ari:") {
        return trailing_number(s)?.parse().ok();
    }
    if s.starts_with("https://") || s.starts_with("http://") {
        for marker in ["sprint=", "/sprints/", "/sprint/"] {
            if let Some(i) = s.find(marker) {
                let id: String = s[i + marker.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                return id.parse().ok();
            }
        }
    }
    None
}

/// The one sprint with this exact name (ignoring case) on the boards.
/// `state` narrows the search.
fn sprint_named(c: &Client, boards: &[u64], name: &str, state: Option<&str>) -> Result<u64> {
    let mut found = BTreeMap::new();
    let mut all = BTreeMap::new();
    for board in boards {
        let q: Vec<(&str, &str)> = state.iter().map(|s| ("state", *s)).collect();
        let path = format!("{AGILE}/board/{board}/sprint");
        let sprints = match paginate_values(c, &path, &q, 0, usize::MAX) {
            Ok(p) => p.values,
            Err(e) if boards.len() > 1 && board_cannot_answer(&e) => continue,
            Err(e) => return Err(e),
        };
        for s in sprints {
            let (Some(id), Some(n)) = (s["id"].as_u64(), s["name"].as_str()) else {
                continue;
            };
            if n.eq_ignore_ascii_case(name.trim()) {
                found.insert(id, n.to_string());
            }
            all.insert(id, n.to_string());
        }
    }
    match found.len() {
        1 => found
            .keys()
            .next()
            .copied()
            .ok_or_else(|| anyhow!("no sprint")),
        0 if all.is_empty() => Err(usage(format!(
            "no sprint named '{name}'; pass the sprint ID"
        ))),
        0 => Err(usage(format!(
            "no sprint named '{name}'; there are {}",
            candidates(&all)
        ))),
        _ => Err(usage(format!(
            "several sprints are named '{name}': {}; pass the ID",
            candidates(&found)
        ))),
    }
}

/// A sprint from an ID, ARI or URL, or from its exact name on `board`.
pub fn resolve_sprint(c: &Client, s: &str, board: Option<&str>) -> Result<u64> {
    if let Some(id) = sprint_id(s) {
        return Ok(id);
    }
    let Some(board) = board else {
        return Err(usage(format!(
            "'{s}' is not a sprint ID, ARI or URL; pass --board-id to look it up by name"
        )));
    };
    sprint_named(c, &[super::resolve_board(c, board)?], s, None)
}

// --- sprint lifecycle --------------------------------------------------------

/// The fields of a sprint create or partial update; times are epoch ms.
#[derive(Debug, Default)]
pub struct Sprint<'a> {
    pub name: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub state: Option<&'a str>,
}

impl Sprint<'_> {
    fn body(&self) -> Result<Map<String, Value>> {
        if let (Some(start), Some(end)) = (self.start, self.end)
            && end <= start
        {
            return Err(usage("the sprint must end after it starts".into()));
        }
        let mut m = Map::new();
        for (k, v) in [
            ("name", self.name),
            ("goal", self.goal),
            ("state", self.state),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v.into());
            }
        }
        for (k, v) in [("startDate", self.start), ("endDate", self.end)] {
            if let Some(ms) = v {
                m.insert(
                    k.into(),
                    crate::time::jira_timestamp(ms).replace("+0000", "Z").into(),
                );
            }
        }
        Ok(m)
    }
}

fn sprint_path(id: u64) -> String {
    format!("{AGILE}/sprint/{id}")
}

pub fn sprint_get(c: &Client, id: u64) -> Result<Value> {
    c.get(&sprint_path(id), &[])
}

pub fn sprint_create(c: &Client, board: u64, s: &Sprint) -> Result<Value> {
    let mut body = s.body()?;
    body.insert("originBoardId".into(), board.into());
    c.post(&format!("{AGILE}/sprint"), &Value::Object(body))
}

/// A partial update. `PUT` on this resource would clear every field left out.
pub fn sprint_update(c: &Client, id: u64, s: &Sprint) -> Result<Value> {
    let body = s.body()?;
    if body.is_empty() {
        return Err(usage(
            "nothing to update: pass --name, --goal, --start-date, --end-date or --state".into(),
        ));
    }
    c.post(&sprint_path(id), &Value::Object(body))
}

/// Activate a sprint. Jira needs both dates, so the start defaults to the
/// sprint's own or `now`, and the end to the sprint's own unless
/// `duration_ms` is given.
pub fn sprint_start(
    c: &Client,
    id: u64,
    s: &Sprint,
    duration_ms: Option<i64>,
    now: i64,
) -> Result<Value> {
    let saved = if s.start.is_some() && (s.end.is_some() || duration_ms.is_some()) {
        Value::Null
    } else {
        sprint_get(c, id)?
    };
    let saved_ms = |k: &str| {
        saved[k]
            .as_str()
            .and_then(|v| crate::time::epoch_millis(v).ok())
    };
    let start = s.start.or_else(|| saved_ms("startDate")).unwrap_or(now);
    let end = s
        .end
        .or_else(|| duration_ms.and_then(|d| start.checked_add(d)))
        .or_else(|| saved_ms("endDate"))
        .ok_or_else(|| {
            usage("starting a sprint needs an end: pass --end-date or --duration 2w".into())
        })?;
    sprint_update(
        c,
        id,
        &Sprint {
            name: s.name,
            goal: s.goal,
            start: Some(start),
            end: Some(end),
            state: Some("active"),
        },
    )
}

/// Where a sprint move goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Sprint(u64),
    Backlog,
}

pub fn move_issues(c: &Client, to: Destination, keys: &[String]) -> Result<()> {
    let path = match to {
        Destination::Sprint(id) => format!("{}/issue", sprint_path(id)),
        Destination::Backlog => format!("{AGILE}/backlog/issue"),
    };
    for chunk in keys.chunks(MOVE_BATCH) {
        c.post(&path, &json!({"issues": chunk}))?;
    }
    Ok(())
}

/// Every work item in the sprint that is not done. Subtasks are left out:
/// they follow their parent and Jira refuses to move them by themselves.
fn unfinished(c: &Client, id: u64) -> Result<Vec<String>> {
    let q = [
        ("fields", "status"),
        (
            "jql",
            "statusCategory != Done AND issuetype not in subTaskIssueTypes()",
        ),
    ];
    let path = format!("{SOFTWARE}/sprint/{id}/issue");
    software_issues(c, &path, &q, 0, usize::MAX)?
        .iter()
        .map(|issue| {
            issue["key"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("an issue in sprint {id} has no key"))
        })
        .collect()
}

/// Close an active sprint, first moving what isn't done to `to`.
pub fn sprint_complete(c: &Client, id: u64, s: &Sprint, to: Option<Destination>) -> Result<Value> {
    let close = Sprint {
        state: Some("closed"),
        ..*s
    };
    // Check everything that can be checked before anything moves.
    close.body()?;
    if to == Some(Destination::Sprint(id)) {
        return Err(usage(
            "unfinished work can't move into the sprint being completed".into(),
        ));
    }
    let mut moved = Vec::new();
    if let Some(to) = to {
        let state = sprint_get(c, id)?;
        if state["state"] != "active" {
            return Err(usage(format!(
                "sprint {id} is {}, and only an active sprint can be completed",
                state["state"].as_str().unwrap_or("not active")
            )));
        }
        if let Destination::Sprint(next) = to
            && sprint_get(c, next)?["state"] == "closed"
        {
            return Err(usage(format!(
                "sprint {next} is closed; pick another destination"
            )));
        }
        moved = unfinished(c, id)?;
        move_issues(c, to, &moved)?;
    }
    let mut v = sprint_update(c, id, &close)?;
    if to.is_some() && v.is_object() {
        v["movedIssues"] = moved.into();
    }
    Ok(v)
}

pub fn sprint_delete(c: &Client, id: u64) -> Result<Value> {
    c.delete(&sprint_path(id), &[])?;
    Ok(json!({"id": id, "deleted": true}))
}

// --- boards ------------------------------------------------------------------

fn board_path(id: u64) -> String {
    format!("{AGILE}/board/{id}")
}

/// A board with its `configuration` (columns, estimation).
pub fn board_get(c: &Client, id: u64) -> Result<Value> {
    let mut v = c.get(&board_path(id), &[])?;
    let configuration = c.get(&format!("{}/configuration", board_path(id)), &[])?;
    if v.is_object() {
        v["configuration"] = configuration;
    }
    super::strip_media(&mut v);
    Ok(v)
}

pub fn board_create(c: &Client, name: &str, kind: super::BoardType, filter: u64) -> Result<Value> {
    c.post(
        &format!("{AGILE}/board"),
        &json!({"name": name, "type": kind.as_str(), "filterId": filter}),
    )
}

pub fn board_delete(c: &Client, id: u64) -> Result<Value> {
    c.delete(&board_path(id), &[])?;
    Ok(json!({"id": id, "deleted": true}))
}

pub fn board_projects(c: &Client, id: u64, start: usize, limit: usize) -> Result<Paged> {
    paginate_values(c, &format!("{}/project", board_path(id)), &[], start, limit)
}

pub fn quick_filters(c: &Client, id: u64, start: usize, limit: usize) -> Result<Paged> {
    paginate_values(
        c,
        &format!("{}/quickfilter", board_path(id)),
        &[],
        start,
        limit,
    )
}

pub fn quick_filter(c: &Client, board: u64, id: &str) -> Result<Value> {
    c.get(
        &format!("{}/quickfilter/{}", board_path(board), enc(id)),
        &[],
    )
}

/// `jql` narrowed by each quick filter, keeping a trailing ORDER BY last.
fn narrowed(filters: &[String], jql: Option<&str>) -> Option<String> {
    let jql = jql.unwrap_or_default().trim();
    if filters.is_empty() {
        return (!jql.is_empty()).then(|| jql.to_string());
    }
    let split = jql
        .as_bytes()
        .windows(8)
        .rposition(|w| w.eq_ignore_ascii_case(b"order by"))
        .unwrap_or(jql.len());
    let (wher, order) = jql.split_at(split);
    let mut parts: Vec<String> = filters.iter().map(|f| format!("({f})")).collect();
    if !wher.trim().is_empty() {
        parts.push(format!("({})", wher.trim()));
    }
    let mut out = parts.join(" AND ");
    if !order.is_empty() {
        out = format!("{out} {order}");
    }
    Some(out)
}

/// A board's backlog, narrowed by quick filters and JQL.
pub fn backlog(
    c: &Client,
    board: u64,
    active_filters: &[String],
    jql: Option<&str>,
    start: usize,
    limit: usize,
) -> Result<Value> {
    let mut filters = Vec::new();
    for id in active_filters {
        let f = quick_filter(c, board, id)?;
        let q = f["jql"]
            .as_str()
            .ok_or_else(|| anyhow!("quick filter {id} has no JQL"))?;
        filters.push(q.to_string());
    }
    let jql = narrowed(&filters, jql);
    let fields = super::LIST_FIELDS.join(",");
    let path = format!("{SOFTWARE}/board/{board}/backlog");
    let mut q = vec![("fields", fields.as_str())];
    if let Some(j) = &jql {
        q.push(("jql", j));
    }
    let issues: Vec<Value> = software_issues(c, &path, &q, start, limit)?
        .iter()
        .map(|i| issue_row(c, i, false))
        .collect();
    Ok(json!({"issues": issues, "startAt": start, "maxResults": limit}))
}

// --- create and update fields ------------------------------------------------

/// The work item being written.
#[derive(Clone, Copy, Debug)]
pub enum On<'a> {
    Issue(&'a str),
    /// A work item about to be created in this space.
    Space(&'a str),
}

/// The Agile flags of `workitem create|update`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Planning<'a> {
    pub sprint: Option<&'a str>,
    pub story_points: Option<&'a str>,
    pub original_estimate: Option<&'a str>,
    pub remaining_estimate: Option<&'a str>,
    pub team: Option<&'a str>,
}

/// [`Planning`] resolved to what gets written.
#[derive(Debug, Default)]
pub struct Planned {
    pub fields: Map<String, Value>,
    pub sprint: Option<Destination>,
}

/// Resolve every flag, making only reads, so nothing is written when one of
/// them is wrong.
pub fn plan(c: &Client, on: On, p: &Planning) -> Result<Planned> {
    let mut out = Planned::default();
    // Values that need no request are checked first.
    let mut tracking = Map::new();
    for (key, value) in [
        ("originalEstimate", p.original_estimate),
        ("remainingEstimate", p.remaining_estimate),
    ] {
        if let Some(v) = value {
            tracking.insert(key.into(), estimate(v)?.into());
        }
    }
    if !tracking.is_empty() {
        out.fields
            .insert("timetracking".into(), Value::Object(tracking));
    }
    let points = p.story_points.map(number_or_none).transpose()?;

    let mut project = None;
    if let Some(s) = p.sprint {
        out.sprint = Some(match (none_or(s.trim()), sprint_id(s)) {
            (None, _) => Destination::Backlog,
            (Some(_), Some(id)) => Destination::Sprint(id),
            (Some(name), None) => {
                let boards = all_boards(c, project_of(c, on, &mut project)?)?;
                Destination::Sprint(sprint_named(c, &boards, name, Some("active,future"))?)
            }
        });
    }
    if let Some(points) = points {
        let project = project_of(c, on, &mut project)?.to_string();
        out.fields.insert(points_field(c, on, &project)?, points);
    }
    if let Some(team) = p.team {
        let value = match none_or(team.trim()) {
            None => Value::Null,
            Some("") => return Err(usage("--team needs a team ID, ARI or none".into())),
            Some(t) if t.starts_with("ari:") => t.rsplit('/').next().unwrap_or(t).into(),
            Some(t) => t.into(),
        };
        out.fields.insert(team_field(c, on)?, value);
    }
    Ok(out)
}

fn project_of<'p>(c: &Client, on: On, cache: &'p mut Option<String>) -> Result<&'p str> {
    if cache.is_none() {
        *cache = Some(match on {
            On::Issue(key) => issue_project(c, key)?,
            On::Space(space) => space.to_string(),
        });
    }
    Ok(cache.as_deref().unwrap_or_default())
}

/// A Jira duration. Plain numbers are seconds; Jira itself would
/// read them in the site's default unit, so they are sent as minutes.
fn estimate(v: &str) -> Result<String> {
    let v = v.trim();
    if v.is_empty() {
        return Err(usage(
            "an estimate can't be empty; use a duration like 2h".into(),
        ));
    }
    match v.parse::<u64>() {
        Ok(s) if s % 60 == 0 => Ok(format!("{}m", s / 60)),
        Ok(s) => Err(usage(format!(
            "{s} seconds is not a whole number of minutes, which is the finest unit Jira stores"
        ))),
        Err(_) => Ok(v.to_string()),
    }
}

fn number_or_none(v: &str) -> Result<Value> {
    let v = v.trim();
    if none_or(v).is_none() {
        return Ok(Value::Null);
    }
    if let Ok(n) = v.parse::<u64>() {
        return Ok(n.into());
    }
    match v.parse::<f64>() {
        Ok(n) if n.is_finite() && n >= 0.0 => Ok(n.into()),
        _ => Err(usage(format!(
            "--story-points '{v}' is not a number or none"
        ))),
    }
}

/// The boards' estimation field, else a field with one of Jira's usual
/// names; several candidates are narrowed to those on the edit screen.
fn points_field(c: &Client, on: On, project: &str) -> Result<String> {
    let mut found = BTreeMap::new();
    for board in all_boards(c, project)? {
        let conf = match c.get(&format!("{}/configuration", board_path(board)), &[]) {
            Ok(v) => v,
            Err(e) if board_cannot_answer(&e) => continue,
            Err(e) => return Err(e),
        };
        let field = &conf["estimation"]["field"];
        // Boards can also estimate in time, which is not story points.
        if let Some(id) = field["fieldId"]
            .as_str()
            .filter(|f| f.starts_with("customfield_"))
        {
            let name = field["displayName"].as_str().unwrap_or(id);
            found.insert(id.to_string(), name.to_string());
        }
    }
    if found.is_empty() {
        for f in c
            .get("/rest/api/3/field", &[])?
            .as_array()
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(name)) = (f["id"].as_str(), f["name"].as_str()) else {
                continue;
            };
            if ["story point estimate", "story points"].contains(&name.to_lowercase().as_str()) {
                found.insert(id.to_string(), name.to_string());
            }
        }
    }
    if found.len() > 1
        && let On::Issue(key) = on
    {
        let meta = c.get(&format!("/rest/api/3/issue/{}/editmeta", enc(key)), &[])?;
        let editable: BTreeMap<String, String> = found
            .iter()
            .filter(|(id, _)| meta["fields"].get(id.as_str()).is_some())
            .map(|(id, name)| (id.clone(), name.clone()))
            .collect();
        if !editable.is_empty() {
            found = editable;
        }
    }
    one_field(found, "story points")
}

fn team_field(c: &Client, on: On) -> Result<String> {
    let mut found = BTreeMap::new();
    for f in c
        .get("/rest/api/3/field", &[])?
        .as_array()
        .into_iter()
        .flatten()
    {
        let is_team = f["schema"]["custom"]
            .as_str()
            .is_some_and(|s| s.ends_with(":atlassian-team"));
        if let Some(id) = f["id"].as_str().filter(|_| is_team) {
            found.insert(id.to_string(), f["name"].as_str().unwrap_or(id).to_string());
        }
    }
    if found.len() > 1
        && let On::Issue(key) = on
    {
        let meta = c.get(&format!("/rest/api/3/issue/{}/editmeta", enc(key)), &[])?;
        found.retain(|id, _| meta["fields"].get(id.as_str()).is_some());
    }
    one_field(found, "team")
}

fn one_field(found: BTreeMap<String, String>, what: &str) -> Result<String> {
    let list: Vec<String> = found.iter().map(|(id, n)| format!("{id} ({n})")).collect();
    match found.len() {
        1 => Ok(found.into_keys().next().unwrap_or_default()),
        0 => Err(usage(format!(
            "no {what} field found; set it with --field ID=VALUE"
        ))),
        _ => Err(usage(format!(
            "several {what} fields: {}; set one with --field ID=VALUE",
            list.join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprint_ids_are_strict() {
        assert_eq!(sprint_id("42"), Some(42));
        assert_eq!(
            sprint_id("ari:cloud:jira-software:site:sprint/42"),
            Some(42)
        );
        assert_eq!(sprint_id("https://x.test/issues?sprint=42"), Some(42));
        for name in ["Team/5", "Sprint 5", "web /sprint/7"] {
            assert_eq!(sprint_id(name), None, "{name}");
        }
    }

    #[test]
    fn quick_filters_keep_order_by_last() {
        let f = ["assignee = currentUser()".to_string()];
        assert_eq!(narrowed(&[], Some("a = 1")).as_deref(), Some("a = 1"));
        assert_eq!(
            narrowed(&f, Some("a = 1 OR b = 2 order by rank")).as_deref(),
            Some("(assignee = currentUser()) AND (a = 1 OR b = 2) order by rank")
        );
    }

    #[test]
    fn estimates_and_points() {
        assert_eq!(estimate("7200").unwrap(), "120m");
        assert_eq!(estimate("1d 4h").unwrap(), "1d 4h");
        assert!(estimate("90s").is_ok() && estimate("61").is_err());
        assert_eq!(number_or_none("5").unwrap(), json!(5));
        assert_eq!(number_or_none("0.5").unwrap(), json!(0.5));
        assert_eq!(number_or_none("none").unwrap(), Value::Null);
        assert!(number_or_none("NaN").is_err());
    }
}

//! Pull request lifecycle: update and reviewers, merge with `--wait`,
//! mergeability, merge-time and cross-repository queries, inbox, for-commit.

use super::{PrMerge, PrQuery, and, enc, paginate, paginate_more, pr_filter, pr_merge_body};
use crate::client::{ApiError, Client, ErrorKind};
use crate::markup::quote;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// How many merged pull requests a merge-date filter checks at most. Each
/// costs at least one activity request.
pub const MERGED_SCAN: usize = 200;
/// How many workspace members are read to resolve a name.
const MEMBER_SCAN: usize = 5000;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn status_of(e: &anyhow::Error) -> Option<u16> {
    ApiError::find(e).and_then(|a| a.status)
}

fn pr_path(path: &str, id: u64) -> String {
    format!("{path}/pullrequests/{id}")
}

// --- users -------------------------------------------------------------------

/// How a reviewer was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum User<'a> {
    /// `me`, a `{uuid}`, an account ID, a nickname or a display name.
    Any(&'a str),
    Uuid(&'a str),
    AccountId(&'a str),
}

fn is_account_id(s: &str) -> bool {
    !s.contains(' ')
        && (s.contains(':') || (s.len() == 24 && s.bytes().all(|b| b.is_ascii_hexdigit())))
}

fn braced(uuid: &str) -> String {
    if uuid.starts_with('{') {
        uuid.to_string()
    } else {
        format!("{{{uuid}}}")
    }
}

fn named(u: &Value, name: &str) -> bool {
    ["nickname", "display_name"]
        .iter()
        .any(|k| u[*k].as_str().is_some_and(|v| v.eq_ignore_ascii_case(name)))
}

/// Whether a user object is the user given.
fn matches(u: &Value, who: User) -> bool {
    match who {
        User::Uuid(id) => u["uuid"] == braced(id).as_str(),
        User::AccountId(id) => u["account_id"] == id,
        User::Any(s) if s.starts_with('{') => u["uuid"] == s,
        User::Any(s) if is_account_id(s) => u["account_id"] == s,
        User::Any(s) => named(u, s),
    }
}

/// The calling user's UUID.
pub fn my_uuid(c: &Client) -> Result<String> {
    c.get("/user", &[])?["uuid"]
        .as_str()
        .filter(|u| !u.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("Bitbucket returned no UUID for the current user"))
}

fn workspace(path: &str) -> &str {
    path.trim_start_matches("/repositories/")
        .split('/')
        .next()
        .unwrap_or_default()
}

/// `{uuid}` or `{account_id}`, as reviewer lists and filters take them.
/// Names are looked up among the repository's workspace members.
pub fn resolve_user(c: &Client, path: &str, who: User) -> Result<Value> {
    match who {
        User::Uuid(id) => Ok(json!({"uuid": braced(id)})),
        User::AccountId(id) => Ok(json!({"account_id": id})),
        User::Any(s) if s.starts_with('{') => Ok(json!({"uuid": s})),
        User::Any(s) if is_account_id(s) => Ok(json!({"account_id": s})),
        User::Any(s) if s.eq_ignore_ascii_case("me") => Ok(json!({"uuid": my_uuid(c)?})),
        User::Any(s) => {
            // The workspace segment of a repository path is already encoded.
            let members = format!("/workspaces/{}/members", workspace(path));
            let found: Vec<Value> = paginate(c, &members, &[], MEMBER_SCAN)?
                .into_iter()
                .map(|m| m["user"].clone())
                .filter(|u| named(u, s))
                .collect();
            match found.as_slice() {
                [u] => Ok(json!({"uuid": u["uuid"]
                    .as_str()
                    .ok_or_else(|| anyhow!("'{s}' has no UUID"))?})),
                [] => Err(usage(format!(
                    "no workspace member is called '{s}'; pass an account ID or {{uuid}}"
                ))),
                many => {
                    let list: Vec<&str> = many.iter().filter_map(|u| u["uuid"].as_str()).collect();
                    Err(usage(format!(
                        "several workspace members are called '{s}': {}; pass the UUID",
                        list.join(", ")
                    )))
                }
            }
        }
    }
}

/// Reviewers for a new pull request, optionally with the repository's
/// effective default reviewers (never the author, whom Bitbucket refuses).
pub fn new_reviewers(
    c: &Client,
    path: &str,
    users: &[User],
    defaults: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let (mut uuids, mut accounts) = (Vec::new(), Vec::new());
    for who in users {
        let u = resolve_user(c, path, *who)?;
        let (list, id) = match u["uuid"].as_str() {
            Some(id) => (&mut uuids, id),
            None => (&mut accounts, u["account_id"].as_str().unwrap_or_default()),
        };
        if !list.iter().any(|x| x == id) {
            list.push(id.to_string());
        }
    }
    if defaults {
        let me = my_uuid(c)?;
        let path = format!("{path}/effective-default-reviewers");
        for d in paginate(c, &path, &[], MEMBER_SCAN)? {
            let u = &d["user"];
            let Some(id) = u["uuid"].as_str() else {
                continue;
            };
            let account = u["account_id"].as_str();
            let have = uuids.iter().any(|x| x == id)
                || accounts.iter().any(|x| Some(x.as_str()) == account);
            if id != me && !have {
                uuids.push(id.to_string());
            }
        }
    }
    Ok((uuids, accounts))
}

// --- update ------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct PrUpdate<'a> {
    pub title: Option<&'a str>,
    /// `Some("")` clears it.
    pub description: Option<&'a str>,
    pub dest: Option<&'a str>,
    /// Replace the reviewers; empty clears them.
    pub set: Option<Vec<User<'a>>>,
    pub add: Vec<User<'a>>,
    pub remove: Vec<User<'a>>,
    pub close_source_branch: Option<bool>,
    pub draft: Option<bool>,
}

/// PUT replaces what it is given and may drop what it isn't, so the current
/// values of every editable field go back with the change.
pub fn pr_update(c: &Client, path: &str, id: u64, u: &PrUpdate) -> Result<Value> {
    if u.title.is_none()
        && u.description.is_none()
        && u.dest.is_none()
        && u.set.is_none()
        && u.add.is_empty()
        && u.remove.is_empty()
        && u.close_source_branch.is_none()
        && u.draft.is_none()
    {
        return Err(usage("nothing to update: pass a field or reviewer change"));
    }
    if u.title.is_some_and(|t| t.trim().is_empty()) {
        return Err(usage("--title can't be empty"));
    }
    let path_pr = pr_path(path, id);
    let now = c.get(&path_pr, &[])?;
    let title = u
        .title
        .or(now["title"].as_str())
        .ok_or_else(|| anyhow!("pull request {id} came back without a title; not changing it"))?;
    let current = now["reviewers"].as_array().ok_or_else(|| {
        anyhow!("pull request {id} came back without its reviewers; not changing it")
    })?;

    // Kept with both identifiers so additions dedupe whichever way they are given.
    let mut reviewers: Vec<Value> = match &u.set {
        Some(set) => set
            .iter()
            .map(|w| resolve_user(c, path, *w))
            .collect::<Result<_>>()?,
        None => current.clone(),
    };
    for who in &u.remove {
        let me;
        let who = match who {
            User::Any(s) if s.eq_ignore_ascii_case("me") => {
                me = my_uuid(c)?;
                User::Uuid(&me)
            }
            other => *other,
        };
        let hits = reviewers.iter().filter(|r| matches(r, who)).count();
        if hits != 1 {
            let label = match who {
                User::Any(s) | User::Uuid(s) | User::AccountId(s) => s,
            };
            return Err(usage(if hits == 0 {
                format!("'{label}' is not a reviewer of pull request {id}")
            } else {
                format!("several reviewers are called '{label}'; pass a UUID or account ID")
            }));
        }
        reviewers.retain(|r| !matches(r, who));
    }
    for who in &u.add {
        let new = resolve_user(c, path, *who)?;
        let have = reviewers.iter().any(|r| {
            ["uuid", "account_id"]
                .iter()
                .any(|k| new[*k].is_string() && r[*k] == new[*k])
        });
        if !have {
            reviewers.push(new);
        }
    }
    let mut sent = Vec::new();
    for r in &reviewers {
        let item = if r["uuid"].is_string() {
            json!({"uuid": r["uuid"]})
        } else if r["account_id"].is_string() {
            json!({"account_id": r["account_id"]})
        } else {
            return Err(anyhow!(
                "a reviewer has no identifier; not changing pull request {id}"
            ));
        };
        if !sent.contains(&item) {
            sent.push(item);
        }
    }

    let mut req = json!({"title": title, "reviewers": sent});
    if let Some(d) = u.description.or(now["description"].as_str()) {
        req["description"] = d.into();
    }
    if let Some(b) = u.dest.or(now["destination"]["branch"]["name"].as_str()) {
        req["destination"] = json!({"branch": {"name": b}});
    }
    if let Some(b) = u
        .close_source_branch
        .or(now["close_source_branch"].as_bool())
    {
        req["close_source_branch"] = b.into();
    }
    if let Some(b) = u.draft.or(now["draft"].as_bool()) {
        req["draft"] = b.into();
    }
    c.put(&path_pr, &req)
}

pub fn remove_request_changes(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.delete(&format!("{}/request-changes", pr_path(path, id)), &[])?;
    Ok(json!({"id": id, "changes_requested": false}))
}

// --- merge -------------------------------------------------------------------

/// How long and how often a merge is polled.
#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub timeout: Duration,
    pub interval: Duration,
}

impl Default for Wait {
    fn default() -> Self {
        Wait {
            timeout: Duration::from_secs(90),
            interval: Duration::from_secs(2),
        }
    }
}

/// `pull-requests merge`. A synchronous merge that times out on Bitbucket
/// (555) is retried as an asynchronous one, unless it went through anyway.
pub fn merge(c: &Client, path: &str, id: u64, o: &PrMerge, wait: Option<Wait>) -> Result<Value> {
    let url = format!("{}/merge", pr_path(path, id));
    let body = pr_merge_body(o);
    let send = |asynchronous: bool| {
        let q: &[(&str, &str)] = if asynchronous {
            &[("async", "true")]
        } else {
            &[]
        };
        c.request_response("POST", &url, q, Some(&body))
    };
    let resp = match send(wait.is_some()) {
        Err(e) if status_of(&e) == Some(555) => {
            let pr = c.get(&pr_path(path, id), &[])?;
            if pr["state"] == "MERGED" {
                return Ok(pr);
            }
            send(true)?
        }
        other => other?,
    };
    if resp.status != 202 {
        return Ok(resp.body);
    }
    let task = resp
        .location
        .as_deref()
        .and_then(|l| l.split(['?', '#']).next())
        .and_then(|l| l.trim_end_matches('/').rsplit('/').next())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            anyhow!("Bitbucket started merging pull request {id} but gave no task to follow; check its state before retrying")
        })?;
    match wait {
        Some(w) => wait_for_merge(c, path, id, task, w),
        None => Ok(json!({"task_status": "PENDING", "task_id": task, "pull_request": id})),
    }
}

pub fn merge_status(c: &Client, path: &str, id: u64, task: &str) -> Result<Value> {
    c.get(
        &format!("{}/merge/task-status/{}", pr_path(path, id), enc(task)),
        &[],
    )
}

fn wait_for_merge(c: &Client, path: &str, id: u64, task: &str, w: Wait) -> Result<Value> {
    let start = Instant::now();
    loop {
        let v = merge_status(c, path, id, task)?;
        match v["task_status"].as_str() {
            Some("SUCCESS") => {
                return match &v["merge_result"] {
                    r @ Value::Object(_) => Ok(r.clone()),
                    _ => c.get(&pr_path(path, id), &[]),
                };
            }
            Some("PENDING") => {}
            other => {
                return Err(anyhow!(
                    "merging pull request {id} ended as {}",
                    other.unwrap_or("an unknown state")
                ));
            }
        }
        let left = w.timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            return Err(ApiError::new(
                ErrorKind::Timeout,
                format!(
                    "pull request {id} was still merging after {}s; don't merge again, check \
                     `atlx bb pull-requests merge-status --pull-request {id} --task {task}`",
                    w.timeout.as_secs()
                ),
            )
            .into());
        }
        std::thread::sleep(w.interval.min(left));
    }
}

// --- mergeability ------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct Exclude {
    pub git_mergeability: bool,
    pub commits_behind: bool,
}

/// `*` matches any run of characters, as in branch restriction patterns.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

fn is_conflict(file: &Value) -> bool {
    matches!(
        file["status"].as_str(),
        Some("merge conflict" | "local deleted" | "remote deleted")
    )
}

/// A best-effort answer computed from public endpoints. Checks that couldn't
/// be made are listed under `unchecked` and don't count as blockers.
pub fn mergeability(c: &Client, path: &str, id: u64, x: Exclude) -> Result<Value> {
    let base = pr_path(path, id);
    let pr = c.get(&base, &[])?;
    let mut blockers = Vec::new();
    let mut unchecked = Vec::new();
    let mut block =
        |kind: &str, message: String| blockers.push(json!({"kind": kind, "message": message}));

    let state = pr["state"].as_str().unwrap_or("UNKNOWN");
    if state != "OPEN" {
        block("not_open", format!("the pull request is {state}"));
    }
    if pr["draft"] == true {
        block("draft", "the pull request is a draft".into());
    }
    let people: Vec<&Value> = pr["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let changes = people
        .iter()
        .filter(|p| p["state"] == "changes_requested")
        .count();
    if changes > 0 {
        block(
            "changes_requested",
            format!("{changes} participant(s) requested changes"),
        );
    }
    let approvals = people
        .iter()
        .filter(|p| p["approved"] == true && p["user"]["uuid"] != pr["author"]["uuid"])
        .count();

    let open_tasks = paginate(c, &format!("{base}/tasks"), &[], usize::MAX)?
        .iter()
        .filter(|t| t["state"] == "UNRESOLVED")
        .count();
    if open_tasks > 0 {
        block(
            "unresolved_tasks",
            format!("{open_tasks} unresolved task(s)"),
        );
    }
    for s in paginate(c, &format!("{base}/statuses"), &[], usize::MAX)? {
        if let Some(st @ ("FAILED" | "STOPPED")) = s["state"].as_str() {
            let name = s["name"]
                .as_str()
                .or(s["key"].as_str())
                .unwrap_or("a build");
            block("failing_status", format!("{name} is {st}"));
        }
    }
    if !x.git_mergeability {
        let conflicts = paginate(c, &format!("{base}/diffstat"), &[], usize::MAX)?
            .iter()
            .filter(|f| is_conflict(f))
            .count();
        if conflicts > 0 {
            block("merge_conflict", format!("{conflicts} conflicting file(s)"));
        }
    }

    let dest = pr["destination"]["branch"]["name"]
        .as_str()
        .unwrap_or_default();
    let restrictions = format!("{path}/branch-restrictions");
    let q = [("kind", "require_approvals_to_merge")];
    match paginate(c, &restrictions, &q, usize::MAX) {
        Ok(rules) => {
            let mut needed = 0;
            for r in rules {
                let n = r["value"].as_u64().unwrap_or(0);
                if r["branch_match_kind"] == "branching_model" {
                    unchecked.push(json!({
                        "kind": "approvals",
                        "reason": format!(
                            "a rule needing {n} approval(s) applies to the {} branch type, which isn't resolved",
                            r["branch_type"].as_str().unwrap_or("unknown")
                        ),
                    }));
                } else if r["pattern"].as_str().is_some_and(|p| glob(p, dest)) {
                    needed = needed.max(n);
                }
            }
            if (approvals as u64) < needed {
                block(
                    "approvals",
                    format!("{approvals} of {needed} required approval(s)"),
                );
            }
        }
        Err(e) if matches!(status_of(&e), Some(403 | 404)) => unchecked.push(json!({
            "kind": "approvals",
            "reason": "branch restrictions can't be read with this token",
        })),
        Err(e) => return Err(e),
    }
    if !x.commits_behind {
        unchecked.push(json!({
            "kind": "maximum_commits_behind",
            "reason": "atlx doesn't count how far the source branch is behind",
        }));
    }
    Ok(json!({
        "id": id,
        "mergeable": blockers.is_empty(),
        "approvals": approvals,
        "blockers": blockers,
        "unchecked": unchecked,
    }))
}

/// The diffstat entries that conflict.
pub fn conflicts(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    let files = paginate(
        c,
        &format!("{}/diffstat", pr_path(path, id)),
        &[],
        usize::MAX,
    )?;
    Ok(files.into_iter().filter(is_conflict).take(limit).collect())
}

// --- queries -----------------------------------------------------------------

pub fn for_commit(c: &Client, path: &str, commit: &str, limit: usize) -> Result<Value> {
    let url = format!("{path}/commit/{}/pullrequests", enc(commit.trim()));
    let size = limit.clamp(1, 50).to_string();
    let first = c.request_response("GET", &url, &[("pagelen", size.as_str())], None)?;
    if first.status == 202 {
        let mut e = ApiError::new(
            ErrorKind::Other,
            "Bitbucket is still indexing pull requests for this repository's commits; try again in a few minutes",
        );
        e.status = Some(202);
        return Err(e.into());
    }
    let mut v = first.body;
    let mut out: Vec<Value> = Vec::new();
    loop {
        out.extend(v["values"].as_array().into_iter().flatten().cloned());
        match v["next"].as_str() {
            Some(next) if out.len() < limit => v = c.get(next, &[])?,
            _ => break,
        }
    }
    out.truncate(limit);
    Ok(out.into())
}

fn merged_at(c: &Client, path: &str, id: u64) -> Result<Option<String>> {
    let mut v = c.get(
        &format!("{}/activity", pr_path(path, id)),
        &[("pagelen", "50")],
    )?;
    loop {
        for item in v["values"].as_array().into_iter().flatten() {
            if item["update"]["state"] == "MERGED"
                && let Some(date) = item["update"]["date"].as_str()
            {
                return Ok(Some(date.to_string()));
            }
        }
        match v["next"].as_str() {
            Some(next) => v = c.get(next, &[])?,
            None => return Ok(None),
        }
    }
}

/// Merged pull requests by merge day (UTC). The REST object has no merge
/// time, so candidates are narrowed by `updated_on` and `created_on`, then
/// each one's activity is read, up to [`MERGED_SCAN`] of them.
pub fn merged(
    c: &Client,
    path: &str,
    o: &PrQuery,
    since: Option<&str>,
    before: Option<&str>,
) -> Result<Value> {
    if o.state.is_some_and(|s| !s.eq_ignore_ascii_case("MERGED")) {
        return Err(usage(
            "--merged-since and --merged-before only go with --state MERGED",
        ));
    }
    if o.page.is_some() {
        return Err(usage(
            "--page can't be combined with the merge date filters",
        ));
    }
    let since = since.map(crate::time::to_date).transpose()?;
    let before = before.map(crate::time::to_date).transpose()?;
    let mut f: Vec<String> = pr_filter(o)?.into_iter().collect();
    if let Some(d) = &since {
        f.push(format!("updated_on >= {d}"));
    }
    if let Some(d) = &before {
        f.push(format!("created_on < {d}"));
    }
    let filter = and(f);
    let mut q = vec![("state", "MERGED"), ("sort", "-updated_on")];
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    let (candidates, unseen) = paginate_more(c, &format!("{path}/pullrequests"), &q, MERGED_SCAN)?;
    let mut out = Vec::new();
    let mut scanned = 0;
    for pr in &candidates {
        if out.len() >= o.limit {
            break;
        }
        scanned += 1;
        let Some(id) = pr["id"].as_u64() else {
            continue;
        };
        let Some(at) = merged_at(c, path, id)? else {
            continue;
        };
        let day = at.get(..10).unwrap_or_default();
        if since.as_deref().is_none_or(|d| day >= d) && before.as_deref().is_none_or(|d| day < d) {
            let mut pr = pr.clone();
            pr["merged_on"] = at.into();
            out.push(pr);
        }
    }
    let more = unseen || scanned < candidates.len();
    Ok(json!({
        "values": out,
        "returned": out.len(),
        "scanned": scanned,
        "hasNextPage": more,
        "hasMore": more,
    }))
}

/// `--role`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Role {
    Author,
    Reviewer,
    Participant,
}

/// The current user's pull requests across repositories.
#[derive(Debug)]
pub struct Mine<'a> {
    pub workspace: &'a str,
    /// Scan only this repository.
    pub repo: Option<&'a str>,
    /// `None` is the union of the inbox's reviewer and author views.
    pub role: Option<Role>,
    /// Keep only what is waiting on the user: reviews not approved yet, and
    /// their own pull requests with changes requested.
    pub waiting: bool,
    pub state: &'a str,
    pub limit: usize,
    pub repo_limit: usize,
    pub repo_role: &'a str,
    pub concurrency: usize,
}

const WITH_PEOPLE: &str = "+values.participants,+values.reviewers";

fn keep(pr: &Value, me: &str, role: Option<Role>, waiting: bool) -> bool {
    let people: Vec<&Value> = pr["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let mine = people.iter().find(|p| p["user"]["uuid"] == me);
    let author = pr["author"]["uuid"] == me
        && (!waiting || people.iter().any(|p| p["state"] == "changes_requested"));
    let reviewer = pr["reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["uuid"] == me)
        && (!waiting || mine.is_none_or(|p| p["approved"] != true));
    match role {
        Some(Role::Author) => author,
        Some(Role::Reviewer) => reviewer,
        Some(Role::Participant) => mine.is_some(),
        None => author || reviewer,
    }
}

/// Bitbucket has no workspace-wide reviewer endpoint, so this costs one
/// request for the user, one or more to list the `repo_limit` most recently
/// updated repositories, and one or more for each of those.
pub fn mine(c: &Client, o: &Mine) -> Result<Value> {
    let state = o.state.to_ascii_uppercase();
    let me = my_uuid(c)?;
    let (repos, repos_left) = match o.repo {
        Some(path) => (vec![path.to_string()], false),
        None => {
            let q = [("role", o.repo_role), ("sort", "-updated_on")];
            let list = format!("/repositories/{}", enc(o.workspace));
            let (repos, left) = paginate_more(c, &list, &q, o.repo_limit)?;
            let paths = repos
                .iter()
                .filter_map(|r| {
                    let (ws, slug) = r["full_name"].as_str()?.split_once('/')?;
                    Some(format!("/repositories/{}/{}", enc(ws), enc(slug)))
                })
                .collect();
            (paths, left)
        }
    };
    let user = quote(&me);
    let filter = match o.role {
        Some(Role::Author) => Some(format!("author.uuid = {user}")),
        Some(Role::Reviewer) => Some(format!("reviewers.uuid = {user}")),
        Some(Role::Participant) => None,
        None => Some(format!("(author.uuid = {user} OR reviewers.uuid = {user})")),
    };
    let scan = |repo: &String| {
        let mut q = vec![
            ("state", state.as_str()),
            ("sort", "-updated_on"),
            ("fields", WITH_PEOPLE),
        ];
        if let Some(f) = &filter {
            q.push(("q", f));
        }
        paginate_more(c, &format!("{repo}/pullrequests"), &q, MERGED_SCAN)
    };

    let mut found = Vec::new();
    let mut errors = Vec::new();
    let mut more = repos_left;
    for chunk in repos.chunks(o.concurrency.max(1)) {
        let results: Vec<Result<(Vec<Value>, bool)>> = std::thread::scope(|s| {
            let running: Vec<_> = chunk
                .iter()
                .map(|repo| s.spawn(move || scan(repo)))
                .collect();
            running
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow!("a repository scan panicked")))
                })
                .collect()
        });
        for (repo, result) in chunk.iter().zip(results) {
            match result {
                Ok((prs, left)) => {
                    more |= left;
                    found.extend(
                        prs.into_iter()
                            .filter(|pr| keep(pr, &me, o.role, o.waiting)),
                    );
                }
                // One unreadable repository doesn't spoil a workspace scan.
                Err(e) if o.repo.is_none() && matches!(status_of(&e), Some(403 | 404)) => {
                    errors.push(json!({"repository": repo, "error": e.to_string()}));
                }
                Err(e) => return Err(e),
            }
        }
    }
    found.sort_by(|a, b| b["updated_on"].as_str().cmp(&a["updated_on"].as_str()));
    more |= found.len() > o.limit;
    found.truncate(o.limit);
    let mut out = json!({
        "values": found,
        "returned": found.len(),
        "reposScanned": repos.len(),
        "hasMore": more,
    });
    if !errors.is_empty() {
        out["errors"] = errors.into();
    }
    Ok(out)
}

// --- default reviewers -------------------------------------------------------

pub fn default_reviewers(c: &Client, path: &str, effective: bool, limit: usize) -> Result<Value> {
    let what = if effective {
        "effective-default-reviewers"
    } else {
        "default-reviewers"
    };
    Ok(paginate(c, &format!("{path}/{what}"), &[], limit)?.into())
}

fn default_reviewer_path(path: &str, uuid: &str) -> Result<String> {
    let uuid = uuid.trim();
    if uuid.is_empty() {
        return Err(usage("pass the reviewer's UUID"));
    }
    Ok(format!("{path}/default-reviewers/{}", enc(&braced(uuid))))
}

pub fn default_reviewer_get(c: &Client, path: &str, uuid: &str) -> Result<Value> {
    c.get(&default_reviewer_path(path, uuid)?, &[])
}

pub fn default_reviewer_add(c: &Client, path: &str, uuid: &str) -> Result<Value> {
    c.request("PUT", &default_reviewer_path(path, uuid)?, &[], None)
}

pub fn default_reviewer_remove(c: &Client, path: &str, uuid: &str) -> Result<Value> {
    c.delete(&default_reviewer_path(path, uuid)?, &[])?;
    Ok(json!({"uuid": braced(uuid.trim()), "deleted": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_globs() {
        for (pattern, branch) in [
            ("main", "main"),
            ("*", "a/b"),
            ("release/*", "release/1"),
            ("*fix*", "a-fix-b"),
        ] {
            assert!(glob(pattern, branch), "{pattern} {branch}");
        }
        for (pattern, branch) in [("main", "main2"), ("release/*", "main"), ("a*a", "a")] {
            assert!(!glob(pattern, branch), "{pattern} {branch}");
        }
    }
}

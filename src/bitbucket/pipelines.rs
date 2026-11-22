//! Pipelines: run, rerun, latest failure, tail, grep, test reports, config.

use super::{
    PipelineGet, PipelineQuery, enc, full_name, paginate, pipeline_get, pipeline_query,
    pipeline_ref,
};
use crate::client::{ApiError, Client, ErrorKind};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Upper bound on test cases read per step.
pub const CASE_CAP: usize = 5000;
const STEP_CAP: usize = 500;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn status_of(e: &anyhow::Error) -> Option<u16> {
    ApiError::find(e).and_then(|a| a.status)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// Whether the 0-based `index`th step is the one `selector` names: a 1-based
/// index, a UUID with or without braces, or the exact name.
pub fn step_matches(selector: &str, index: usize, step: &Value) -> bool {
    let sel = selector.trim();
    let uuid = text(step, "uuid");
    sel == text(step, "name")
        || (!uuid.is_empty() && sel.trim_matches(['{', '}']) == uuid.trim_matches(['{', '}']))
        || sel.parse::<usize>().is_ok_and(|n| n == index + 1)
}

/// The selected steps with their 0-based index; a selector matching none or
/// several is an error.
pub(super) fn select<'a>(
    steps: &'a [Value],
    selector: Option<&str>,
) -> Result<Vec<(usize, &'a Value)>> {
    let all = steps.iter().enumerate();
    let Some(sel) = selector else {
        return Ok(all.collect());
    };
    let found: Vec<(usize, &Value)> = all.filter(|(i, s)| step_matches(sel, *i, s)).collect();
    if found.len() == 1 {
        return Ok(found);
    }
    let names: Vec<String> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{} {}", i + 1, text(s, "name")))
        .collect();
    Err(usage(format!(
        "--step '{sel}' matches {} steps; use an index or UUID. The steps are: {}",
        found.len(),
        names.join(", ")
    )))
}

fn pipeline(c: &Client, path: &str, r: &str) -> Result<(Value, String)> {
    let p = c.get(&format!("{path}/pipelines/{}", enc(&pipeline_ref(r))), &[])?;
    let uuid = p["uuid"]
        .as_str()
        .ok_or_else(|| anyhow!("pipeline has no uuid"))?
        .to_string();
    Ok((p, uuid))
}

fn steps(c: &Client, path: &str, uuid: &str) -> Result<Vec<Value>> {
    paginate(
        c,
        &format!("{path}/pipelines/{}/steps", enc(uuid)),
        &[],
        STEP_CAP,
    )
}

fn step_path(path: &str, pipeline: &str, step: &Value) -> String {
    format!(
        "{path}/pipelines/{}/steps/{}",
        enc(pipeline),
        enc(text(step, "uuid"))
    )
}

// --- run -------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Run<'a> {
    pub branch: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub commit: Option<&'a str>,
    /// The custom pipeline's name.
    pub custom: Option<&'a str>,
    /// Key, value, secured.
    pub variables: Vec<(String, String, bool)>,
}

/// `pipeline run|trigger`. Secured values are never echoed.
pub fn run(c: &Client, path: &str, r: &Run) -> Result<Value> {
    let named = match (r.branch, r.tag) {
        (Some(_), Some(_)) => return Err(usage("pass --branch or --tag, not both")),
        (Some(b), None) => Some(("branch", b)),
        (None, Some(t)) => Some(("tag", t)),
        (None, None) => None,
    };
    let mut target = match (named, r.commit, r.custom) {
        (Some((kind, name)), _, _) => {
            json!({"type": "pipeline_ref_target", "ref_type": kind, "ref_name": name})
        }
        (None, Some(_), Some(_)) => json!({"type": "pipeline_commit_target"}),
        (None, Some(_), None) => {
            return Err(usage("--commit needs --custom, --branch or --tag"));
        }
        (None, None, _) => return Err(usage("pass --branch, --tag, or --commit with --custom")),
    };
    if let Some(hash) = r.commit {
        target["commit"] = json!({"type": "commit", "hash": hash});
    }
    if let Some(pattern) = r.custom {
        target["selector"] = json!({"type": "custom", "pattern": pattern});
    }
    let mut req = json!({"target": target});
    if !r.variables.is_empty() {
        req["variables"] = r
            .variables
            .iter()
            .map(|(k, v, secured)| json!({"key": k, "value": v, "secured": secured}))
            .collect();
    }
    let mut v = c.post(&format!("{path}/pipelines/"), &req)?;
    hide_secured(&mut v["variables"]);
    Ok(v)
}

/// Drop the values of secured variables from a list of them.
pub(super) fn hide_secured(list: &mut Value) {
    for var in list.as_array_mut().into_iter().flatten() {
        if var["secured"] == true
            && let Some(m) = var.as_object_mut()
        {
            m.shift_remove("value");
        }
    }
}

/// The result page of a run.
pub fn web_url(path: &str, pipeline: &Value) -> Option<String> {
    let n = pipeline["build_number"].as_u64()?;
    let repo = full_name(path);
    Some(format!(
        "https://bitbucket.org/{repo}/pipelines/results/{n}"
    ))
}

/// `pipeline rerun-failed`. REST can't rerun only the failed steps, so this
/// starts a new run of the same target, which repeats every step, deploys
/// included. That needs `whole` to be asked for.
pub fn rerun(c: &Client, path: &str, r: &str, whole: bool) -> Result<Value> {
    if !whole {
        return Err(usage(
            "Bitbucket's REST API can't rerun only the failed steps. Pass --whole-pipeline to start a new run of the same target; it repeats every step, deployments included",
        ));
    }
    let (p, _) = pipeline(c, path, r)?;
    let result = p["state"]["result"]["name"].as_str().unwrap_or_default();
    if !matches!(result, "FAILED" | "ERROR" | "STOPPED") {
        return Err(usage(
            "that pipeline hasn't failed, so there is nothing to rerun",
        ));
    }
    let target = &p["target"];
    if !target.is_object() {
        return Err(anyhow!(
            "the pipeline came back without its target; not starting a run"
        ));
    }
    // The stored commit matters: without it the branch's head would run.
    if !target["commit"]["hash"].is_string() {
        return Err(anyhow!(
            "the pipeline came back without its commit; not starting a run"
        ));
    }
    let mut again = serde_json::Map::new();
    for key in [
        "type",
        "ref_type",
        "ref_name",
        "selector",
        "source",
        "destination",
    ] {
        if let Some(v) = target.get(key) {
            again.insert(key.into(), v.clone());
        }
    }
    for key in ["commit", "destination_commit"] {
        if let Some(hash) = target[key]["hash"].as_str() {
            again.insert(key.into(), json!({"type": "commit", "hash": hash}));
        }
    }
    if let Some(id) = target["pullrequest"]["id"].as_u64() {
        again.insert("pullrequest".into(), json!({"id": id}));
    }
    let mut req = json!({"target": again});
    if let Some(vars) = p["variables"].as_array().filter(|v| !v.is_empty()) {
        if vars.iter().any(|v| v["secured"] == true) {
            return Err(usage(
                "that run had secured variables, whose values can't be read back; use pipeline run with --secured-variable",
            ));
        }
        req["variables"] = vars
            .iter()
            .map(|v| json!({"key": v["key"], "value": v["value"], "secured": false}))
            .collect();
    }
    let mut out = c.post(&format!("{path}/pipelines/"), &req)?;
    if out.is_object() {
        out["rerunOf"] = p["build_number"].clone();
        out["rerunScope"] = "all_steps".into();
    }
    Ok(out)
}

// --- reads -------------------------------------------------------------------

/// What `pipeline get` adds to each selected step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tests {
    pub reports: bool,
    pub cases: bool,
    /// 0 means all, up to [`CASE_CAP`].
    pub case_limit: usize,
}

/// `pipeline get`, with test reports and cases.
pub fn get(c: &Client, path: &str, r: &str, o: &PipelineGet, t: &Tests) -> Result<Value> {
    let mut out = pipeline_get(c, path, r, o)?;
    if !t.reports && !t.cases {
        return Ok(out);
    }
    let uuid = text(&out, "uuid").to_string();
    let limit = match t.case_limit {
        0 => CASE_CAP,
        n => n.min(CASE_CAP),
    };
    for (i, step) in out["steps"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if o.step.is_some_and(|s| !step_matches(s, i, step)) {
            continue;
        }
        let base = format!("{}/test_reports", step_path(path, &uuid, step));
        // 404 means the step published no reports.
        if t.reports {
            step["test_report"] = match c.get(&base, &[]) {
                Ok(v) => v,
                Err(e) if status_of(&e) == Some(404) => Value::Null,
                Err(e) => return Err(e),
            };
        }
        if t.cases {
            step["test_cases"] = match paginate(c, &format!("{base}/test_cases"), &[], limit) {
                Ok(v) => v.into(),
                Err(e) if status_of(&e) == Some(404) => json!([]),
                Err(e) => return Err(e),
            };
        }
    }
    Ok(out)
}

/// `pipeline latest-failure`: the newest matching failure, as `get --logs`.
pub fn latest_failure(
    c: &Client,
    path: &str,
    q: &PipelineQuery,
    all_steps: bool,
    lines: usize,
) -> Result<Value> {
    let status = q.status.unwrap_or("FAILED");
    let query = PipelineQuery {
        status: Some(status),
        limit: 1,
        ..*q
    };
    let found = pipeline_query(c, path, &query)?;
    let Some(uuid) = found[0]["uuid"].as_str() else {
        return Err(ApiError::new(
            ErrorKind::NotFound,
            format!("no {status} pipeline among the 100 most recent that match"),
        )
        .into());
    };
    let o = PipelineGet {
        logs: true,
        // A stopped run has no failed step to narrow to.
        failed_steps: !all_steps && !status.eq_ignore_ascii_case("STOPPED"),
        lines,
        step: None,
    };
    pipeline_get(c, path, uuid, &o)
}

/// A step's log from byte `from`, or `None` while it has none.
fn log_from(c: &Client, step_path: &str, from: usize) -> Result<Option<Vec<u8>>> {
    let range = format!("bytes={from}-");
    let headers = [("Range", range.as_str())];
    let headers: &[(&str, &str)] = if from == 0 { &[] } else { &headers };
    match c.get_bytes_with(&format!("{step_path}/log"), &[], headers) {
        Ok(b) if b.status == 206 || from == 0 => Ok(Some(b.data)),
        // A server that ignores Range sends the whole log again.
        Ok(b) => Ok(Some(b.data.get(from..).unwrap_or_default().to_vec())),
        Err(e) if status_of(&e) == Some(416) => Ok(Some(Vec::new())),
        Err(e) if status_of(&e) == Some(404) => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Grep<'a> {
    pub pattern: &'a str,
    pub ignore_case: bool,
    pub context: usize,
    pub max_matches: usize,
    pub step: Option<&'a str>,
}

/// `pipeline grep`: plain substring search through the step logs.
pub fn grep(c: &Client, path: &str, r: &str, o: &Grep) -> Result<Value> {
    if o.pattern.is_empty() {
        return Err(usage("the pattern can't be empty"));
    }
    let (p, uuid) = pipeline(c, path, r)?;
    let all = steps(c, path, &uuid)?;
    let fold = |s: &str| {
        if o.ignore_case {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    };
    let needle = fold(o.pattern);
    let mut found = Vec::new();
    let mut truncated = false;
    'steps: for (index, step) in select(&all, o.step)? {
        let Some(log) = log_from(c, &step_path(path, &uuid, step), 0)? else {
            continue;
        };
        let log = String::from_utf8_lossy(&log);
        let lines: Vec<&str> = log.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !fold(line).contains(&needle) {
                continue;
            }
            if found.len() >= o.max_matches {
                truncated = true;
                break 'steps;
            }
            let mut hit = json!({
                "step": text(step, "name"),
                "step_index": index + 1,
                "step_uuid": text(step, "uuid"),
                "line": i + 1,
                "text": line,
            });
            if o.context > 0 {
                let end = (i + 1).saturating_add(o.context).min(lines.len());
                hit["before"] = json!(lines[i.saturating_sub(o.context)..i]);
                hit["after"] = json!(lines[i + 1..end]);
            }
            found.push(hit);
        }
    }
    Ok(json!({
        "pipeline": p["build_number"],
        "pattern": o.pattern,
        "matches": found,
        "truncated": truncated,
    }))
}

// --- tail --------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Tail<'a> {
    pub step: Option<&'a str>,
    /// Lines of existing log to show first; `None` is all of them.
    pub lines: Option<usize>,
    pub follow: bool,
    pub interval: Duration,
    pub timeout: Duration,
}

struct Cursor {
    offset: usize,
    /// An unfinished last line.
    partial: Vec<u8>,
    complete: bool,
}

fn finished(v: &Value) -> bool {
    v["state"]["name"] == "COMPLETED"
}

/// Follow step logs, handing `emit` one event per line and a final `done`.
/// Returns whether the pipeline, or the one selected step, succeeded; with
/// `follow` off that is always true.
pub fn tail(
    c: &Client,
    path: &str,
    r: &str,
    o: &Tail,
    emit: &mut dyn FnMut(&Value) -> Result<()>,
) -> Result<bool> {
    let begun = Instant::now();
    let mut cursors: HashMap<String, Cursor> = HashMap::new();
    let (mut p, uuid) = pipeline(c, path, r)?;
    let mut first = true;
    loop {
        // State is read before the logs, so a finished step's log is whole.
        let pipeline_done = finished(&p);
        let paused = matches!(
            p["state"]["stage"]["name"].as_str(),
            Some("PAUSED" | "HALTED")
        );
        let all = steps(c, path, &uuid)?;
        // Steps can appear after the pipeline starts; a named one must exist.
        let selected = select(&all, o.step)?;
        for (index, step) in &selected {
            let id = text(step, "uuid").to_string();
            let is_new = !cursors.contains_key(&id);
            let cur = cursors.entry(id.clone()).or_insert(Cursor {
                offset: 0,
                partial: Vec::new(),
                complete: false,
            });
            if cur.complete || step["state"]["name"] == "PENDING" {
                continue;
            }
            let done = finished(step);
            let mut data = std::mem::take(&mut cur.partial);
            if let Some(new) = log_from(c, &step_path(path, &uuid, step), cur.offset)? {
                cur.offset += new.len();
                data.extend(new);
            }
            let ends = done || !o.follow;
            if !ends {
                let cut = data.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
                cur.partial = data.split_off(cut);
            }
            cur.complete = done;
            let shown = String::from_utf8_lossy(&data);
            let mut lines: Vec<&str> = shown.lines().collect();
            // Only what was logged before tail began is shortened.
            if first
                && is_new
                && let Some(n) = o.lines
            {
                lines.drain(..lines.len().saturating_sub(n));
            }
            for line in lines {
                emit(&json!({
                    "event": "line",
                    "step": text(step, "name"),
                    "step_index": index + 1,
                    "step_uuid": id,
                    "text": line,
                }))?;
            }
        }
        let subject = match (o.step, selected.as_slice()) {
            (Some(_), [(_, step)]) => *step,
            _ => &p,
        };
        let subject_done = match o.step {
            Some(_) => finished(subject),
            None => pipeline_done,
        };
        if !o.follow || subject_done || paused {
            let result = super::state_of(subject).1;
            emit(&json!({
                "event": "done",
                "pipeline": p["build_number"],
                "state": subject["state"]["name"],
                "result": result,
            }))?;
            let passed = matches!(result.as_deref(), Some("SUCCESSFUL" | "PASSED"));
            return Ok(!o.follow || (subject_done && passed));
        }
        let left = o.timeout.saturating_sub(begun.elapsed());
        if left.is_zero() {
            return Err(ApiError::new(
                ErrorKind::Timeout,
                format!(
                    "the pipeline was still running after {}s",
                    o.timeout.as_secs()
                ),
            )
            .into());
        }
        std::thread::sleep(o.interval.min(left));
        first = false;
        p = pipeline(c, path, &uuid)?.0;
    }
}

// --- config ------------------------------------------------------------------

pub fn config_get(c: &Client, path: &str) -> Result<Value> {
    c.get(&format!("{path}/pipelines_config"), &[])
}

pub fn config_update(c: &Client, path: &str, enabled: bool) -> Result<Value> {
    c.put(
        &format!("{path}/pipelines_config"),
        &json!({"enabled": enabled}),
    )
}

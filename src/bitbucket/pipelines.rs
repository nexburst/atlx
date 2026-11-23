//! Pipelines: run, rerun, latest failure, tail, grep, test reports, config.

use super::{Listed, Paging, PipelineQuery, enc, not_found, pipeline_ref, scan_all, state_of};
use crate::client::{ApiError, Client, ErrorKind};
use crate::paging::{self, Cursor, Position};
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

fn text_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// Whether the 0-based `index`th step is the one `selector` names: a 1-based
/// index, a UUID with or without braces, or the exact name.
pub fn step_matches(selector: &str, index: usize, step: &Value) -> bool {
    let sel = selector.trim();
    let uuid = text_of(step, "uuid");
    sel == text_of(step, "name")
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
        .map(|(i, s)| format!("{} {}", i + 1, text_of(s, "name")))
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
    scan_all(
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
        enc(text_of(step, "uuid"))
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

/// Start a run. Secured values are never echoed.
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

/// Start a failed run again. REST can't rerun only the failed steps, so
/// this starts a new run of the same target and commit, which repeats every
/// step, deployments included.
pub fn rerun(c: &Client, path: &str, r: &str) -> Result<Value> {
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
                "that run had secured variables, whose values can't be read back; start it with `pipeline run --secured-variable`",
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

/// Steps of a run, each with its 1-based `index`, from `--cursor` on.
pub fn step_list(c: &Client, path: &str, r: &str, p: &Paging) -> Result<Listed> {
    let (_, uuid) = pipeline(c, path, r)?;
    let all: Vec<Value> = steps(c, path, &uuid)?
        .into_iter()
        .enumerate()
        .map(|(i, s)| indexed(s, i))
        .collect();
    let start = match p.cursor.as_deref() {
        None => 0,
        Some(s) => match Cursor::decode(s)?.position {
            Position::Offset(o) => usize::try_from(o).unwrap_or(usize::MAX),
            Position::Start => 0,
            _ => return Err(usage("--cursor is not a cursor from a step list")),
        },
    };
    let limit = p.limit.clamp(1, paging::HARD_CAP);
    let end = start.saturating_add(limit).min(all.len());
    let page: Vec<Value> = all
        .get(start..end)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let next = (end < all.len()).then(|| Cursor::at(Position::Offset(end as u64)).encode());
    Ok((page, next))
}

fn indexed(mut step: Value, i: usize) -> Value {
    if step.is_object() {
        step["index"] = (i + 1).into();
    }
    step
}

/// What `pipeline step get` adds to a step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tests {
    /// Add the test cases, not just the report summary.
    pub cases: bool,
    /// 0 means all, up to [`CASE_CAP`].
    pub case_limit: usize,
}

/// One step with its test report (`test_report`, `null` when the step
/// published none) and, if asked, its test cases.
pub fn step_get(c: &Client, path: &str, r: &str, selector: &str, t: &Tests) -> Result<Value> {
    let (_, uuid) = pipeline(c, path, r)?;
    let all = steps(c, path, &uuid)?;
    let (index, step) = select(&all, Some(selector))?
        .into_iter()
        .next()
        .ok_or_else(|| usage(format!("no step '{selector}'")))?;
    let mut step = indexed(step.clone(), index);
    let base = format!("{}/test_reports", step_path(path, &uuid, &step));
    // 404 means the step published no reports.
    step["test_report"] = match c.get(&base, &[]) {
        Ok(v) => v,
        Err(e) if status_of(&e) == Some(404) => Value::Null,
        Err(e) => return Err(e),
    };
    if t.cases {
        let limit = match t.case_limit {
            0 => CASE_CAP,
            n => n.min(CASE_CAP),
        };
        step["test_cases"] = match scan_all(c, &format!("{base}/test_cases"), &[], limit) {
            Ok(v) => v.into(),
            Err(e) if status_of(&e) == Some(404) => json!([]),
            Err(e) => return Err(e),
        };
    }
    Ok(step)
}

/// What `pipeline log` reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogOpts<'a> {
    /// One step: 1-based index, UUID or name.
    pub step: Option<&'a str>,
    /// Only failed steps; without a pipeline, the newest failed run.
    pub failed: bool,
    /// Without a pipeline: the newest run on this branch.
    pub branch: Option<&'a str>,
    /// Only the lines containing this.
    pub grep: Option<Grep<'a>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Grep<'a> {
    pub pattern: &'a str,
    pub ignore_case: bool,
    pub context: usize,
    pub max_matches: usize,
}

/// The run `pipeline log` reads: the one named, else the newest (failed,
/// with `failed`) one, on `branch` if given.
pub fn pick(c: &Client, path: &str, r: Option<&str>, o: &LogOpts) -> Result<(Value, String)> {
    if let Some(r) = r {
        return pipeline(c, path, r);
    }
    let q = PipelineQuery {
        branch: o.branch,
        ..Default::default()
    };
    let failed = |v: &Value| matches!(state_of(v).1.as_deref(), Some("FAILED" | "ERROR"));
    let mut qs = vec![("sort", "-created_on")];
    if let Some(b) = q.branch {
        qs.push(("target.branch", b));
    }
    let (found, _) =
        super::list_where(c, &format!("{path}/pipelines"), &qs, &Paging::new(1), |v| {
            Ok(!o.failed || failed(v))
        })?;
    let Some(p) = found.into_iter().next() else {
        let what = if o.failed {
            "failed pipeline"
        } else {
            "pipeline"
        };
        let on = o.branch.map_or(String::new(), |b| format!(" on {b}"));
        return Err(not_found(format!(
            "no {what}{on} among the most recent runs"
        )));
    };
    let uuid = p["uuid"]
        .as_str()
        .ok_or_else(|| anyhow!("pipeline has no uuid"))?
        .to_string();
    Ok((p, uuid))
}

/// One record per selected step: `{pipeline, step, stepIndex, stepUuid,
/// state, result, log, matches}`. `log` is the whole log (the command
/// layer cuts it); with `grep`, `log` is null and `matches` lists the
/// matching lines.
pub fn logs(c: &Client, path: &str, r: Option<&str>, o: &LogOpts) -> Result<Value> {
    if o.grep.is_some_and(|g| g.pattern.is_empty()) {
        return Err(usage("--grep can't be empty"));
    }
    let (p, uuid) = pick(c, path, r, o)?;
    let all = steps(c, path, &uuid)?;
    let mut out = Vec::new();
    let mut matches = 0usize;
    for (index, step) in select(&all, o.step)? {
        let (state, result) = state_of(step);
        if o.failed && !matches!(result.as_deref(), Some("FAILED" | "ERROR")) {
            continue;
        }
        let text = match log_from(c, &step_path(path, &uuid, step), 0)? {
            Some(b) => String::from_utf8_lossy(&b).into_owned(),
            None => String::new(),
        };
        let mut rec = json!({
            "pipeline": p["build_number"],
            "pipelineUuid": uuid,
            "step": text_of(step, "name"),
            "stepIndex": index + 1,
            "stepUuid": text_of(step, "uuid"),
            "state": state,
            "result": result,
            "log": null,
            "matches": null,
        });
        match &o.grep {
            None => rec["log"] = text.into(),
            Some(g) => {
                let left = g.max_matches.saturating_sub(matches);
                let (found, more) = grep_lines(&text, g, left);
                matches += found.len();
                if more {
                    crate::output::notes::warn(
                        "more_matches",
                        format!(
                            "stopped after {} matches; raise --max-matches",
                            g.max_matches
                        ),
                    );
                }
                rec["matches"] = found.into();
            }
        }
        out.push(rec);
        if o.grep.is_some_and(|g| matches >= g.max_matches) {
            break;
        }
    }
    Ok(out.into())
}

/// Matching lines of a log, numbered from 1, and whether more were left
/// out at `max`.
fn grep_lines(log: &str, g: &Grep, max: usize) -> (Vec<Value>, bool) {
    let fold = |s: &str| {
        if g.ignore_case {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    };
    let needle = fold(g.pattern);
    let lines: Vec<&str> = log.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !fold(line).contains(&needle) {
            continue;
        }
        if found.len() >= max {
            return (found, true);
        }
        let mut hit = json!({"line": i + 1, "text": line});
        if g.context > 0 {
            let end = (i + 1).saturating_add(g.context).min(lines.len());
            hit["before"] = json!(lines[i.saturating_sub(g.context)..i]);
            hit["after"] = json!(lines[i + 1..end]);
        }
        found.push(hit);
    }
    (found, false)
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

struct LogCursor {
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
    let mut cursors: HashMap<String, LogCursor> = HashMap::new();
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
            let id = text_of(step, "uuid").to_string();
            let is_new = !cursors.contains_key(&id);
            let cur = cursors.entry(id.clone()).or_insert(LogCursor {
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
                    "step": text_of(step, "name"),
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
            let result = state_of(subject).1;
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

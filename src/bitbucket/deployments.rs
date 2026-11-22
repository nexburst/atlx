//! Deployments, their environments, history and commits.

use super::{enc, paginate, paginate_more, pipelines::hide_secured};
use crate::client::{ApiError, Client, ErrorKind};
use crate::time::epoch_millis;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

/// How many deployments a history read looks at. REST can neither sort nor
/// filter them, so that happens here.
pub const SCAN: usize = 5000;

fn bare(uuid: &str) -> &str {
    uuid.trim().trim_matches(['{', '}'])
}

fn braced(uuid: &str) -> String {
    format!("{{{}}}", bare(uuid))
}

fn looks_like_uuid(s: &str) -> bool {
    let s = bare(s);
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

pub fn environments(c: &Client, path: &str) -> Result<Value> {
    Ok(paginate(c, &format!("{path}/environments"), &[], SCAN)?.into())
}

/// An environment UUID from a UUID, name or slug.
pub fn environment(c: &Client, path: &str, r: &str) -> Result<String> {
    if looks_like_uuid(r) {
        return Ok(braced(r));
    }
    let all = paginate(c, &format!("{path}/environments"), &[], SCAN)?;
    let found: Vec<&str> = all
        .iter()
        .filter(|e| {
            ["name", "slug"].iter().any(|k| {
                e[*k]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(r.trim()))
            })
        })
        .filter_map(|e| e["uuid"].as_str())
        .collect();
    match found.as_slice() {
        [id] => Ok(id.to_string()),
        _ => Err(ApiError::usage(format!(
            "'{r}' matches {} environments; see `bb deployment environments query`",
            found.len()
        ))
        .into()),
    }
}

/// Environment variables, without the values of secured ones.
pub fn variables(c: &Client, path: &str, env: &str) -> Result<Value> {
    let id = environment(c, path, env)?;
    let url = format!(
        "{path}/deployments_config/environments/{}/variables",
        enc(&id)
    );
    let mut list = Value::Array(paginate(c, &url, &[], SCAN)?);
    hide_secured(&mut list);
    Ok(list)
}

fn in_environment(d: &Value, env: &str) -> bool {
    d["environment"]["uuid"]
        .as_str()
        .is_some_and(|u| bare(u) == bare(env))
}

fn started(d: &Value) -> Option<i64> {
    d["state"]["started_on"]
        .as_str()
        .and_then(|s| epoch_millis(s).ok())
}

fn outcome(d: &Value) -> &str {
    d["state"]["status"]["name"]
        .as_str()
        .or(d["state"]["name"].as_str())
        .unwrap_or_default()
}

fn commit_of(d: &Value) -> Option<&str> {
    ["release", "deployable"]
        .iter()
        .find_map(|k| d[*k]["commit"]["hash"].as_str())
}

pub fn query(c: &Client, path: &str, env: Option<&str>, limit: usize) -> Result<Value> {
    let url = format!("{path}/deployments");
    let Some(env) = env else {
        return Ok(paginate(c, &url, &[], limit)?.into());
    };
    let id = environment(c, path, env)?;
    Ok(paginate(c, &url, &[], SCAN)?
        .into_iter()
        .filter(|d| in_environment(d, &id))
        .take(limit)
        .collect())
}

pub fn get(c: &Client, path: &str, uuid: &str) -> Result<Value> {
    c.get(&format!("{path}/deployments/{}", enc(&braced(uuid))), &[])
}

#[derive(Debug, Default)]
pub struct History<'a> {
    pub environment: &'a str,
    pub statuses: &'a [String],
    pub pipeline: Option<&'a str>,
    pub after: Option<&'a str>,
    pub before: Option<&'a str>,
    pub ascending: bool,
    pub limit: usize,
}

pub fn history(c: &Client, path: &str, o: &History) -> Result<Value> {
    let after = o.after.map(epoch_millis).transpose()?;
    let before = o.before.map(epoch_millis).transpose()?;
    let env = environment(c, path, o.environment)?;
    let (all, unseen) = paginate_more(c, &format!("{path}/deployments"), &[], SCAN)?;
    let mut found: Vec<Value> = all
        .into_iter()
        .filter(|d| {
            let at = started(d);
            in_environment(d, &env)
                && (o.statuses.is_empty()
                    || o.statuses
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(outcome(d))))
                && o.pipeline.is_none_or(|p| {
                    d["release"]["pipeline"]["uuid"]
                        .as_str()
                        .is_some_and(|u| bare(u) == bare(p))
                })
                && after.is_none_or(|t| at.is_some_and(|a| a >= t))
                && before.is_none_or(|t| at.is_some_and(|a| a <= t))
        })
        .collect();
    found.sort_by_key(started);
    if !o.ascending {
        found.reverse();
    }
    let more = unseen || found.len() > o.limit;
    found.truncate(o.limit);
    Ok(json!({"values": found, "hasMore": more}))
}

pub fn history_get(c: &Client, path: &str, uuid: &str, env: &str) -> Result<Value> {
    let env = environment(c, path, env)?;
    let d = get(c, path, uuid)?;
    if !in_environment(&d, &env) {
        return Err(ApiError::new(
            ErrorKind::NotFound,
            format!("deployment {uuid} is not in that environment"),
        )
        .into());
    }
    Ok(d)
}

/// What a deployment added and removed compared with the environment's
/// previous successful one.
pub fn commits(c: &Client, path: &str, env: &str, deployment: &str, limit: usize) -> Result<Value> {
    let env = environment(c, path, env)?;
    let target = history_get(c, path, deployment, &env)?;
    let at = started(&target).ok_or_else(|| anyhow!("that deployment has no start time"))?;
    let new = commit_of(&target).ok_or_else(|| anyhow!("that deployment has no commit"))?;
    let (all, unseen) = paginate_more(c, &format!("{path}/deployments"), &[], SCAN)?;
    if unseen {
        return Err(anyhow!(
            "the repository has more than {SCAN} deployments, so the previous one can't be found reliably"
        ));
    }
    let previous = all
        .iter()
        .filter(|d| in_environment(d, &env) && outcome(d) == "SUCCESSFUL")
        .filter(|d| started(d).is_some_and(|t| t < at))
        .max_by_key(|d| started(d));
    let Some(previous) = previous else {
        return Ok(json!({
            "deployment": target["uuid"],
            "previous": null,
            "added": [],
            "removed": [],
            "hasMore": false,
        }));
    };
    let old =
        commit_of(previous).ok_or_else(|| anyhow!("the previous deployment has no commit"))?;
    let url = format!("{path}/commits");
    let between = |include: &str, exclude: &str| {
        paginate_more(
            c,
            &url,
            &[("include", include), ("exclude", exclude)],
            limit,
        )
    };
    let (added, more_added) = between(new, old)?;
    let (removed, more_removed) = between(old, new)?;
    Ok(json!({
        "deployment": target["uuid"],
        "previous": previous["uuid"],
        "added": added,
        "removed": removed,
        "hasMore": more_added || more_removed,
    }))
}

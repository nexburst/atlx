//! `bb pipeline` actions and `bb deployment`.

use super::{RepoArgs, positive_limit};
use crate::bitbucket::{self, deployments, pipelines};
use crate::cli::{Out, util};
use crate::client::{ApiError, Client};
use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;
use std::io::Write;
use std::time::Duration;

use super::lifecycle::PathOf;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

#[derive(Args)]
pub(crate) struct RunArgs {
    /// Run on this branch
    #[arg(long, conflicts_with = "tag")]
    branch: Option<String>,
    /// Run on this tag
    #[arg(long)]
    tag: Option<String>,
    /// Run at this commit; needs --custom unless --branch or --tag is given
    #[arg(long)]
    commit: Option<String>,
    /// The custom pipeline to run
    #[arg(long, visible_alias = "selector")]
    custom: Option<String>,
    /// KEY=VALUE; repeatable
    #[arg(long, value_name = "KEY=VALUE", num_args = 1..)]
    variable: Vec<String>,
    /// KEY, whose value is read from the environment variable of that name,
    /// so it stays out of the command line; repeatable
    #[arg(long, value_name = "KEY", num_args = 1..)]
    secured_variable: Vec<String>,
    /// Add the new run's web URL as `url`
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct RerunArgs {
    /// Build number, {uuid}, or result URL
    #[arg(long)]
    pipeline: String,
    /// Agree to repeat every step: REST can't rerun only the failed ones
    /// (atlx extra)
    #[arg(long)]
    whole_pipeline: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct LatestFailureArgs {
    /// PENDING, IN_PROGRESS, COMPLETED
    #[arg(long)]
    state: Option<String>,
    /// FAILED (default), ERROR or STOPPED
    #[arg(long, ignore_case = true, value_parser = ["FAILED", "ERROR", "STOPPED"])]
    status: Option<String>,
    #[arg(long)]
    branch: Option<String>,
    /// PUSH, MANUAL, SCHEDULE, PULLREQUEST
    #[arg(long)]
    trigger: Option<String>,
    /// The pipeline's pattern, such as a custom pipeline's name
    #[arg(long)]
    pattern: Option<String>,
    /// Logs of every step, not only the failed ones
    #[arg(long)]
    all_steps: bool,
    /// Max log lines per step (0 = all)
    #[arg(long, default_value_t = 40)]
    lines: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct GrepArgs {
    /// Text to look for (not a regular expression)
    #[arg(value_name = "pattern", conflicts_with = "pattern")]
    text: Option<String>,
    /// Same as the positional argument (atlx extra)
    #[arg(long)]
    pattern: Option<String>,
    /// Build number, {uuid}, or result URL
    #[arg(long)]
    pipeline: String,
    /// Only this step: 1-based index, UUID or name
    #[arg(long)]
    step: Option<String>,
    /// Ignore case
    #[arg(long, short = 'i')]
    ignore_case: bool,
    /// Lines of context around each match
    #[arg(long, short = 'C', default_value_t = 0)]
    context: usize,
    /// Stop after this many matches
    #[arg(long, default_value_t = 200, value_parser = positive_limit)]
    max_matches: usize,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Args)]
pub(crate) struct TailArgs {
    /// Pipeline build number or result URL (a step in the URL selects it)
    target: Option<String>,
    /// Build number, {uuid}, or result URL
    #[arg(long, conflicts_with = "target")]
    pipeline: Option<String>,
    /// One step: 1-based index, UUID or name [default: every step]
    #[arg(long)]
    step: Option<String>,
    /// Existing log lines to show first: a number, or all
    #[arg(long, short = 'n', default_value = "10")]
    lines: String,
    /// Print what is logged so far and exit
    #[arg(long)]
    no_follow: bool,
    /// Milliseconds between polls
    #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u64).range(250..))]
    poll_interval: u64,
    /// Seconds between polls (atlx extra)
    #[arg(long, conflicts_with = "poll_interval", value_parser = clap::value_parser!(u64).range(1..))]
    interval: Option<u64>,
    /// Give up after this many seconds (atlx extra)
    #[arg(long, default_value_t = 1800)]
    timeout: u64,
    /// Accepted for compatibility; atlx always polls
    #[arg(long, hide = true)]
    no_ws: bool,
    #[command(flatten)]
    repo: RepoArgs,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// The repository's Pipelines settings
    Get {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Turn Pipelines on or off; off stops all future runs
    Update {
        /// true or false
        #[arg(long)]
        enabled: Option<bool>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Direction {
    Asc,
    Desc,
}

#[derive(Subcommand)]
pub(crate) enum DeploymentCmd {
    /// List deployments
    Query {
        /// Only this environment: UUID, name or slug (atlx extra)
        #[arg(long)]
        environment: Option<String>,
        #[arg(long, short = 'n', default_value_t = 25, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One deployment
    Get {
        /// Deployment UUID
        uuid: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Deployment environments
    Environments {
        #[command(subcommand)]
        cmd: EnvironmentsCmd,
    },
    /// One environment's settings
    Environment {
        #[command(subcommand)]
        cmd: EnvironmentCmd,
    },
    /// An environment's past deployments
    History {
        #[command(subcommand)]
        cmd: HistoryCmd,
    },
    /// What a deployment changed
    Commits {
        #[command(subcommand)]
        cmd: CommitsCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum EnvironmentsCmd {
    /// List the environments
    Query {
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum EnvironmentCmd {
    /// Deployment variables
    Variable {
        #[command(subcommand)]
        cmd: VariableCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum VariableCmd {
    /// List the variables; secured values are never shown
    Query {
        /// Environment UUID, name or slug
        #[arg(value_name = "environment", required_unless_present = "environment")]
        environment_arg: Option<String>,
        /// Same as the positional argument (atlx extra)
        #[arg(long, conflicts_with = "environment_arg")]
        environment: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum HistoryCmd {
    /// Deployment attempts, newest first
    Query {
        /// Environment UUID, name or slug
        #[arg(long)]
        environment: String,
        /// Only this outcome, e.g. SUCCESSFUL or FAILED; repeatable
        #[arg(long)]
        status: Vec<String>,
        /// Only deployments from this pipeline
        #[arg(long)]
        pipeline_uuid: Option<String>,
        /// Started at or after this time
        #[arg(long)]
        started_after: Option<String>,
        /// Started at or before this time
        #[arg(long)]
        started_before: Option<String>,
        /// By start time
        #[arg(long, value_enum, default_value = "desc")]
        sort: Direction,
        #[arg(long, short = 'n', default_value_t = 25, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One deployment in an environment
    Get {
        /// Deployment UUID
        #[arg(value_name = "deployment-uuid")]
        uuid: String,
        /// Environment UUID, name or slug
        #[arg(long)]
        environment: String,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum CommitsCmd {
    /// Commits added and removed since the previous successful deployment
    Query {
        /// Environment UUID, name or slug
        #[arg(long)]
        environment: String,
        /// Deployment UUID
        #[arg(long)]
        deployment: String,
        #[arg(long, short = 'n', default_value_t = 50, value_parser = positive_limit)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

/// `--variable` and `--secured-variable`. Error messages name keys only:
/// what follows `=` may be a secret.
fn variables(
    plain: &[String],
    secured: &[String],
    env: impl Fn(&str) -> Option<String>,
) -> Result<Vec<(String, String, bool)>> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for item in plain {
        let Some((k, v)) = item.split_once('=').filter(|(k, _)| !k.is_empty()) else {
            return Err(usage("--variable takes KEY=VALUE"));
        };
        out.push((k.to_string(), v.to_string(), false));
    }
    for key in secured {
        if let Some((key, _)) = key.split_once('=') {
            return Err(usage(format!(
                "--secured-variable takes only the key, so the secret stays out of the command line: set {key} in the environment and pass --secured-variable {key}"
            )));
        }
        let Some(value) = env(key) else {
            return Err(usage(format!(
                "--secured-variable {key}: there is no environment variable of that name"
            )));
        };
        out.push((key.clone(), value, true));
    }
    let mut keys: Vec<&str> = out.iter().map(|(k, _, _)| k.as_str()).collect();
    keys.sort_unstable();
    if let Some(pair) = keys.windows(2).find(|w| w[0] == w[1]) {
        return Err(usage(format!("variable {} is given twice", pair[0])));
    }
    Ok(out)
}

pub(super) fn run(c: &Client, path: PathOf, a: RunArgs) -> Result<Value> {
    let variables = variables(&a.variable, &a.secured_variable, |k| std::env::var(k).ok())?;
    let path = path(&a.repo)?;
    let mut v = pipelines::run(
        c,
        &path,
        &pipelines::Run {
            branch: a.branch.as_deref(),
            tag: a.tag.as_deref(),
            commit: a.commit.as_deref(),
            custom: a.custom.as_deref(),
            variables,
        },
    )?;
    if a.open
        && let Some(url) = pipelines::web_url(&path, &v)
    {
        v["url"] = url.into();
    }
    Ok(v)
}

pub(super) fn rerun(c: &Client, path: PathOf, a: RerunArgs) -> Result<Value> {
    pipelines::rerun(c, &path(&a.repo)?, &a.pipeline, a.whole_pipeline)
}

pub(super) fn latest_failure(c: &Client, path: PathOf, a: LatestFailureArgs) -> Result<Value> {
    let q = bitbucket::PipelineQuery {
        state: a.state.as_deref(),
        status: a.status.as_deref(),
        branch: a.branch.as_deref(),
        trigger: a.trigger.as_deref(),
        pattern: a.pattern.as_deref(),
        limit: 1,
    };
    pipelines::latest_failure(c, &path(&a.repo)?, &q, a.all_steps, a.lines)
}

pub(super) fn grep(c: &Client, path: PathOf, a: GrepArgs) -> Result<Out> {
    let pattern = a
        .text
        .or(a.pattern)
        .ok_or_else(|| usage("pass the text to look for"))?;
    let o = pipelines::Grep {
        pattern: &pattern,
        ignore_case: a.ignore_case,
        context: a.context,
        max_matches: a.max_matches,
        step: a.step.as_deref(),
    };
    let full = pipelines::grep(c, &path(&a.repo)?, &a.pipeline, &o)?;
    Ok(Out::Doc {
        text: grep_text(&full),
        full,
    })
}

/// `step:line: text`, with context lines as `step-line- text`, like grep.
fn grep_text(v: &Value) -> String {
    let mut out = Vec::new();
    for m in v["matches"].as_array().into_iter().flatten() {
        let step = m["step"].as_str().unwrap_or_default();
        let n = m["line"].as_u64().unwrap_or_default();
        let lines = |k: &str| -> Vec<&str> {
            m[k].as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect()
        };
        let before = lines("before");
        let first = n.saturating_sub(before.len() as u64);
        for (line, at) in before.iter().zip(first..) {
            out.push(format!("{step}-{at}- {line}"));
        }
        out.push(format!(
            "{step}:{n}: {}",
            m["text"].as_str().unwrap_or_default()
        ));
        for (line, at) in lines("after").iter().zip(n + 1..) {
            out.push(format!("{step}-{at}- {line}"));
        }
    }
    if v["truncated"] == true {
        out.push("(more matches: raise --max-matches)".into());
    }
    out.join("\n")
}

pub(super) fn tail(c: &Client, path: PathOf, a: TailArgs) -> Result<Out> {
    let target = a
        .pipeline
        .or(a.target)
        .ok_or_else(|| usage("pass the pipeline as an argument or with --pipeline"))?;
    let url_step = target
        .split_once("/steps/")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .and_then(|s| urlencoding::decode(s).ok())
        .map(|s| s.into_owned());
    let step = url_step.or(a.step);
    let lines = if a.lines.eq_ignore_ascii_case("all") {
        None
    } else {
        Some(
            a.lines
                .parse()
                .map_err(|_| usage("--lines takes a number or all"))?,
        )
    };
    let o = pipelines::Tail {
        step: step.as_deref(),
        lines,
        follow: !a.no_follow,
        interval: a
            .interval
            .map_or(Duration::from_millis(a.poll_interval), Duration::from_secs),
        timeout: Duration::from_secs(a.timeout),
    };
    let path = path(&a.repo)?;
    let text = util::text_output();
    let mut out = std::io::stdout();
    let mut emit = |event: &Value| -> Result<()> {
        let field = |k: &str| event[k].as_str().unwrap_or_default();
        let line = if !text {
            event.to_string()
        } else if event["event"] == "done" {
            format!(
                "pipeline {}: {} {}",
                event["pipeline"],
                field("state"),
                field("result")
            )
        } else if o.step.is_some() {
            field("text").to_string()
        } else {
            format!("[{}] {}", field("step"), field("text"))
        };
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .context("writing the log to stdout")
    };
    let ok = pipelines::tail(c, &path, &target, &o, &mut emit)?;
    Ok(Out::Streamed(if ok { 0 } else { 1 }))
}

pub(super) fn config(c: &Client, path: PathOf, cmd: ConfigCmd) -> Result<Value> {
    match cmd {
        ConfigCmd::Get { repo } => pipelines::config_get(c, &path(&repo)?),
        ConfigCmd::Update { enabled, repo } => {
            let enabled = enabled.ok_or_else(|| usage("pass --enabled true or false"))?;
            pipelines::config_update(c, &path(&repo)?, enabled)
        }
    }
}

pub(super) fn deployment(c: &Client, path: PathOf, cmd: DeploymentCmd) -> Result<Value> {
    match cmd {
        DeploymentCmd::Query {
            environment,
            limit,
            repo,
        } => deployments::query(c, &path(&repo)?, environment.as_deref(), limit),
        DeploymentCmd::Get { uuid, repo } => deployments::get(c, &path(&repo)?, &uuid),
        DeploymentCmd::Environments {
            cmd: EnvironmentsCmd::Query { repo },
        } => deployments::environments(c, &path(&repo)?),
        DeploymentCmd::Environment {
            cmd:
                EnvironmentCmd::Variable {
                    cmd:
                        VariableCmd::Query {
                            environment_arg,
                            environment,
                            repo,
                        },
                },
        } => {
            let env = environment_arg
                .or(environment)
                .ok_or_else(|| usage("pass the environment"))?;
            deployments::variables(c, &path(&repo)?, &env)
        }
        DeploymentCmd::History {
            cmd:
                HistoryCmd::Query {
                    environment,
                    status,
                    pipeline_uuid,
                    started_after,
                    started_before,
                    sort,
                    limit,
                    repo,
                },
        } => deployments::history(
            c,
            &path(&repo)?,
            &deployments::History {
                environment: &environment,
                statuses: &status,
                pipeline: pipeline_uuid.as_deref(),
                after: started_after.as_deref(),
                before: started_before.as_deref(),
                ascending: sort == Direction::Asc,
                limit,
            },
        ),
        DeploymentCmd::History {
            cmd:
                HistoryCmd::Get {
                    uuid,
                    environment,
                    repo,
                },
        } => deployments::history_get(c, &path(&repo)?, &uuid, &environment),
        DeploymentCmd::Commits {
            cmd:
                CommitsCmd::Query {
                    environment,
                    deployment,
                    limit,
                    repo,
                },
        } => deployments::commits(c, &path(&repo)?, &environment, &deployment, limit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secured_values_come_from_the_environment_and_stay_out_of_errors() {
        let env = |k: &str| (k == "TOKEN").then(|| "hidden".to_string());
        let v = variables(&["A=b=c,d".into()], &["TOKEN".into()], env).unwrap();
        assert_eq!(v[0], ("A".into(), "b=c,d".into(), false));
        assert_eq!(v[1], ("TOKEN".into(), "hidden".into(), true));
        let err = variables(&[], &["TOKEN=hunter2".into()], env).unwrap_err();
        assert!(!err.to_string().contains("hunter2"), "{err}");
        assert!(variables(&[], &["ABSENT".into()], env).is_err());
        assert!(variables(&["TOKEN=x".into()], &["TOKEN".into()], env).is_err());
    }

    #[test]
    fn grep_text_numbers_context() {
        let v = json!({"matches": [
            {"step": "T", "line": 3, "text": "boom", "before": ["a", "b"], "after": ["c"]}
        ]});
        assert_eq!(grep_text(&v), "T-1- a\nT-2- b\nT:3: boom\nT-4- c");
    }
}

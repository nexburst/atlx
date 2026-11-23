//! `bb pipeline ...`, `bb deployment ...` and `bb environment ...`.

use super::{BatchArgs, Ctx, ListArgs, RepoArg, get_many, listed};
use crate::bitbucket::{self, deployments, pipelines, records as rec};
use crate::cli::{Out, util};
use crate::client::ApiError;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum PipelineCmd {
    /// Pipeline runs, newest first
    List {
        /// PENDING, IN_PROGRESS or COMPLETED
        #[arg(long)]
        state: Option<String>,
        /// SUCCESSFUL, FAILED, ERROR or STOPPED
        #[arg(long)]
        result: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// PUSH, MANUAL, SCHEDULE or PULLREQUEST
        #[arg(long)]
        trigger: Option<String>,
        /// Custom pipeline name or selector pattern
        #[arg(long)]
        pattern: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more runs: build number, {uuid} or result URL
    Get {
        #[arg(required = true, value_name = "PIPELINE")]
        pipelines: Vec<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Start a run on a branch, tag or commit, or start a failed one again
    Run(RunArgs),
    /// Step logs (the last 200 lines of each; --log-offset reads further back)
    ///
    /// Without a pipeline: the newest run (the newest failed one with
    /// --failed), on --branch if given. --follow waits for the run to end;
    /// in text mode it prints lines as they come, and the exit code is 1 if
    /// the run (or --step) did not succeed.
    Log(LogArgs),
    /// Steps of a run
    Step {
        #[command(subcommand)]
        cmd: StepCmd,
    },
    /// Pipelines settings of a repository
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Args)]
pub(crate) struct RunArgs {
    /// Run on this branch
    #[arg(long, conflicts_with_all = ["tag", "rerun"])]
    branch: Option<String>,
    /// Run on this tag
    #[arg(long, conflicts_with = "rerun")]
    tag: Option<String>,
    /// Run at this commit; needs --custom unless --branch or --tag is given
    #[arg(long, conflicts_with = "rerun")]
    commit: Option<String>,
    /// The custom pipeline to run
    #[arg(long, conflicts_with = "rerun")]
    custom: Option<String>,
    /// KEY=VALUE; repeatable
    #[arg(long, value_name = "KEY=VALUE", num_args = 1.., conflicts_with = "rerun")]
    variable: Vec<String>,
    /// KEY, whose value is read from the environment variable of that name,
    /// so it stays out of the command line; repeatable
    #[arg(long, value_name = "KEY", num_args = 1.., conflicts_with = "rerun")]
    secured_variable: Vec<String>,
    /// Start this failed run again: same target, same commit, every step
    /// (REST can't rerun only the failed steps)
    #[arg(long, value_name = "PIPELINE")]
    rerun: Option<String>,
    #[command(flatten)]
    repo: RepoArg,
}

#[derive(Args)]
pub(crate) struct LogArgs {
    /// Build number, {uuid} or result URL [default: the newest run]
    #[arg(value_name = "PIPELINE")]
    pipeline: Option<String>,
    /// One step: 1-based index, UUID or name
    #[arg(long)]
    step: Option<String>,
    /// Only failed steps; without a pipeline, the newest failed run
    #[arg(long)]
    failed: bool,
    /// Without a pipeline: the newest run on this branch
    #[arg(long, conflicts_with = "pipeline")]
    branch: Option<String>,
    /// Only lines containing this text (not a regular expression)
    #[arg(long, value_name = "TEXT", conflicts_with = "follow")]
    grep: Option<String>,
    /// With --grep: ignore case
    #[arg(long, short = 'i', requires = "grep")]
    ignore_case: bool,
    /// With --grep: lines of context around each match
    #[arg(
        long,
        short = 'C',
        value_name = "N",
        default_value_t = 0,
        requires = "grep"
    )]
    context: usize,
    /// With --grep: stop after this many matches
    #[arg(long, value_name = "N", default_value_t = 200, value_parser = super::positive)]
    max_matches: usize,
    /// Wait until the run (or --step) ends
    #[arg(long, conflicts_with = "failed")]
    follow: bool,
    /// With --follow: seconds between polls
    #[arg(long, value_name = "SECONDS", default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..))]
    interval: u64,
    /// With --follow: give up after this many seconds
    #[arg(long, value_name = "SECONDS", default_value_t = 1800)]
    timeout: u64,
    /// Skip this many lines from the end of each log
    #[arg(long, value_name = "N")]
    log_offset: Option<usize>,
    #[command(flatten)]
    repo: RepoArg,
}

#[derive(Subcommand)]
pub(crate) enum StepCmd {
    /// The steps of a run
    List {
        /// Build number, {uuid} or result URL
        #[arg(long)]
        pipeline: String,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One step with its test report
    Get {
        /// 1-based index, UUID or name
        step: String,
        /// Build number, {uuid} or result URL
        #[arg(long)]
        pipeline: String,
        /// Add the test cases
        #[arg(long)]
        test_cases: bool,
        /// With --test-cases: at most this many (0 = all, up to 5000)
        #[arg(long, value_name = "N", default_value_t = 100)]
        case_limit: usize,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// Whether Pipelines is on
    Get {
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Turn Pipelines on or off; off stops all future runs
    Update {
        /// true or false
        #[arg(long, required = true)]
        enabled: Option<bool>,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum DeploymentCmd {
    /// Deployments; any filter reads up to 5000 of them and sorts by start, newest first
    List {
        /// Environment UUID, name or slug
        #[arg(long)]
        environment: Option<String>,
        /// Only this outcome, e.g. SUCCESSFUL or FAILED; repeatable
        #[arg(long)]
        status: Vec<String>,
        /// Only deployments from this pipeline (UUID)
        #[arg(long, value_name = "UUID")]
        pipeline: Option<String>,
        /// Started at or after this time (YYYY-MM-DD, ISO time, or 7d)
        #[arg(long, value_name = "TIME")]
        started_after: Option<String>,
        /// Started at or before this time
        #[arg(long, value_name = "TIME")]
        started_before: Option<String>,
        /// Oldest first
        #[arg(long)]
        oldest_first: bool,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// One or more deployments by UUID
    Get {
        #[arg(required = true, value_name = "UUID")]
        uuids: Vec<String>,
        /// Fail unless each is in this environment (UUID, name or slug)
        #[arg(long)]
        environment: Option<String>,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        batch: BatchArgs,
    },
    /// Commits a deployment added and removed since its environment's
    /// previous successful one
    Commits {
        /// Deployment UUID
        uuid: String,
        /// Fail unless it is in this environment (UUID, name or slug)
        #[arg(long)]
        environment: Option<String>,
        /// At most this many commits each way
        #[arg(long, value_name = "N", default_value_t = 50, value_parser = super::positive)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum EnvironmentCmd {
    /// Deployment environments
    List {
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Deployment variables
    Variable {
        #[command(subcommand)]
        cmd: VariableCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum VariableCmd {
    /// An environment's variables; secured values are never shown
    List {
        /// Environment UUID, name or slug
        #[arg(long)]
        environment: String,
        #[command(flatten)]
        repo: RepoArg,
        #[command(flatten)]
        list: ListArgs,
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

pub(super) fn pipeline(ctx: &Ctx, cmd: PipelineCmd) -> Result<Out> {
    let c = ctx.c;
    let data = match cmd {
        PipelineCmd::List {
            state,
            result,
            branch,
            trigger,
            pattern,
            repo,
            list,
        } => {
            let q = bitbucket::PipelineQuery {
                state: state.as_deref(),
                result: result.as_deref(),
                branch: branch.as_deref(),
                trigger: trigger.as_deref(),
                pattern: pattern.as_deref(),
            };
            listed(bitbucket::pipeline_list(
                c,
                &ctx.repo(&repo)?,
                &q,
                &list.paging(),
            )?)
        }
        PipelineCmd::Get {
            pipelines,
            repo,
            batch,
        } => {
            let p = ctx.repo(&repo)?;
            get_many(pipelines, &batch, |id| bitbucket::pipeline_get(c, &p, id))?
        }
        PipelineCmd::Run(a) => {
            let p = ctx.repo(&a.repo)?;
            match &a.rerun {
                Some(r) => pipelines::rerun(c, &p, r)?,
                None => {
                    let vars =
                        variables(&a.variable, &a.secured_variable, |k| std::env::var(k).ok())?;
                    pipelines::run(
                        c,
                        &p,
                        &pipelines::Run {
                            branch: a.branch.as_deref(),
                            tag: a.tag.as_deref(),
                            commit: a.commit.as_deref(),
                            custom: a.custom.as_deref(),
                            variables: vars,
                        },
                    )?
                }
            }
        }
        PipelineCmd::Log(a) => return log(ctx, a),
        PipelineCmd::Step {
            cmd:
                StepCmd::List {
                    pipeline,
                    repo,
                    list,
                },
        } => listed(pipelines::step_list(
            c,
            &ctx.repo(&repo)?,
            &pipeline,
            &list.paging(),
        )?),
        PipelineCmd::Step {
            cmd:
                StepCmd::Get {
                    step,
                    pipeline,
                    test_cases,
                    case_limit,
                    repo,
                },
        } => pipelines::step_get(
            c,
            &ctx.repo(&repo)?,
            &pipeline,
            &step,
            &pipelines::Tests {
                cases: test_cases,
                case_limit,
            },
        )?,
        PipelineCmd::Config {
            cmd: ConfigCmd::Get { repo },
        } => pipelines::config_get(c, &ctx.repo(&repo)?)?,
        PipelineCmd::Config {
            cmd: ConfigCmd::Update { enabled, repo },
        } => {
            let enabled = enabled.ok_or_else(|| usage("pass --enabled true or false"))?;
            pipelines::config_update(c, &ctx.repo(&repo)?, enabled)?
        }
    };
    Ok(Out::Data(data))
}

fn log(ctx: &Ctx, a: LogArgs) -> Result<Out> {
    let path = ctx.repo(&a.repo)?;
    let grep = a.grep.as_deref().map(|pattern| pipelines::Grep {
        pattern,
        ignore_case: a.ignore_case,
        context: a.context,
        max_matches: a.max_matches,
    });
    let o = pipelines::LogOpts {
        step: a.step.as_deref(),
        failed: a.failed,
        branch: a.branch.as_deref(),
        grep,
    };
    if !a.follow {
        return Ok(Out::Data(pipelines::logs(
            ctx.c,
            &path,
            a.pipeline.as_deref(),
            &o,
        )?));
    }
    follow(ctx, &path, &a, &o)
}

/// `pipeline log --follow`: poll until the run ends. Text mode prints lines
/// as they come; JSON modes print the logs as records at the end. Exit 1
/// when the run (or the one step) did not succeed.
fn follow(ctx: &Ctx, path: &str, a: &LogArgs, o: &pipelines::LogOpts) -> Result<Out> {
    let (p, uuid) = pipelines::pick(ctx.c, path, a.pipeline.as_deref(), o)?;
    let text = util::text_output();
    let t = pipelines::Tail {
        step: o.step,
        // Text mode starts with the last lines logged so far, like the
        // record's budget; JSON modes keep everything and cut at the end.
        lines: text.then_some(rec::LOG_BUDGET.limit),
        follow: true,
        interval: Duration::from_secs(a.interval),
        timeout: Duration::from_secs(a.timeout),
    };
    let mut out = std::io::stdout();
    let mut steps: BTreeMap<u64, (Value, Vec<String>)> = BTreeMap::new();
    let mut done = Value::Null;
    let mut emit = |event: &Value| -> Result<()> {
        if event["event"] == "done" {
            done = event.clone();
        } else {
            let index = event["step_index"].as_u64().unwrap_or_default();
            let entry = steps
                .entry(index)
                .or_insert_with(|| (event.clone(), Vec::new()));
            entry
                .1
                .push(event["text"].as_str().unwrap_or_default().to_string());
        }
        if !text {
            return Ok(());
        }
        let field = |k: &str| event[k].as_str().unwrap_or_default();
        let line = if event["event"] == "done" {
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
    let ok = pipelines::tail(ctx.c, path, &uuid, &t, &mut emit)?;
    if text {
        return Ok(Out::Streamed(if ok { 0 } else { 1 }));
    }
    if !ok {
        util::mark_failed();
    }
    // The steps' final states, read once at the end.
    let (finals, _) = pipelines::step_list(ctx.c, path, &uuid, &bitbucket::Paging::new(1000))?;
    let state_of = |index: u64| {
        finals
            .iter()
            .find(|s| s["index"].as_u64() == Some(index))
            .map(bitbucket::state_of)
            .unwrap_or_default()
    };
    let records: Vec<Value> = steps
        .into_iter()
        .map(|(index, (first, lines))| {
            let (state, result) = state_of(index);
            json!({
                "pipeline": p["build_number"],
                "pipelineUuid": uuid,
                "step": first["step"],
                "stepIndex": index,
                "stepUuid": first["step_uuid"],
                "state": state,
                "result": result,
                "log": lines.join("\n"),
                "matches": null,
            })
        })
        .collect();
    if !done.is_null() && o.step.is_none() {
        crate::output::notes::warn_value(json!({
            "code": "pipeline_done",
            "message": format!(
                "pipeline {} ended {} {}",
                done["pipeline"],
                done["state"].as_str().unwrap_or_default(),
                done["result"].as_str().unwrap_or_default()
            ),
            "state": done["state"],
            "result": done["result"],
        }));
    }
    Ok(Out::Data(records.into()))
}

pub(super) fn deployment(ctx: &Ctx, cmd: DeploymentCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        DeploymentCmd::List {
            environment,
            status,
            pipeline,
            started_after,
            started_before,
            oldest_first,
            repo,
            list,
        } => Ok(listed(deployments::deployment_list(
            c,
            &ctx.repo(&repo)?,
            &deployments::Filter {
                environment: environment.as_deref(),
                statuses: &status,
                pipeline: pipeline.as_deref(),
                after: started_after.as_deref(),
                before: started_before.as_deref(),
                ascending: oldest_first,
            },
            &list.paging(),
        )?)),
        DeploymentCmd::Get {
            uuids,
            environment,
            repo,
            batch,
        } => {
            let p = ctx.repo(&repo)?;
            get_many(uuids, &batch, |u| {
                deployments::get(c, &p, u, environment.as_deref())
            })
        }
        DeploymentCmd::Commits {
            uuid,
            environment,
            limit,
            repo,
        } => deployments::commits(c, &ctx.repo(&repo)?, &uuid, environment.as_deref(), limit),
    }
}

pub(super) fn environment(ctx: &Ctx, cmd: EnvironmentCmd) -> Result<Value> {
    let c = ctx.c;
    match cmd {
        EnvironmentCmd::List { repo, list } => Ok(listed(deployments::environments(
            c,
            &ctx.repo(&repo)?,
            &list.paging(),
        )?)),
        EnvironmentCmd::Variable {
            cmd:
                VariableCmd::List {
                    environment,
                    repo,
                    list,
                },
        } => Ok(listed(deployments::variables(
            c,
            &ctx.repo(&repo)?,
            &environment,
            &list.paging(),
        )?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

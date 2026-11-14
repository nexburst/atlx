//! `atlx login`, `logout`, `whoami` and `setup`, plus interactive prompts.

use super::Globals;
use super::util::text_arg;
use crate::config::{self, Config};
use crate::{bitbucket, jira};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};

#[derive(Args)]
pub(crate) struct LoginArgs {
    /// API token (https://id.atlassian.com/manage-profile/security/api-tokens); `-` reads stdin
    #[arg(long)]
    token: Option<String>,
    /// Skip credential verification
    #[arg(long)]
    no_verify: bool,
    /// Re-prompt for everything, ignoring saved values
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
pub(crate) enum SetupCmd {
    /// Set up the Bitbucket workspace and token
    Bitbucket {
        #[arg(long)]
        workspace: Option<String>,
        /// Bitbucket-scoped API token; `-` reads stdin
        #[arg(long)]
        token: Option<String>,
        /// Bitbucket account email, if different from the Atlassian one
        #[arg(long)]
        email: Option<String>,
        /// Prompt even when a Bitbucket token is already configured
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_verify: bool,
    },
    /// Save the default Atlassian site (no re-login)
    DefaultSite { site: Option<String> },
}

pub(crate) fn logout() -> Result<Value> {
    let p = config::path()?;
    let existed = p.exists();
    if existed {
        std::fs::remove_file(&p)?;
    }
    Ok(json!({"removed": existed, "config": p.display().to_string()}))
}

pub(crate) fn whoami(g: &Globals) -> Result<Value> {
    let cfg = g.config()?;
    let me = jira::myself(&cfg.jira()?)?;
    Ok(
        json!({"name": me["displayName"], "email": me["emailAddress"], "account_id": me["accountId"],
               "site": cfg.site, "bitbucket_workspace": cfg.bitbucket.workspace}),
    )
}

#[cfg(unix)]
struct EchoGuard;

#[cfg(unix)]
impl EchoGuard {
    /// Disable terminal echo for stdin and restore it automatically on drop.
    fn disable() -> Result<Self> {
        let status = std::process::Command::new("stty")
            .arg("-echo")
            .stdin(std::process::Stdio::inherit())
            .status()
            .context("disabling terminal echo via stty")?;
        if !status.success() {
            bail!("failed to disable terminal echo via stty");
        }
        Ok(Self)
    }
}

#[cfg(unix)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .stdin(std::process::Stdio::inherit())
            .status();
        eprintln!();
    }
}

#[cfg(not(unix))]
struct EchoGuard;

#[cfg(not(unix))]
impl EchoGuard {
    fn disable() -> Result<Self> {
        Ok(Self)
    }
}

fn read_prompt_line(label: &str, secret: bool) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("missing {label}; pass it as a flag");
    }
    eprint!("{label}: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    {
        let _guard = if secret {
            Some(EchoGuard::disable()?)
        } else {
            None
        };
        std::io::stdin().read_line(&mut line)?;
    }
    let line = line.trim().to_string();
    if line.is_empty() {
        bail!("{label} is required");
    }
    Ok(line)
}

fn prompt(label: &str) -> Result<String> {
    read_prompt_line(label, false)
}

fn prompt_secret(label: &str) -> Result<String> {
    read_prompt_line(label, true)
}

fn keep(saved: Option<String>, force: bool) -> Option<String> {
    saved.filter(|_| !force)
}

pub(crate) fn login(g: &Globals, a: LoginArgs) -> Result<Value> {
    let mut cfg = Config::load_file()?;
    let site = match g.site.clone().or(keep(cfg.site.take(), a.force)) {
        Some(s) => s,
        None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
    };
    cfg.site = Some(config::normalize_site(&site));
    cfg.email = Some(match g.user.clone().or(keep(cfg.email.take(), a.force)) {
        Some(s) => s,
        None => prompt("Email")?,
    });
    cfg.token = Some(match a.token {
        Some(t) => text_arg(t)?.trim().to_string(),
        None => match keep(cfg.token.take(), a.force) {
            Some(t) => t,
            None => prompt_secret(
                "API token (https://id.atlassian.com/manage-profile/security/api-tokens)",
            )?,
        },
    });
    cfg.timeout_ms = g.timeout_ms;
    let mut out = json!({"site": cfg.site, "email": cfg.email});
    if !a.no_verify {
        let me = jira::myself(&cfg.jira()?).context("credentials check failed; nothing saved")?;
        out["name"] = me["displayName"].clone();
        out["account_id"] = me["accountId"].clone();
    }
    out["saved"] = cfg.save()?.display().to_string().into();
    Ok(out)
}

pub(crate) fn setup(g: &Globals, cmd: SetupCmd) -> Result<Value> {
    let mut cfg = Config::load_file()?;
    match cmd {
        SetupCmd::DefaultSite { site } => {
            let site = match site.or(g.site.clone()) {
                Some(s) => s,
                None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
            };
            cfg.site = Some(config::normalize_site(&site));
            Ok(json!({"site": cfg.site, "saved": cfg.save()?.display().to_string()}))
        }
        SetupCmd::Bitbucket {
            workspace,
            token,
            email,
            force,
            no_verify,
        } => {
            let ws = match workspace.or(keep(cfg.bitbucket.workspace.take(), force)) {
                Some(w) => w,
                None => prompt("Bitbucket workspace slug")?,
            };
            cfg.bitbucket.workspace = Some(ws.clone());
            if email.is_some() {
                cfg.bitbucket.email = email;
            }
            cfg.bitbucket.token = Some(match token {
                Some(t) => text_arg(t)?.trim().to_string(),
                None => match keep(cfg.bitbucket.token.take(), force) {
                    Some(t) => t,
                    None => prompt_secret(
                        "Bitbucket API token (scopes: account, repository, pullrequest, pipeline)",
                    )?,
                },
            });
            cfg.timeout_ms = g.timeout_ms;
            let mut out = json!({"workspace": ws});
            if !no_verify {
                let (c, _) = cfg.bitbucket()?;
                let q = bitbucket::RepoQuery {
                    query: None,
                    project: None,
                    language: None,
                    sort: "-updated_on",
                    role: None,
                    limit: 1,
                };
                bitbucket::repo_query(&c, &ws, &q)
                    .context("Bitbucket check failed; nothing saved")?;
                out["verified"] = true.into();
            }
            out["saved"] = cfg.save()?.display().to_string().into();
            Ok(out)
        }
    }
}

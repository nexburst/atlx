//! `atlx login`, `logout`, `whoami`, `setup` and `auth profile`, plus
//! interactive prompts.

use super::Globals;
use super::util::{require_yes, text_arg};
use crate::client::ApiError;
use crate::config::{self, Config, ConfigFile, DEFAULT_PROFILE, Profile};
use crate::{bitbucket, jira};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};

#[derive(Args)]
pub(crate) struct LoginArgs {
    /// API token (https://id.atlassian.com/manage-profile/security/api-tokens); `-` reads stdin
    #[arg(long, conflicts_with = "token_cmd")]
    token: Option<String>,
    /// Save a shell command that prints the API token, instead of the token itself (atlx extra)
    #[arg(long, value_name = "CMD")]
    token_cmd: Option<String>,
    /// Skip credential verification
    #[arg(long)]
    no_verify: bool,
    /// Re-prompt for everything, ignoring saved values
    #[arg(short, long)]
    force: bool,
    /// Save into a new profile; without a name, one is derived from the site
    #[arg(long, value_name = "NAME", num_args = 0..=1, default_missing_value = "")]
    new_profile: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum SetupCmd {
    /// Set up the Bitbucket workspace and token
    #[command(visible_alias = "bb")]
    Bitbucket {
        #[arg(long)]
        workspace: Option<String>,
        /// Bitbucket-scoped API token; `-` reads stdin
        #[arg(long, conflicts_with = "token_cmd")]
        token: Option<String>,
        /// Save a shell command that prints the Bitbucket token (atlx extra)
        #[arg(long, value_name = "CMD")]
        token_cmd: Option<String>,
        /// Bitbucket account email, if different from the Atlassian one
        #[arg(long)]
        email: Option<String>,
        /// Prompt even when a Bitbucket token is already configured
        #[arg(short, long)]
        force: bool,
        #[arg(long)]
        no_verify: bool,
    },
    /// Save the default Atlassian site (no re-login)
    DefaultSite { site: Option<String> },
}

#[derive(Subcommand)]
pub(crate) enum AuthCmd {
    /// List, rename and choose the default login profile
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProfileCmd {
    /// Show each profile's site, email, workspace and token source (never the token)
    List,
    /// Move the top-level (v1) credentials into a named profile
    Migrate {
        /// Confirm the move
        #[arg(short, long)]
        yes: bool,
    },
    /// Rename a profile (renaming `default` moves the top-level keys)
    Rename { name: String, new_name: String },
    /// Make a profile the one used when -p and ATLX_PROFILE are not set
    Default { name: String },
}

/// The profile a write command targets: `-p`/`ATLX_PROFILE`, else the saved
/// default. Named profiles that don't exist yet are created on save.
fn target(g: &Globals, file: &ConfigFile) -> Result<(String, Profile)> {
    let name = file.resolve_name(config::requested_profile(g.profile.as_deref()).as_deref());
    if name != DEFAULT_PROFILE {
        config::validate_name(&name)?;
    }
    let p = file.get(&name).cloned().unwrap_or_default();
    Ok((name, p))
}

/// Store `p` as `name`; a profile created into a file with no default
/// becomes the default.
fn store(file: &mut ConfigFile, name: &str, p: Profile) {
    let creating = !file.exists(name);
    *file.get_mut(name) = p;
    if creating && file.default_name() == DEFAULT_PROFILE && file.base.is_empty() {
        file.default_profile = Some(name.to_string());
    }
}

pub(crate) fn logout(g: &Globals) -> Result<Value> {
    let p = config::path()?;
    let mut file = ConfigFile::load_from(&p)?;
    let name = file.resolve_name(config::requested_profile(g.profile.as_deref()).as_deref());
    if !file.exists(&name) {
        return Err(ApiError::usage(format!("no profile named `{name}`")).into());
    }
    let removed = file.remove(&name);
    let mut out = json!({"profile": name, "removed": removed, "config": p.display().to_string()});
    if file.is_empty() {
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
        out["file_removed"] = true.into();
    } else {
        file.save_to(&p)?;
    }
    Ok(out)
}

pub(crate) fn whoami(g: &Globals) -> Result<Value> {
    let cfg = g.config()?;
    let me = jira::myself(&cfg.jira()?)?;
    Ok(json!({
        "name": me["displayName"], "email": me["emailAddress"], "account_id": me["accountId"],
        "site": cfg.site, "bitbucket_workspace": cfg.bitbucket.workspace,
        "profile": cfg.profile, "token_source": cfg.token_source(),
    }))
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

/// Fill in `token`/`token_cmd`: an explicit flag wins, then the saved value
/// (unless `force`), then an interactive prompt.
fn set_token(
    token: &mut Option<String>,
    token_cmd: &mut Option<String>,
    flag: Option<String>,
    cmd_flag: Option<String>,
    force: bool,
    label: &str,
) -> Result<()> {
    if let Some(t) = flag {
        (*token, *token_cmd) = (Some(text_arg(t)?.trim().to_string()), None);
    } else if let Some(c) = cmd_flag {
        if c.trim().is_empty() {
            return Err(ApiError::usage("--token-cmd must not be empty").into());
        }
        (*token, *token_cmd) = (None, Some(c));
    } else if force || (token.is_none() && token_cmd.is_none()) {
        (*token, *token_cmd) = (Some(prompt_secret(label)?), None);
    }
    Ok(())
}

pub(crate) fn login(g: &Globals, a: LoginArgs) -> Result<Value> {
    let mut file = ConfigFile::load()?;
    let (name, mut p) = match a.new_profile.as_deref() {
        Some("") => (None, Profile::default()),
        Some(n) => {
            config::validate_name(n)?;
            if file.exists(n) {
                return Err(ApiError::usage(format!(
                    "profile `{n}` already exists; update it with `atlx login -p {n}`"
                ))
                .into());
            }
            (Some(n.to_string()), Profile::default())
        }
        None => {
            let (n, p) = target(g, &file)?;
            (Some(n), p)
        }
    };
    let site = match g.site.clone().or(keep(p.site.take(), a.force)) {
        Some(s) => s,
        None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
    };
    p.site = Some(config::normalize_site(&site));
    p.email = Some(match g.user.clone().or(keep(p.email.take(), a.force)) {
        Some(s) => s,
        None => prompt("Email")?,
    });
    set_token(
        &mut p.token,
        &mut p.token_cmd,
        a.token,
        a.token_cmd,
        a.force,
        "API token (https://id.atlassian.com/manage-profile/security/api-tokens)",
    )?;
    let name = name.unwrap_or_else(|| file.suggest_name(p.site.as_deref()));
    let mut out = json!({
        "profile": name, "site": p.site, "email": p.email, "token_source": p.token_source(),
    });
    if !a.no_verify {
        let mut cfg = Config::from_profile(&name, &p);
        cfg.timeout_ms = g.timeout_ms;
        let me = jira::myself(&cfg.jira()?).context("credentials check failed; nothing saved")?;
        out["name"] = me["displayName"].clone();
        out["account_id"] = me["accountId"].clone();
    }
    store(&mut file, &name, p);
    out["default"] = (file.default_name() == name).into();
    out["saved"] = file.save()?.display().to_string().into();
    Ok(out)
}

pub(crate) fn setup(g: &Globals, cmd: SetupCmd) -> Result<Value> {
    let mut file = ConfigFile::load()?;
    let (name, mut p) = target(g, &file)?;
    match cmd {
        SetupCmd::DefaultSite { site } => {
            let site = match site.or(g.site.clone()) {
                Some(s) => s,
                None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
            };
            p.site = Some(config::normalize_site(&site));
            let site = p.site.clone();
            store(&mut file, &name, p);
            Ok(json!({"profile": name, "site": site, "saved": file.save()?.display().to_string()}))
        }
        SetupCmd::Bitbucket {
            workspace,
            token,
            token_cmd,
            email,
            force,
            no_verify,
        } => {
            let ws = match workspace.or(keep(p.bitbucket.workspace.take(), force)) {
                Some(w) => w,
                None => prompt("Bitbucket workspace slug")?,
            };
            p.bitbucket.workspace = Some(ws.clone());
            if email.is_some() {
                p.bitbucket.email = email;
            }
            set_token(
                &mut p.bitbucket.token,
                &mut p.bitbucket.token_cmd,
                token,
                token_cmd,
                force,
                "Bitbucket API token (scopes: account, repository, pullrequest, pipeline)",
            )?;
            let mut out = json!({
                "profile": name, "workspace": ws, "token_source": p.bitbucket_token_source(),
            });
            if !no_verify {
                let mut cfg = Config::from_profile(&name, &p);
                cfg.timeout_ms = g.timeout_ms;
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
            store(&mut file, &name, p);
            out["saved"] = file.save()?.display().to_string().into();
            Ok(out)
        }
    }
}

pub(crate) fn auth_cmd(g: &Globals, cmd: AuthCmd) -> Result<Value> {
    let AuthCmd::Profile { cmd } = cmd;
    let mut file = ConfigFile::load()?;
    match cmd {
        ProfileCmd::List => {
            let active =
                file.resolve_name(config::requested_profile(g.profile.as_deref()).as_deref());
            let rows: Vec<Value> = file
                .names()
                .into_iter()
                .map(|n| {
                    let p = file.get(&n).cloned().unwrap_or_default();
                    json!({
                        "name": n,
                        "default": file.default_name() == n,
                        "active": active == n,
                        "site": p.site,
                        "email": p.email,
                        "bitbucket_workspace": p.bitbucket.workspace,
                        "token_source": p.token_source(),
                        "bitbucket_token_source": p.bitbucket_token_source(),
                    })
                })
                .collect();
            Ok(Value::Array(rows))
        }
        ProfileCmd::Default { name } => {
            if !file.exists(&name) {
                return Err(ApiError::usage(format!("no profile named `{name}`")).into());
            }
            file.default_profile = (name != DEFAULT_PROFILE).then(|| name.clone());
            Ok(json!({"default_profile": name, "saved": file.save()?.display().to_string()}))
        }
        ProfileCmd::Rename { name, new_name } => {
            file.rename(&name, &new_name)?;
            Ok(json!({
                "renamed": name, "profile": new_name,
                "default": file.default_name() == new_name,
                "saved": file.save()?.display().to_string(),
            }))
        }
        ProfileCmd::Migrate { yes } => {
            if file.base.is_empty() {
                return Ok(
                    json!({"migrated": false, "detail": "no top-level credentials to migrate"}),
                );
            }
            let to = file.suggest_name(file.base.site.as_deref());
            require_yes(
                yes,
                &format!("moving the top-level credentials into profile `{to}`"),
            )?;
            file.rename(DEFAULT_PROFILE, &to)?;
            Ok(json!({
                "migrated": true, "profile": to,
                "default": file.default_name() == to,
                "saved": file.save()?.display().to_string(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_token_prefers_flags_then_saved_then_prompt() {
        let (mut t, mut c) = (Some("old".to_string()), None);
        set_token(&mut t, &mut c, None, Some("pass x".into()), false, "x").unwrap();
        assert_eq!((t.as_deref(), c.as_deref()), (None, Some("pass x")));
        // Saved command kept when nothing new is given.
        set_token(&mut t, &mut c, None, None, false, "x").unwrap();
        assert_eq!(c.as_deref(), Some("pass x"));
        set_token(&mut t, &mut c, Some("new".into()), None, false, "x").unwrap();
        assert_eq!((t.as_deref(), c.as_deref()), (Some("new"), None));
        assert!(set_token(&mut t, &mut c, None, Some(" ".into()), false, "x").is_err());
    }
}

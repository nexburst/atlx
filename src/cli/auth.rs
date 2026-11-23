//! `atlx auth`: `login`, `logout`, `status`, `bitbucket` and `profile …`.
//!
//! Nothing here prompts unless stdin is a terminal, so an agent never
//! waits on input: missing credentials are a `config` error telling it to
//! ask the person to run `atlx auth login`.

use super::Globals;
use super::util::{require_yes, text_arg};
use crate::client::{ApiError, Client};
use crate::config::{self, Config, ConfigFile, DEFAULT_PROFILE, Profile};
use crate::schema::{CommandMeta, Effect, Field, Record};
use crate::{bitbucket, jira};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};

#[derive(Subcommand)]
pub(crate) enum AuthCmd {
    /// Save credentials (site, email, API token) into a profile
    Login(LoginArgs),
    /// Remove the selected profile and its saved credentials
    Logout,
    /// Who is signed in, on which site, with which token source; which
    /// products answer; whether read-only mode is on. Run this first
    Status,
    /// Save the Bitbucket workspace and token for the profile
    Bitbucket(BitbucketArgs),
    /// List, rename and choose login profiles
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
}

#[derive(Args)]
pub(crate) struct LoginArgs {
    /// API token (https://id.atlassian.com/manage-profile/security/api-tokens); `-` reads stdin
    #[arg(long, conflicts_with = "token_cmd")]
    token: Option<String>,
    /// Save a shell command that prints the API token, instead of the token itself
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

#[derive(Args)]
pub(crate) struct BitbucketArgs {
    /// Default workspace slug
    #[arg(long)]
    workspace: Option<String>,
    /// Bitbucket-scoped API token; `-` reads stdin
    #[arg(long, conflicts_with = "token_cmd")]
    token: Option<String>,
    /// Save a shell command that prints the Bitbucket token
    #[arg(long, value_name = "CMD")]
    token_cmd: Option<String>,
    /// Bitbucket account email, if different from the Atlassian one
    #[arg(long)]
    email: Option<String>,
    /// Prompt even when a Bitbucket token is already configured
    #[arg(short, long)]
    force: bool,
    /// Skip the check that the token can list the workspace's repositories
    #[arg(long)]
    no_verify: bool,
}

#[derive(Subcommand)]
pub(crate) enum ProfileCmd {
    /// Show each profile's site, email, workspace and token source (never the token)
    List,
    /// Save `-s/--site` as the selected profile's site, without logging in again
    Update,
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

const USER: crate::schema::Source =
    crate::schema::Source::Object(&[("id", "user.id"), ("name", "user.name")]);

/// `auth status`.
pub(crate) const STATUS: Record = Record {
    name: "auth.status",
    description: "the signed-in account, its site and what it can reach",
    primary: "profile",
    fields: &[
        Field::new("profile", "string", "selected profile", "profile"),
        Field::new("site", "string", "Atlassian site", "site"),
        Field::new("email", "string", "account email", "email"),
        Field::new("user", "user", "the Jira account ({id, name})", "user").source(USER),
        Field::new(
            "tokenSource",
            "string",
            "where the token comes from: env, config, token_cmd",
            "tokenSource",
        ),
        Field::new(
            "readOnly",
            "boolean",
            "whether mutating requests are refused",
            "readOnly",
        ),
        Field::new(
            "products",
            "object",
            "per product: ok, not_configured, or the error code it answered with",
            "products",
        ),
        Field::new(
            "bitbucketWorkspace",
            "string",
            "default Bitbucket workspace",
            "bitbucketWorkspace",
        ),
        Field::new(
            "defaults",
            "object",
            "repo-local defaults (.atlx.toml and the git remote)",
            "defaults",
        )
        .full(),
    ],
};

/// `auth profile list`.
pub(crate) const PROFILE: Record = Record {
    name: "auth.profile",
    description: "a saved login profile (never the token)",
    primary: "name",
    fields: &[
        Field::new("name", "string", "profile name", "name"),
        Field::new(
            "default",
            "boolean",
            "used when none is selected",
            "default",
        ),
        Field::new("active", "boolean", "selected for this run", "active"),
        Field::new("site", "string", "Atlassian site", "site"),
        Field::new("email", "string", "account email", "email"),
        Field::new(
            "workspace",
            "string",
            "default Bitbucket workspace",
            "workspace",
        ),
        Field::new(
            "tokenSource",
            "string",
            "config, token_cmd or none",
            "tokenSource",
        ),
        Field::new(
            "bitbucketTokenSource",
            "string",
            "where the Bitbucket token comes from",
            "bitbucketTokenSource",
        )
        .full(),
        Field::new("readOnly", "boolean", "read-only profile", "readOnly"),
    ],
};

pub(crate) const META: &[CommandMeta] = &[
    CommandMeta::new("auth login", Effect::Write).examples(&[
        "atlx auth login --new-profile work -s acme -u me@acme.com --token-cmd 'pass show atlassian'",
        "atlx auth login -p work --force",
    ]),
    CommandMeta::new("auth logout", Effect::Write).examples(&["atlx auth logout -p work"]),
    CommandMeta::new("auth status", Effect::Read)
        .record(&STATUS)
        .examples(&["atlx auth status", "atlx auth status -p work --fields user,products"]),
    CommandMeta::new("auth bitbucket", Effect::Write).examples(&[
        "atlx auth bitbucket --workspace acme --token-cmd 'pass show bitbucket'",
    ]),
    CommandMeta::new("auth profile list", Effect::Read)
        .record(&PROFILE)
        .examples(&["atlx auth profile list", "atlx auth profile list -o ids"]),
    CommandMeta::new("auth profile update", Effect::Write)
        .examples(&["atlx auth profile update -p work -s acme.atlassian.net"]),
    CommandMeta::new("auth profile migrate", Effect::Destructive)
        .examples(&["atlx auth profile migrate --dry-run", "atlx auth profile migrate --yes"]),
    CommandMeta::new("auth profile rename", Effect::Write)
        .examples(&["atlx auth profile rename default work"]),
    CommandMeta::new("auth profile default", Effect::Write)
        .examples(&["atlx auth profile default work"]),
];

pub(crate) fn run(g: &Globals, cmd: AuthCmd) -> Result<Value> {
    match cmd {
        AuthCmd::Login(a) => login(g, a),
        AuthCmd::Logout => logout(g),
        AuthCmd::Status => status(g),
        AuthCmd::Bitbucket(a) => setup_bitbucket(g, a),
        AuthCmd::Profile { cmd } => profile_cmd(g, cmd),
    }
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

/// Local config writes honour `--dry-run` and read-only mode too.
fn guard_local(g: &Globals, what: &str) -> Result<()> {
    if g.read_only_requested() {
        return Err(ApiError::refused(format!("read-only mode: not {what}")).into());
    }
    Ok(())
}

fn save(g: &Globals, file: &ConfigFile, what: &str) -> Result<Value> {
    let p = config::path()?;
    if g.dry_run {
        return Err(crate::client::DryRun {
            method: "WRITE".into(),
            url: p.display().to_string(),
            body: Some(json!({"action": what})),
        }
        .into());
    }
    guard_local(g, what)?;
    file.save_to(&p)?;
    Ok(p.display().to_string().into())
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
        if g.dry_run {
            return Err(crate::client::DryRun {
                method: "DELETE".into(),
                url: p.display().to_string(),
                body: None,
            }
            .into());
        }
        guard_local(g, "removing the config file")?;
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
        out["fileRemoved"] = true.into();
    } else {
        save(g, &file, &format!("removing profile {name}"))?;
    }
    Ok(out)
}

/// `auth status`: one call that says who, where, how, and what answers.
pub(crate) fn status(g: &Globals) -> Result<Value> {
    let cfg = g.config()?;
    let jira_client = cfg.jira()?;
    let (me, confluence, bb) = std::thread::scope(|s| {
        let cf = s.spawn(|| product_check(cfg.confluence(), "/api/v2/spaces", &[("limit", "1")]));
        let bb = s.spawn(|| {
            if !cfg.bitbucket_configured() {
                return Value::from("not_configured");
            }
            product_check(cfg.bitbucket().map(|(c, _)| c), "/user", &[])
        });
        let me = jira::myself(&jira_client);
        (
            me,
            cf.join().unwrap_or_else(|_| "other".into()),
            bb.join().unwrap_or_else(|_| "other".into()),
        )
    });
    let me = me?;
    Ok(json!({
        "profile": cfg.profile,
        "site": cfg.site.as_deref().map(config::normalize_site),
        "email": cfg.email,
        "user": {"id": me["accountId"], "name": me["displayName"]},
        "tokenSource": cfg.token_source(),
        "readOnly": cfg.read_only,
        "products": {"jira": "ok", "confluence": confluence, "bitbucket": bb},
        "bitbucketWorkspace": cfg.bitbucket.workspace,
        "defaults": {
            "file": cfg.local.path.as_ref().map(|p| p.display().to_string()),
            "jiraProject": cfg.local.jira.project,
            "confluenceSpace": cfg.local.confluence.space,
            "bitbucketRepo": cfg.default_repo,
        },
    }))
}

/// `ok`, or the error code a product answered a cheap read with.
fn product_check(client: Result<Client>, path: &str, query: &[(&str, &str)]) -> Value {
    match client.and_then(|c| c.get(path, query)) {
        Ok(_) => "ok".into(),
        Err(e) => ApiError::from_anyhow(&e).code().into(),
    }
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
        return Err(ApiError::config(format!("missing {label}"))
            .with_hint(
                "ask the person to run `atlx auth login` in a terminal; never pass a token on the command line",
            )
            .into());
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
                    "profile `{n}` already exists; update it with `atlx auth login -p {n}`"
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
        "profile": name, "site": p.site, "email": p.email, "tokenSource": p.token_source(),
    });
    if !a.no_verify {
        let mut cfg = Config::from_profile(&name, &p);
        cfg.timeout_ms = g.timeout_ms;
        let me = jira::myself(&cfg.jira()?).context("credentials check failed; nothing saved")?;
        out["user"] = json!({"id": me["accountId"], "name": me["displayName"]});
    }
    store(&mut file, &name, p);
    out["default"] = (file.default_name() == name).into();
    out["saved"] = save(g, &file, &format!("saving profile {name}"))?;
    Ok(out)
}

fn setup_bitbucket(g: &Globals, a: BitbucketArgs) -> Result<Value> {
    let mut file = ConfigFile::load()?;
    let (name, mut p) = target(g, &file)?;
    let ws = match a.workspace.or(keep(p.bitbucket.workspace.take(), a.force)) {
        Some(w) => w,
        None => prompt("Bitbucket workspace slug")?,
    };
    p.bitbucket.workspace = Some(ws.clone());
    if a.email.is_some() {
        p.bitbucket.email = a.email;
    }
    set_token(
        &mut p.bitbucket.token,
        &mut p.bitbucket.token_cmd,
        a.token,
        a.token_cmd,
        a.force,
        "Bitbucket API token (scopes: account, repository, pullrequest, pipeline)",
    )?;
    let mut out = json!({
        "profile": name, "workspace": ws, "tokenSource": p.bitbucket_token_source(),
    });
    if !a.no_verify {
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
        bitbucket::repo_query(&c, &ws, &q).context("Bitbucket check failed; nothing saved")?;
        out["verified"] = true.into();
    }
    store(&mut file, &name, p);
    out["saved"] = save(g, &file, &format!("saving Bitbucket settings for {name}"))?;
    Ok(out)
}

fn profile_cmd(g: &Globals, cmd: ProfileCmd) -> Result<Value> {
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
                        "workspace": p.bitbucket.workspace,
                        "tokenSource": p.token_source(),
                        "bitbucketTokenSource": p.bitbucket_token_source(),
                        "readOnly": p.read_only.unwrap_or(false),
                    })
                })
                .collect();
            Ok(Value::Array(rows))
        }
        ProfileCmd::Update => {
            let Some(site) = g.site.clone() else {
                return Err(ApiError::usage("pass the new site with -s/--site")
                    .with_suggestions(["atlx auth profile update -s acme.atlassian.net".into()])
                    .into());
            };
            let (name, mut p) = target(g, &file)?;
            p.site = Some(config::normalize_site(&site));
            let site = p.site.clone();
            store(&mut file, &name, p);
            let saved = save(g, &file, &format!("updating profile {name}"))?;
            Ok(json!({"profile": name, "site": site, "saved": saved}))
        }
        ProfileCmd::Default { name } => {
            if !file.exists(&name) {
                return Err(ApiError::usage(format!("no profile named `{name}`")).into());
            }
            file.default_profile = (name != DEFAULT_PROFILE).then(|| name.clone());
            let saved = save(g, &file, &format!("making {name} the default"))?;
            Ok(json!({"defaultProfile": name, "saved": saved}))
        }
        ProfileCmd::Rename { name, new_name } => {
            file.rename(&name, &new_name)?;
            let saved = save(g, &file, &format!("renaming {name} to {new_name}"))?;
            Ok(json!({
                "renamed": name, "profile": new_name,
                "default": file.default_name() == new_name,
                "saved": saved,
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
            let saved = save(g, &file, &format!("moving the top-level profile to {to}"))?;
            Ok(json!({
                "migrated": true, "profile": to,
                "default": file.default_name() == to,
                "saved": saved,
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

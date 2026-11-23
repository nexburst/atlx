//! Configuration: a TOML file (`~/.config/atlx/config.toml`) holding one or
//! more named profiles, overridden by `ATLX_*` environment variables.
//!
//! The file layout (all keys optional):
//!
//! ```toml
//! default_profile = "work"        # profile used when none is selected
//!
//! # Top-level keys form the implicit `default` profile (the v1 layout).
//! site = "acme.atlassian.net"
//! email = "me@acme.com"
//! token = "..."                    # or: token_cmd = "op read op://..."
//! [bitbucket]
//! workspace = "acme"
//!
//! [profiles.work]
//! site = "work.atlassian.net"
//! email = "me@work.com"
//! token_cmd = "pass show atlassian/work"
//! [profiles.work.bitbucket]
//! workspace = "work"
//! token_cmd = "pass show bitbucket/work"
//! ```
//!
//! Precedence, highest first: `-s`/`-u` flags, `ATLX_*` env vars, the
//! selected profile (`-p`/`ATLX_PROFILE`, then the `profile` of a
//! repo-local `.atlx.toml`, then the saved default).
//!
//! A profile may set `read_only = true`; so may `ATLX_READ_ONLY=1` or
//! `--read-only`. Then every client refuses mutating requests.
//!
//! The repo-local `.atlx.toml` (found by walking up from the working
//! directory) holds non-secret defaults only:
//!
//! ```toml
//! profile = "work"
//! [jira]
//! project = "PROJ"
//! [confluence]
//! space = "ENG"
//! [bitbucket]
//! repo = "acme/api"
//! ```
//!
//! Without `bitbucket.repo`, the Bitbucket repository defaults to the
//! `origin` remote in `.git/config` when it points at bitbucket.org.

use crate::client::{ApiError, Client, Mode};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const BITBUCKET_API: &str = "https://api.bitbucket.org/2.0";

/// Environment variable that points Bitbucket at a local mock server, for
/// tests. Only loopback URLs are accepted, so it can't send the token
/// anywhere else.
pub const BITBUCKET_API_ENV: &str = "ATLX_BITBUCKET_API";

/// The Bitbucket API base: [`BITBUCKET_API`], or a loopback override from
/// [`BITBUCKET_API_ENV`].
pub fn bitbucket_api() -> Result<String> {
    match std::env::var(BITBUCKET_API_ENV) {
        Ok(v) if !v.trim().is_empty() => {
            let v = v.trim().trim_end_matches('/').to_string();
            if crate::client::is_loopback_url(&v) {
                Ok(v)
            } else {
                Err(ApiError::config(format!(
                    "{BITBUCKET_API_ENV} must be a loopback URL (it is for local test servers), got '{v}'"
                ))
                .into())
            }
        }
        _ => Ok(BITBUCKET_API.into()),
    }
}

/// Name of the implicit profile made of the file's top-level keys. As a
/// `-p` value it means "whichever profile is the saved default".
pub const DEFAULT_PROFILE: &str = "default";

/// How long a `token_cmd` may run before it is killed.
pub const TOKEN_CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// Credentials for one Atlassian site (and optionally one Bitbucket workspace).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Atlassian Cloud site, e.g. `acme.atlassian.net`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Atlassian account email used for basic auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Atlassian API token (https://id.atlassian.com/manage-profile/security/api-tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Shell command whose stdout is the API token; used when `token` is unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Bitbucket::is_empty")]
    pub bitbucket: Bitbucket,
    /// Refuse every mutating request made with this profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

impl Profile {
    pub fn is_empty(&self) -> bool {
        self.site.is_none()
            && self.email.is_none()
            && self.token.is_none()
            && self.token_cmd.is_none()
            && self.bitbucket.is_empty()
            && self.read_only.is_none()
    }

    /// Where this profile's Atlassian token would come from (`config`,
    /// `token_cmd` or `none`), without evaluating anything.
    pub fn token_source(&self) -> &'static str {
        source_name(self.token.is_some(), self.token_cmd.is_some(), false)
    }

    /// Like [`Profile::token_source`] for the Bitbucket token, which falls
    /// back to the Atlassian one.
    pub fn bitbucket_token_source(&self) -> &'static str {
        match source_name(
            self.bitbucket.token.is_some(),
            self.bitbucket.token_cmd.is_some(),
            false,
        ) {
            "none" => self.token_source(),
            s => s,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bitbucket {
    /// Default workspace slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Defaults to the top-level `email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// API token with Bitbucket scopes. Defaults to the top-level `token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Shell command whose stdout is the Bitbucket token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cmd: Option<String>,
}

impl Bitbucket {
    fn is_empty(&self) -> bool {
        self.workspace.is_none()
            && self.email.is_none()
            && self.token.is_none()
            && self.token_cmd.is_none()
    }
}

fn source_name(token: bool, cmd: bool, env: bool) -> &'static str {
    match (token, cmd, env) {
        (true, _, true) => "env",
        (true, _, false) => "config",
        (false, true, _) => "token_cmd",
        (false, false, _) => "none",
    }
}

/// The config file as stored on disk.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Profile used when neither `-p` nor `ATLX_PROFILE` picks one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// The implicit `default` profile (top-level keys).
    #[serde(flatten)]
    pub base: Profile,
    /// Named profiles (`[profiles.<name>]`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
}

impl ConfigFile {
    pub fn load() -> Result<Self> {
        Self::load_from(&path()?)
    }

    pub fn load_from(p: &Path) -> Result<Self> {
        if !p.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let f: Self = toml::from_str(text)?;
        if f.profiles.contains_key(DEFAULT_PROFILE) {
            return Err(ApiError::config(format!(
                "`[profiles.{DEFAULT_PROFILE}]` is reserved; the top-level keys are the `{DEFAULT_PROFILE}` profile"
            ))
            .into());
        }
        Ok(f)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let p = path()?;
        self.save_to(&p)?;
        Ok(p)
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(p, &self.to_toml()?).with_context(|| format!("writing {}", p.display()))
    }

    pub fn is_empty(&self) -> bool {
        self.default_profile.is_none() && self.base.is_empty() && self.profiles.is_empty()
    }

    /// The profile used when none is selected.
    pub fn default_name(&self) -> &str {
        self.default_profile
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(DEFAULT_PROFILE)
    }

    /// Resolve a `-p`/`ATLX_PROFILE` value: none, or `default`, means the
    /// saved default.
    pub fn resolve_name(&self, requested: Option<&str>) -> String {
        match requested.map(str::trim).filter(|n| !n.is_empty()) {
            None | Some(DEFAULT_PROFILE) => self.default_name().to_string(),
            Some(n) => n.to_string(),
        }
    }

    /// A profile by its literal name (`default` is the top-level one).
    pub fn get(&self, name: &str) -> Option<&Profile> {
        if name == DEFAULT_PROFILE {
            Some(&self.base)
        } else {
            self.profiles.get(name)
        }
    }

    pub fn exists(&self, name: &str) -> bool {
        name == DEFAULT_PROFILE || self.profiles.contains_key(name)
    }

    /// A profile by literal name, created empty if it does not exist.
    pub fn get_mut(&mut self, name: &str) -> &mut Profile {
        if name == DEFAULT_PROFILE {
            &mut self.base
        } else {
            self.profiles.entry(name.to_string()).or_default()
        }
    }

    /// Remove a profile. Returns whether it held anything.
    pub fn remove(&mut self, name: &str) -> bool {
        let removed = if name == DEFAULT_PROFILE {
            !std::mem::take(&mut self.base).is_empty()
        } else {
            self.profiles.remove(name).is_some()
        };
        if self.default_profile.as_deref() == Some(name) {
            self.default_profile = None;
        }
        removed
    }

    /// Rename a profile, keeping it the default if it was. Renaming the
    /// top-level `default` profile moves its keys into `[profiles.<to>]`.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        validate_name(to)?;
        if self.profiles.contains_key(to) {
            return Err(ApiError::usage(format!("profile `{to}` already exists")).into());
        }
        let was_default = self.default_name() == from;
        let p = if from == DEFAULT_PROFILE {
            if self.base.is_empty() {
                return Err(ApiError::usage(format!(
                    "the `{DEFAULT_PROFILE}` profile is empty; nothing to rename"
                ))
                .into());
            }
            std::mem::take(&mut self.base)
        } else {
            self.profiles
                .remove(from)
                .ok_or_else(|| ApiError::usage(format!("no profile named `{from}`")))?
        };
        self.profiles.insert(to.to_string(), p);
        if was_default {
            self.default_profile = Some(to.to_string());
        }
        Ok(())
    }

    /// Profile names in listing order: `default` first when it holds
    /// anything (or when there is nothing else), then the named ones.
    pub fn names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.base.is_empty() || self.profiles.is_empty() {
            out.push(DEFAULT_PROFILE.to_string());
        }
        out.extend(self.profiles.keys().cloned());
        out
    }

    /// A free profile name derived from a site, e.g. `acme` for
    /// `acme.atlassian.net`, with a numeric suffix if it is taken.
    pub fn suggest_name(&self, site: Option<&str>) -> String {
        let stem: String = site
            .map(normalize_site)
            .and_then(|s| s.split('.').next().map(str::to_string))
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let stem = if stem.is_empty() || stem == DEFAULT_PROFILE {
            "profile".to_string()
        } else {
            stem
        };
        let mut name = stem.clone();
        let mut n = 2;
        while self.profiles.contains_key(&name) {
            name = format!("{stem}-{n}");
            n += 1;
        }
        name
    }
}

/// Check that `name` can be used for a named profile.
pub fn validate_name(name: &str) -> Result<()> {
    if name == DEFAULT_PROFILE {
        return Err(ApiError::usage(format!(
            "`{DEFAULT_PROFILE}` is reserved for the top-level profile"
        ))
        .into());
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ApiError::usage(format!(
            "invalid profile name `{name}`: use letters, digits, `-`, `_` or `.`"
        ))
        .into());
    }
    Ok(())
}

pub fn path() -> Result<PathBuf> {
    if let Some(p) = env("ATLX_CONFIG") {
        return Ok(p.into());
    }
    let base = match env("XDG_CONFIG_HOME") {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(env("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(base.join("atlx").join("config.toml"))
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// The profile asked for by `-p` (if given) or `ATLX_PROFILE`.
pub fn requested_profile(flag: Option<&str>) -> Option<String> {
    flag.map(str::to_string).or_else(|| env("ATLX_PROFILE"))
}

pub fn normalize_site(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.trim_end_matches('/');
    // A bare site prefix is accepted: `acme` -> `acme.atlassian.net`.
    if s.contains('.') {
        s.to_string()
    } else {
        format!("{s}.atlassian.net")
    }
}

/// Run a credential helper and return its trimmed stdout.
///
/// The command runs through `sh -c` (Unix) or `cmd /C` (Windows) with stdin
/// closed and stderr passed through. It is killed after `timeout`. Errors
/// never include what the command printed on stdout.
pub fn run_token_cmd(cmd: &str, timeout: Duration) -> Result<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").raw_arg(cmd);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("token_cmd could not be started")?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let res = stdout.read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send(res);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait().context("waiting for token_cmd")? {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("token_cmd timed out after {}s", timeout.as_secs_f32());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        bail!("token_cmd failed ({status}); its output was discarded");
    }
    // A grandchild may still hold stdout open; don't wait on it forever.
    let out = rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()) + Duration::from_secs(1))
        .map_err(|_| anyhow!("token_cmd exited but its stdout stayed open"))?
        .context("reading token_cmd output")?;
    let token = String::from_utf8(out)
        .map_err(|_| anyhow!("token_cmd printed non-UTF-8 output"))?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("token_cmd printed nothing");
    }
    Ok(token)
}

/// Lazily evaluated secrets, so a `token_cmd` runs at most once per process
/// and only when a client is actually built.
#[derive(Debug, Default, Clone)]
pub struct SecretCache {
    token: OnceLock<std::result::Result<String, String>>,
    bitbucket_token: OnceLock<std::result::Result<String, String>>,
}

/// The effective settings for one run: the selected profile with env vars
/// applied (and, in the CLI, `-s`/`-u` on top).
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Name of the selected profile.
    pub profile: String,
    pub site: Option<String>,
    pub email: Option<String>,
    pub token: Option<String>,
    pub token_cmd: Option<String>,
    pub bitbucket: Bitbucket,
    /// Whether `token` / `token_cmd` came from the environment.
    pub token_from_env: bool,
    /// Whether `bitbucket.token` / `bitbucket.token_cmd` came from the environment.
    pub bitbucket_token_from_env: bool,
    /// Per-request timeout (`--timeout-ms`); not persisted.
    pub timeout_ms: Option<u64>,
    pub cache: SecretCache,
    /// Refuse mutating requests (profile `read_only`, `ATLX_READ_ONLY`,
    /// `--read-only`).
    pub read_only: bool,
    /// Stop at the first mutating request and print it (`--dry-run`).
    pub dry_run: bool,
    /// The repo-local `.atlx.toml` defaults, if one was found.
    pub local: LocalDefaults,
    /// The repository the Bitbucket commands default to: `.atlx.toml`'s
    /// `bitbucket.repo`, else the git `origin` remote on bitbucket.org.
    pub default_repo: Option<String>,
}

impl Config {
    /// Load the config file and select a profile (`flag`, else
    /// `ATLX_PROFILE`, else the saved default), then apply `ATLX_*` env vars.
    pub fn load(flag: Option<&str>) -> Result<Self> {
        let cwd = std::env::current_dir().ok();
        let local = match &cwd {
            Some(d) => LocalDefaults::discover(d)?,
            None => LocalDefaults::default(),
        };
        let mut c = Self::resolve_with(&ConfigFile::load()?, flag, &env, local)?;
        if c.default_repo.is_none()
            && let Some(d) = &cwd
        {
            c.default_repo = git_remote_repo(d);
        }
        Ok(c)
    }

    /// [`Config::load`] with the file and environment passed in.
    pub fn resolve(
        file: &ConfigFile,
        flag: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        Self::resolve_with(file, flag, env, LocalDefaults::default())
    }

    /// [`Config::resolve`] with repo-local defaults, whose `profile` ranks
    /// below `-p` and `ATLX_PROFILE`.
    pub fn resolve_with(
        file: &ConfigFile,
        flag: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
        local: LocalDefaults,
    ) -> Result<Self> {
        let requested = flag
            .map(str::to_string)
            .or_else(|| env("ATLX_PROFILE"))
            .or_else(|| local.profile.clone());
        let name = file.resolve_name(requested.as_deref());
        let profile = file.get(&name).ok_or_else(|| {
            ApiError::config(format!(
                "no profile named `{name}` (known: {}); create it with `atlx login -p {name}`",
                file.names().join(", ")
            ))
        })?;
        let mut c = Self::from_profile(&name, profile);
        c.apply_env(env);
        c.default_repo = local.bitbucket.repo.clone();
        c.local = local;
        Ok(c)
    }

    /// Settings from one profile, without the environment.
    pub fn from_profile(name: &str, p: &Profile) -> Self {
        Self {
            profile: name.to_string(),
            site: p.site.clone(),
            email: p.email.clone(),
            token: p.token.clone(),
            token_cmd: p.token_cmd.clone(),
            bitbucket: p.bitbucket.clone(),
            read_only: p.read_only.unwrap_or(false),
            ..Default::default()
        }
    }

    fn apply_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        let set = |slot: &mut Option<String>, key: &str| {
            if let Some(v) = env(key) {
                *slot = Some(v);
            }
        };
        if let Some(v) = env("ATLX_READ_ONLY") {
            self.read_only = !matches!(v.trim(), "0" | "false" | "no" | "off");
        }
        set(&mut self.site, "ATLX_SITE");
        set(&mut self.email, "ATLX_EMAIL");
        set(&mut self.bitbucket.workspace, "ATLX_BB_WORKSPACE");
        set(&mut self.bitbucket.email, "ATLX_BB_EMAIL");
        // A token from the environment, literal or command, replaces both
        // file keys; a literal token beats a command at the same level.
        if let Some(t) = env("ATLX_TOKEN") {
            (self.token, self.token_cmd, self.token_from_env) = (Some(t), None, true);
        } else if let Some(c) = env("ATLX_TOKEN_CMD") {
            (self.token, self.token_cmd, self.token_from_env) = (None, Some(c), true);
        }
        if let Some(t) = env("ATLX_BB_TOKEN") {
            self.bitbucket.token = Some(t);
            self.bitbucket.token_cmd = None;
            self.bitbucket_token_from_env = true;
        } else if let Some(c) = env("ATLX_BB_TOKEN_CMD") {
            self.bitbucket.token = None;
            self.bitbucket.token_cmd = Some(c);
            self.bitbucket_token_from_env = true;
        }
    }

    /// Where the Atlassian token comes from: `env`, `config`, `token_cmd`
    /// or `none`. Never evaluates a `token_cmd`.
    pub fn token_source(&self) -> &'static str {
        source_name(
            self.token.is_some(),
            self.token_cmd.is_some(),
            self.token_from_env,
        )
    }

    /// Where the Bitbucket token comes from (falls back to the Atlassian one).
    pub fn bitbucket_token_source(&self) -> &'static str {
        match source_name(
            self.bitbucket.token.is_some(),
            self.bitbucket.token_cmd.is_some(),
            self.bitbucket_token_from_env,
        ) {
            "none" => self.token_source(),
            s => s,
        }
    }

    /// How clients built from this config treat mutating requests.
    /// Read-only wins over dry-run.
    pub fn mode(&self) -> Mode {
        if self.read_only {
            Mode::ReadOnly
        } else if self.dry_run {
            Mode::DryRun
        } else {
            Mode::Live
        }
    }

    fn login_hint(&self) -> String {
        if self.profile.is_empty() || self.profile == DEFAULT_PROFILE {
            "atlx auth login".into()
        } else {
            format!("atlx auth login -p {}", self.profile)
        }
    }

    /// `https://SITE`, or `http://127.0.0.1:PORT` when the site is an explicit
    /// `http://` loopback address (used by the offline tests).
    fn site_url(&self) -> Result<String> {
        let raw = self.site.as_deref().ok_or_else(|| {
            ApiError::config(format!(
                "no Atlassian site configured; run `{}` or set ATLX_SITE",
                self.login_hint()
            ))
        })?;
        if let Some(rest) = raw.trim().strip_prefix("http://") {
            let rest = rest.trim_end_matches('/');
            let host = rest.rsplit_once(':').map_or(rest, |(h, _)| h);
            if matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
                return Ok(format!("http://{rest}"));
            }
        }
        Ok(format!("https://{}", normalize_site(raw)))
    }

    fn token(&self) -> Result<String> {
        let res = self
            .cache
            .token
            .get_or_init(|| match (&self.token, &self.token_cmd) {
                (Some(t), _) => Ok(t.clone()),
                (None, Some(cmd)) => {
                    run_token_cmd(cmd, TOKEN_CMD_TIMEOUT).map_err(|e| format!("{e:#}"))
                }
                (None, None) => Err(format!(
                    "no API token configured; run `{}` or set ATLX_TOKEN or ATLX_TOKEN_CMD",
                    self.login_hint()
                )),
            });
        res.clone().map_err(|e| ApiError::config(e).into())
    }

    /// Email, base URL and token, checked in that order; the token (and so
    /// any `token_cmd`) comes last.
    fn creds(&self, suffix: &str) -> Result<(String, String, String)> {
        let email = self.email.clone().ok_or_else(|| {
            ApiError::config(format!(
                "no email configured; run `{}` or set ATLX_EMAIL",
                self.login_hint()
            ))
        })?;
        let base = format!("{}{suffix}", self.site_url()?);
        Ok((base, email, self.token()?))
    }

    pub fn jira(&self) -> Result<Client> {
        let (base, email, token) = self.creds("")?;
        Ok(Client::new(base, &email, &token, self.timeout_ms).with_mode(self.mode()))
    }

    pub fn confluence(&self) -> Result<Client> {
        let (base, email, token) = self.creds("/wiki")?;
        Ok(Client::new(base, &email, &token, self.timeout_ms).with_mode(self.mode()))
    }

    pub fn bitbucket_configured(&self) -> bool {
        self.bitbucket.workspace.is_some()
    }

    fn bitbucket_hint(&self) -> String {
        if self.profile.is_empty() || self.profile == DEFAULT_PROFILE {
            "atlx auth bitbucket".into()
        } else {
            format!("atlx auth bitbucket -p {}", self.profile)
        }
    }

    /// Bitbucket client and default workspace.
    pub fn bitbucket(&self) -> Result<(Client, Option<String>)> {
        let email = self
            .bitbucket
            .email
            .as_deref()
            .or(self.email.as_deref())
            .ok_or_else(|| {
                ApiError::config(format!(
                    "no Bitbucket email configured; run `{}` or set ATLX_BB_EMAIL",
                    self.bitbucket_hint()
                ))
            })?;
        let res = self.cache.bitbucket_token.get_or_init(|| {
            match (&self.bitbucket.token, &self.bitbucket.token_cmd) {
                (Some(t), _) => Ok(t.clone()),
                (None, Some(cmd)) => run_token_cmd(cmd, TOKEN_CMD_TIMEOUT)
                    .map_err(|e| format!("bitbucket {e:#}")),
                (None, None) if self.token.is_some() || self.token_cmd.is_some() => {
                    self.token().map_err(|e| format!("{e:#}"))
                }
                (None, None) => Err(format!(
                    "no Bitbucket API token configured; run `{}` or set ATLX_BB_TOKEN or ATLX_BB_TOKEN_CMD",
                    self.bitbucket_hint()
                )),
            }
        });
        let token = res.clone().map_err(ApiError::config)?;
        Ok((
            Client::new(bitbucket_api()?, email, &token, self.timeout_ms).with_mode(self.mode()),
            self.bitbucket.workspace.clone(),
        ))
    }
}

/// The name of the repo-local defaults file.
pub const LOCAL_FILE: &str = ".atlx.toml";

/// Keys that hold secrets and are never accepted in `.atlx.toml`, which is
/// meant to be committed.
const SECRET_KEYS: &[&str] = &[
    "token",
    "token_cmd",
    "api_token",
    "password",
    "secret",
    "email",
    "site",
];

/// Non-secret defaults from a repo-local `.atlx.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalDefaults {
    /// Where the file was found (not part of the file).
    #[serde(skip)]
    pub path: Option<PathBuf>,
    /// Profile to use when `-p` and `ATLX_PROFILE` are not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default)]
    pub jira: LocalJira,
    #[serde(default)]
    pub confluence: LocalConfluence,
    #[serde(default)]
    pub bitbucket: LocalBitbucket,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalJira {
    /// Default project key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfluence {
    /// Default space key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalBitbucket {
    /// Default repository, `workspace/slug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

impl LocalDefaults {
    /// Parse a `.atlx.toml`. Secret keys anywhere in it are a `config`
    /// error, as are unknown keys.
    pub fn parse(text: &str) -> Result<Self> {
        let raw: toml::Value = toml::from_str(text)
            .map_err(|e| ApiError::config(format!("{LOCAL_FILE} is not valid TOML: {e}")))?;
        let mut found = Vec::new();
        find_secret_keys(&raw, "", &mut found);
        if !found.is_empty() {
            return Err(ApiError::config(format!(
                "{LOCAL_FILE} must not hold credentials or site settings (found {}); keep them in the user config with `atlx auth login`",
                found.join(", ")
            ))
            .into());
        }
        let d: Self = raw.try_into().map_err(|e| {
            ApiError::config(format!(
                "{LOCAL_FILE}: {e}; allowed keys are profile, jira.project, confluence.space, bitbucket.repo"
            ))
        })?;
        if let Some(r) = &d.bitbucket.repo
            && !is_repo_slug(r)
        {
            return Err(ApiError::config(format!(
                "{LOCAL_FILE}: bitbucket.repo `{r}` must be `workspace/slug`"
            ))
            .into());
        }
        Ok(d)
    }

    /// The nearest `.atlx.toml` in `start` or one of its parents, parsed;
    /// empty defaults when there is none.
    pub fn discover(start: &Path) -> Result<Self> {
        for dir in start.ancestors() {
            let p = dir.join(LOCAL_FILE);
            if p.is_file() {
                let text = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading {}", p.display()))?;
                let mut d =
                    Self::parse(&text).with_context(|| format!("loading {}", p.display()))?;
                d.path = Some(p);
                return Ok(d);
            }
        }
        Ok(Self::default())
    }
}

fn find_secret_keys(v: &toml::Value, prefix: &str, out: &mut Vec<String>) {
    if let toml::Value::Table(t) = v {
        for (k, val) in t {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            if SECRET_KEYS.contains(&k.to_ascii_lowercase().as_str()) {
                out.push(path.clone());
            }
            find_secret_keys(val, &path, out);
        }
    }
}

fn is_repo_slug(s: &str) -> bool {
    let mut parts = s.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// `workspace/slug` of the `origin` remote of the git repository that
/// contains `start`, when that remote is on bitbucket.org. Reads
/// `.git/config` directly (worktrees included); never runs git.
pub fn git_remote_repo(start: &Path) -> Option<String> {
    let config = git_config_path(start)?;
    let text = std::fs::read_to_string(config).ok()?;
    origin_url(&text).and_then(|u| bitbucket_repo_from_url(&u))
}

fn git_config_path(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot = dir.join(".git");
        if dot.is_dir() {
            return Some(dot.join("config"));
        }
        if dot.is_file() {
            // A worktree or submodule: `gitdir: <path>`.
            let text = std::fs::read_to_string(&dot).ok()?;
            let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let gitdir = dir.join(gitdir);
            let own = gitdir.join("config");
            if own.is_file() {
                return Some(own);
            }
            let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
            return Some(gitdir.join(common.trim()).join("config"));
        }
    }
    None
}

/// The `url` of `[remote "origin"]` in a git config file.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            let section: String = l.chars().filter(|c| !c.is_whitespace()).collect();
            in_origin = section == "[remote\"origin\"]";
            continue;
        }
        if in_origin
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == "url"
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// `workspace/slug` from a bitbucket.org clone URL (HTTPS, SSH or scp-like).
pub fn bitbucket_repo_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = if let Some((_, after)) = url.split_once("://") {
        let after = after.rsplit_once('@').map_or(after, |(_, h)| h);
        let (host, path) = after.split_once('/')?;
        let host = host.split(':').next()?;
        if !host.eq_ignore_ascii_case("bitbucket.org") {
            return None;
        }
        path.to_string()
    } else {
        // scp-like: git@bitbucket.org:ws/slug.git
        let (userhost, path) = url.split_once(':')?;
        let host = userhost.rsplit_once('@').map_or(userhost, |(_, h)| h);
        if !host.eq_ignore_ascii_case("bitbucket.org") {
            return None;
        }
        path.to_string()
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    is_repo_slug(rest).then(|| rest.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Error,
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileInspection {
    pub status: CheckStatus,
    pub path: String,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub detail: String,
}

/// Inspect the config file's existence and (on Unix) permissions.
pub fn inspect_file(path: &Path) -> FileInspection {
    let path_str = path.display().to_string();
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return FileInspection {
                status: CheckStatus::NotConfigured,
                path: path_str,
                exists: false,
                mode: None,
                detail: "config file does not exist (run `atlx auth login` or set ATLX_* env vars)"
                    .into(),
            };
        }
        Err(e) => {
            return FileInspection {
                status: CheckStatus::Error,
                path: path_str,
                exists: false,
                mode: None,
                detail: format!("failed to inspect config file: {e}"),
            };
        }
    };
    if !meta.is_file() {
        return FileInspection {
            status: CheckStatus::Error,
            path: path_str,
            exists: true,
            mode: None,
            detail: "config path exists but is not a regular file".into(),
        };
    }
    inspect_permissions(path, path_str, &meta)
}

#[cfg(unix)]
fn inspect_permissions(path: &Path, path_str: String, meta: &std::fs::Metadata) -> FileInspection {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    let mode_str = format!("{mode:04o}");
    let (status, detail) = if mode & 0o077 != 0 {
        (
            CheckStatus::Warn,
            format!(
                "permissions {mode_str} allow group/other access; run `chmod 600 {}`",
                path.display()
            ),
        )
    } else {
        (CheckStatus::Ok, format!("exists (mode {mode_str})"))
    };
    FileInspection {
        status,
        path: path_str,
        exists: true,
        mode: Some(mode_str),
        detail,
    }
}

#[cfg(not(unix))]
fn inspect_permissions(
    _path: &Path,
    path_str: String,
    _meta: &std::fs::Metadata,
) -> FileInspection {
    FileInspection {
        status: CheckStatus::Ok,
        path: path_str,
        exists: true,
        mode: None,
        detail: "exists".into(),
    }
}

#[cfg(unix)]
fn write_private(p: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    // `mode` only applies on creation; tighten an existing file too.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(p: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(p, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_site() {
        assert_eq!(
            normalize_site("https://acme.atlassian.net/"),
            "acme.atlassian.net"
        );
        assert_eq!(normalize_site(" acme.atlassian.net "), "acme.atlassian.net");
        assert_eq!(normalize_site("http://x.y//"), "x.y");
        assert_eq!(normalize_site("acme"), "acme.atlassian.net");
    }

    const LEGACY: &str = r#"
site = "legacy.atlassian.net"
email = "me@legacy.com"
token = "legacy-token"

[bitbucket]
workspace = "legacy-ws"
"#;

    const V2: &str = r#"
default_profile = "work"
site = "legacy.atlassian.net"
email = "me@legacy.com"
token = "legacy-token"

[bitbucket]
workspace = "legacy-ws"

[profiles.work]
site = "work.atlassian.net"
email = "me@work.com"
token_cmd = "echo work-token"

[profiles.work.bitbucket]
workspace = "work-ws"
token_cmd = "echo bb-token"

[profiles.sandbox]
site = "sandbox"
email = "me@sandbox.com"
token = "sandbox-token"
"#;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn legacy_config_loads_as_default_profile_and_roundtrips() {
        let f = ConfigFile::parse(LEGACY).unwrap();
        assert!(f.profiles.is_empty());
        assert_eq!(f.default_name(), DEFAULT_PROFILE);
        assert_eq!(f.base.site.as_deref(), Some("legacy.atlassian.net"));
        assert_eq!(f.base.bitbucket.workspace.as_deref(), Some("legacy-ws"));
        let text = f.to_toml().unwrap();
        assert!(!text.contains("profiles"), "{text}");
        assert!(!text.contains("default_profile"), "{text}");
        assert_eq!(ConfigFile::parse(&text).unwrap(), f);

        let c = Config::resolve(&f, None, &no_env).unwrap();
        assert_eq!(c.profile, DEFAULT_PROFILE);
        assert_eq!(c.token.as_deref(), Some("legacy-token"));
        assert_eq!(c.token_source(), "config");

        let empty = ConfigFile::parse("").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.names(), vec![DEFAULT_PROFILE.to_string()]);
    }

    #[test]
    fn v2_config_roundtrips() {
        let f = ConfigFile::parse(V2).unwrap();
        assert_eq!(f.default_name(), "work");
        assert_eq!(f.names(), ["default", "sandbox", "work"]);
        let work = f.get("work").unwrap();
        assert_eq!(work.token_cmd.as_deref(), Some("echo work-token"));
        assert_eq!(work.bitbucket.workspace.as_deref(), Some("work-ws"));
        assert_eq!(work.token_source(), "token_cmd");
        assert_eq!(f.get("sandbox").unwrap().bitbucket_token_source(), "config");
        let text = f.to_toml().unwrap();
        assert_eq!(ConfigFile::parse(&text).unwrap(), f);
    }

    #[test]
    fn rejects_reserved_profile_table() {
        let err = ConfigFile::parse("[profiles.default]\nsite = \"x\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("reserved"));
    }

    #[test]
    fn profile_selection() {
        let f = ConfigFile::parse(V2).unwrap();
        // No selection and `default` both mean the saved default.
        assert_eq!(Config::resolve(&f, None, &no_env).unwrap().profile, "work");
        assert_eq!(
            Config::resolve(&f, Some("default"), &no_env)
                .unwrap()
                .profile,
            "work"
        );
        // ATLX_PROFILE selects; the flag beats it.
        let env = env_map(&[("ATLX_PROFILE", "sandbox")]);
        let c = Config::resolve(&f, None, &env).unwrap();
        assert_eq!(c.profile, "sandbox");
        assert_eq!(c.site.as_deref(), Some("sandbox"));
        assert_eq!(
            Config::resolve(&f, Some("work"), &env).unwrap().profile,
            "work"
        );
        // Unknown profile is an error naming the known ones.
        let err = Config::resolve(&f, Some("nope"), &no_env).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nope") && msg.contains("sandbox"), "{msg}");
        // Without default_profile, `default` is the top-level profile.
        let legacy = ConfigFile::parse(LEGACY).unwrap();
        let c = Config::resolve(&legacy, Some("default"), &no_env).unwrap();
        assert_eq!(c.site.as_deref(), Some("legacy.atlassian.net"));
    }

    type Case<'a> = (
        &'a [(&'a str, &'a str)],
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        &'a str,
    );

    #[test]
    fn precedence_matrix() {
        let f = ConfigFile::parse(V2).unwrap();
        // (env, expected site, token, token_cmd, token source)
        let cases: &[Case] = &[
            (&[], "sandbox", Some("sandbox-token"), None, "config"),
            (
                &[("ATLX_SITE", "envsite"), ("ATLX_TOKEN", "env-token")],
                "envsite",
                Some("env-token"),
                None,
                "env",
            ),
            (
                &[("ATLX_TOKEN_CMD", "echo env-cmd")],
                "sandbox",
                None,
                Some("echo env-cmd"),
                "token_cmd",
            ),
            (
                &[("ATLX_TOKEN", "env-token"), ("ATLX_TOKEN_CMD", "echo x")],
                "sandbox",
                Some("env-token"),
                None,
                "env",
            ),
        ];
        for (pairs, site, token, cmd, source) in cases {
            let mut all = vec![("ATLX_PROFILE", "sandbox")];
            all.extend_from_slice(pairs);
            let c = Config::resolve(&f, None, &env_map(&all)).unwrap();
            assert_eq!(c.site.as_deref(), Some(*site), "{pairs:?}");
            assert_eq!(c.token.as_deref(), *token, "{pairs:?}");
            assert_eq!(c.token_cmd.as_deref(), *cmd, "{pairs:?}");
            assert_eq!(c.token_source(), *source, "{pairs:?}");
        }

        // Bitbucket: the profile's token_cmd, beaten by an env literal.
        let c = Config::resolve(&f, Some("work"), &no_env).unwrap();
        assert_eq!(c.bitbucket_token_source(), "token_cmd");
        assert_eq!(c.bitbucket.workspace.as_deref(), Some("work-ws"));
        let env = env_map(&[("ATLX_BB_TOKEN", "bb"), ("ATLX_BB_WORKSPACE", "w2")]);
        let c = Config::resolve(&f, Some("work"), &env).unwrap();
        assert_eq!(c.bitbucket_token_source(), "env");
        assert_eq!(c.bitbucket.token_cmd, None);
        assert_eq!(c.bitbucket.workspace.as_deref(), Some("w2"));
        // Sandbox has no Bitbucket token, so it shares the Atlassian one.
        let c = Config::resolve(&f, Some("sandbox"), &no_env).unwrap();
        assert_eq!(c.bitbucket_token_source(), "config");
    }

    #[test]
    fn token_cmd_is_evaluated_lazily_and_once() {
        let f = ConfigFile::parse(V2).unwrap();
        let c = Config::resolve(&f, Some("work"), &no_env).unwrap();
        assert!(c.cache.token.get().is_none());
        c.jira().unwrap();
        assert_eq!(c.cache.token.get().unwrap().as_deref(), Ok("work-token"));
        c.confluence().unwrap();
        c.bitbucket().unwrap();
        assert_eq!(
            c.cache.bitbucket_token.get().unwrap().as_deref(),
            Ok("bb-token")
        );
    }

    #[test]
    fn token_cmd_success_trims_output() {
        let t = run_token_cmd("echo secret-value", TOKEN_CMD_TIMEOUT).unwrap();
        assert_eq!(t, "secret-value");
    }

    #[test]
    fn token_cmd_failure_does_not_echo_stdout() {
        let err = run_token_cmd("echo leaked-secret && exit 3", TOKEN_CMD_TIMEOUT).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("token_cmd failed"), "{msg}");
        assert!(!msg.contains("leaked-secret"), "{msg}");

        let err = run_token_cmd("exit 0", TOKEN_CMD_TIMEOUT).unwrap_err();
        assert!(format!("{err:#}").contains("printed nothing"));

        let f = ConfigFile::parse("email = \"e\"\nsite = \"s\"\ntoken_cmd = \"exit 7\"\n").unwrap();
        let c = Config::resolve(&f, None, &no_env).unwrap();
        let err = c.jira().err().unwrap();
        assert!(format!("{err:#}").contains("token_cmd failed"));
        assert_eq!(
            ApiError::from_anyhow(&err).kind,
            crate::client::ErrorKind::Config
        );
    }

    #[cfg(unix)]
    #[test]
    fn token_cmd_times_out() {
        let start = Instant::now();
        let err = run_token_cmd("sleep 5", Duration::from_millis(200)).unwrap_err();
        assert!(format!("{err:#}").contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn rename_remove_and_default() {
        let mut f = ConfigFile::parse(LEGACY).unwrap();
        // Renaming the implicit default moves it under [profiles] and keeps it default.
        f.rename(DEFAULT_PROFILE, "legacy").unwrap();
        assert!(f.base.is_empty());
        assert_eq!(f.default_name(), "legacy");
        assert_eq!(f.names(), ["legacy"]);
        assert!(f.rename("legacy", "default").is_err());
        f.get_mut("other").site = Some("o".into());
        assert!(f.rename("legacy", "other").is_err());
        f.rename("legacy", "old").unwrap();
        assert_eq!(f.default_name(), "old");
        assert!(f.remove("old"));
        assert_eq!(f.default_name(), DEFAULT_PROFILE);
        assert!(!f.remove("old"));
        assert_eq!(f.suggest_name(Some("other.atlassian.net")), "other-2");
        assert_eq!(f.suggest_name(Some("https://acme.atlassian.net/")), "acme");
        assert_eq!(f.suggest_name(None), "profile");
        assert!(validate_name("a b").is_err());
        assert!(validate_name("work-2").is_ok());
    }

    #[test]
    fn read_only_comes_from_profile_or_env() {
        let f = ConfigFile::parse("site = \"s\"\nemail = \"e\"\ntoken = \"t\"\nread_only = true\n")
            .unwrap();
        let c = Config::resolve(&f, None, &no_env).unwrap();
        assert!(c.read_only);
        assert_eq!(c.mode(), Mode::ReadOnly);
        assert_eq!(c.jira().unwrap().mode(), Mode::ReadOnly);
        let f = ConfigFile::parse(LEGACY).unwrap();
        let c = Config::resolve(&f, None, &env_map(&[("ATLX_READ_ONLY", "1")])).unwrap();
        assert_eq!(c.mode(), Mode::ReadOnly);
        let c = Config::resolve(&f, None, &env_map(&[("ATLX_READ_ONLY", "0")])).unwrap();
        assert_eq!(c.mode(), Mode::Live);
        let mut c = Config::resolve(&f, None, &no_env).unwrap();
        c.dry_run = true;
        assert_eq!(c.bitbucket().unwrap().0.mode(), Mode::DryRun);
        c.read_only = true;
        assert_eq!(c.mode(), Mode::ReadOnly, "read-only wins over dry-run");
    }

    #[test]
    fn local_defaults_parse_and_reject_secrets() {
        let d = LocalDefaults::parse(
            "profile = \"work\"\n[jira]\nproject = \"PROJ\"\n[confluence]\nspace = \"ENG\"\n[bitbucket]\nrepo = \"acme/api\"\n",
        )
        .unwrap();
        assert_eq!(d.profile.as_deref(), Some("work"));
        assert_eq!(d.jira.project.as_deref(), Some("PROJ"));
        assert_eq!(d.confluence.space.as_deref(), Some("ENG"));
        assert_eq!(d.bitbucket.repo.as_deref(), Some("acme/api"));
        for bad in [
            "token = \"x\"",
            "[bitbucket]\ntoken_cmd = \"pass x\"",
            "[jira]\nproject = \"P\"\napi_token = \"x\"",
            "site = \"acme\"",
        ] {
            let err = LocalDefaults::parse(bad).unwrap_err();
            let api = ApiError::find(&err).unwrap();
            assert_eq!(api.kind, crate::client::ErrorKind::Config, "{bad}");
            assert!(api.message.contains("must not hold credentials"), "{bad}");
        }
        assert!(LocalDefaults::parse("colour = \"red\"").is_err());
        assert!(LocalDefaults::parse("[bitbucket]\nrepo = \"no-slash\"").is_err());
    }

    #[test]
    fn local_defaults_are_found_walking_up_and_pick_the_profile() {
        let root = std::env::temp_dir().join(format!("atlx-local-{}", std::process::id()));
        let deep = root.join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(LocalDefaults::discover(&deep).unwrap().path, None);
        std::fs::write(root.join("a").join(LOCAL_FILE), "profile = \"sandbox\"\n").unwrap();
        let d = LocalDefaults::discover(&deep).unwrap();
        assert_eq!(d.path, Some(root.join("a").join(LOCAL_FILE)));
        let f = ConfigFile::parse(V2).unwrap();
        let c = Config::resolve_with(&f, None, &no_env, d.clone()).unwrap();
        assert_eq!(c.profile, "sandbox");
        // ATLX_PROFILE and -p still win.
        let env = env_map(&[("ATLX_PROFILE", "work")]);
        assert_eq!(
            Config::resolve_with(&f, None, &env, d.clone())
                .unwrap()
                .profile,
            "work"
        );
        assert_eq!(
            Config::resolve_with(&f, Some("default"), &no_env, d)
                .unwrap()
                .profile,
            "work"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bitbucket_repo_defaults_from_the_git_remote() {
        for (url, want) in [
            ("git@bitbucket.org:acme/api.git", Some("acme/api")),
            ("https://me@bitbucket.org/acme/api.git", Some("acme/api")),
            ("https://bitbucket.org/acme/api", Some("acme/api")),
            (
                "ssh://git@bitbucket.org/acme/web-app.git",
                Some("acme/web-app"),
            ),
            ("git@github.com:acme/api.git", None),
            ("https://bitbucket.org.evil.example/acme/api.git", None),
            ("https://bitbucket.org/acme", None),
        ] {
            assert_eq!(bitbucket_repo_from_url(url).as_deref(), want, "{url}");
        }
        let root = std::env::temp_dir().join(format!("atlx-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sub = root.join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = git@bitbucket.org:other/x.git\n[remote \"origin\"]\n\turl = git@bitbucket.org:acme/api.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        )
        .unwrap();
        assert_eq!(git_remote_repo(&sub).as_deref(), Some("acme/api"));
        // A worktree: `.git` is a file pointing at the common directory.
        let wt = root.join("wt");
        let wt_git = root.join(".git/worktrees/wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        assert_eq!(git_remote_repo(&wt).as_deref(), Some("acme/api"));
        // `.atlx.toml` wins over the remote.
        let d = LocalDefaults::parse("[bitbucket]\nrepo = \"acme/other\"\n").unwrap();
        let f = ConfigFile::parse(LEGACY).unwrap();
        let c = Config::resolve_with(&f, None, &no_env, d).unwrap();
        assert_eq!(c.default_repo.as_deref(), Some("acme/other"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saves_to_temp_path() {
        let dir = std::env::temp_dir().join(format!("atlx-cfg-save-{}", std::process::id()));
        let p = dir.join("config.toml");
        let f = ConfigFile::parse(V2).unwrap();
        f.save_to(&p).unwrap();
        assert_eq!(ConfigFile::load_from(&p).unwrap(), f);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspects_missing_config_file() {
        let p =
            std::env::temp_dir().join(format!("atlx-doctor-missing-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let res = inspect_file(&p);
        assert_eq!(res.status, CheckStatus::NotConfigured);
        assert!(!res.exists);
        assert!(res.mode.is_none());
    }

    #[test]
    fn inspects_existing_config_file_permissions() {
        let p = std::env::temp_dir().join(format!("atlx-doctor-perms-{}.toml", std::process::id()));
        std::fs::write(&p, "site = \"acme.atlassian.net\"\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            let ok_res = inspect_file(&p);
            assert_eq!(ok_res.status, CheckStatus::Ok);
            assert!(ok_res.exists);
            assert_eq!(ok_res.mode.as_deref(), Some("0600"));

            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            let warn_res = inspect_file(&p);
            assert_eq!(warn_res.status, CheckStatus::Warn);
            assert!(warn_res.exists);
            assert_eq!(warn_res.mode.as_deref(), Some("0644"));
        }

        #[cfg(not(unix))]
        {
            let res = inspect_file(&p);
            assert_eq!(res.status, CheckStatus::Ok);
            assert!(res.exists);
            assert!(res.mode.is_none());
        }

        let _ = std::fs::remove_file(&p);
    }
}

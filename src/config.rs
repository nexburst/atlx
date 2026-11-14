//! Configuration: a TOML file (`~/.config/atlx/config.toml`) overridden by
//! `ATLX_*` environment variables.

use crate::client::Client;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const BITBUCKET_API: &str = "https://api.bitbucket.org/2.0";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Atlassian Cloud site, e.g. `acme.atlassian.net`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Atlassian account email used for basic auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Atlassian API token (https://id.atlassian.com/manage-profile/security/api-tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Bitbucket::is_empty")]
    pub bitbucket: Bitbucket,
    /// Per-request timeout (`--timeout-ms`); not persisted.
    #[serde(skip)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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
}

impl Bitbucket {
    fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.email.is_none() && self.token.is_none()
    }
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

pub fn normalize_site(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.trim_end_matches('/');
    // twg accepts a bare site prefix: `acme` -> `acme.atlassian.net`.
    if s.contains('.') {
        s.to_string()
    } else {
        format!("{s}.atlassian.net")
    }
}

impl Config {
    /// The config file only, without environment overrides.
    pub fn load_file() -> Result<Self> {
        let p = path()?;
        if !p.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    /// The config file with `ATLX_*` environment variables applied on top.
    pub fn load() -> Result<Self> {
        let mut c = Self::load_file()?;
        c.apply_env();
        Ok(c)
    }

    fn apply_env(&mut self) {
        let set = |slot: &mut Option<String>, key: &str| {
            if let Some(v) = env(key) {
                *slot = Some(v);
            }
        };
        set(&mut self.site, "ATLX_SITE");
        set(&mut self.email, "ATLX_EMAIL");
        set(&mut self.token, "ATLX_TOKEN");
        set(&mut self.bitbucket.workspace, "ATLX_BB_WORKSPACE");
        set(&mut self.bitbucket.email, "ATLX_BB_EMAIL");
        set(&mut self.bitbucket.token, "ATLX_BB_TOKEN");
    }

    pub fn save(&self) -> Result<PathBuf> {
        let p = path()?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        write_private(&p, &text).with_context(|| format!("writing {}", p.display()))?;
        Ok(p)
    }

    fn site(&self) -> Result<String> {
        self.site.as_deref().map(normalize_site).ok_or_else(|| {
            anyhow!("no Atlassian site configured; run `atlx login` or set ATLX_SITE")
        })
    }

    fn creds(&self) -> Result<(&str, &str)> {
        let email = self
            .email
            .as_deref()
            .ok_or_else(|| anyhow!("no email configured; run `atlx login` or set ATLX_EMAIL"))?;
        let token = self.token.as_deref().ok_or_else(|| {
            anyhow!("no API token configured; run `atlx login` or set ATLX_TOKEN")
        })?;
        Ok((email, token))
    }

    pub fn jira(&self) -> Result<Client> {
        let (email, token) = self.creds()?;
        Ok(Client::new(
            format!("https://{}", self.site()?),
            email,
            token,
            self.timeout_ms,
        ))
    }

    pub fn confluence(&self) -> Result<Client> {
        let (email, token) = self.creds()?;
        Ok(Client::new(
            format!("https://{}/wiki", self.site()?),
            email,
            token,
            self.timeout_ms,
        ))
    }

    pub fn bitbucket_configured(&self) -> bool {
        self.bitbucket.workspace.is_some()
    }

    /// Bitbucket client and default workspace.
    pub fn bitbucket(&self) -> Result<(Client, Option<String>)> {
        let email = self
            .bitbucket
            .email
            .as_deref()
            .or(self.email.as_deref())
            .ok_or_else(|| {
                anyhow!(
                    "no Bitbucket email configured; run `atlx setup bitbucket` or set ATLX_BB_EMAIL"
                )
            })?;
        let token = self.bitbucket.token.as_deref().or(self.token.as_deref()).ok_or_else(|| {
            anyhow!("no Bitbucket API token configured; run `atlx setup bitbucket` or set ATLX_BB_TOKEN")
        })?;
        Ok((
            Client::new(BITBUCKET_API.into(), email, token, self.timeout_ms),
            self.bitbucket.workspace.clone(),
        ))
    }
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
                detail: "config file does not exist (run `atlx login` or set ATLX_* env vars)"
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

    #[test]
    fn roundtrips_toml() {
        let c = Config {
            site: Some("a.atlassian.net".into()),
            email: Some("me@example.com".into()),
            token: Some("t".into()),
            bitbucket: Bitbucket {
                workspace: Some("ws".into()),
                ..Default::default()
            },
            timeout_ms: None,
        };
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.bitbucket.workspace.as_deref(), Some("ws"));
        assert_eq!(back.site.as_deref(), Some("a.atlassian.net"));
        let empty: Config = toml::from_str("").unwrap();
        assert!(empty.site.is_none());
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

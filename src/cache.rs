//! A small on-disk cache for slow-changing lookup tables, never content.
//!
//! Only the tables in [`Table`] can be stored: Jira's field id/name map,
//! issue types and priorities, Confluence space key to id, and the
//! Bitbucket current user. Issues, pages, comments, pull requests, diffs and
//! logs have no `Table` and so can never be written here.
//!
//! Files live at `$XDG_CACHE_HOME/atlx/<site-hash>/<table>.json` (falling
//! back to `~/.cache`), are written with mode 0600 and expire after
//! [`TTL`]. `--no-cache` or `ATLX_NO_CACHE=1` turns the cache off: every
//! lookup fetches and nothing is written.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a cached table is used before it is fetched again.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The lookup tables the cache may hold. Nothing else is ever written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    /// Jira field id to name (`customfield_10016` is `Story Points`).
    JiraFields,
    JiraIssueTypes,
    JiraPriorities,
    /// Confluence space key to id.
    ConfluenceSpaces,
    /// The authenticated Bitbucket user's UUID.
    BitbucketUser,
}

impl Table {
    pub const ALL: [Table; 5] = [
        Table::JiraFields,
        Table::JiraIssueTypes,
        Table::JiraPriorities,
        Table::ConfluenceSpaces,
        Table::BitbucketUser,
    ];

    /// The file stem the table is stored under.
    pub fn name(self) -> &'static str {
        match self {
            Table::JiraFields => "jira-fields",
            Table::JiraIssueTypes => "jira-issue-types",
            Table::JiraPriorities => "jira-priorities",
            Table::ConfluenceSpaces => "confluence-spaces",
            Table::BitbucketUser => "bitbucket-user",
        }
    }
}

static DISABLED: AtomicBool = AtomicBool::new(false);

/// Turn the process-wide cache off (`--no-cache`).
pub fn disable() {
    DISABLED.store(true, Ordering::SeqCst);
}

/// Whether the cache is on: not disabled by `--no-cache` or `ATLX_NO_CACHE`.
pub fn enabled() -> bool {
    if DISABLED.load(Ordering::SeqCst) {
        return false;
    }
    !std::env::var("ATLX_NO_CACHE")
        .is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false" | "no" | "off"))
}

/// The cache root: `$XDG_CACHE_HOME/atlx`, else `~/.cache/atlx`.
pub fn root() -> Option<PathBuf> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let base = var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| var("LOCALAPPDATA").map(PathBuf::from))?;
    Some(base.join("atlx"))
}

/// One cache rooted at a directory.
#[derive(Debug, Clone)]
pub struct Cache {
    /// `None` when the cache is off.
    root: Option<PathBuf>,
    ttl: Duration,
}

impl Cache {
    /// The process cache: [`root`], unless the cache is off.
    pub fn global() -> Self {
        Self {
            root: if enabled() { root() } else { None },
            ttl: TTL,
        }
    }

    /// A cache rooted at `dir` (tests).
    pub fn at(dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            root: Some(dir.into()),
            ttl,
        }
    }

    /// A cache that never reads or writes.
    pub fn off() -> Self {
        Self {
            root: None,
            ttl: TTL,
        }
    }

    /// Where `table` for `scope` (a site base URL) is stored.
    pub fn path(&self, scope: &str, table: Table) -> Option<PathBuf> {
        Some(
            self.root
                .as_ref()?
                .join(scope_dir(scope))
                .join(format!("{}.json", table.name())),
        )
    }

    /// The cached table if it is younger than the TTL, else `fetch()`'s
    /// result, which is stored. A failed write is ignored: the cache is an
    /// optimisation.
    pub fn get_or_fetch(
        &self,
        scope: &str,
        table: Table,
        fetch: impl FnOnce() -> Result<Value>,
    ) -> Result<Value> {
        if let Some(v) = self.read(scope, table) {
            return Ok(v);
        }
        let v = fetch()?;
        self.write(scope, table, &v);
        Ok(v)
    }

    /// Look something up in `table`: from the cache when `find` succeeds
    /// there, else after one fresh fetch (so a field created since the table
    /// was cached is found). `None` when it is in neither.
    pub fn lookup<T>(
        &self,
        scope: &str,
        table: Table,
        fetch: impl Fn() -> Result<Value>,
        find: impl Fn(&Value) -> Option<T>,
    ) -> Result<Option<T>> {
        if let Some(v) = self.read(scope, table)
            && let Some(hit) = find(&v)
        {
            return Ok(Some(hit));
        }
        let v = fetch()?;
        self.write(scope, table, &v);
        Ok(find(&v))
    }

    fn read(&self, scope: &str, table: Table) -> Option<Value> {
        let p = self.path(scope, table)?;
        let text = std::fs::read_to_string(&p).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let at = v["fetchedAt"].as_u64()?;
        let age = now_secs().saturating_sub(at);
        (age < self.ttl.as_secs()).then(|| v["data"].clone())
    }

    fn write(&self, scope: &str, table: Table, data: &Value) {
        let Some(p) = self.path(scope, table) else {
            return;
        };
        let doc = json!({"fetchedAt": now_secs(), "data": data});
        let _ = write_private(&p, &doc.to_string());
    }

    /// Remove every cached table. Returns the number of files removed.
    pub fn clear(&self) -> Result<usize> {
        let Some(root) = &self.root else {
            return Ok(0);
        };
        if !root.exists() {
            return Ok(0);
        }
        let mut n = 0;
        for site in
            std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
        {
            let site = site?.path();
            if site.is_dir() {
                for f in std::fs::read_dir(&site)? {
                    let f = f?.path();
                    if f.extension().is_some_and(|e| e == "json") {
                        std::fs::remove_file(&f)?;
                        n += 1;
                    }
                }
                let _ = std::fs::remove_dir(&site);
            }
        }
        Ok(n)
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A stable directory name for a site: 16 hex digits of its FNV-1a hash.
fn scope_dir(scope: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in scope.trim_end_matches('/').to_ascii_lowercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(unix)]
fn write_private(p: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if let Some(dir) = p.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(text.as_bytes())?;
    drop(f);
    std::fs::rename(&tmp, p)
}

#[cfg(not(unix))]
fn write_private(p: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("atlx-cache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn caches_within_the_ttl_and_refetches_after() {
        let d = dir("ttl");
        let c = Cache::at(&d, TTL);
        let calls = Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            Ok(json!({"customfield_1": "Story Points"}))
        };
        let site = "https://acme.atlassian.net";
        assert_eq!(
            c.get_or_fetch(site, Table::JiraFields, fetch).unwrap()["customfield_1"],
            "Story Points"
        );
        c.get_or_fetch(site, Table::JiraFields, fetch).unwrap();
        assert_eq!(calls.get(), 1, "second read comes from the cache");

        // An expired entry is fetched again.
        let p = c.path(site, Table::JiraFields).unwrap();
        std::fs::write(&p, json!({"fetchedAt": 1, "data": {}}).to_string()).unwrap();
        c.get_or_fetch(site, Table::JiraFields, fetch).unwrap();
        assert_eq!(calls.get(), 2);

        // Another site has its own directory.
        c.get_or_fetch("https://other.atlassian.net", Table::JiraFields, fetch)
            .unwrap();
        assert_eq!(calls.get(), 3);
        assert_eq!(c.clear().unwrap(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_miss_triggers_one_refetch() {
        let d = dir("miss");
        let c = Cache::at(&d, TTL);
        let calls = Cell::new(0);
        let site = "https://acme.atlassian.net";
        let fetch = || {
            calls.set(calls.get() + 1);
            Ok(if calls.get() == 1 {
                json!({"a": 1})
            } else {
                json!({"a": 1, "b": 2})
            })
        };
        let find = |k: &'static str| move |v: &Value| v.get(k).and_then(Value::as_u64);
        assert_eq!(
            c.lookup(site, Table::JiraFields, fetch, find("a")).unwrap(),
            Some(1)
        );
        assert_eq!(
            c.lookup(site, Table::JiraFields, fetch, find("a")).unwrap(),
            Some(1)
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(
            c.lookup(site, Table::JiraFields, fetch, find("b")).unwrap(),
            Some(2)
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(
            c.lookup(site, Table::JiraFields, fetch, find("z")).unwrap(),
            None
        );
        assert_eq!(calls.get(), 3, "exactly one refetch per miss");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn off_never_writes() {
        let calls = Cell::new(0);
        let c = Cache::off();
        for _ in 0..2 {
            c.get_or_fetch("s", Table::BitbucketUser, || {
                calls.set(calls.get() + 1);
                Ok(json!("u"))
            })
            .unwrap();
        }
        assert_eq!(calls.get(), 2);
        assert!(c.path("s", Table::BitbucketUser).is_none());
        assert_eq!(c.clear().unwrap(), 0);
    }

    #[test]
    fn only_listed_tables_are_written_with_private_modes() {
        let d = dir("files");
        let c = Cache::at(&d, TTL);
        for t in Table::ALL {
            c.get_or_fetch("https://acme.atlassian.net", t, || Ok(json!({})))
                .unwrap();
        }
        let site_dirs: Vec<_> = std::fs::read_dir(&d).unwrap().collect();
        assert_eq!(site_dirs.len(), 1);
        let site = site_dirs[0].as_ref().unwrap().path();
        let mut names: Vec<String> = std::fs::read_dir(&site)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut want: Vec<String> = Table::ALL
            .iter()
            .map(|t| format!("{}.json", t.name()))
            .collect();
        want.sort();
        assert_eq!(names, want);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for n in &names {
                let mode = std::fs::metadata(site.join(n))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{n}");
            }
            let mode = std::fs::metadata(&site).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

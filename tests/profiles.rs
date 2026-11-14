//! Named profiles and credential helpers, driven through the built binary
//! with an isolated `ATLX_CONFIG` in a temp directory.

mod common;

use base64::Engine;
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("atlx-profiles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn config(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    fn read(&self) -> String {
        std::fs::read_to_string(self.config()).unwrap_or_default()
    }

    fn write(&self, text: &str) {
        std::fs::write(self.config(), text).unwrap();
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
        cmd.args(args).env("ATLX_CONFIG", self.config());
        for k in [
            "ATLX_SITE",
            "ATLX_EMAIL",
            "ATLX_TOKEN",
            "ATLX_TOKEN_CMD",
            "ATLX_PROFILE",
            "ATLX_OUTPUT",
            "ATLX_BB_WORKSPACE",
            "ATLX_BB_EMAIL",
            "ATLX_BB_TOKEN",
            "ATLX_BB_TOKEN_CMD",
        ] {
            cmd.env_remove(k);
        }
        cmd.stdin(Stdio::null());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("run atlx")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ok_json(o: &Output) -> Value {
    assert!(
        o.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice(&o.stdout).expect("JSON on stdout")
}

fn path_arg(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn login_creates_profiles_and_auth_profile_commands_manage_them() {
    let sb = Sandbox::new("manage");
    // A named profile with a credential helper; it becomes the default.
    let out = ok_json(&sb.run(&[
        "login",
        "--new-profile",
        "work",
        "-s",
        "work",
        "-u",
        "me@work.com",
        "--token-cmd",
        "echo work-token",
        "--no-verify",
        "-o",
        "json",
    ]));
    assert_eq!(out["profile"], "work");
    assert_eq!(out["token_source"], "token_cmd");
    assert_eq!(out["default"], true);
    let text = sb.read();
    assert!(text.contains("default_profile = \"work\""), "{text}");
    assert!(text.contains("[profiles.work]"), "{text}");
    assert!(text.contains("token_cmd = \"echo work-token\""), "{text}");

    // A second profile from a literal token on stdin, name derived from the site.
    let mut child = sb
        .cmd(&[
            "login",
            "--new-profile",
            "-s",
            "https://sandbox.atlassian.net/",
            "-u",
            "me@sandbox.com",
            "--token",
            "-",
            "--no-verify",
            "-o",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"sandbox-secret\n")
        .unwrap();
    let out = ok_json(&child.wait_with_output().unwrap());
    assert_eq!(out["profile"], "sandbox");
    assert_eq!(out["default"], false);

    // Creating an existing profile again is refused.
    let dup = sb.run(&[
        "login",
        "--new-profile",
        "work",
        "-s",
        "x",
        "-u",
        "y",
        "--token-cmd",
        "echo z",
        "--no-verify",
    ]);
    assert!(!dup.status.success());

    // `list` shows sources, never tokens, and never runs token_cmd.
    let o = sb.run(&["auth", "profile", "list", "-o", "json"]);
    let raw = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(!raw.contains("sandbox-secret"), "{raw}");
    let rows = ok_json(&o);
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["name"], "sandbox");
    assert_eq!(rows[0]["token_source"], "config");
    assert_eq!(rows[1]["name"], "work");
    assert_eq!(rows[1]["token_source"], "token_cmd");
    assert_eq!(rows[1]["default"], true);
    assert_eq!(rows[1]["active"], true);

    // ATLX_PROFILE and -p change the active profile.
    let rows = ok_json(
        &sb.cmd(&["auth", "profile", "list", "-o", "json"])
            .env("ATLX_PROFILE", "sandbox")
            .output()
            .unwrap(),
    );
    assert_eq!(rows[0]["active"], true);
    let rows = ok_json(&sb.run(&["-p", "default", "auth", "profile", "list", "-o", "json"]));
    assert_eq!(rows[1]["active"], true, "`default` means the saved default");

    // Per-profile setup.
    ok_json(&sb.run(&[
        "setup",
        "default-site",
        "sb2",
        "-p",
        "sandbox",
        "-o",
        "json",
    ]));
    assert!(sb.read().contains("site = \"sb2.atlassian.net\""));
    let out = ok_json(&sb.run(&[
        "setup",
        "bitbucket",
        "--workspace",
        "acme",
        "--token-cmd",
        "echo bb",
        "--no-verify",
        "-p",
        "sandbox",
        "-o",
        "json",
    ]));
    assert_eq!(out["profile"], "sandbox");
    assert_eq!(out["token_source"], "token_cmd");

    // default / rename / logout.
    ok_json(&sb.run(&["auth", "profile", "default", "sandbox", "-o", "json"]));
    assert!(sb.read().contains("default_profile = \"sandbox\""));
    let out = ok_json(&sb.run(&["auth", "profile", "rename", "sandbox", "sb", "-o", "json"]));
    assert_eq!(out["default"], true);
    assert!(sb.read().contains("default_profile = \"sb\""));
    assert!(
        !sb.run(&["auth", "profile", "default", "nope"])
            .status
            .success()
    );

    let out = ok_json(&sb.run(&["logout", "-p", "work", "-o", "json"]));
    assert_eq!(out["removed"], true);
    let text = sb.read();
    assert!(!text.contains("[profiles.work]"), "{text}");
    assert!(text.contains("[profiles.sb]"), "{text}");
    ok_json(&sb.run(&["logout", "-o", "json"]));
    assert!(!sb.config().exists(), "empty config file is removed");
}

#[test]
fn legacy_config_migrates_into_a_named_profile() {
    let sb = Sandbox::new("migrate");
    sb.write("site = \"acme.atlassian.net\"\nemail = \"me@acme.com\"\ntoken = \"t\"\n");
    let rows = ok_json(&sb.run(&["auth", "profile", "list", "-o", "json"]));
    assert_eq!(rows[0]["name"], "default");
    assert_eq!(rows[0]["site"], "acme.atlassian.net");

    assert!(!sb.run(&["auth", "profile", "migrate"]).status.success());
    let out = ok_json(&sb.run(&["auth", "profile", "migrate", "--yes", "-o", "json"]));
    assert_eq!(out["profile"], "acme");
    let text = sb.read();
    assert!(text.contains("default_profile = \"acme\""), "{text}");
    assert!(text.contains("[profiles.acme]"), "{text}");
    assert!(!text.starts_with("site"), "{text}");
}

#[test]
fn unknown_profile_is_a_config_error() {
    let sb = Sandbox::new("unknown");
    sb.write("site = \"acme.atlassian.net\"\n");
    let o = sb.run(&["-p", "nope", "jira", "workitem", "get", "X-1"]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("nope"), "{err}");
}

#[test]
fn token_cmd_runs_only_when_a_client_is_built() {
    let sb = Sandbox::new("lazy");
    let marker = sb.dir.join("ran.txt");
    sb.write(&format!(
        "site = \"acme\"\nemail = \"me@acme.com\"\ntoken_cmd = {}\n",
        toml_str(&format!("echo x > {}", path_arg(&marker)))
    ));
    ok_json(&sb.run(&["auth", "profile", "list", "-o", "json"]));
    assert!(sb.run(&["jira", "--help"]).status.success());
    assert!(!marker.exists(), "token_cmd ran without a client");
}

#[test]
fn whoami_uses_the_profile_token_cmd_and_reports_its_source() {
    let srv = MockServer::with_handler(|_| {
        MockResponse::json(
            200,
            json!({"displayName": "Me", "emailAddress": "me@work.com", "accountId": "a1"}),
        )
    });
    let sb = Sandbox::new("whoami");
    sb.write(
        "site = \"legacy\"\nemail = \"legacy@x.com\"\ntoken = \"legacy\"\n\n\
         [profiles.work]\nsite = \"work\"\nemail = \"me@work.com\"\ntoken_cmd = \"echo cmd-token\"\n",
    );
    let out = ok_json(
        &sb.cmd(&["-p", "work", "whoami", "-o", "json"])
            .env("ATLX_SITE", srv.base_url())
            .output()
            .unwrap(),
    );
    assert_eq!(out["profile"], "work");
    assert_eq!(out["token_source"], "token_cmd");
    let raw = serde_json::to_string(&out).unwrap();
    assert!(!raw.contains("cmd-token"), "{raw}");
    let auth = srv.requests()[0]
        .header("authorization")
        .unwrap()
        .to_string();
    let expected = base64::engine::general_purpose::STANDARD.encode("me@work.com:cmd-token");
    assert_eq!(auth, format!("Basic {expected}"));

    // A failing helper gives a clear error without echoing its stdout.
    sb.write(
        "site = \"acme\"\nemail = \"me@acme.com\"\ntoken_cmd = \"echo leaked-value && exit 4\"\n",
    );
    let o = sb
        .cmd(&["whoami", "-o", "json"])
        .env("ATLX_SITE", srv.base_url())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(all.contains("token_cmd failed"), "{all}");
    assert!(!all.contains("leaked-value"), "{all}");
}

/// A TOML basic string (escapes backslashes in Windows paths).
fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

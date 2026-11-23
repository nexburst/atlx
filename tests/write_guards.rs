//! Client-level write safety: `--dry-run` sends no mutating request and
//! prints it; read-only mode refuses every mutating request (product
//! commands and `atlx api` alike); `post_read` requests still go out; a
//! destructive command refuses without `--yes`.

mod common;

use atlx::client::{ApiError, DryRun, ErrorKind, Mode};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn ok_server() -> MockServer {
    MockServer::with_handler(|_| MockResponse::json(200, json!({"id": "1"})))
}

#[test]
fn dry_run_client_stops_at_the_first_write_and_lets_reads_through() {
    let srv = ok_server();
    let c = srv.client().with_mode(Mode::DryRun);
    c.get("/rest/api/3/issue/P-1", &[]).unwrap();
    c.post_read(
        "/rest/api/3/issue/bulkfetch",
        &json!({"issueIdsOrKeys": ["P-1"]}),
    )
    .unwrap();
    let err = c
        .request(
            "PUT",
            "/rest/api/3/issue/P-1",
            &[("notifyUsers", "false")],
            Some(&json!({"fields": {"summary": "S"}})),
        )
        .unwrap_err();
    let d = DryRun::find(&err).expect("a dry-run stop");
    assert_eq!(d.method, "PUT");
    assert!(
        d.url.ends_with("/rest/api/3/issue/P-1?notifyUsers=false"),
        "{}",
        d.url
    );
    assert_eq!(d.body, Some(json!({"fields": {"summary": "S"}})));
    let methods: Vec<String> = srv.requests().into_iter().map(|r| r.method).collect();
    assert_eq!(methods, ["GET", "POST"], "only the read and the post_read");
}

#[test]
fn read_only_client_refuses_every_write() {
    let srv = ok_server();
    let c = srv.client().with_mode(Mode::ReadOnly);
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let err = c.request(method, "/x/1", &[], None).unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.kind, ErrorKind::Refused, "{method}");
        assert_eq!(api.exit_code(), 8);
    }
    let parts = [atlx::client::Part::text("a", "b")];
    assert!(c.post_multipart("/up", &parts, &[]).is_err());
    c.post_read("/search", &json!({})).unwrap();
    c.get("/x/1", &[]).unwrap();
    assert_eq!(srv.requests().len(), 2);
}

struct Run {
    dir: PathBuf,
}

impl Run {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("atlx-guards-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn cmd(&self, site: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atlx"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("ATLX_") {
                cmd.env_remove(&key);
            }
        }
        cmd.args(args)
            .env("ATLX_CONFIG", self.dir.join("config.toml"))
            .env("ATLX_SITE", site)
            .env("ATLX_EMAIL", "me@example.com")
            .env("ATLX_TOKEN", "fake")
            .current_dir(&self.dir)
            .stdin(Stdio::null());
        cmd
    }

    fn run(&self, site: &str, args: &[&str]) -> Output {
        self.cmd(site, args).output().unwrap()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn json_out(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

const COMMENT: &[&str] = &[
    "jira",
    "issue",
    "comment",
    "create",
    "--issue",
    "P-1",
    "--body",
    "hi",
    "--body-format",
    "plain",
    "-o",
    "json",
];

#[test]
fn dry_run_on_a_product_write_prints_the_request_and_sends_no_write() {
    let srv = ok_server();
    let r = Run::new("dry");
    let args: Vec<&str> = COMMENT.iter().copied().chain(["--dry-run"]).collect();
    let o = r.run(&srv.base_url(), &args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    let v = json_out(&o);
    assert_eq!(v["ok"], true);
    assert_eq!(v["dryRun"], true);
    let req = &v["data"]["request"];
    assert_eq!(req["method"], "POST");
    assert!(
        req["url"]
            .as_str()
            .unwrap()
            .contains("/rest/api/3/issue/P-1/comment")
    );
    assert!(req["body"].is_object());
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn read_only_refuses_product_writes_and_the_api_escape_hatch() {
    let srv = ok_server();
    let r = Run::new("ro");
    let site = srv.base_url();
    let args: Vec<&str> = COMMENT.iter().copied().chain(["--read-only"]).collect();
    let o = r.run(&site, &args);
    assert_eq!(o.status.code(), Some(8));
    assert_eq!(json_out(&o)["error"]["code"], "refused");

    let o = r
        .cmd(
            &site,
            &[
                "api",
                "jira",
                "/rest/api/3/issue",
                "-X",
                "POST",
                "-d",
                "{}",
                "-o",
                "json",
            ],
        )
        .env("ATLX_READ_ONLY", "1")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(8));
    let v = json_out(&o);
    assert_eq!(v["error"]["code"], "refused");
    assert!(v["error"]["hint"].as_str().unwrap().contains("read-only"));

    // A profile can be read-only too.
    std::fs::write(
        r.dir.join("config.toml"),
        "email = \"me@example.com\"\ntoken = \"t\"\nread_only = true\n",
    )
    .unwrap();
    let o = r.run(&site, &["api", "jira", "/x", "-X", "DELETE", "-o", "json"]);
    assert_eq!(o.status.code(), Some(8));
    // Reads still work.
    let o = r.run(&site, &["api", "jira", "/rest/api/3/myself", "-o", "json"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(srv.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn api_dry_run_shows_the_write() {
    let srv = ok_server();
    let r = Run::new("apidry");
    let o = r.run(
        &srv.base_url(),
        &[
            "api",
            "jira",
            "/rest/api/3/issue/P-1",
            "-X",
            "DELETE",
            "--dry-run",
            "-o",
            "json",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(json_out(&o)["data"]["request"]["method"], "DELETE");
    assert!(srv.requests().is_empty());
}

#[test]
fn destructive_commands_refuse_without_yes_and_preview_with_dry_run() {
    let r = Run::new("yes");
    let cfg = r.dir.join("config.toml");
    let legacy = "site = \"acme.atlassian.net\"\nemail = \"me@acme.com\"\ntoken = \"t\"\n";
    std::fs::write(&cfg, legacy).unwrap();
    let o = r.run(
        "http://127.0.0.1:9",
        &["auth", "profile", "migrate", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(8));
    let v = json_out(&o);
    assert_eq!(v["error"]["code"], "refused");
    assert!(v["error"]["hint"].as_str().unwrap().contains("--dry-run"));
    let o = r.run(
        "http://127.0.0.1:9",
        &["auth", "profile", "migrate", "--dry-run", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(json_out(&o)["dryRun"], true);
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        legacy,
        "nothing written"
    );
    let o = r.run(
        "http://127.0.0.1:9",
        &["auth", "profile", "migrate", "--yes", "-o", "json"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(
        std::fs::read_to_string(&cfg)
            .unwrap()
            .contains("[profiles.acme]")
    );
}

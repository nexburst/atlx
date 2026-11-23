//! Parse, grammar and registry tests for `atlx bitbucket`, and dispatch
//! tests against a small local HTTP server.

use super::{BbCmd, Ctx, META, dispatch};
use crate::cli::describe::leaves;
use crate::cli::shape::{OutputArgs, Shaping};
use crate::cli::tests::shell_words;
use crate::cli::{Cli, Cmd, Out, leaf_matches};
use crate::client::{Client, DryRun, Mode};
use crate::output::Format;
use crate::schema::Effect;
use clap::{CommandFactory, FromArgMatches, Parser};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

fn parses(args: &str) {
    if let Err(e) = Cli::try_parse_from(shell_words(args)) {
        panic!("{args}: {e}");
    }
}

fn fails(args: &str) {
    assert!(
        Cli::try_parse_from(shell_words(args)).is_err(),
        "expected a parse error for: {args}"
    );
}

#[test]
fn parses_the_command_tree() {
    for ok in [
        "atlx bb repo list --workspace acme --name api -q 'is_private = true' --limit 5",
        "atlx bb repo list --all --sort name",
        "atlx bb repo get",
        "atlx bb repo get acme/api acme/web --strict --concurrency 2",
        "atlx bb repo create acme/new --public --project CORE",
        "atlx bb repo file get src/a.rs --ref main -r acme/api --content-offset 10",
        "atlx bb repo file list --path src --depth 2 --cursor t:50:2",
        "atlx bb repo file download a.png --to /tmp/a.png --overwrite",
        "atlx bb repo file search needle ext:rs --workspace acme",
        "atlx bb repo branch list --author ann --sort -name",
        "atlx bb repo branch get main dev",
        "atlx bb repo branch create x --from main",
        "atlx bb repo branch delete x --yes",
        "atlx bb repo branch delete x -y",
        "atlx bb repo commit list --include a --include b --exclude main",
        "atlx bb repo commit get abc1234",
        "atlx bb repo commit create -m msg --branch dev --file a.md",
        "atlx bb repo commit create -m msg --default-branch --file-local x --file-remote a --delete b",
        "atlx bb repo contributor list --range v1..main --since 30d",
        "atlx bb repo report list --commit abc",
        "atlx bb repo report create lint --commit abc --report {}",
        "atlx bb repo default-reviewer list --effective",
        "atlx bb repo default-reviewer add me --yes",
        "atlx bb workspace get",
        "atlx bb workspace member list --name ann --permissions",
        "atlx bb workspace member get me ann",
        "atlx bb pr list --state MERGED --merged-since 7d --reviewer me",
        "atlx bb pr list --mine --role reviewer --waiting --repo-limit 5",
        "atlx bb pr list --commit abc123",
        "atlx bb pr search login --in title --state ALL",
        "atlx bb pr get 1 2 https://bitbucket.org/acme/api/pull-requests/3",
        "atlx bb pr get 1 --body-offset 100",
        "atlx bb pr create --title x --source b --reviewer 'Cy Lee',ann --default-reviewers --body-file pr.md",
        "atlx bb pr update 4 --title x --clear-body --ready --keep-source-branch",
        "atlx bb pr merge 4 --strategy squash --if-head abc1234 --timeout 5 --yes",
        "atlx bb pr merge 4 --no-wait --keep-source-branch -y",
        "atlx bb pr decline 4 --yes",
        "atlx bb pr approve 4",
        "atlx bb pr unapprove 4",
        "atlx bb pr request-changes 4 --withdraw",
        "atlx bb pr diff 4 --path a.rs --grep needle --diff-offset 40000",
        "atlx bb pr diff 4 --patch",
        "atlx bb pr diffstat 4 --conflicts",
        "atlx bb pr commits 4 --limit 5",
        "atlx bb pr activity 4 --type comment",
        "atlx bb pr checks 4 --no-conflicts",
        "atlx bb pr comment list --pr 42",
        "atlx bb pr comment get 9 10 --pr 42",
        "atlx bb pr comment create --pr 42 --body hi --reply-to 7 --pending -r acme/api",
        "atlx bb pr comment create --pr 42 --body hi --path src/a.rs --line 3 --from-line 2",
        "atlx bb pr comment create --pr 42 --body-file n.md --path a --start-line 1 --end-line 4 --start-from-line 1 --end-from-line 3",
        "atlx bb pr comment update 9 --pr 42 --body new",
        "atlx bb pr comment delete 9 --pr 42 --yes",
        "atlx bb pr comment resolve 9 --pr 42",
        "atlx bb pr comment reopen 9 --pr 42",
        "atlx bb pr task list --pr 42",
        "atlx bb pr task get 6 --pr 42",
        "atlx bb pr task create --pr 42 --body t --comment 9",
        "atlx bb pr task update 6 --pr 42 --body t",
        "atlx bb pr task delete 6 --pr 42 -y",
        "atlx bb pr task resolve 6 --pr 42",
        "atlx bb pr task reopen 6 --pr 42",
        "atlx bb pr reviewer list --pr 42",
        "atlx bb pr reviewer add ann me --pr 42",
        "atlx bb pr reviewer remove ann --pr 42",
        "atlx bb pipeline list --result FAILED --branch main --pattern nightly",
        "atlx bb pipeline get 101 102",
        "atlx bb pipeline run --branch main --variable A=b,c --secured-variable TOKEN",
        "atlx bb pipeline run --commit abc --custom nightly",
        "atlx bb pipeline run --rerun 101",
        "atlx bb pipeline log",
        "atlx bb pipeline log --failed --branch main",
        "atlx bb pipeline log 101 --step 2 --grep boom -i -C 2 --max-matches 5",
        "atlx bb pipeline log 101 --follow --interval 5 --timeout 60",
        "atlx bb pipeline log 101 --log-offset 200",
        "atlx bb pipeline step list --pipeline 101",
        "atlx bb pipeline step get 2 --pipeline 101 --test-cases --case-limit 0",
        "atlx bb pipeline config get",
        "atlx bb pipeline config update --enabled false",
        "atlx bb deployment list --environment production --status FAILED --oldest-first",
        "atlx bb deployment get u1 u2 --environment e",
        "atlx bb deployment commits u1 --limit 10",
        "atlx bb environment list",
        "atlx bb environment variable list --environment production",
        "atlx bitbucket pr list",
    ] {
        parses(ok);
    }
}

#[test]
fn rejects_bad_invocations() {
    for bad in [
        // Old paths and flags are gone.
        "atlx bb pull-requests query",
        "atlx bb prs query",
        "atlx bb inbox",
        "atlx bb branch query",
        "atlx bb repo query",
        "atlx bb search code x",
        "atlx bb pr list --page 2",
        "atlx bb repo get --workspace acme",
        "atlx bb pr comment create --pull-request 42 --text hi",
        "atlx bb pr get 1 --full-x",
        // Flags that need each other or exclude each other.
        "atlx bb repo commit create -m msg --file a.md",
        "atlx bb repo commit create -m msg --branch dev --default-branch",
        "atlx bb pr update 4 --draft --ready",
        "atlx bb pr update 4 --body x --clear-body",
        "atlx bb pr merge 4 --no-wait --timeout 5",
        "atlx bb pr merge 4 --close-source-branch --keep-source-branch",
        "atlx bb pr merge 4 --strategy invalid",
        "atlx bb pr list --role reviewer",
        "atlx bb pr list --commit abc --state OPEN",
        "atlx bb pr diff 4 --patch --grep x",
        "atlx bb pr comment create --pr 42 --body hi --line 3",
        "atlx bb pr comment create --pr 42 --body hi --path a --line 3 --start-line 1 --end-line 4",
        "atlx bb pr comment create --pr 42 --body hi --path a --start-line 3",
        "atlx bb pr comment create --pr 42 --body hi --path a --line 0",
        "atlx bb pr comment create --pr 42 --body hi --reply-to abc",
        "atlx bb pr comment create --pr 42 --body a --body-file b",
        "atlx bb pr comment update 0 --pr 42 --body x",
        "atlx bb pr comment get --pr 42",
        "atlx bb pr comment list",
        "atlx bb pr get",
        "atlx bb pr get 1 --concurrency 0",
        "atlx bb pr get 1 --concurrency 21",
        "atlx bb pr diffstat 42 --limit 0",
        "atlx bb pr list --limit 5 --all",
        "atlx bb pipeline run --branch main --tag v1",
        "atlx bb pipeline run --rerun 1 --branch main",
        "atlx bb pipeline log 1 --branch main",
        "atlx bb pipeline log 1 --follow --grep x",
        "atlx bb pipeline log 1 --follow --failed",
        "atlx bb pipeline log 1 -i",
        "atlx bb pipeline config update",
        "atlx bb environment variable list",
        "atlx bb repo get acme/api --repo acme/web --repo acme/x",
    ] {
        fails(bad);
    }
}

/// Every verb a Bitbucket command may end with.
const VERBS: &[&str] = &[
    "list",
    "search",
    "get",
    "create",
    "update",
    "delete",
    "merge",
    "decline",
    "approve",
    "unapprove",
    "request-changes",
    "diff",
    "diffstat",
    "commits",
    "activity",
    "checks",
    "resolve",
    "reopen",
    "add",
    "remove",
    "run",
    "log",
    "download",
];

const FORBIDDEN_FLAGS: &[&str] = &[
    "--id",
    "--pull-request",
    "--max-results",
    "--start-at",
    "--first",
    "--after",
    "--page",
    "--text",
    "--batch-concurrency",
    "--format",
];

#[test]
fn every_path_follows_the_grammar() {
    let all: Vec<_> = leaves()
        .into_iter()
        .filter(|l| l.path.starts_with("bitbucket "))
        .collect();
    assert!(all.len() > 60, "{}", all.len());
    for l in &all {
        let words: Vec<&str> = l.path.split(' ').collect();
        assert!(
            (3..=4).contains(&words.len()),
            "{}: at most one sub-resource",
            l.path
        );
        let verb = words.last().unwrap();
        assert!(VERBS.contains(verb), "{}: `{verb}` is not a verb", l.path);
        for a in l.cmd.get_arguments() {
            if let Some(long) = a.get_long() {
                let flag = format!("--{long}");
                assert!(
                    !FORBIDDEN_FLAGS.contains(&flag.as_str()),
                    "{}: {flag}",
                    l.path
                );
                // Local flags must not shadow the global output controls.
                assert!(
                    ![
                        "--fields",
                        "--full",
                        "--raw",
                        "--max-chars",
                        "--output-file",
                        "--dry-run"
                    ]
                    .contains(&flag.as_str())
                        || a.is_global_set(),
                    "{}: local {flag}",
                    l.path
                );
            }
        }
    }
}

#[test]
fn every_command_is_registered_with_a_record_and_examples() {
    let paths: Vec<String> = leaves()
        .into_iter()
        .filter(|l| l.path.starts_with("bitbucket "))
        .map(|l| l.path)
        .collect();
    for p in &paths {
        let m = META
            .iter()
            .find(|m| m.path == p)
            .unwrap_or_else(|| panic!("{p} has no CommandMeta"));
        assert!(m.record.is_some(), "{p} has no record");
        assert!(!m.examples.is_empty(), "{p} has no example");
    }
    assert_eq!(META.len(), paths.len(), "META lists a command that is gone");
}

#[test]
fn destructive_commands_take_yes_and_others_do_not() {
    for l in leaves()
        .into_iter()
        .filter(|l| l.path.starts_with("bitbucket "))
    {
        let m = META.iter().find(|m| m.path == l.path).unwrap();
        let has_yes = l.cmd.get_arguments().any(|a| a.get_id() == "yes");
        assert_eq!(
            has_yes,
            m.effect == Effect::Destructive,
            "{}: --yes goes exactly with destructive commands",
            l.path
        );
        if m.effect == Effect::Destructive {
            assert!(
                m.examples
                    .iter()
                    .any(|e| e.contains("--yes") || e.contains("--dry-run")),
                "{}: examples should show --yes",
                l.path
            );
        }
    }
    let destructive: Vec<&str> = META
        .iter()
        .filter(|m| m.effect == Effect::Destructive)
        .map(|m| m.path)
        .collect();
    for p in [
        "bitbucket pr merge",
        "bitbucket pr decline",
        "bitbucket repo branch delete",
    ] {
        assert!(destructive.contains(&p), "{p}");
    }
}

// --- a local HTTP server -----------------------------------------------------

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
}

type Handler = dyn Fn(&str, &str) -> (u16, String) + Send + Sync;

/// Answers every request with `handler(method, path_and_query)` until
/// dropped; records what it saw.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    fn new(handler: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let log = log.clone();
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 8192];
                    let end = loop {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        raw.extend_from_slice(&buf[..n]);
                        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&raw[..end]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    while raw.len() < end + len {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        raw.extend_from_slice(&buf[..n]);
                    }
                    let mut first = head.lines().next().unwrap_or_default().split(' ');
                    let method = first.next().unwrap_or_default().to_string();
                    let path = first.next().unwrap_or_default().to_string();
                    log.lock().unwrap().push(Seen {
                        method: method.clone(),
                        path: path.clone(),
                    });
                    let (status, body) = handler(&method, &path);
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                });
            }
        });
        Mock { base, seen }
    }

    fn client(&self, mode: Mode) -> Client {
        Client::new(self.base.clone(), "u@example.com", "t", Some(5000)).with_mode(mode)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// Parse a command line into its Bitbucket command and its leaf matches.
fn parse(line: &str) -> (BbCmd, clap::ArgMatches, String) {
    let argv = shell_words(line);
    let m = Cli::command().try_get_matches_from(&argv).unwrap();
    let path = crate::cli::command_path(&m);
    let leaf = leaf_matches(&m).clone();
    let cli = Cli::from_arg_matches(&m).unwrap();
    match cli.cmd {
        Cmd::Bitbucket { cmd } => (cmd, leaf, path),
        _ => panic!("not a bitbucket command: {line}"),
    }
}

fn run(c: &Client, line: &str) -> anyhow::Result<Out> {
    let ctx = Ctx {
        c,
        default_ws: Some("acme"),
        default_repo: Some("acme/api"),
    };
    dispatch(&ctx, parse(line).0)
}

/// A REST object rich enough for every command's reads to go through.
fn anything() -> Value {
    json!({
        "values": [], "uuid": "{11111111-2222-4333-8444-555555555555}",
        "title": "T", "participants": [],
        "reviewers": [{"uuid": "{b}", "nickname": "bob", "display_name": "Bob"}],
        "mainbranch": {"name": "main"},
        "target": {"hash": "abc1234def", "commit": {"hash": "abc1234def"}},
        "source": {"commit": {"hash": "3f2a9c1b7e44"}},
        "state": {"name": "COMPLETED", "result": {"name": "FAILED"}},
        "build_number": 1, "type": "commit_file",
        "user": {"uuid": "{11111111-2222-4333-8444-555555555555}", "display_name": "Ann Lee"},
    })
}

#[test]
fn every_write_stops_at_the_first_request_in_dry_run() {
    let srv = Mock::new(|method, path| {
        let mut v = anything();
        // Name lookups find exactly one Ann Lee and one bob.
        if path.contains("/members") {
            v["values"] = json!([
                {"user": {"uuid": "{a}", "display_name": "Ann Lee"}},
                {"user": {"uuid": "{b}", "nickname": "bob", "display_name": "Bob"}},
            ]);
        }
        let _ = method;
        (200, v.to_string())
    });
    let c = srv.client(Mode::DryRun);
    for m in META.iter().filter(|m| m.effect != Effect::Read) {
        for ex in m.examples.iter().filter(|e| !e.contains("--dry-run")) {
            let before = srv.seen().len();
            let out = run(&c, ex);
            let writes: Vec<Seen> = srv.seen()[before..]
                .iter()
                .filter(|s| s.method != "GET")
                .cloned()
                .collect();
            assert!(writes.is_empty(), "{ex} sent {writes:?}");
            match out {
                Err(e) if DryRun::find(&e).is_some() => {}
                // The example names a local file this test doesn't create.
                Err(e) if format!("{e:#}").starts_with("reading ") => {}
                Err(e) => panic!("{ex}: stopped before its write: {e:#}"),
                Ok(_) => panic!("{ex}: finished without a write"),
            }
        }
    }
}

#[test]
fn read_only_refuses_every_write() {
    let srv = Mock::new(|_, _| (200, anything().to_string()));
    let c = srv.client(Mode::ReadOnly);
    let Err(err) = run(&c, "atlx bb pr approve 4") else {
        panic!("approve went through in read-only mode")
    };
    let api = crate::client::ApiError::find(&err).unwrap();
    assert_eq!(api.exit_code(), 8);
    assert!(srv.seen().iter().all(|s| s.method == "GET"));
}

fn shaped(line: &str, out: Out) -> Value {
    let (_, leaf, path) = parse(line);
    let s = Shaping::new(OutputArgs::default(), Format::Json, &path, Some(&leaf)).unwrap();
    match out {
        Out::Data(v) => s.finish(v, 0).unwrap(),
        _ => panic!("{line}: not data"),
    }
}

#[test]
fn a_large_diff_is_cut_at_the_budget_and_continues() {
    let mut diff =
        String::from("diff --git a/big b/big\n--- a/big\n+++ b/big\n@@ -0,0 +1,40000 @@\n");
    let mut n = 0;
    while diff.len() < 1_000_000 {
        diff.push_str(&format!("+line {n} of a generated file\n"));
        n += 1;
    }
    let body = diff.clone();
    let srv = Mock::new(move |_, _| (200, body.clone()));
    let c = srv.client(Mode::Live);

    let line = "atlx bb pr diff 7";
    let env = shaped(line, run(&c, line).unwrap());
    let shown = env["data"]["diff"].as_str().unwrap();
    assert!(shown.chars().count() <= 40_000, "{}", shown.len());
    assert!(shown.ends_with('\n'), "cut at a line end");
    let t = &env["truncated"][0];
    assert_eq!(t["field"], "data.diff");
    assert_eq!(t["total"], diff.chars().count());
    let more = t["more"].as_str().unwrap();
    assert_eq!(more, format!("--diff-offset {}", shown.chars().count()));

    let line2 = format!("atlx bb pr diff 7 {more}");
    let env2 = shaped(&line2, run(&c, &line2).unwrap());
    let next = env2["data"]["diff"].as_str().unwrap();
    assert!(
        diff[shown.len()..].starts_with(next),
        "continues where the cut was"
    );
    assert_eq!(env2["truncated"][0]["field"], "data.diff");
}

#[test]
fn logs_keep_their_last_lines() {
    let log: String = (1..=500).map(|i| format!("line {i}\n")).collect();
    let srv = Mock::new(move |_, path| {
        if path.ends_with("/log") {
            return (200, log.clone());
        }
        if path.contains("/steps") {
            let s = json!({"values": [{"uuid": "{s}", "name": "Build", "state": {"name": "COMPLETED", "result": {"name": "FAILED"}}}]});
            return (200, s.to_string());
        }
        (200, json!({"uuid": "{p}", "build_number": 9}).to_string())
    });
    let c = srv.client(Mode::Live);
    let line = "atlx bb pipeline log 9";
    let env = shaped(line, run(&c, line).unwrap());
    let rec = &env["data"][0];
    assert_eq!(rec["step"], "Build");
    assert_eq!(rec["result"], "FAILED");
    let shown = rec["log"].as_str().unwrap();
    assert!(shown.starts_with("line 301\n") && shown.ends_with("line 500\n"));
    assert_eq!(env["truncated"][0]["more"], "--log-offset 200");
    let line = "atlx bb pipeline log 9 --log-offset 200";
    let env = shaped(line, run(&c, line).unwrap());
    let shown = env["data"][0]["log"].as_str().unwrap();
    assert!(shown.starts_with("line 101\n") && shown.ends_with("line 300\n"));
}

#[test]
fn lists_print_records_and_a_cursor() {
    let srv = Mock::new(|_, path| {
        let host = "http://example.invalid";
        let v = if path.contains("page=2") {
            json!({"values": [{"id": 3, "title": "c", "description": "long"}]})
        } else {
            json!({
                "values": [{"id": 1, "title": "a", "description": "long", "links": {"html": {"href": "u1"}}},
                           {"id": 2, "title": "b"}],
                "next": format!("{host}/repositories/acme/api/pullrequests?pagelen=2&page=2"),
            })
        };
        (200, v.to_string())
    });
    let c = srv.client(Mode::Live);
    let line = "atlx bb pr list --limit 2";
    let env = shaped(line, run(&c, line).unwrap());
    assert_eq!(env["data"][0]["id"], 1);
    assert_eq!(env["data"][0]["url"], "u1");
    assert!(env["data"][0].get("body").is_none(), "lists carry no body");
    assert_eq!(env["next"], "t:2:2");
    let line = "atlx bb pr list --limit 2 --cursor t:2:2";
    let env = shaped(line, run(&c, line).unwrap());
    assert_eq!(env["data"][0]["id"], 3);
    assert_eq!(env["next"], Value::Null);
    let q = &srv.seen()[1].path;
    assert!(q.contains("pagelen=2") && q.contains("page=2"), "{q}");
}

#[test]
fn a_pr_url_names_its_own_repository() {
    let srv = Mock::new(|_, _| (200, json!({"id": 3, "title": "t"}).to_string()));
    let c = srv.client(Mode::Live);
    run(
        &c,
        "atlx bb pr get https://bitbucket.org/team/web/pull-requests/3",
    )
    .unwrap();
    assert_eq!(srv.seen()[0].path, "/repositories/team/web/pullrequests/3");
    run(&c, "atlx bb pr get 4").unwrap();
    assert_eq!(srv.seen()[1].path, "/repositories/acme/api/pullrequests/4");
}

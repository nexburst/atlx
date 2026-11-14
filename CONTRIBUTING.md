# Contributing to atlx

Thanks for helping. atlx aims to be a drop-in, open alternative to Atlassian's
`twg` CLI for Jira, Confluence and Bitbucket Cloud, built only on public REST
APIs. This guide covers how the code is laid out, how to add a command, and
the conventions every change follows. Coding agents should also read
[AGENTS.md](AGENTS.md).

## Build and test

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the last three on Linux, macOS and Windows. Live smoke tests against a
real site are `#[ignore]`d; see the README's Testing section.

## Module layout

| Path | Holds |
| --- | --- |
| `src/main.rs` | A few lines that call `atlx::cli::main()` |
| `src/lib.rs` | Module list and the shared `Product` enum |
| `src/client.rs`, `src/config.rs`, `src/output.rs`, `src/markup.rs`, `src/time.rs` | HTTP client, config and credentials, output formats, body-format conversion, date helpers |
| `src/<product>.rs` (`jira`, `confluence`, `bitbucket`, `search`) | API functions. They call REST endpoints and return a summarized `serde_json::Value` |
| `src/cli/mod.rs` | `Cli` with the global flags, the top-level `Cmd`, `Globals`, `Out`, dispatch and `main()` |
| `src/cli/<product>.rs` (`jira`, `confluence`, `bitbucket`) | That product's clap enums and its dispatch function, plus parse tests |
| `src/cli/auth.rs`, `doctor.rs`, `api.rs`, `search.rs`, `user.rs` | The non-product commands |
| `src/cli/util.rs` | Helpers shared by commands: `batch`, `text_arg`, `body_arg`, `require_yes` |
| `tests/common/mod.rs` | Offline mock HTTP server (`MockServer`, `MockResponse`, `RecordedRequest`) |
| `tests/<product>.rs`, `tests/client.rs`, `tests/search.rs` | Mock-HTTP tests for each product's API functions |
| `tests/cli_tree.rs` + `tests/snapshots/cli-tree.txt` | Snapshot of the full command tree |
| `tests/live_smoke.rs` | Opt-in live checks (`#[ignore]`) |

The split is deliberate: clap code never calls `ureq` directly, and API
functions never parse arguments. That keeps API functions testable against
the mock server and keeps product PRs out of each other's files.

## Adding a command

1. **Clap variant**: add it to the product's enum in `src/cli/<product>.rs`,
   using twg's command path and flag names (check `twg <cmd> --help`).
2. **API function**: add it to `src/<product>.rs`. It takes a `&Client` and
   typed arguments, and returns a summarized `Value`.
3. **Dispatch**: wire the variant to the function in the product's dispatch
   function in `src/cli/<product>.rs`.
4. **Mock-HTTP test** in `tests/<product>.rs`: assert the method, path, query
   and body sent, and the summarized output.
5. **Parse test**: add the twg-style invocation to the `tests` module in
   `src/cli/<product>.rs` (or to `parses_twg_style_invocations` in
   `src/cli/mod.rs`).
6. **Regenerate the CLI snapshot** and review the diff:
   `UPDATE_SNAPSHOTS=1 cargo test --test cli_tree`.
7. **README**: update the command table.
8. **`COMPAT.md`**: add or update the row for the command (once #15 lands).

## Conventions

- **twg naming.** If twg has the command or flag, use its exact path and
  spelling. atlx-only commands are labelled "(atlx extra)" in their help.
- **Output.** Summary keys are `snake_case`. Text on a terminal, JSON when
  piped, and always overridable with `-o text|json|jsonl`.
- **Errors** go to stderr with a non-zero exit code. Never print errors to
  stdout.
- **Destructive operations** (delete, merge, decline, ...) require `--yes`.
  Add `--dry-run` where it is feasible to show the request without sending it.
- **Dependencies.** Don't add a crate without a justification in the PR. HTTP
  is sync `ureq` 2; there is no async runtime.
- **Tests stay offline.** Anything that needs a real site is `#[ignore]`.
- **Public REST only**: Jira REST v3 and Agile 1.0, Confluence v2 (v1 where v2
  lacks an endpoint), Bitbucket 2.0. Features that need Teamwork Graph, the
  GraphQL gateway or Rovo are out of scope.

## Parallel work

- Run at most one PR at a time per `area:*` label. Issues in the same area
  edit the same `src/cli/<product>.rs` and `src/<product>.rs`.
- If you run several worktrees (or agents) at once, give each its own
  `CARGO_TARGET_DIR`. A shared target directory makes the builds clobber each
  other.

## twg text

Never copy twg's help text, descriptions or skill files into this repository;
they are "All rights reserved". Command paths and flag names are interface
facts and may be reused. Write every help string and doc in your own words.

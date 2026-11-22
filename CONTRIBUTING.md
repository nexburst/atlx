# Contributing to atlx

Thanks for helping. atlx is an open CLI for Jira, Confluence and Bitbucket
Cloud, built only on Atlassian's public REST APIs. This guide covers how the code is laid out, how to add a command, and
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
| `src/<product>.rs` (`jira`, `confluence`, `bitbucket`, `search`) | API functions. They call REST endpoints and return the `data` part of the output as a `serde_json::Value` |
| `src/cli/mod.rs` | `Cli` with the global flags, the top-level `Cmd`, `Globals`, `Out`, dispatch and `main()` |
| `src/cli/<product>.rs` (`jira`, `confluence`, `bitbucket`) | That product's clap enums and its dispatch function, plus parse tests |
| `src/cli/auth.rs`, `doctor.rs`, `api.rs`, `search.rs`, `user.rs`, `completion.rs` | The non-product commands (`completion.rs` also holds the hidden `man` generator) |
| `src/cli/util.rs` | Helpers shared by commands: `batch` (parallel, `--batch-concurrency`), `text_arg`, `body_arg`, `require_yes`, `deprecated` |
| `tests/common/mod.rs` | Offline mock HTTP server (`MockServer`, `MockResponse`, `RecordedRequest`) |
| `tests/<product>.rs`, `tests/client.rs`, `tests/search.rs` | Mock-HTTP tests for each product's API functions |
| `tests/cli_tree.rs` + `tests/snapshots/cli-tree.txt` | Snapshot of the full command tree |
| `tests/twg_shapes.rs` + `tests/fixtures/twg/` | Frozen output shapes, checked against mock REST responses |
| `tests/output_contract.rs` | Output shapes and exit codes for edge cases (empty pages, partial batches) |
| `tests/live_smoke.rs` | Opt-in live checks (`#[ignore]`) |
| `tests/twg_compat.rs` | Checks `compat/corpus.txt`, `COMPAT.md` and `compat/coverage.txt` against the parser |
| `compat/twg-<version>.txt` | Command paths, positionals and flag names from twg 1.4.4, kept by hand |

The split is deliberate: clap code never calls `ureq` directly, and API
functions never parse arguments. That keeps API functions testable against
the mock server and keeps product PRs out of each other's files.

## Adding a command

1. **Clap variant**: add it to the product's enum in `src/cli/<product>.rs`,
   using the command path and flag names from `compat/twg-1.4.4.txt` or the
   public commands catalog when the command is listed there.
2. **API function**: add it to `src/<product>.rs`. It takes a `&Client` and
   typed arguments, and returns the command's `data` as a `Value`.
3. **Dispatch**: wire the variant to the function in the product's dispatch
   function in `src/cli/<product>.rs`, wrapping Jira, Confluence and user
   results in `envelope("<operation>", "<resourceType>")` with a `request`
   echo of the inputs.
4. **Mock-HTTP test** in `tests/<product>.rs`: assert the method, path, query
   and body sent, and the output keys.
5. **Parse test**: add the invocation to the `tests` module in
   `src/cli/<product>.rs` (or to `parses_twg_style_invocations` in
   `src/cli/mod.rs`).
6. **Regenerate the CLI snapshot** and review the diff:
   `UPDATE_SNAPSHOTS=1 cargo test --test cli_tree`.
7. **README**: update the command table.
8. **`COMPAT.md` and the corpus**: set the command's row in `COMPAT.md`, flip
   the matching `gap #N` lines in `compat/corpus.txt` to `ok`, add a corpus
   line for any new invocation, and record the new coverage with
   `UPDATE_COVERAGE=1 cargo test --test twg_compat`. See "Command coverage"
   below.

## Conventions

- **Naming.** If the inventory or catalog has the command or flag, use its
  exact path and spelling. atlx-only commands are labelled "(atlx extra)" in their help.
- **Output.** Jira, Confluence, user and account commands return
  `output::envelope::envelope(<operation>, <resourceType>)` around data that
  keeps the REST payload's own key names, and Bitbucket commands return the
  REST objects unwrapped. Existing shapes are a contract: change them only on
  purpose, with a changelog entry. Text on a
  terminal, JSON when piped, and always overridable with `-o text|json|jsonl`.
- **Errors.** Return them as `anyhow::Error`. For a classified failure, return
  a `client::ApiError` (`ApiError::usage(..)` for bad arguments,
  `ApiError::config(..)` for missing setup; the client produces HTTP and network
  errors itself). `main` prints them as described in the README's "Errors and
  exit codes" section. Commands never print errors themselves.
- **Confirmation.** Follow the command family's existing policy, which the
  README documents: some families require `--yes`, others (Jira deletes,
  merge, decline) don't, and scripts rely on both. Where `--yes` isn't
  required it is accepted as a hidden no-op. Nothing prompts. Add `--dry-run` where it
  is feasible to show the request without sending it.
- **Renames.** When a command is renamed, the new name becomes the visible one and the old spelling stays as a hidden command or flag that calls
  `util::deprecated(old, new)`. The note prints only in text mode on a
  terminal, never in JSON or piped output.
- **Dependencies.** Don't add a crate without a justification in the PR. HTTP
  is sync `ureq` 2; there is no async runtime.
- **Tests stay offline.** Anything that needs a real site is `#[ignore]`.
- **Public REST only**: Jira REST v3 and Agile 1.0, Confluence v2 (v1 where v2
  lacks an endpoint), Bitbucket 2.0. Features that need Teamwork Graph, the
  GraphQL gateway or Rovo are out of scope.

## Independence from twg

atlx is built from Atlassian's public REST documentation and API specs, not
from twg. Keep it that way:

- Don't run twg to develop or test atlx, and don't record, commit or publish
  twg's output.
- Don't publish comparisons, benchmarks or other assessments of twg (in
  issues, PRs, docs or commit messages).
- Command paths and flag names may match twg's. Take them from
  `compat/twg-1.4.4.txt` or Atlassian's public
  [commands catalog](https://developer.atlassian.com/cloud/twg-cli/commands/commands-catalog/)
  and changelog. Behavior and output follow the REST APIs and atlx's own
  docs.
- Never copy twg's help text, descriptions, docs or skill files; they are
  "All rights reserved". Write every help string and doc in your own words.

## Command coverage

Three files track which command paths atlx has, and `tests/twg_compat.rs`
checks all three offline:

- **`COMPAT.md`** has one row per command path in the inventory. Statuses:
  ✅ supported, 🟡 partial, 🔁 accepted through an alias, ➕ atlx-only, 📋
  planned, ⛔ out of scope. The test checks each status against the CLI tree
  snapshot: ✅/🟡 paths must exist, 🔁 paths must exist only through an alias,
  📋/⛔ paths must not parse. 🟡 and 📋 rows must link an issue.
- **`compat/corpus.txt`** has invocations, each tagged `ok` (must
  parse) or `gap #<issue>` (must fail to parse). When your change makes a gap
  line parse, the test fails until you flip it to `ok`. Every flag on a
  line must exist for that command in the inventory. Write lines yourself
  with placeholders (`PROJ-1`, `12345`, `ENG`, `acme`, `api`); never copy
  examples from twg's help, docs or skills. Tag a gap with the issue that will
  close it; use `#13` for public-REST work that has no issue yet.
- **`compat/coverage.txt`** is the share of inventory commands atlx accepts. The
  test fails if it drops; `UPDATE_COVERAGE=1 cargo test --test twg_compat`
  records a rise.

To add command paths that appear later, edit `compat/twg-1.4.4.txt` by hand
from the public commands catalog or changelog. Don't regenerate it by running
twg.

## Parallel work

- Run at most one PR at a time per `area:*` label. Issues in the same area
  edit the same `src/cli/<product>.rs` and `src/<product>.rs`.
- If you run several worktrees (or agents) at once, give each its own
  `CARGO_TARGET_DIR`. A shared target directory makes the builds clobber each
  other.

## Cutting a release

Releases are built by `.github/workflows/release.yml` when a `vX.Y.Z` tag is
pushed. Only maintainers do this.

1. On a branch, bump `version` in `Cargo.toml` and run `cargo check` so
   `Cargo.lock` picks it up.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`,
   add a fresh empty `## [Unreleased]` above it, and update the compare links
   at the bottom. The workflow uses this section as the release notes and fails
   if it is missing.
3. Merge the PR, then tag the merge commit on `main` and push the tag:

   ```sh
   git checkout main && git pull
   git tag -a vX.Y.Z -m "atlx vX.Y.Z"
   git push origin vX.Y.Z
   ```

4. The workflow checks that the tag matches `Cargo.toml`, runs
   `cargo publish --dry-run`, builds all seven targets, uploads the archives
   and `SHA256SUMS` to a new GitHub release, then runs `cargo publish`. The
   publish step is skipped with a warning when the `CARGO_REGISTRY_TOKEN`
   repository secret is not set.
5. Check the install paths: `cargo binstall atlx` and `cargo install atlx`.

To rehearse without releasing, run the workflow by hand (Actions, Release, Run
workflow, or `gh workflow run release.yml --ref <branch>`). It builds and
uploads the same archives as workflow artifacts and skips the release and
publish jobs. Pull requests that edit the workflow run it the same way.

Archive names must keep matching `pkg-url` in `[package.metadata.binstall]`:
`atlx-vX.Y.Z-<target>.tgz`, or `.zip` on Windows, with the binary at the root.


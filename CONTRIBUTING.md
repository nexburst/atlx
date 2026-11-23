# Contributing to atlx

Thanks for helping. atlx is a command line for Jira, Jira Service Management,
Confluence and Bitbucket Cloud, built only on their public REST APIs and
designed first for coding agents. This guide covers the layout, how to add a
command, and the rules every change follows. Coding agents should also read
[AGENTS.md](AGENTS.md).

## Build and test

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the last three on Linux, macOS and Windows. The suite is offline: it
talks only to a mock HTTP server on localhost. To prove a change keeps it
that way, run it without a network:

```sh
unshare -rn sh -c 'ip link set lo up && cargo test --locked --offline --no-fail-fast'
```

Live smoke tests against a real site are `#[ignore]`d; see the README's
Contributing section.

## Layout

| Path | Holds |
| --- | --- |
| `src/main.rs`, `src/lib.rs` | Entry point and module list |
| `src/client.rs` | HTTP client: retries, `ApiError` with its codes, hints and exit codes, the dry-run and read-only modes, `post_read` |
| `src/config.rs` | Config file, profiles, credentials, `.atlx.toml`, the git-remote repository default |
| `src/output.rs`, `src/output/` | The envelope, JSON/JSONL/ids/text rendering, path selection |
| `src/schema/` | `Record`, `Field`, views, projection, truncation budgets, `CommandMeta` and the registry lookup |
| `src/paging.rs` | `--limit`/`--cursor` handling and cursor encoding |
| `src/cache.rs` | The metadata cache (lookup tables only, never content) |
| `src/markup/` | Markdown, ADF and storage conversion with loss reporting |
| `src/<product>.rs`, `src/<product>/` (`jira`, `jsm`, `confluence`, `bitbucket`) | API functions, which call REST endpoints and return REST payloads; `records.rs` defines the product's records |
| `src/cli/mod.rs` | `Cli` with the global flags, top-level dispatch, the core `CommandMeta` table and `META_TABLES` |
| `src/cli/<product>.rs`, `src/cli/<product>/` | Clap enums, dispatch, the product's `META` table and parse tests |
| `src/cli/describe.rs` | `atlx describe`, built from the clap tree and the registry |
| `src/cli/shape.rs` | Applies `--fields`, `--full`, `--raw`, `--max-chars` and the envelope to a command's result |
| `src/cli/util.rs` | Shared helpers: multi-ID `batch`, stdin IDs, body arguments |
| `src/cli/skill.rs`, `skills/atlx/` | The agent skill and `atlx skill` |
| `tests/common/mod.rs` | The offline mock HTTP server |
| `tests/<product>*.rs` | Mock-HTTP tests per product, including grammar and write-guard tests |
| `tests/describe.rs`, `tests/snapshots/describe/` | Snapshot of `atlx describe` per product |
| `tests/write_guards.rs`, `tests/confluence_write_guards.rs` | Dry-run, read-only, `--yes` and version guards |
| `tests/output.rs`, `tests/output_contract.rs`, `tests/errors.rs` | Envelope, output modes, error codes and exit codes |
| `tests/markup.rs`, `tests/fixtures/markup/` | Conversion goldens, round-trip and lossy fixtures |
| `tests/skill.rs` | Generates the skill's command references from `describe`; checks doc links |
| `tests/live_smoke.rs` | Opt-in live checks (`#[ignore]`) |

The split is deliberate: clap code never calls `ureq` directly, API functions
never parse arguments, and records turn REST payloads into output. That keeps
API functions testable against the mock server, keeps `--raw` honest, and
keeps product changes out of each other's files.

## The contract

`atlx describe` is the contract. Everything below exists so that it stays
small, predictable and true.

- **Grammar.** `atlx <product> <resource> [<sub-resource>] <verb>`, at most
  one sub-resource level. Nouns are singular and are the REST API's own
  (`issue`, `project`, `page`, `pr`). The verb is last: `list`, `search`,
  `get`, `create`, `update`, `delete`, or a REST action (`transition`,
  `merge`, `approve`, ...). The target is positional; parents are named
  flags with the parent's noun (`--issue`, `--page`, `--pr`, `--repo`,
  `--project`, `--space`). One flag name per concept everywhere: `--limit`,
  `--cursor`, `--body`, `--body-file`, `--body-format`, `--yes`,
  `--if-version`, `-q/--query`. No `--id`, no `--max-results`, no per-command
  paging flags. Each product has a test that checks this for every path.
- **Records.** Every command returns a registered record with a short
  default view (6 to 12 fields), a full view, flat fields with the shared
  names (`id`, `key`, `title`, `url`, `created`, `updated`, `author`,
  `body`), people as `{id, name}`, no avatar URLs or `self` links. Lists
  carry no bodies. Large text fields declare a truncation budget and the
  flag that continues them.
- **Envelope and errors.** Commands return data; `main` wraps it. Failures
  are `client::ApiError` values with the right `ErrorKind` (`usage`,
  `config`, `not_found`, `conflict`, `refused`, ...), so the code, hint and
  exit code follow. Commands never print errors themselves.
- **Writes.** Every non-GET request is a write unless it goes through
  `Client::post_read`, so dry-run and read-only mode cover new commands
  automatically and a forgotten annotation fails closed. A command is
  `destructive` when it deletes, purges, archives, merges, declines, restores
  an old version, changes permissions or sharing, or does something JSM
  customers see; the registry then requires `--yes`. Do not call a
  confirmation helper by hand, and never prompt.
- **Bodies** go through `src/markup`: Markdown by default, ADF as the model,
  storage only as checked pass-through, no HTML input. A write that would
  lose content is refused; a read that cannot show content says so with a
  `lossy_body` warning.

## Adding a command

1. **Clap variant** in `src/cli/<product>.rs` (or its submodule), following
   the grammar above.
2. **API function** in `src/<product>.rs` or `src/<product>/`. It takes a
   `&Client` and typed arguments and returns the REST payload as a `Value`.
   A POST that only reads uses `post_read`. List functions page through
   `paging`.
3. **Record** in `src/<product>/records.rs`: reuse one when the command
   returns the same thing as another command; otherwise add one with a
   description per field and the default view marked.
4. **`CommandMeta`** in the product's `META` table: path, `effect` (`read`,
   `write` or `destructive`), `record`, and one to three `examples`, each a
   full `atlx ...` line. A test parses every example.
5. **Dispatch** from the variant to the API function.
6. **Tests**, all offline:
   - a mock-HTTP test in `tests/<product>*.rs` that asserts the method, path,
     query and body sent and the record printed;
   - a parse test in the product's CLI tests;
   - write guards: the table-driven tests in `tests/jira.rs`,
     `src/cli/bitbucket/tests.rs` and `tests/confluence_write_guards.rs` walk
     the registry, so a destructive command is checked to refuse without
     `--yes` and send nothing, and a write is checked to stop under
     `--dry-run`. Extend them when your command needs extra arguments or
     mock responses to reach its first write.
7. **Snapshots**: regenerate and review the diffs.
   - `UPDATE_SNAPSHOTS=1 cargo test --test describe`
   - `UPDATE_SKILL=1 cargo test --test skill` (the skill's command references)
8. **Changelog**: a line under `## [Unreleased]`.

The README has no command table to update: its overview lists resources, and
the full list lives in `describe` and the generated skill references.

## Conventions

- **Dependencies.** Don't add a crate without a justification in the PR. HTTP
  is sync `ureq` 2; there is no async runtime.
- **Tests stay offline.** Anything that needs a real site is `#[ignore]`.
- **Public REST only**: Jira REST v3 and Agile 1.0, the Service Desk API,
  Confluence v2 (v1 where v2 lacks an endpoint), Bitbucket 2.0. Features that
  need Teamwork Graph, the GraphQL gateway or Rovo are out of scope.
- **Help text** is for people and shows in `describe` as `summary`: one plain
  sentence, no internal module names.
- **Docs.** Every `atlx ...` line in the README and the skill is parsed by a
  test, so keep examples real (single-quote arguments with spaces) and keep
  placeholders out of code spans that start with `atlx`.

## Independence from twg

atlx is built from Atlassian's public REST documentation and API specs.
Keep it independent of Atlassian's twg CLI:

- Don't run twg to develop or test atlx, and don't record, commit or publish
  its output.
- Don't publish comparisons, benchmarks or other assessments of twg, in
  issues, PRs, docs or commit messages.
- Never copy twg's help text, descriptions, docs or skill files. Write every
  help string and doc in your own words.

## Parallel work

- Run at most one PR at a time per `area:*` label. Issues in the same area
  edit the same product files.
- If you run several worktrees (or agents) at once, give each its own
  `CARGO_TARGET_DIR`. A shared target directory makes the builds clobber each
  other.

## Cutting a release

No release has been published yet. Releases are built by
`.github/workflows/release.yml` when a `vX.Y.Z` tag is pushed. Only
maintainers do this.

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
   `cargo publish --dry-run`, builds all seven targets with completions and
   man pages, uploads the archives and `SHA256SUMS` to a new GitHub release,
   then runs `cargo publish`. The publish step is skipped with a warning when
   the `CARGO_REGISTRY_TOKEN` repository secret is not set.
5. Check the install paths: `cargo binstall atlx` and `cargo install atlx`.

To rehearse without releasing, run the workflow by hand (Actions, Release, Run
workflow, or `gh workflow run release.yml --ref <branch>`). It builds and
uploads the same archives as workflow artifacts and skips the release and
publish jobs. Pull requests that edit the workflow run it the same way.

Archive names must keep matching `pkg-url` in `[package.metadata.binstall]`:
`atlx-vX.Y.Z-<target>.tgz`, or `.zip` on Windows, with the binary at the root.

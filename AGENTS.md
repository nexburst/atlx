# AGENTS.md

Notes for coding agents working on atlx. The full guide is
[CONTRIBUTING.md](CONTRIBUTING.md).

## Commands

```sh
export CARGO_TARGET_DIR=...        # one per worktree when running in parallel
cargo check --all-targets          # fast loop
cargo test --test jira             # one test binary
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # before pushing
UPDATE_SNAPSHOTS=1 cargo test --test describe   # after any CLI or record change
UPDATE_SKILL=1 cargo test --test skill          # regenerate the skill's command references
```

## Where code goes

- API calls: `src/<product>.rs` and `src/<product>/`. They return REST
  payloads as `serde_json::Value`; records in `src/<product>/records.rs` turn
  them into output.
- Clap definitions, dispatch and the `META` registry table:
  `src/cli/<product>.rs` (and its submodules). Global flags and top-level
  dispatch are in `src/cli/mod.rs`; shared helpers in `src/cli/util.rs`.
- Mock-HTTP tests: `tests/<product>*.rs`, using `tests/common/mod.rs`.
- Parse tests: the product's CLI test module.

## Definition of done

- The command follows the grammar `<product> <resource> [<sub-resource>]
  <verb>`: REST nouns, singular; verb last; target positional; parents as
  named flags (`--issue`, `--page`, `--pr`, `--repo`, ...); shared flag names
  (`--limit`, `--cursor`, `--body*`, `--yes`, `--if-version`).
- It has a `CommandMeta` with its `effect` (`read`, `write`,
  `destructive`), its `record`, and at least one example that parses.
- It returns a registered record with a short default view; lists carry no
  bodies; big text fields have a truncation budget.
- Writes need nothing per command: dry-run and read-only mode are in the
  client. A POST that reads uses `Client::post_read`. Destructive commands
  get `--yes` from the registry; never call a confirmation helper by hand and
  never prompt.
- Tests are offline: a mock-HTTP test of the request and the record, a parse
  test, and the product's write-guard tests still pass (destructive refuses
  without `--yes` and sends nothing; writes stop under `--dry-run`).
- `describe` snapshots and skill references are regenerated and the diffs
  are intended.
- Errors are `client::ApiError` with the right kind; `main` prints them.
- fmt, clippy with `-D warnings`, and tests all pass.
- No new dependency without a stated reason. Sync `ureq` 2, no async.

## Independence from twg

- Build from Atlassian's public REST docs and specs. Never run twg to develop
  or test atlx, and never record, commit or publish its output.
- Never publish comparisons, benchmarks or assessments of twg, in issues, PRs,
  docs or commit messages.
- Never copy twg help text, descriptions, docs or skill files into the repo.
  All prose must be original.

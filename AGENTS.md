# AGENTS.md

Notes for coding agents working on atlx. The full guide is
[CONTRIBUTING.md](CONTRIBUTING.md).

## Commands

```sh
export CARGO_TARGET_DIR=...        # one per worktree when running in parallel
cargo check --all-targets          # fast loop
cargo test --test jira             # one test binary
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # before pushing
UPDATE_SNAPSHOTS=1 cargo test --test cli_tree   # after any CLI change
```

## Where code goes

- API calls: `src/<product>.rs`. They return twg's `data` as a `serde_json::Value`;
  dispatch wraps it with `output::envelope::envelope` (not for Bitbucket).
- Clap definitions and dispatch: `src/cli/<product>.rs`. Shared helpers are in
  `src/cli/util.rs`, global flags and top-level dispatch in `src/cli/mod.rs`.
- Mock-HTTP tests: `tests/<product>.rs`, using `tests/common/mod.rs`.
- Parse tests: the `tests` module of `src/cli/<product>.rs`.

## Definition of done

- Commands and flags use twg's names when twg has them.
- New API functions have a mock-HTTP test; new commands have a parse test.
- `tests/snapshots/cli-tree.txt` is regenerated and the diff is intended.
- README command table and `COMPAT.md` row updated; matching `gap` lines in
  `compat/corpus.txt` flipped to `ok` (`cargo test --test twg_compat`).
- fmt, clippy with `-D warnings`, and tests all pass.
- No new dependency without a stated reason. Sync `ureq` 2, no async.
- Confirmation follows twg: require `--yes` (`util::require_yes`) only where
  twg does; elsewhere accept it as a hidden no-op and never prompt. Return
  errors as `client::ApiError` where the class matters; `main` prints them.
- Renamed commands and flags stay as hidden aliases that call
  `util::deprecated(old, new)`; the note only shows in text mode on a terminal.

## twg text rule

Never copy twg help text, descriptions or skill files into the repo. Command
paths and flag names are fine; all prose must be original.

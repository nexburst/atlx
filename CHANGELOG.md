# Changelog

All notable changes to atlx are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow copies the section whose heading matches the tag into the
GitHub release notes, so keep one `## [X.Y.Z] - YYYY-MM-DD` heading per release.

## [Unreleased]

### Added

- `atlx completion script <bash|zsh|fish|powershell|elvish>` prints a shell
  completion script, using the same command shape as twg (#35).
- Man pages generated from the command tree, shipped in release archives (#35).
- Release workflow: tagged cross-platform builds for Linux (gnu and static
  musl, x86_64 and aarch64), macOS (x86_64 and Apple silicon) and Windows
  (x86_64 and ARM64), named to match the `cargo binstall` metadata, with
  `SHA256SUMS`, release notes from this file and a crates.io publish (#35).
- HTTP client transport: binary downloads, multipart uploads, per-request
  headers, retries on 429 and 5xx with backoff, and response bodies larger
  than 10 MB (#16).
- `CONTRIBUTING.md` and `AGENTS.md` contributor guides (#14).

- Typed errors: `client::ApiError` carries the error class, HTTP status,
  request and hint, and `client::Bytes` reports its HTTP status (#17).

### Changed

- Errors follow twg: in JSON modes an `{"ok": false, "error": {...}}` envelope
  goes to stdout and a readable report to stderr. Every failure exits 1
  (clap argument errors included, previously 2), and a partly failed batch
  read exits 3. Batch items now carry structured errors plus a `summary`, and
  `atlx search` reports per-product errors as objects (#17).
- Rich-text bodies convert with better fidelity: Markdown and HTML to Jira ADF
  and Confluence storage, and ADF back to GitHub-flavoured Markdown (#22).
- The CLI moved into `src/cli/` and the mock-HTTP tests were split per product,
  with a snapshot test of the whole command tree. No behaviour change (#14).

## [0.1.0] - 2026-11-13

First release.

### Added

- twg-compatible commands for Jira (`jira workitem get/search/query/create/
  update/comment create/transition/transitions query`), Confluence (`confluence
  search query/text`, `content get/create/update/list`, `space get`) and
  Bitbucket (`bb repo`, `bb pull-requests`, `bb pipeline`), on the public REST
  APIs with API-token auth.
- `login`, `logout`, `whoami` and `setup bitbucket/default-site`.
- Cross-product `atlx search` and a raw `atlx api` escape hatch.
- Output as text on a terminal and JSON when piped, plus `-o jsonl`.
- Jira metadata and Agile commands: `workitem create-meta`, issue links and
  remote links, `field list`, `project list/get`, `board list`, `sprint
  list/issues`, and `user search` (#3).
- Confluence comments (including inline), page tree, personal space, lookup by
  title, labels and delete (#4).
- Bitbucket PR diff, activity and tasks; approve, request changes, merge and
  decline; repository files and branches; code search (#5).
- `atlx doctor` for config, permission and connectivity checks; GitHub Actions
  CI on Linux, macOS and Windows; `cargo binstall` metadata; twg compatibility
  notes in the README (#6).
- Offline mock-HTTP integration tests and opt-in live smoke tests (#2).

### Fixed

- Argument validation, pagination and auth-origin edge cases (#1).

[Unreleased]: https://github.com/pedrosanzmtz/atlx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pedrosanzmtz/atlx/releases/tag/v0.1.0

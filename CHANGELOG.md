# Changelog

All notable changes to atlx are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow copies the section whose heading matches the tag into the
GitHub release notes, so keep one `## [X.Y.Z] - YYYY-MM-DD` heading per release.

## [Unreleased]

## [0.1.0] - 2026-11-23

The first release. atlx is a command line for Jira, Jira Service Management,
Confluence and Bitbucket Cloud, built on Atlassian's public REST APIs and
designed first for coding agents and scripts.

### The contract

- **Grammar.** `atlx <product> <resource> [<sub-resource>] <verb>`, with
  singular REST nouns (`jira issue`, `confluence page`, `bb pr`), the verb
  last, the target positional and parents as named flags (`--issue`,
  `--page`, `--pr`, `--repo workspace/slug`, `--project`, `--space`).
- **Envelope.** Every command prints
  `{"ok":…,"data":…,"next":…,"truncated":…,"warnings":…}`, one line of
  compact JSON when piped. `-o jsonl` streams records with an `_end` trailer,
  `-o ids` prints one ID per line, and `get` reads IDs from stdin with `-`.
- **Records.** Commands return curated records with a short default view;
  `--full` shows every field, `--fields a,b.c` picks some, and `--raw`
  returns the REST payload.
- **Truncation.** Bodies, diffs, file contents and pipeline logs are cut on a
  block boundary within a budget, and `truncated` names the flag that
  continues them. `--max-chars` changes the budget.
- **Paging.** Every `list` and `search` takes `--limit`, `--cursor` and
  `--all`, and returns `next`.
- **Errors and exit codes.** `error.code` is one of `usage`, `config`,
  `auth`, `forbidden`, `not_found`, `conflict`, `validation`,
  `rate_limited`, `server`, `network`, `timeout`, `refused` or `other`, with a
  `hint` and `retryable`. Exit codes: 0 ok, 1 failed, 2 usage, 3 partial,
  4 config/auth/forbidden, 5 not found, 6 conflict, 7 retry later, 8 refused
  by a safety guard.
- **`atlx describe`** prints every command's flags, effect, record fields,
  examples and exit codes as JSON.

### Safe writes

- `--dry-run` works on every command: reads run, the first write is printed,
  and nothing is sent.
- Read-only mode (`--read-only`, `ATLX_READ_ONLY=1` or `read_only = true` in
  a profile) refuses every write, `atlx api` included.
- `--yes` is required on every destructive command (deletes, merges,
  declines, restores, permission changes, customer-visible JSM actions) and
  on `atlx api -X DELETE`. Nothing prompts.
- Confluence `page update` and `page edit` require `--if-version N`, and
  `bb pr merge --if-head <sha>` checks the head commit. A stale version exits 6.
- Requests whose path has a `.` or `..` segment are refused before they are
  sent.

### Bodies

Markdown is the default for reading and writing in every product, converted
through ADF with loss reporting. A write that cannot be represented, or a
full-body replace that would destroy macros or layouts, is refused (exit 8).
A lossy read carries a `lossy_body` warning. `--body-format` also accepts
`adf`, `storage` and `plain`.

### Products

- **Jira:** issues (create, bulk create, update, transition, archive),
  comments, changelog, worklogs, attachments, links, watchers, votes and
  properties; boards, sprints and backlog; projects, versions, components,
  filters, dashboards, fields, statuses and users.
- **Jira Service Management:** requests, participants, approvals, SLAs,
  desks, request types, queues and knowledge base search.
- **Confluence:** search, pages (section reads, targeted `page edit`, move,
  copy, archive, trash and restore), comment threads and inline comments,
  versions and diffs, labels, attachments, tasks, page states, templates and
  spaces.
- **Bitbucket Cloud:** repositories, files without a checkout, code search,
  branches, commits, Code Insights reports, pull requests (review, inline
  comments, tasks, reviewers, checks, diffs, merge), pipelines and logs,
  deployments and environments.
- **Core:** `auth` with named profiles and credential helpers, cross-product
  `atlx search`, `atlx doctor`, the raw `atlx api` escape hatch, a 24-hour
  metadata cache (never content), `.atlx.toml` repository defaults, shell
  completions, man pages and an agent skill (`atlx skill install`).

[Unreleased]: https://github.com/nexburst/atlx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nexburst/atlx/releases/tag/v0.1.0

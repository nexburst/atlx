# Changelog

All notable changes to atlx are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow copies the section whose heading matches the tag into the
GitHub release notes, so keep one `## [X.Y.Z] - YYYY-MM-DD` heading per release.

## [Unreleased]

### Security

- `confluence content create`, `update` and `delete` (including `--purge`)
  wrote without confirmation where twg refuses. They now need `--yes`.
  `update` needs `--snapshot-token`, so it can't overwrite a newer edit
  unseen, and a body needs `--format` and `--ack-body-formats`. Scripts that
  relied on the old behavior must add these flags.

### Added

- Jira dashboards (#73): `jira dashboard query|get|create|update|copy|
  delete|bulk-edit`, `dashboard gadget` and `dashboard item-property`.
  `update` keeps what isn't given; deletes and bulk edits need `--yes`.
- Confluence (#71): `content tasks list|get|complete|reopen`, `content
  status get|set`, `content list-available-statuses`, `content
  watch|unwatch`, `space watch|unwatch`, `space permission list|available`
  and `templates list|get`.
- Jira in bulk (#69): `jira workitem bulk-get|create-bulk|bulk-transition|
  archive|unarchive`, `jira workitem property get|query|set|delete`, and
  `user get|bulk-lookup`. A bulk transition needs `--yes`; partial results
  exit 3.
- Jira Service Management over the public Service Desk API (#37): `jsm
  request`, `request-type`, `approval`, `portal`/`servicedesk`, `queue`,
  `sla get` and `knowledge-article query`. Request comments are public to
  customers unless `--internal` is given.
- An agent skill written for atlx, with `atlx skill install` and `atlx skill
  print` (#36).
- Bitbucket repository commands (#33): `branch create|delete`, `commit
  query|create`, Code Insights `commit report create|query`, `workspace
  get`, `workspace member query|get`, `repo create|list-files|url|
  contributors` and `search prs`.
- Confluence lifecycle (#30): `content move|copy|trash|untrash|purge|archive|
  publish|delete-draft`, `create --private --content-width --title-emoji`,
  `space create|update|archive|unarchive|delete`, and `tree` for whole
  spaces, with `--type`, depth 10 and a 500-node cap. Writes need `--yes`
  where twg's do; `space delete` refuses content URLs.
- Jira administration (#26): space lifecycle and lookups, versions,
  components, saved filters, and `jira workitem query --filter`. Space
  delete (to the trash), archive and restore need `--yes` and accept only an
  exact key, ID, ARI or project URL.
- Bitbucket Pipelines and deployments (#34): `pipeline run|trigger`,
  `latest-failure`, `tail`, `grep`, `rerun-failed --whole-pipeline`, `config
  get|update`, `get --test-reports --test-cases`, `query --pattern`, and
  `bb deployment`. Secured variable values come from the environment and
  are never printed. `pipeline get --step` now takes an index and reports
  an unknown step; `query --status PASSED` now matches successful runs.
- Bitbucket pull request lifecycle (#32): `pull-requests update`, reviewers
  by name on `create`, `merge --wait`, `merge-status`, `mergeability`,
  `conflicts`, `for-commit`, `remove-request-changes`, default reviewers,
  `query --reviewer --page --merged-since --merged-before --scope me`, and
  `bb inbox`.
- Confluence comment threads (#29): `content comments
  get|reply|update|delete|resolve|reopen`, inline comments with `create
  --text-selection`, and `list --include-replies --sort --status
  --resolution-status --content-type --reference-space`. `delete` needs
  `--yes`, as in twg; resolution is for inline comments only.
- Jira Agile (#25): `jira sprint get|create|update|start|complete|delete`,
  `jira board get|create|delete`, `board backlog|projects|quick-filter
  query`, `board quick-filter get`, and `--sprint`, `--story-points`,
  `--original-estimate`, `--remaining-estimate` and `--team` on `workitem
  create|update`. Names are resolved before anything is written.
- Jira collaboration commands (#24): `jira workitem attachment
  query|get|download|thumbnail|upload|delete`, `worklog
  add|update|query|get|delete|changed|deleted`, `watcher query|add|remove`
  and `vote query|add|remove`. Attachment and worklog deletes need `--yes`,
  as in twg.
- Confluence versions, labels and attachments (#28): `content versions
  list|get|diff|restore` (also `content history list|get|diff`), `labels
  add|remove|watch|unwatch`, `labels list --content-type --reference-space`,
  `attachments list|get|download|upload|delete`, an offline `content
  body-formats` guide and `content get --format url`. Commands confirm with
  `--yes` where twg's do. A download without `--out` refuses hidden or
  otherwise unsafe file names from the server.
- Jira triage commands with twg's names and output (#23): `jira workitem
  comment query|update|delete`, `comment create --parent-id`, `changelog
  query`, `field update-metadata`, `link query`, `unlink workitem|weblink`,
  `delete`, `types query|get`, `statuses query` and `priorities query`.
  Comment and changelog pages print `pageInfo` with the next `--after`
  cursor. Deletes ask for no confirmation, as in twg.

- Bitbucket pull request review loop (#31): `comment create --reply-to
  --pending` and inline anchors (`--path` with `--line`, `--from-line` or a
  `--start-line/--end-line` or `--start-from-line/--end-from-line` range);
  `comment query|get|update|delete|resolve|reopen`; `task
  create|get|update|delete|resolve|reopen`; `diffstat`, `commits`, `patch`;
  and `diff-line`, which finds diff lines by text and gives the line numbers
  an inline comment needs. Comment and task `delete` need `--yes`, as in twg.

- Confluence section reads and targeted edits (#27): `content get --detail
  outline` (headings with level, anchor, ADF `localId` and word count),
  `--section <heading>` (repeatable; Markdown by default, the exact storage
  slice with `--format html`), the atlx-extra `--format adf`, and
  `content update --edits`/`--edits-file` with twg's `localId` operations
  plus heading- and text-based extras. Edits apply all-or-nothing to the ADF
  body, honour `--snapshot-token`, and `--dry-run` prints the resulting HTML
  and a unified diff without writing.

- twg's global output controls: `--select` (dot-path projection with
  `@compact`, `@evidence` and `@rows` presets), `--output-file`,
  `--output-summary [none|stats|auto|inline]` with its YAML envelope,
  `--agent-fields`, and `--api-version v2`. `confluence content get
  --body-only --output-file` keeps the snapshot token on stdout (#19).

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
- Named profiles: global `-p/--profile` and `ATLX_PROFILE`, `login
  --new-profile`, per-profile `setup bitbucket` and `setup default-site`, and
  `auth profile list|default|rename|migrate`. v1 config files load unchanged as
  the `default` profile (#20).
- Credential helpers: `token_cmd` / `bitbucket.token_cmd`, `ATLX_TOKEN_CMD`,
  `ATLX_BB_TOKEN_CMD` and `--token-cmd` on `login` and `setup bitbucket`,
  run lazily with a 10 s timeout. `whoami` and `doctor` report the active
  profile and token source (#20).

- twg 1.4.4 command names and flags for commands that had drifted (#18):
  `jira space query|list|get`, `jira board query`, `jira board sprints query`,
  `jira sprint workitems query`, `jira workitem field create-metadata` (also
  `custom-fields`), `jira workitem link-types query`, `jira workitem link
  workitem` (repeatable `--target-id`) and `link weblink` (with `--global-id`,
  `--application-name/-type`, `--summary`, `--icon-url`, `--resolved`),
  `confluence content labels list --id`, `comments list|create --id` with
  `--comment-type` and `--body-format` (create also takes `--body-file`),
  `confluence tree --target`, `confluence space get|list --labels`,
  `bb branch query` (`--author`, `--sort`), `bb pull-requests task query`,
  `merge --pull-request --merge-strategy --merge-message
  --keep-source-branch`, batched `bb pull-requests activity <id...>` with
  `--strict`, `bb repo query --repo`, the `-w`/`-r` short flags, and `user
  search --name --query --email --account-id -n`.
- `--batch-concurrency <1-20>` (default 5) on `jira workitem get`, `bb
  pull-requests get` and `bb pull-requests activity`: batch reads now run in
  parallel on scoped threads and keep the input order (#18).

- `scripts/conformance.sh` compares twg's and atlx's JSON on a live site
  (read-only; outputs stay in the git-ignored `target/conformance/`), and
  `scripts/twg-fixtures.sh` turns a run into the synthetic fixtures behind
  `tests/twg_shapes.rs` (#21).
- twg commands that need Teamwork Graph, Rovo or the GraphQL gateway (`rovo`,
  `work`, `context`, `jira board cells query`, ...) fail with "not supported
  by atlx (requires Teamwork Graph)" in twg's error envelope and exit code 2.
  Planned twg commands fail with a pointer to their issue (#21).
- `user search --compact` (#21).

### Changed

- **JSON output now matches twg** (#21). Jira, Confluence, user and account
  commands print twg's envelope (`apiVersion`, `command`, `request`, `data`,
  `pageInfo`, `meta`) with twg's key names, which are mostly the REST APIs'
  own: `jira workitem get` prints `data: [<issue>]` with REST fields next to
  `key`, `query`/`search` print `data.issues` rows and `pageInfo.nextCursor`
  instead of `next_cursor`, `transitions query` prints `{id, name, toName,
  category}`, `space query` prints `projects`, `board query` `boards`, and
  `whoami` twg's profile fields. Bitbucket commands print Bitbucket's REST
  objects without an envelope, as twg does. Batches gain `returned`,
  `omitted` and `truncated` counts. `-o jsonl` adds twg's `_twg` locator to
  each row. The old snake_case summary keys are gone, and `--select` presets
  no longer carry fallbacks for them.
- `user search` returns 20 users by default, as twg does (was 25).
- `jira space query` sorts by key unless `--sort-by` is given, as twg does.
- Merge and decline no longer require `--yes`,
  matching twg; `--yes` is still accepted and ignored (#18).
- The 0.1 names (`jira project`, `jira board list`, `jira sprint list|issues`,
  `jira workitem create-meta`, `jira workitem link types|create|remote`, `bb
  repo branches`, `bb pull-requests tasks`, positional `labels`/`comments`/
  `merge` forms, and `--inline`, `--format`, `--strategy`, `--message`) still
  work but are hidden from help. In text mode on a terminal they print a
  one-line note naming the new form; they will be removed in 0.3 (#18).
- `atlx logout` removes only the selected profile (the whole file only when
  nothing else is left), as twg does (#20).
- `login` and `setup bitbucket` accept `-f` for `--force`, and `setup bb` is an
  alias of `setup bitbucket`, matching twg (#20).

- Errors follow twg: in JSON modes an `{"ok": false, "error": {...}}` envelope
  goes to stdout and a readable report to stderr. Every failure exits 1
  (clap argument errors included, previously 2), and a partly failed batch
  read exits 3. Batch items now carry structured errors plus a `summary`, and
  `atlx search` reports per-product errors as objects (#17).
- Rich-text bodies convert with better fidelity: Markdown and HTML to Jira ADF
  and Confluence storage, and ADF back to GitHub-flavoured Markdown (#22).
- The CLI moved into `src/cli/` and the mock-HTTP tests were split per product,
  with a snapshot test of the whole command tree. No behaviour change (#14).

### Fixed

- Moved off deprecated REST endpoints (#66). `bb search code` uses
  `codesearch`: the token needs `repository:read` and `workspace:read`, and
  hits keep `file.path` and `content_match_count` (now matched chunks) but
  otherwise change shape. `jira sprint workitems query` and `jira board
  backlog query` page by token, so no longer report `total`. Priorities and
  Confluence children use their replacement endpoints.
- Checked against Atlassian's OpenAPI specs (#65): `confluence content
  untrash` restores through REST v2 (v1's endpoint is gone); `labels list`
  refuses content types REST can't list instead of calling a removed
  endpoint; `jira filter share add` sends `authenticated` for `loggedin`.
- Descending sort values such as `--sort -name` can be typed with a space
  in `bb branch query`, `bb repo query`, `jira space query` and `confluence
  content attachments list` (#65).
- Requests whose path has a `.` or `..` segment, and DELETEs with an empty
  one, are refused before they are sent. An ID of `..` used to address the
  parent resource, so deleting a comment could delete its work item.

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

[Unreleased]: https://github.com/nexburst/atlx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nexburst/atlx/releases/tag/v0.1.0

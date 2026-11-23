# Changelog

All notable changes to atlx are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow copies the section whose heading matches the tag into the
GitHub release notes, so keep one `## [X.Y.Z] - YYYY-MM-DD` heading per release.

## [Unreleased]

### Breaking: a command line designed for coding agents

atlx is redesigned around one contract for agents and scripts: one grammar,
one envelope, curated records, explicit truncation, one paging model, its
own error codes and exit codes, a self-describing schema, and fail-closed
writes. Nearly every command path, flag and output shape changes. Old paths
are not kept as aliases; they fail with a usage error (exit 2) and
suggestions. `atlx describe` documents the new tree.

- **Grammar.** `atlx <product> <resource> [<sub-resource>] <verb>`: singular
  REST nouns (`jira issue`, `jira project`, `confluence page`, `bb pr`), the
  verb last (`list`, `search`, `get`, `create`, `update`, `delete`, or a
  REST action), the target positional and parents as named flags (`--issue`,
  `--page`, `--pr`, `--repo workspace/slug`, `--project`, `--space`).
  `query` is gone; `--id`, `--issue-id`, `--pull-request` and the like are
  gone.
- **Envelope.** Every command in every product, Bitbucket included, prints
  `{"ok":…,"data":…,"next":…,"truncated":…,"warnings":…}` as one line of
  compact JSON when piped. `data` is a list for lists and multi-ID gets,
  with no wrapper key; partial results carry `errors`. `apiVersion`,
  `command`, `request`, `meta` and `pageInfo` are gone, and stderr stays
  empty in JSON modes. `-o jsonl` ends with an `_end` trailer line; new
  `-o ids` prints one ID per line, and `get` reads IDs from stdin with `-`.
- **Records.** Commands return curated records instead of REST payloads:
  a short default view, `--full` for every field, `--fields a,b.c` to pick
  (an unknown name is a usage error), `--raw` for the REST payload.
  `--select`, its presets, `--agent-fields`, `--output-summary` and
  `--api-version` are removed.
- **Truncation.** Bodies (20 000 characters), pull request diffs (40 000),
  file contents (20 000) and pipeline logs (last 200 lines) are cut on a
  block boundary and announced in `truncated` with the flag that continues
  them (`--body-offset`, `--diff-offset`, `--content-offset`,
  `--log-offset`). `--max-chars` changes the budget; `0` turns it off.
- **Paging.** Every `list` and `search` takes `--limit` (default 25, at most
  1000), `--cursor` and `--all`, and returns `next`. `--max-results`,
  `--start-at`, `--first`, `--after` and `--page` are removed.
  `--concurrency` replaces `--batch-concurrency`.
- **Errors and exit codes.** `error.code` is one of `usage`, `config`,
  `auth`, `forbidden`, `not_found`, `conflict`, `validation`,
  `rate_limited`, `server`, `network`, `timeout`, `refused`, `other`, with a
  `hint` and `retryable`. Exit codes: 0 ok, 1 failed, 2 usage, 3 partial,
  4 config/auth/forbidden, 5 not found, 6 conflict, 7 retry later, 8
  refused by a safety guard. The old error codes, `kind`, `statusCode`,
  `downstreamError` and `retry.guidance` are removed, and exit 2 no longer
  means "unsupported".
- **`atlx describe`** prints every command's flags, effect, record fields,
  examples and exit codes as JSON (`--records`, `--errors`, `--all`).
- **Safe writes.** `--dry-run` is global and works on every command (reads
  run, the first write is printed, nothing is sent). Read-only mode
  (`--read-only`, `ATLX_READ_ONLY=1`, `read_only = true` in a profile)
  refuses every write, `api` included. `--yes` is required on every
  destructive command and nowhere else: Jira deletes, sprint, version and
  component deletes and `bb pr merge`/`decline` now need it; Confluence
  create and update, label add and watch no longer need it. Nothing
  prompts.
- **Version checks.** Confluence `page update` and `page edit` require
  `--if-version N` (replaces `--snapshot-token`); a stale version exits 6.
  `bb pr merge --if-head <sha>` is new.
- **Bodies.** Markdown is the default for reading and writing in every
  product, converted through ADF with loss reporting: a Markdown write that
  cannot be represented, or a full-body replace that would destroy macros or
  layouts, is refused (exit 8); a lossy read carries a `lossy_body` warning
  and markers. HTML is no longer an input format; `--body-format` is the one
  flag (`markdown`, `adf`, `storage`, `plain`). `--format`,
  `--description-format` and `--ack-body-formats` are removed. Jira `issue
  get` returns the description as Markdown.
- **Auth.** `login`, `logout`, `whoami` and `setup` move under `auth`
  (`auth status` replaces `whoami`). Agents are never prompted: missing
  credentials fail with `config` (exit 4).
- **Repository defaults.** An optional `.atlx.toml` sets `profile`,
  `jira.project`, `confluence.space` and `bitbucket.repo`; Bitbucket `--repo`
  also defaults to the git `origin` remote.
- **Metadata cache.** Jira field names, issue types and priorities,
  Confluence space IDs and the Bitbucket user are cached for 24 hours
  (`atlx cache path|clear`, `--no-cache`). Content is never cached.
- **Skill.** `skills/atlx` is rewritten; its command references are
  generated from `atlx describe`. Reinstall with `atlx skill install --force`.

#### Core commands

| Old | New |
| --- | --- |
| `login`, `logout` | `auth login`, `auth logout` |
| `whoami` | `auth status` |
| `setup bitbucket` | `auth bitbucket` |
| `setup default-site` | `auth profile update -s SITE` |
| `user search`, `user get`, `user bulk-lookup` | `jira user search`, `jira user get` (several IDs or emails) |
| | `describe`, `cache path`, `cache clear` (new) |

#### Jira

| Old | New |
| --- | --- |
| `jira workitem get`, `bulk-get` | `jira issue get KEY...` |
| `jira workitem query`, `search` | `jira issue list --jql`, `jira issue search TEXT` |
| `jira workitem create`, `create-bulk` | `jira issue create` (`--bulk-file`) |
| `jira workitem update`, `delete`, `archive`, `unarchive` | `jira issue update`, `delete`, `archive`, `restore` |
| `jira workitem transition`, `bulk-transition` | `jira issue transition KEY... --to STATUS` |
| `jira workitem transitions query` | `jira transition list --issue KEY` |
| `jira workitem comment query`, `changelog query` | `jira issue comment list`, `jira issue changelog list` |
| `jira workitem worklog add`, `query`, `changed`, `deleted` | `jira issue worklog create`, `list` (`--changed-since`, `--deleted-since`) |
| `jira workitem attachment query`, `thumbnail` | `jira issue attachment list`, `download --thumbnail` |
| `jira workitem link workitem`, `link query`, `unlink workitem` | `jira issue link create`, `list`, `delete` |
| `jira workitem link weblink`, `unlink weblink` | `jira issue weblink create`, `delete` |
| `jira workitem watcher`, `vote` `query`/`add`/`remove` | `jira issue watcher list`/`create`/`delete`, `jira issue vote get`/`create`/`delete` |
| `jira workitem property query`, `set` | `jira issue property list`, `update` |
| `jira workitem types`, `statuses`, `priorities`, `link-types` | `jira issue-type`, `status`, `priority`, `link-type` `list`/`get` |
| `jira workitem field create-metadata`, `update-metadata` | `jira field list --project P --type T`, `jira field list --issue KEY` |
| `jira space query`, `get`, `create`, `update`, `delete`, `archive`, `restore` | `jira project list`, `get`, `create`, `update`, `delete`, `archive`, `restore` |
| `jira space versions query`, `space version ...` | `jira project version list`, `project version ...` |
| `jira space component query`, `component ...` | `jira project component list`, `project component ...` |
| `jira space issue-types`, `space status query` | `jira issue-type list --project`, `jira status list --project` |
| `jira board query`, `quick-filter query` | `jira board list`, `board quick-filter list` |
| `jira board backlog query`, `sprint workitems query` | `jira issue list --backlog BOARD`, `jira issue list --sprint SPRINT` |
| `jira board projects query`, `board sprints query` | `jira project list --board`, `jira sprint list --board` |
| `jira filter query`, `share query`/`add`/`remove`, `subscription query` | `jira filter list`, `filter share list`/`create`/`delete`, `filter subscription list` |
| `jira filter add-favourite`, `remove-favourite`, `change-owner` | `jira filter update --favourite BOOL`, `--owner` |
| `jira filter columns get`/`set`, `reset-columns` | `jira filter column list`/`update`/`delete` |
| `jira dashboard query`, `gadget query`/`add`, `gadget catalog query` | `jira dashboard list`, `dashboard gadget list`/`create`, `dashboard gadget list --catalog` |
| `jira dashboard item-property query`/`set` | `jira dashboard property list`/`update` |
| `jira space types query`, `space notification-scheme get`, `version counts`, `component counts`, `dashboard bulk-edit` | removed; use `api` (version `get` includes issue counts) |

#### Jira Service Management

| Old | New |
| --- | --- |
| `jsm request query`, `request validate` | `jsm request list`, `jsm request create --dry-run` |
| `jsm request transition query`, `transition create` | `jsm transition list --request KEY`, `jsm request transition KEY --to X` |
| `jsm request participant add`, `remove` | `jsm request participant create`, `delete` |
| `jsm approval list`/`get`/`approve`/`reject` | `jsm request approval list`/`get`/`approve`/`decline` |
| `jsm sla get` | `jsm request sla list`, `get` |
| `jsm portal query`, `servicedesk query` | `jsm desk list` |
| `jsm request-type query`, `field get`, `fields get`, `form get` | `jsm request-type list`, `jsm request-type get ID --desk D --full` |
| `jsm queue query`, `queue workitems query` | `jsm queue list`, `jsm queue issue list` |
| `jsm knowledge-article query` | `jsm article search TEXT` |

#### Confluence

| Old | New |
| --- | --- |
| `confluence search query`, `search text` | `confluence search [TEXT] [--cql CQL]` |
| `confluence content get`, `list`, `create`, `update` | `confluence page get`, `list`, `create`, `update` |
| `confluence content update --edits` | `confluence page edit` |
| `confluence content children` | `confluence page list --parent ID` |
| `confluence content trash`, `purge`, `delete-draft`, `untrash` | `confluence page delete`, `delete --purge`, `delete --draft`, `restore` |
| `confluence content move`, `copy`, `archive`, `publish`, `watch`, `unwatch` | `confluence page move`, `copy`, `archive`, `publish`, `watch`, `unwatch` |
| `confluence tree` | `confluence page tree`, `confluence space tree` |
| `confluence content comments ...`, `comments reply`, `resolve`, `reopen` | `confluence page comment ...`, `comment create --comment ID`, `comment update --resolve`/`--reopen` |
| `confluence content attachments ...` | `confluence page attachment ...` |
| `confluence content labels list`/`add`/`remove`, `labels watch`/`unwatch` | `confluence page label list`/`create`/`delete`, `confluence label watch`/`unwatch` |
| `confluence content versions ...`, `content history ...` | `confluence page version list`/`get`/`diff`/`restore` |
| `confluence content tasks list`/`get`/`complete`/`reopen` | `confluence page task list`/`get`, `task update --status complete`/`open` |
| `confluence content status get`/`set`, `list-available-statuses` | `confluence page state get`/`update`/`list` |
| `confluence space unarchive`, `space me` | `confluence space restore`, `confluence space get --personal` |
| `confluence templates list`, `get` | `confluence template list`, `get` |
| `confluence content body-formats`, `space permission available` | removed (body formats are documented in the README and the skill) |

#### Bitbucket

| Old | New |
| --- | --- |
| `bb repo query`, `repo file`, `repo list-files`, `repo contributors` | `bb repo list`, `bb repo file get`, `bb repo file list`, `bb repo contributor list` |
| `bb search code`, `bb repo url` | `bb repo file search TEXT`; removed (records carry `url`) |
| `bb branch query`/`create`/`delete`, `repo branches` | `bb repo branch list`/`create`/`delete` |
| `bb commit query`/`create`, `commit report query`/`create` | `bb repo commit list`/`create`, `bb repo report list`/`create` |
| `bb pull-requests query`, `search prs`, `for-commit`, `inbox` | `bb pr list`, `bb pr search TEXT`, `bb pr list --commit`, `bb pr list --mine --waiting` |
| `bb pull-requests get`, `create`, `update`, `merge`, `decline`, `approve`, `unapprove`, `request-changes` | `bb pr get`, `create`, `update`, `merge`, `decline`, `approve`, `unapprove`, `request-changes` |
| `bb pull-requests remove-request-changes` | `bb pr request-changes --withdraw` |
| `bb pull-requests diff`, `patch`, `diff-line`, `diffstat`, `commits`, `activity` | `bb pr diff`, `diff --patch`, `diff --grep`, `diffstat`, `commits`, `activity` |
| `bb pull-requests mergeability`, `conflicts`, `merge-status` | `bb pr checks`, `bb pr diffstat --conflicts`, `bb pr get` |
| `bb pull-requests comment ...`, `task ...`, `tasks` | `bb pr comment ...`, `bb pr task ...`, `bb pr task list` |
| `bb pull-requests update --add-reviewer` | `bb pr reviewer add`/`remove`/`list` |
| `bb pull-requests default-reviewer ...`, `effective-default-reviewer query` | `bb repo default-reviewer ...`, `default-reviewer list --effective` |
| `bb pipeline query`, `trigger`, `rerun-failed` | `bb pipeline list`, `bb pipeline run`, `bb pipeline run --rerun` |
| `bb pipeline get --logs`, `latest-failure`, `tail`, `grep` | `bb pipeline log`, `log --failed`, `log --follow`, `log --grep` |
| `bb pipeline get --test-reports` | `bb pipeline step get STEP --pipeline N` |
| `bb deployment query`, `deployment history query`/`get` | `bb deployment list`/`get` (`--environment`) |
| `bb deployment commits query`, `deployment environments query`, `deployment environment variable query` | `bb deployment commits UUID`, `bb environment list`, `bb environment variable list` |
| `bb workspace member query` | `bb workspace member list` |

### Added

Since 0.1.0, besides the redesign: Jira collaboration (attachments,
worklogs, watchers, votes, properties), Agile (boards, sprints, backlog),
administration (projects, versions, components, filters, dashboards) and
bulk create; Jira Service Management requests, approvals, SLAs, queues and
knowledge base search; Confluence comment threads and inline comments,
versions, labels, attachments, tasks, page states, templates, page and space
lifecycle, section reads and targeted `page edit`; Bitbucket pull request
review (inline comments, tasks, reviewers, checks), repositories, branches,
commits without a checkout, Code Insights reports, pipelines, logs and
deployments; named profiles and credential helpers; shell completions, man
pages and the release workflow.

### Security

- Confluence bodies are no longer sent as storage format built from
  unchecked HTML, which could drop content Confluence did not accept.
- Requests whose path has a `.` or `..` segment, and DELETEs with an empty
  one, are refused before they are sent. An ID of `..` used to address the
  parent resource.

### Fixed

- Moved off deprecated REST endpoints (#66) and checked requests against
  Atlassian's OpenAPI specs (#65). Bitbucket code search uses `codesearch`
  and needs `repository:read` and `workspace:read`.

## [0.1.0] - 2026-11-13

First release.

### Added

- Commands for Jira (`jira workitem get/search/query/create/
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
  CI on Linux, macOS and Windows; `cargo binstall` metadata (#6).
- Offline mock-HTTP integration tests and opt-in live smoke tests (#2).

### Fixed

- Argument validation, pagination and auth-origin edge cases (#1).

[Unreleased]: https://github.com/nexburst/atlx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nexburst/atlx/releases/tag/v0.1.0

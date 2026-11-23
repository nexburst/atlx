# Jira Service Management commands

<!-- Generated from `atlx describe` (contract 1) by `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->

Covers: `jsm`: requests, approvals, SLAs, desks, request types, queues, articles. Every command also takes the global flags (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, `--read-only`, `-p`, ...). "paged" commands take `--limit` (default 25, at most 1000), `--cursor` and `--all`; "multi-ID" commands take several IDs (or `-` for stdin) plus `--strict` and `--concurrency`. `...` marks a repeatable value. "Returns" lists the record's default fields; `atlx describe <path>` has every field, flag type and default.

## jsm request

- **`jsm request list`** (read, paged). Requests you can see, by status, desk, type or text.
  Takes flags `--status open|closed|all`, `--desk`, `--type`, `-q/--query`.
  Returns `jsm.request`: key, title, status, type, desk, reporter, created, url.
  - `atlx jsm request list --status open --desk HELP`
  - `atlx jsm request list -q vpn --limit 10`
- **`jsm request get`** (read, multi-ID). Requests by key, ID or URL.
  Takes args `KEYS...`.
  Returns `jsm.request`: key, title, status, type, desk, reporter, created, url.
  - `atlx jsm request get HELP-12`
- **`jsm request create`** (write). Raise a request; the values are checked against the type's fields first (preview with --dry-run).
  Takes flags `--desk (required)`, `--type`, `--summary`, `--description`, `--field...`, `--fields-json`, `--answer...`, `--answers-json`, `--answers-file`, `--on-behalf-of`.
  Returns `jsm.request`: key, title, status, type, desk, reporter, created, url.
  - `atlx jsm request create --desk HELP --type 'Get IT help' --summary 'VPN down' --description 'Since 9am'`
  - `atlx jsm request create --desk HELP --type 'Get IT help' --answers-file answers.json --dry-run`
- **`jsm request transition`** (destructive, needs --yes). Make a customer transition (customers see it).
  Takes args `KEY`; flags `--to (required)`, `--comment`, `-y/--yes`.
  Returns `jsm.transitioned`: request, transition, transitionId.
  - `atlx jsm request transition HELP-12 --to Resolve --comment 'Fixed' --yes`

## jsm request comment

- **`jsm request comment list`** (read, paged). A request's comments, public and internal.
  Takes flags `--request (required)`.
  Returns `jsm.comment`: id, author, created, public, body.
  - `atlx jsm request comment list --request HELP-12`
- **`jsm request comment create`** (destructive, needs --yes). Add a comment; customers see it unless --internal.
  Takes flags `--request (required)`, `--body`, `--body-file`, `--internal`, `-y/--yes`.
  Returns `jsm.comment`: id, author, created, public, body.
  - `atlx jsm request comment create --request HELP-12 --body 'Looking into it' --yes`
  - `atlx jsm request comment create --request HELP-12 --body 'Agent note' --internal --yes`

## jsm request participant

- **`jsm request participant list`** (read, paged). A request's participants.
  Takes flags `--request (required)`.
  Returns `jsm.participant`: id, name, email, deleted.
  - `atlx jsm request participant list --request HELP-12`
- **`jsm request participant create`** (write). Add participants.
  Takes args `USERS...`; flags `--request (required)`.
  Returns `jsm.participant`: id, name, email, deleted.
  - `atlx jsm request participant create alice@example.com --request HELP-12`
- **`jsm request participant delete`** (destructive, needs --yes). Remove participants.
  Takes args `USERS...`; flags `--request (required)`, `-y/--yes`.
  Returns `jsm.participant`: id, name, email, deleted.
  - `atlx jsm request participant delete alice@example.com --request HELP-12 --yes`

## jsm request approval

- **`jsm request approval list`** (read, paged). A request's approvals.
  Takes flags `--request (required)`.
  Returns `jsm.approval`: id, name, decision, canAnswer, approvers, created.
  - `atlx jsm request approval list --request HELP-12`
- **`jsm request approval get`** (read, multi-ID). Approvals by ID.
  Takes args `IDS...`; flags `--request (required)`.
  Returns `jsm.approval`: id, name, decision, canAnswer, approvers, created.
  - `atlx jsm request approval get 7 --request HELP-12`
- **`jsm request approval approve`** (destructive, needs --yes). Approve, as yourself.
  Takes args `[ID]`; flags `--request (required)`, `-y/--yes`.
  Returns `jsm.approval`: id, name, decision, canAnswer, approvers, created.
  - `atlx jsm request approval approve --request HELP-12 --yes`
- **`jsm request approval decline`** (destructive, needs --yes). Decline, as yourself.
  Takes args `[ID]`; flags `--request (required)`, `-y/--yes`.
  Returns `jsm.approval`: id, name, decision, canAnswer, approvers, created.
  - `atlx jsm request approval decline 7 --request HELP-12 --yes`

## jsm request sla

- **`jsm request sla list`** (read, paged). A request's SLAs.
  Takes flags `--request (required)`.
  Returns `jsm.sla`: id, name, breached, paused, remaining, goal, breachTime.
  - `atlx jsm request sla list --request HELP-12`
- **`jsm request sla get`** (read, multi-ID). SLAs by ID or name.
  Takes args `IDS...`; flags `--request (required)`.
  Returns `jsm.sla`: id, name, breached, paused, remaining, goal, breachTime.
  - `atlx jsm request sla get 'Time to resolution' --request HELP-12`

## jsm transition

- **`jsm transition list`** (read, paged). The transitions you can make on a request.
  Takes flags `--request (required)`.
  Returns `jsm.transition`: id, name.
  - `atlx jsm transition list --request HELP-12`

## jsm desk

- **`jsm desk list`** (read, paged). Service desks, by project key or name.
  Takes flags `-q/--query`.
  Returns `jsm.desk`: id, key, name.
  - `atlx jsm desk list -q help`

## jsm request-type

- **`jsm request-type list`** (read, paged). Request types, of one service desk or all.
  Takes flags `--desk`, `-q/--query`.
  Returns `jsm.request-type`: id, name, description.
  - `atlx jsm request-type list --desk HELP`
- **`jsm request-type get`** (read, multi-ID). Request types by ID or exact name, with their fields and an example answers document (use --full to see them).
  Takes args `IDS...`; flags `--desk (required)`.
  Returns `jsm.request-type`: id, name, description.
  - `atlx jsm request-type get 'Get IT help' --desk HELP --full`

## jsm queue

- **`jsm queue list`** (read, paged). A service desk's queues, with counts.
  Takes flags `--desk (required)`.
  Returns `jsm.queue`: id, name, count.
  - `atlx jsm queue list --desk HELP`

## jsm queue issue

- **`jsm queue issue list`** (read, paged). The issues in a queue.
  Takes flags `--queue (required)`, `--desk (required)`.
  Returns `jira.issue-row`: key, title, type, status, assignee, updated.
  - `atlx jsm queue issue list --queue 3 --desk HELP`

## jsm article

- **`jsm article search`** (read, paged). Knowledge base articles matching text, in one desk or all.
  Takes args `TEXT...`; flags `--desk`.
  Returns `jsm.article`: title, excerpt, url.
  - `atlx jsm article search vpn --desk HELP`

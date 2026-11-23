# Jira commands

<!-- Generated from `atlx describe` (contract 1) by `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->

Covers: `jira`: issues, projects, boards, sprints, filters, dashboards, metadata, users. Every command also takes the global flags (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, `--read-only`, `-p`, ...). "paged" commands take `--limit` (default 25, at most 1000), `--cursor` and `--all`; "multi-ID" commands take several IDs (or `-` for stdin) plus `--strict` and `--concurrency`. `...` marks a repeatable value. "Returns" lists the record's default fields; `atlx describe <path>` has every field, flag type and default.

## jira issue

- **`jira issue list`** (read, paged). Issues matching JQL or filters, newest change first; or the issues of a sprint or a board's backlog.
  Takes flags `--jql`, `--filter`, `--project`, `--status...`, `--assignee`, `--type...`, `--label...`, `--order-by`, `--sprint`, `--backlog`, `--quick-filter...`.
  Returns `jira.issue-row`: key, title, type, status, assignee, updated.
  - `atlx jira issue list --assignee me --status 'In Progress'`
  - `atlx jira issue list --jql 'project = PROJ AND updated >= -7d' --limit 50`
  - `atlx jira issue list --sprint 42 --fields key,title,status`
- **`jira issue search`** (read, paged). Issues whose text matches, newest change first.
  Takes args `TEXT...`; flags `--project`.
  Returns `jira.issue-row`: key, title, type, status, assignee, updated.
  - `atlx jira issue search login timeout --project PROJ`
- **`jira issue get`** (read, multi-ID). Issues by key, ID or URL, with the description as Markdown.
  Takes args `KEYS...`; flags `--body-format markdown|adf`, `--body-offset`.
  Returns `jira.issue`: key, title, type, status, priority, assignee, reporter, project, parent, labels, created, updated, url, description.
  - `atlx jira issue get PROJ-1`
  - `atlx jira issue get PROJ-1 PROJ-2 --full`
- **`jira issue create`** (write). Create an issue, or up to 50 from a file.
  Takes flags `--project`, `--type`, `--summary`, `--description`, `--description-file`, `--body-format markdown|adf|plain`, `--assignee`, `--reporter`, `--priority`, `--label...`, `--parent`, `--field...`, `--fields-json`, `--sprint`, `--story-points`, `--original-estimate`, `--remaining-estimate`, `--team`, `--bulk-file`.
  Returns `jira.issue-change`: key, id, url, updated, sprint.
  - `atlx jira issue create --project PROJ --type Bug --summary 'Login fails' --description-file bug.md`
  - `atlx jira issue create --project PROJ --summary 'Spike' --field 'Story Points=3' --dry-run`
  - `atlx jira issue create --bulk-file issues.json`
- **`jira issue update`** (write). Change an issue's fields.
  Takes args `KEY`; flags `--summary`, `--description`, `--description-file`, `--body-format markdown|adf|plain`, `--allow-lossy`, `-y/--yes`, `--assignee`, `--type`, `--priority`, `--label...`, `--add-label...`, `--remove-label...`, `--parent`, `--due`, `--fix-version...`, `--add-fix-version...`, `--remove-fix-version...`, `--component...`, `--add-component...`, `--remove-component...`, `--resolution`, `--field...`, `--fields-json`, `--sprint`, `--story-points`, `--original-estimate`, `--remaining-estimate`, `--team`.
  Returns `jira.issue-change`: key, id, url, updated, sprint.
  - `atlx jira issue update PROJ-1 --summary 'New title' --add-label backend`
  - `atlx jira issue update PROJ-1 --description-file desc.md --dry-run`
- **`jira issue delete`** (destructive, needs --yes, multi-ID). Delete issues.
  Takes args `KEYS...`; flags `--delete-subtasks`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue delete PROJ-1 --yes`
- **`jira issue transition`** (write, multi-ID). Move issues through their workflow.
  Takes args `KEYS...`; flags `--to (required)`, `--field...`, `--fields-json`, `--comment`, `--body-format markdown|adf|plain`.
  Returns `jira.issue-action`: key, status, transition, assignee, archived.
  - `atlx jira issue transition PROJ-1 --to Done`
  - `atlx jira issue transition PROJ-1 PROJ-2 --to 'In Review' --comment 'Ready'`
- **`jira issue assign`** (write, multi-ID). Assign issues.
  Takes args `KEYS...`; flags `--to (required)`.
  Returns `jira.issue-action`: key, status, transition, assignee, archived.
  - `atlx jira issue assign PROJ-1 --to me`
  - `atlx jira issue assign PROJ-1 --to none`
- **`jira issue archive`** (destructive, needs --yes). Archive issues (Premium and Enterprise, admins).
  Takes args `KEYS...`; flags `-y/--yes`.
  Returns `jira.issue-action`: key, status, transition, assignee, archived.
  - `atlx jira issue archive PROJ-1 PROJ-2 --yes`
- **`jira issue restore`** (destructive, needs --yes). Bring archived issues back.
  Takes args `KEYS...`; flags `-y/--yes`.
  Returns `jira.issue-action`: key, status, transition, assignee, archived.
  - `atlx jira issue restore PROJ-1 --yes`

## jira issue comment

- **`jira issue comment list`** (read, paged). An issue's comments, oldest first.
  Takes flags `--issue (required)`, `--newest-first`, `--body-format markdown|adf`, `--body-offset`.
  Returns `jira.comment`: id, author, created, body.
  - `atlx jira issue comment list --issue PROJ-1 --newest-first --limit 5`
- **`jira issue comment get`** (read, multi-ID). Comments by ID.
  Takes args `IDS...`; flags `--issue (required)`, `--body-format markdown|adf`, `--body-offset`.
  Returns `jira.comment`: id, author, created, body.
  - `atlx jira issue comment get 10001 --issue PROJ-1`
- **`jira issue comment create`** (write). Add a comment.
  Takes flags `--issue (required)`, `--body`, `--body-file`, `--body-format markdown|adf|plain`, `--internal`, `--visibility-role`, `--visibility-group`, `--parent`.
  Returns `jira.comment`: id, author, created, body.
  - `atlx jira issue comment create --issue PROJ-1 --body 'Fixed in **main**'`
  - `atlx jira issue comment create --issue PROJ-1 --body-file note.md --visibility-role Developers`
- **`jira issue comment update`** (write). Replace a comment's body.
  Takes args `ID`; flags `--issue (required)`, `--body`, `--body-file`, `--body-format markdown|adf|plain`, `--allow-lossy`, `-y/--yes`.
  Returns `jira.comment`: id, author, created, body.
  - `atlx jira issue comment update 10001 --issue PROJ-1 --body 'Updated text'`
- **`jira issue comment delete`** (destructive, needs --yes). Delete a comment.
  Takes args `ID`; flags `--issue (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue comment delete 10001 --issue PROJ-1 --yes`

## jira issue worklog

- **`jira issue worklog list`** (read, paged). An issue's worklogs; or, site-wide, the worklogs changed or deleted since a time.
  Takes flags `--issue`, `--started-after`, `--started-before`, `--changed-since`, `--deleted-since`.
  Returns `jira.worklog`: id, author, started, timeSpent, seconds, comment.
  - `atlx jira issue worklog list --issue PROJ-1`
  - `atlx jira issue worklog list --changed-since 7d`
- **`jira issue worklog get`** (read, multi-ID). Worklogs by ID.
  Takes args `IDS...`; flags `--issue (required)`.
  Returns `jira.worklog`: id, author, started, timeSpent, seconds, comment.
  - `atlx jira issue worklog get 10010 --issue PROJ-1`
- **`jira issue worklog create`** (write). Log time on an issue.
  Takes flags `--issue (required)`, `--time-spent`, `--time-spent-seconds`, `--started`, `--comment`, `--body-format markdown|adf|plain`, `--visibility-json`, `--adjust-estimate auto|leave|new|manual`, `--new-estimate`, `--notify-users true|false`, `--override-editable`, `--reduce-by`.
  Returns `jira.worklog`: id, author, started, timeSpent, seconds, comment.
  - `atlx jira issue worklog create --issue PROJ-1 --time-spent '1h 30m' --comment 'Pairing'`
- **`jira issue worklog update`** (write). Change a worklog.
  Takes args `ID`; flags `--issue (required)`, `--time-spent`, `--time-spent-seconds`, `--started`, `--comment`, `--body-format markdown|adf|plain`, `--visibility-json`, `--adjust-estimate auto|leave|new|manual`, `--new-estimate`, `--notify-users true|false`, `--override-editable`, `--reduce-by`.
  Returns `jira.worklog`: id, author, started, timeSpent, seconds, comment.
  - `atlx jira issue worklog update 10010 --issue PROJ-1 --time-spent 2h`
- **`jira issue worklog delete`** (destructive, needs --yes). Delete a worklog.
  Takes args `ID`; flags `--issue (required)`, `--adjust-estimate auto|leave|new|manual`, `--new-estimate`, `--notify-users true|false`, `--override-editable`, `--increase-by`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue worklog delete 10010 --issue PROJ-1 --yes`

## jira issue attachment

- **`jira issue attachment list`** (read, paged). The files attached to an issue.
  Takes flags `--issue (required)`.
  Returns `jira.attachment`: id, filename, size, mimeType, author, created.
  - `atlx jira issue attachment list --issue PROJ-1`
- **`jira issue attachment get`** (read, multi-ID). Attachments' metadata by ID.
  Takes args `IDS...`.
  Returns `jira.attachment`: id, filename, size, mimeType, author, created.
  - `atlx jira issue attachment get 10020`
- **`jira issue attachment download`** (read). Save an attachment (or its thumbnail) to a local file.
  Takes args `ID`; flags `--path`, `--force`, `--thumbnail`.
  Returns `jira.download`: id, path, bytes.
  - `atlx jira issue attachment download 10020 --path ./log.txt`
- **`jira issue attachment upload`** (write). Attach local files to an issue.
  Takes flags `--issue (required)`, `--file... (required)`.
  Returns `jira.attachment`: id, filename, size, mimeType, author, created.
  - `atlx jira issue attachment upload --issue PROJ-1 --file ./screenshot.png`
- **`jira issue attachment delete`** (destructive, needs --yes). Delete an attachment.
  Takes args `ID`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue attachment delete 10020 --yes`

## jira issue link

- **`jira issue link list`** (read, paged). An issue's links to other issues.
  Takes flags `--issue (required)`.
  Returns `jira.link`: id, type, direction, relation, issue, title, status.
  - `atlx jira issue link list --issue PROJ-1`
- **`jira issue link create`** (write). Link an issue to others: `--issue P-1 --target P-2 --type Blocks` means P-1 blocks P-2.
  Takes flags `--issue (required)`, `--target... (required)`, `--type (required)`.
  Returns `jira.link-created`: source, target, type.
  - `atlx jira issue link create --issue PROJ-1 --target PROJ-2 --type Blocks`
- **`jira issue link delete`** (destructive, needs --yes). Delete a link (its ID is in `issue link list`).
  Takes args `ID`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue link delete 10500 --yes`

## jira issue weblink

- **`jira issue weblink list`** (read, paged). An issue's remote links.
  Takes flags `--issue (required)`.
  Returns `jira.weblink`: id, title, url, relationship.
  - `atlx jira issue weblink list --issue PROJ-1`
- **`jira issue weblink create`** (write). Add a remote link.
  Takes flags `--issue (required)`, `--url (required)`, `--title (required)`, `--global-id`, `--relationship`, `--application-name`, `--application-type`, `--summary`, `--icon-url`, `--resolved`.
  Returns `jira.weblink`: id, title, url, relationship.
  - `atlx jira issue weblink create --issue PROJ-1 --url https://example.com/pr/5 --title 'PR 5'`
- **`jira issue weblink delete`** (destructive, needs --yes). Delete a remote link by ID, global ID or URL.
  Takes args `[ID]`; flags `--issue (required)`, `--global-id`, `--url`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue weblink delete 7 --issue PROJ-1 --yes`
  - `atlx jira issue weblink delete --issue PROJ-1 --url https://example.com/pr/5 --yes`

## jira issue watcher

- **`jira issue watcher list`** (read, paged). Who watches an issue.
  Takes flags `--issue (required)`.
  Returns `jira.user`: id, name, email, active.
  - `atlx jira issue watcher list --issue PROJ-1`
- **`jira issue watcher create`** (write). Add a watcher.
  Takes args `[USER]`; flags `--issue (required)`.
  Returns `jira.user`: id, name, email, active.
  - `atlx jira issue watcher create --issue PROJ-1`
- **`jira issue watcher delete`** (destructive, needs --yes). Remove a watcher.
  Takes args `[USER]`; flags `--issue (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue watcher delete me --issue PROJ-1 --yes`

## jira issue vote

- **`jira issue vote get`** (read). The votes on an issue.
  Takes flags `--issue (required)`.
  Returns `jira.votes`: votes, hasVoted, voters.
  - `atlx jira issue vote get --issue PROJ-1`
- **`jira issue vote create`** (write). Vote for an issue as yourself.
  Takes flags `--issue (required)`.
  Returns `jira.votes`: votes, hasVoted, voters.
  - `atlx jira issue vote create --issue PROJ-1`
- **`jira issue vote delete`** (destructive, needs --yes). Take your vote back.
  Takes flags `--issue (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue vote delete --issue PROJ-1 --yes`

## jira issue property

- **`jira issue property list`** (read, paged). The property keys on an issue.
  Takes flags `--issue (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira issue property list --issue PROJ-1`
- **`jira issue property get`** (read). One property's value.
  Takes args `KEY`; flags `--issue (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira issue property get build --issue PROJ-1`
- **`jira issue property update`** (write). Create or replace a property.
  Takes args `KEY`; flags `--issue (required)`, `--value (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira issue property update build --issue PROJ-1 --value '{"ok":true}'`
- **`jira issue property delete`** (destructive, needs --yes). Delete a property.
  Takes args `KEY`; flags `--issue (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira issue property delete build --issue PROJ-1 --yes`

## jira issue changelog

- **`jira issue changelog list`** (read, paged). An issue's change history, oldest first.
  Takes flags `--issue (required)`.
  Returns `jira.change`: id, author, created, items.
  - `atlx jira issue changelog list --issue PROJ-1 --limit 10`

## jira project

- **`jira project list`** (read, paged). Projects, by text, type, category or status; or the projects of a board.
  Takes flags `-q/--query`, `--type`, `--category`, `--status`, `--order-by`, `--board`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project list -q web`
  - `atlx jira project list --board 12`
- **`jira project get`** (read, multi-ID). Projects by key or ID.
  Takes args `KEYS...`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project get PROJ`
- **`jira project create`** (write). Create a project.
  Takes flags `--key (required)`, `--name (required)`, `--type`, `--template`, `--lead`, `--description`, `--url`, `--assignee-type`, `--category`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project create --key NEW --name 'New project' --type software --lead 5b10ac8d82e05b22cc7d4ef5`
- **`jira project update`** (write). Change a project.
  Takes args `KEY`; flags `--name`, `--new-key`, `--lead`, `--description`, `--url`, `--assignee-type`, `--category`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project update PROJ --description 'Web app'`
- **`jira project delete`** (destructive, needs --yes). Move a project to the trash (restorable for 60 days).
  Takes args `KEY`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira project delete PROJ --yes`
- **`jira project archive`** (destructive, needs --yes). Archive a project.
  Takes args `KEY`; flags `-y/--yes`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project archive PROJ --yes`
- **`jira project restore`** (destructive, needs --yes). Bring an archived or trashed project back.
  Takes args `KEY`; flags `-y/--yes`.
  Returns `jira.project`: key, id, name, type, lead.
  - `atlx jira project restore PROJ --yes`

## jira project version

- **`jira project version list`** (read, paged). A project's versions.
  Takes flags `--project (required)`, `-q/--query`, `--status`, `--order-by`.
  Returns `jira.version`: id, name, released, archived, releaseDate.
  - `atlx jira project version list --project PROJ --status unreleased`
- **`jira project version get`** (read, multi-ID). Versions by ID, with issue counts.
  Takes args `IDS...`.
  Returns `jira.version`: id, name, released, archived, releaseDate.
  - `atlx jira project version get 10100`
- **`jira project version create`** (write). Create a version.
  Takes flags `--project (required)`, `--name (required)`, `--description`, `--start-date`, `--release-date`, `--released true|false`, `--archived true|false`.
  Returns `jira.version`: id, name, released, archived, releaseDate.
  - `atlx jira project version create --project PROJ --name 1.2 --release-date 2026-12-01`
- **`jira project version update`** (write). Change a version.
  Takes args `ID`; flags `--name`, `--description`, `--start-date`, `--release-date`, `--released true|false`, `--archived true|false`.
  Returns `jira.version`: id, name, released, archived, releaseDate.
  - `atlx jira project version update 10100 --released true`
- **`jira project version delete`** (destructive, needs --yes). Delete a version.
  Takes args `ID`; flags `--move-fix-issues-to`, `--move-affected-issues-to`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira project version delete 10100 --move-fix-issues-to 10101 --yes`
- **`jira project version merge`** (destructive, needs --yes). Delete a version, moving its issues to another.
  Takes args `ID`; flags `--into (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira project version merge 10100 --into 10101 --yes`
- **`jira project version move`** (write). Reorder a version.
  Takes args `ID`; flags `--after-version`, `--position First|Last|Earlier|Later`.
  Returns `jira.version`: id, name, released, archived, releaseDate.
  - `atlx jira project version move 10100 --position First`

## jira project component

- **`jira project component list`** (read, paged). A project's components.
  Takes flags `--project (required)`.
  Returns `jira.component`: id, name, lead, assigneeType.
  - `atlx jira project component list --project PROJ`
- **`jira project component get`** (read, multi-ID). Components by ID.
  Takes args `IDS...`.
  Returns `jira.component`: id, name, lead, assigneeType.
  - `atlx jira project component get 10200`
- **`jira project component create`** (write). Create a component.
  Takes flags `--project (required)`, `--name (required)`, `--description`, `--lead`, `--assignee-type`.
  Returns `jira.component`: id, name, lead, assigneeType.
  - `atlx jira project component create --project PROJ --name API`
- **`jira project component update`** (write). Change a component.
  Takes args `ID`; flags `--name`, `--description`, `--lead`, `--assignee-type`.
  Returns `jira.component`: id, name, lead, assigneeType.
  - `atlx jira project component update 10200 --description 'Public API'`
- **`jira project component delete`** (destructive, needs --yes). Delete a component.
  Takes args `ID`; flags `--move-issues-to`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira project component delete 10200 --yes`

## jira board

- **`jira board list`** (read, paged). Agile boards, by project, type, name or filter.
  Takes flags `--project`, `--type scrum|kanban`, `-q/--query`, `--filter`.
  Returns `jira.board`: id, name, type, project.
  - `atlx jira board list --project PROJ`
- **`jira board get`** (read, multi-ID). Boards by ID, URL or exact name, with columns and estimation.
  Takes args `IDS...`.
  Returns `jira.board`: id, name, type, project.
  - `atlx jira board get 12`
- **`jira board create`** (write). Create a board on a filter.
  Takes flags `--name (required)`, `--type scrum|kanban (required)`, `--filter (required)`.
  Returns `jira.board`: id, name, type, project.
  - `atlx jira board create --name Team --type scrum --filter 10100`
- **`jira board delete`** (destructive, needs --yes). Delete a board.
  Takes args `ID`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira board delete 12 --yes`

## jira board quick-filter

- **`jira board quick-filter list`** (read, paged). A board's quick filters.
  Takes flags `--board (required)`.
  Returns `jira.quick-filter`: id, name, jql.
  - `atlx jira board quick-filter list --board 12`
- **`jira board quick-filter get`** (read, multi-ID). Quick filters by ID.
  Takes args `IDS...`; flags `--board (required)`.
  Returns `jira.quick-filter`: id, name, jql.
  - `atlx jira board quick-filter get 3 --board 12`

## jira sprint

- **`jira sprint list`** (read, paged). A board's sprints.
  Takes flags `--board`, `--project`, `--issue`, `--state`, `--current`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint list --board 12 --state active,future`
  - `atlx jira sprint list --project PROJ --current`
- **`jira sprint get`** (read, multi-ID). Sprints by ID, ARI or URL.
  Takes args `IDS...`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint get 42`
- **`jira sprint create`** (write). Create a future sprint on a board.
  Takes flags `--board (required)`, `--name`, `--goal`, `--start-date`, `--end-date`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint create --board 12 --name 'Sprint 5' --goal 'Ship login'`
- **`jira sprint update`** (write). Change some of a sprint's fields.
  Takes args `ID`; flags `--board`, `--name`, `--goal`, `--start-date`, `--end-date`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint update 42 --goal 'Ship login'`
- **`jira sprint start`** (write). Start a sprint; it needs --end-date or --duration unless it has an end.
  Takes args `ID`; flags `--board`, `--name`, `--goal`, `--start-date`, `--end-date`, `--duration`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint start 42 --duration 2w`
- **`jira sprint complete`** (write). Complete an active sprint.
  Takes args `ID`; flags `--board`, `--name`, `--goal`, `--start-date`, `--end-date`, `--move-open-to`.
  Returns `jira.sprint`: id, name, state, start, end, goal.
  - `atlx jira sprint complete 42 --move-open-to backlog`
- **`jira sprint delete`** (destructive, needs --yes). Delete a sprint.
  Takes args `ID`; flags `--board`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira sprint delete 42 --yes`

## jira filter

- **`jira filter list`** (read, paged). Saved filters, by name, owner, group or project; or your favourites.
  Takes flags `-q/--query`, `--owner`, `--group`, `--project`, `--order-by`, `--override-share-permissions`, `--favourite`.
  Returns `jira.filter`: id, name, jql, owner, favourite.
  - `atlx jira filter list -q bugs`
  - `atlx jira filter list --favourite`
- **`jira filter get`** (read, multi-ID). Filters by ID, URL or exact name.
  Takes args `IDS...`.
  Returns `jira.filter`: id, name, jql, owner, favourite.
  - `atlx jira filter get 10100`
- **`jira filter create`** (write). Create a filter.
  Takes flags `--name (required)`, `--jql (required)`, `--description`, `--favourite true|false`.
  Returns `jira.filter`: id, name, jql, owner, favourite.
  - `atlx jira filter create --name 'My bugs' --jql 'assignee = currentUser() AND type = Bug'`
- **`jira filter update`** (write). Change a filter: name, JQL, description, favourite, owner.
  Takes args `ID`; flags `--name`, `--jql`, `--description`, `--favourite true|false`, `--owner`.
  Returns `jira.filter`: id, name, jql, owner, favourite.
  - `atlx jira filter update 10100 --jql 'project = PROJ'`
- **`jira filter delete`** (destructive, needs --yes). Delete a filter.
  Takes args `ID`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira filter delete 10100 --yes`

## jira filter share

- **`jira filter share list`** (read, paged). A filter's share permissions.
  Takes flags `--filter (required)`.
  Returns `jira.share`: id, type, project, role, group, user.
  - `atlx jira filter share list --filter 10100`
- **`jira filter share create`** (destructive, needs --yes). Share a filter: a REST share permission such as {"type":"group","groupname":"dev"}.
  Takes flags `--filter (required)`, `--permission-json (required)`, `-y/--yes`.
  Returns `jira.share`: id, type, project, role, group, user.
  - `atlx jira filter share create --filter 10100 --permission-json '{"type":"group","groupname":"dev"}' --yes`
- **`jira filter share delete`** (destructive, needs --yes). Remove a share permission.
  Takes args `ID`; flags `--filter (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira filter share delete 3 --filter 10100 --yes`

## jira filter column

- **`jira filter column list`** (read, paged). A filter's columns.
  Takes flags `--filter (required)`.
  Returns `jira.column`: label, value.
  - `atlx jira filter column list --filter 10100`
- **`jira filter column update`** (write). Set a filter's columns.
  Takes flags `--filter (required)`, `--column... (required)`.
  Returns `jira.column`: label, value.
  - `atlx jira filter column update --filter 10100 --column issuekey --column summary`
- **`jira filter column delete`** (destructive, needs --yes). Reset a filter's columns to the default.
  Takes flags `--filter (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira filter column delete --filter 10100 --yes`

## jira filter subscription

- **`jira filter subscription list`** (read, paged). A filter's subscriptions.
  Takes flags `--filter (required)`.
  Returns `jira.subscription`: id, user, group.
  - `atlx jira filter subscription list --filter 10100`

## jira dashboard

- **`jira dashboard list`** (read, paged). Dashboards, by name, owner, group or project.
  Takes flags `-q/--query`, `--owner`, `--group`, `--project`, `--order-by`.
  Returns `jira.dashboard`: id, name, owner, favourite.
  - `atlx jira dashboard list -q team`
- **`jira dashboard get`** (read, multi-ID). Dashboards by ID.
  Takes args `IDS...`.
  Returns `jira.dashboard`: id, name, owner, favourite.
  - `atlx jira dashboard get 10000`
- **`jira dashboard create`** (write). Create a dashboard (private unless permissions are given).
  Takes flags `--name`, `--description`, `--share-permissions-json`, `--edit-permissions-json`, `--extend-admin-permissions`.
  Returns `jira.dashboard`: id, name, owner, favourite.
  - `atlx jira dashboard create --name 'Team board'`
- **`jira dashboard update`** (destructive, needs --yes). Change a dashboard's name, description or permissions (the update replaces its permissions, so it needs --yes).
  Takes args `ID`; flags `--name`, `--description`, `--share-permissions-json`, `--edit-permissions-json`, `--extend-admin-permissions`, `-y/--yes`.
  Returns `jira.dashboard`: id, name, owner, favourite.
  - `atlx jira dashboard update 10000 --name 'Team' --yes`
- **`jira dashboard copy`** (write). Copy a dashboard.
  Takes args `ID`; flags `--name`, `--description`, `--share-permissions-json`, `--edit-permissions-json`, `--extend-admin-permissions`.
  Returns `jira.dashboard`: id, name, owner, favourite.
  - `atlx jira dashboard copy 10000 --name 'Team (copy)'`
- **`jira dashboard delete`** (destructive, needs --yes). Delete a dashboard.
  Takes args `ID`; flags `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira dashboard delete 10000 --yes`

## jira dashboard gadget

- **`jira dashboard gadget list`** (read, paged). A dashboard's gadgets, or (with --catalog) those that can be added.
  Takes flags `--dashboard`, `--catalog`, `--module-key...`, `--uri...`.
  Returns `jira.gadget`: id, title, moduleKey, uri, color, position.
  - `atlx jira dashboard gadget list --dashboard 10000`
  - `atlx jira dashboard gadget list --catalog`
- **`jira dashboard gadget create`** (write). Add a gadget by module key or URI.
  Takes flags `--dashboard (required)`, `--module-key`, `--uri`, `--skip-validation`, `--gadget-json`, `--title`, `--color`, `--row`, `--column`.
  Returns `jira.gadget`: id, title, moduleKey, uri, color, position.
  - `atlx jira dashboard gadget create --dashboard 10000 --module-key com.example:gadget --row 0 --column 0`
- **`jira dashboard gadget update`** (write). Change a gadget's title, colour or position.
  Takes args `ID`; flags `--dashboard (required)`, `--gadget-json`, `--title`, `--color`, `--row`, `--column`.
  Returns `jira.gadget`: id, title, moduleKey, uri, color, position.
  - `atlx jira dashboard gadget update 5 --dashboard 10000 --title 'Open bugs'`
- **`jira dashboard gadget delete`** (destructive, needs --yes). Remove a gadget.
  Takes args `ID`; flags `--dashboard (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira dashboard gadget delete 5 --dashboard 10000 --yes`

## jira dashboard property

- **`jira dashboard property list`** (read, paged). An item's property keys.
  Takes flags `--dashboard (required)`, `--item (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira dashboard property list --dashboard 10000 --item 5`
- **`jira dashboard property get`** (read). One property's value.
  Takes args `KEY`; flags `--dashboard (required)`, `--item (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira dashboard property get config --dashboard 10000 --item 5`
- **`jira dashboard property update`** (write). Create or replace a property.
  Takes args `KEY`; flags `--dashboard (required)`, `--item (required)`, `--value (required)`.
  Returns `jira.property`: key, value.
  - `atlx jira dashboard property update config --dashboard 10000 --item 5 --value '{"rows":10}'`
- **`jira dashboard property delete`** (destructive, needs --yes). Delete a property.
  Takes args `KEY`; flags `--dashboard (required)`, `--item (required)`, `-y/--yes`.
  Returns `jira.deleted`: id, deleted, issue.
  - `atlx jira dashboard property delete config --dashboard 10000 --item 5 --yes`

## jira field

- **`jira field list`** (read, paged). Fields: the site's, a create screen's (--project, --type) or an issue's edit screen (--issue), with what --field values they take.
  Takes flags `-q/--query`, `--custom`, `--project`, `--type`, `--issue`.
  Returns `jira.field`: id, name, type, custom, required, acceptedInput.
  - `atlx jira field list -q story --custom`
  - `atlx jira field list --project PROJ --type Bug`
  - `atlx jira field list --issue PROJ-1`

## jira issue-type

- **`jira issue-type list`** (read, paged). Issue types: the site's, or a project's.
  Takes flags `--project`, `--level`.
  Returns `jira.issue-type`: id, name, subtask, level.
  - `atlx jira issue-type list --project PROJ`
- **`jira issue-type get`** (read, multi-ID). Issue types by ID or ARI.
  Takes args `IDS...`.
  Returns `jira.issue-type`: id, name, subtask, level.
  - `atlx jira issue-type get 10001`

## jira status

- **`jira status list`** (read, paged). Statuses: the site's, or each status a project uses.
  Takes flags `--project`.
  Returns `jira.status`: id, name, category.
  - `atlx jira status list --project PROJ`
- **`jira status get`** (read, multi-ID). Statuses by ID or ARI.
  Takes args `IDS...`.
  Returns `jira.status`: id, name, category.
  - `atlx jira status get 3`

## jira priority

- **`jira priority list`** (read, paged). The site's priorities.
  Returns `jira.priority`: id, name.
  - `atlx jira priority list`
- **`jira priority get`** (read, multi-ID). Priorities by ID or ARI.
  Takes args `IDS...`.
  Returns `jira.priority`: id, name.
  - `atlx jira priority get 1`

## jira link-type

- **`jira link-type list`** (read, paged). The site's issue link types.
  Returns `jira.link-type`: id, name, inward, outward.
  - `atlx jira link-type list`

## jira transition

- **`jira transition list`** (read, paged). The transitions an issue can make now, with the fields they require.
  Takes flags `--issue (required)`.
  Returns `jira.transition`: id, name, to, category.
  - `atlx jira transition list --issue PROJ-1`

## jira user

- **`jira user search`** (read, paged). Accounts whose name or email matches.
  Takes args `TEXT...`.
  Returns `jira.user`: id, name, email, active.
  - `atlx jira user search alice`
- **`jira user get`** (read, multi-ID). Accounts by `me`, account ID, identity ARI or exact email.
  Takes args `USERS...`.
  Returns `jira.user`: id, name, email, active.
  - `atlx jira user get me`
  - `atlx jira user get alice@example.com`

# twg compatibility

**Goal: atlx behaves exactly like twg** for Jira, Confluence, Bitbucket, users
and Jira Service Management. That means the same command paths, positional
arguments and flag names, the same `{"data": ...}` JSON envelope and key names,
and no confirmation gates where twg has none (`--yes` stays accepted as a
no-op). Commands that need Atlassian's Teamwork Graph, the GraphQL gateway or
Rovo are out of scope; atlx is built only on the public REST APIs. These rules
come from the maintainer decisions on
[#18](https://github.com/nexburst/atlx/issues/18) and
[#21](https://github.com/nexburst/atlx/issues/21).

This page compares atlx with **twg 1.4.4**. The twg side comes from
[`compat/twg-1.4.4.txt`](compat/twg-1.4.4.txt), made by
[`scripts/twg-inventory.sh`](scripts/twg-inventory.sh), which records command
paths, positionals and flag names only. Coverage per product is in
[`compat/coverage.txt`](compat/coverage.txt).

## Status values

| Status | Meaning |
| --- | --- |
| ✅ | Supported under twg's path, with all of twg's flags that apply |
| 🟡 | Supported under twg's path, but flags or behavior are missing (listed, with the issue) |
| 🔁 | Accepted under twg's path through an alias of a differently named atlx command |
| ➕ | atlx only; twg 1.4.4 has no such command |
| 📋 | Planned; the note links the issue. `#13 (unscheduled)` means public REST covers it but no issue is open yet |
| ⛔ | Out of scope: needs Teamwork Graph, the GraphQL gateway, Rovo or Atlassian AI, or is specific to twg's OAuth setup |

Flags that only feed twg's GraphQL transport (`--variables-json`,
`--cloud-id`, `--*-input`, ...) are listed as "not planned" and do not make a
row 🟡. Bitbucket's `--workspace`/`--repo`, which twg declares on the
`bitbucket` group, are accepted on every atlx Bitbucket command.

## How this page is kept accurate

`cargo test --test twg_compat` runs offline on every CI platform and checks:

- **This table against the parser.** Every twg leaf command in the inventory
  needs a row. ✅ and 🟡 commands must exist in
  [`tests/snapshots/cli-tree.txt`](tests/snapshots/cli-tree.txt), 🔁 commands
  must exist only through an alias, and 📋 and ⛔ commands must not parse.
- **The parse corpus**, [`compat/corpus.txt`](compat/corpus.txt): twg-style
  invocations tagged `ok` (must parse) or `gap #<issue>` (must fail to parse).
  When a change makes a gap line parse, the test fails until the line is
  flipped to `ok` and the row here is updated.
- **Coverage**, which may only go up. Record a rise with
  `UPDATE_COVERAGE=1 cargo test --test twg_compat`.

These checks are about parsing. Output shapes are covered by
[`tests/twg_shapes.rs`](tests/twg_shapes.rs) and the
[JSON output shapes](#json-output-shapes) section below.

## Global flags and behavior

| twg | atlx | Notes |
| --- | --- | --- |
| `-o, --output text\|json\|jsonl` | ✅ | atlx defaults to JSON when stdout is piped |
| `-s, --site` | ✅ | |
| `-u, --user` | ✅ | |
| `--timeout-ms` | ✅ | |
| `-p, --profile` | ✅ | named profiles ([#20](https://github.com/nexburst/atlx/issues/20)) |
| `--select`, `--agent-fields`, `--output-summary` | ✅ | output controls ([#19](https://github.com/nexburst/atlx/issues/19)); see the README |
| `--api-version` | ✅ | `v2` is accepted and other versions are rejected ([#19](https://github.com/nexburst/atlx/issues/19)) |
| `--output-file` | ➕ | twg 1.4.4 lists no such global flag; atlx writes stdout to a file |
| `--usage-counts` | ⛔ | usage counts come from the GraphQL gateway |
| `--output-shape` | ⛔ | the shapes reshape Teamwork Graph relationships |
| `{"data": ...}` JSON envelope and key names | ✅ | `apiVersion`, `command`, `request`, `data`, `pageInfo`, `meta` as twg prints them; Bitbucket unwrapped, as in twg ([#21](https://github.com/nexburst/atlx/issues/21)); see [JSON output shapes](#json-output-shapes) |
| JSONL rows with `_twg` locators | ✅ | one line per element of the main collection ([#21](https://github.com/nexburst/atlx/issues/21)) |
| Teamwork Graph, Rovo and GraphQL-only commands | ⛔ | fail with "not supported by atlx (requires Teamwork Graph)" in twg's error envelope, `category: "unsupported"`, exit code 2; 📋 commands fail with a pointer to their issue ([#21](https://github.com/nexburst/atlx/issues/21)) |
| Exit codes and the JSON error envelope | ✅ | [#17](https://github.com/nexburst/atlx/issues/17) |
| No `--yes` on merge, decline and content delete | ✅ | `--yes` is still accepted and ignored; nothing asks for confirmation ([#18](https://github.com/nexburst/atlx/issues/18)) |
| API-token auth | ➕ | twg uses OAuth; atlx uses Atlassian and Bitbucket API tokens |

## JSON output shapes

Maintainer decision on [#21](https://github.com/nexburst/atlx/issues/21):
**atlx behaves as twg.** The default JSON and JSONL output uses twg's
envelope and key names; there is no separate compatibility mode. Commands
that need Teamwork Graph or Rovo fail with "not supported by atlx (requires
Teamwork Graph)".

`scripts/conformance.sh` runs read-only commands through twg 1.4.4 and atlx
against the same site and reports every key path only one side prints. The
run for #21 used a site with Jira but no Confluence and no Bitbucket
repositories, so:

| Area | How the shape was established | Result |
| --- | --- | --- |
| `whoami`, `user search` (with and without `--compact`) | Live side by side | Same key paths. `whoami` data comes from Jira's `myself`: `zoneinfo` is the Jira time zone (twg prints `null`) and `extendedProfile` is all `null`; profile, site and token source are in `meta` |
| Jira `workitem get` (default, `--comments`, `--fields`, `--full`, several IDs), `workitem query` (rows, `--fields`, paging), `workitem search`, `transitions query`, `field create-metadata`, `link-types query`, `space query`, `space get`, `board query` | Live side by side | Same key paths and types. Differences in values only: `self` links use the site URL (twg goes through `api.atlassian.com`), and `meta` describes atlx's REST source instead of twg's backend |
| Jira `board sprints query`, `sprint workitems query` | Inferred: the site has no sprints. Follows the pattern twg uses for the other Agile lists (REST page, `values` renamed `sprints`; work item rows as in `workitem query`) | Not verified |
| Jira `workitem comment query`, `changelog query` (default, `--first`, `--after`, `--order-by`), `field update-metadata` (with and without `--include-system-fields`), `types query` (`--project`, `--issue --level -n`, `--id`), `types get`, `statuses query`, `priorities query` (all, and by `--id`) | Live side by side ([#23](https://github.com/nexburst/atlx/issues/23)) | Same key paths, and the same `data` values once the `self` host is normalised. Comment and changelog pages print `pageInfo` on the last page too (`hasNextPage: false`, `nextCursor: null`); `types query` adds top-level `warnings` when `--limit` cut the list or `--id` was used |
| Jira `workitem attachment`, `worklog`, `watcher`, `vote` | Inferred, not run live ([#24](https://github.com/nexburst/atlx/issues/24)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Jira `sprint` lifecycle, `board get\|create\|delete\|backlog\|projects\|quick-filter` | Inferred, not run live ([#25](https://github.com/nexburst/atlx/issues/25)) | Built from the public Agile 1.0 payloads and tested against mock responses only; key paths have not been compared with twg |
| Jira `space` administration, `filter` | Inferred, not run live ([#26](https://github.com/nexburst/atlx/issues/26)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Confluence `content versions`, `history`, `labels add\|remove\|watch\|unwatch`, `attachments`, `body-formats`, `get --format url` | Inferred, not run live ([#28](https://github.com/nexburst/atlx/issues/28)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Confluence `content comments` threads | Inferred, not run live ([#29](https://github.com/nexburst/atlx/issues/29)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Confluence content and space lifecycle, space `tree` | Inferred, not run live ([#30](https://github.com/nexburst/atlx/issues/30)) | Built from the public REST payloads and tested against mock responses only; the width and emoji property values are unverified too |
| Bitbucket pull request lifecycle, `inbox` | Inferred, not run live ([#32](https://github.com/nexburst/atlx/issues/32)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Bitbucket `pipeline` actions, `deployment` | Inferred, not run live ([#34](https://github.com/nexburst/atlx/issues/34)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Bitbucket branch and commit writes, workspaces, repository extras | Inferred, not run live ([#33](https://github.com/nexburst/atlx/issues/33)) | Built from the public REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Jira Service Management `jsm` | Inferred, not run live ([#37](https://github.com/nexburst/atlx/issues/37)) | Built from the public Service Desk REST payloads and tested against mock responses only; key paths have not been compared with twg |
| Jira `workitem link query` | Live, but the site's work items have no links, so only the empty list was compared. With links, the REST `issuelinks` entries are inferred | Partly verified |
| Jira writes (`create`, `update`, `comment create`, `transition`, `link workitem`, `link weblink`, `comment update`, `comment delete`, `delete`, `unlink workitem`, `unlink weblink`) | Inferred (the run was read-only): REST responses with camelCase atlx additions (`url`, `commentId`, ...); deletes print `{id, deleted: true}`-style objects | Not verified |
| Confluence `content get`, `search query`, `search text` | Inferred from twg's `--select` preset expansions (see `src/output/select.rs`): `id`, `type`, `title`, `status`, `spaceId`, `externalUrl`, `detail`, `snapshotToken`, `summary.{excerpt,wordCount,sectionCount}`, `outline[].{level,text,anchor}`, `body.{format,value}`; search passes REST's `results` through with `totalSize`, `size`, `nextCursor` | Not verified; `aiSummary` and `body.lossyConversion` are not produced. atlx adds `outline[].localId` and `outline[].wordCount`, and `body.sections` for `--section` reads. The `anchor` spelling (whitespace runs to `-`) is a guess |
| Confluence `content update --edits` | Inferred from twg's help: the dry run's `data.body.value` holds the resulting HTML | Not verified; atlx's dry run also has `fromVersion`, `request` and a unified `diff` |
| Other Confluence commands | Inferred: REST v2 objects with `externalUrl` and `snapshotToken`, lists as `{results, nextCursor}` | Not verified |
| Bitbucket `pull-requests comment ...`, `task ...`, `diffstat`, `commits`, `diff-line` (#31) | Inferred; no Bitbucket workspace was available. REST 2.0 comment, task, diffstat and commit objects, lists bare as in `task query`. Deletes and `comment reopen`, which Bitbucket answers with 204, print `{id, pullRequest, deleted: true}` or `{id, pullRequest, resolved: false}`. `diff-line` prints a list of `{path, type, line, fromLine, text, hunk}` (plus `oldPath` on renames): `line` goes to `--line`, `fromLine` to `--from-line` | Not verified |
| Bitbucket | Partly live: twg printed `repo query` as a bare list and a failed batch `pull-requests get` as `{workspace, repository, items, summary, partial}`, both without an envelope. The rest is inferred from the presets: REST 2.0 objects plus `_comments`, `_statuses`, `_repo` and the step log fields | Not verified with data. One known difference: when every ID of a Bitbucket batch fails, twg still prints the batch object (exit 1) while atlx prints the error envelope (exit 1) |
| Account commands (`login`, `logout`, `auth profile ...`, `setup ...`, `doctor`) and `atlx search` | Envelope only. twg's data here describes its OAuth token store, which atlx doesn't have, so the data keeps atlx's own keys | Intentional |
| Errors | The error envelope from #17; JSONL error lines get `_twg: {collection: "root", index: 0}` as in twg | Live |

### Confluence targeted edits

`content update --edits <json>` and `--edits-file <path>` (`-` reads stdin)
take a JSON array of operations in twg's format: each has a `name`, the
target node's ADF `localId` and, where it adds content, an HTML `value`.
atlx reads the page with `body-format=atlas_doc_format`, applies every
operation to a copy of the ADF document, and sends the result back as
`atlas_doc_format` with the version number plus one. Nodes the operations
don't touch are sent back unchanged. If any operation fails (unknown
`localId`, two nodes with the same ID, a bad argument) the command exits
with a usage error and writes nothing. `--snapshot-token` is checked as for
full-body updates, and `--dry-run` prints the resulting HTML and a unified
diff without a PUT.

| Operation | Fields | Notes |
| --- | --- | --- |
| replaceNode | `localId`, `value` | The new nodes take the target's shape: inline content for an inline node, list items for a list item, rows for a row |
| insertNodeBefore, insertNodeAfter | `localId`, `value` | Same shaping as `replaceNode` |
| deleteNode | `localId` | |
| removeNodes | `arguments.localIds` | All IDs must exist |
| moveNode | `arguments.{localId, targetLocalId, position}` | `position` is `before` or `after` (default); twg's other positions, if any, are unknown |
| appendNodeToEnd | `value` | Appended to the end of the document |
| setAttrs | `localId`, `arguments.attrs` | Merged into the node's `attrs` |

Fields may sit at the top level or under `arguments`. Node `localId`s come
from `content get --detail outline` (headings) or the atlx extra
`content get --detail full --format adf` (every node). In twg the IDs come
from a full read; atlx's `--format html` is the storage format, which doesn't
carry ADF IDs, hence `adf`. The operation list is taken from twg's help and
has not been checked against a live site.

atlx extras, for pages whose nodes lack `localId`s, use `op` (or `name`) and
work on top-level headings. A heading query matches case-insensitively,
exact text first, and must select one heading:

| Operation | Fields |
| --- | --- |
| replaceText | `find`, `replace`, `count` (a number, default 1, or `"all"`). The match must sit inside one text run; no match, or more than `count`, is an error |
| replaceSection | `heading`, `value`, `format` (`html`, default, or `md`). Keeps the heading |
| appendToSection, prependToSection | `heading`, `value`, `format` |
| deleteSection | `heading`. Removes the heading too |
| insertSectionAfter | `heading`, `newHeading`, `level` (default: the anchor heading's), `value`, `format` |

`tests/twg_shapes.rs` replays the live-verified commands against synthetic
fixtures (`tests/fixtures/twg/`, made by `scripts/twg-fixtures.sh`, values
replaced and key names kept) so the shapes can't drift. Re-run the
conformance script on a site with Confluence and Bitbucket data to verify the
inferred rows.

## Jira

161 twg commands: ✅ 99, 🟡 1, 📋 51, ⛔ 10.

| twg command | atlx | Notes |
| --- | --- | --- |
| `jira board backlog query` | ✅ | #25; `--jql` and `--limit` are atlx extras; GraphQL-only `--variables-json` not planned |
| `jira board backlog-view query` | ⛔ | board view model is only in the GraphQL gateway |
| `jira board cells query` | ⛔ | board view model is only in the GraphQL gateway |
| `jira board create` | ✅ | #25; GraphQL-only `--variables-json` not planned |
| `jira board delete` | ✅ | #25; GraphQL-only `--variables-json` not planned |
| `jira board get` | ✅ | #25; the board plus its `configuration`; GraphQL-only `--variables-json` not planned |
| `jira board projects query` | ✅ | #25; GraphQL-only `--variables-json` not planned |
| `jira board query` | ✅ | #18; `--issue` uses the work item's space; GraphQL-only `--variables-json` not planned |
| `jira board quick-filter get` | ✅ | #25; GraphQL-only `--variables-json` not planned |
| `jira board quick-filter query` | ✅ | #25; GraphQL-only `--variables-json` not planned |
| `jira board scope query` | ⛔ | board scope model is only in the GraphQL gateway |
| `jira board sprints query` | ✅ | #18; `--project`/`--issue` use the space's only board; GraphQL-only `--variables-json` not planned |
| `jira board view-settings query` | ⛔ | board view model is only in the GraphQL gateway |
| `jira dashboard bulk-edit` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard copy` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard create` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard delete` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard gadget add` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard gadget catalog query` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard gadget delete` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard gadget query` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard gadget update` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard get` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard item-property delete` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard item-property get` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard item-property query` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard item-property set` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard query` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira dashboard update` | 📋 | #13 (unscheduled); REST v3 dashboards |
| `jira field cancel-delete` | 📋 | #13 (unscheduled); REST v3 custom field admin |
| `jira field create` | 📋 | #13 (unscheduled); REST v3 custom field admin |
| `jira field delete` | 📋 | #13 (unscheduled); REST v3 custom field admin |
| `jira field update` | 📋 | #13 (unscheduled); REST v3 custom field admin |
| `jira filter add-favourite` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter change-owner` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter columns get` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter columns set` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter create` | ✅ | #26; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira filter delete` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter get` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter query` | ✅ | #26; `--favourite` is an atlx extra; `--owner` is resolved to an account ID; GraphQL-only `--variables-json` not planned |
| `jira filter remove-favourite` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter reset-columns` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter share add` | ✅ | #26; `loggedin` is sent as `authenticated` (#65); GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira filter share query` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter share remove` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter subscription query` | ✅ | #26; GraphQL-only `--variables-json` not planned |
| `jira filter update` | ✅ | #26; reads the filter and sends unchanged fields back; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira space archive` | ✅ | #26; takes only a key, ID, ARI or /projects/ URL, never a name; GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space component counts` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space component create` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space component delete` | ✅ | #26; no confirmation, as in twg; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space component get` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space component query` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space component update` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space create` | ✅ | #26; GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space delete` | ✅ | #26; moves it to the trash; takes only a key, ID, ARI or /projects/ URL, never a name; GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space get` | ✅ | #18; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space issue-types` | ✅ | #26 |
| `jira space notification-scheme get` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space query` | ✅ | #18; `--after` takes a numeric offset (REST pages by number); GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space restore` | ✅ | #26; takes only a key, ID, ARI or /projects/ URL, never a name; GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space status query` | ✅ | #26 |
| `jira space types query` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space update` | ✅ | #26; GraphQL-only `--cloud-id`, `--input-json`, `--variables-json` not planned |
| `jira space version counts` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version create` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version delete` | ✅ | #26; uses `removeAndSwap`; no confirmation, as in twg; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version get` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version merge` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version move` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space version update` | ✅ | #26; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira space versions query` | ✅ | #26; `--after` is an offset; GraphQL-only `--cloud-id`, `--variables-json` not planned |
| `jira sprint complete` | ✅ | #25; moves unfinished work (not subtasks) first when given a destination; `--move-open-to` is an alias; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira sprint create` | ✅ | #25; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira sprint delete` | ✅ | #25; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira sprint get` | ✅ | #25; `--id` is an atlx extra; GraphQL-only `--variables-json` not planned |
| `jira sprint snapshot` | 📋 | #13; a computed report with no REST endpoint of its own |
| `jira sprint start` | ✅ | #25; the start defaults to now and the end is required, or `--duration` (atlx extra); GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira sprint update` | ✅ | #25; partial update; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira sprint workitems query` | ✅ | #18; GraphQL-only `--variables-json` not planned |
| `jira workitem archive` | 📋 | #13 (unscheduled); REST v3 issue archival |
| `jira workitem attachment delete` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem attachment download` | ✅ | #24; `--out` and `--force` are atlx extras; unsafe server file names need `--output-path`; GraphQL-only `--variables-json` not planned |
| `jira workitem attachment get` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem attachment query` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem attachment thumbnail` | ✅ | #24; `--out` and `--force` are atlx extras; GraphQL-only `--variables-json` not planned |
| `jira workitem attachment upload` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem bulk-get` | 📋 | #13 (unscheduled); REST v3 bulkfetch |
| `jira workitem bulk-transition` | 📋 | #13 (unscheduled); REST v3 bulk transition |
| `jira workitem changelog query` | ✅ | #23; one REST page per call, `--after` takes the previous page's `pageInfo.nextCursor`; GraphQL-only `--variables-json` not planned |
| `jira workitem clone` | 📋 | #13 (unscheduled); read then create |
| `jira workitem comment create` | ✅ | `--parent-id` sends REST `parentId` (#23; inferred, not checked live); GraphQL-only `--variables-json` not planned |
| `jira workitem comment delete` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem comment query` | ✅ | #23; one REST page per call, `--after` takes the previous page's `pageInfo.nextCursor`; GraphQL-only `--variables-json` not planned |
| `jira workitem comment update` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem create` | ✅ | GraphQL-only `--variables-json` not planned |
| `jira workitem create-bulk` | 📋 | #13 (unscheduled); REST v3 bulk create |
| `jira workitem delete` | ✅ | #23; REST v3 issue delete, no confirmation as in twg; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem field create-metadata` | ✅ | #18; also reachable as `jira workitem custom-fields create-metadata` |
| `jira workitem field update-metadata` | ✅ | #23; REST v3 editmeta, custom fields unless `--include-system-fields` |
| `jira workitem get` | ✅ | `--batch-concurrency` fetches in parallel and keeps input order (#18); GraphQL-only `--variables-json` not planned |
| `jira workitem link artifact` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link branch` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link build` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link commit` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link deployment` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link goal` | ⛔ | Atlassian Goals link (GraphQL gateway) |
| `jira workitem link loom` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link meeting` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link page` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link pr` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link project` | ⛔ | Atlassian Projects link (GraphQL gateway) |
| `jira workitem link query` | ✅ | #23; the work item's REST `issuelinks`; GraphQL-only `--variables-json` not planned |
| `jira workitem link repo` | 📋 | #13 (unscheduled); a remote link (REST v3 remotelink) |
| `jira workitem link weblink` | ✅ | #18 |
| `jira workitem link workitem` | ✅ | #18; `--id` is the side that reads with the outward phrase (`--id A --target-id B --link-type-id Blocks` means A blocks B), sent as REST `inwardIssue` |
| `jira workitem link-types query` | ✅ | #18; REST returns every link type at once, so the GraphQL paging flags `--after`, `--before`, `--first`, `--last` and `--cloud-id`, `--variables-json` are not planned |
| `jira workitem priorities query` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem project-link-candidates query` | ⛔ | Atlassian Projects relationship (Teamwork Graph) |
| `jira workitem property delete` | 📋 | #13 (unscheduled); REST v3 issue properties |
| `jira workitem property get` | 📋 | #13 (unscheduled); REST v3 issue properties |
| `jira workitem property query` | 📋 | #13 (unscheduled); REST v3 issue properties |
| `jira workitem property set` | 📋 | #13 (unscheduled); REST v3 issue properties |
| `jira workitem query` | 🟡 | `--before` and `--last` parse but return a usage error: REST search only pages forward (#18); GraphQL-only `--cloud-id`, `--field-sets-input`, `--issue-search-input`, `--namespace`, `--options`, `--save-jqlto-user-history`, `--scope`, `--variables-json`, `--view-config-input` not planned |
| `jira workitem search` | ✅ |  |
| `jira workitem similar` | ⛔ | semantic similarity needs Atlassian AI |
| `jira workitem statuses query` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem transition` | ✅ |  |
| `jira workitem transitions query` | ✅ |  |
| `jira workitem types get` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem types query` | ✅ | #23; REST v3 `issuetype/project`; `--id` looks types up as `types get` does; GraphQL-only `--variables-json` not planned |
| `jira workitem unarchive` | 📋 | #13 (unscheduled); REST v3 issue archival |
| `jira workitem unlink artifact` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink branch` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink build` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink commit` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink deployment` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink goal` | ⛔ | Atlassian Goals link (GraphQL gateway) |
| `jira workitem unlink loom` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink meeting` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink page` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink pr` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink project` | ⛔ | Atlassian Projects link (GraphQL gateway) |
| `jira workitem unlink repo` | 📋 | #13 (unscheduled); delete a remote link (REST v3 remotelink) |
| `jira workitem unlink weblink` | ✅ | #23; `--url` deletes the one remote link pointing there |
| `jira workitem unlink workitem` | ✅ | #23; GraphQL-only `--variables-json` not planned |
| `jira workitem update` | ✅ | #25 added sprint, story points, estimates and team; GraphQL-only `--variables-json` not planned |
| `jira workitem vote add` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem vote query` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem vote remove` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem watcher add` | ✅ | #24; `--id` is the work item; `--issue-id` is an alias; GraphQL-only `--variables-json` not planned |
| `jira workitem watcher query` | ✅ | #24; GraphQL-only `--variables-json` not planned |
| `jira workitem watcher remove` | ✅ | #24; `--id` is the work item; `--issue-id` is an alias; GraphQL-only `--variables-json` not planned |
| `jira workitem worklog add` | ✅ | #24; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog changed` | ✅ | #24; one page; pass `until` as the next `--since`; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog delete` | ✅ | #24; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog deleted` | ✅ | #24; one page; pass `until` as the next `--since`; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog get` | ✅ | #24; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog query` | ✅ | #24; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira workitem worklog update` | ✅ | #24; GraphQL-only `--input-json`, `--variables-json` not planned |
| `jira board list` | ➕ | old name of `jira board query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira field list` | ➕ | list fields with ids; twg's `jira field` is admin CRUD |
| `jira project get` | ➕ | old name of `jira space get` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira project list` | ➕ | old name of `jira space query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira sprint issues` | ➕ | old name of `jira sprint workitems query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira sprint list` | ➕ | old name of `jira board sprints query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira workitem create-meta` | ➕ | old name of `jira workitem field create-metadata` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira workitem link create` | ➕ | old name of `jira workitem link workitem` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira workitem link remote` | ➕ | old name of `jira workitem link weblink` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `jira workitem link types` | ➕ | old name of `jira workitem link-types query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |

## Confluence

121 twg commands: ✅ 40, 🟡 9, 📋 35, ⛔ 37.

| twg command | atlx | Notes |
| --- | --- | --- |
| `confluence approvals settings site get` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence approvals settings site set` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence approvals settings space get` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence approvals settings space set` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content analytics` | 📋 | #13 (unscheduled); v1 analytics views/viewers |
| `confluence content approvals approve` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals cancel` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals clear` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals due-date clear` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals due-date set` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals enabled` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals get` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals history` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals message set` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals pending` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals request` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals request-changes` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content approvals reviewers update` | ⛔ | page approvals are only in the GraphQL gateway |
| `confluence content archive` | ✅ | #30; pages; `--wait` is an atlx extra |
| `confluence content attachments delete` | ✅ | `--purge` is an atlx extra |
| `confluence content attachments download` | ✅ | without `--out`, hidden or otherwise unsafe server file names are refused |
| `confluence content attachments get` | ✅ | |
| `confluence content attachments list` | ✅ | |
| `confluence content attachments upload` | ✅ | repeatable `--file` and `--minor-edit` are atlx extras |
| `confluence content body-formats` | ✅ | atlx's own guide; `--reference` is rejected (it has no linked documents) |
| `confluence content comments create` | ✅ | #29; inline with `--text-selection`; the match count is counted when not given. The old positional `<ref>` and `--format` still work (hidden, deprecated) |
| `confluence content comments delete` | ✅ | #29 |
| `confluence content comments get` | ✅ | #29; up to 500 replies, then `repliesTruncated` |
| `confluence content comments list` | 🟡 | #13: lists footer comments unless `--comment-type inline` (twg detects both), and REST filters `--resolution-status` on inline comments only. The old positional `<ref>`, `--inline` and `--format` still work (hidden, deprecated) |
| `confluence content comments reopen` | 🟡 | #13: inline comments only; REST has no resolution for footer comments |
| `confluence content comments reply` | ✅ | #29 |
| `confluence content comments resolve` | 🟡 | #13: inline comments only; REST has no resolution for footer comments |
| `confluence content comments update` | ✅ | #29 |
| `confluence content convert` | 📋 | #13; v1 contentbody/convert |
| `confluence content copy` | ✅ | #30; the `--copy-*` flags are atlx extras |
| `confluence content create` | 🟡 | missing `--synced-container-url` (#13); folders and other types are not created. Width and emoji are content properties set after creating |
| `confluence content delete-draft` | 🟡 | #13: only drafts that were never published; refuses otherwise |
| `confluence content disable-public-link` | ⛔ | public links are only in the GraphQL gateway |
| `confluence content enable-public-link` | ⛔ | public links are only in the GraphQL gateway |
| `confluence content export` | ⛔ | no public PDF/Word export endpoint |
| `confluence content export-status` | ⛔ | no public PDF/Word export endpoint |
| `confluence content get` | ✅ | `--detail ai_summary` is not offered (Atlassian AI); `--format adf` is an atlx extra that shows node `localId`s for `--edits` |
| `confluence content get-public-link` | ⛔ | public links are only in the GraphQL gateway |
| `confluence content history diff` | ✅ | same as `versions diff` |
| `confluence content history get` | ✅ | same as `versions get` |
| `confluence content history list` | ✅ | same as `versions list` |
| `confluence content labels add` | ✅ | |
| `confluence content labels list` | 🟡 | #65: pages, blog posts and custom content; public REST has no endpoint for whiteboard, database or folder labels |
| `confluence content labels remove` | ✅ | |
| `confluence content labels unwatch` | ✅ | |
| `confluence content labels watch` | ✅ | |
| `confluence content list` | ✅ |  |
| `confluence content list-available-statuses` | 📋 | #13 (unscheduled); v1 content states |
| `confluence content macros resolve` | 📋 | #13 (unscheduled); v1 async body conversion, to confirm |
| `confluence content macros resolve-status` | 📋 | #13 (unscheduled); v1 async body conversion, to confirm |
| `confluence content move` | ✅ | #30; `--target` with `--position` is an atlx extra |
| `confluence content permissions add` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content permissions check` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content permissions clear` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content permissions list` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content permissions remove` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content permissions replace` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content publish` | ✅ | #30 |
| `confluence content purge` | 🟡 | #13: pages and blog posts only; checks that the content is in the trash |
| `confluence content reactions add` | ⛔ | reactions are only in the GraphQL gateway |
| `confluence content reactions list` | ⛔ | reactions are only in the GraphQL gateway |
| `confluence content reactions remove` | ⛔ | reactions are only in the GraphQL gateway |
| `confluence content restriction-state get` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content restriction-state set` | 📋 | #13 (unscheduled); v1 content restrictions |
| `confluence content set-owner` | 📋 | #13 (unscheduled); public endpoint to confirm |
| `confluence content star` | ⛔ | starring is only in the GraphQL gateway |
| `confluence content status get` | 📋 | #13 (unscheduled); v1 content states |
| `confluence content status set` | 📋 | #13 (unscheduled); v1 content states |
| `confluence content tasks complete` | 📋 | #13 (unscheduled); v2 tasks |
| `confluence content tasks get` | 📋 | #13 (unscheduled); v2 tasks |
| `confluence content tasks list` | 📋 | #13 (unscheduled); v2 tasks |
| `confluence content tasks reopen` | 📋 | #13 (unscheduled); v2 tasks |
| `confluence content trash` | 🟡 | #13: pages and blog posts only |
| `confluence content unarchive` | ⛔ | no public REST endpoint |
| `confluence content unstar` | ⛔ | starring is only in the GraphQL gateway |
| `confluence content untrash` | 🟡 | #65: pages and blog posts only (REST v2) |
| `confluence content unwatch` | 📋 | #13 (unscheduled); v1 user/watch |
| `confluence content update` | 🟡 | missing `--content-width`, `--header-image-file`, `--header-image-file-id`, `--header-image-position`, `--header-image-url`, `--reference-space`, `--remove-header-image` (#13). `--minor-edit` is sent but absent from the v2 request schema, so Confluence may ignore it (#65). `--context-content`/`--context-content-file` are not planned: they only apply to whiteboard and database bodies, which public REST doesn't return. `--edits`/`--edits-file` take twg's operations (see [Confluence targeted edits](#confluence-targeted-edits)) |
| `confluence content versions diff` | ✅ | computed locally from the two bodies |
| `confluence content versions get` | ✅ | |
| `confluence content versions list` | ✅ | |
| `confluence content versions restore` | ✅ | v1 restore operation |
| `confluence content watch` | 📋 | #13 (unscheduled); v1 user/watch |
| `confluence group get` | 📋 | #13 (unscheduled); v1 groups |
| `confluence remix infographic create` | ⛔ | AI generation (Atlassian AI) |
| `confluence remix infographic edit` | ⛔ | AI generation (Atlassian AI) |
| `confluence remix maui create` | ⛔ | AI generation (Atlassian AI) |
| `confluence remix maui edit` | ⛔ | AI generation (Atlassian AI) |
| `confluence remix maui get` | ⛔ | AI generation (Atlassian AI) |
| `confluence search query` | ✅ |  |
| `confluence search text` | ✅ |  |
| `confluence space archive` | ✅ | #30 |
| `confluence space create` | 🟡 | missing `--role-assignments-file` (#13) |
| `confluence space delete` | ✅ | #30; permanent; takes only a key, ID, ARI or space URL, never a name or a content URL |
| `confluence space get` | ✅ | `--labels` adds a `labels` array (#18) |
| `confluence space instructions get` | ⛔ | Rovo space instructions |
| `confluence space instructions set` | ⛔ | Rovo space instructions |
| `confluence space list` | ✅ | `--labels` adds a `labels` array to each space (#18) |
| `confluence space me` | ✅ |  |
| `confluence space permission available` | 📋 | #13 (unscheduled); v2 space permissions |
| `confluence space permission list` | 📋 | #13 (unscheduled); v2 space permissions |
| `confluence space role assignment list` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space role create` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space role get` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space role list` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space role mode` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space role update` | 📋 | #13 (unscheduled); v2 space roles |
| `confluence space star` | ⛔ | starring is only in the GraphQL gateway |
| `confluence space unarchive` | ✅ | #30 |
| `confluence space unstar` | ⛔ | starring is only in the GraphQL gateway |
| `confluence space unwatch` | 📋 | #13 (unscheduled); v1 user/watch |
| `confluence space update` | ✅ | #30; `--homepage` is an atlx extra |
| `confluence space watch` | 📋 | #13 (unscheduled); v1 user/watch |
| `confluence templates get` | 📋 | #13 (unscheduled); v1 templates |
| `confluence templates list` | 📋 | #13 (unscheduled); v1 templates |
| `confluence tree` | ✅ | #30; pages or whole spaces, depth 10, at most 500 nodes then `truncated`; `--type` keeps the containers of matches |
| `confluence content children` | ➕ | direct children of a page |
| `confluence content delete` | ➕ | twg 1.4.4 splits this into `trash` and `purge`, which need `--yes` (#30); here no confirmation is asked and `--yes` is ignored (#18) |
| `confluence content labels` | ➕ | old form `labels <ref>` of `confluence content labels list --id <ref>` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |

## Bitbucket

78 twg commands: ✅ 66, 🟡 9, 📋 3.

| twg command | atlx | Notes |
| --- | --- | --- |
| `bitbucket branch create` | ✅ | #33 |
| `bitbucket branch delete` | ✅ | #33 |
| `bitbucket branch query` | ✅ | #18; `--author` is matched by atlx over up to 1000 branches, since Bitbucket can't filter by author |
| `bitbucket commit create` | ✅ | #33; `--file repoPath=localPath` is an atlx extra; a branch must be named, or `--default-branch` |
| `bitbucket commit query` | ✅ | #33; `--include`, `--exclude`, `--path` are atlx extras |
| `bitbucket commit report create` | ✅ | #33 |
| `bitbucket commit report query` | ✅ | #33 |
| `bitbucket deployment commits query` | ✅ | #34; compared with the previous successful deployment among up to 5000 |
| `bitbucket deployment environment variable create` | 📋 | #13; twg reads the value from the clipboard |
| `bitbucket deployment environment variable delete` | 📋 | #13 |
| `bitbucket deployment environment variable query` | ✅ | #34; secured values are never shown; `--environment` is an atlx extra |
| `bitbucket deployment environment variable update` | 📋 | #13; twg reads the value from the clipboard |
| `bitbucket deployment environments query` | ✅ | #34 |
| `bitbucket deployment get` | ✅ | #34 |
| `bitbucket deployment history get` | ✅ | #34 |
| `bitbucket deployment history query` | ✅ | #34; filtered and sorted client-side over up to 5000 deployments |
| `bitbucket deployment query` | ✅ | #34; `--environment` is an atlx extra |
| `bitbucket inbox` | ✅ | #32; scans the most recently updated repositories (`--repo-limit`, 25), one request or more each; `--role all` is an atlx extra |
| `bitbucket pipeline config get` | ✅ | #34 |
| `bitbucket pipeline config update` | ✅ | #34 |
| `bitbucket pipeline get` | ✅ | #34 added test reports and cases |
| `bitbucket pipeline grep` | 🟡 | missing `--regex` (#13): plain substring matching, no regex dependency |
| `bitbucket pipeline latest-failure` | ✅ | #34; searches the 100 most recent runs |
| `bitbucket pipeline query` | ✅ | #34 added `--pattern` |
| `bitbucket pipeline rerun-failed` | 🟡 | #13: REST has no rerun of failed steps only, so this starts the whole pipeline again and needs `--whole-pipeline` (atlx extra) to say so |
| `bitbucket pipeline run` | ✅ | #34; `--secured-variable KEY` reads the environment, as in twg |
| `bitbucket pipeline tail` | ✅ | #34; polls (`--no-ws` is a no-op); follows every step unless `--step`; exits 1 when the pipeline fails |
| `bitbucket pipeline trigger` | ✅ | #34; same as `run` |
| `bitbucket pull-requests activity` | ✅ | takes several ids as a batch, with `--strict` and `--batch-concurrency` (#18) |
| `bitbucket pull-requests approve` | ✅ |  |
| `bitbucket pull-requests comment create` | 🟡 | missing `--image`, `--image-name` (#13 (unscheduled); upload through Downloads). `--reply-to`, `--pending` and the inline `--path`/`--line`/`--from-line`/range flags work (#31) |
| `bitbucket pull-requests comment delete` | ✅ | needs `--yes`, as in twg (#31) |
| `bitbucket pull-requests comment get` | ✅ | #31 |
| `bitbucket pull-requests comment query` | ✅ | deleted comments are left out, oldest first (#31) |
| `bitbucket pull-requests comment reopen` | ✅ | #31 |
| `bitbucket pull-requests comment resolve` | ✅ | #31 |
| `bitbucket pull-requests comment update` | ✅ | #31 |
| `bitbucket pull-requests commits` | ✅ | #31 |
| `bitbucket pull-requests conflicts` | ✅ | #32; from the diffstat |
| `bitbucket pull-requests create` | 🟡 | missing `--image`, `--image-name` (#13); `--default-reviewers` is an atlx extra |
| `bitbucket pull-requests decline` | ✅ | no confirmation is asked; `--yes` is ignored (#18) |
| `bitbucket pull-requests default-reviewer add` | ✅ | #32 |
| `bitbucket pull-requests default-reviewer get` | ✅ | #32 |
| `bitbucket pull-requests default-reviewer query` | ✅ | #32 |
| `bitbucket pull-requests default-reviewer remove` | ✅ | #32 |
| `bitbucket pull-requests diff` | ✅ |  |
| `bitbucket pull-requests diff-line` | ✅ | atlx finds the matches in the REST diff (#31); output shape inferred, see [JSON output shapes](#json-output-shapes) |
| `bitbucket pull-requests diffstat` | ✅ | #31 |
| `bitbucket pull-requests effective-default-reviewer query` | ✅ | #32 |
| `bitbucket pull-requests for-commit` | ✅ | #32; a repository still being indexed gives an error saying so |
| `bitbucket pull-requests get` | ✅ | `--batch-concurrency` fetches in parallel and keeps input order (#18) |
| `bitbucket pull-requests merge` | ✅ | #32; `--wait` polls the merge task; no confirmation is asked. The old positional id, `--strategy`, `--message` and `--yes` still work (hidden, deprecated) |
| `bitbucket pull-requests merge-status` | ✅ | #32 |
| `bitbucket pull-requests mergeability` | 🟡 | #13: computed from public endpoints; commits-behind and branching-model rules are reported as `unchecked` |
| `bitbucket pull-requests patch` | ✅ | raw text, as `diff` (#31) |
| `bitbucket pull-requests query` | 🟡 | #13: `--scope me` scans recent repositories and takes only `--state`, `--role`, `--limit`; merge dates are refined from each candidate's activity (200 at most) |
| `bitbucket pull-requests remove-request-changes` | ✅ | #32 |
| `bitbucket pull-requests request-changes` | ✅ |  |
| `bitbucket pull-requests task create` | ✅ | #31 |
| `bitbucket pull-requests task delete` | ✅ | needs `--yes`, as in twg (#31) |
| `bitbucket pull-requests task get` | ✅ | #31 |
| `bitbucket pull-requests task query` | ✅ | #18 |
| `bitbucket pull-requests task reopen` | ✅ | #31 |
| `bitbucket pull-requests task resolve` | ✅ | #31 |
| `bitbucket pull-requests task update` | ✅ | #31 |
| `bitbucket pull-requests unapprove` | ✅ |  |
| `bitbucket pull-requests update` | 🟡 | missing `--image`, `--image-name` (#13) |
| `bitbucket repo contributors` | 🟡 | missing `--repo-path` (#13): counts through REST over 1000 commits, where twg reads local git |
| `bitbucket repo create` | ✅ | #33; private unless `--public` |
| `bitbucket repo file` | ✅ |  |
| `bitbucket repo get` | ✅ |  |
| `bitbucket repo list-files` | ✅ | #33; `--max-depth` is an atlx extra |
| `bitbucket repo query` | ✅ | `--repo workspace/slug` or a URL selects the workspace (#18) |
| `bitbucket repo url` | ✅ | #33; offline; `--path`, `--ref`, `--line` are atlx extras |
| `bitbucket search prs` | 🟡 | #13: `--scope me` scans the 25 most recently updated repositories and matches the text client-side |
| `bitbucket workspace get` | ✅ | #33 |
| `bitbucket workspace member get` | ✅ | #33 |
| `bitbucket workspace member query` | ✅ | #33; `-q` is an atlx extra |
| `bitbucket pull-requests tasks` | ➕ | old name of `bb pull-requests task query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `bitbucket repo branches` | ➕ | old name of `bb branch query` (#18); hidden; prints a deprecation note in text mode on a terminal; removed in 0.3 |
| `bitbucket search code` | ➕ | Bitbucket `codesearch` (#66; needs `repository:read` and `workspace:read`; searches default branches); twg's `search-code` uses Code Intelligence |

## Users

7 twg commands: ✅ 1, 📋 2, ⛔ 4.

| twg command | atlx | Notes |
| --- | --- | --- |
| `user bulk-lookup` | 📋 | #13 (unscheduled); REST v3 user/bulk |
| `user direct-reports` | ⛔ | reporting lines come from the Teamwork Graph |
| `user get` | 📋 | #13 (unscheduled); REST v3 user / myself |
| `user manager` | ⛔ | reporting lines come from the Teamwork Graph |
| `user manager-chain` | ⛔ | reporting lines come from the Teamwork Graph |
| `user peers` | ⛔ | reporting lines come from the Teamwork Graph |
| `user search` | ✅ | `--compact` is accepted; the JSON is the same either way, as in twg (#21). `--name`, `--query` and `--email` all use Jira's user search, so `sources` is always `["jira-rest"]` (#18); GraphQL-only `--include-slack` not planned |

## Accounts and setup

15 twg commands: ✅ 10, ⛔ 5.

| twg command | atlx | Notes |
| --- | --- | --- |
| `auth profile default` | ✅ |  |
| `auth profile list` | ✅ |  |
| `auth profile migrate` | ✅ |  |
| `auth profile rename` | ✅ |  |
| `auth refresh` | ⛔ | refreshes twg OAuth tokens; atlx uses API tokens |
| `auth storage migrate` | ⛔ | manages twg's encrypted token store; atlx keeps tokens in its config |
| `auth storage reset` | ⛔ | manages twg's encrypted token store; atlx keeps tokens in its config |
| `auth storage rotate-key` | ⛔ | manages twg's encrypted token store; atlx keeps tokens in its config |
| `auth storage status` | ⛔ | manages twg's encrypted token store; atlx keeps tokens in its config |
| `doctor` | ✅ |  |
| `login` | ✅ | `--env` selects a twg environment and does not apply; atlx adds `--token`, `--token-cmd` |
| `logout` | ✅ |  |
| `setup bitbucket` | ✅ |  |
| `setup default-site` | ✅ |  |
| `whoami` | ✅ |  |

## Jira Service Management

121 twg commands: ✅ 14, 🟡 4, 📋 47, ⛔ 56.

| twg command | atlx | Notes |
| --- | --- | --- |
| `jsm agents availability get` | ⛔ | agent availability is only in the GraphQL gateway |
| `jsm agents availability list` | ⛔ | agent availability is only in the GraphQL gateway |
| `jsm agents availability set` | ⛔ | agent availability is only in the GraphQL gateway |
| `jsm agents list` | ⛔ | agent availability is only in the GraphQL gateway |
| `jsm alert acknowledge` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert assign` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert close` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert create` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert delete` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert escalate` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert get` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert note create` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert note query` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert query` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert request get` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert snooze` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert unacknowledge` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm alert update` | 📋 | #13 (unscheduled); JSM Ops alerts REST API |
| `jsm approval approve` | ✅ | #37 |
| `jsm approval get` | ✅ | #37 |
| `jsm approval list` | ✅ | #37 |
| `jsm approval reject` | ✅ | #37 |
| `jsm conversation claim create` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation close create` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation message query` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation query` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation settings query` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation settings update` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation workspace create` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm conversation workspace query` | ⛔ | virtual agent conversations (GraphQL gateway) |
| `jsm form answers list` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form attach` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form describe` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form fields list` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form fill` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form get` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form list` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form reopen` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm form submit` | 📋 | #13 (unscheduled); Jira Forms REST API |
| `jsm help-article query` | ⛔ | help center articles (GraphQL gateway) |
| `jsm help-center config get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center create` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center customer-experience list` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center get-by-project` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center hub-media-config get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center layout-translations list` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center list` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center list-basic` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center list-by-project` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center media-config get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center page get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center page list` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center permission-settings get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center permissions get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center product-entities query` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center reporting get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-center topic get` | ⛔ | help centers are only in the GraphQL gateway |
| `jsm help-object-store create` | ⛔ | help center object store (GraphQL gateway) |
| `jsm help-object-store query` | ⛔ | help center object store (GraphQL gateway) |
| `jsm incident affected-service query` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident affected-services query` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident alert query` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident create` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident delete` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident get` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident link affected-service` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident link alert` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident query` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident responder add` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident responder query` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident responder remove` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident transition` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident unlink affected-service` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident unlink alert` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm incident update` | 📋 | #13 (unscheduled); JSM Ops and Jira REST |
| `jsm knowledge-app-link query` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm knowledge-article query` | 🟡 | #13: REST can only search, so `--query` (atlx extra) is required |
| `jsm knowledge-base create` | ⛔ | knowledge configuration (GraphQL gateway) |
| `jsm knowledge-base query` | ⛔ | knowledge configuration (GraphQL gateway) |
| `jsm knowledge-base search query` | ⛔ | knowledge configuration (GraphQL gateway) |
| `jsm knowledge-capability query` | ⛔ | knowledge configuration (GraphQL gateway) |
| `jsm knowledge-discovery create` | ⛔ | Rovo knowledge discovery |
| `jsm knowledge-discovery query` | ⛔ | Rovo knowledge discovery |
| `jsm knowledge-permission bulk query` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm knowledge-source-type get` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source link` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source permission update` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source query` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source suggestion query` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source unlink` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm linked-source view update` | ⛔ | knowledge connectors (GraphQL gateway) |
| `jsm portal query` | 🟡 | #13: service desks from REST; `--help-center-ari` is GraphQL-only; `--query-term` filters client-side |
| `jsm post-incident-review create` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review delete` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review get` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review incident get` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review link incident` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review query` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review unlink incident` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm post-incident-review update` | 📋 | #13 (unscheduled); PIRs are Jira work items |
| `jsm request comment create` | ✅ | #37; public unless `--internal`, as in twg |
| `jsm request comment list` | ✅ | #37 |
| `jsm request create` | ✅ | #37 |
| `jsm request participant add` | ✅ | #37 |
| `jsm request participant list` | ✅ | #37 |
| `jsm request participant remove` | ✅ | #37 |
| `jsm request validate` | ✅ | #37; checks against the request type's fields without creating anything |
| `jsm request-type field get` | ✅ | #37 |
| `jsm request-type fields get` | ✅ | #37 |
| `jsm request-type form get` | 🟡 | #13: built from the request type's REST fields, so no Forms layout or conditions |
| `jsm request-type query` | 🟡 | #13: `--help-center-ari` is GraphQL-only; `--service-desk` is an alias of `--portal-id` |
| `jsm resolution-state get` | ⛔ | resolution states (GraphQL gateway) |
| `jsm service get` | ⛔ | service registry (GraphQL gateway) |
| `jsm service query` | ⛔ | service registry (GraphQL gateway) |
| `jsm service search` | ⛔ | service registry (GraphQL gateway) |
| `jsm service-tier query` | ⛔ | service registry (GraphQL gateway) |
| `jsm sla get` | ✅ | #37 |
| `jsm sla metrics` | ⛔ | SLA reporting is only in the GraphQL gateway |
| `jsm sla workitems` | ⛔ | SLA reporting is only in the GraphQL gateway |
| `jsm support-site-article query` | ⛔ | help center articles (GraphQL gateway) |

## atlx extras

| twg command | atlx | Notes |
| --- | --- | --- |
| `api` | ➕ | raw REST call |
| `completion script` | ➕ | shell completions |
| `man` | ➕ | man page |
| `search` | ➕ | Jira, Confluence and Bitbucket in one call |
| `jsm servicedesk query` | ➕ | service desks; the same list as `jsm portal query` (#37) |
| `jsm request get` | ➕ | one customer request (#37) |
| `jsm request query` | ➕ | your requests, by status, desk and type (#37) |
| `jsm request transition query` | ➕ | customer transitions (#37) |
| `jsm request transition create` | ➕ | makes one (#37) |
| `jsm queue query` | ➕ | agent queues (#37) |
| `jsm queue workitems query` | ➕ | work items in a queue (#37) |
| `skill install` | ➕ | installs the agent skill in `skills/atlx` (#36) |
| `skill print` | ➕ | prints it (#36) |

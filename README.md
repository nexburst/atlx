# atlx

A command line for **Jira**, **Jira Service Management**, **Confluence** and
**Bitbucket** Cloud, built for coding agents and usable by people.

An agent driving a shell pays for every token it reads, cannot see a screen
and cannot ask a follow-up question. So every atlx command returns a small,
typed, predictable record by default; every command can describe its own
inputs, outputs and risks as JSON; every failure says what class it is and
what to do next, in the exit code as well as the body; and no write loses
data silently: bodies go through one checked Markdown/ADF pipeline that
refuses what it cannot represent, every write can be previewed, and the whole
tool can be switched to read-only. People get the same commands with text
tables.

```sh
atlx jira issue list --assignee me --status 'In Progress'
atlx confluence page get 12345 --section Rollback
atlx bb pr checks 42 --repo acme/api
atlx describe jira issue transition
```

## Scope

- **Independent.** atlx is an independent open-source project. It is not
  affiliated with or endorsed by Atlassian, and it designs its own command
  grammar, output and error contract.
- **Public REST APIs only**: Jira REST v3 and Agile 1.0, Jira Service
  Management, Confluence v2 (v1 where v2 has no endpoint) and Bitbucket 2.0.
  Features that need Teamwork Graph, the GraphQL gateway or Rovo are out of
  scope; `search` covers cross-product search over REST, and `api`
  reaches any other REST endpoint.
- **API tokens** with HTTP Basic auth. There is no OAuth flow.

## Install

No release has been published yet. Install from source with a Rust
toolchain:

```sh
git clone https://github.com/nexburst/atlx && cd atlx
cargo install --path . --locked
```

Shell completions: `atlx completion script bash` prints a bash completion
script (also `zsh`, `fish`, `powershell` and `elvish`). For example:

```sh
mkdir -p ~/.local/share/bash-completion/completions
atlx completion script bash > ~/.local/share/bash-completion/completions/atlx
```

Release archives will ship prebuilt binaries for Linux, macOS and Windows
together with completions and man pages.

## Authenticate

Create an API token at
<https://id.atlassian.com/manage-profile/security/api-tokens>, then:

```sh
atlx auth login -s acme -u you@acme.com --token -    # paste the token on stdin
atlx auth status
```

For Bitbucket, create an API token with Bitbucket scopes (read scopes for
account, repositories, pull requests and pipelines; write scopes for what you
want to change):

```sh
atlx auth bitbucket --workspace acme --token -
```

`atlx auth status` is the first command an agent should run: it reports the
profile, site, account, where the token comes from, which products answer and
whether read-only mode is on. atlx never prompts an agent: without
credentials a command fails with `config` (exit 4) and a hint to ask the
person to run `atlx auth login`. `atlx doctor` runs deeper checks.

The config lives in `~/.config/atlx/config.toml` (mode 0600). Environment
variables override it: `ATLX_SITE`, `ATLX_EMAIL`, `ATLX_TOKEN`,
`ATLX_TOKEN_CMD`, `ATLX_BB_WORKSPACE`, `ATLX_BB_EMAIL`, `ATLX_BB_TOKEN`,
`ATLX_BB_TOKEN_CMD`, `ATLX_PROFILE`, `ATLX_CONFIG`, `ATLX_OUTPUT`,
`ATLX_READ_ONLY`, `ATLX_NO_CACHE`.

### Profiles

Several sites or workspaces live side by side as named profiles, chosen with
`-p/--profile <name>` or `ATLX_PROFILE`:

```sh
atlx auth login --new-profile sandbox -s acme-sandbox -u you@acme.com --token -
atlx auth bitbucket -p sandbox --workspace acme-sandbox --token -
atlx -p sandbox jira issue get SBX-1
atlx auth profile list                  # name, site, email, workspace, token source
atlx auth profile default sandbox       # used when -p is not given
atlx auth profile rename sandbox sbx
atlx auth logout -p sbx                 # remove one profile
```

```toml
default_profile = "work"

[profiles.work]
site = "work.atlassian.net"
email = "you@work.com"
token_cmd = "op read op://Private/atlassian/credential"
read_only = true                     # refuse every write with this profile

[profiles.work.bitbucket]
workspace = "work"
token_cmd = "pass show bitbucket/work"
```

Precedence, highest first: `-s`/`-u` flags, `ATLX_*` variables, the selected
profile (`-p`, `ATLX_PROFILE`, the `profile` in `.atlx.toml`, then the saved
default). A v1 config with top-level `site`, `email` and `token` keeps
working as the profile `default`; `atlx auth profile migrate --yes` moves it
into a named profile.

**Credential helpers.** Instead of a token, save a command that prints one:
`token_cmd` in the config, `atlx auth login --token-cmd '<cmd>'`, or
`ATLX_TOKEN_CMD`/`ATLX_BB_TOKEN_CMD`. It runs through `sh -c` (`cmd /C` on
Windows) only when a request is about to be made, at most once per run, with
a 10 second timeout. Its output is never shown; `auth status` and
`auth profile list` report only where the token comes from.

### Repository defaults: `.atlx.toml`

An optional `.atlx.toml`, found by walking up from the working directory,
holds non-secret defaults for the repository you are in:

```toml
profile = "work"
[jira]
project = "PROJ"        # the project for `jira issue create`
[confluence]
space = "ENG"
[bitbucket]
repo = "acme/api"       # --repo for every Bitbucket repository command
```

Only these keys are accepted; tokens, sites and emails are rejected. Without
`bitbucket.repo`, `--repo` defaults to the `origin` remote in `.git/config`
when it points at bitbucket.org. `atlx auth status --full` shows the defaults
in effect. (`confluence.space` is read and reported, but no command uses it
as a default yet.)

## Commands

The grammar is `atlx <product> <resource> [<sub-resource>] <verb>`, with the
verb last:

- **Nouns** are the REST API's own, singular: `jira issue`, `jira project`,
  `confluence page`, `bb pr`.
- **Standard verbs** have one meaning each: `list` (enumerate or filter, with
  `--jql`, `--cql`, `-q` and structured filters), `search` (free text), `get`
  (one or more IDs; `-` reads IDs from stdin), `create`, `update`, `delete`.
  Other verbs are REST actions: `transition`, `assign`, `merge`, `decline`,
  `approve`, `start`, `complete`, `run`, `log`, `edit`, `download`,
  `upload`, and a few more.
- **The target is positional**; its parents are named flags: `--issue`,
  `--page`, `--pr`, `--project`, `--space`, `--board`, `--sprint`, `--desk`,
  `--request`, `--repo workspace/slug`.

| Product | Resources (sub-resources) |
| --- | --- |
| `jira` | `issue` (`comment`, `worklog`, `attachment`, `link`, `weblink`, `watcher`, `vote`, `property`, `changelog`), `project` (`version`, `component`), `board` (`quick-filter`), `sprint`, `filter` (`share`, `column`, `subscription`), `dashboard` (`gadget`, `property`), `field`, `issue-type`, `status`, `priority`, `link-type`, `transition`, `user` |
| `jsm` | `request` (`comment`, `participant`, `approval`, `sla`), `transition`, `desk`, `request-type`, `queue` (`issue`), `article` |
| `confluence` | `search`, `page` (`comment`, `attachment`, `label`, `version`, `task`, `state`), `space` (`permission`), `template`, `label` |
| `bitbucket` (alias `bb`) | `repo` (`file`, `branch`, `commit`, `contributor`, `report`, `default-reviewer`), `pr` (`comment`, `task`, `reviewer`), `pipeline` (`step`, `config`), `deployment`, `environment` (`variable`), `workspace` (`member`) |
| top level | `auth`, `doctor`, `search`, `api`, `describe`, `cache`, `completion`, `skill` |

A few examples per product:

```sh
# Jira
atlx jira issue list --jql 'project = PROJ AND updated >= -7d' --limit 50
atlx jira issue get PROJ-1 PROJ-2 --fields key,title,status
atlx jira issue create --project PROJ --type Bug --summary 'Login fails' --description-file bug.md
atlx jira issue transition PROJ-1 --to Done
atlx jira issue comment create --issue PROJ-1 --body 'Fixed in **main**'
atlx jira field list --project PROJ --type Bug       # what a create needs
atlx jira sprint list --project PROJ --current

# Jira Service Management
atlx jsm request list --status open --desk HELP
atlx jsm request comment create --request HELP-12 --body 'Agent note' --internal --yes

# Confluence
atlx confluence search --cql 'type = page AND space = ENG AND title ~ "release"'
atlx confluence page get 12345 --outline
atlx confluence page update 12345 --if-version 7 --body-file page.md --dry-run
atlx confluence page comment list --page 12345 --inline --resolution open

# Bitbucket
atlx bb pr list --mine --waiting
atlx bb pr diff 42 --grep validate_token
atlx bb pr merge 42 --strategy squash --if-head 3f2a9c1b7e44 --yes
atlx bb pipeline log --failed --branch main

# Everything else
atlx search 'login timeout' --limit 5
atlx api jira /rest/api/3/myself
```

The full list, with every flag, is in `atlx describe` and in the generated
skill references ([skills/atlx/references](skills/atlx/references)).

### `atlx describe`

The CLI documents itself as JSON, generated from the same tree and record
registry that run the commands, so it cannot drift:

```sh
atlx describe                      # every command: path, summary, effect
atlx describe jira                 # one product
atlx describe jira issue list      # one command's full contract
atlx describe --all                # every command's full contract
atlx describe --records            # record types and their fields
atlx describe --errors             # error codes and exit codes
```

A command's contract lists its positionals and flags (type, required,
default, allowed values, environment variable), its `effect` (`read`,
`write` or `destructive`), whether it needs `--yes` (`confirm`), the record
it returns with each field's type and whether it is in the default view,
examples, and the exit codes it can return. The top-level `contract` number
is raised on any breaking change to records, the envelope or exit codes.
`--help` is still there for people.

## Output

When stdout is a terminal atlx prints text tables; when it is piped it prints
one line of compact JSON. `-o/--output` (or `ATLX_OUTPUT`) picks one:

| `-o` | Prints |
| --- | --- |
| `json` | the envelope, one line |
| `jsonl` | one record per line, then a trailer `{"_end":{"next":…,"truncated":…,"errors":…}}` |
| `ids` | one primary ID per line, to pipe into another command |
| `text` | tables and fields for people |

```sh
atlx jira issue list --jql 'project = PROJ AND status = Open' -o ids | atlx jira issue get - --fields key,title,description
```

### The envelope

Every command, in every product, prints the same shape:

```json
{"ok":true,"data":{…},"next":"o:25","truncated":[…],"warnings":[…]}
{"ok":false,"error":{"code":"not_found","message":"…","hint":"…","retryable":false}}
{"ok":false,"partial":true,"data":[…],"errors":[{"input":"PROJ-9","error":{…}}]}
```

- `ok` is true exactly when the exit code is 0.
- `data` is a list for `list`, `search` and multi-ID `get`, and an object
  otherwise. There is no wrapper key around collections. (The top-level
  `search` returns an object keyed by product, and `api` puts the REST response
  in `data` untouched.)
- `next` appears on every `list` and `search`: a cursor when there is more,
  `null` when there is no more.
- `truncated` and `warnings` appear only when they have entries.
- In JSON modes stdout carries exactly one document and stderr stays empty.
- `--dry-run` prints `{"ok":true,"dryRun":true,"data":{"request":{…}}}`
  (see [Writing safely](#writing-safely)).

### Records and fields

Each command returns a registered record: flat, the same field names across
products for the same idea (`id`, `key`, `title`, `url`, `created`,
`updated`, `author`, `body`), people as `{id, name}`, and no avatar URLs or
`self` links. Jira custom fields appear under their display names.

- The default view holds the few fields that answer most questions. `list`
  never includes bodies; `get` includes `body` (or `description`) as
  Markdown.
- `--fields key,status,assignee.name` picks fields from the full record. An
  unknown name is a usage error (exit 2) that lists the valid ones.
- `--full` prints every field of the record.
- `--raw` prints the REST payload unprojected.
- `--output-file PATH` writes `data` (or the body, with `--body-only`) to a
  file and prints `{"file","bytes","lines"}` instead.

### Paging

Every `list` and `search` takes `--limit N` (default 25, at most 1000 per
invocation; atlx fetches as many REST pages as it needs), `--cursor` and
`--all` (up to the 1000 cap). To read more, pass the envelope's `next` back:

```sh
atlx confluence page list --space ENG --limit 100
atlx confluence page list --space ENG --limit 100 --cursor "$NEXT"   # the previous result's next
```

The cursor is opaque: atlx encodes offsets, page numbers or the API's own
tokens in it. A few Bitbucket scans that cannot resume (`pr list --mine`, for
example) return `next: null` with an `incomplete` warning when they stopped
early.

### Truncation

Large text is cut at a budget, on a block boundary, and the cut is announced
in the envelope, never inside the text:

```json
"truncated":[{"field":"data.body","shown":20000,"total":181234,"unit":"chars","more":"--body-offset 20000"}]
```

Budgets: bodies 20 000 characters (`--body-offset`), pull request diffs
40 000 characters (`--diff-offset`), repository files 20 000 characters
(`--content-offset`), pipeline logs the last 200 lines of each step
(`--log-offset`). `--max-chars N` changes a character budget and
`--max-chars 0` turns it off. For Confluence pages, `page get --outline` and
`--section <heading>` read part of a page instead. Text-mode table cells keep
their own `…` cut.

## Errors and exit codes

A failure prints an error envelope on stdout in JSON modes, and a readable
report on stderr in text mode:

```json
{"ok":false,"error":{"code":"conflict","message":"page 12345 is at version 9, you edited version 7","hint":"re-read it (`atlx confluence page get 12345`), re-apply your change, pass --if-version 9","retryable":false}}
```

- `code` is one of a closed set (below). `hint` says what to do next.
- When a server answered, `status`, `request` (`method`, `url`) and
  `requestId` are added.
- `retryable` is true for classes worth retrying later; `retryAfterMs` is
  given when the service said. atlx has already retried 429s and, for
  idempotent requests, 502/503/504 and transport errors.
- A network error or timeout on a write adds `"writeState":"unknown"`: the
  write may or may not have happened, so read before retrying.
- Jira field errors come through as `error.fields`; refused bodies list the
  nodes in `error.losses`.
- Usage errors from the argument parser are structured too, with
  `suggestions`.
- URLs never include credentials, and request headers are never printed.

| Exit | `error.code` | What to do |
| --- | --- | --- |
| 0 | | success |
| 1 | `validation`, `server` (after retries), `other` | read `error` |
| 2 | `usage` | fix the command; see `suggestions` or `atlx describe` |
| 3 | partial: some inputs of a multi-input command failed | use `data`, inspect `errors` |
| 4 | `config`, `auth`, `forbidden` | stop and ask the person |
| 5 | `not_found` | check the ID; the item may also be invisible to this account |
| 6 | `conflict` | re-read, re-apply, retry |
| 7 | `rate_limited`, `network`, `timeout` | retry later |
| 8 | `refused`: a safety guard stopped a write | do not work around it; ask if unsure |

Multi-ID reads carry on past failures and keep the input order; failed inputs
go to `errors` and the command exits 3. With `--strict` any failure fails the
whole command with that failure's exit code. `atlx describe --errors` prints
this table as JSON.

## Writing safely

- **`--dry-run`** works on every command. It is enforced in the HTTP client:
  reads run normally, and the first write is printed instead of sent.
  Confluence `page update` and `page edit` also print a Markdown diff of the
  change.
- **Read-only mode** (`--read-only`, `ATLX_READ_ONLY=1`, or `read_only = true`
  in a profile) refuses every write with `refused`, exit 8, including writes
  through `api`. Reads sent as POST (Jira bulk fetch, Bitbucket code
  search) still work.
- **`--yes`** is required on every destructive command, and only there:
  deletes and purges, archives, merges and declines, permission and sharing
  changes, version restores, and everything customers see in JSM.
  `atlx describe` marks them `destructive`, and a command's contract shows
  `"confirm":true`. Without `--yes`
  the command fails with `refused` (exit 8) and the hint shows the same
  command with `--dry-run`. Nothing ever prompts.
- **`--if-version N`** is required on Confluence `page update` and `page
  edit`: the `version` of the page you read. If the page changed since, the
  write fails with `conflict` (exit 6) and nothing is sent.
- **`--if-head <sha>`** on `bb pr merge` merges only if the pull request's
  source head is still that commit (`head` in `pr get` and `pr checks`); a
  mismatch exits 6 and sends no merge.
- Jira issues have no version or ETag, so there is no check there: read just
  before you write.

## Bodies

Jira and Confluence store rich text as ADF. atlx reads and writes Markdown by
default and converts it through one checked pipeline; Bitbucket bodies are
Markdown already. `--body-format` picks another format:

| Format | Read | Write |
| --- | --- | --- |
| `markdown` (default) | ADF rendered as Markdown | parsed to ADF; refused if anything has no ADF form |
| `adf` | the stored ADF JSON | ADF JSON, sent unchanged |
| `storage` (Confluence) | storage XHTML, unchanged | checked for well-formedness and known elements, sent unchanged |
| `plain` | | text; blank lines separate paragraphs |

HTML is not an input format.

**The Markdown dialect** is CommonMark with GFM tables, strikethrough and
task lists, plus panels as alert blockquotes (`> [!NOTE]`, `> [!TIP]`,
`> [!IMPORTANT]`, `> [!WARNING]`, `> [!CAUTION]`, `> [!SUCCESS]`), expand
blocks as `<details>`/`<summary>`, mentions `@[Name](accountid:ID)`, status
lozenges `[TEXT](status:green)`, dates `[2026-01-31](date:2026-01-31)`,
emoji `:shortname:`, smart links `[url](url "card")`, code blocks with a
language, and `<u>`, `<sub>`, `<sup>`, `<br>`.

**The guarantee.** For a document built only from those constructs, ADF to
Markdown to ADF gives the same document (ignoring `localId`s and layout
attributes), and the test suite checks it for each construct. For anything
else, atlx says so instead of losing it:

- On read, the envelope carries a `lossy_body` warning naming the nodes, and
  the Markdown has a marker where each one was
  (`<!-- atlx: extension:jira (not shown) -->`).
- A Markdown write containing something with no ADF form (raw HTML, for
  example) is refused, exit 8, listing each construct. Nothing is sent.
- A full-body Markdown replace (`confluence page update`, `jira issue update
  --description`, comment updates) of a body that holds content Markdown
  cannot express (macros, layouts, non-image media) is refused, exit 8,
  naming the nodes it would destroy. `--allow-lossy --yes` overrides.
- `confluence page edit` changes only the nodes its operations target and
  sends every other node back unchanged, so it is the safe way to edit a
  rich page:

```sh
atlx confluence page edit 12345 --if-version 7 --edits '[{"name":"replaceText","find":"v1.1","replace":"v1.2"}]' --dry-run
```

The dialect, the edit operations and the rules are written up in
[skills/atlx/references/bodies.md](skills/atlx/references/bodies.md).

## Metadata cache

atlx caches a few slow-changing lookup tables so records stay readable and
inputs forgiving: Jira field names (so `--field 'Story Points=3'` works),
issue types and priorities, Confluence space key to ID, and the Bitbucket
user. They live in `$XDG_CACHE_HOME/atlx/` (mode 0600) for 24 hours, and an
unknown name triggers one refetch before failing. Issues, pages, comments,
pull requests, diffs and logs are never written to disk. `--no-cache` or
`ATLX_NO_CACHE=1` turns it off; `atlx cache path` and `atlx cache clear`
manage it.

## Agent skill

atlx ships an agent skill (a `SKILL.md` with references) that teaches a
coding agent the grammar, the output and exit-code contract and the write
rules in about 1 500 tokens, and points it at `atlx describe` for the rest.

```sh
atlx skill install                 # ~/.claude/skills/atlx
atlx skill install --target agents # .agents/skills/atlx in this directory
atlx skill print
```

`install` leaves files that differ alone unless `--force` is given. The
per-product references are generated from `atlx describe`, so they match the
binary they ship with. Source: [skills/atlx](skills/atlx/SKILL.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the layout, the definition of done
and the conventions, and [AGENTS.md](AGENTS.md) for the short version for
coding agents.

The test suite runs offline against a mock HTTP server:

```sh
cargo test
```

Opt-in read-only live smoke tests are `#[ignore]`d in `tests/live_smoke.rs`:

```sh
ATLX_SITE=acme ATLX_EMAIL=you@acme.com ATLX_TOKEN=... cargo test -- --ignored
```

Optional variables for deeper checks: `ATLX_TEST_ISSUE`, `ATLX_TEST_PAGE`,
`ATLX_BB_WORKSPACE`, `ATLX_BB_REPO`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state
otherwise, any contribution you submit for inclusion in atlx is dual licensed
as above, without any additional terms or conditions.

atlx is an independent project and is not affiliated with or endorsed by
Atlassian.

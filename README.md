# atlx

An open, agent-friendly CLI for **Jira**, **Confluence** and **Bitbucket** Cloud.
A community alternative to Atlassian's [`twg`](https://github.com/atlassian/twg-cli)
built only on the public REST APIs: no Teamwork Graph, Rovo credits or early-access
program needed, just an API token.

**Follows twg's command tree.** atlx uses twg's command paths and flag names for
Jira, Confluence and Bitbucket, and the goal is that any twg invocation for those
products works unchanged with `atlx`:

```sh
twg  jira workitem get PROJ-1 --comments -o json
atlx jira workitem get PROJ-1 --comments -o json
```

Coverage is not complete yet. [COMPAT.md](COMPAT.md) lists every twg 1.4.4
command with its status in atlx, and CI checks it against the parser.

Global flags match too: `-o/--output text|json|jsonl`, `-s/--site` (prefix like
`acme` or a hostname), `-u/--user`, `-p/--profile`, `--timeout-ms`. When stdout is piped, atlx
defaults to JSON.

## Install

Prebuilt binaries are published on the
[releases page](https://github.com/pedrosanzmtz/atlx/releases) for Linux
(x86_64 and aarch64 glibc, plus a static x86_64 musl build), macOS (Intel and
Apple silicon) and Windows (x86_64 and ARM64).

**cargo-binstall** downloads the matching archive without compiling anything:

```sh
cargo binstall atlx
```

**Manual download.** Archives are named `atlx-vX.Y.Z-<target>.tgz` (`.zip` on
Windows). Each one holds the `atlx` binary at the top level, the licenses, this
README, the changelog, shell completions in `completions/` and man pages in
`man/`. Check the download against `SHA256SUMS` from the same release:

```sh
curl -LO https://github.com/pedrosanzmtz/atlx/releases/download/v0.1.0/atlx-v0.1.0-x86_64-unknown-linux-gnu.tgz
curl -LO https://github.com/pedrosanzmtz/atlx/releases/download/v0.1.0/SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf atlx-v0.1.0-x86_64-unknown-linux-gnu.tgz atlx
install -m 755 atlx ~/.local/bin/
```

**From crates.io** (needs a Rust toolchain):

```sh
cargo install atlx --locked
```

**From source:**

```sh
git clone https://github.com/pedrosanzmtz/atlx && cd atlx
cargo install --path . --locked
```

### Shell completions

`atlx completion script <bash|zsh|fish|powershell|elvish>` prints a completion
script. Install it once for the shell you use:

```sh
# bash
mkdir -p ~/.local/share/bash-completion/completions
atlx completion script bash > ~/.local/share/bash-completion/completions/atlx

# zsh: any directory on $fpath, loaded before compinit runs
mkdir -p ~/.zfunc
atlx completion script zsh > ~/.zfunc/_atlx
# in ~/.zshrc: fpath=(~/.zfunc $fpath); autoload -Uz compinit; compinit

# fish
atlx completion script fish > ~/.config/fish/completions/atlx.fish
```

For PowerShell, add `atlx completion script powershell | Out-String |
Invoke-Expression` to your `$PROFILE`. Release archives also ship these files
pre-generated in `completions/`.

Man pages are in the archive's `man/` directory (`atlx.1` plus one page per
subcommand, such as `atlx-jira-workitem-get.1`). Copy them to
`~/.local/share/man/man1/` and run `man atlx`.

## Authenticate

Create an API token at <https://id.atlassian.com/manage-profile/security/api-tokens>, then:

```sh
atlx login -s acme -u you@acme.com --token -     # paste the token on stdin
atlx whoami
```

For Bitbucket, create an API token with Bitbucket scopes (account, repository,
pull request, pipeline read; write scopes to create PRs/comments):

```sh
atlx setup bitbucket --workspace acme --token -
```

Config lives in `~/.config/atlx/config.toml` (mode 0600). Environment overrides:
`ATLX_SITE`, `ATLX_EMAIL`, `ATLX_TOKEN`, `ATLX_TOKEN_CMD`, `ATLX_BB_WORKSPACE`,
`ATLX_BB_EMAIL`, `ATLX_BB_TOKEN`, `ATLX_BB_TOKEN_CMD`, `ATLX_PROFILE`,
`ATLX_CONFIG`, `ATLX_OUTPUT`.

### Profiles

Several sites or workspaces can live side by side as named profiles, chosen
with `-p/--profile <name>` or `ATLX_PROFILE` (as with twg, `-p default` means
the saved default):

```sh
atlx login --new-profile sandbox -s acme-sandbox -u you@acme.com --token -
atlx setup bitbucket -p sandbox --workspace acme-sandbox --token -
atlx -p sandbox jira workitem get SBX-1
atlx auth profile list                  # name, site, email, workspace, token source
atlx auth profile default sandbox       # use it when -p is not given
atlx auth profile rename sandbox sbx
atlx logout -p sbx                      # remove one profile
```

Precedence, highest first: `-s`/`-u` flags, `ATLX_*` env vars, the selected
profile. A v1 config (top-level `site`, `email`, `token`, `[bitbucket]`) keeps
working as the profile named `default`; `atlx auth profile migrate --yes` moves
it into a named profile.

```toml
default_profile = "work"

[profiles.work]
site = "work.atlassian.net"
email = "you@work.com"
token_cmd = "op read op://Private/atlassian/credential"

[profiles.work.bitbucket]
workspace = "work"
token_cmd = "pass show bitbucket/work"
```

### Credential helpers

Instead of storing a token, save a command that prints it: `token_cmd` (and
`bitbucket.token_cmd`) in the config, `atlx login --token-cmd '<cmd>'`,
`atlx setup bitbucket --token-cmd '<cmd>'`, or the `ATLX_TOKEN_CMD` /
`ATLX_BB_TOKEN_CMD` env vars. The command runs through `sh -c` (`cmd /C` on
Windows) only when a request is about to be made, at most once per run, with
a 10 second timeout; its stdout is trimmed and never shown in errors.
`atlx whoami`, `atlx doctor` and `atlx auth profile list` report where the token
comes from (`config`, `env` or `token_cmd`), never its value.

## Commands

| twg-compatible command | Notes |
| --- | --- |
| `login`, `logout`, `whoami`, `setup bitbucket`, `setup default-site` | API-token auth instead of OAuth; `-p/--profile`, `login --new-profile`, `--token-cmd` (atlx extra) |
| `auth profile list`, `auth profile default`, `auth profile rename`, `auth profile migrate` | Profiles in `config.toml`; `list` never shows tokens |
| `user search <query>` | `--limit`; returns `account_id`, `display_name`, `email`, `active`, `account_type` |
| `jira workitem get <KEY...>` | `--comments`, `--remote-links`, `--full`, `--fields`, `--field`, `--strict`; batches |
| `jira workitem search <text...>` | `text ~ "…" ORDER BY updated DESC`; `--limit`, `--after` |
| `jira workitem query --jql` | `--limit`, `--after` (cursor), `--fields` |
| `jira workitem create` | `--space --type --summary --description[-format] --assignee --reporter --priority --labels --parent --field --fields-json` |
| `jira workitem create-meta --space` | `--type` to inspect required/allowed fields for an issue type |
| `jira workitem update --id` | fields, `--add-/--remove-labels`, components, fix versions, `--status`, `--comment` |
| `jira workitem comment create` | `--issue-id --body --body-format`, visibility, `--internal` |
| `jira workitem transition --id` | omit `--transition-id` to list; `--field`, `--transition-comment` |
| `jira workitem transitions query --id` | includes required screen fields |
| `jira workitem link types\|create\|remote` | `--inward/--outward/--type` or `--id/--url/--title/--relationship` |
| `jira field list` | `--query`, `--custom-only` |
| `jira project list\|get` | `--query`, `--limit`, or `<key-or-id>` |
| `jira board list`, `jira sprint list\|issues` | Agile boards (`--project`, `--type`) and sprints (`--board`, `--state`, `--sprint`, `--jql`) |
| `confluence search query --cql` / `search text` | `--excerpt`, `--cursor`, `--updated-since 7d` |
| `confluence content get <ref...>` | `--detail summary\|full`, `--format html\|md`, `--body-only`, `--reference-space` |
| `confluence content create` / `update` / `delete` | `--body`/`--body-file`, `--format html\|markdown\|plain`, `--snapshot-token`, `--dry-run`; delete needs `--yes`, optional `--purge` |
| `confluence content comments list\|create` | `--inline`, `--format md\|html`, `--limit`, `--cursor`; `--body`, `--format` |
| `confluence content list\|children\|labels`, `confluence tree` | `--space`, `--limit`, `--cursor`; `tree` fetches ancestors + `--depth` (max 5) |
| `confluence space list\|get\|me` | `me` resolves current user's personal space |
| `bitbucket\|bb repo query\|get\|file\|branches` | `--workspace`, `--repo` (slug, `ws/slug` or URL), `--ref`, `--query` |
| `bb pull-requests query\|get\|create\|diff\|activity\|tasks`, `comment create` | state/author/branch/date filters; `--comments --diff --statuses`, `--type` |
| `bb pull-requests approve\|unapprove\|request-changes\|merge\|decline` | `--strategy merge_commit\|squash\|fast_forward`, `--close-source-branch`, `--yes` |
| `bb pipeline query\|get`, `bb search code` | `--logs --failed-steps --lines`; workspace code search with `--repo` |
| `completion script <shell>` | bash, zsh, fish, powershell or elvish completion script on stdout |

atlx extras: `atlx doctor [--strict]` (verify config permissions and Jira,
Confluence and Bitbucket connectivity), `atlx search "<text>"` (Jira +
Confluence + Bitbucket in parallel) and `atlx api <jira|confluence|bitbucket>
<path>` for any REST endpoint not wrapped yet.

### Body formats

Like twg, bodies default to `html` and accept `markdown` and `plain`. Confluence
stores HTML as-is. Jira descriptions/comments are converted to ADF: headings,
nested lists, task lists, tables, code blocks, quotes, links, images, line
breaks and bold/italic/underline/strikethrough/code marks survive. Reads
(`--format md`, Jira descriptions and comments) come back as GitHub-flavoured
Markdown.

Errors go to stderr with a non-zero exit code. In `search`, a failing product is
reported inline (`{"error": ...}`) so the other results still come through.

## Compatibility with twg

The target is for `atlx` to behave exactly like `twg` for Jira, Confluence and
Bitbucket Cloud: the same command names, flags, output envelope and
confirmation behavior. [COMPAT.md](COMPAT.md) tracks how far along that is,
command by command. Today these differences remain:

- **JSON payloads are not wrapped yet**: `atlx` emits unwrapped JSON
  objects/arrays (and `{"items": [...], "summary": {...}}` for multi-ID batches). Switching to
  `twg`'s `{"data": ...}` envelope and key names is planned in
  [#21](https://github.com/pedrosanzmtz/atlx/issues/21).
- **Destructive commands still need `--yes`**: `twg` has no such flag on
  merge, decline or content delete.
  [#18](https://github.com/pedrosanzmtz/atlx/issues/18) removes the
  requirement (`--yes` stays accepted).
- **API token authentication**: `atlx` uses Atlassian and Bitbucket API tokens
  with HTTP Basic auth (`atlx login`, `atlx setup bitbucket`, or `ATLX_*`
  environment variables) instead of `twg`'s OAuth flow.
- **No Teamwork Graph / Rovo commands**: `twg` commands backed by Atlassian's
  private graph/Rovo APIs (`rovo`, `work`, `context`, `docs`, `org-tree`, ...)
  are intentionally excluded; `atlx` uses only the public REST APIs (with
  `atlx search` as a cross-product alternative). COMPAT.md marks these ⛔.

## Errors and exit codes

Errors follow `twg`. In `json` and `jsonl` mode (set with `-o`, or the default
when stdout is piped), a failed command prints an error envelope on **stdout**
(on one line for `jsonl`). A readable report always goes to **stderr**.

```json
{
  "ok": false,
  "error": {
    "code": "TWG_TOOL_EXECUTION_FAILED",
    "kind": "twg_tool_execution_failed",
    "message": "Issue does not exist or you do not have permission to see it.",
    "statusCode": 404,
    "requestId": "8c1f6a5a9eae434c",
    "downstreamError": {"message": "Issue does not exist ...", "statusCode": 404, "service": "backend"},
    "category": "not_found",
    "method": "GET",
    "url": "https://acme.atlassian.net/rest/api/3/issue/NOPE-1?fields=...",
    "retry": {"recommended": false, "guidance": "do_not_retry_unchanged"}
  }
}
```

- `code` uses twg's codes. `TWG_RATE_LIMITED` (429) and
  `TWG_UPSTREAM_UNAVAILABLE` (502, 503, 504) come with `retry_with_backoff`.
  `TWG_NETWORK_ERROR` and `TWG_REQUEST_TIMEOUT` come with `retry_later`. The
  other codes are `TWG_UPSTREAM_FAILURE` (other 5xx),
  `TWG_TOOL_EXECUTION_FAILED` (other HTTP errors) and `TWG_COMMAND_FAILED`
  (local failures). `kind` is `code` in lower case.
- `statusCode` is the HTTP status. As in twg, local failures report 400
  (invalid arguments) or 401 (missing credentials). `downstreamError` appears
  only when a server actually answered.
- `category` (atlx extra) is a stable class: `auth` (401), `forbidden` (403),
  `not_found` (404, 410), `conflict` (409, 412), `validation` (400, 422 and
  other 4xx), `rate_limited` (429), `server` (5xx), `network`, `timeout`,
  `config` (missing site, email or token), `usage` (for example a missing
  `--yes`) or `other`.
- `hint`, `method`, `url`, `traceId` and `requestId` appear when they are known.
  URLs never include credentials, and request headers are never printed.

In text mode, stdout stays empty and stderr shows `✗ Failed <command>.`
followed by the same report (`Error:`, `Code:`, `HTTP status:`, ...).

Multi-ID reads (`jira workitem get A B`, `confluence content get`,
`bb pull-requests get`) carry on past failures. Each failed item looks like
`{"input": "B", "ok": false, "error": {"message", "status", "code",
"category"}}`, and `summary` counts the requested, succeeded and failed inputs.
With `--strict`, any failure makes the whole command fail.

`atlx search` reports a failed product as `{"error": {...}}`, using the same
`error` object.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Any failure: API, network, configuration or argument errors (unknown flags included), a `--strict` batch with a failed input, or a batch where every input failed |
| `3` | Partial batch: some, but not all, inputs of a multi-ID read failed. The output is still printed |

`atlx doctor --strict` keeps its own non-zero exit when checks fail. To tell
failures apart, use the envelope's `category`, `statusCode` and `retry`.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
module layout, the "adding a command" checklist and conventions, and
[AGENTS.md](AGENTS.md) for a condensed version aimed at coding agents. Each
product's API calls live in `src/<product>.rs`; its twg-shaped commands live in
`src/cli/<product>.rs`.

### Testing

Run the offline unit and mock-HTTP integration test suite with:

```sh
cargo test
```

Opt-in read-only live smoke tests against real Atlassian Cloud / Bitbucket APIs
are marked `#[ignore]` in `tests/live_smoke.rs`. Run them with:

```sh
ATLX_SITE=acme ATLX_EMAIL=you@acme.com ATLX_TOKEN=... cargo test -- --ignored
```

Optional environment variables for deeper checks: `ATLX_TEST_ISSUE`,
`ATLX_TEST_PAGE`, `ATLX_BB_WORKSPACE`, and `ATLX_BB_REPO`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise,
any contribution you submit for inclusion in atlx is dual licensed as above,
without any additional terms or conditions.

atlx is an independent project and is not affiliated with or endorsed by Atlassian.

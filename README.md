# atlx

An open, agent-friendly CLI for **Jira**, **Confluence** and **Bitbucket** Cloud.
A community alternative to Atlassian's [`twg`](https://github.com/atlassian/twg-cli)
built only on the public REST APIs: no Teamwork Graph, Rovo credits or early-access
program needed, just an API token.

**Same commands and flags as twg.** Anything you (or your agent's skills) run with
`twg` for Jira, Confluence or Bitbucket works the same way with `atlx`:

```sh
twg  jira workitem get PROJ-1 --comments -o json
atlx jira workitem get PROJ-1 --comments -o json
```

Global flags match too: `-o/--output text|json|jsonl`, `-s/--site` (prefix like
`acme` or a hostname), `-u/--user`, `--timeout-ms`. When stdout is piped, atlx
defaults to JSON.

## Install

```sh
cargo install --path .
```

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
`ATLX_SITE`, `ATLX_EMAIL`, `ATLX_TOKEN`, `ATLX_BB_WORKSPACE`, `ATLX_BB_EMAIL`,
`ATLX_BB_TOKEN`, `ATLX_CONFIG`, `ATLX_OUTPUT`.

## Commands

| twg-compatible command | Notes |
| --- | --- |
| `login`, `logout`, `whoami`, `setup bitbucket`, `setup default-site` | API-token auth instead of OAuth |
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

atlx extras: `atlx search "<text>"` (Jira + Confluence + Bitbucket in parallel)
and `atlx api <jira|confluence|bitbucket> <path>` for any REST endpoint not
wrapped yet.

Not covered: twg's Teamwork Graph/Rovo commands (`rovo`, `work`, `context`,
`docs`, `org-tree`, ...). They need Atlassian's private graph APIs.

### Body formats

Like twg, bodies default to `html` and accept `markdown` and `plain`. Confluence
stores HTML as-is. Jira descriptions/comments are converted to ADF: headings,
lists, code blocks, quotes, bold, inline code and links survive; other HTML is
simplified.

Errors go to stderr with a non-zero exit code. In `search`, a failing product is
reported inline (`{"error": ...}`) so the other results still come through.

## Contributing

Contributions are welcome. `cargo test` and `cargo clippy` should pass. Each
product lives in its own module (`src/jira.rs`, `src/confluence.rs`,
`src/bitbucket.rs`); the twg-shaped command tree is in `src/main.rs`.

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

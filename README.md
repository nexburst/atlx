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
| `jira workitem get <KEY...>` | `--comments`, `--remote-links`, `--full`, `--fields`, `--field`, `--strict`; batches |
| `jira workitem search <text...>` | `text ~ "…" ORDER BY updated DESC`; `--limit`, `--after` |
| `jira workitem query --jql` | `--limit`, `--after` (cursor), `--fields` |
| `jira workitem create` | `--space --type --summary --description[-format] --assignee --reporter --priority --labels --parent --field --fields-json` |
| `jira workitem update --id` | fields, `--add-/--remove-labels`, components, fix versions, `--status`, `--comment` |
| `jira workitem comment create` | `--issue-id --body --body-format`, visibility, `--internal` |
| `jira workitem transition --id` | omit `--transition-id` to list; `--field`, `--transition-comment` |
| `jira workitem transitions query --id` | includes required screen fields |
| `confluence search query --cql` / `search text` | `--excerpt`, `--cursor`, `--updated-since 7d` |
| `confluence content get <id-or-url...>` | `--detail summary\|full`, `--format html\|md`, `--body-only` |
| `confluence content create` / `update` | `--body`/`--body-file`, `--format html\|markdown\|plain`, `--snapshot-token`, `--dry-run` |
| `confluence content list --space`, `confluence space list\|get` | |
| `bitbucket\|bb repo query\|get` | `--workspace`, `--repo` (slug, `ws/slug` or URL) |
| `bb pull-requests query\|get\|create`, `comment create` | state/author/branch/date filters; `--comments --diff --statuses` |
| `bb pipeline query\|get` | `--logs --failed-steps --lines` |

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

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise,
any contribution you submit for inclusion in atlx is dual licensed as above,
without any additional terms or conditions.

atlx is an independent project and is not affiliated with or endorsed by Atlassian.

# Core commands

<!-- Generated from `atlx describe` (contract 1) by `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->

Covers: `auth`, `doctor`, `search`, `api`, `describe`, `cache`, `completion`, `skill`. Every command also takes the global flags (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, `--read-only`, `-p`, ...). "paged" commands take `--limit` (default 25, at most 1000), `--cursor` and `--all`; "multi-ID" commands take several IDs (or `-` for stdin) plus `--strict` and `--concurrency`. `...` marks a repeatable value. "Returns" lists the record's default fields; `atlx describe <path>` has every field, flag type and default.

## auth

- **`auth login`** (write). Save credentials (site, email, API token) into a profile.
  Takes flags `--token`, `--token-cmd`, `--no-verify`, `-f/--force`, `--new-profile`.
  - `atlx auth login --new-profile work -s acme -u me@acme.com --token-cmd 'pass show atlassian'`
  - `atlx auth login -p work --force`
- **`auth logout`** (write). Remove the selected profile and its saved credentials.
  - `atlx auth logout -p work`
- **`auth status`** (read). Who is signed in, on which site, with which token source; which products answer; whether read-only mode is on. Run this first.
  Returns `auth.status`: profile, site, email, user, tokenSource, readOnly, products, bitbucketWorkspace.
  - `atlx auth status`
  - `atlx auth status -p work --fields user,products`
- **`auth bitbucket`** (write). Save the Bitbucket workspace and token for the profile.
  Takes flags `--workspace`, `--token`, `--token-cmd`, `--email`, `-f/--force`, `--no-verify`.
  - `atlx auth bitbucket --workspace acme --token-cmd 'pass show bitbucket'`

## auth profile

- **`auth profile list`** (read). Show each profile's site, email, workspace and token source (never the token).
  Returns `auth.profile`: name, default, active, site, email, workspace, tokenSource, readOnly.
  - `atlx auth profile list`
  - `atlx auth profile list -o ids`
- **`auth profile update`** (write). Save `-s/--site` as the selected profile's site, without logging in again.
  - `atlx auth profile update -p work -s acme.atlassian.net`
- **`auth profile migrate`** (destructive, needs --yes). Move the top-level (v1) credentials into a named profile.
  Takes flags `-y/--yes`.
  - `atlx auth profile migrate --dry-run`
  - `atlx auth profile migrate --yes`
- **`auth profile rename`** (write). Rename a profile (renaming `default` moves the top-level keys).
  Takes args `NAME` `NEW_NAME`.
  - `atlx auth profile rename default work`
- **`auth profile default`** (write). Make a profile the one used when -p and ATLX_PROFILE are not set.
  Takes args `NAME`.
  - `atlx auth profile default work`

## doctor

- **`doctor`** (read, multi-ID). Check config, permissions, and service connectivity in depth.
  - `atlx doctor`
  - `atlx doctor --strict`

## search

- **`search`** (read). Search Jira, Confluence and Bitbucket at once.
  Takes args `QUERY`; flags `--only jira|confluence|bitbucket...`.
  - `atlx search 'login timeout' --limit 5`
  - `atlx search deploy --only jira,confluence`

## api

- **`api`** (by-method). Call any REST endpoint directly.
  Takes args `jira|confluence|bitbucket` `PATH`; flags `-X/--method`, `-d/--data`, `-q/--query...`, `-y/--yes`.
  - `atlx api jira /rest/api/3/myself`
  - `atlx api confluence /api/v2/pages -q limit=5`
  - `atlx api jira /rest/api/3/issue -X POST -d @issue.json --dry-run`
  - `atlx api jira /rest/api/3/issue/PROJ-1 -X DELETE --yes`

## describe

- **`describe`** (read). Describe commands, record types and error codes as JSON.
  Takes args `[PATH...]`; flags `--records`, `--errors`.
  - `atlx describe`
  - `atlx describe auth status`
  - `atlx describe --errors`

## cache

- **`cache path`** (read). Show where the metadata cache lives, whether it is on, and which tables it may hold.
  - `atlx cache path`
- **`cache clear`** (write). Delete every cached table (they are fetched again when needed).
  - `atlx cache clear`

## completion

- **`completion script`** (read). Print a completion script for a shell to stdout.
  Takes args `bash|elvish|fish|powershell|zsh`.
  - `atlx completion script bash`

## skill

- **`skill install`** (write). Write the skill where an agent finds it.
  Takes flags `--dir`, `--target claude|agents`, `--force`.
  - `atlx skill install`
- **`skill print`** (read). Print SKILL.md (all files with -o json).
  - `atlx skill print`

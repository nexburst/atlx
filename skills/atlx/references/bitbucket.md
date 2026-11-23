# Bitbucket commands

<!-- Generated from `atlx describe` (contract 1) by `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->

Covers: `bitbucket` (alias `bb`): repositories, pull requests, pipelines, deployments. Every command also takes the global flags (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, `--read-only`, `-p`, ...). "paged" commands take `--limit` (default 25, at most 1000), `--cursor` and `--all`; "multi-ID" commands take several IDs (or `-` for stdin) plus `--strict` and `--concurrency`. `...` marks a repeatable value. "Returns" lists the record's default fields; `atlx describe <path>` has every field, flag type and default.

## bitbucket repo

- **`bitbucket repo list`** (read, paged). List the repositories of a workspace, most recently updated first.
  Takes flags `--workspace`, `--name`, `-q/--query`, `--project`, `--language`, `--role`, `--sort`.
  Returns `bitbucket.repo`: fullName, name, workspace, project, private, language, mainBranch, updated, url.
  - `atlx bitbucket repo list --workspace acme --name api`
  - `atlx bitbucket repo list -q 'project.key = "CORE"' --limit 50`
- **`bitbucket repo get`** (read, multi-ID). One or more repositories (workspace/slug; `-` reads them from stdin).
  Takes args `[REPO...]`.
  Returns `bitbucket.repo`: fullName, name, workspace, project, private, language, mainBranch, updated, url.
  - `atlx bitbucket repo get acme/api`
  - `atlx bitbucket repo get acme/api acme/web`
- **`bitbucket repo create`** (write). Create a repository (private unless --public).
  Takes args `REPO`; flags `--name`, `--description`, `--language`, `--project`, `--public`.
  Returns `bitbucket.repo`: fullName, name, workspace, project, private, language, mainBranch, updated, url.
  - `atlx bitbucket repo create acme/tools --project CORE`
  - `atlx bitbucket repo create acme/site --public --description 'Docs site'`

## bitbucket repo file

- **`bitbucket repo file get`** (read). A file's text (the first 20 000 characters; --content-offset continues).
  Takes args `PATH`; flags `--ref`, `--content-offset`, `-r/--repo`.
  Returns `bitbucket.file`: path, ref, commit, size, content.
  - `atlx bitbucket repo file get README.md --repo acme/api`
  - `atlx bitbucket repo file get src/main.rs --ref v1.2.0 --content-offset 20000`
- **`bitbucket repo file list`** (read, paged). A directory's entries.
  Takes flags `--path`, `--ref`, `--depth`, `-r/--repo`.
  Returns `bitbucket.file-entry`: path, type, size.
  - `atlx bitbucket repo file list --repo acme/api`
  - `atlx bitbucket repo file list --path src --depth 3 --ref main`
- **`bitbucket repo file download`** (read). Save a file, text or binary, to disk.
  Takes args `PATH`; flags `--to`, `--overwrite`, `--ref`, `-r/--repo`.
  Returns `bitbucket.download`: path, file, bytes.
  - `atlx bitbucket repo file download assets/logo.png --to ./logo.png`
- **`bitbucket repo file search`** (read, paged). Search code; `repo:`, `path:`, `ext:`, `lang:` words in the text filter.
  Takes args `TEXT...`; flags `--workspace`, `-r/--repo`.
  Returns `bitbucket.code-hit`: path, repo, ref, matches, url.
  - `atlx bitbucket repo file search 'fn main' --repo acme/api`
  - `atlx bitbucket repo file search 'TODO ext:rs' --workspace acme --limit 50`

## bitbucket repo branch

- **`bitbucket repo branch list`** (read, paged). Branches, newest head commit first.
  Takes flags `--name`, `-q/--query`, `--author`, `--sort`, `-r/--repo`.
  Returns `bitbucket.branch`: name, commit, date, author, title.
  - `atlx bitbucket repo branch list --name feature/`
  - `atlx bitbucket repo branch list --author 'Ann Lee' --sort name`
- **`bitbucket repo branch get`** (read, multi-ID). One or more branches.
  Takes args `NAME...`; flags `-r/--repo`.
  Returns `bitbucket.branch`: name, commit, date, author, title.
  - `atlx bitbucket repo branch get main develop --repo acme/api`
- **`bitbucket repo branch create`** (write). Create a branch.
  Takes args `NAME`; flags `--from`, `-r/--repo`.
  Returns `bitbucket.branch`: name, commit, date, author, title.
  - `atlx bitbucket repo branch create feature/login --from main`
  - `atlx bitbucket repo branch create hotfix/1 --from 3f2a9c1`
- **`bitbucket repo branch delete`** (destructive, needs --yes). Delete a branch.
  Takes args `NAME`; flags `-y/--yes`, `-r/--repo`.
  Returns `bitbucket.deleted`: id, deleted.
  - `atlx bitbucket repo branch delete feature/old --yes`

## bitbucket repo commit

- **`bitbucket repo commit list`** (read, paged). Commit history, newest first, of the main branch by default.
  Takes flags `--branch`, `--include...`, `--exclude...`, `--path`, `-r/--repo`.
  Returns `bitbucket.commit`: hash, date, author, title, url.
  - `atlx bitbucket repo commit list --branch develop --limit 10`
  - `atlx bitbucket repo commit list --include feature/x --exclude main --path src/`
- **`bitbucket repo commit get`** (read, multi-ID). One or more commits.
  Takes args `HASH...`; flags `-r/--repo`.
  Returns `bitbucket.commit`: hash, date, author, title, url.
  - `atlx bitbucket repo commit get 3f2a9c1 --repo acme/api`
- **`bitbucket repo commit create`** (write). Commit files to a branch without a checkout.
  Takes flags `-m/--message (required)`, `--file...`, `--file-local...`, `--file-remote...`, `--files-from`, `--delete...`, `--branch`, `--default-branch`, `--author`, `--parent`, `-r/--repo`.
  Returns `bitbucket.commit-result`: hash, branch, message, files.
  - `atlx bitbucket repo commit create -m 'Update docs' --branch docs --file README.md`
  - `atlx bitbucket repo commit create -m 'Drop old config' --default-branch --delete old.yml --parent 3f2a9c1`

## bitbucket repo contributor

- **`bitbucket repo contributor list`** (read). Commit authors by commit count, over the 1000 most recent commits.
  Takes flags `--ref`, `--range`, `--since`, `-r/--repo`.
  Returns `bitbucket.contributor`: name, id, commits.
  - `atlx bitbucket repo contributor list --since 90d`
  - `atlx bitbucket repo contributor list --range v1.0..main --limit 5`

## bitbucket repo report

- **`bitbucket repo report list`** (read, paged). A commit's reports.
  Takes flags `--commit (required)`, `-r/--repo`.
  Returns `bitbucket.report`: id, title, type, result, reporter, updated, link.
  - `atlx bitbucket repo report list --commit 3f2a9c1`
- **`bitbucket repo report create`** (write). Create or replace a report.
  Takes args `ID`; flags `--commit (required)`, `--report (required)`, `-r/--repo`.
  Returns `bitbucket.report`: id, title, type, result, reporter, updated, link.
  - `atlx bitbucket repo report create lint --commit 3f2a9c1 --report @report.json`

## bitbucket repo default-reviewer

- **`bitbucket repo default-reviewer list`** (read, paged). The repository's default reviewers.
  Takes flags `--effective`, `-r/--repo`.
  Returns `bitbucket.user`: id, name, nickname, accountId.
  - `atlx bitbucket repo default-reviewer list --repo acme/api`
  - `atlx bitbucket repo default-reviewer list --effective`
- **`bitbucket repo default-reviewer get`** (read). One default reviewer.
  Takes args `USER`; flags `-r/--repo`.
  Returns `bitbucket.user`: id, name, nickname, accountId.
  - `atlx bitbucket repo default-reviewer get '{1a2b3c4d-0000-4000-8000-000000000001}'`
- **`bitbucket repo default-reviewer add`** (destructive, needs --yes). Make a user a default reviewer of every new pull request.
  Takes args `USER`; flags `-y/--yes`, `-r/--repo`.
  Returns `bitbucket.user`: id, name, nickname, accountId.
  - `atlx bitbucket repo default-reviewer add 'Ann Lee' --yes`
- **`bitbucket repo default-reviewer remove`** (destructive, needs --yes). Stop a user being a default reviewer.
  Takes args `USER`; flags `-y/--yes`, `-r/--repo`.
  Returns `bitbucket.deleted`: id, deleted.
  - `atlx bitbucket repo default-reviewer remove 'Ann Lee' --yes`

## bitbucket pr

- **`bitbucket pr list`** (read, paged). Pull requests of a repository, most recently updated first.
  Takes flags `--state`, `--author`, `--reviewer`, `--source`, `--dest`, `-q/--query`, `--created-since`, `--created-before`, `--updated-since`, `--updated-before`, `--merged-since`, `--merged-before`, `--commit`, `--mine`, `--role author|reviewer|participant`, `--waiting`, `--workspace`, `--repo-limit`, `-r/--repo`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr list --repo acme/api --state OPEN --dest main`
  - `atlx bitbucket pr list --mine --waiting`
  - `atlx bitbucket pr list --state MERGED --merged-since 7d`
- **`bitbucket pr search`** (read, paged). Pull requests whose title or description contains the text.
  Takes args `TEXT...`; flags `--in both|title|description`, `--state`, `--mine`, `--role author|reviewer|participant`, `--waiting`, `--workspace`, `--repo-limit`, `-r/--repo`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr search 'login timeout' --state ALL`
  - `atlx bitbucket pr search flaky --in title --mine`
- **`bitbucket pr get`** (read, multi-ID). One or more pull requests, with description and reviewers (`-` reads numbers from stdin).
  Takes args `PR...`; flags `--body-offset`, `-r/--repo`.
  Returns `bitbucket.pr`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url, reviewers, approvals, body.
  - `atlx bitbucket pr get 42 --repo acme/api`
  - `atlx bitbucket pr get 42 43 44`
  - `atlx bitbucket pr get https://bitbucket.org/acme/api/pull-requests/42`
- **`bitbucket pr create`** (write). Open a pull request.
  Takes flags `--title (required)`, `--source (required)`, `--dest`, `--body`, `--body-file`, `--reviewer...`, `--default-reviewers`, `--close-source-branch`, `--draft`, `-r/--repo`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr create --title 'Add login' --source feature/login --body-file pr.md`
  - `atlx bitbucket pr create --title Fix --source fix/1 --dest develop --reviewer 'Ann Lee' --default-reviewers --draft`
- **`bitbucket pr update`** (write). Change a pull request's title, description, destination or flags.
  Takes args `PR`; flags `-r/--repo`, `--title`, `--body`, `--body-file`, `--clear-body`, `--dest`, `--close-source-branch`, `--keep-source-branch`, `--draft`, `--ready`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr update 42 --title 'Add login (v2)'`
  - `atlx bitbucket pr update 42 --ready --keep-source-branch`
- **`bitbucket pr merge`** (destructive, needs --yes). Merge a pull request, waiting until Bitbucket has finished.
  Takes args `PR`; flags `-r/--repo`, `--strategy merge_commit|squash|fast_forward`, `--message`, `--close-source-branch`, `--keep-source-branch`, `--if-head`, `--no-wait`, `--timeout`, `-y/--yes`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr merge 42 --strategy squash --if-head 3f2a9c1b7e44 --yes`
  - `atlx bitbucket pr merge 42 --dry-run`
- **`bitbucket pr decline`** (destructive, needs --yes). Decline a pull request.
  Takes args `PR`; flags `-r/--repo`, `-y/--yes`.
  Returns `bitbucket.pr-row`: id, title, state, author, source, destination, head, draft, comments, tasks, created, updated, url.
  - `atlx bitbucket pr decline 42 --yes`
- **`bitbucket pr approve`** (write). Approve a pull request.
  Takes args `PR`; flags `-r/--repo`.
  Returns `bitbucket.review`: pr, approved, state, user, date.
  - `atlx bitbucket pr approve 42`
- **`bitbucket pr unapprove`** (write). Withdraw your approval.
  Takes args `PR`; flags `-r/--repo`.
  Returns `bitbucket.review`: pr, approved, state, user, date.
  - `atlx bitbucket pr unapprove 42`
- **`bitbucket pr request-changes`** (write). Request changes on a pull request.
  Takes args `PR`; flags `-r/--repo`, `--withdraw`.
  Returns `bitbucket.review`: pr, approved, state, user, date.
  - `atlx bitbucket pr request-changes 42`
  - `atlx bitbucket pr request-changes 42 --withdraw`
- **`bitbucket pr diff`** (read). The diff (the first 40 000 characters; --diff-offset continues).
  Takes args `PR`; flags `-r/--repo`, `--path`, `--patch`, `--grep`, `--diff-offset`.
  Returns `bitbucket.pr-diff`: pr, path, diff, matches.
  - `atlx bitbucket pr diff 42 --path src/auth.rs`
  - `atlx bitbucket pr diff 42 --grep validate_token`
  - `atlx bitbucket pr diff 42 --diff-offset 40000`
- **`bitbucket pr diffstat`** (read, paged). Files changed, with lines added and removed.
  Takes args `PR`; flags `-r/--repo`, `--conflicts`.
  Returns `bitbucket.diffstat`: path, status, added, removed.
  - `atlx bitbucket pr diffstat 42`
  - `atlx bitbucket pr diffstat 42 --conflicts`
- **`bitbucket pr commits`** (read, paged). Commits in a pull request.
  Takes args `PR`; flags `-r/--repo`.
  Returns `bitbucket.commit`: hash, date, author, title, url.
  - `atlx bitbucket pr commits 42`
- **`bitbucket pr activity`** (read, paged). Events: comments, approvals, updates.
  Takes args `PR`; flags `-r/--repo`, `--type`.
  Returns `bitbucket.pr-activity`: type, date, user, text, comment.
  - `atlx bitbucket pr activity 42 --type comment --limit 50`
- **`bitbucket pr checks`** (read). What gates the merge: build statuses, approvals, tasks, conflicts.
  Takes args `PR`; flags `-r/--repo`, `--no-conflicts`.
  Returns `bitbucket.pr-checks`: pr, head, mergeable, approvals, blockers, statuses, unchecked.
  - `atlx bitbucket pr checks 42`
  - `atlx bitbucket pr checks 42 --no-conflicts`

## bitbucket pr comment

- **`bitbucket pr comment list`** (read, paged). Comments, oldest first, without deleted ones.
  Takes flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment list --pr 42`
- **`bitbucket pr comment get`** (read, multi-ID). One or more comments.
  Takes args `ID...`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment get 901 902 --pr 42`
- **`bitbucket pr comment create`** (write). A general, reply, pending or inline comment.
  Takes flags `--pr (required)`, `-r/--repo`, `--body`, `--body-file`, `--reply-to`, `--pending`, `--path`, `--line`, `--start-line`, `--end-line`, `--from-line`, `--start-from-line`, `--end-from-line`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment create --pr 42 --body 'Looks good'`
  - `atlx bitbucket pr comment create --pr 42 --body-file note.md --path src/auth.rs --line 18`
  - `atlx bitbucket pr comment create --pr 42 --body Agreed --reply-to 901`
- **`bitbucket pr comment update`** (write). Replace a comment's text.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`, `--body`, `--body-file`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment update 901 --pr 42 --body 'Fixed wording'`
- **`bitbucket pr comment delete`** (destructive, needs --yes). Delete a comment.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`, `-y/--yes`.
  Returns `bitbucket.deleted`: id, deleted.
  - `atlx bitbucket pr comment delete 901 --pr 42 --yes`
- **`bitbucket pr comment resolve`** (write). Mark a comment thread resolved.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment resolve 901 --pr 42`
- **`bitbucket pr comment reopen`** (write). Reopen a resolved comment thread.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-comment`: id, author, body, path, line, fromLine, parent, resolved, pending, created.
  - `atlx bitbucket pr comment reopen 901 --pr 42`

## bitbucket pr task

- **`bitbucket pr task list`** (read, paged). Tasks.
  Takes flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task list --pr 42`
- **`bitbucket pr task get`** (read, multi-ID). One or more tasks.
  Takes args `ID...`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task get 7 --pr 42`
- **`bitbucket pr task create`** (write). Add a task, optionally tied to a comment.
  Takes flags `--pr (required)`, `-r/--repo`, `--body`, `--body-file`, `--comment`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task create --pr 42 --body 'Add a test'`
  - `atlx bitbucket pr task create --pr 42 --body 'Rename this' --comment 901`
- **`bitbucket pr task update`** (write). Replace a task's text.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`, `--body`, `--body-file`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task update 7 --pr 42 --body 'Add two tests'`
- **`bitbucket pr task delete`** (destructive, needs --yes). Delete a task.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`, `-y/--yes`.
  Returns `bitbucket.deleted`: id, deleted.
  - `atlx bitbucket pr task delete 7 --pr 42 --yes`
- **`bitbucket pr task resolve`** (write). Mark a task done.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task resolve 7 --pr 42`
- **`bitbucket pr task reopen`** (write). Mark a done task open again.
  Takes args `ID`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.pr-task`: id, body, state, creator, comment, created.
  - `atlx bitbucket pr task reopen 7 --pr 42`

## bitbucket pr reviewer

- **`bitbucket pr reviewer list`** (read). Reviewers with their review state.
  Takes flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.reviewer`: id, name, approved, state.
  - `atlx bitbucket pr reviewer list --pr 42`
- **`bitbucket pr reviewer add`** (write). Add reviewers: names, nicknames, account IDs, {uuid}s or me.
  Takes args `USER...`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.reviewer`: id, name, approved, state.
  - `atlx bitbucket pr reviewer add 'Ann Lee' bob --pr 42`
  - `atlx bitbucket pr reviewer add me --pr 42`
- **`bitbucket pr reviewer remove`** (write). Remove reviewers.
  Takes args `USER...`; flags `--pr (required)`, `-r/--repo`.
  Returns `bitbucket.reviewer`: id, name, approved, state.
  - `atlx bitbucket pr reviewer remove bob --pr 42`

## bitbucket pipeline

- **`bitbucket pipeline list`** (read, paged). Pipeline runs, newest first.
  Takes flags `--state`, `--result`, `--branch`, `--trigger`, `--pattern`, `-r/--repo`.
  Returns `bitbucket.pipeline`: buildNumber, state, result, ref, commit, pattern, trigger, creator, created, completed, duration, url.
  - `atlx bitbucket pipeline list --branch main --result FAILED`
  - `atlx bitbucket pipeline list --pattern nightly --limit 5`
- **`bitbucket pipeline get`** (read, multi-ID). One or more runs: build number, {uuid} or result URL.
  Takes args `PIPELINE...`; flags `-r/--repo`.
  Returns `bitbucket.pipeline`: buildNumber, state, result, ref, commit, pattern, trigger, creator, created, completed, duration, url.
  - `atlx bitbucket pipeline get 1234`
  - `atlx bitbucket pipeline get 1234 1235`
- **`bitbucket pipeline run`** (write). Start a run on a branch, tag or commit, or start a failed one again.
  Takes flags `--branch`, `--tag`, `--commit`, `--custom`, `--variable...`, `--secured-variable...`, `--rerun`, `-r/--repo`.
  Returns `bitbucket.pipeline`: buildNumber, state, result, ref, commit, pattern, trigger, creator, created, completed, duration, url.
  - `atlx bitbucket pipeline run --branch main`
  - `atlx bitbucket pipeline run --branch main --custom nightly --variable ENV=staging`
  - `atlx bitbucket pipeline run --rerun 1234`
- **`bitbucket pipeline log`** (read). Step logs (the last 200 lines of each; --log-offset reads further back).
  Takes args `[PIPELINE]`; flags `--step`, `--failed`, `--branch`, `--grep`, `-i/--ignore-case`, `-C/--context`, `--max-matches`, `--follow`, `--interval`, `--timeout`, `--log-offset`, `-r/--repo`.
  Returns `bitbucket.pipeline-log`: pipeline, step, stepIndex, state, result, log, matches, stepUuid.
  - `atlx bitbucket pipeline log --failed --branch main`
  - `atlx bitbucket pipeline log 1234 --step 2 --log-offset 200`
  - `atlx bitbucket pipeline log 1234 --grep 'error:' -i -C 2`

## bitbucket pipeline step

- **`bitbucket pipeline step list`** (read, paged). The steps of a run.
  Takes flags `--pipeline (required)`, `-r/--repo`.
  Returns `bitbucket.pipeline-step`: index, name, state, result, started, completed, duration, uuid, testReport, testCases.
  - `atlx bitbucket pipeline step list --pipeline 1234`
- **`bitbucket pipeline step get`** (read). One step with its test report.
  Takes args `STEP`; flags `--pipeline (required)`, `--test-cases`, `--case-limit`, `-r/--repo`.
  Returns `bitbucket.pipeline-step`: index, name, state, result, started, completed, duration, uuid, testReport, testCases.
  - `atlx bitbucket pipeline step get 2 --pipeline 1234`
  - `atlx bitbucket pipeline step get Test --pipeline 1234 --test-cases`

## bitbucket pipeline config

- **`bitbucket pipeline config get`** (read). Whether Pipelines is on.
  Takes flags `-r/--repo`.
  Returns `bitbucket.pipeline-config`: enabled, repo.
  - `atlx bitbucket pipeline config get`
- **`bitbucket pipeline config update`** (write). Turn Pipelines on or off; off stops all future runs.
  Takes flags `--enabled true|false (required)`, `-r/--repo`.
  Returns `bitbucket.pipeline-config`: enabled, repo.
  - `atlx bitbucket pipeline config update --enabled false`

## bitbucket deployment

- **`bitbucket deployment list`** (read, paged). Deployments; any filter reads up to 5000 of them and sorts by start, newest first.
  Takes flags `--environment`, `--status...`, `--pipeline`, `--started-after`, `--started-before`, `--oldest-first`, `-r/--repo`.
  Returns `bitbucket.deployment`: uuid, environment, state, started, completed, commit, pipeline.
  - `atlx bitbucket deployment list --environment production --limit 5`
  - `atlx bitbucket deployment list --environment staging --status FAILED --started-after 7d`
- **`bitbucket deployment get`** (read, multi-ID). One or more deployments by UUID.
  Takes args `UUID...`; flags `--environment`, `-r/--repo`.
  Returns `bitbucket.deployment`: uuid, environment, state, started, completed, commit, pipeline.
  - `atlx bitbucket deployment get '{0b1c2d3e-0000-4000-8000-000000000001}'`
- **`bitbucket deployment commits`** (read). Commits a deployment added and removed since its environment's previous successful one.
  Takes args `UUID`; flags `--environment`, `-r/--repo`.
  Returns `bitbucket.deployment-commits`: deployment, environment, previous, added, removed.
  - `atlx bitbucket deployment commits '{0b1c2d3e-0000-4000-8000-000000000001}'`

## bitbucket environment

- **`bitbucket environment list`** (read, paged). Deployment environments.
  Takes flags `-r/--repo`.
  Returns `bitbucket.environment`: uuid, name, slug, type.
  - `atlx bitbucket environment list --repo acme/api`

## bitbucket environment variable

- **`bitbucket environment variable list`** (read, paged). An environment's variables; secured values are never shown.
  Takes flags `--environment (required)`, `-r/--repo`.
  Returns `bitbucket.variable`: key, value, secured.
  - `atlx bitbucket environment variable list --environment production`

## bitbucket workspace

- **`bitbucket workspace get`** (read, multi-ID). One or more workspaces [default: the current one].
  Takes args `[SLUG...]`.
  Returns `bitbucket.workspace`: slug, name, private, url.
  - `atlx bitbucket workspace get`
  - `atlx bitbucket workspace get acme`

## bitbucket workspace member

- **`bitbucket workspace member list`** (read, paged). List members.
  Takes flags `--name`, `--permissions`, `--workspace`.
  Returns `bitbucket.member`: id, name, nickname, accountId, permission.
  - `atlx bitbucket workspace member list --name ann`
  - `atlx bitbucket workspace member list --workspace acme --permissions`
- **`bitbucket workspace member get`** (read, multi-ID). One or more members.
  Takes args `USER...`; flags `--workspace`.
  Returns `bitbucket.member`: id, name, nickname, accountId, permission.
  - `atlx bitbucket workspace member get me`
  - `atlx bitbucket workspace member get 'Ann Lee' --workspace acme`

# Confluence commands

<!-- Generated from `atlx describe` (contract 1) by `UPDATE_SKILL=1 cargo test --test skill`. Do not edit by hand. -->

Covers: `confluence`: search, pages, comments, attachments, labels, versions, spaces. Every command also takes the global flags (`-o`, `--fields`, `--full`, `--raw`, `--max-chars`, `--dry-run`, `--read-only`, `-p`, ...). "paged" commands take `--limit` (default 25, at most 1000), `--cursor` and `--all`; "multi-ID" commands take several IDs (or `-` for stdin) plus `--strict` and `--concurrency`. `...` marks a repeatable value. "Returns" lists the record's default fields; `atlx describe <path>` has every field, flag type and default.

## confluence

- **`confluence search`** (read, paged). Search content with free text or CQL.
  Takes args `[TEXT...]`; flags `--cql`, `--updated-since`, `--created-since`.
  Returns `confluence.search-hit`: id, type, title, space, excerpt, updated, url.
  - `atlx confluence search 'deploy runbook' --limit 5`
  - `atlx confluence search --cql 'type = page AND space = ENG AND title ~ "release"'`
  - `atlx confluence search onboarding --updated-since 30d --fields id,title,url`

## confluence page

- **`confluence page list`** (read, paged). List the pages or blog posts of a space, or the child pages of a page.
  Takes flags `--space`, `--parent`, `--type page|blogpost`, `--status current|draft|archived|trashed`.
  Returns `confluence.page-item`: id, type, title, status, spaceId, parentId, version, updated, url.
  - `atlx confluence page list --space ENG --limit 10`
  - `atlx confluence page list --parent 12345 --all -o ids`
  - `atlx confluence page list --space ENG --type blogpost --status draft`
- **`confluence page get`** (read, multi-ID). Read pages or blog posts by ID, URL or exact title; the body is Markdown unless --body-format says otherwise.
  Takes args `ID...`; flags `--space`, `--type page|blogpost`, `--body-format markdown|adf|storage|plain`, `--section...`, `--outline`, `--body-offset`, `--body-only`, `--draft`.
  Returns `confluence.page`: id, type, title, status, spaceId, parentId, version, updated, url, body, outline.
  - `atlx confluence page get 12345`
  - `atlx confluence page get 12345 --section Rollback --fields title,version,body`
  - `atlx confluence page get 12345 --outline`
  - `atlx confluence page get Runbook --space ENG --body-format adf`
- **`confluence page create`** (write). Create a page or blog post.
  Takes flags `--space (required)`, `--title (required)`, `--body`, `--body-file`, `--body-format markdown|adf|storage|plain`, `--parent`, `--type page|blogpost`, `--draft`, `--private`, `--width narrow|wide|max`, `--emoji`.
  Returns `confluence.page-item`: id, type, title, status, spaceId, parentId, version, updated, url.
  - `atlx confluence page create --space ENG --title 'Release 1.2' --body-file notes.md`
  - `atlx confluence page create --space ENG --parent 12345 --title Child --body '# Hello' --dry-run`
- **`confluence page update`** (write). Replace the title or the whole body (needs --if-version).
  Takes args `ID`; flags `--if-version (required)`, `--version-message`, `--minor-edit`, `--draft`, `--type page|blogpost`, `--title`, `--body`, `--body-file`, `--body-format markdown|adf|storage|plain`, `--allow-lossy`, `-y/--yes`.
  Returns `confluence.page-item`: id, type, title, status, spaceId, parentId, version, updated, url.
  - `atlx confluence page update 12345 --if-version 7 --title 'New title'`
  - `atlx confluence page update 12345 --if-version 7 --body-file page.md --dry-run`
  - `atlx confluence page update 12345 --if-version 7 --body-file page.json --body-format adf`
- **`confluence page edit`** (write). Change parts of the body with edit operations (needs --if-version).
  Takes args `ID`; flags `--if-version (required)`, `--version-message`, `--minor-edit`, `--draft`, `--type page|blogpost`, `--edits`, `--edits-file`, `--title`.
  Returns `confluence.page-item`: id, type, title, status, spaceId, parentId, version, updated, url.
  - `atlx confluence page edit 12345 --if-version 7 --edits '[{"name":"replaceText","find":"v1.1","replace":"v1.2"}]'`
  - `atlx confluence page edit 12345 --if-version 7 --edits-file edits.json --dry-run`
- **`confluence page delete`** (destructive, needs --yes). Move to the trash; with --purge delete from the trash for good; with --draft delete a draft that was never published.
  Takes args `ID`; flags `--space`, `--purge`, `--draft`, `--type page|blogpost`, `-y/--yes`.
  - `atlx confluence page delete 12345 --yes`
  - `atlx confluence page delete 12345 --purge --yes`
  - `atlx confluence page delete 12345 --draft --dry-run`
- **`confluence page restore`** (write). Bring a page or blog post back from the trash.
  Takes args `ID`; flags `--space`.
  - `atlx confluence page restore 12345`
- **`confluence page move`** (write). Move a page under or next to another page.
  Takes args `ID`; flags `--space`, `--parent`, `--before`, `--after`.
  - `atlx confluence page move 12345 --parent 67890`
  - `atlx confluence page move 12345 --before 67890`
- **`confluence page copy`** (write). Copy a page under a parent or into a space.
  Takes args `ID`; flags `--space`, `--parent`, `--to-space`, `--title`, `--attachments`, `--labels`, `--permissions`.
  - `atlx confluence page copy 12345 --parent 67890 --title 'Copy of runbook'`
  - `atlx confluence page copy 12345 --to-space OPS --attachments --labels`
- **`confluence page archive`** (destructive, needs --yes). Archive a page.
  Takes args `ID`; flags `--space`, `--wait`, `--timeout`, `-y/--yes`.
  - `atlx confluence page archive 12345 --yes`
  - `atlx confluence page archive 12345 --wait --yes`
- **`confluence page publish`** (write). Publish a draft.
  Takes args `ID`; flags `--space`, `--type page|blogpost`, `--title`.
  Returns `confluence.page-item`: id, type, title, status, spaceId, parentId, version, updated, url.
  - `atlx confluence page publish 12345 --title 'Final title'`
- **`confluence page watch`** (write). Watch a page or blog post (get notified about changes).
  Takes args `ID`; flags `--space`.
  - `atlx confluence page watch 12345`
- **`confluence page unwatch`** (write). Stop watching a page or blog post.
  Takes args `ID`; flags `--space`.
  - `atlx confluence page unwatch 12345`
- **`confluence page tree`** (read). A page's ancestors and its descendants.
  Takes args `ID`; flags `--depth`, `--type page|whiteboard|database`.
  - `atlx confluence page tree 12345`
  - `atlx confluence page tree 12345 --depth 4 --type whiteboard`

## confluence page comment

- **`confluence page comment list`** (read, paged). List the footer (default) or inline comments of a page or blog post.
  Takes flags `--page (required)`, `--space`, `--type page|blogpost`, `--inline`, `--resolution open|resolved|reopened|dangling`, `--status current|draft`, `--sort created-date|-created-date`, `--replies`, `--body-format markdown|adf|storage|plain`.
  Returns `confluence.comment`: id, type, pageId, parentId, author, created, version, resolution, body, replies, url.
  - `atlx confluence page comment list --page 12345`
  - `atlx confluence page comment list --page 12345 --inline --resolution open --replies`
- **`confluence page comment get`** (read, multi-ID). Read comments by ID, ARI or URL.
  Takes args `ID...`; flags `--inline`, `--replies`, `--body-format markdown|adf|storage|plain`.
  Returns `confluence.comment`: id, type, pageId, parentId, author, created, version, resolution, body, replies, url.
  - `atlx confluence page comment get 777`
  - `atlx confluence page comment get 777 778 --replies`
- **`confluence page comment create`** (write). Add a footer comment, an inline comment on selected text, or a reply.
  Takes flags `--page`, `--comment`, `--space`, `--body`, `--body-file`, `--body-format markdown|adf|storage|plain`, `--text-selection`, `--text-selection-match-count`, `--text-selection-match-index`.
  Returns `confluence.comment`: id, type, pageId, parentId, author, created, version, resolution, body, replies, url.
  - `atlx confluence page comment create --page 12345 --body 'Looks good'`
  - `atlx confluence page comment create --page 12345 --body 'Typo?' --text-selection 'teh'`
  - `atlx confluence page comment create --comment 777 --body Thanks`
- **`confluence page comment update`** (write). Replace a comment's body, or resolve or reopen an inline comment.
  Takes args `ID`; flags `--body`, `--body-file`, `--body-format markdown|adf|storage|plain`, `--if-version`, `--resolve`, `--reopen`, `--inline`, `--allow-lossy`, `-y/--yes`.
  Returns `confluence.comment`: id, type, pageId, parentId, author, created, version, resolution, body, replies, url.
  - `atlx confluence page comment update 777 --body 'Fixed wording' --if-version 2`
  - `atlx confluence page comment update 777 --resolve`
- **`confluence page comment delete`** (destructive, needs --yes). Delete a comment for good.
  Takes args `ID`; flags `-y/--yes`.
  - `atlx confluence page comment delete 777 --yes`

## confluence page attachment

- **`confluence page attachment list`** (read, paged). List a page's attachments.
  Takes flags `--page (required)`, `--space`, `--media-type`, `--filename`, `--status current|archived|trashed`, `--sort`.
  Returns `confluence.attachment`: id, title, mediaType, fileSize, version, pageId, comment, url.
  - `atlx confluence page attachment list --page 12345`
  - `atlx confluence page attachment list --page 12345 --media-type image/png`
- **`confluence page attachment get`** (read, multi-ID). Attachment details by ID (att12345) or URL.
  Takes args `ID...`.
  Returns `confluence.attachment`: id, title, mediaType, fileSize, version, pageId, comment, url.
  - `atlx confluence page attachment get att456`
- **`confluence page attachment download`** (read). Save an attachment to a local file.
  Takes args `ID`; flags `--out`, `--force`.
  - `atlx confluence page attachment download att456 --out diagram.png`
- **`confluence page attachment upload`** (write). Attach local files; a file with the name of an existing attachment becomes its new version.
  Takes flags `--page (required)`, `--space`, `--file... (required)`, `--comment`, `--minor-edit`.
  Returns `confluence.attachment`: id, title, mediaType, fileSize, version, pageId, comment, url.
  - `atlx confluence page attachment upload --page 12345 --file diagram.png`
  - `atlx confluence page attachment upload --page 12345 --file a.pdf --file b.pdf --comment v2`
- **`confluence page attachment delete`** (destructive, needs --yes). Move an attachment to the trash, or with --purge delete it from the trash for good.
  Takes args `ID`; flags `--purge`, `-y/--yes`.
  - `atlx confluence page attachment delete att456 --yes`
  - `atlx confluence page attachment delete att456 --purge --yes`

## confluence page label

- **`confluence page label list`** (read, paged). List a page's labels.
  Takes flags `--page (required)`, `--space`, `--type page|blogpost|custom-content`.
  Returns `confluence.label`: name, prefix, id.
  - `atlx confluence page label list --page 12345`
- **`confluence page label create`** (write). Add labels; prints the page's labels afterwards.
  Takes args `LABEL...`; flags `--page (required)`, `--space`, `--prefix`.
  Returns `confluence.label`: name, prefix, id.
  - `atlx confluence page label create runbook ops --page 12345`
- **`confluence page label delete`** (destructive, needs --yes). Remove labels.
  Takes args `LABEL...`; flags `--page (required)`, `--space`, `--prefix`, `-y/--yes`.
  - `atlx confluence page label delete draft --page 12345 --yes`

## confluence page version

- **`confluence page version list`** (read, paged). List versions, newest first.
  Takes flags `--page (required)`, `--space`.
  Returns `confluence.version`: number, message, minorEdit, author, created, title, body.
  - `atlx confluence page version list --page 12345 --limit 5`
- **`confluence page version get`** (read). One version, with the title and body it had.
  Takes args `N`; flags `--page (required)`, `--space`, `--body-format markdown|adf|storage|plain`.
  Returns `confluence.version`: number, message, minorEdit, author, created, title, body.
  - `atlx confluence page version get 3 --page 12345`
- **`confluence page version diff`** (read). A unified diff between two versions' bodies.
  Takes flags `--page (required)`, `--space`, `--from (required)`, `--to (required)`, `--body-format markdown|adf|storage|plain`.
  - `atlx confluence page version diff --page 12345 --from 3 --to 5`
- **`confluence page version restore`** (destructive, needs --yes). Make an old version the latest again (adds a new version).
  Takes args `N`; flags `--page (required)`, `--space`, `--message`, `--restore-title`, `-y/--yes`.
  - `atlx confluence page version restore 3 --page 12345 --message 'Undo bad edit' --yes`

## confluence page task

- **`confluence page task list`** (read, paged). List the tasks of a page, or every task you can see.
  Takes flags `--page`, `--space`, `--type page|blogpost`, `--status open|complete|all`.
  Returns `confluence.task`: id, status, pageId, spaceId, assignee, due, body.
  - `atlx confluence page task list --page 12345 --status open`
  - `atlx confluence page task list --status open --limit 50`
- **`confluence page task get`** (read, multi-ID). Tasks by ID.
  Takes args `ID...`.
  Returns `confluence.task`: id, status, pageId, spaceId, assignee, due, body.
  - `atlx confluence page task get 42`
- **`confluence page task update`** (write). Complete or reopen a task.
  Takes args `ID`; flags `--status open|complete (required)`.
  Returns `confluence.task`: id, status, pageId, spaceId, assignee, due, body.
  - `atlx confluence page task update 42 --status complete`

## confluence page state

- **`confluence page state get`** (read). The status shown on a page.
  Takes args `ID`; flags `--space`, `--draft`.
  - `atlx confluence page state get 12345`
- **`confluence page state update`** (write). Set the status (publishes a new version).
  Takes args `ID`; flags `--space`, `--draft`, `--state-id`, `--name`, `--color`.
  - `atlx confluence page state update 12345 --name 'In review' --color '#ffc400'`
  - `atlx confluence page state update 12345 --state-id 3`
- **`confluence page state list`** (read). The statuses a page can be given.
  Takes args `ID`; flags `--space`.
  - `atlx confluence page state list 12345`

## confluence space

- **`confluence space list`** (read, paged). List spaces.
  Takes flags `--type global|personal`, `--status current|archived`, `--key...`, `--labels`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space list --limit 10`
  - `atlx confluence space list --type personal --all -o ids`
- **`confluence space get`** (read, multi-ID). Spaces by key or ID, or your personal space.
  Takes args `[SPACE...]`; flags `--personal`, `--labels`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space get ENG`
  - `atlx confluence space get ENG OPS --labels --full`
  - `atlx confluence space get --personal`
- **`confluence space create`** (write). Create a space.
  Takes flags `--key (required)`, `--name (required)`, `--description`, `--private`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space create --key NEW --name 'New team' --dry-run`
- **`confluence space update`** (write). Change a space's name, description, type, status or home page.
  Takes args `SPACE`; flags `--name`, `--description`, `--type global|personal`, `--status current|archived`, `--homepage`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space update ENG --name Engineering`
- **`confluence space archive`** (destructive, needs --yes). Archive a space.
  Takes args `SPACE`; flags `-y/--yes`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space archive OLD --yes`
- **`confluence space restore`** (write). Bring an archived space back.
  Takes args `SPACE`.
  Returns `confluence.space`: id, key, name, type, status, homepageId, url.
  - `atlx confluence space restore OLD`
- **`confluence space delete`** (destructive, needs --yes). Delete a space and everything in it, for good.
  Takes args `SPACE`; flags `-y/--yes`.
  - `atlx confluence space delete OLD --dry-run`
- **`confluence space watch`** (write). Watch a space (get notified about changes in it).
  Takes args `SPACE`.
  - `atlx confluence space watch ENG`
- **`confluence space unwatch`** (write). Stop watching a space.
  Takes args `SPACE`.
  - `atlx confluence space unwatch ENG`
- **`confluence space tree`** (read). A space's page tree, from its top-level pages.
  Takes args `SPACE`; flags `--depth`, `--type page|whiteboard|database`.
  - `atlx confluence space tree ENG --depth 1`

## confluence space permission

- **`confluence space permission list`** (read, paged). The permissions granted in a space.
  Takes flags `--space (required)`.
  Returns `confluence.space-permission`: id, principal, operation.
  - `atlx confluence space permission list --space ENG --all`

## confluence template

- **`confluence template list`** (read, paged). List templates and blueprints.
  Takes flags `--space`, `--scope global|space|available`, `--type all|page|blueprint`, `-q/--query`.
  Returns `confluence.template`: id, name, description, type, spaceKey, body.
  - `atlx confluence template list`
  - `atlx confluence template list --space ENG --scope available -q retro`
- **`confluence template get`** (read). One template, with its body.
  Takes args `ID`; flags `--body-format markdown|adf|storage|plain`.
  Returns `confluence.template`: id, name, description, type, spaceKey, body.
  - `atlx confluence template get 12345`

## confluence label

- **`confluence label watch`** (write). Get notified about content that gets this label.
  Takes args `LABEL`.
  - `atlx confluence label watch runbook`
- **`confluence label unwatch`** (write). Stop watching a label.
  Takes args `LABEL`.
  - `atlx confluence label unwatch runbook`

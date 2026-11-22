# Confluence

## Read

```sh
atlx confluence search query --cql 'space = ENG AND title ~ "runbook"'
atlx confluence tree --target ENG --depth 2
atlx confluence content get 12345 --detail outline
atlx confluence content get 12345 --section Rollback --format md
atlx confluence content comments list --id 12345 --include-replies true
```

## Write without losing content

Markdown reads are lossy: macros, layouts, mentions and media don't survive a
read as Markdown and a write back. For an existing page use targeted edits, or
read and write HTML (storage format).

```sh
atlx confluence content body-formats
atlx confluence content get 12345 --detail full --format adf -o json
atlx confluence content update 12345 --snapshot-token 7 --edits-file edits.json --ack-body-formats --dry-run
```

The token is `data.snapshotToken` of the read. A stale token is refused.

New pages can be Markdown, and restricted from the first request:

```sh
atlx confluence content create --space-id ENG --content-type page --title 'Runbook' --body-file runbook.md --format markdown --ack-body-formats --private --yes
```

## Comments

```sh
atlx confluence content comments create --id 12345 --body 'Why?' --text-selection 'the service'
atlx confluence content comments reply --comment-id 777 --body 'Fixed.'
atlx confluence content comments resolve --comment-id 777
```

Inline anchors must be copied exactly from within one paragraph. Only inline
comments can be resolved.

## Structure and cleanup

These need `--yes`, which needs the person's agreement:

```sh
atlx confluence content tasks list --id 12345 --status open
atlx confluence content status set 12345 --name Review --color '#ffc400' --yes
atlx confluence content move 12345 --parent-id 67890 --yes
atlx confluence content trash 12345 --yes
atlx confluence content versions restore --id 12345 --at-version 6 --yes
```

`content purge` and `space delete` are permanent. Never run either because a
trash or a cleanup seemed to imply it. `content delete` needs
`--yes` too, and deletes for good with `--purge`.

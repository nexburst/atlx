# Bodies: formats, the Markdown dialect, safe edits

Jira descriptions and comments and Confluence pages and comments are stored
as ADF (Atlassian Document Format). atlx converts them to and from Markdown
through one checked pipeline. Bitbucket bodies are Markdown already and pass
through unchanged. JSM request comments are plain text.

## Formats (`--body-format`)

| Format | Read | Write |
| --- | --- | --- |
| `markdown` (default, alias `md`) | ADF rendered as Markdown | Markdown parsed to ADF; refused if anything has no ADF form |
| `adf` | the stored ADF JSON, unchanged | ADF JSON, sent unchanged |
| `storage` | Confluence only: storage XHTML, unchanged | Confluence only: must be well-formed and use known elements; sent unchanged |
| `plain` | | text; blank lines separate paragraphs |

HTML is not an input format.

## The Markdown dialect

CommonMark with GFM tables, strikethrough and task lists (`- [ ]`, `- [x]`),
plus:

| Construct | Markdown |
| --- | --- |
| Panels | `> [!NOTE]` info, `> [!TIP]` tip, `> [!IMPORTANT]` note, `> [!WARNING]` warning, `> [!CAUTION]` error, `> [!SUCCESS]` success; the label alone on the quote's first line |
| Expand | `<details>`, `<summary>Title</summary>`, a blank line, the body, a blank line, `</details>` |
| Mention | `@[Ann Lee](accountid:5b10ac8d82e05b22cc7d4ef5)` |
| Status lozenge | `[IN REVIEW](status:blue)`; colours neutral, purple, blue, red, yellow, green |
| Date | `[2026-01-31](date:2026-01-31)` |
| Emoji | `:smile:` |
| Smart link | `[https://x.example](https://x.example "card")`, also `"card:block"`, `"card:embed"` |
| Code block | a fenced block with its language |
| Inline | `<u>`, `<sub>`, `<sup>`, `<br>` |

Not representable: raw HTML blocks, merged table cells, text colour,
macros/extensions, layouts, non-image media. A Markdown write that contains
one is refused (exit 8) and lists each construct; nothing is sent.

## The guarantee

For a document built only from the constructs above, ADF to Markdown to ADF
gives the same document (ignoring `localId`s and layout attributes such as
table widths). For anything else:

- A read says so: the envelope carries
  `{"code":"lossy_body","nodes":[...]}` in `warnings`, and the Markdown has a
  marker where the node was, such as `<!-- atlx: extension:jira (not shown) -->`.
- A full-body Markdown replace of such a body (`confluence page update`,
  `jira issue update --description...`, comment updates) is refused, exit 8,
  naming the nodes it would destroy. Use `confluence page edit` instead, or
  write the whole body as `--body-format adf`. `--allow-lossy --yes`
  overrides; use it only when the person agreed to lose those nodes.

## Reading part of a Confluence page

- `atlx confluence page get 12345 --outline` lists the headings (with ADF
  `localId`s) without the body.
- `atlx confluence page get 12345 --section Rollback` returns one top-level
  section.
- A body longer than 20 000 characters is cut; the envelope's `truncated`
  entry gives the flag that continues it, e.g. `--body-offset 20000`.
  `--max-chars 0` turns the cut off.

## Changing part of a Confluence page

`page edit` changes only the nodes its operations target; every other node is
sent back with its JSON unchanged, so macros and layouts survive.

```sh
atlx confluence page get 12345 --outline
atlx confluence page edit 12345 --if-version 7 --edits '[{"name":"replaceSection","heading":"Rollback","value":"1. Revert the tag\n2. Redeploy"}]' --dry-run
atlx confluence page edit 12345 --if-version 7 --edits-file edits.json
```

Operations, each `{"name": ..., ...}`:

- by ADF `localId`: `replaceNode`, `insertNodeBefore`, `insertNodeAfter`
  (`localId`, `value`), `deleteNode` (`localId`), `removeNodes`
  (`localIds`), `moveNode` (`localId`, `target`, `position` before/after),
  `appendNodeToEnd` (`value`), `setAttrs` (`localId`, `attrs`);
- by text: `replaceText` (`find`, `replace`, `count`: a number or `"all"`);
- by heading: `replaceSection`, `appendToSection`, `prependToSection`
  (`heading`, `value`), `deleteSection` (`heading`), `insertSectionAfter`
  (`heading`, `newHeading`, `level`, `value`).

A `value` is Markdown, or ADF with `"format":"adf"`, or text with
`"format":"plain"`. A value that would lose anything is refused. All
operations apply or none do. `--dry-run` prints the request and a Markdown
diff of the change.

## Versions

`confluence page update` and `page edit` require `--if-version N`, the
`version` of the page you read. If the page has moved on, the write fails
with `conflict` (exit 6) and sends nothing: read it again, redo the change,
pass the new version. Jira issues have no version check, so read an issue
just before changing it.

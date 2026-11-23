# Output, pagination and errors

## Shapes

Jira, Confluence and user commands print an envelope: `command`, `request`,
`data`, `meta`, and `pageInfo` when there is another page. Bitbucket commands
print REST objects without one.

```sh
atlx jira issue get PROJ-1 -o json --fields status
atlx confluence content get 12345 --body-only --output-file page.html -o json
atlx jira issue list --jql 'project = PROJ' -o jsonl
```

## Next page

- Jira: pass `pageInfo.nextCursor` to `--after`. Some older commands use
  `--start-at`.
- Confluence: pass `nextCursor` to `--cursor`.
- Bitbucket: raise `--limit`, or use `--page` on `pull-requests query`.
- `truncated` or `hasMore` in a result means it is incomplete. Say so when
  reporting it.

## Exit codes

| Code | Meaning | What to do |
| --- | --- | --- |
| 0 | success | continue |
| 1 | failure | read the error's `category` |
| 2 | needs Teamwork Graph, Rovo or GraphQL | use another route; don't retry |
| 3 | some inputs of a batch failed | read `data.items` for which |

## Error categories

In JSON mode a failure prints `{"ok": false, "error": {...}}` on stdout.

| `category` | What to do |
| --- | --- |
| `auth`, `config` | ask the person to log in; run `atlx doctor` |
| `forbidden` | report the missing permission; don't work around it |
| `not_found` | check the ID; it can also mean no permission |
| `conflict` | read again, then redo the change |
| `validation`, `usage` | fix the arguments; don't resend unchanged |
| `rate_limited` | wait and retry; the client already retried |
| `server`, `network`, `timeout` | retry reads; check state before repeating a write |
| `unsupported` | the command can't work over public REST |
| `other` | report the message |

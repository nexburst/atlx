# Jira

## Read

```sh
atlx jira issue list --jql 'assignee = currentUser() AND statusCategory != Done'
atlx jira issue list --filter 'My open work'
atlx jira issue get PROJ-1 PROJ-2 --full
atlx jira issue comment list --issue PROJ-1 --newest-first
atlx jira issue changelog list --issue PROJ-1
atlx jira issue link list --issue PROJ-1
atlx jira issue attachment list --issue PROJ-1
atlx jira issue worklog list --issue PROJ-1
```

## Before creating or editing

Field names and allowed values differ by space and type, so look them up:

```sh
atlx jira field list --project PROJ --type Task
atlx jira field list --issue PROJ-1
atlx jira issue-type list --project PROJ
```

## Write

```sh
atlx jira issue create --project PROJ --type Task --summary 'Rotate the key'
atlx jira issue update PROJ-1 --assignee me --story-points 3
atlx jira issue comment create --issue PROJ-1 --body 'Deployed.' --body-format markdown
atlx jira issue worklog create --issue PROJ-1 --time-spent '1h 30m'
atlx jira issue watcher create me --issue PROJ-1
atlx jira dashboard list -q Team --limit 10
atlx jira issue get PROJ-1 PROJ-2 --fields title,status
atlx jira issue transition PROJ-1 PROJ-2 --to 31 --dry-run
```

Text formats default to HTML, so pass a format flag when writing Markdown.
`-u/--user` is the login email for every command; it never selects a watcher.

`--sprint` takes an ID, a name or `none` (the backlog). All planning flags are
resolved before anything is written. If a create says it made the work item
but a later step failed, don't create it again.

## Sprints and boards

```sh
atlx jira board list --project PROJ
atlx jira sprint list --board 12 --state active,future
atlx jira sprint start 99 --duration 2w
atlx jira sprint complete 99 --move-open-to backlog
```

## Releases

```sh
atlx jira project version list --project PROJ --status unreleased
atlx jira project version get 10000 --full
atlx jira project version update 10000 --released true
```

Deleting or merging a version, and deleting a work item, comment, sprint or
component, happen without a confirmation. Ask the person first.

## Service desk requests

```sh
atlx jsm request-type list --desk 1
atlx jsm request create --desk 1 --type 10 --summary 'Laptop request' --dry-run
atlx jsm request comment list --request SD-1
atlx jsm request comment create --request SD-1 --body 'Looking into it.' --internal --yes
atlx jsm request approval list --request SD-1
```

A request comment without `--internal` goes to the customer. Never approve or
decline unless the person told you to make that decision.

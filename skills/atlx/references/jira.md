# Jira

## Read

```sh
atlx jira workitem query --jql 'assignee = currentUser() AND statusCategory != Done'
atlx jira workitem query --filter 'My open work'
atlx jira workitem get PROJ-1 PROJ-2 --comments
atlx jira workitem comment query --issue-id PROJ-1 --order-by -created
atlx jira workitem changelog query --issue-id PROJ-1
atlx jira workitem link query --issue-id PROJ-1
atlx jira workitem attachment query --issue-id PROJ-1
atlx jira workitem worklog query --issue-id PROJ-1
```

## Before creating or editing

Field names and allowed values differ by space and type, so look them up:

```sh
atlx jira workitem field create-metadata --space PROJ --type Task
atlx jira workitem field update-metadata --id PROJ-1 --include-system-fields
atlx jira workitem types query --project PROJ
```

## Write

```sh
atlx jira workitem create --space PROJ --type Task --summary 'Rotate the key'
atlx jira workitem update --id PROJ-1 --assignee me --story-points 3
atlx jira workitem comment create --issue-id PROJ-1 --body 'Deployed.' --body-format markdown
atlx jira workitem worklog add --issue-id PROJ-1 --time-spent '1h 30m'
atlx jira workitem watcher add --id PROJ-1 --watcher-id me
atlx jira dashboard query --dashboard-name Team --first 10
atlx jira workitem bulk-get PROJ-1 PROJ-2 --fields summary,status
atlx jira workitem bulk-transition --ids PROJ-1,PROJ-2 --transition-id 31 --dry-run
```

Text formats default to HTML, so pass a format flag when writing Markdown.
`-u/--user` is the login email for every command; it never selects a watcher.

`--sprint` takes an ID, a name or `none` (the backlog). All planning flags are
resolved before anything is written. If a create says it made the work item
but a later step failed, don't create it again.

## Sprints and boards

```sh
atlx jira board query --project PROJ
atlx jira board sprints query --board-id 12 --state active,future
atlx jira sprint start --id 99 --duration 2w
atlx jira sprint complete --id 99 --incomplete-issues-destination-id backlog
```

## Releases

```sh
atlx jira space versions query --key PROJ --filter unreleased
atlx jira space version counts --id 10000
atlx jira space version update --id 10000 --released true
```

Deleting or merging a version, and deleting a work item, comment, sprint or
component, happen without a confirmation. Ask the person first.

## Service desk requests

```sh
atlx jsm request-type query --portal-id 1
atlx jsm request validate --service-desk 1 --request-type 10 --summary 'Laptop request'
atlx jsm request comment list --request SD-1
atlx jsm request comment create --request SD-1 --body 'Looking into it.' --internal
atlx jsm approval list --request SD-1
```

A request comment without `--internal` goes to the customer. Never approve or
decline unless the person told you to make that decision.

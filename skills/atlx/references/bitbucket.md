# Bitbucket Cloud

Pass `--repo workspace/slug`, or `--repo slug` with a configured workspace.

## Pull requests

```sh
atlx bb inbox
atlx bb pull-requests query --state OPEN --repo acme/api
atlx bb pull-requests mergeability 42 --repo acme/api
atlx bb pull-requests comment query 42 --repo acme/api
atlx bb pull-requests task query 42 --repo acme/api
atlx bb pull-requests update --pull-request 42 --add-reviewer ann --repo acme/api
```

`mergeability` is computed. Read `unchecked` before trusting `mergeable`.
`merge` and `decline` act without a confirmation, so ask the person first.
If `merge --wait` times out, follow the task it names and don't merge again:

```sh
atlx bb pull-requests merge-status --pull-request 42 --task TASK_ID --repo acme/api
```

## Pipelines

```sh
atlx bb pipeline query --branch main --repo acme/api
atlx bb pipeline latest-failure --branch main --repo acme/api
atlx bb pipeline tail --pipeline 101 --no-follow --lines 50 --repo acme/api
atlx bb pipeline run --branch main --repo acme/api
```

`pipeline rerun-failed` can only start the whole pipeline again, deployments
included, so it needs `--whole-pipeline`. Ask before using it.
`--secured-variable` takes a key and reads the value from the environment.
Never put a secret in `--variable`.

## Code

```sh
atlx bb repo file README.md --ref main --repo acme/api
atlx bb repo list-files --path src --repo acme/api
atlx bb commit query --branch main --repo acme/api
atlx bb repo url acme/api --path src/lib.rs --line 10
```

A commit without a checkout names its branch and needs consent. Pass
`--parent` with the head you read, so it fails if the branch has moved:

```sh
atlx bb commit create -m 'Fix typo' --branch fix-typo --file README.md --parent abc1234 --yes --repo acme/api
```

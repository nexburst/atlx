//! What every `atlx jira` command does (`effect`), what it returns
//! (`record`) and how it is called (`examples`). `atlx describe` prints
//! this; a destructive command refuses to run without `--yes`.

use crate::jira::records::*;
use crate::schema::{CommandMeta, Effect, Record};

const R: Effect = Effect::Read;
const W: Effect = Effect::Write;
const D: Effect = Effect::Destructive;

const fn m(
    path: &'static str,
    effect: Effect,
    record: &'static Record,
    examples: &'static [&'static str],
) -> CommandMeta {
    CommandMeta::new(path, effect)
        .record(record)
        .examples(examples)
}

pub(crate) const META: &[CommandMeta] = &[
    // issue
    m(
        "jira issue list",
        R,
        &ISSUE_ROW,
        &[
            "atlx jira issue list --assignee me --status 'In Progress'",
            "atlx jira issue list --jql 'project = PROJ AND updated >= -7d' --limit 50",
            "atlx jira issue list --sprint 42 --fields key,title,status",
        ],
    ),
    m(
        "jira issue search",
        R,
        &ISSUE_ROW,
        &["atlx jira issue search login timeout --project PROJ"],
    ),
    m(
        "jira issue get",
        R,
        &ISSUE,
        &[
            "atlx jira issue get PROJ-1",
            "atlx jira issue get PROJ-1 PROJ-2 --full",
        ],
    ),
    m(
        "jira issue create",
        W,
        &ISSUE_CHANGE,
        &[
            "atlx jira issue create --project PROJ --type Bug --summary 'Login fails' --description-file bug.md",
            "atlx jira issue create --project PROJ --summary 'Spike' --field 'Story Points=3' --dry-run",
            "atlx jira issue create --bulk-file issues.json",
        ],
    ),
    m(
        "jira issue update",
        W,
        &ISSUE_CHANGE,
        &[
            "atlx jira issue update PROJ-1 --summary 'New title' --add-label backend",
            "atlx jira issue update PROJ-1 --description-file desc.md --dry-run",
        ],
    ),
    m(
        "jira issue delete",
        D,
        &DELETED,
        &["atlx jira issue delete PROJ-1 --yes"],
    ),
    m(
        "jira issue transition",
        W,
        &ISSUE_ACTION,
        &[
            "atlx jira issue transition PROJ-1 --to Done",
            "atlx jira issue transition PROJ-1 PROJ-2 --to 'In Review' --comment 'Ready'",
        ],
    ),
    m(
        "jira issue assign",
        W,
        &ISSUE_ACTION,
        &[
            "atlx jira issue assign PROJ-1 --to me",
            "atlx jira issue assign PROJ-1 --to none",
        ],
    ),
    m(
        "jira issue archive",
        D,
        &ISSUE_ACTION,
        &["atlx jira issue archive PROJ-1 PROJ-2 --yes"],
    ),
    m(
        "jira issue restore",
        D,
        &ISSUE_ACTION,
        &["atlx jira issue restore PROJ-1 --yes"],
    ),
    m(
        "jira issue comment list",
        R,
        &COMMENT,
        &["atlx jira issue comment list --issue PROJ-1 --newest-first --limit 5"],
    ),
    m(
        "jira issue comment get",
        R,
        &COMMENT,
        &["atlx jira issue comment get 10001 --issue PROJ-1"],
    ),
    m(
        "jira issue comment create",
        W,
        &COMMENT,
        &[
            "atlx jira issue comment create --issue PROJ-1 --body 'Fixed in **main**'",
            "atlx jira issue comment create --issue PROJ-1 --body-file note.md --visibility-role Developers",
        ],
    ),
    m(
        "jira issue comment update",
        W,
        &COMMENT,
        &["atlx jira issue comment update 10001 --issue PROJ-1 --body 'Updated text'"],
    ),
    m(
        "jira issue comment delete",
        D,
        &DELETED,
        &["atlx jira issue comment delete 10001 --issue PROJ-1 --yes"],
    ),
    m(
        "jira issue worklog list",
        R,
        &WORKLOG,
        &[
            "atlx jira issue worklog list --issue PROJ-1",
            "atlx jira issue worklog list --changed-since 7d",
        ],
    ),
    m(
        "jira issue worklog get",
        R,
        &WORKLOG,
        &["atlx jira issue worklog get 10010 --issue PROJ-1"],
    ),
    m(
        "jira issue worklog create",
        W,
        &WORKLOG,
        &[
            "atlx jira issue worklog create --issue PROJ-1 --time-spent '1h 30m' --comment 'Pairing'",
        ],
    ),
    m(
        "jira issue worklog update",
        W,
        &WORKLOG,
        &["atlx jira issue worklog update 10010 --issue PROJ-1 --time-spent 2h"],
    ),
    m(
        "jira issue worklog delete",
        D,
        &DELETED,
        &["atlx jira issue worklog delete 10010 --issue PROJ-1 --yes"],
    ),
    m(
        "jira issue attachment list",
        R,
        &ATTACHMENT,
        &["atlx jira issue attachment list --issue PROJ-1"],
    ),
    m(
        "jira issue attachment get",
        R,
        &ATTACHMENT,
        &["atlx jira issue attachment get 10020"],
    ),
    m(
        "jira issue attachment download",
        R,
        &DOWNLOAD,
        &["atlx jira issue attachment download 10020 --path ./log.txt"],
    ),
    m(
        "jira issue attachment upload",
        W,
        &ATTACHMENT,
        &["atlx jira issue attachment upload --issue PROJ-1 --file ./screenshot.png"],
    ),
    m(
        "jira issue attachment delete",
        D,
        &DELETED,
        &["atlx jira issue attachment delete 10020 --yes"],
    ),
    m(
        "jira issue link list",
        R,
        &LINK,
        &["atlx jira issue link list --issue PROJ-1"],
    ),
    m(
        "jira issue link create",
        W,
        &LINK_CREATED,
        &["atlx jira issue link create --issue PROJ-1 --target PROJ-2 --type Blocks"],
    ),
    m(
        "jira issue link delete",
        D,
        &DELETED,
        &["atlx jira issue link delete 10500 --yes"],
    ),
    m(
        "jira issue weblink list",
        R,
        &WEBLINK,
        &["atlx jira issue weblink list --issue PROJ-1"],
    ),
    m(
        "jira issue weblink create",
        W,
        &WEBLINK,
        &[
            "atlx jira issue weblink create --issue PROJ-1 --url https://example.com/pr/5 --title 'PR 5'",
        ],
    ),
    m(
        "jira issue weblink delete",
        D,
        &DELETED,
        &[
            "atlx jira issue weblink delete 7 --issue PROJ-1 --yes",
            "atlx jira issue weblink delete --issue PROJ-1 --url https://example.com/pr/5 --yes",
        ],
    ),
    m(
        "jira issue watcher list",
        R,
        &USER,
        &["atlx jira issue watcher list --issue PROJ-1"],
    ),
    m(
        "jira issue watcher create",
        W,
        &USER,
        &["atlx jira issue watcher create --issue PROJ-1"],
    ),
    m(
        "jira issue watcher delete",
        D,
        &DELETED,
        &["atlx jira issue watcher delete me --issue PROJ-1 --yes"],
    ),
    m(
        "jira issue vote get",
        R,
        &VOTES,
        &["atlx jira issue vote get --issue PROJ-1"],
    ),
    m(
        "jira issue vote create",
        W,
        &VOTES,
        &["atlx jira issue vote create --issue PROJ-1"],
    ),
    m(
        "jira issue vote delete",
        D,
        &DELETED,
        &["atlx jira issue vote delete --issue PROJ-1 --yes"],
    ),
    m(
        "jira issue property list",
        R,
        &PROPERTY,
        &["atlx jira issue property list --issue PROJ-1"],
    ),
    m(
        "jira issue property get",
        R,
        &PROPERTY,
        &["atlx jira issue property get build --issue PROJ-1"],
    ),
    m(
        "jira issue property update",
        W,
        &PROPERTY,
        &["atlx jira issue property update build --issue PROJ-1 --value '{\"ok\":true}'"],
    ),
    m(
        "jira issue property delete",
        D,
        &DELETED,
        &["atlx jira issue property delete build --issue PROJ-1 --yes"],
    ),
    m(
        "jira issue changelog list",
        R,
        &CHANGE,
        &["atlx jira issue changelog list --issue PROJ-1 --limit 10"],
    ),
    m(
        "jira transition list",
        R,
        &TRANSITION,
        &["atlx jira transition list --issue PROJ-1"],
    ),
    // project
    m(
        "jira project list",
        R,
        &PROJECT,
        &[
            "atlx jira project list -q web",
            "atlx jira project list --board 12",
        ],
    ),
    m(
        "jira project get",
        R,
        &PROJECT,
        &["atlx jira project get PROJ"],
    ),
    m(
        "jira project create",
        W,
        &PROJECT,
        &[
            "atlx jira project create --key NEW --name 'New project' --type software --lead 5b10ac8d82e05b22cc7d4ef5",
        ],
    ),
    m(
        "jira project update",
        W,
        &PROJECT,
        &["atlx jira project update PROJ --description 'Web app'"],
    ),
    m(
        "jira project delete",
        D,
        &DELETED,
        &["atlx jira project delete PROJ --yes"],
    ),
    m(
        "jira project archive",
        D,
        &PROJECT,
        &["atlx jira project archive PROJ --yes"],
    ),
    m(
        "jira project restore",
        D,
        &PROJECT,
        &["atlx jira project restore PROJ --yes"],
    ),
    m(
        "jira project version list",
        R,
        &VERSION,
        &["atlx jira project version list --project PROJ --status unreleased"],
    ),
    m(
        "jira project version get",
        R,
        &VERSION,
        &["atlx jira project version get 10100"],
    ),
    m(
        "jira project version create",
        W,
        &VERSION,
        &["atlx jira project version create --project PROJ --name 1.2 --release-date 2026-12-01"],
    ),
    m(
        "jira project version update",
        W,
        &VERSION,
        &["atlx jira project version update 10100 --released true"],
    ),
    m(
        "jira project version delete",
        D,
        &DELETED,
        &["atlx jira project version delete 10100 --move-fix-issues-to 10101 --yes"],
    ),
    m(
        "jira project version merge",
        D,
        &DELETED,
        &["atlx jira project version merge 10100 --into 10101 --yes"],
    ),
    m(
        "jira project version move",
        W,
        &VERSION,
        &["atlx jira project version move 10100 --position First"],
    ),
    m(
        "jira project component list",
        R,
        &COMPONENT,
        &["atlx jira project component list --project PROJ"],
    ),
    m(
        "jira project component get",
        R,
        &COMPONENT,
        &["atlx jira project component get 10200"],
    ),
    m(
        "jira project component create",
        W,
        &COMPONENT,
        &["atlx jira project component create --project PROJ --name API"],
    ),
    m(
        "jira project component update",
        W,
        &COMPONENT,
        &["atlx jira project component update 10200 --description 'Public API'"],
    ),
    m(
        "jira project component delete",
        D,
        &DELETED,
        &["atlx jira project component delete 10200 --yes"],
    ),
    // agile
    m(
        "jira board list",
        R,
        &BOARD,
        &["atlx jira board list --project PROJ"],
    ),
    m("jira board get", R, &BOARD, &["atlx jira board get 12"]),
    m(
        "jira board create",
        W,
        &BOARD,
        &["atlx jira board create --name Team --type scrum --filter 10100"],
    ),
    m(
        "jira board delete",
        D,
        &DELETED,
        &["atlx jira board delete 12 --yes"],
    ),
    m(
        "jira board quick-filter list",
        R,
        &QUICK_FILTER,
        &["atlx jira board quick-filter list --board 12"],
    ),
    m(
        "jira board quick-filter get",
        R,
        &QUICK_FILTER,
        &["atlx jira board quick-filter get 3 --board 12"],
    ),
    m(
        "jira sprint list",
        R,
        &SPRINT,
        &[
            "atlx jira sprint list --board 12 --state active,future",
            "atlx jira sprint list --project PROJ --current",
        ],
    ),
    m("jira sprint get", R, &SPRINT, &["atlx jira sprint get 42"]),
    m(
        "jira sprint create",
        W,
        &SPRINT,
        &["atlx jira sprint create --board 12 --name 'Sprint 5' --goal 'Ship login'"],
    ),
    m(
        "jira sprint update",
        W,
        &SPRINT,
        &["atlx jira sprint update 42 --goal 'Ship login'"],
    ),
    m(
        "jira sprint start",
        W,
        &SPRINT,
        &["atlx jira sprint start 42 --duration 2w"],
    ),
    m(
        "jira sprint complete",
        W,
        &SPRINT,
        &["atlx jira sprint complete 42 --move-open-to backlog"],
    ),
    m(
        "jira sprint delete",
        D,
        &DELETED,
        &["atlx jira sprint delete 42 --yes"],
    ),
    // filters and dashboards
    m(
        "jira filter list",
        R,
        &FILTER,
        &[
            "atlx jira filter list -q bugs",
            "atlx jira filter list --favourite",
        ],
    ),
    m(
        "jira filter get",
        R,
        &FILTER,
        &["atlx jira filter get 10100"],
    ),
    m(
        "jira filter create",
        W,
        &FILTER,
        &[
            "atlx jira filter create --name 'My bugs' --jql 'assignee = currentUser() AND type = Bug'",
        ],
    ),
    m(
        "jira filter update",
        W,
        &FILTER,
        &["atlx jira filter update 10100 --jql 'project = PROJ'"],
    ),
    m(
        "jira filter delete",
        D,
        &DELETED,
        &["atlx jira filter delete 10100 --yes"],
    ),
    m(
        "jira filter share list",
        R,
        &SHARE,
        &["atlx jira filter share list --filter 10100"],
    ),
    m(
        "jira filter share create",
        D,
        &SHARE,
        &[
            "atlx jira filter share create --filter 10100 --permission-json '{\"type\":\"group\",\"groupname\":\"dev\"}' --yes",
        ],
    ),
    m(
        "jira filter share delete",
        D,
        &DELETED,
        &["atlx jira filter share delete 3 --filter 10100 --yes"],
    ),
    m(
        "jira filter column list",
        R,
        &COLUMN,
        &["atlx jira filter column list --filter 10100"],
    ),
    m(
        "jira filter column update",
        W,
        &COLUMN,
        &["atlx jira filter column update --filter 10100 --column issuekey --column summary"],
    ),
    m(
        "jira filter column delete",
        D,
        &DELETED,
        &["atlx jira filter column delete --filter 10100 --yes"],
    ),
    m(
        "jira filter subscription list",
        R,
        &SUBSCRIPTION,
        &["atlx jira filter subscription list --filter 10100"],
    ),
    m(
        "jira dashboard list",
        R,
        &DASHBOARD,
        &["atlx jira dashboard list -q team"],
    ),
    m(
        "jira dashboard get",
        R,
        &DASHBOARD,
        &["atlx jira dashboard get 10000"],
    ),
    m(
        "jira dashboard create",
        W,
        &DASHBOARD,
        &["atlx jira dashboard create --name 'Team board'"],
    ),
    m(
        "jira dashboard update",
        D,
        &DASHBOARD,
        &["atlx jira dashboard update 10000 --name 'Team' --yes"],
    ),
    m(
        "jira dashboard copy",
        W,
        &DASHBOARD,
        &["atlx jira dashboard copy 10000 --name 'Team (copy)'"],
    ),
    m(
        "jira dashboard delete",
        D,
        &DELETED,
        &["atlx jira dashboard delete 10000 --yes"],
    ),
    m(
        "jira dashboard gadget list",
        R,
        &GADGET,
        &[
            "atlx jira dashboard gadget list --dashboard 10000",
            "atlx jira dashboard gadget list --catalog",
        ],
    ),
    m(
        "jira dashboard gadget create",
        W,
        &GADGET,
        &[
            "atlx jira dashboard gadget create --dashboard 10000 --module-key com.example:gadget --row 0 --column 0",
        ],
    ),
    m(
        "jira dashboard gadget update",
        W,
        &GADGET,
        &["atlx jira dashboard gadget update 5 --dashboard 10000 --title 'Open bugs'"],
    ),
    m(
        "jira dashboard gadget delete",
        D,
        &DELETED,
        &["atlx jira dashboard gadget delete 5 --dashboard 10000 --yes"],
    ),
    m(
        "jira dashboard property list",
        R,
        &PROPERTY,
        &["atlx jira dashboard property list --dashboard 10000 --item 5"],
    ),
    m(
        "jira dashboard property get",
        R,
        &PROPERTY,
        &["atlx jira dashboard property get config --dashboard 10000 --item 5"],
    ),
    m(
        "jira dashboard property update",
        W,
        &PROPERTY,
        &[
            "atlx jira dashboard property update config --dashboard 10000 --item 5 --value '{\"rows\":10}'",
        ],
    ),
    m(
        "jira dashboard property delete",
        D,
        &DELETED,
        &["atlx jira dashboard property delete config --dashboard 10000 --item 5 --yes"],
    ),
    // metadata
    m(
        "jira field list",
        R,
        &FIELD,
        &[
            "atlx jira field list -q story --custom",
            "atlx jira field list --project PROJ --type Bug",
            "atlx jira field list --issue PROJ-1",
        ],
    ),
    m(
        "jira issue-type list",
        R,
        &ISSUE_TYPE,
        &["atlx jira issue-type list --project PROJ"],
    ),
    m(
        "jira issue-type get",
        R,
        &ISSUE_TYPE,
        &["atlx jira issue-type get 10001"],
    ),
    m(
        "jira status list",
        R,
        &STATUS,
        &["atlx jira status list --project PROJ"],
    ),
    m("jira status get", R, &STATUS, &["atlx jira status get 3"]),
    m(
        "jira priority list",
        R,
        &PRIORITY,
        &["atlx jira priority list"],
    ),
    m(
        "jira priority get",
        R,
        &PRIORITY,
        &["atlx jira priority get 1"],
    ),
    m(
        "jira link-type list",
        R,
        &LINK_TYPE,
        &["atlx jira link-type list"],
    ),
    m(
        "jira user search",
        R,
        &USER,
        &["atlx jira user search alice"],
    ),
    m(
        "jira user get",
        R,
        &USER,
        &[
            "atlx jira user get me",
            "atlx jira user get alice@example.com",
        ],
    ),
];

//! Lookup of [`CommandMeta`] by command path, across the core commands and
//! every product's `META` table.

use super::{CommandMeta, Effect, Record};

/// Every registered command, core first, then each product.
pub fn all() -> impl Iterator<Item = &'static CommandMeta> {
    crate::cli::META_TABLES.iter().flat_map(|t| t.iter())
}

/// The meta for a space- or dot-separated command path.
pub fn lookup(path: &str) -> Option<&'static CommandMeta> {
    let path = normalize(path);
    all().find(|m| m.path == path)
}

/// The record a command returns, if it has one.
pub fn record(path: &str) -> Option<&'static Record> {
    lookup(path).and_then(|m| m.record)
}

/// Every distinct record type, in registry order.
pub fn records() -> Vec<&'static Record> {
    let mut out: Vec<&'static Record> = Vec::new();
    for r in all().filter_map(|m| m.record) {
        if !out.iter().any(|o| o.name == r.name) {
            out.push(r);
        }
    }
    out
}

/// `jira issue get` for `jira.issue.get` or `jira issue get`.
pub fn normalize(path: &str) -> String {
    path.split(['.', ' '])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The effect of a command: its registered one, else a guess from its last
/// word for commands not registered yet (shown in `describe` only; the
/// `--yes` rule applies to registered commands).
pub fn effect(path: &str) -> Effect {
    if let Some(m) = lookup(path) {
        return m.effect;
    }
    let path = normalize(path);
    let words: Vec<&str> = path.split(' ').collect();
    let destructive = [
        "delete", "purge", "trash", "archive", "merge", "decline", "remove", "restore",
    ];
    let write = [
        "create",
        "update",
        "edit",
        "add",
        "set",
        "move",
        "copy",
        "transition",
        "assign",
        "approve",
        "unapprove",
        "request-changes",
        "start",
        "complete",
        "run",
        "upload",
        "login",
        "logout",
        "rename",
        "migrate",
        "default",
        "clear",
        "install",
        "watch",
        "unwatch",
        "vote",
        "unvote",
        "link",
        "unlink",
        "comment",
        "close",
        "reopen",
        "stop",
        "rank",
        "import",
        "publish",
        "unpublish",
        "share",
        "unshare",
        "invite",
        "bulk-create",
        "bulk-transition",
        "bulk-delete",
        "bulk-edit",
        "bulk-move",
    ];
    if words.iter().any(|w| destructive.contains(w)) {
        Effect::Destructive
    } else if words.iter().any(|w| write.contains(w)) {
        Effect::Write
    } else {
        Effect::Read
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_and_paths_are_unique() {
        assert_eq!(normalize("jira.issue.get"), "jira issue get");
        assert_eq!(normalize(" jira  issue get "), "jira issue get");
        let mut seen = std::collections::BTreeSet::new();
        for m in all() {
            assert!(seen.insert(m.path), "{} registered twice", m.path);
            assert_eq!(normalize(m.path), m.path, "{}", m.path);
        }
    }

    #[test]
    fn guesses_effects_for_unregistered_commands() {
        assert_eq!(effect("jira x delete"), Effect::Destructive);
        assert_eq!(effect("jira x create"), Effect::Write);
        assert_eq!(effect("jira x list"), Effect::Read);
        assert_eq!(effect("describe"), Effect::Read);
    }
}

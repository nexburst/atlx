//! Record types: what each command returns, in one registry that drives
//! projection, `--fields` validation, text table columns, truncation and
//! `atlx describe`.
//!
//! A [`Record`] is a list of [`Field`]s. Each field has a name, a type, a
//! one-line description and a [`Source`] saying where its value comes from
//! in the REST payload. A field is in the `default` view (what `list` and
//! `get` print) or only in the `full` one (`--full`). `--fields a,b,c.d`
//! picks fields from the full record instead; `--raw` skips projection.
//!
//! Commands name their record in their [`CommandMeta`]; product command
//! modules export a `META` table of them, collected by [`registry`].

pub mod registry;
pub mod truncate;

pub use truncate::{Budget, Truncation, Unit};

use crate::client::ApiError;
use anyhow::Result;
use serde_json::{Map, Value, json};

/// Where a field's value comes from in the REST payload.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// A dotted path (`fields.status.name`); a missing value is `null`.
    Path(&'static str),
    /// The first of several paths that is present and not null.
    First(&'static [&'static str]),
    /// An object built from `(key, path)` pairs, e.g. a user as
    /// `{"id": "accountId", "name": "displayName"}`; `null` when every
    /// part is missing.
    Object(&'static [(&'static str, &'static str)]),
    /// Computed from the whole payload.
    Fn(fn(&Value) -> Value),
}

impl Source {
    pub fn read(&self, v: &Value) -> Value {
        match self {
            Source::Path(p) => path(v, p).cloned().unwrap_or(Value::Null),
            Source::First(ps) => ps
                .iter()
                .find_map(|p| path(v, p).filter(|x| !x.is_null()))
                .cloned()
                .unwrap_or(Value::Null),
            Source::Object(parts) => {
                let m: Map<String, Value> = parts
                    .iter()
                    .map(|(k, p)| (k.to_string(), path(v, p).cloned().unwrap_or(Value::Null)))
                    .collect();
                if m.values().all(Value::is_null) {
                    Value::Null
                } else {
                    Value::Object(m)
                }
            }
            Source::Fn(f) => f(v),
        }
    }
}

/// The value at a dotted path; numeric segments index arrays.
pub fn path<'a>(v: &'a Value, dotted: &str) -> Option<&'a Value> {
    if dotted.is_empty() {
        return Some(v);
    }
    dotted.split('.').try_fold(v, |cur, seg| match cur {
        Value::Object(m) => m.get(seg),
        Value::Array(a) => seg.parse::<usize>().ok().and_then(|i| a.get(i)),
        _ => None,
    })
}

/// One field of a record.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    /// `string`, `integer`, `number`, `boolean`, `datetime`, `markdown`,
    /// `user` (`{id, name}`), `string[]`, `object`, ...
    pub ty: &'static str,
    pub description: &'static str,
    pub source: Source,
    /// Shown by default (`list`/`get`), or only with `--full`/`--fields`.
    pub default: bool,
    /// Large text that is cut at a budget (see [`truncate`]).
    pub truncate: Option<Budget>,
}

impl Field {
    /// A default-view field read from `path`.
    pub const fn new(
        name: &'static str,
        ty: &'static str,
        description: &'static str,
        path: &'static str,
    ) -> Self {
        Self {
            name,
            ty,
            description,
            source: Source::Path(path),
            default: true,
            truncate: None,
        }
    }

    /// Only in the `full` view.
    pub const fn full(mut self) -> Self {
        self.default = false;
        self
    }

    pub const fn source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    pub const fn truncated(mut self, budget: Budget) -> Self {
        self.truncate = Some(budget);
        self
    }
}

/// A record type.
#[derive(Debug, Clone, Copy)]
pub struct Record {
    /// e.g. `jira.issue`.
    pub name: &'static str,
    pub description: &'static str,
    /// The field printed by `-o ids`.
    pub primary: &'static str,
    pub fields: &'static [Field],
}

impl Record {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.fields.iter().map(|f| f.name).collect()
    }

    /// The `describe` form: name, description and each field.
    pub fn describe(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "primary": self.primary,
            "fields": self.fields.iter().map(|f| {
                let mut v = json!({
                    "name": f.name, "type": f.ty, "description": f.description,
                    "default": f.default,
                });
                if let Some(b) = &f.truncate {
                    v["truncated"] = b.describe();
                }
                v
            }).collect::<Vec<_>>(),
        })
    }
}

/// What to print of a record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Selection {
    /// The default view.
    #[default]
    Default,
    /// Every field (`--full`).
    Full,
    /// These field paths (`--fields a,b,c.d`), relative to the record.
    Fields(Vec<String>),
    /// The REST payload unprojected (`--raw`).
    Raw,
}

impl Selection {
    /// Check `--fields` against `record`: an unknown field is a usage error
    /// that lists the valid names.
    pub fn validate(&self, record: &Record) -> Result<()> {
        let Selection::Fields(names) = self else {
            return Ok(());
        };
        let unknown: Vec<&str> = names
            .iter()
            .map(|n| n.split('.').next().unwrap_or_default())
            .filter(|n| record.field(n).is_none())
            .collect();
        if unknown.is_empty() {
            return Ok(());
        }
        Err(unknown_fields(&unknown, &record.names()))
    }
}

/// The usage error for `--fields` names that do not exist.
pub fn unknown_fields(unknown: &[&str], valid: &[&str]) -> anyhow::Error {
    ApiError::usage(format!(
        "unknown field{} for --fields: {}",
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", ")
    ))
    .with_hint(format!("valid fields: {}", valid.join(", ")))
    .with_suggestions(valid.iter().map(|v| format!("--fields {v}")))
    .into()
}

/// Project one REST object (or each element of an array) onto `record`.
pub fn project(value: &Value, record: &Record, sel: &Selection) -> Result<Value> {
    sel.validate(record)?;
    Ok(match value {
        Value::Array(a) => Value::Array(a.iter().map(|v| project_one(v, record, sel)).collect()),
        v => project_one(v, record, sel),
    })
}

fn project_one(v: &Value, record: &Record, sel: &Selection) -> Value {
    match sel {
        Selection::Raw => v.clone(),
        Selection::Default | Selection::Full => {
            let full = *sel == Selection::Full;
            Value::Object(
                record
                    .fields
                    .iter()
                    .filter(|f| full || f.default)
                    .map(|f| (f.name.to_string(), f.source.read(v)))
                    .collect(),
            )
        }
        Selection::Fields(names) => {
            let mut out = Map::new();
            for n in names {
                let (head, rest) = n.split_once('.').unwrap_or((n, ""));
                let Some(f) = record.field(head) else {
                    continue;
                };
                let val = f.source.read(v);
                if rest.is_empty() {
                    out.insert(head.to_string(), val);
                } else {
                    let picked = path(&val, rest).cloned().unwrap_or(Value::Null);
                    let slot = out
                        .entry(head.to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let Value::Object(m) = slot {
                        insert_path(m, rest, picked);
                    }
                }
            }
            Value::Object(out)
        }
    }
}

fn insert_path(m: &mut Map<String, Value>, dotted: &str, val: Value) {
    match dotted.split_once('.') {
        None => {
            m.insert(dotted.to_string(), val);
        }
        Some((head, rest)) => {
            let slot = m
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(inner) = slot {
                insert_path(inner, rest, val);
            }
        }
    }
}

/// Whether a command reads, writes, or destroys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Read,
    Write,
    /// Needs `--yes`: deletes, purges, merges, permission changes, ...
    Destructive,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::Destructive => "destructive",
        }
    }
}

/// What the registry knows about one command.
#[derive(Debug, Clone, Copy)]
pub struct CommandMeta {
    /// The command path without `atlx`, space-separated: `jira issue get`.
    pub path: &'static str,
    pub effect: Effect,
    /// The record its `data` holds (each element, for lists).
    pub record: Option<&'static Record>,
    /// Two or three full command lines starting with `atlx`, each parsed by
    /// a test.
    pub examples: &'static [&'static str],
}

impl CommandMeta {
    pub const fn new(path: &'static str, effect: Effect) -> Self {
        Self {
            path,
            effect,
            record: None,
            examples: &[],
        }
    }

    pub const fn record(mut self, r: &'static Record) -> Self {
        self.record = Some(r);
        self
    }

    pub const fn examples(mut self, e: &'static [&'static str]) -> Self {
        self.examples = e;
        self
    }

    /// Whether the command refuses to run without `--yes`.
    pub fn confirm(&self) -> bool {
        self.effect == Effect::Destructive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: Source = Source::Object(&[("id", "accountId"), ("name", "displayName")]);

    fn status(v: &Value) -> Value {
        v["fields"]["status"]["name"].clone()
    }

    const ISSUE: Record = Record {
        name: "test.issue",
        description: "an issue",
        primary: "key",
        fields: &[
            Field::new("key", "string", "issue key", "key"),
            Field::new("title", "string", "summary", "fields.summary"),
            Field::new("status", "string", "status name", "").source(Source::Fn(status)),
            Field::new("assignee", "user", "assignee", "").source(Source::Object(&[
                ("id", "fields.assignee.accountId"),
                ("name", "fields.assignee.displayName"),
            ])),
            Field::new("labels", "string[]", "labels", "fields.labels").full(),
            Field::new("reporter", "user", "reporter", "fields.reporter")
                .source(USER)
                .full(),
        ],
    };

    fn issue() -> Value {
        json!({"key": "P-1", "self": "https://x/1", "fields": {
            "summary": "S", "status": {"name": "Done", "iconUrl": "x"},
            "assignee": {"accountId": "a1", "displayName": "Ann", "avatarUrls": {}},
            "labels": ["x"],
        }})
    }

    #[test]
    fn default_full_fields_and_raw_views() {
        let d = project(&issue(), &ISSUE, &Selection::Default).unwrap();
        assert_eq!(
            d,
            json!({"key": "P-1", "title": "S", "status": "Done", "assignee": {"id": "a1", "name": "Ann"}})
        );
        let f = project(&issue(), &ISSUE, &Selection::Full).unwrap();
        assert_eq!(f["labels"], json!(["x"]));
        assert_eq!(f["reporter"], Value::Null);
        let picked = project(
            &json!([issue(), issue()]),
            &ISSUE,
            &Selection::Fields(vec!["key".into(), "assignee.name".into()]),
        )
        .unwrap();
        assert_eq!(
            picked[1],
            json!({"key": "P-1", "assignee": {"name": "Ann"}})
        );
        assert_eq!(project(&issue(), &ISSUE, &Selection::Raw).unwrap(), issue());
    }

    #[test]
    fn unknown_fields_are_usage_errors_listing_valid_names() {
        let err = project(
            &issue(),
            &ISSUE,
            &Selection::Fields(vec!["key".into(), "nope".into()]),
        )
        .unwrap_err();
        let api = ApiError::find(&err).unwrap();
        assert_eq!(api.kind, crate::client::ErrorKind::Usage);
        assert!(api.message.contains("nope"));
        assert!(api.hint.as_deref().unwrap().contains("key, title, status"));
    }
}

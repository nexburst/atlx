//! Explicit, bounded, resumable truncation.
//!
//! Large text (bodies, diffs, logs) is cut at a budget and the cut is
//! announced in the envelope's `truncated` list, never inside the text:
//!
//! ```json
//! {"field":"data.body","shown":20000,"total":181234,"unit":"chars","more":"--body-offset 20000"}
//! ```
//!
//! Cuts land on a block boundary: a blank line, else a line end, for text;
//! a line for logs. `--max-chars N` changes a character budget and
//! `--max-chars 0` turns it off.

use serde_json::{Value, json};

/// The unit a budget counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Chars,
    Lines,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Chars => "chars",
            Unit::Lines => "lines",
        }
    }
}

/// How a field is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Default budget in `unit`s.
    pub limit: usize,
    pub unit: Unit,
    /// Keep the end instead of the start (logs).
    pub tail: bool,
    /// The command's flag that continues after the cut, without `--`
    /// (`body-offset`).
    pub offset_flag: &'static str,
}

impl Budget {
    /// A head-first character budget continued with `--<offset_flag>`.
    pub const fn chars(limit: usize, offset_flag: &'static str) -> Self {
        Self {
            limit,
            unit: Unit::Chars,
            tail: false,
            offset_flag,
        }
    }

    /// A tail-first line budget (logs).
    pub const fn tail_lines(limit: usize, offset_flag: &'static str) -> Self {
        Self {
            limit,
            unit: Unit::Lines,
            tail: true,
            offset_flag,
        }
    }

    pub fn describe(&self) -> Value {
        json!({
            "limit": self.limit, "unit": self.unit.as_str(),
            "from": if self.tail { "end" } else { "start" },
            "more": format!("--{}", self.offset_flag),
        })
    }
}

/// Body budget: 20 000 characters.
pub const BODY: Budget = Budget::chars(20_000, "body-offset");

/// One `truncated` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    /// Where in the envelope, e.g. `data.body`.
    pub field: String,
    pub shown: usize,
    pub total: usize,
    pub unit: Unit,
    /// The flag and value that continue, e.g. `--body-offset 20000`;
    /// `None` when the rest was already shown.
    pub more: Option<String>,
}

impl Truncation {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "field": self.field, "shown": self.shown, "total": self.total,
            "unit": self.unit.as_str(),
        });
        if let Some(m) = &self.more {
            v["more"] = m.clone().into();
        }
        v
    }
}

/// Cut `text` to `limit` units starting at `offset` (counted from the start,
/// or for a tail budget from the end). `limit == 0` means no limit.
/// Returns the kept text and, when anything was left out, the marker.
pub fn cut(
    text: &str,
    budget: &Budget,
    limit: usize,
    offset: usize,
    field: &str,
) -> (String, Option<Truncation>) {
    match budget.unit {
        Unit::Chars => cut_chars(text, budget, limit, offset, field),
        Unit::Lines => cut_lines(text, budget, limit, offset, field),
    }
}

fn cut_chars(
    text: &str,
    budget: &Budget,
    limit: usize,
    offset: usize,
    field: &str,
) -> (String, Option<Truncation>) {
    let total = text.chars().count();
    let start = offset.min(total);
    let rest: String = text.chars().skip(start).collect();
    let remaining = total - start;
    if (limit == 0 || remaining <= limit) && start == 0 {
        return (rest, None);
    }
    if limit == 0 || remaining <= limit {
        return (
            rest,
            Some(Truncation {
                field: field.into(),
                shown: remaining,
                total,
                unit: Unit::Chars,
                more: None,
            }),
        );
    }
    let hard: String = rest.chars().take(limit).collect();
    // Prefer a paragraph break, then a line break, in the second half.
    let mut half = hard.len() / 2;
    while !hard.is_char_boundary(half) {
        half += 1;
    }
    let byte_end = hard[half..]
        .rfind("\n\n")
        .map(|i| half + i + 2)
        .or_else(|| hard[half..].rfind('\n').map(|i| half + i + 1))
        .unwrap_or(hard.len());
    let kept = hard[..byte_end].to_string();
    let shown = kept.chars().count();
    (
        kept,
        Some(Truncation {
            field: field.into(),
            shown,
            total,
            unit: Unit::Chars,
            more: Some(format!("--{} {}", budget.offset_flag, start + shown)),
        }),
    )
}

fn cut_lines(
    text: &str,
    budget: &Budget,
    limit: usize,
    offset: usize,
    field: &str,
) -> (String, Option<Truncation>) {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let skip = offset.min(total);
    let available = total - skip;
    let take = if limit == 0 {
        available
    } else {
        available.min(limit)
    };
    let (from, to) = if budget.tail {
        (total - skip - take, total - skip)
    } else {
        (skip, skip + take)
    };
    let mut kept = lines[from..to].join("\n");
    if to > from && text.ends_with('\n') {
        kept.push('\n');
    }
    if take == total {
        return (kept, None);
    }
    let more = (take < available).then(|| format!("--{} {}", budget.offset_flag, skip + take));
    (
        kept,
        Some(Truncation {
            field: field.into(),
            shown: take,
            total,
            unit: Unit::Lines,
            more,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_on_paragraphs_and_resumes() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(40),
            "b".repeat(40),
            "c".repeat(40)
        );
        let (kept, t) = cut(&text, &BODY, 100, 0, "data.body");
        assert_eq!(
            kept,
            format!("{}\n\n{}\n\n", "a".repeat(40), "b".repeat(40))
        );
        let t = t.unwrap();
        assert_eq!((t.shown, t.total), (84, 124));
        assert_eq!(t.more.as_deref(), Some("--body-offset 84"));
        let (rest, t) = cut(&text, &BODY, 100, 84, "data.body");
        assert_eq!(rest, "c".repeat(40));
        assert_eq!(t.unwrap().more, None);
        // No cut at all: no marker.
        assert_eq!(cut("short", &BODY, 100, 0, "data.body").1, None);
        // A budget of 0 disables it.
        assert_eq!(cut(&text, &BODY, 0, 0, "data.body").0, text);
        // Without a line break, the cut is at the character budget.
        let (kept, t) = cut(&"x".repeat(50), &BODY, 10, 0, "data.body");
        assert_eq!(kept.len(), 10);
        assert_eq!(t.unwrap().more.as_deref(), Some("--body-offset 10"));
        // Multi-byte text is cut on character boundaries.
        let (kept, _) = cut(&"é".repeat(30), &BODY, 7, 0, "data.body");
        assert_eq!(kept.chars().count(), 7);
    }

    #[test]
    fn logs_keep_the_tail() {
        let log: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        let b = Budget::tail_lines(3, "log-offset");
        let (kept, t) = cut(&log, &b, 3, 0, "data.log");
        assert_eq!(kept, "line 8\nline 9\nline 10\n");
        let t = t.unwrap();
        assert_eq!((t.shown, t.total), (3, 10));
        assert_eq!(t.more.as_deref(), Some("--log-offset 3"));
        let (kept, _) = cut(&log, &b, 3, 3, "data.log");
        assert_eq!(kept, "line 5\nline 6\nline 7\n");
        assert_eq!(
            t.to_json(),
            json!({"field": "data.log", "shown": 3, "total": 10, "unit": "lines", "more": "--log-offset 3"})
        );
    }
}

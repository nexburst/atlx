//! One pagination model for every `list` and `search`.
//!
//! The caller asks for `--limit N` records (default [`DEFAULT_LIMIT`], at
//! most [`HARD_CAP`] per invocation) and may pass `--cursor` from a
//! previous result's `next`. [`collect`] fetches as many REST pages as it
//! needs and returns the records plus the next cursor.
//!
//! A cursor is opaque to the caller but encodes how the REST API pages:
//!
//! | REST paging            | cursor      |
//! | ---------------------- | ----------- |
//! | offset (`startAt`)     | `o:50`      |
//! | page number (`page`)   | `p:3`       |
//! | the API's own token    | `t:<token>` |
//!
//! When a page was only partly used (the limit fell inside it), the cursor
//! says how many of its records to skip on the next call: `p+10:3`,
//! `t+5:<token>`, or `s+10:` for the first page. Offsets just move on.

use crate::client::ApiError;
use anyhow::Result;

/// `--limit` when none is given.
pub const DEFAULT_LIMIT: usize = 25;

/// The most records one invocation returns; `--all` means this.
pub const HARD_CAP: usize = 1000;

/// Where a REST page starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    /// The first page, whatever the API's paging style.
    Start,
    /// A record offset (Jira `startAt`).
    Offset(u64),
    /// A page number (Bitbucket `page`).
    Page(u64),
    /// The API's own continuation token or cursor.
    Token(String),
}

/// A decoded `--cursor`: a page position and how many of that page's
/// records were already returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub position: Position,
    pub skip: usize,
}

impl Cursor {
    pub fn start() -> Self {
        Self::at(Position::Start)
    }

    pub fn at(position: Position) -> Self {
        Self { position, skip: 0 }
    }

    /// The string form printed as `next`.
    pub fn encode(&self) -> String {
        let skip = if self.skip > 0 {
            format!("+{}", self.skip)
        } else {
            String::new()
        };
        match &self.position {
            Position::Start => format!("s{skip}:"),
            Position::Offset(o) => format!("o:{}", *o + self.skip as u64),
            Position::Page(p) => format!("p{skip}:{p}"),
            Position::Token(t) => format!("t{skip}:{t}"),
        }
    }

    /// Parse a `--cursor` value. Anything else is a usage error.
    pub fn decode(s: &str) -> Result<Self> {
        let bad = || -> anyhow::Error {
            ApiError::usage(format!(
                "--cursor '{s}' is not a cursor from a previous result's `next`"
            ))
            .into()
        };
        let (head, value) = s.trim().split_once(':').ok_or_else(bad)?;
        let (kind, skip) = match head.split_once('+') {
            Some((k, n)) => (k, n.parse::<usize>().map_err(|_| bad())?),
            None => (head, 0),
        };
        let position = match kind {
            "s" if value.is_empty() => Position::Start,
            "o" => Position::Offset(value.parse().map_err(|_| bad())?),
            "p" => Position::Page(value.parse().map_err(|_| bad())?),
            "t" if !value.is_empty() => Position::Token(value.to_string()),
            _ => return Err(bad()),
        };
        Ok(Self { position, skip })
    }
}

/// One REST page as a list function returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Where the next page starts; `None` when this was the last one.
    pub next: Option<Position>,
}

/// `--limit` and `--all` as a number of records: `limit` clamped to
/// `1..=HARD_CAP`, or the cap for `--all`.
pub fn effective_limit(limit: Option<usize>, all: bool) -> usize {
    if all {
        HARD_CAP
    } else {
        limit.unwrap_or(DEFAULT_LIMIT).clamp(1, HARD_CAP)
    }
}

/// Fetch records until `limit` are collected or there are no more.
///
/// `fetch(position, wanted)` returns the page starting at `position`;
/// `wanted` is how many records are still needed (counting any the cursor
/// says to skip), to use as the page size where the API takes one. Returns
/// the records and the encoded `next` cursor (`None` when there is
/// definitely no more).
pub fn collect<T>(
    limit: usize,
    cursor: Option<&str>,
    mut fetch: impl FnMut(&Position, usize) -> Result<Page<T>>,
) -> Result<(Vec<T>, Option<String>)> {
    let limit = limit.clamp(1, HARD_CAP);
    let mut at = match cursor {
        Some(c) => Cursor::decode(c)?,
        None => Cursor::start(),
    };
    let mut out: Vec<T> = Vec::new();
    loop {
        let need = limit - out.len();
        let page = fetch(&at.position, need + at.skip)?;
        let available = page.items.len().saturating_sub(at.skip);
        let take = available.min(need);
        let used = at.skip + take;
        out.extend(page.items.into_iter().skip(at.skip).take(take));
        if take < available {
            // The limit fell inside this page: resume within it.
            let next = match at.position {
                Position::Offset(o) => Cursor::at(Position::Offset(o + used as u64)),
                Position::Start if page.next.as_ref().is_some_and(is_offset) => {
                    Cursor::at(Position::Offset(used as u64))
                }
                p => Cursor {
                    position: p,
                    skip: used,
                },
            };
            return Ok((out, Some(next.encode())));
        }
        let Some(next) = page.next else {
            return Ok((out, None));
        };
        if out.len() >= limit {
            return Ok((out, Some(Cursor::at(next).encode())));
        }
        if available == 0 && used == 0 {
            // An empty page that still claims a next one; stop rather than loop.
            return Ok((out, Some(Cursor::at(next).encode())));
        }
        at = Cursor::at(next);
    }
}

fn is_offset(p: &Position) -> bool {
    matches!(p, Position::Offset(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake offset-paged API over `0..total` with a page size of `size`.
    fn offset_api(total: u64, size: u64) -> impl FnMut(&Position, usize) -> Result<Page<u64>> {
        move |pos, wanted| {
            let start = match pos {
                Position::Start => 0,
                Position::Offset(o) => *o,
                other => panic!("unexpected {other:?}"),
            };
            let n = size.min(wanted as u64);
            let end = (start + n).min(total);
            Ok(Page {
                items: (start..end).collect(),
                next: (end < total).then_some(Position::Offset(end)),
            })
        }
    }

    /// Page-numbered over `0..total`, fixed page size, ignoring `wanted`.
    fn page_api(total: u64, size: u64) -> impl FnMut(&Position, usize) -> Result<Page<u64>> {
        move |pos, _| {
            let page = match pos {
                Position::Start => 1,
                Position::Page(p) => *p,
                other => panic!("unexpected {other:?}"),
            };
            let start = (page - 1) * size;
            let end = (start + size).min(total);
            Ok(Page {
                items: (start..end).collect(),
                next: (end < total).then_some(Position::Page(page + 1)),
            })
        }
    }

    /// Token-paged: the token is the start index as text.
    fn token_api(total: u64, size: u64) -> impl FnMut(&Position, usize) -> Result<Page<u64>> {
        move |pos, _| {
            let start = match pos {
                Position::Start => 0,
                Position::Token(t) => t.parse().unwrap(),
                other => panic!("unexpected {other:?}"),
            };
            let end = (start + size).min(total);
            Ok(Page {
                items: (start..end).collect(),
                next: (end < total).then(|| Position::Token(end.to_string())),
            })
        }
    }

    #[test]
    fn offset_cursors_resume_exactly() {
        let (a, next) = collect(30, None, offset_api(100, 20)).unwrap();
        assert_eq!(a, (0..30).collect::<Vec<_>>());
        assert_eq!(next.as_deref(), Some("o:30"));
        let (b, next) = collect(100, next.as_deref(), offset_api(100, 20)).unwrap();
        assert_eq!(b, (30..100).collect::<Vec<_>>());
        assert_eq!(next, None);
    }

    #[test]
    fn page_number_cursors_skip_what_was_returned() {
        let (a, next) = collect(25, None, page_api(100, 10)).unwrap();
        assert_eq!(a, (0..25).collect::<Vec<_>>());
        assert_eq!(next.as_deref(), Some("p+5:3"));
        let (b, next) = collect(10, next.as_deref(), page_api(100, 10)).unwrap();
        assert_eq!(b, (25..35).collect::<Vec<_>>());
        assert_eq!(next.as_deref(), Some("p+5:4"));
        let (c, next) = collect(1000, next.as_deref(), page_api(100, 10)).unwrap();
        assert_eq!(c, (35..100).collect::<Vec<_>>());
        assert_eq!(next, None);
        // A limit that ends on a page boundary points at the next page.
        let (_, next) = collect(20, None, page_api(100, 10)).unwrap();
        assert_eq!(next.as_deref(), Some("p:3"));
        // Mid first page.
        let (_, next) = collect(3, None, page_api(100, 10)).unwrap();
        assert_eq!(next.as_deref(), Some("s+3:"));
        let (d, _) = collect(4, next.as_deref(), page_api(100, 10)).unwrap();
        assert_eq!(d, vec![3, 4, 5, 6]);
    }

    #[test]
    fn token_cursors_round_trip() {
        let (a, next) = collect(15, None, token_api(40, 10)).unwrap();
        assert_eq!(a, (0..15).collect::<Vec<_>>());
        assert_eq!(next.as_deref(), Some("t+5:10"));
        let (b, next) = collect(25, next.as_deref(), token_api(40, 10)).unwrap();
        assert_eq!(b, (15..40).collect::<Vec<_>>());
        assert_eq!(next, None);
        let c = Cursor::decode("t:abc:def==").unwrap();
        assert_eq!(c.position, Position::Token("abc:def==".into()));
        assert_eq!(Cursor::decode(&c.encode()).unwrap(), c);
    }

    #[test]
    fn the_hard_cap_bounds_every_invocation() {
        let mut calls = 0;
        let mut api = offset_api(5000, 100);
        let (items, next) = collect(5000, None, |p, w| {
            calls += 1;
            api(p, w)
        })
        .unwrap();
        assert_eq!(items.len(), HARD_CAP);
        assert_eq!(next.as_deref(), Some("o:1000"));
        assert_eq!(calls, 10);
        assert_eq!(effective_limit(None, true), HARD_CAP);
        assert_eq!(effective_limit(Some(5000), false), HARD_CAP);
        assert_eq!(effective_limit(None, false), DEFAULT_LIMIT);
        assert_eq!(effective_limit(Some(0), false), 1);
    }

    #[test]
    fn bad_cursors_are_usage_errors() {
        for bad in ["", "x", "o:abc", "q:1", "p+x:1", "t:", "s:1"] {
            let err = Cursor::decode(bad).unwrap_err();
            assert_eq!(
                ApiError::find(&err).unwrap().kind,
                crate::client::ErrorKind::Usage,
                "{bad}"
            );
        }
    }

    #[test]
    fn empty_results_have_no_next() {
        let (items, next) = collect(25, None, offset_api(0, 10)).unwrap();
        assert!(items.is_empty());
        assert_eq!(next, None);
    }
}

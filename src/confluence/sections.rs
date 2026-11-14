//! Page sections: the heading outline of an ADF body, and `--section` slices
//! of a storage-format body.
//!
//! A section runs from its heading to the next heading of the same or a
//! higher level. In storage format a section also ends where the element
//! that contains its heading closes (a layout cell, a panel), so a slice
//! never cuts through markup it doesn't own.

use crate::client::ApiError;
use crate::markup::decode_entities;
use serde_json::{Value, json};

/// A heading found in a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// The anchor for a heading: its text with whitespace runs turned into `-`.
pub fn anchor(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join("-")
}

/// Indexes of the headings that `query` selects: case-insensitive exact
/// matches if there are any, else headings that contain the query.
pub fn match_headings<'a>(
    texts: impl IntoIterator<Item = &'a str> + Clone,
    query: &str,
) -> Vec<usize> {
    let q = normalize(query);
    let exact: Vec<usize> = texts
        .clone()
        .into_iter()
        .enumerate()
        .filter(|(_, t)| normalize(t) == q)
        .map(|(i, _)| i)
        .collect();
    if !exact.is_empty() || q.is_empty() {
        return exact;
    }
    texts
        .into_iter()
        .enumerate()
        .filter(|(_, t)| normalize(t).contains(&q))
        .map(|(i, _)| i)
        .collect()
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The usage error for a heading query that matched nothing.
pub fn no_match(query: &str, headings: &[Heading]) -> anyhow::Error {
    let list = if headings.is_empty() {
        "the page has no headings".to_string()
    } else {
        let names: Vec<String> = headings
            .iter()
            .map(|h| format!("{} {}", "#".repeat(h.level as usize), h.text))
            .collect();
        format!("available headings: {}", names.join("; "))
    };
    ApiError::usage(format!("no heading matches '{query}'; {list}")).into()
}

// --- ADF outline ---------------------------------------------------------------

enum Item {
    Heading(Heading, Value),
    Words(usize),
}

fn adf_text(v: &Value, out: &mut String) {
    if let Some(t) = v["text"].as_str() {
        out.push_str(t);
    }
    if let Some(a) = v["content"].as_array() {
        for c in a {
            adf_text(c, out);
            if c["type"] != "text" && !out.ends_with(' ') {
                out.push(' ');
            }
        }
    }
}

/// Plain text of an ADF node.
pub fn adf_plain(v: &Value) -> String {
    let mut s = String::new();
    adf_text(v, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn walk(v: &Value, items: &mut Vec<Item>) {
    if v["type"] == "heading" {
        let level = v["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8;
        items.push(Item::Heading(
            Heading {
                level,
                text: adf_plain(v),
            },
            v["attrs"]["localId"].clone(),
        ));
        return;
    }
    if let Some(t) = v["text"].as_str() {
        items.push(Item::Words(t.split_whitespace().count()));
    }
    if let Some(a) = v["content"].as_array() {
        for c in a {
            walk(c, items);
        }
    }
}

/// `--detail outline`: every heading in document order with its level, text,
/// anchor, ADF `localId` (null when the node has none) and the number of
/// words in its section, subsections included.
pub fn outline(doc: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    walk(doc, &mut items);
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Item::Heading(h, local_id) = item else {
            continue;
        };
        let words: usize = items[i + 1..]
            .iter()
            .take_while(|x| !matches!(x, Item::Heading(n, _) if n.level <= h.level))
            .map(|x| match x {
                Item::Words(n) => *n,
                Item::Heading(..) => 0,
            })
            .sum();
        out.push(json!({
            "level": h.level,
            "text": h.text,
            "anchor": anchor(&h.text),
            "localId": local_id,
            "wordCount": words,
        }));
    }
    out
}

// --- storage slices ------------------------------------------------------------

/// A heading in a storage body with the byte range of its section.
#[derive(Clone, Debug)]
pub struct StorageSection {
    pub heading: Heading,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
enum Tok {
    /// Element opened at this depth (depth before the element).
    Open {
        pos: usize,
        depth: usize,
        heading: Option<u8>,
    },
    /// Element closed; `depth` is the depth after closing.
    Close {
        pos: usize,
        end: usize,
        depth: usize,
    },
}

const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "col", "area", "base", "wbr", "source",
];

fn heading_level(name: &str) -> Option<u8> {
    let b = name.as_bytes();
    (b.len() == 2 && b[0].eq_ignore_ascii_case(&b'h') && (b'1'..=b'6').contains(&b[1]))
        .then(|| b[1] - b'0')
}

/// Element open/close positions with nesting depth. CDATA sections and
/// comments are skipped, so code macros can't fake headings.
fn scan(s: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(off) = s[i..].find('<') {
        let pos = i + off;
        let rest = &s[pos..];
        let skip_to = |end: &str| rest.find(end).map_or(s.len(), |e| pos + e + end.len());
        if rest.starts_with("<![CDATA[") {
            i = skip_to("]]>");
            continue;
        }
        if rest.starts_with("<!--") {
            i = skip_to("-->");
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = skip_to(">");
            continue;
        }
        let Some(gt) = tag_end(rest) else { break };
        let end = pos + gt + 1;
        let inner = &s[pos + 1..end - 1];
        if inner.starts_with('/') {
            depth = depth.saturating_sub(1);
            toks.push(Tok::Close { pos, end, depth });
        } else {
            let name: String = inner
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();
            let self_closing = inner.trim_end().ends_with('/')
                || VOID.contains(&name.to_ascii_lowercase().as_str());
            if !self_closing {
                toks.push(Tok::Open {
                    pos,
                    depth,
                    heading: heading_level(&name),
                });
                depth += 1;
            }
        }
        i = end;
    }
    toks
}

/// Index of the `>` that ends the tag at the start of `s`, honouring quotes.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        out.push(' ');
        match tag_end(&rest[i..]) {
            Some(e) => rest = &rest[i + e + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every heading in a storage body with its section's byte range.
pub fn storage_sections(storage: &str) -> Vec<StorageSection> {
    let toks = scan(storage);
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let Tok::Open {
            pos,
            depth,
            heading: Some(level),
        } = *t
        else {
            continue;
        };
        // The heading's own closing tag.
        let Some(close) = toks[i + 1..]
            .iter()
            .position(|t| matches!(t, Tok::Close { depth: d, .. } if *d == depth))
        else {
            continue;
        };
        let close = i + 1 + close;
        let Tok::Close {
            pos: text_end,
            end: heading_end,
            ..
        } = toks[close]
        else {
            unreachable!()
        };
        let text_start = storage[pos..].find('>').map_or(pos, |g| pos + g + 1);
        let text = strip_tags(&storage[text_start..text_end.max(text_start)]);
        let end = toks[close + 1..]
            .iter()
            .find_map(|t| match *t {
                Tok::Open {
                    pos: p,
                    heading: Some(l),
                    ..
                } if l <= level => Some(p),
                Tok::Close {
                    pos: p, depth: d, ..
                } if d < depth => Some(p),
                _ => None,
            })
            .unwrap_or(storage.len());
        out.push(StorageSection {
            heading: Heading { level, text },
            start: pos,
            end: end.max(heading_end),
        });
    }
    out
}

/// The sections a set of `--section` queries selects, in document order and
/// without sections already contained in another selected one.
pub fn select_storage(storage: &str, queries: &[String]) -> anyhow::Result<Vec<StorageSection>> {
    let all = storage_sections(storage);
    let headings: Vec<Heading> = all.iter().map(|s| s.heading.clone()).collect();
    let mut picked: Vec<usize> = Vec::new();
    for q in queries {
        let m = match_headings(headings.iter().map(|h| h.text.as_str()), q);
        if m.is_empty() {
            return Err(no_match(q, &headings));
        }
        picked.extend(m);
    }
    picked.sort_unstable();
    picked.dedup();
    let mut out: Vec<StorageSection> = Vec::new();
    for i in picked {
        let s = &all[i];
        if out.iter().any(|o| o.start <= s.start && s.end <= o.end) {
            continue;
        }
        out.push(s.clone());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<h1>Runbook</h1><p>Intro</p>\
        <h2>Deploy &amp; Verify</h2><p>Steps</p>\
        <h3>Checks</h3><ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[<h2>not a heading</h2>]]></ac:plain-text-body></ac:structured-macro>\
        <h2>Rollback</h2><p>Undo</p>";

    #[test]
    fn slices_nested_sections_and_skips_cdata() {
        let s = storage_sections(PAGE);
        let names: Vec<_> = s.iter().map(|s| s.heading.text.as_str()).collect();
        assert_eq!(names, ["Runbook", "Deploy & Verify", "Checks", "Rollback"]);
        assert_eq!(&PAGE[s[0].start..s[0].end], PAGE);
        let deploy = &PAGE[s[1].start..s[1].end];
        assert!(deploy.starts_with("<h2>Deploy"));
        assert!(deploy.ends_with("</ac:structured-macro>"));
        assert_eq!(&PAGE[s[3].start..s[3].end], "<h2>Rollback</h2><p>Undo</p>");
    }

    #[test]
    fn section_ends_with_its_container() {
        let page = "<ac:layout><ac:layout-section><ac:layout-cell><h2>Left</h2><p>a</p></ac:layout-cell>\
                    <ac:layout-cell><p>b</p></ac:layout-cell></ac:layout-section></ac:layout>";
        let s = storage_sections(page);
        assert_eq!(&page[s[0].start..s[0].end], "<h2>Left</h2><p>a</p>");
    }

    #[test]
    fn matching_prefers_exact_then_contains() {
        let texts = ["Rollback plan", "Rollback", "Notes"];
        assert_eq!(match_headings(texts, "ROLLBACK"), [1]);
        assert_eq!(match_headings(texts, "back"), [0, 1]);
        assert!(match_headings(texts, "zzz").is_empty());
        let picked = select_storage(PAGE, &["deploy".into(), "checks".into()]).unwrap();
        assert_eq!(picked.len(), 1, "Checks sits inside Deploy");
        let err = select_storage(PAGE, &["nope".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("## Deploy & Verify"), "{err}");
    }

    #[test]
    fn outline_counts_words_per_section() {
        let doc = json!({"type":"doc","content":[
            {"type":"heading","attrs":{"level":1,"localId":"h1"},"content":[{"type":"text","text":"Top"}]},
            {"type":"paragraph","content":[{"type":"text","text":"one two"}]},
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Sub part"}]},
            {"type":"paragraph","content":[{"type":"text","text":"three"}]},
            {"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"Next"}]}
        ]});
        let o = outline(&doc);
        assert_eq!(o.len(), 3);
        assert_eq!(o[0]["localId"], "h1");
        assert_eq!(o[0]["wordCount"], 3);
        assert_eq!(o[1]["anchor"], "Sub-part");
        assert_eq!(o[1]["localId"], Value::Null);
        assert_eq!(o[1]["wordCount"], 1);
        assert_eq!(o[2]["wordCount"], 0);
    }
}

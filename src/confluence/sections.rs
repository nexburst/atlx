//! Page sections: the heading outline of an ADF body, and `--section`
//! slices of it.
//!
//! A section runs from its heading to the next heading of the same or a
//! higher level.

use crate::client::ApiError;
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

// --- ADF sections ----------------------------------------------------------------

/// The top-level headings of a document with their index.
fn top_headings(doc: &Value) -> Vec<(usize, Heading)> {
    doc["content"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, n)| n["type"] == "heading")
        .map(|(i, n)| {
            (
                i,
                Heading {
                    level: n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8,
                    text: adf_plain(n),
                },
            )
        })
        .collect()
}

/// The top-level block range `[start, end)` of the section under the
/// heading at `heads[h]`: up to the next heading of the same or a higher
/// level.
fn section_range(heads: &[(usize, Heading)], h: usize, len: usize) -> (usize, usize) {
    let (start, level) = (heads[h].0, heads[h].1.level);
    let end = heads[h + 1..]
        .iter()
        .find(|(_, x)| x.level <= level)
        .map_or(len, |(i, _)| *i);
    (start, end)
}

/// `--section`: a document holding only the sections the queries select,
/// in document order, each once. Only top-level headings count (a heading
/// inside a panel or layout does not start a section).
pub fn select(doc: &Value, queries: &[String]) -> anyhow::Result<Value> {
    let nodes = doc["content"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let heads = top_headings(doc);
    let mut keep = vec![false; nodes.len()];
    for q in queries {
        let m = match_headings(heads.iter().map(|(_, h)| h.text.as_str()), q);
        if m.is_empty() {
            let hs: Vec<Heading> = heads.iter().map(|(_, h)| h.clone()).collect();
            return Err(no_match(q, &hs));
        }
        for h in m {
            let (start, end) = section_range(&heads, h, nodes.len());
            keep[start..end].iter_mut().for_each(|k| *k = true);
        }
    }
    let content: Vec<Value> = nodes
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(n, _)| n.clone())
        .collect();
    Ok(json!({"type": "doc", "version": 1, "content": content}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        let h = |l: u8, t: &str| json!({"type":"heading","attrs":{"level":l},"content":[{"type":"text","text":t}]});
        let p = |t: &str| json!({"type":"paragraph","content":[{"type":"text","text":t}]});
        json!({"type":"doc","version":1,"content":[
            h(1, "Runbook"), p("Intro"), h(2, "Deploy & Verify"), p("Steps"),
            h(3, "Checks"), p("c"), h(2, "Rollback"), p("Undo"),
        ]})
    }

    #[test]
    fn sections_run_to_the_next_heading_of_their_level() {
        let d = select(&doc(), &["deploy".into()]).unwrap();
        let texts: Vec<String> = d["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(adf_plain)
            .collect();
        assert_eq!(texts, ["Deploy & Verify", "Steps", "Checks", "c"]);
        let both = select(&doc(), &["checks".into(), "rollback".into()]).unwrap();
        assert_eq!(both["content"].as_array().unwrap().len(), 4);
        let nested = select(&doc(), &["deploy".into(), "checks".into()]).unwrap();
        assert_eq!(
            nested["content"].as_array().unwrap().len(),
            4,
            "Checks sits inside Deploy"
        );
    }

    #[test]
    fn matching_prefers_exact_then_contains() {
        let texts = ["Rollback plan", "Rollback", "Notes"];
        assert_eq!(match_headings(texts, "ROLLBACK"), [1]);
        assert_eq!(match_headings(texts, "back"), [0, 1]);
        assert!(match_headings(texts, "zzz").is_empty());
        let err = select(&doc(), &["nope".into()]).unwrap_err().to_string();
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

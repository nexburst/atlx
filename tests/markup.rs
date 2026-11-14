//! Golden tests for rich-text conversions.
//!
//! Inputs live in `tests/fixtures/markup/`:
//! - `<name>.md`: Markdown input. Expected `<name>.adf.json` and
//!   `<name>.storage.html`; Markdown -> ADF -> Markdown must reproduce the
//!   input exactly (the fixtures are written in canonical form).
//! - `<name>.html`: HTML / storage input. Expected `<name>.adf.json` and
//!   `<name>.read.md` (the Markdown a read produces).
//!
//! Every ADF produced is checked against a structural validator. Run with
//! `UPDATE_SNAPSHOTS=1` to rewrite the expected files after an intentional change.

use std::fs;
use std::path::{Path, PathBuf};

use atlx::markup::{
    TextFormat, adf_to_markdown, adf_to_storage, html_to_adf, markdown_to_adf, storage_to_text,
    to_adf, to_storage,
};
use serde_json::{Value, json};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markup")
}

fn bless() -> bool {
    std::env::var_os("UPDATE_SNAPSHOTS").is_some()
}

fn read(p: &Path) -> String {
    fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// Compare `actual` with the expected file, or write it when blessing.
fn check(path: &Path, actual: &str, failures: &mut Vec<String>) {
    if bless() {
        fs::write(path, actual).unwrap();
        return;
    }
    let expected = read(path);
    if expected != actual {
        failures.push(format!(
            "{}\n--- expected\n{expected}\n--- actual\n{actual}",
            path.display()
        ));
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap() + "\n"
}

fn inputs(ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_str().unwrap();
            name.ends_with(ext) && name.matches('.').count() == 1
        })
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no *{ext} fixtures");
    v
}

fn sibling(p: &Path, suffix: &str) -> PathBuf {
    let stem = p.file_stem().unwrap().to_str().unwrap();
    p.with_file_name(format!("{stem}{suffix}"))
}

// ---------------------------------------------------------------------------
// Structural ADF validator (the subset of the schema Jira enforces that our
// converters can get wrong).

const INLINE: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "inlineCard",
    "date",
    "status",
];
const BLOCK: &[&str] = &[
    "paragraph",
    "heading",
    "bulletList",
    "orderedList",
    "taskList",
    "codeBlock",
    "blockquote",
    "rule",
    "table",
    "mediaSingle",
    "panel",
];

fn ty(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("<missing>")
}

fn kids(v: &Value) -> &[Value] {
    v["content"].as_array().map_or(&[], Vec::as_slice)
}

fn validate(doc: &Value) -> Result<(), String> {
    if ty(doc) != "doc" || doc["version"] != 1 {
        return Err("root must be {type: doc, version: 1}".into());
    }
    for c in kids(doc) {
        expect_one_of(c, BLOCK, "doc")?;
        node(c)?;
    }
    Ok(())
}

fn expect_one_of(v: &Value, allowed: &[&str], parent: &str) -> Result<(), String> {
    if allowed.contains(&ty(v)) {
        Ok(())
    } else {
        Err(format!("{} not allowed in {parent}: {v}", ty(v)))
    }
}

fn node(v: &Value) -> Result<(), String> {
    let t = ty(v);
    let all = |allowed: &[&str]| -> Result<(), String> {
        for c in kids(v) {
            expect_one_of(c, allowed, t)?;
            node(c)?;
        }
        Ok(())
    };
    match t {
        "text" => {
            let text = v["text"].as_str().ok_or("text node without text")?;
            if text.is_empty() {
                return Err("empty text node".into());
            }
            let marks: Vec<&str> = v["marks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(ty)
                .collect();
            if marks.contains(&"code") && marks.iter().any(|m| !matches!(*m, "code" | "link")) {
                return Err(format!("code mark combined with other marks: {v}"));
            }
            let mut sorted = marks.clone();
            sorted.sort();
            sorted.dedup();
            if sorted.len() != marks.len() {
                return Err(format!("duplicate marks: {v}"));
            }
            for m in v["marks"].as_array().into_iter().flatten() {
                if ty(m) == "link" && m["attrs"]["href"].as_str().is_none() {
                    return Err(format!("link without href: {v}"));
                }
            }
            Ok(())
        }
        "paragraph" | "heading" => {
            if t == "heading" && !(1..=6).contains(&v["attrs"]["level"].as_u64().unwrap_or(0)) {
                return Err(format!("bad heading level: {v}"));
            }
            all(INLINE)
        }
        "bulletList" | "orderedList" => {
            if kids(v).is_empty() {
                return Err(format!("empty {t}"));
            }
            all(&["listItem"])
        }
        "listItem" => {
            match kids(v).first() {
                Some(f) if ty(f) == "paragraph" => {}
                _ => return Err(format!("listItem must start with a paragraph: {v}")),
            }
            all(&[
                "paragraph",
                "bulletList",
                "orderedList",
                "taskList",
                "codeBlock",
                "mediaSingle",
            ])
        }
        "taskList" => {
            if v["attrs"]["localId"].as_str().is_none() {
                return Err("taskList without localId".into());
            }
            match kids(v).first() {
                Some(f) if ty(f) == "taskItem" => {}
                _ => return Err(format!("taskList must start with a taskItem: {v}")),
            }
            all(&["taskItem", "taskList"])
        }
        "taskItem" => {
            if v["attrs"]["localId"].as_str().is_none()
                || !matches!(v["attrs"]["state"].as_str(), Some("TODO" | "DONE"))
            {
                return Err(format!("taskItem needs localId and state: {v}"));
            }
            all(INLINE)
        }
        "codeBlock" => {
            for c in kids(v) {
                if ty(c) != "text" || c.get("marks").is_some() {
                    return Err(format!("codeBlock may only hold unmarked text: {v}"));
                }
                node(c)?;
            }
            Ok(())
        }
        "blockquote" => all(&[
            "paragraph",
            "bulletList",
            "orderedList",
            "codeBlock",
            "mediaSingle",
        ]),
        "table" => {
            if kids(v).is_empty() {
                return Err("empty table".into());
            }
            all(&["tableRow"])
        }
        "tableRow" => all(&["tableCell", "tableHeader"]),
        "tableCell" | "tableHeader" => {
            if kids(v).is_empty() {
                return Err(format!("empty {t}"));
            }
            all(&[
                "paragraph",
                "heading",
                "bulletList",
                "orderedList",
                "taskList",
                "codeBlock",
                "blockquote",
                "rule",
                "mediaSingle",
                "panel",
            ])
        }
        "mediaSingle" => {
            if kids(v).len() != 1 || ty(&kids(v)[0]) != "media" {
                return Err(format!("mediaSingle must hold one media: {v}"));
            }
            let a = &kids(v)[0]["attrs"];
            if a["type"] != "external" || a["url"].as_str().is_none() {
                return Err(format!("media must be external with url: {v}"));
            }
            Ok(())
        }
        "mention" => v["attrs"]["id"]
            .as_str()
            .map(|_| ())
            .ok_or_else(|| format!("mention without id: {v}")),
        "rule" | "hardBreak" | "emoji" | "inlineCard" | "date" | "status" => Ok(()),
        "panel" => all(&["paragraph", "bulletList", "orderedList", "heading"]),
        _ => Err(format!("unexpected node type {t}")),
    }
}

// ---------------------------------------------------------------------------

#[test]
fn markdown_fixtures() {
    let mut failures = Vec::new();
    for input in inputs(".md") {
        let md = read(&input);
        let adf = markdown_to_adf(&md);
        if let Err(e) = validate(&adf) {
            failures.push(format!("{}: invalid ADF: {e}", input.display()));
        }
        check(&sibling(&input, ".adf.json"), &pretty(&adf), &mut failures);
        check(
            &sibling(&input, ".storage.html"),
            &(adf_to_storage(&adf) + "\n"),
            &mut failures,
        );
        // Markdown -> ADF -> Markdown reproduces the canonical input.
        let back = adf_to_markdown(&adf) + "\n";
        if back != md {
            failures.push(format!(
                "{}: roundtrip changed the Markdown\n--- input\n{md}\n--- output\n{back}",
                input.display()
            ));
        }
        // ... and parsing the output again yields the same ADF.
        if markdown_to_adf(&back) != adf {
            failures.push(format!("{}: ADF not stable on re-parse", input.display()));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn html_fixtures() {
    let mut failures = Vec::new();
    for input in inputs(".html") {
        let html = read(&input);
        let adf = html_to_adf(&html);
        if let Err(e) = validate(&adf) {
            failures.push(format!("{}: invalid ADF: {e}", input.display()));
        }
        check(&sibling(&input, ".adf.json"), &pretty(&adf), &mut failures);
        let md = storage_to_text(&html) + "\n";
        check(&sibling(&input, ".read.md"), &md, &mut failures);
        // The Markdown a read produces is itself stable.
        let again = adf_to_markdown(&markdown_to_adf(&md)) + "\n";
        if again != md {
            failures.push(format!(
                "{}: read Markdown not idempotent\n--- first\n{md}\n--- second\n{again}",
                input.display()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn every_format_produces_valid_adf() {
    let samples = [
        "",
        "plain",
        "**bold `code`**",
        "- \n- [ ] task\n  1. nested",
        "> # heading in quote\n> > nested quote\n> ---",
        "| a |\n|---|\n| |",
        "![img](https://x.y/i.png) and text",
        "* [x] done\n    * plain child",
        "<div><p>raw html block</p></div>",
        "1. a\n\n   ```\n   code\n   ```",
    ];
    for s in samples {
        for f in [TextFormat::Markdown, TextFormat::Html, TextFormat::Plain] {
            let adf = to_adf(s, f);
            validate(&adf).unwrap_or_else(|e| panic!("{f:?} {s:?}: {e}\n{adf:#}"));
        }
    }
    let html = [
        "<ul><li>a<ul><li>b</li></ul></li><li><p>c</p><p>d</p></li></ul>",
        "<p>a<p>b<table><tr><td>x<td>y</table>",
        "<li>orphan</li><td>cell</td>",
        "<b><i>unclosed",
        "<pre><code class=\"language-rust\">fn x() {}\n</code></pre>",
        "<p><img src=\"https://x.y/a.png\" alt=\"A\"></p><p>see <img src=\"https://x.y/b.png\"></p>",
        "<blockquote><h2>T</h2><blockquote>deep</blockquote><hr></blockquote>",
        "<ac:task-list><ac:task><ac:task-status>complete</ac:task-status><ac:task-body>x<ac:task-list><ac:task><ac:task-status>incomplete</ac:task-status><ac:task-body>y</ac:task-body></ac:task></ac:task-list></ac:task-body></ac:task></ac:task-list>",
        "<table><tbody><tr><th></th></tr></tbody></table>",
        "<code>a</code><strong><code>b</code></strong>",
        "<p>&nbsp;</p><p>   </p>",
    ];
    for h in html {
        let adf = html_to_adf(h);
        validate(&adf).unwrap_or_else(|e| panic!("{h:?}: {e}\n{adf:#}"));
    }
}

#[test]
fn markdown_roundtrip_is_idempotent_for_messy_input() {
    // Non-canonical Markdown settles after one pass.
    let samples = [
        "Some *emphasis* and __strong__ text\nwith a soft break.",
        "* a\n* b\n    * c\n\n+ other list",
        "1) one\n2) two",
        "Setext\n======\n\nText_with_underscores and a * star.",
        "A [ref link][r].\n\n[r]: https://example.com",
        "<https://example.com> and <me@example.com>",
        "**bold *nested em* text** ~~gone~~ <u>under</u>",
        "Literal \\*stars\\* and \\# hash and 1\\. dot",
        "| h1 | h2 |\n|:--|--:|\n| `a|b` | x \\| y |\n| only one |",
        "- [ ] todo\n- [x] done\n  - [ ] child",
        "Line one  \nline two\\\nline three",
        "~~~python\nprint('hi')\n~~~",
        "    indented code",
        "# Title #\n\n***\n\n> quote\ncontinued",
        "Text with &amp; entity and &copy; and <span>inline html</span>.",
    ];
    for s in samples {
        let once = adf_to_markdown(&markdown_to_adf(s));
        let adf1 = markdown_to_adf(&once);
        let twice = adf_to_markdown(&adf1);
        assert_eq!(once, twice, "not idempotent for {s:?}");
        validate(&adf1).unwrap_or_else(|e| panic!("{s:?}: {e}"));
    }
}

#[test]
fn escaping_survives_roundtrip() {
    let tricky = [
        "1. not a list",
        "# not a heading",
        "- not a bullet",
        "> not a quote",
        "*not em* and _not em_ and **not strong**",
        "a `tick` and [brackets](not-a-link)",
        "<b>not html</b> & &amp; entity",
        "snake_case_name and 2 * 3 * 4",
        "trailing backslash \\",
        "pipes | in | text",
        "~~not struck~~",
    ];
    for t in tricky {
        let adf = json!({"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": t}]}
        ]});
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "{t:?} rendered as {md:?}");
    }
    // The same text inside a table cell, where pipes must be escaped too.
    for t in tricky {
        let adf = json!({"type": "doc", "version": 1, "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [{"type": "paragraph", "content": [{"type": "text", "text": t}]}]}
            ]}
        ]}]});
        let md = adf_to_markdown(&adf);
        assert_eq!(markdown_to_adf(&md), adf, "cell {t:?} rendered as {md:?}");
    }
}

#[test]
fn marks_nest_and_split_cleanly() {
    let t = |text: &str, marks: Value| json!({"type": "text", "text": text, "marks": marks});
    let adf = json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [
        t("bold ", json!([{"type": "strong"}])),
        t("both", json!([{"type": "strong"}, {"type": "em"}])),
        t(" bold", json!([{"type": "strong"}])),
        {"type": "text", "text": " plain "},
        t("link ", json!([{"type": "link", "attrs": {"href": "https://x.y/a b"}}])),
        t("code", json!([{"type": "link", "attrs": {"href": "https://x.y/a b"}}, {"type": "code"}])),
    ]}]});
    let md = adf_to_markdown(&adf);
    assert_eq!(
        md,
        "**bold *both* bold** plain [link `code`](<https://x.y/a b>)"
    );
    assert_eq!(markdown_to_adf(&md), adf);
}

#[test]
fn read_rendering_of_jira_nodes() {
    let adf = json!({"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [
            {"type": "mention", "attrs": {"id": "acc-1", "text": "@Ana Lopez"}},
            {"type": "text", "text": " and "},
            {"type": "mention", "attrs": {"id": "acc-2"}},
            {"type": "text", "text": " "},
            {"type": "emoji", "attrs": {"shortName": ":smile:", "text": "😄"}},
            {"type": "text", "text": " due "},
            {"type": "date", "attrs": {"timestamp": "1767225600000"}},
            {"type": "text", "text": " "},
            {"type": "status", "attrs": {"text": "IN PROGRESS", "color": "blue"}},
            {"type": "text", "text": " "},
            {"type": "inlineCard", "attrs": {"url": "https://x.atlassian.net/browse/A-1"}},
        ]},
        {"type": "panel", "attrs": {"panelType": "info"}, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "Heads up"}]}
        ]},
        {"type": "expand", "attrs": {"title": "Details"}, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "Hidden"}]}
        ]},
        {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"type": "file", "id": "abc", "collection": "c", "alt": "shot.png"}}]},
        {"type": "orderedList", "attrs": {"order": 3}, "content": [
            {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "three"}]}]},
            {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "four"}]}]}
        ]}
    ]});
    assert_eq!(
        adf_to_markdown(&adf),
        "@Ana Lopez and @acc-2 😄 due 2026-01-01 \\[IN PROGRESS] <https://x.atlassian.net/browse/A-1>\n\n\
         > Heads up\n\n**Details**\n\nHidden\n\n[attachment: shot.png]\n\n3. three\n4. four"
    );
}

#[test]
fn html_format_keeps_rich_marks_for_jira() {
    let adf = to_adf(
        "<p><em>it</em> <u>under</u> <s>gone</s> <del>del</del> a<br>b</p>",
        TextFormat::Html,
    );
    assert_eq!(
        adf["content"][0]["content"],
        json!([
            {"type": "text", "text": "it", "marks": [{"type": "em"}]},
            {"type": "text", "text": " "},
            {"type": "text", "text": "under", "marks": [{"type": "underline"}]},
            {"type": "text", "text": " "},
            {"type": "text", "text": "gone", "marks": [{"type": "strike"}]},
            {"type": "text", "text": " "},
            {"type": "text", "text": "del", "marks": [{"type": "strike"}]},
            {"type": "text", "text": " a"},
            {"type": "hardBreak"},
            {"type": "text", "text": "b"}
        ])
    );
}

#[test]
fn storage_passthrough_and_plain() {
    assert_eq!(to_storage("<p>x</p>", TextFormat::Html), "<p>x</p>");
    assert_eq!(
        to_storage("a & b\nc", TextFormat::Plain),
        "<p>a &amp; b<br/>c</p>"
    );
    assert_eq!(
        to_storage("Hello **world**", TextFormat::Markdown),
        "<p>Hello <strong>world</strong></p>"
    );
}

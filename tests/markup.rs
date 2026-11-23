//! Golden tests for rich-text conversions.
//!
//! Inputs live in `tests/fixtures/markup/`:
//! - `<name>.md`: Markdown input. Expected `<name>.adf.json` and
//!   `<name>.storage.html`; Markdown -> ADF -> Markdown must reproduce the
//!   input exactly (the fixtures are written in canonical form).
//! - `<name>.html`: HTML / storage input. Expected `<name>.adf.json` and
//!   `<name>.read.md` (the Markdown a read produces).
//! - `roundtrip/<name>.adf.json`: one supported node type each (plus `rich`);
//!   ADF -> Markdown (`<name>.md`) -> ADF must be structurally identical and
//!   report no loss.
//! - `lossy/<name>.adf.json`: one unsupported construct each; the renderer
//!   must report the node named in `lossy/expected-nodes.json`, and the
//!   rendered Markdown (with its markers) is `<name>.md`.
//!
//! Every ADF produced is checked against a structural validator. Run with
//! `UPDATE_SNAPSHOTS=1` to rewrite the expected files after an intentional change.

use std::fs;
use std::path::{Path, PathBuf};

use atlx::markup::{
    Body, BodyErrorKind, BodyFormat, TextFormat, adf_to_markdown, adf_to_storage,
    adf_to_storage_checked, fidelity, html_to_adf, lossy_warning, markdown_to_adf,
    markdown_to_storage_checked, parse_body, render_markdown, render_storage, storage,
    storage_to_text, to_adf, to_storage, unrepresentable,
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
    "expand",
    "blockCard",
    "embedCard",
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
                "nestedExpand",
                "blockCard",
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
        "blockCard" | "embedCard" => v["attrs"]["url"]
            .as_str()
            .map(|_| ())
            .ok_or_else(|| format!("{t} without url: {v}")),
        "panel" => all(&[
            "paragraph",
            "bulletList",
            "orderedList",
            "heading",
            "taskList",
            "codeBlock",
            "mediaSingle",
            "rule",
            "blockCard",
        ]),
        "expand" | "nestedExpand" => {
            let mut allowed = BLOCK.to_vec();
            allowed.retain(|b| *b != "expand");
            allowed.push("nestedExpand");
            all(&allowed)
        }
        _ => Err(format!("unexpected node type {t}")),
    }
}

// ---------------------------------------------------------------------------

#[test]
fn markdown_fixtures() {
    let mut failures = Vec::new();
    for input in inputs(".md") {
        let md = read(&input);
        let converted = markdown_to_adf(&md);
        if !converted.lost.is_empty() {
            failures.push(format!("{}: lossy: {:?}", input.display(), converted.lost));
        }
        let adf = converted.adf;
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
        if markdown_to_adf(&back).adf != adf {
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
        // The Markdown a read produces is itself stable, unless it marks
        // content it could not show (markers do not convert back).
        if !render_storage(&html).lossy.is_empty() {
            continue;
        }
        let again = adf_to_markdown(&markdown_to_adf(&md).adf) + "\n";
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
        let once = adf_to_markdown(&markdown_to_adf(s).adf);
        let adf1 = markdown_to_adf(&once).adf;
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
        assert_eq!(markdown_to_adf(&md).adf, adf, "{t:?} rendered as {md:?}");
    }
    // The same text inside a table cell, where pipes must be escaped too.
    for t in tricky {
        let adf = json!({"type": "doc", "version": 1, "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [{"type": "paragraph", "content": [{"type": "text", "text": t}]}]}
            ]}
        ]}]});
        let md = adf_to_markdown(&adf);
        assert_eq!(
            markdown_to_adf(&md).adf,
            adf,
            "cell {t:?} rendered as {md:?}"
        );
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
    assert_eq!(markdown_to_adf(&md).adf, adf);
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
    let r = render_markdown(&adf);
    assert_eq!(
        r.text,
        "@[Ana Lopez](accountid:acc-1) and @[](accountid:acc-2) :smile: due [2026-01-01](date:2026-01-01) \
         [IN PROGRESS](status:blue) [https://x.atlassian.net/browse/A-1](https://x.atlassian.net/browse/A-1 \"card\")\n\n\
         > [!NOTE]\n> Heads up\n\n<details>\n<summary>Details</summary>\n\nHidden\n\n</details>\n\n\
         <!-- atlx: media:file shot.png (not shown) -->\n\n3. three\n4. four"
    );
    // Only the attached file is lost.
    assert_eq!(
        r.lossy.iter().map(|l| l.node.as_str()).collect::<Vec<_>>(),
        ["media:file"]
    );
    assert_eq!(r.lossy[0].at, "/content/3/content/0");
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
    assert_eq!(
        to_storage("<p>x</p>", TextFormat::Html).unwrap(),
        "<p>x</p>"
    );
    assert_eq!(
        to_storage("a & b\nc", TextFormat::Plain).unwrap(),
        "<p>a &amp; b<br/>c</p>"
    );
    assert_eq!(
        to_storage("Hello **world**", TextFormat::Markdown).unwrap(),
        "<p>Hello <strong>world</strong></p>"
    );
}

// ---------------------------------------------------------------------------
// Fidelity: round trips, losses, markers, refusals (D11).

fn sub_inputs(sub: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir().join(sub))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_str().unwrap().ends_with(".adf.json"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no fixtures in {sub}");
    v
}

fn stem(p: &Path) -> String {
    p.file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .trim_end_matches(".adf.json")
        .to_string()
}

/// For every supported node type (and a combined document), ADF -> Markdown
/// -> ADF is structurally identical, nothing is reported lost, and the
/// Markdown matches its golden file.
#[test]
fn supported_nodes_roundtrip() {
    let mut failures = Vec::new();
    for input in sub_inputs("roundtrip") {
        let name = stem(&input);
        let adf: Value = serde_json::from_str(&read(&input)).unwrap();
        let r = render_markdown(&adf);
        if !r.lossy.is_empty() {
            failures.push(format!("{name}: render reported {:?}", r.lossy));
        }
        check(
            &input.with_file_name(format!("{name}.md")),
            &(r.text.clone() + "\n"),
            &mut failures,
        );
        let back = markdown_to_adf(&r.text);
        if !back.lost.is_empty() {
            failures.push(format!("{name}: parse reported {:?}", back.lost));
        }
        if let Err(e) = validate(&back.adf) {
            failures.push(format!("{name}: invalid ADF: {e}"));
        }
        if !fidelity::structurally_equal(&adf, &back.adf) {
            failures.push(format!(
                "{name}: round trip changed the document\n--- markdown\n{}\n--- before\n{}\n--- after\n{}",
                r.text,
                pretty(&fidelity::canonical(&adf)),
                pretty(&fidelity::canonical(&back.adf))
            ));
        }
        let u = unrepresentable(&adf);
        if !u.is_empty() {
            failures.push(format!("{name}: unrepresentable reported {u:?}"));
        }
        // Writing it as storage loses nothing either, except panel types
        // and times of day that storage format has no form for.
        if !matches!(name.as_str(), "panel" | "date") {
            let (_, lost) = adf_to_storage_checked(&back.adf);
            if !lost.is_empty() {
                failures.push(format!("{name}: storage writer reported {lost:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Each unsupported node type is reported by the renderer, marked in the
/// Markdown, listed by `unrepresentable`, and refused on write-back.
#[test]
fn unsupported_nodes_are_reported_and_marked() {
    let expected: serde_json::Map<String, Value> =
        serde_json::from_str(&read(&dir().join("lossy/expected-nodes.json"))).unwrap();
    let mut failures = Vec::new();
    for input in sub_inputs("lossy") {
        let name = stem(&input);
        let node = expected[&name].as_str().unwrap();
        let adf: Value = serde_json::from_str(&read(&input)).unwrap();
        let r = render_markdown(&adf);
        if !r.lossy.iter().any(|l| l.node == node) {
            failures.push(format!("{name}: expected a {node} loss, got {:?}", r.lossy));
        }
        if r.lossy.iter().any(|l| !l.at.starts_with('/')) {
            failures.push(format!(
                "{name}: loss without a JSON pointer: {:?}",
                r.lossy
            ));
        }
        check(
            &input.with_file_name(format!("{name}.md")),
            &(r.text.clone() + "\n"),
            &mut failures,
        );
        let w = r.warning().unwrap();
        if w["code"] != "lossy_body" || !w["nodes"].as_array().unwrap().contains(&json!(node)) {
            failures.push(format!("{name}: bad warning {w}"));
        }
        let u = unrepresentable(&adf);
        if !u.iter().any(|l| l.node == node) {
            failures.push(format!("{name}: unrepresentable missed {node}: {u:?}"));
        }
        // A marker refuses to convert back.
        let marked = r.text.contains("<!-- atlx: ");
        let back = markdown_to_adf(&r.text);
        if marked && back.lost.is_empty() {
            failures.push(format!("{name}: marker accepted on write-back"));
        }
    }
    for name in [
        "extension",
        "inlineExtension",
        "mediaGroup",
        "unknown",
        "fileMedia",
    ] {
        let r = render_markdown(
            &serde_json::from_str(&read(&dir().join(format!("lossy/{name}.adf.json")))).unwrap(),
        );
        if !r.text.contains("(not shown) -->") {
            failures.push(format!("{name}: no marker in {}", r.text));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

#[test]
fn marker_text_matches_the_documented_form() {
    let adf = json!({"type": "doc", "version": 1, "content": [
        {"type": "extension", "attrs": {"extensionKey": "toc"}}
    ]});
    assert_eq!(
        render_markdown(&adf).text,
        "<!-- atlx: extension:toc (not shown) -->"
    );
    assert_eq!(
        lossy_warning(&render_markdown(&adf).lossy).unwrap()["nodes"],
        json!(["extension:toc"])
    );
    assert!(lossy_warning(&[]).is_none());
}

/// Round trips that look plausible but are not two-way are caught by
/// `unrepresentable`, not only by the renderer.
#[test]
fn unrepresentable_catches_one_way_mappings() {
    let cases = [
        // A heading with a line break cannot be one Markdown line.
        json!({"type": "doc", "version": 1, "content": [{"type": "heading", "attrs": {"level": 2}, "content": [
            {"type": "text", "text": "a"}, {"type": "hardBreak"}, {"type": "text", "text": "b"}]}]}),
        // Header cells after the first row.
        json!({"type": "doc", "version": 1, "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [{"type": "tableHeader", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "h"}]}]}]},
            {"type": "tableRow", "content": [{"type": "tableHeader", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]}]}]}]}),
        // Two paragraphs in a cell.
        json!({"type": "doc", "version": 1, "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [{"type": "tableHeader", "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "a"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}]}]}]}),
        // A link whose address is reserved for mentions.
        json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "accountid:1"}}]}]}]}),
        // A heading inside a blockquote (normalization flattens it).
        json!({"type": "doc", "version": 1, "content": [{"type": "blockquote", "content": [
            {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "q"}]}]}]}),
    ];
    for c in cases {
        assert!(!unrepresentable(&c).is_empty(), "not caught: {c}");
    }
    // Layout-only attributes, edge whitespace and empty paragraphs are not
    // losses.
    let plain = json!({"type": "doc", "version": 1, "content": [
        {"type": "table", "attrs": {"layout": "wide", "width": 900, "localId": "x"}, "content": [
            {"type": "tableRow", "content": [{"type": "tableHeader", "attrs": {"colwidth": [120], "background": "#eee"},
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "h"}]}]}]}]},
        {"type": "paragraph", "content": [{"type": "text", "text": "  spaced  "}]},
        {"type": "paragraph", "content": []},
        {"type": "codeBlock", "attrs": {"language": "sh"}, "marks": [{"type": "breakout", "attrs": {"mode": "wide"}}],
            "content": [{"type": "text", "text": "ls"}]}
    ]});
    assert_eq!(unrepresentable(&plain), vec![]);
}

#[test]
fn markdown_with_raw_html_converts_with_a_loss() {
    let c = markdown_to_adf("Intro\n\n<div class=\"x\">\n<p>raw</p>\n</div>\n\nAfter");
    assert_eq!(c.lost.len(), 1, "{:?}", c.lost);
    assert_eq!(c.lost[0].node, "html");
    assert_eq!(c.lost[0].at, "line 3");
    let e = c.into_lossless().unwrap_err();
    assert_eq!(e.kind, BodyErrorKind::Refused);
    assert!(e.to_string().contains("raw HTML"), "{e}");

    let inline = markdown_to_adf("a <span>b</span> <u>c</u>");
    assert_eq!(
        inline
            .lost
            .iter()
            .map(|l| l.node.as_str())
            .collect::<Vec<_>>(),
        ["html", "html"]
    );
    // atlx's own markers are recognised and explained.
    let m = markdown_to_adf("x\n\n<!-- atlx: extension:jira (not shown) -->\n\ny");
    assert!(m.lost[0].detail.contains("extension:jira"), "{:?}", m.lost);
    // Supported inline HTML and <details> are not losses.
    let ok = markdown_to_adf(
        "a<br>b <sub>2</sub>\n\n<details>\n<summary>T</summary>\n\nbody\n\n</details>",
    );
    assert_eq!(ok.lost, vec![]);
    assert_eq!(ok.adf["content"][1]["type"], "expand");
}

#[test]
fn markdown_constructs_adf_cannot_hold_are_reported() {
    let cases = [
        ("> # heading in a quote", "heading"),
        ("> quote\n>\n> ---", "rule"),
        ("> > nested", "blockquote"),
        ("- # heading in a list", "heading"),
        ("| a |\n|:--|\n| b |", "table"),
        ("- [ ] task\n\n  second paragraph", "paragraph"),
        ("text ![img](https://x.y/i.png) text", "image"),
        ("[x](status:pink)", "status"),
        ("[x](date:someday)", "date"),
        ("```rust title=x\ncode\n```", "codeBlock"),
        ("**@[Ana](accountid:1)**", "mention"),
    ];
    for (md, node) in cases {
        let c = markdown_to_adf(md);
        assert!(
            c.lost.iter().any(|l| l.node == node),
            "{md:?}: expected {node}, got {:?}",
            c.lost
        );
    }
}

#[test]
fn widened_dialect_parses_to_adf_nodes() {
    let c = markdown_to_adf(
        "> [!WARNING]\n> Careful\n\n> [!SUCCESS]\n> Done\n\n\
         Hi @[Ana](accountid:557058:1) [OK](status:green) [2026-01-31](date:2026-01-31) :wave: \
         [https://x.y](https://x.y \"card\")\n\n\\[!NOTE] and \\:smile:",
    );
    assert_eq!(c.lost, vec![]);
    let d = &c.adf["content"];
    assert_eq!(d[0]["attrs"]["panelType"], "warning");
    assert_eq!(d[1]["attrs"]["panelType"], "success");
    let p = &d[2]["content"];
    assert_eq!(p[0], json!({"type": "text", "text": "Hi "}));
    assert_eq!(
        p[1],
        json!({"type": "mention", "attrs": {"id": "557058:1", "text": "@Ana"}})
    );
    assert_eq!(p[3]["type"], "status");
    assert_eq!(p[5]["attrs"]["timestamp"], "1769817600000");
    assert_eq!(
        p[7],
        json!({"type": "emoji", "attrs": {"shortName": ":wave:"}})
    );
    assert_eq!(
        p[9],
        json!({"type": "inlineCard", "attrs": {"url": "https://x.y"}})
    );
    // Escaped forms stay text.
    assert_eq!(
        d[3]["content"],
        json!([{"type": "text", "text": "[!NOTE] and :smile:"}])
    );
    // A table with an empty header row has no header.
    let t = markdown_to_adf("| | |\n| --- | --- |\n| a | b |").adf;
    assert_eq!(t["content"][0]["content"].as_array().unwrap().len(), 1);
    assert_eq!(
        t["content"][0]["content"][0]["content"][0]["type"],
        "tableCell"
    );
}

/// HTML given as a Confluence body is refused before anything is sent
/// unless it is well-formed storage made of storage elements.
#[test]
fn html_body_fails_closed() {
    for (html, node) in [
        (
            "<p>open <custom-widget>x</custom-widget></p>",
            "<custom-widget>",
        ),
        ("<p>unbalanced <b>bold</p>", "<b>"),
        ("<p>line<br>break</p>", "<br>"),
        ("<p>AT&T</p>", "&"),
        (
            "<div><details><summary>s</summary>x</details></div>",
            "<details>",
        ),
    ] {
        let e = to_storage(html, TextFormat::Html).unwrap_err();
        assert_eq!(e.kind, BodyErrorKind::Refused, "{html}");
        assert!(
            e.losses.iter().any(|l| l.node == node),
            "{html}: {:?}",
            e.losses
        );
        // The callers' `?` carries it as an anyhow::Error.
        let any: anyhow::Error = e.into();
        assert!(any.to_string().contains("storage format"), "{any}");
    }
    let ok = "<h1>T</h1><p>a &amp; b<br/>c &nbsp;</p>\
              <ac:structured-macro ac:name=\"info\"><ac:rich-text-body><p>x</p></ac:rich-text-body></ac:structured-macro>";
    assert_eq!(to_storage(ok, TextFormat::Html).unwrap(), ok);
    assert!(storage::check(ok).is_ok());
    // Markdown that would lose content is refused for storage too.
    let e = to_storage("<section>raw</section>", TextFormat::Markdown).unwrap_err();
    assert!(e.is_refused());
    assert!(markdown_to_storage_checked("> [!IMPORTANT]\n> purple").is_err());
    assert!(markdown_to_storage_checked("> [!NOTE]\n> blue").is_ok());
}

#[test]
fn parse_body_formats() {
    assert!(matches!(
        parse_body("**x**", BodyFormat::Markdown),
        Ok(Body::Adf(_))
    ));
    assert!(matches!(
        parse_body("a\n\nb", BodyFormat::Plain),
        Ok(Body::Adf(_))
    ));
    let adf = r#"{"type":"doc","version":1,"content":[{"type":"paragraph","content":[]}]}"#;
    assert!(matches!(parse_body(adf, BodyFormat::Adf), Ok(Body::Adf(v)) if v["type"] == "doc"));
    let one = r#"{"type":"paragraph","content":[]}"#;
    assert!(
        matches!(parse_body(one, BodyFormat::Adf), Ok(Body::Adf(v)) if v["content"][0]["type"] == "paragraph")
    );
    let e = parse_body("{not json", BodyFormat::Adf).unwrap_err();
    assert_eq!(e.kind, BodyErrorKind::Invalid);
    assert_eq!(
        parse_body("<p>x</p>", BodyFormat::Storage).unwrap(),
        Body::Storage("<p>x</p>".into())
    );
    assert!(
        parse_body("<p>x", BodyFormat::Storage)
            .unwrap_err()
            .is_refused()
    );
    assert!(
        parse_body("<div>x</div>", BodyFormat::Markdown)
            .unwrap_err()
            .is_refused()
    );
}

#[test]
fn storage_reads_mark_what_they_cannot_show() {
    let s = "<p>Intro</p>\
             <ac:structured-macro ac:name=\"toc\"/>\
             <ac:layout><ac:layout-section ac:type=\"two_equal\"><ac:layout-cell><p>left</p></ac:layout-cell>\
             <ac:layout-cell><p>right</p></ac:layout-cell></ac:layout-section></ac:layout>\
             <p>See <ac:structured-macro ac:name=\"jira\"><ac:parameter ac:name=\"key\">OPS-1</ac:parameter></ac:structured-macro></p>\
             <ac:structured-macro ac:name=\"details\"><ac:rich-text-body><p>body text</p></ac:rich-text-body></ac:structured-macro>\
             <p><ac:image><ri:attachment ri:filename=\"shot.png\"/></ac:image></p>";
    let r = render_storage(s);
    let nodes: Vec<&str> = r.lossy.iter().map(|l| l.node.as_str()).collect();
    for n in [
        "extension:toc",
        "layoutSection",
        "inlineExtension:jira",
        "bodiedExtension:details",
        "media:file",
    ] {
        assert!(nodes.contains(&n), "missing {n} in {nodes:?}\n{}", r.text);
    }
    assert!(
        r.text.contains("<!-- atlx: extension:toc (not shown) -->"),
        "{}",
        r.text
    );
    assert!(
        r.text
            .contains("<!-- atlx: inlineExtension:jira OPS-1 (not shown) -->"),
        "{}",
        r.text
    );
    assert!(r.text.contains("left") && r.text.contains("right") && r.text.contains("body text"));
    // The same storage parsed for a write reports the losses without markers.
    let c = atlx::markup::html_to_adf_checked(s);
    assert!(c.lost.iter().any(|l| l.node == "extension:toc"));
    assert!(!adf_to_markdown(&c.adf).contains("atlx:"));
}

#[test]
fn storage_round_trip_of_widened_nodes() {
    // Markdown -> storage -> ADF keeps panels, expands, statuses, dates,
    // mentions and cards.
    let md = "> [!WARNING]\n> Careful\n\n<details>\n<summary>More</summary>\n\nHidden\n\n</details>\n\n\
              @[Ana](accountid:1) [OK](status:green) [2026-01-31](date:2026-01-31) [https://x.y](https://x.y \"card\")";
    let st = markdown_to_storage_checked(md).unwrap();
    assert!(storage::check(&st).is_ok(), "{st}");
    let back = html_to_adf(&st);
    let mut a = markdown_to_adf(md).adf;
    // Storage has no mention text (Confluence shows the account's name).
    a["content"][2]["content"][0]["attrs"]
        .as_object_mut()
        .unwrap()
        .remove("text");
    assert!(
        fidelity::structurally_equal(&a, &back),
        "{st}\n{}\n{}",
        pretty(&a),
        pretty(&back)
    );
}

/// Converter code may not drop content in a catch-all arm without saying
/// why: every `_ => {}` in `src/markup/` has a comment on the line before.
#[test]
fn no_silent_catch_all_arms() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/markup");
    let mut bad = Vec::new();
    for e in fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        let text = fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.trim_start().starts_with("_ => {}") {
                let prev = i.checked_sub(1).map_or("", |j| lines[j].trim());
                if !prev.starts_with("//") {
                    bad.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "uncommented catch-all arms: {bad:?}");
}

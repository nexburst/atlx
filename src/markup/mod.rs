//! Conversions between text formats and Atlassian's rich formats:
//! ADF (Jira descriptions/comments) and Confluence storage format (XHTML).
//!
//! Input bodies follow twg's `--*-format html|markdown|plain` convention.
//! Markdown and HTML inputs are parsed into ADF, which is the common model:
//! Markdown reaches storage format through ADF, and reads render ADF (or
//! storage parsed into ADF) back to GitHub-flavoured Markdown.

mod adf;
mod html;
mod markdown;
mod to_markdown;
mod to_storage;

use clap::ValueEnum;
use serde_json::{Value, json};

pub(crate) use html::decode as decode_entities;
pub use html::html_to_adf;
pub use markdown::markdown_to_adf;
pub use to_markdown::adf_to_markdown;
pub use to_storage::adf_to_storage;

/// Input format for bodies, descriptions and comments (twg: html is the default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum TextFormat {
    #[default]
    Html,
    #[value(alias = "md")]
    Markdown,
    Plain,
}

/// Any supported input -> ADF (Jira).
pub fn to_adf(s: &str, f: TextFormat) -> Value {
    match f {
        TextFormat::Plain => text_to_adf(s),
        TextFormat::Markdown => markdown_to_adf(s),
        TextFormat::Html => html_to_adf(s),
    }
}

/// Any supported input -> Confluence storage format. HTML passes through.
pub fn to_storage(s: &str, f: TextFormat) -> String {
    match f {
        TextFormat::Html => s.to_string(),
        TextFormat::Markdown => markdown_to_storage(s),
        TextFormat::Plain => text_to_storage(s),
    }
}

/// Markdown -> Confluence storage format (via ADF).
pub fn markdown_to_storage(s: &str) -> String {
    adf_to_storage(&markdown_to_adf(s))
}

/// Confluence storage format / HTML -> Markdown (via ADF).
pub fn storage_to_markdown(html: &str) -> String {
    adf_to_markdown(&html_to_adf(html))
}

/// Alias of [`adf_to_markdown`], kept for existing callers.
pub fn adf_to_text(v: &Value) -> String {
    adf_to_markdown(v)
}

/// Alias of [`storage_to_markdown`], kept for existing callers.
pub fn storage_to_text(html: &str) -> String {
    storage_to_markdown(html)
}

fn paragraphs(s: &str) -> impl Iterator<Item = &str> {
    s.split("\n\n")
        .map(|p| p.trim_matches('\n'))
        .filter(|p| !p.trim().is_empty())
}

/// Plain text -> ADF document. Blank lines separate paragraphs; single
/// newlines become hard breaks.
pub fn text_to_adf(s: &str) -> Value {
    let s = s.replace("\r\n", "\n");
    let content: Vec<Value> = paragraphs(&s)
        .map(|p| {
            let mut inline = Vec::new();
            for (i, line) in p.lines().enumerate() {
                if i > 0 {
                    inline.push(json!({"type": "hardBreak"}));
                }
                if !line.is_empty() {
                    inline.push(json!({"type": "text", "text": line}));
                }
            }
            json!({"type": "paragraph", "content": inline})
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": content})
}

/// Plain text -> Confluence storage format.
pub fn text_to_storage(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    paragraphs(&s)
        .map(|p| {
            format!(
                "<p>{}</p>",
                p.lines().map(escape_html).collect::<Vec<_>>().join("<br/>")
            )
        })
        .collect()
}

pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Confluence search highlights matches with `@@@hl@@@...@@@endhl@@@`.
pub fn strip_highlight(s: &str) -> String {
    s.replace("@@@hl@@@", "").replace("@@@endhl@@@", "")
}

/// Quote a value for JQL / CQL / Bitbucket `q` filters.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_adf_roundtrip() {
        let adf = text_to_adf("Hello\nworld\n\nSecond para");
        assert_eq!(adf["content"].as_array().unwrap().len(), 2);
        assert_eq!(adf["content"][0]["content"][1]["type"], "hardBreak");
        // Hard breaks read back as Markdown hard breaks (backslash + newline).
        assert_eq!(adf_to_text(&adf), "Hello\\\nworld\n\nSecond para");
    }

    #[test]
    fn adf_rich_nodes() {
        let doc = json!({"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Title"}]},
            {"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[
                    {"type":"text","text":"docs","marks":[{"type":"link","attrs":{"href":"https://x.y"}}]}]}]}
            ]},
            {"type":"paragraph","content":[{"type":"mention","attrs":{"text":"@Ana"}},{"type":"text","text":" please"}]},
            {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"fn main() {}"}]}
        ]});
        assert_eq!(
            adf_to_text(&doc),
            "## Title\n\n- one\n- [docs](https://x.y)\n\n@Ana please\n\n```rust\nfn main() {}\n```"
        );
    }

    #[test]
    fn markdown_roundtrips_through_adf() {
        let md = "## Plan\n\nShip **today** with `cargo`, see [docs](https://x.y).\n\n- one\n- two\n\n1. a\n2. b\n\n```sh\nmake\n```\n\n> quoted";
        let adf = markdown_to_adf(md);
        assert_eq!(
            adf["content"][1]["content"][1]["marks"][0]["type"],
            "strong"
        );
        assert_eq!(adf["content"][2]["type"], "bulletList");
        assert_eq!(adf["content"][3]["type"], "orderedList");
        assert_eq!(adf_to_text(&adf), md);
    }

    #[test]
    fn markdown_to_storage_format() {
        assert_eq!(
            markdown_to_storage("# T\n\nA & **b**\n\n- x\n\n```\nq\n```"),
            "<h1>T</h1><p>A &amp; <strong>b</strong></p><ul><li>x</li></ul>\
             <ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[q]]></ac:plain-text-body></ac:structured-macro>"
        );
    }

    #[test]
    fn html_input_to_adf() {
        let adf = to_adf(
            "<h2>Hi</h2><p>See <a href=\"https://x.y\">this</a> <strong>now</strong></p><ul><li>a</li></ul>",
            TextFormat::Html,
        );
        assert_eq!(
            adf_to_text(&adf),
            "## Hi\n\nSee [this](https://x.y) **now**\n\n- a"
        );
    }

    #[test]
    fn storage_conversions() {
        assert_eq!(
            text_to_storage("a < b\nc\n\nd"),
            "<p>a &lt; b<br/>c</p><p>d</p>"
        );
        let html = "<h1>Intro</h1><p>Fish &amp; chips&#33;</p><ul><li>one</li><li>two</li></ul>\
                    <ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[x < 1]]></ac:plain-text-body></ac:structured-macro>";
        assert_eq!(
            storage_to_text(html),
            "# Intro\n\nFish & chips!\n\n- one\n- two\n\n```\nx < 1\n```"
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
        assert_eq!(strip_highlight("@@@hl@@@foo@@@endhl@@@ bar"), "foo bar");
    }
}

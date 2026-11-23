//! Conversions between text formats and Atlassian's rich formats:
//! ADF (Jira and Confluence bodies) and Confluence storage format (XHTML).
//!
//! ADF is the internal model. Markdown is parsed into ADF and ADF is
//! rendered back to Markdown; storage format is passed through only after a
//! well-formedness check ([`storage::check`]).
//!
//! Every converter reports what it could not represent as a list of
//! [`Loss`]es instead of dropping it silently:
//! - [`markdown_to_adf`] returns [`Converted`] (`adf` + `lost`); a write
//!   must refuse when `lost` is non-empty ([`Converted::into_lossless`]).
//! - [`render_markdown`] returns [`Rendered`] (`text` + `lossy`); the text
//!   carries a one-line `<!-- atlx: ... -->` marker where a node could not be
//!   shown, and [`lossy_warning`] builds the envelope warning.
//! - [`unrepresentable`] lists the nodes of an ADF document that a Markdown
//!   round trip would destroy, for refusing a full-body Markdown replace.
//!
//! The Markdown dialect: CommonMark with GFM tables, strikethrough and task
//! lists, plus
//! - panels as alert blockquotes: `> [!NOTE]` (info), `> [!TIP]` (tip),
//!   `> [!IMPORTANT]` (note), `> [!WARNING]` (warning), `> [!CAUTION]`
//!   (error), `> [!SUCCESS]` (success);
//! - expand blocks as `<details><summary>Title</summary> ... </details>`;
//! - mentions `@[Name](accountid:ID)`, status lozenges `[TEXT](status:color)`,
//!   dates `[2026-01-31](date:2026-01-31)`, emoji `:shortname:`;
//! - smart links `[url](url "card")`, `"card:block"`, `"card:embed"`;
//! - `<u>`, `<sub>`, `<sup>` and `<br>` inline.
//!
//! For a document built only from those constructs, ADF -> Markdown -> ADF
//! is structurally identical (ignoring `localId`s and presentational
//! attributes such as table widths); see `tests/markup.rs`.

mod adf;
pub mod fidelity;
mod html;
mod markdown;
pub mod storage;
mod to_markdown;
mod to_storage;

use std::fmt;

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};

pub use fidelity::unrepresentable;
pub(crate) use html::decode as decode_entities;
pub use markdown::markdown_to_adf;
pub use to_markdown::render_markdown;
pub use to_storage::{adf_to_storage, adf_to_storage_checked};

// ---------------------------------------------------------------------------
// Formats

/// Body formats for reading and writing (D11). `html` is not one of them:
/// HTML is not a format Atlassian stores, and converting it loses content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum BodyFormat {
    /// Markdown, converted through ADF (see `atlx describe --records` for the extensions).
    #[default]
    #[value(alias = "md")]
    Markdown,
    /// Atlassian Document Format JSON, sent and returned unchanged.
    Adf,
    /// Confluence storage format (XHTML), checked for well-formedness and
    /// otherwise passed through unchanged.
    Storage,
    /// Plain text: blank lines separate paragraphs.
    Plain,
}

/// Input format accepted by the commands that have not moved to
/// [`BodyFormat`] yet. Kept so the tree compiles until the product packages
/// switch; `Html` is no longer trusted (see [`to_storage`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum TextFormat {
    #[default]
    Html,
    #[value(alias = "md")]
    Markdown,
    Plain,
}

// ---------------------------------------------------------------------------
// Losses

/// Something a converter could not represent in the target format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Loss {
    /// The node, mark or construct, e.g. `extension:jira`, `layoutSection`,
    /// `mark:textColor`, `html`.
    pub node: String,
    /// Where it was: a JSON pointer into the ADF input (`/content/3`),
    /// `line N` for Markdown input, or the enclosing node type when only
    /// that is known.
    pub at: String,
    /// What happened to it, in a sentence.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl Loss {
    pub fn new(node: impl Into<String>, at: impl Into<String>, detail: impl Into<String>) -> Self {
        Loss {
            node: node.into(),
            at: at.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.node, self.at)?;
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}

/// The distinct node names of `losses`, in first-seen order.
pub fn loss_nodes(losses: &[Loss]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in losses {
        if !out.contains(&l.node) {
            out.push(l.node.clone());
        }
    }
    out
}

/// The envelope warning for a lossy read, or `None` when nothing was lost:
/// `{"code": "lossy_body", "nodes": [...], "message": ...}`.
pub fn lossy_warning(losses: &[Loss]) -> Option<Value> {
    if losses.is_empty() {
        return None;
    }
    Some(json!({
        "code": "lossy_body",
        "nodes": loss_nodes(losses),
        "message": "the body has content Markdown cannot show; it is marked with <!-- atlx: ... --> comments. Read it with the adf body format to see it, and do not write this Markdown back as a full body.",
    }))
}

/// Markdown (or HTML) parsed into ADF, with what could not be mapped.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    pub adf: Value,
    pub lost: Vec<Loss>,
}

impl Converted {
    /// The ADF, or a `refused` error listing every construct that has no
    /// ADF mapping. Writes call this so nothing is sent when anything would
    /// be lost.
    pub fn into_lossless(self) -> Result<Value, BodyError> {
        if self.lost.is_empty() {
            Ok(self.adf)
        } else {
            Err(BodyError::refused(
                "the body has content that cannot be converted to ADF without loss",
                self.lost,
                "remove or rewrite the listed constructs (raw HTML is not supported), or pass the body as ADF",
            ))
        }
    }
}

/// ADF rendered as Markdown, with what could not be shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub text: String,
    pub lossy: Vec<Loss>,
}

impl Rendered {
    /// See [`lossy_warning`].
    pub fn warning(&self) -> Option<Value> {
        lossy_warning(&self.lossy)
    }
}

// ---------------------------------------------------------------------------
// Errors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyErrorKind {
    /// A safety guard: the body would lose content (error code `refused`).
    Refused,
    /// The body is not valid in its declared format (not JSON, not ADF).
    Invalid,
}

/// Why a body was not accepted. Nothing has been sent when this is returned.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyError {
    pub kind: BodyErrorKind,
    pub message: String,
    pub losses: Vec<Loss>,
    pub hint: String,
}

impl BodyError {
    pub fn refused(message: impl Into<String>, losses: Vec<Loss>, hint: impl Into<String>) -> Self {
        BodyError {
            kind: BodyErrorKind::Refused,
            message: message.into(),
            losses,
            hint: hint.into(),
        }
    }

    pub fn invalid(message: impl Into<String>, hint: impl Into<String>) -> Self {
        BodyError {
            kind: BodyErrorKind::Invalid,
            message: message.into(),
            losses: Vec::new(),
            hint: hint.into(),
        }
    }

    pub fn is_refused(&self) -> bool {
        self.kind == BodyErrorKind::Refused
    }

    /// The node names of [`BodyError::losses`], for the error body.
    pub fn nodes(&self) -> Vec<String> {
        loss_nodes(&self.losses)
    }
}

impl fmt::Display for BodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        for (i, l) in self.losses.iter().enumerate() {
            f.write_str(if i == 0 { ": " } else { "; " })?;
            write!(f, "{l}")?;
        }
        if !self.hint.is_empty() {
            write!(f, " (hint: {})", self.hint)?;
        }
        Ok(())
    }
}

impl std::error::Error for BodyError {}

// ---------------------------------------------------------------------------
// Checked entry points

/// A body ready to send: ADF (Jira, Confluence `atlas_doc_format`) or
/// checked storage (Confluence `storage`).
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Adf(Value),
    Storage(String),
}

/// Parse a body given on the command line. Markdown that would lose
/// anything is refused; ADF must be a JSON `doc`; storage must be well-formed
/// and use only storage-format elements. Nothing is converted silently.
pub fn parse_body(s: &str, f: BodyFormat) -> Result<Body, BodyError> {
    match f {
        BodyFormat::Markdown => markdown_to_adf(s).into_lossless().map(Body::Adf),
        BodyFormat::Plain => Ok(Body::Adf(text_to_adf(s))),
        BodyFormat::Adf => parse_adf(s).map(Body::Adf),
        BodyFormat::Storage => {
            storage::check(s)?;
            Ok(Body::Storage(s.to_string()))
        }
    }
}

/// Parse an ADF JSON document (or a single block node, which is wrapped).
pub fn parse_adf(s: &str) -> Result<Value, BodyError> {
    let v: Value = serde_json::from_str(s).map_err(|e| {
        BodyError::invalid(
            format!("the body is not valid JSON: {e}"),
            "pass an ADF document such as {\"type\":\"doc\",\"version\":1,\"content\":[...]}",
        )
    })?;
    match v["type"].as_str() {
        Some("doc") if v["content"].is_array() => Ok(v),
        Some(t) if t != "doc" && v.is_object() => {
            Ok(json!({"type": "doc", "version": 1, "content": [v]}))
        }
        _ => Err(BodyError::invalid(
            "the body is not an ADF document",
            "pass an ADF document such as {\"type\":\"doc\",\"version\":1,\"content\":[...]}",
        )),
    }
}

/// HTML / storage -> ADF, with what the reader could not map.
pub fn html_to_adf_checked(s: &str) -> Converted {
    let (adf, lost) = html::parse(s, false);
    Converted { adf, lost }
}

/// Confluence storage -> Markdown for reads: what the storage reader or the
/// Markdown renderer could not represent is marked in the text and listed.
pub fn render_storage(s: &str) -> Rendered {
    let (adf, mut lossy) = html::parse(s, true);
    let r = render_markdown(&adf);
    lossy.extend(r.lossy);
    Rendered {
        text: r.text,
        lossy,
    }
}

/// Markdown -> storage, refusing anything either step would lose.
pub fn markdown_to_storage_checked(s: &str) -> Result<String, BodyError> {
    let adf = markdown_to_adf(s).into_lossless()?;
    let (out, lost) = adf_to_storage_checked(&adf);
    if lost.is_empty() {
        Ok(out)
    } else {
        Err(BodyError::refused(
            "the body has content that storage format cannot hold",
            lost,
            "remove the listed constructs, or pass the body as ADF",
        ))
    }
}

// ---------------------------------------------------------------------------
// Compatibility wrappers (callers switch to the checked API above)

/// Any supported input -> ADF (Jira). Lossy: prefer [`parse_body`].
pub fn to_adf(s: &str, f: TextFormat) -> Value {
    match f {
        TextFormat::Plain => text_to_adf(s),
        TextFormat::Markdown => markdown_to_adf(s).adf,
        TextFormat::Html => html_to_adf(s),
    }
}

/// Any supported input -> Confluence storage format. Fails closed:
/// - `Html` is sent unchanged only when it is well-formed storage that uses
///   no element outside the storage vocabulary ([`storage::check`]);
///   anything else would be silently dropped by Confluence, so it is refused.
/// - `Markdown` is refused when any construct would be lost on the way.
pub fn to_storage(s: &str, f: TextFormat) -> Result<String, BodyError> {
    match f {
        TextFormat::Html => {
            storage::check(s)?;
            Ok(s.to_string())
        }
        TextFormat::Markdown => markdown_to_storage_checked(s),
        TextFormat::Plain => Ok(text_to_storage(s)),
    }
}

/// HTML / storage -> ADF. Lossy: prefer [`html_to_adf_checked`].
pub fn html_to_adf(s: &str) -> Value {
    html::parse(s, false).0
}

/// ADF -> Markdown. Lossy parts are marked in the text; use
/// [`render_markdown`] to get the list.
pub fn adf_to_markdown(v: &Value) -> String {
    render_markdown(v).text
}

/// Markdown -> Confluence storage format (via ADF). Lossy: prefer
/// [`markdown_to_storage_checked`].
pub fn markdown_to_storage(s: &str) -> String {
    adf_to_storage(&markdown_to_adf(s).adf)
}

/// Confluence storage format -> Markdown (via ADF), with loss markers; use
/// [`render_storage`] to get the list.
pub fn storage_to_markdown(html: &str) -> String {
    render_storage(html).text
}

/// Alias of [`adf_to_markdown`], kept for existing callers.
pub fn adf_to_text(v: &Value) -> String {
    adf_to_markdown(v)
}

/// Alias of [`storage_to_markdown`], kept for existing callers.
pub fn storage_to_text(html: &str) -> String {
    storage_to_markdown(html)
}

// ---------------------------------------------------------------------------
// Plain text and small helpers

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
            {"type":"paragraph","content":[{"type":"mention","attrs":{"id":"557058:1","text":"@Ana"}},{"type":"text","text":" please"}]},
            {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"fn main() {}"}]}
        ]});
        assert_eq!(
            adf_to_text(&doc),
            "## Title\n\n- one\n- [docs](https://x.y)\n\n@[Ana](accountid:557058:1) please\n\n```rust\nfn main() {}\n```"
        );
    }

    #[test]
    fn markdown_roundtrips_through_adf() {
        let md = "## Plan\n\nShip **today** with `cargo`, see [docs](https://x.y).\n\n- one\n- two\n\n1. a\n2. b\n\n```sh\nmake\n```\n\n> quoted";
        let adf = markdown_to_adf(md).adf;
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

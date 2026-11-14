//! Markdown (CommonMark + GFM tables, strikethrough and task lists) -> ADF.

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};

use super::adf::{Builder, IMAGE, node};
use super::html;

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Markdown -> ADF document.
pub fn markdown_to_adf(s: &str) -> Value {
    let s = s.replace("\r\n", "\n");
    let mut b = Builder::new();
    let mut in_head = false;
    let mut html_block = String::new();
    // Alt text for images being collected: (url, alt).
    let mut image: Option<(String, String)> = None;

    for ev in Parser::new_ext(&s, options()) {
        if let Some((_, alt)) = image.as_mut() {
            match &ev {
                Event::Text(t) | Event::Code(t) => {
                    alt.push_str(t);
                    continue;
                }
                Event::End(TagEnd::Image) => {}
                _ => continue,
            }
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => b.open("p", node("paragraph", Map::new())),
                Tag::Heading { level, .. } => {
                    let mut n = node("heading", Map::new());
                    n["attrs"] = json!({"level": level as usize});
                    b.open("h", n);
                }
                Tag::BlockQuote(_) => b.open("blockquote", node("blockquote", Map::new())),
                Tag::CodeBlock(k) => {
                    let mut n = node("codeBlock", Map::new());
                    if let CodeBlockKind::Fenced(info) = k
                        && let Some(lang) = info.split_whitespace().next()
                    {
                        n["attrs"] = json!({"language": lang});
                    }
                    b.open("pre", n);
                }
                Tag::HtmlBlock => html_block.clear(),
                Tag::List(start) => {
                    let mut n = node(
                        if start.is_some() {
                            "orderedList"
                        } else {
                            "bulletList"
                        },
                        Map::new(),
                    );
                    if let Some(st) = start.filter(|&st| st != 1) {
                        n["attrs"] = json!({"order": st});
                    }
                    b.open("list", n);
                }
                Tag::Item => b.open("li", node("listItem", Map::new())),
                Tag::Table(_) => b.open("table", node("table", Map::new())),
                Tag::TableHead => {
                    in_head = true;
                    b.open("tr", node("tableRow", Map::new()));
                }
                Tag::TableRow => b.open("tr", node("tableRow", Map::new())),
                Tag::TableCell => b.open(
                    "td",
                    node(
                        if in_head { "tableHeader" } else { "tableCell" },
                        Map::new(),
                    ),
                ),
                Tag::Emphasis => b.push_mark("em", json!({"type": "em"})),
                Tag::Strong => b.push_mark("strong", json!({"type": "strong"})),
                Tag::Strikethrough => b.push_mark("s", json!({"type": "strike"})),
                Tag::Superscript => {
                    b.push_mark("sup", json!({"type": "subsup", "attrs": {"type": "sup"}}))
                }
                Tag::Subscript => {
                    b.push_mark("sub", json!({"type": "subsup", "attrs": {"type": "sub"}}))
                }
                Tag::Link {
                    link_type,
                    dest_url,
                    ..
                } => {
                    let href = if link_type == LinkType::Email && !dest_url.starts_with("mailto:") {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    };
                    b.push_mark("a", json!({"type": "link", "attrs": {"href": href}}));
                }
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableRow
                | TagEnd::TableCell => b.close(),
                TagEnd::TableHead => {
                    in_head = false;
                    b.close();
                }
                TagEnd::CodeBlock => b.close(),
                TagEnd::HtmlBlock => {
                    for n in html::html_blocks(&html_block) {
                        b.push(n);
                    }
                    html_block.clear();
                }
                TagEnd::Emphasis => b.pop_mark("em"),
                TagEnd::Strong => b.pop_mark("strong"),
                TagEnd::Strikethrough => b.pop_mark("s"),
                TagEnd::Superscript => b.pop_mark("sup"),
                TagEnd::Subscript => b.pop_mark("sub"),
                TagEnd::Link => b.pop_mark("a"),
                TagEnd::Image => {
                    if let Some((url, alt)) = image.take() {
                        b.push(json!({"type": IMAGE, "attrs": {"url": url, "alt": alt}}));
                    }
                }
                _ => {}
            },
            Event::Text(t) => b.text(&t),
            Event::Code(t) => {
                b.push_mark("code", json!({"type": "code"}));
                b.text(&t);
                b.pop_mark("code");
            }
            Event::Html(t) => html_block.push_str(&t),
            Event::InlineHtml(t) => inline_html(&mut b, &t),
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.push(json!({"type": "hardBreak"})),
            Event::Rule => b.push(json!({"type": "rule"})),
            Event::TaskListMarker(done) => {
                if let Some(parent) = b.parent_of_mut("listItem") {
                    parent["type"] = "taskList".into();
                    if let Some(o) = parent.get_mut("attrs").and_then(Value::as_object_mut) {
                        o.remove("order");
                    }
                }
                if let Some(item) = b.open_node_mut("listItem") {
                    item["type"] = "taskItem".into();
                    item["attrs"] = json!({"state": if done { "DONE" } else { "TODO" }});
                }
            }
            Event::InlineMath(t) | Event::DisplayMath(t) | Event::FootnoteReference(t) => {
                b.text(&t)
            }
        }
    }
    b.finish()
}

/// The small set of inline HTML tags Markdown writers use for formatting
/// Markdown lacks (underline, sub/superscript, line breaks). Anything else
/// stays literal text.
fn inline_html(b: &mut Builder, raw: &str) {
    let t = raw.trim();
    let Some(inner) = t.strip_prefix('<').and_then(|t| t.strip_suffix('>')) else {
        b.text(raw);
        return;
    };
    let closing = inner.starts_with('/');
    let name = inner
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim()
        .to_ascii_lowercase();
    let mark = match name.as_str() {
        "br" => {
            b.push(json!({"type": "hardBreak"}));
            return;
        }
        "u" | "ins" => json!({"type": "underline"}),
        "s" | "del" | "strike" => json!({"type": "strike"}),
        "b" | "strong" => json!({"type": "strong"}),
        "i" | "em" => json!({"type": "em"}),
        "sub" => json!({"type": "subsup", "attrs": {"type": "sub"}}),
        "sup" => json!({"type": "subsup", "attrs": {"type": "sup"}}),
        _ => {
            b.text(raw);
            return;
        }
    };
    let tag = format!("html:{name}");
    if closing {
        b.pop_mark(&tag);
    } else {
        b.push_mark(&tag, mark);
    }
}

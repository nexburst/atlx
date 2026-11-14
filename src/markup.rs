//! Conversions between text formats and Atlassian's rich formats:
//! ADF (Jira descriptions/comments) and Confluence storage format (XHTML).
//!
//! Input bodies follow twg's `--*-format html|markdown|plain` convention.

use clap::ValueEnum;
use serde_json::{Value, json};

/// Input format for bodies, descriptions and comments (twg: html is the default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum TextFormat {
    #[default]
    Html,
    #[value(alias = "md")]
    Markdown,
    Plain,
}

/// Any supported input -> ADF (Jira). HTML goes through markdown, so it is
/// simplified to headings, lists, code, quotes, bold, inline code and links.
pub fn to_adf(s: &str, f: TextFormat) -> Value {
    match f {
        TextFormat::Plain => text_to_adf(s),
        TextFormat::Markdown => md_to_adf(s),
        TextFormat::Html => md_to_adf(&storage_to_text(s)),
    }
}

/// Any supported input -> Confluence storage format. HTML passes through.
pub fn to_storage(s: &str, f: TextFormat) -> String {
    match f {
        TextFormat::Html => s.to_string(),
        TextFormat::Markdown => md_to_storage(s),
        TextFormat::Plain => text_to_storage(s),
    }
}

fn paragraphs(s: &str) -> impl Iterator<Item = &str> {
    s.split("\n\n").map(|p| p.trim_matches('\n')).filter(|p| !p.trim().is_empty())
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

// ---------------------------------------------------------------------------
// Minimal markdown: block structure plus **bold**, `code` and [links](url).

#[derive(Debug, PartialEq)]
enum Block {
    Heading(usize, String),
    Para(String),
    List { ordered: bool, items: Vec<String> },
    Code(String, String),
    Quote(String),
    Rule,
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let n = t.bytes().take_while(|&b| b == b'#').count();
    ((1..=6).contains(&n) && t[n..].starts_with(' ')).then(|| (n, t[n + 1..].trim()))
}

fn list_item(t: &str) -> Option<(bool, &str)> {
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some((false, rest));
        }
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0 && t[digits..].starts_with(". ")).then(|| (true, &t[digits + 2..]))
}

fn parse_md(s: &str) -> Vec<Block> {
    let s = s.replace("\r\n", "\n");
    let lines: Vec<&str> = s.lines().collect();
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join(" ")));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(lang) = t.strip_prefix("```") {
            flush(&mut para, &mut out);
            let lang = lang.trim().to_string();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            out.push(Block::Code(lang, code.join("\n")));
            i += 1;
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some((level, text)) = heading(t) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(level, text.to_string()));
        } else if t == "---" || t == "***" || t == "___" {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if let Some((ordered, first)) = list_item(t) {
            flush(&mut para, &mut out);
            let mut items = vec![first.to_string()];
            while let Some((o, item)) = lines.get(i + 1).and_then(|l| list_item(l.trim())) {
                if o != ordered {
                    break;
                }
                items.push(item.to_string());
                i += 1;
            }
            out.push(Block::List { ordered, items });
        } else if t.starts_with('>') {
            flush(&mut para, &mut out);
            let mut q = vec![t.trim_start_matches('>').trim()];
            while let Some(l) = lines.get(i + 1).map(|l| l.trim()).filter(|l| l.starts_with('>')) {
                q.push(l.trim_start_matches('>').trim());
                i += 1;
            }
            out.push(Block::Quote(q.join(" ")));
        } else {
            para.push(t);
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

#[derive(Debug, PartialEq)]
enum Inline {
    Text(String),
    Bold(String),
    Code(String),
    Link(String, String),
}

fn parse_inline(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        let parsed = if let Some(r) = rest.strip_prefix("**") {
            r.find("**").map(|e| (Inline::Bold(r[..e].to_string()), &r[e + 2..]))
        } else if let Some(r) = rest.strip_prefix('`') {
            r.find('`').map(|e| (Inline::Code(r[..e].to_string()), &r[e + 1..]))
        } else if let Some(r) = rest.strip_prefix('[') {
            r.find("](").and_then(|m| {
                let after = &r[m + 2..];
                after.find(')').map(|e| (Inline::Link(r[..m].to_string(), after[..e].to_string()), &after[e + 1..]))
            })
        } else {
            None
        };
        match parsed {
            Some((node, next)) => {
                if !text.is_empty() {
                    out.push(Inline::Text(std::mem::take(&mut text)));
                }
                out.push(node);
                rest = next;
            }
            None => {
                let c = rest.chars().next().unwrap();
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !text.is_empty() {
        out.push(Inline::Text(text));
    }
    out
}

fn inline_adf(s: &str) -> Vec<Value> {
    parse_inline(s)
        .into_iter()
        .filter_map(|n| {
            let (text, mark) = match n {
                Inline::Text(t) => (t, None),
                Inline::Bold(t) => (t, Some(json!({"type": "strong"}))),
                Inline::Code(t) => (t, Some(json!({"type": "code"}))),
                Inline::Link(t, href) => (t, Some(json!({"type": "link", "attrs": {"href": href}}))),
            };
            if text.is_empty() {
                return None;
            }
            let mut node = json!({"type": "text", "text": text});
            if let Some(m) = mark {
                node["marks"] = json!([m]);
            }
            Some(node)
        })
        .collect()
}

fn md_to_adf(s: &str) -> Value {
    let para = |t: &str| json!({"type": "paragraph", "content": inline_adf(t)});
    let content: Vec<Value> = parse_md(s)
        .into_iter()
        .map(|b| match b {
            Block::Heading(level, t) => json!({"type": "heading", "attrs": {"level": level}, "content": inline_adf(&t)}),
            Block::Para(t) => para(&t),
            Block::List { ordered, items } => json!({
                "type": if ordered { "orderedList" } else { "bulletList" },
                "content": items.iter().map(|i| json!({"type": "listItem", "content": [para(i)]})).collect::<Vec<_>>(),
            }),
            Block::Code(lang, code) => {
                let mut node = json!({"type": "codeBlock", "content": []});
                if !code.is_empty() {
                    node["content"] = json!([{"type": "text", "text": code}]);
                }
                if !lang.is_empty() {
                    node["attrs"] = json!({"language": lang});
                }
                node
            }
            Block::Quote(t) => json!({"type": "blockquote", "content": [para(&t)]}),
            Block::Rule => json!({"type": "rule"}),
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": content})
}

fn inline_html(s: &str) -> String {
    parse_inline(s)
        .into_iter()
        .map(|n| match n {
            Inline::Text(t) => escape_html(&t),
            Inline::Bold(t) => format!("<strong>{}</strong>", escape_html(&t)),
            Inline::Code(t) => format!("<code>{}</code>", escape_html(&t)),
            Inline::Link(t, h) => format!("<a href=\"{}\">{}</a>", escape_html(&h), escape_html(&t)),
        })
        .collect()
}

fn md_to_storage(s: &str) -> String {
    parse_md(s)
        .into_iter()
        .map(|b| match b {
            Block::Heading(l, t) => format!("<h{l}>{}</h{l}>", inline_html(&t)),
            Block::Para(t) => format!("<p>{}</p>", inline_html(&t)),
            Block::List { ordered, items } => {
                let tag = if ordered { "ol" } else { "ul" };
                let lis: String = items.iter().map(|i| format!("<li>{}</li>", inline_html(i))).collect();
                format!("<{tag}>{lis}</{tag}>")
            }
            Block::Code(lang, code) => {
                let lang = if lang.is_empty() {
                    String::new()
                } else {
                    format!("<ac:parameter ac:name=\"language\">{}</ac:parameter>", escape_html(&lang))
                };
                format!(
                    "<ac:structured-macro ac:name=\"code\">{lang}<ac:plain-text-body><![CDATA[{}]]></ac:plain-text-body></ac:structured-macro>",
                    code.replace("]]>", "]]]]><![CDATA[>")
                )
            }
            Block::Quote(t) => format!("<blockquote><p>{}</p></blockquote>", inline_html(&t)),
            Block::Rule => "<hr/>".into(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ADF -> text

/// ADF document -> readable plain text (markdown-ish).
pub fn adf_to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => {
            let mut out = String::new();
            render(v, &mut out, 0);
            tidy(&out)
        }
    }
}

fn children(v: &Value, out: &mut String, depth: usize) {
    for c in v["content"].as_array().into_iter().flatten() {
        render(c, out, depth);
    }
}

fn render(v: &Value, out: &mut String, depth: usize) {
    let attr = |k: &str| v["attrs"][k].as_str().unwrap_or("");
    match v["type"].as_str().unwrap_or("") {
        "text" => {
            let text = v["text"].as_str().unwrap_or("");
            let marks = v["marks"].as_array().cloned().unwrap_or_default();
            let href = marks.iter().find(|m| m["type"] == "link").and_then(|m| m["attrs"]["href"].as_str());
            let has = |t: &str| marks.iter().any(|m| m["type"] == t);
            match href {
                Some(h) if h != text => out.push_str(&format!("[{text}]({h})")),
                _ if has("code") => out.push_str(&format!("`{text}`")),
                _ if has("strong") => out.push_str(&format!("**{text}**")),
                _ => out.push_str(text),
            }
        }
        "hardBreak" => out.push('\n'),
        "paragraph" => {
            children(v, out, depth);
            out.push_str("\n\n");
        }
        "heading" => {
            let level = v["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
            out.push_str(&"#".repeat(level));
            out.push(' ');
            children(v, out, depth);
            out.push_str("\n\n");
        }
        kind @ ("bulletList" | "orderedList" | "taskList") => {
            for (i, item) in v["content"].as_array().into_iter().flatten().enumerate() {
                let marker = match kind {
                    "orderedList" => format!("{}. ", i + 1),
                    "taskList" if item["attrs"]["state"] == "DONE" => "- [x] ".into(),
                    "taskList" => "- [ ] ".into(),
                    _ => "- ".into(),
                };
                let mut s = String::new();
                children(item, &mut s, depth + 1);
                let s = tidy(&s).replace("\n\n", "\n");
                out.push_str(&"  ".repeat(depth));
                out.push_str(&marker);
                out.push_str(&s.replace('\n', &format!("\n{}", "  ".repeat(depth + 1))));
                out.push('\n');
            }
            if depth == 0 {
                out.push('\n');
            }
        }
        "taskItem" => {
            children(v, out, depth);
            out.push('\n');
        }
        "codeBlock" => {
            out.push_str("```");
            out.push_str(attr("language"));
            out.push('\n');
            children(v, out, depth);
            out.push_str("\n```\n\n");
        }
        "blockquote" | "panel" => {
            let mut s = String::new();
            children(v, &mut s, depth);
            for line in tidy(&s).lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
        "rule" => out.push_str("---\n\n"),
        "mention" | "emoji" | "status" => {
            let t = attr("text");
            out.push_str(if t.is_empty() { attr("shortName") } else { t });
        }
        "inlineCard" | "blockCard" | "embedCard" => out.push_str(attr("url")),
        "date" => out.push_str(attr("timestamp")),
        "table" => {
            for row in v["content"].as_array().into_iter().flatten() {
                let cells: Vec<String> = row["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|cell| {
                        let mut s = String::new();
                        children(cell, &mut s, depth);
                        tidy(&s).replace('\n', " ")
                    })
                    .collect();
                out.push_str("| ");
                out.push_str(&cells.join(" | "));
                out.push_str(" |\n");
            }
            out.push('\n');
        }
        _ => children(v, out, depth),
    }
}

/// Trim trailing spaces per line and collapse runs of blank lines.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Plain text -> Confluence storage format.
pub fn text_to_storage(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    paragraphs(&s)
        .map(|p| format!("<p>{}</p>", p.lines().map(escape_html).collect::<Vec<_>>().join("<br/>")))
        .collect()
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[i..].find('"')?;
    Some(tag[i..i + end].replace("&amp;", "&"))
}

/// Confluence storage format / HTML -> markdown-ish text.
pub fn storage_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut links: Vec<Option<String>> = Vec::new();
    let mut in_pre = false;
    let mut rest = html;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('&') {
            let end = rest.char_indices().take(12).find(|&(_, c)| c == ';').map(|(i, _)| i);
            match end.and_then(|e| decode_entity(&rest[1..e]).map(|c| (e, c))) {
                Some((e, c)) => {
                    out.push(c);
                    rest = &rest[e + 1..];
                }
                None => {
                    out.push('&');
                    rest = &rest[1..];
                }
            }
            continue;
        }
        if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>").unwrap_or(cdata.len());
            let code = &cdata[..end];
            if in_pre {
                out.push_str(code);
            } else {
                out.push_str(&format!("\n\n```\n{code}\n```\n\n"));
            }
            rest = cdata.get(end + 3..).unwrap_or("");
            continue;
        }
        let Some(close) = rest.find('>') else {
            out.push_str(rest);
            rest = "";
            break;
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/')
            .collect::<String>()
            .to_ascii_lowercase();
        match (name.as_str(), closing) {
            ("br", _) => out.push('\n'),
            ("li", false) => out.push_str("\n- "),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                let n = name[1..].parse::<usize>().unwrap_or(1);
                out.push_str(&format!("\n\n{} ", "#".repeat(n)));
            }
            ("strong" | "b", _) => out.push_str("**"),
            ("code", _) if !in_pre => out.push('`'),
            ("pre", false) => {
                in_pre = true;
                out.push_str("\n\n```\n");
            }
            ("pre", true) => {
                in_pre = false;
                out.push_str("\n```\n\n");
            }
            ("a", false) => {
                let href = attr_value(tag, "href");
                if href.is_some() {
                    out.push('[');
                }
                links.push(href);
            }
            ("a", true) => {
                if let Some(Some(h)) = links.pop() {
                    out.push_str(&format!("]({h})"));
                }
            }
            ("td" | "th", true) => out.push_str(" | "),
            ("p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "blockquote", _) => {
                out.push_str("\n\n")
            }
            ("tr", true) => out.push('\n'),
            _ => {}
        }
    }
    out.push_str(rest);
    tidy(&out)
}

fn decode_entity(e: &str) -> Option<char> {
    match e {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        _ => {
            let n = e.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
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
        assert_eq!(adf_to_text(&adf), "Hello\nworld\n\nSecond para");
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
        let adf = md_to_adf(md);
        assert_eq!(adf["content"][1]["content"][1]["marks"][0]["type"], "strong");
        assert_eq!(adf["content"][2]["type"], "bulletList");
        assert_eq!(adf["content"][3]["type"], "orderedList");
        assert_eq!(adf_to_text(&adf), md);
    }

    #[test]
    fn markdown_to_storage() {
        assert_eq!(
            md_to_storage("# T\n\nA & **b**\n\n- x\n\n```\nq\n```"),
            "<h1>T</h1><p>A &amp; <strong>b</strong></p><ul><li>x</li></ul>\
             <ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[q]]></ac:plain-text-body></ac:structured-macro>"
        );
    }

    #[test]
    fn html_input_to_adf() {
        let adf = to_adf("<h2>Hi</h2><p>See <a href=\"https://x.y\">this</a> <strong>now</strong></p><ul><li>a</li></ul>", TextFormat::Html);
        assert_eq!(adf_to_text(&adf), "## Hi\n\nSee [this](https://x.y) **now**\n\n- a");
    }

    #[test]
    fn storage_conversions() {
        assert_eq!(text_to_storage("a < b\nc\n\nd"), "<p>a &lt; b<br/>c</p><p>d</p>");
        let html = "<h1>Intro</h1><p>Fish &amp; chips&#33;</p><ul><li>one</li><li>two</li></ul>\
                    <ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[x < 1]]></ac:plain-text-body></ac:structured-macro>";
        assert_eq!(storage_to_text(html), "# Intro\n\nFish & chips!\n\n- one\n- two\n\n```\nx < 1\n```");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
        assert_eq!(strip_highlight("@@@hl@@@foo@@@endhl@@@ bar"), "foo bar");
    }
}

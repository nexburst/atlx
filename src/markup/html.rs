//! A tolerant HTML / Confluence storage (XHTML) reader that builds ADF.
//!
//! It is not a full HTML parser: it tokenizes tags, text, entities and CDATA
//! sections, maps the elements atlx understands to ADF nodes, and keeps the
//! text of everything else.

use serde_json::{Map, Value, json};

use super::adf::{BREAK, Builder, IMAGE, content, node};

#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    End(String),
    Text(String),
    CData(&'a str),
}

const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "col", "area", "base", "wbr", "source",
];
const RAW_TEXT: &[&str] = &["script", "style"];

fn tokenize(html: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = html;
    let flush = |text: &mut String, out: &mut Vec<Tok>| {
        if !text.is_empty() {
            out.push(Tok::Text(std::mem::take(text)));
        }
    };
    while let Some(i) = rest.find(['<', '&']) {
        text.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('&') {
            let (c, used) = entity(rest);
            text.push_str(&c);
            rest = &rest[used..];
            continue;
        }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            flush(&mut text, &mut out);
            let end = r.find("]]>").unwrap_or(r.len());
            out.push(Tok::CData(&r[..end]));
            rest = r.get(end + 3..).unwrap_or("");
            continue;
        }
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |e| &rest[e + 1..]);
            continue;
        }
        let next = rest[1..].chars().next();
        if !next.is_some_and(|c| c.is_ascii_alphabetic() || c == '/') {
            text.push('<');
            rest = &rest[1..];
            continue;
        }
        let Some(close) = tag_end(rest) else {
            text.push_str(rest);
            rest = "";
            break;
        };
        let inner = &rest[1..close];
        rest = &rest[close + 1..];
        flush(&mut text, &mut out);
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Tok::End(name.trim().to_ascii_lowercase()));
            continue;
        }
        let (name, attrs, self_closing) = parse_tag(inner);
        if RAW_TEXT.contains(&name.as_str()) && !self_closing {
            let close_tag = format!("</{name}");
            let end = rest.to_ascii_lowercase().find(&close_tag);
            rest = match end {
                Some(e) => rest[e..].find('>').map_or("", |g| &rest[e + g + 1..]),
                None => "",
            };
            continue;
        }
        out.push(Tok::Start {
            self_closing: self_closing || VOID.contains(&name.as_str()),
            name,
            attrs,
        });
    }
    text.push_str(rest);
    flush(&mut text, &mut out);
    out
}

/// Index of the `>` ending the tag at the start of `s`, skipping quoted values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_tag(inner: &str) -> (String, Vec<(String, String)>, bool) {
    let self_closing = inner.trim_end().ends_with('/');
    let inner = inner.trim_end().trim_end_matches('/');
    let name_end = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        let mut value = String::new();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            let (v, after) = match r.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &r[1..];
                    let e = body.find(q).unwrap_or(body.len());
                    (&body[..e], body.get(e + 1..).unwrap_or(""))
                }
                _ => {
                    let e = r.find(char::is_whitespace).unwrap_or(r.len());
                    (&r[..e], &r[e..])
                }
            };
            value = decode(v);
            rest = after.trim_start();
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    (name, attrs, self_closing)
}

/// Decode entities in an attribute value or text.
pub(crate) fn decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let (c, used) = entity(&rest[i..]);
        out.push_str(&c);
        rest = &rest[i + used..];
    }
    out.push_str(rest);
    out
}

/// Decode the entity at the start of `s` (which begins with `&`). Returns the
/// decoded text and the number of bytes consumed.
fn entity(s: &str) -> (String, usize) {
    let end = s
        .char_indices()
        .take(12)
        .find(|&(_, c)| c == ';')
        .map(|(i, _)| i);
    match end.and_then(|e| decode_entity(&s[1..e]).map(|c| (e, c))) {
        Some((e, c)) => (c.to_string(), e + 1),
        None => ("&".into(), 1),
    }
}

fn decode_entity(e: &str) -> Option<char> {
    Some(match e {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "times" => '×',
        "divide" => '÷',
        "plusmn" => '±',
        "euro" => '€',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        _ => {
            let n = e.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            return char::from_u32(code);
        }
    })
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Language from `class="language-x"` / `lang-x` or `data-language`.
fn code_language(attrs: &[(String, String)]) -> Option<String> {
    if let Some(l) = attr(attrs, "data-language").filter(|l| !l.is_empty()) {
        return Some(l.to_string());
    }
    attr(attrs, "class")?
        .split_whitespace()
        .find_map(|c| c.strip_prefix("language-").or(c.strip_prefix("lang-")))
        .map(str::to_string)
}

/// Containers that only separate blocks; their own text is kept.
const TRANSPARENT_BLOCKS: &[&str] = &[
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "details",
    "summary",
    "dl",
    "dt",
    "dd",
    "center",
    "address",
    "ac:layout",
    "ac:layout-section",
    "ac:layout-cell",
    "ac:rich-text-body",
    "ac:adf-content",
    "body",
    "html",
];

/// Elements whose content is metadata, not text.
const SKIP: &[&str] = &[
    "head",
    "title",
    "template",
    "caption",
    "colgroup",
    "ac:adf-fallback",
    "ac:adf-attribute",
    "ac:task-id",
    "ac:placeholder",
    "ac:parameter",
    "ac:task-status",
];

const PANEL_MACROS: &[&str] = &[
    "info", "note", "warning", "tip", "panel", "success", "error",
];

/// What a special container (macro, ADF node, link, image) opened.
struct Frame {
    tag: String,
    opened: bool,
    name: String,
    params: Vec<(String, String)>,
    /// For `ac:link` / `ac:image`: the resource reference attributes.
    target: Vec<(String, String)>,
    texts_before: usize,
}

struct Walker {
    b: Builder,
    pre: usize,
    frames: Vec<Frame>,
    skip: Option<(String, usize, String)>,
    texts: usize,
    last_space: bool,
    param_key: String,
}

/// HTML / storage -> ADF document.
pub fn html_to_adf(html: &str) -> Value {
    let mut w = Walker {
        b: Builder::new(),
        pre: 0,
        frames: Vec::new(),
        skip: None,
        texts: 0,
        last_space: true,
        param_key: String::new(),
    };
    for tok in tokenize(html) {
        w.token(tok);
    }
    w.b.finish()
}

/// HTML -> normalized ADF block nodes (for HTML blocks inside Markdown).
pub(crate) fn html_blocks(html: &str) -> Vec<Value> {
    content(&html_to_adf(html)).to_vec()
}

impl Walker {
    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.texts += 1;
        if self.pre > 0 || self.b.top_kind() == "codeBlock" {
            self.b.text(s);
            return;
        }
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !self.last_space {
                    out.push(' ');
                }
                self.last_space = true;
            } else {
                out.push(c);
                self.last_space = false;
            }
        }
        self.b.text(&out);
    }

    /// Inside a paragraph or heading, macros are inline (status, jira, ...).
    fn inline_context(&self) -> bool {
        matches!(self.b.top_kind(), "paragraph" | "heading")
    }

    fn block_break(&mut self) {
        self.last_space = true;
        self.b.push(json!({"type": BREAK}));
    }

    fn open(&mut self, tag: &str, n: Value) {
        self.last_space = true;
        self.b.open(tag, n);
    }

    fn close(&mut self, tag: &str) {
        self.last_space = true;
        self.b.close_tag(tag);
    }

    fn token(&mut self, tok: Tok) {
        if let Some((tag, depth, buf)) = self.skip.as_mut() {
            match &tok {
                Tok::Start {
                    name, self_closing, ..
                } if name == tag && !self_closing => *depth += 1,
                Tok::End(name) if name == tag => {
                    *depth -= 1;
                    if *depth == 0 {
                        let (tag, _, buf) = self.skip.take().unwrap();
                        self.captured(&tag, buf);
                    }
                }
                Tok::Text(t) => buf.push_str(t),
                Tok::CData(t) => buf.push_str(t),
                _ => {}
            }
            return;
        }
        match tok {
            Tok::Text(t) => self.text(&t),
            Tok::CData(t) => self.cdata(t),
            Tok::Start {
                name,
                attrs,
                self_closing,
            } => {
                self.start(&name, &attrs, self_closing);
                if self_closing && !VOID.contains(&name.as_str()) {
                    self.end(&name);
                }
            }
            Tok::End(name) => self.end(&name),
        }
    }

    fn cdata(&mut self, t: &str) {
        let in_body = self
            .frames
            .last()
            .is_some_and(|f| f.tag == "ac:plain-text-body");
        if in_body || self.pre > 0 || self.b.top_kind() == "codeBlock" {
            self.b.text(t);
        } else if self
            .frames
            .last()
            .is_some_and(|f| f.tag == "ac:plain-text-link-body")
        {
            self.text(t);
        } else {
            self.b.open("pre", node("codeBlock", Map::new()));
            self.b.text(t);
            self.b.close_tag("pre");
        }
        self.texts += 1;
    }

    fn captured(&mut self, tag: &str, buf: String) {
        match tag {
            "ac:parameter" => {
                let key = std::mem::take(&mut self.param_key);
                if let Some(f) = self.frames.last_mut() {
                    f.params.push((key, buf.trim().to_string()));
                }
            }
            "ac:task-status" => {
                if let Some(item) = self.b.open_node_mut("taskItem") {
                    let done = buf.trim() == "complete";
                    item["attrs"]["state"] = if done { "DONE" } else { "TODO" }.into();
                }
            }
            _ => {}
        }
    }

    fn frame(&mut self, tag: &str, name: &str, opened: bool) {
        self.frames.push(Frame {
            tag: tag.into(),
            opened,
            name: name.into(),
            params: Vec::new(),
            target: Vec::new(),
            texts_before: self.texts,
        });
    }

    fn start(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        if SKIP.contains(&name) {
            if !self_closing {
                if name == "ac:parameter" {
                    self.param_key = attr(attrs, "ac:name").unwrap_or("").to_string();
                }
                self.skip = Some((name.into(), 1, String::new()));
            }
            return;
        }
        if TRANSPARENT_BLOCKS.contains(&name) {
            self.block_break();
            return;
        }
        let empty = Map::new;
        match name {
            "p" => {
                if self.b.top_tag() == "p" {
                    self.close("p");
                }
                self.open("p", node("paragraph", empty()));
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.b.top_tag() == "p" {
                    self.close("p");
                }
                let mut n = node("heading", empty());
                n["attrs"] = json!({"level": name[1..].parse::<u8>().unwrap_or(1)});
                self.open(name, n);
            }
            "ul" | "ol" => {
                let mut n = node(
                    if name == "ol" {
                        "orderedList"
                    } else {
                        "bulletList"
                    },
                    empty(),
                );
                if let Some(start) = attr(attrs, "start")
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .filter(|&s| s != 1 && name == "ol")
                {
                    n["attrs"] = json!({"order": start});
                }
                self.open(name, n);
            }
            "li" => {
                if self.b.nearest(&["li", "ul", "ol"]) == Some("li") {
                    self.close("li");
                }
                self.open("li", node("listItem", empty()));
            }
            "blockquote" => self.open(name, node("blockquote", empty())),
            "pre" => {
                self.pre += 1;
                let mut n = node("codeBlock", empty());
                if let Some(l) = code_language(attrs) {
                    n["attrs"] = json!({"language": l});
                }
                self.open("pre", n);
            }
            "code" if self.pre > 0 => {
                if let Some(l) = code_language(attrs)
                    && let Some(cb) = self.b.open_node_mut("codeBlock")
                    && cb["attrs"]["language"].is_null()
                {
                    cb["attrs"] = json!({"language": l});
                }
            }
            "code" | "tt" | "kbd" | "samp" => self.b.push_mark(name, json!({"type": "code"})),
            "strong" | "b" => self.b.push_mark(name, json!({"type": "strong"})),
            "em" | "i" => self.b.push_mark(name, json!({"type": "em"})),
            "u" | "ins" => self.b.push_mark(name, json!({"type": "underline"})),
            "s" | "del" | "strike" => self.b.push_mark(name, json!({"type": "strike"})),
            "sub" | "sup" => self
                .b
                .push_mark(name, json!({"type": "subsup", "attrs": {"type": name}})),
            "a" => {
                if let Some(href) = attr(attrs, "href").filter(|h| !h.is_empty()) {
                    self.b
                        .push_mark("a", json!({"type": "link", "attrs": {"href": href}}));
                }
            }
            "br" => {
                if self.pre > 0 {
                    self.b.text("\n");
                } else {
                    self.b.push(json!({"type": "hardBreak"}));
                    self.last_space = true;
                }
            }
            "hr" => {
                self.b.push(json!({"type": "rule"}));
                self.last_space = true;
            }
            "img" => {
                if let Some(src) = attr(attrs, "src").filter(|s| !s.is_empty()) {
                    let alt = attr(attrs, "alt").unwrap_or("");
                    self.b
                        .push(json!({"type": IMAGE, "attrs": {"url": src, "alt": alt}}));
                    self.last_space = false;
                }
            }
            "table" => self.open("table", node("table", empty())),
            "tr" => {
                if self.b.nearest(&["tr", "table"]) == Some("tr") {
                    self.close("tr");
                }
                self.open("tr", node("tableRow", empty()));
            }
            "td" | "th" => {
                if matches!(self.b.nearest(&["td", "th", "tr"]), Some("td" | "th")) {
                    let t = self.b.nearest(&["td", "th"]).unwrap().to_string();
                    self.close(&t);
                }
                let ty = if name == "th" {
                    "tableHeader"
                } else {
                    "tableCell"
                };
                let mut n = node(ty, empty());
                let span = |k: &str| attr(attrs, k).and_then(|v| v.parse::<u64>().ok());
                let mut a = Map::new();
                if let Some(c) = span("colspan").filter(|&c| c > 1) {
                    a.insert("colspan".into(), c.into());
                }
                if let Some(r) = span("rowspan").filter(|&r| r > 1) {
                    a.insert("rowspan".into(), r.into());
                }
                if !a.is_empty() {
                    n["attrs"] = Value::Object(a);
                }
                self.open(name, n);
            }
            "ac:task-list" => {
                self.open(name, node("taskList", empty()));
            }
            "ac:task" => {
                let mut n = node("taskItem", empty());
                n["attrs"] = json!({"state": "TODO"});
                self.open(name, n);
            }
            "ac:structured-macro" | "ac:macro" => {
                let mname = attr(attrs, "ac:name").unwrap_or("").to_string();
                let panel = PANEL_MACROS.contains(&mname.as_str());
                if panel {
                    self.open(name, node("blockquote", empty()));
                } else if !self.inline_context() {
                    self.block_break();
                }
                self.frame(name, &mname, panel);
            }
            "ac:plain-text-body" => {
                let lang = self
                    .frames
                    .last()
                    .and_then(|f| f.params.iter().find(|(k, _)| k == "language"))
                    .map(|(_, v)| v.clone())
                    .filter(|l| !l.is_empty());
                let mut n = node("codeBlock", empty());
                if let Some(l) = lang {
                    n["attrs"] = json!({"language": l});
                }
                self.open(name, n);
                self.frame(name, "", true);
            }
            "ac:adf-node" => {
                let panel = attr(attrs, "type") == Some("panel");
                if panel {
                    self.open(name, node("blockquote", empty()));
                } else {
                    self.block_break();
                }
                self.frame(name, "", panel);
            }
            "ac:link" | "ac:image" => {
                self.frame(name, "", false);
                if let Some(f) = self.frames.last_mut() {
                    f.params.extend(attrs.iter().cloned());
                }
            }
            "ac:plain-text-link-body" => self.frame(name, "", false),
            n if n.starts_with("ri:") => {
                if let Some(f) = self.frames.last_mut()
                    && matches!(f.tag.as_str(), "ac:link" | "ac:image")
                {
                    f.target.push(("\0kind".into(), n.to_string()));
                    f.target.extend(attrs.iter().cloned());
                } else if n == "ri:user"
                    && let Some(id) = attr(attrs, "ri:account-id")
                {
                    self.b.push(json!({"type": "mention", "attrs": {"id": id}}));
                }
            }
            "ac:emoticon" => {
                let short = attr(attrs, "ac:emoji-shortname")
                    .map(str::to_string)
                    .or_else(|| attr(attrs, "ac:name").map(|n| format!(":{n}:")))
                    .unwrap_or_default();
                let mut a = json!({"shortName": short});
                if let Some(f) = attr(attrs, "ac:emoji-fallback") {
                    a["text"] = f.into();
                }
                self.b.push(json!({"type": "emoji", "attrs": a}));
                self.last_space = false;
            }
            "time" => {
                if let Some(d) = attr(attrs, "datetime") {
                    self.text(d);
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, name: &str) {
        if TRANSPARENT_BLOCKS.contains(&name) {
            self.block_break();
            return;
        }
        match name {
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "blockquote"
            | "table" | "tr" | "td" | "th" | "ac:task-list" | "ac:task" => self.close(name),
            "pre" => {
                self.pre = self.pre.saturating_sub(1);
                self.close("pre");
            }
            "code" if self.pre > 0 => {}
            "code" | "tt" | "kbd" | "samp" | "strong" | "b" | "em" | "i" | "u" | "ins" | "s"
            | "del" | "strike" | "sub" | "sup" | "a" => self.b.pop_mark(name),
            "ac:structured-macro"
            | "ac:macro"
            | "ac:plain-text-body"
            | "ac:adf-node"
            | "ac:link"
            | "ac:image"
            | "ac:plain-text-link-body" => {
                let Some(i) = self.frames.iter().rposition(|f| f.tag == name) else {
                    return;
                };
                let f = self.frames.remove(i);
                if f.opened {
                    self.close(name);
                } else if name == "ac:link" {
                    self.link_end(&f);
                } else if name == "ac:image" {
                    self.image_end(&f);
                } else if name == "ac:structured-macro" || name == "ac:macro" {
                    self.macro_end(&f);
                    if !self.inline_context() {
                        self.block_break();
                    }
                } else if name == "ac:adf-node" {
                    self.block_break();
                }
            }
            _ => {}
        }
    }

    fn link_end(&mut self, f: &Frame) {
        let has_body = self.texts > f.texts_before;
        let kind = attr(&f.target, "\0kind").unwrap_or("");
        if kind == "ri:user" {
            if !has_body && let Some(id) = attr(&f.target, "ri:account-id") {
                self.b.push(json!({"type": "mention", "attrs": {"id": id}}));
                self.last_space = false;
            }
            return;
        }
        if has_body {
            return;
        }
        let label = attr(&f.target, "ri:content-title")
            .or(attr(&f.target, "ri:filename"))
            .or(attr(&f.target, "ri:value"))
            .or(attr(&f.target, "ri:space-key"))
            .unwrap_or("")
            .to_string();
        if kind == "ri:url" {
            self.b.push_mark(
                "\0link",
                json!({"type": "link", "attrs": {"href": label.clone()}}),
            );
            self.text(&label);
            self.b.pop_mark("\0link");
        } else {
            self.text(&label);
        }
    }

    fn image_end(&mut self, f: &Frame) {
        let kind = attr(&f.target, "\0kind").unwrap_or("");
        let alt = attr(&f.params, "ac:alt").unwrap_or("");
        if kind == "ri:url"
            && let Some(url) = attr(&f.target, "ri:value")
        {
            self.b
                .push(json!({"type": IMAGE, "attrs": {"url": url, "alt": alt}}));
        } else if let Some(file) = attr(&f.target, "ri:filename") {
            self.text(file);
        }
    }

    fn macro_end(&mut self, f: &Frame) {
        let p = |k: &str| attr(&f.params, k).filter(|v| !v.is_empty());
        match f.name.as_str() {
            "status" => {
                if let Some(t) = p("title") {
                    self.b.push(json!({"type": "status", "attrs": {"text": t, "color": p("colour").unwrap_or("neutral").to_lowercase()}}));
                    self.last_space = false;
                }
            }
            "jira" => {
                if let Some(k) = p("key") {
                    self.text(k);
                }
            }
            _ => {}
        }
    }
}

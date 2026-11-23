//! A tolerant HTML / Confluence storage (XHTML) reader that builds ADF.
//!
//! It is not a full HTML parser: it tokenizes tags, text, entities and CDATA
//! sections, maps the elements atlx understands to ADF nodes, and keeps the
//! text of everything else. Every element, macro or reference it cannot map
//! is reported as a [`super::Loss`]. Comments, doctypes, processing
//! instructions and `script`/`style` bodies are skipped: they are not content.

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
            // Any other character is part of the tag.
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

/// Containers that only group blocks; their text is kept and nothing about
/// them has an ADF form worth reporting.
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
    "dl",
    "dt",
    "dd",
    "center",
    "address",
    "ac:rich-text-body",
    "ac:adf-content",
    "body",
    "html",
];

/// Containers whose structure has no ADF mapping here: their text is kept,
/// the structure is reported.
const STRUCTURAL_BLOCKS: &[(&str, &str)] = &[
    ("details", "html:details"),
    ("summary", "html:summary"),
    ("ac:layout", "layoutSection"),
];

/// Inline or grouping elements whose text is kept and whose markup carries
/// no content (styling, table sections, task bodies, link bodies).
const HARMLESS: &[&str] = &[
    "span",
    "font",
    "small",
    "big",
    "cite",
    "abbr",
    "q",
    "mark",
    "label",
    "thead",
    "tbody",
    "tfoot",
    "col",
    "ac:task-body",
    "ac:link-body",
    "ac:layout-section",
    "ac:layout-cell",
    "ac:adf-extension",
];

/// Elements whose content is metadata, not text. `caption` and
/// `ac:placeholder` hold text and are reported when skipped.
const SKIP: &[&str] = &[
    "head",
    "title",
    "template",
    "caption",
    "colgroup",
    "ac:adf-fallback",
    "ac:adf-attribute",
    "ac:task-id",
    "ac:task-uuid",
    "ac:placeholder",
    "ac:parameter",
    "ac:task-status",
];

/// Storage panel macros and the ADF panel types they become.
const PANEL_MACROS: &[(&str, &str)] = &[
    ("info", "info"),
    ("tip", "success"),
    ("note", "warning"),
    ("warning", "error"),
    ("success", "success"),
    ("error", "error"),
    ("panel", "info"),
];

/// Macros this reader maps to ADF (others become extensions).
const KNOWN_MACROS: &[&str] = &["code", "noformat", "status", "expand"];

/// What a special container (macro, ADF node, link, image) opened.
struct Frame {
    tag: String,
    opened: bool,
    name: String,
    params: Vec<(String, String)>,
    /// For `ac:link` / `ac:image`: the resource reference attributes.
    target: Vec<(String, String)>,
    texts_before: usize,
    /// For macros: whether an `ac:rich-text-body` was seen.
    bodied: bool,
}

struct Walker {
    b: Builder,
    /// Emit unmapped macros as ADF extension nodes (for reads, so the
    /// renderer marks them) instead of only reporting them.
    markers: bool,
    pre: usize,
    frames: Vec<Frame>,
    skip: Option<(String, usize, String)>,
    texts: usize,
    last_space: bool,
    param_key: String,
    /// A smart link being read: (ADF card type, url).
    card: Option<(&'static str, String)>,
}

/// HTML / storage -> ADF document, with what could not be mapped.
pub(crate) fn parse(html: &str, markers: bool) -> (Value, Vec<super::Loss>) {
    let mut w = Walker {
        b: Builder::new(),
        markers,
        pre: 0,
        frames: Vec::new(),
        skip: None,
        texts: 0,
        last_space: true,
        param_key: String::new(),
        card: None,
    };
    for tok in tokenize(html) {
        w.token(tok);
    }
    w.b.finish()
}

/// HTML -> normalized ADF block nodes (for HTML blocks inside Markdown).
/// The caller reports the HTML block itself as a loss.
pub(crate) fn html_blocks(html: &str) -> Vec<Value> {
    content(&parse(html, false).0).to_vec()
}

impl Walker {
    fn lose(&mut self, node: impl Into<String>, detail: &str) {
        let at = self
            .frames
            .last()
            .map_or_else(|| self.b.top_kind().to_string(), |f| f.tag.clone());
        let at = if at.is_empty() || at == "doc" {
            "storage".to_string()
        } else {
            at
        };
        self.b.lose(super::Loss::new(node, at, detail));
    }

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
                // Markup inside a skipped element: its text is collected.
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
            "ac:adf-attribute" => {
                let key = std::mem::take(&mut self.param_key);
                if key == "panel-type"
                    && let Some(p) = self.b.open_node_mut("panel")
                {
                    p["attrs"]["panelType"] = buf.trim().into();
                } else if let Some(f) = self.frames.last_mut() {
                    f.params.push((key, buf.trim().to_string()));
                }
            }
            "ac:task-status" => {
                if let Some(item) = self.b.open_node_mut("taskItem") {
                    let done = buf.trim() == "complete";
                    item["attrs"]["state"] = if done { "DONE" } else { "TODO" }.into();
                }
            }
            "a" => {
                if let Some((ty, url)) = self.card.take() {
                    self.b.push(json!({"type": ty, "attrs": {"url": url}}));
                    self.last_space = false;
                }
            }
            "caption" | "ac:placeholder" if !buf.trim().is_empty() => {
                let node = if tag == "caption" {
                    "html:caption"
                } else {
                    "placeholder"
                };
                self.lose(node, "this text is not kept");
            }
            // Metadata (head, title, colgroup, task ids, ADF fallbacks).
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
            bodied: false,
        });
    }

    fn start(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        if SKIP.contains(&name) {
            if !self_closing {
                if name == "ac:parameter" {
                    self.param_key = attr(attrs, "ac:name").unwrap_or("").to_string();
                } else if name == "ac:adf-attribute" {
                    self.param_key = attr(attrs, "key").unwrap_or("").to_string();
                }
                self.skip = Some((name.into(), 1, String::new()));
            }
            return;
        }
        if TRANSPARENT_BLOCKS.contains(&name) {
            if name == "ac:rich-text-body"
                && let Some(f) = self.frames.last_mut()
            {
                f.bodied = true;
                if f.name == "expand"
                    && let Some(t) = attr(&f.params, "title").filter(|t| !t.is_empty())
                {
                    let t = t.to_string();
                    self.b.top_mut()["attrs"] = json!({"title": t});
                    return self.block_break();
                }
                if !f.opened && !KNOWN_MACROS.contains(&f.name.as_str()) && self.markers {
                    let n = json!({"type": "bodiedExtension", "attrs": {"extensionKey": f.name.clone()}});
                    self.block_break();
                    self.b.push(n);
                }
            }
            self.block_break();
            return;
        }
        if let Some((_, node)) = STRUCTURAL_BLOCKS.iter().find(|(t, _)| *t == name) {
            if self.markers && *node == "layoutSection" {
                self.block_break();
                self.b.push(json!({"type": "layoutSection"}));
            } else {
                self.lose(*node, "this structure is not kept; its text is");
            }
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
                let href = attr(attrs, "href").filter(|h| !h.is_empty());
                let card = match attr(attrs, "data-card-appearance") {
                    Some("inline") => Some("inlineCard"),
                    Some("block") => Some("blockCard"),
                    Some("embed") => Some("embedCard"),
                    _ => None,
                };
                match (href, card) {
                    (Some(h), Some(ty)) if !self_closing => {
                        self.card = Some((ty, h.to_string()));
                        self.skip = Some(("a".into(), 1, String::new()));
                    }
                    (Some(h), _) => {
                        let mut m = json!({"type": "link", "attrs": {"href": h}});
                        if let Some(t) = attr(attrs, "title").filter(|t| !t.is_empty()) {
                            m["attrs"]["title"] = t.into();
                        }
                        self.b.push_mark("a", m);
                    }
                    // An anchor without a target carries only its text.
                    (None, _) => {}
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
                } else {
                    self.lose("html:img", "an image without a source");
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
                let panel = PANEL_MACROS.iter().find(|(m, _)| *m == mname);
                if let Some((_, t)) = panel {
                    let mut n = node("panel", empty());
                    n["attrs"] = json!({"panelType": t});
                    self.open(name, n);
                } else if mname == "expand" {
                    let ty = if self.b.nearest(&["ac:structured-macro"]).is_some() {
                        "nestedExpand"
                    } else {
                        "expand"
                    };
                    self.open(name, node(ty, empty()));
                } else if !self.inline_context() {
                    self.block_break();
                }
                let opened = panel.is_some() || mname == "expand";
                self.frame(name, &mname, opened);
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
                if let Some(f) = self.frames.last_mut() {
                    f.bodied = true;
                }
                self.open(name, n);
                self.frame(name, "", true);
            }
            "ac:adf-node" => {
                let ty = attr(attrs, "type").unwrap_or("").to_string();
                if ty == "panel" {
                    let mut n = node("panel", empty());
                    n["attrs"] = json!({"panelType": "info"});
                    self.open(name, n);
                } else {
                    self.lose(
                        format!("adf-node:{ty}"),
                        "this ADF node is not kept; its text is",
                    );
                    self.block_break();
                }
                self.frame(name, &ty, ty == "panel");
            }
            "ac:link" | "ac:image" => {
                self.frame(name, "", false);
                if let Some(f) = self.frames.last_mut() {
                    f.params.extend(attrs.iter().cloned());
                }
            }
            "ac:plain-text-link-body" => self.frame(name, "", false),
            "ac:inline-comment-marker" => {
                self.lose("mark:annotation", "inline comments are not kept");
            }
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
                } else {
                    self.lose(n, "a resource reference outside a link is not kept");
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
                    match super::markdown::parse_date(d) {
                        Some(ms) => {
                            self.b.push(
                                json!({"type": "date", "attrs": {"timestamp": ms.to_string()}}),
                            );
                            self.last_space = false;
                        }
                        None => {
                            self.lose("date", "a date that is not YYYY-MM-DD is kept as text");
                            self.text(d);
                        }
                    }
                }
            }
            n if HARMLESS.contains(&n) => {}
            other => {
                let label = format!("html:{other}");
                self.lose(label, "this element has no ADF mapping; its text is kept");
            }
        }
    }

    fn end(&mut self, name: &str) {
        if TRANSPARENT_BLOCKS.contains(&name) || STRUCTURAL_BLOCKS.iter().any(|(t, _)| *t == name) {
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
                    self.opened_end(name, &f);
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
            // Closing tags of elements whose start was mapped, reported or
            // harmless; nothing is pending for them.
            _ => {}
        }
    }

    /// Finish a macro that opened an ADF node (panel, expand, code body).
    fn opened_end(&mut self, name: &str, f: &Frame) {
        if name != "ac:structured-macro" && name != "ac:macro" {
            return;
        }
        let p = |k: &str| attr(&f.params, k).filter(|v| !v.is_empty());
        if f.name != "expand" && (p("title").is_some() || f.name == "panel") {
            self.lose(
                format!("macro:{}", f.name),
                "the panel's title and colours are not kept",
            );
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
        if kind != "ri:url" {
            self.lose(
                format!("link:{}", kind.trim_start_matches("ri:")),
                "a link to Confluence content is kept as its text only",
            );
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
            if self.markers {
                self.b.push(json!({"type": "mediaSingle", "content": [
                    {"type": "media", "attrs": {"type": "file", "alt": file}}
                ]}));
            } else {
                self.lose("media:file", "an attached image is kept as its file name");
                self.text(file);
            }
        } else {
            self.lose("media", "an image without a URL or file is not kept");
        }
    }

    fn macro_end(&mut self, f: &Frame) {
        let p = |k: &str| attr(&f.params, k).filter(|v| !v.is_empty());
        match f.name.as_str() {
            "status" => {
                if let Some(t) = p("title") {
                    self.b.push(json!({"type": "status", "attrs": {"text": t, "color": status_color(p("colour").unwrap_or("neutral"))}}));
                    self.last_space = false;
                } else {
                    self.lose("macro:status", "a status without a title");
                }
            }
            // `code` / `noformat` with a body were opened as code blocks.
            "code" | "noformat" if f.bodied => {}
            name if f.bodied => {
                // The body's text was kept; the marker (if any) was pushed
                // when the body started.
                if !self.markers {
                    self.lose(
                        format!("extension:{name}"),
                        "this macro is not kept; its body text is",
                    );
                }
            }
            name => {
                let inline = self.inline_context();
                if self.markers {
                    let mut params = Map::new();
                    for (k, v) in &f.params {
                        params.insert(k.clone(), json!({"value": v}));
                    }
                    self.b.push(json!({
                        "type": if inline { "inlineExtension" } else { "extension" },
                        "attrs": {"extensionKey": name, "parameters": {"macroParams": params}},
                    }));
                    self.last_space = false;
                } else {
                    self.lose(format!("extension:{name}"), "this macro is not kept");
                    if name == "jira"
                        && let Some(k) = p("key")
                    {
                        self.text(k);
                    }
                }
            }
        }
    }
}

/// Storage status colours (`Green`, `Grey`, ...) -> ADF status colours.
fn status_color(c: &str) -> String {
    match c.to_ascii_lowercase().as_str() {
        "grey" | "gray" => "neutral".into(),
        other => other.into(),
    }
}

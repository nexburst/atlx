//! ADF -> Confluence storage format (XHTML with `ac:`/`ri:` elements).
//!
//! Covers the node set the Markdown dialect produces. Anything else is
//! recorded as a [`Loss`]; [`super::markdown_to_storage_checked`] refuses
//! the body in that case.

use serde_json::Value;

use super::Loss;
use super::adf::{content, is_inline, kind};
use super::escape_html;

/// ADF document -> Confluence storage format. Lossy: prefer
/// [`adf_to_storage_checked`].
pub fn adf_to_storage(v: &Value) -> String {
    adf_to_storage_checked(v).0
}

/// ADF document -> storage format, with what storage could not hold.
pub fn adf_to_storage_checked(v: &Value) -> (String, Vec<Loss>) {
    let mut w = Writer::default();
    if kind(v) == "doc" {
        w.blocks(content(v), "/content");
    } else {
        w.blocks(std::slice::from_ref(v), "");
    }
    (w.out, w.lost)
}

/// Storage panel macros for ADF panel types. The other ADF panel types
/// (`note`, `tip`, `custom`) have no storage macro.
const PANELS: &[(&str, &str)] = &[
    ("info", "info"),
    ("success", "tip"),
    ("warning", "note"),
    ("error", "warning"),
];

/// Storage status colours for ADF status colours.
const COLOURS: &[(&str, &str)] = &[
    ("neutral", "Grey"),
    ("purple", "Purple"),
    ("blue", "Blue"),
    ("red", "Red"),
    ("yellow", "Yellow"),
    ("green", "Green"),
];

#[derive(Default)]
struct Writer {
    out: String,
    task_id: usize,
    lost: Vec<Loss>,
}

fn param(name: &str, value: &str) -> String {
    format!(
        "<ac:parameter ac:name=\"{name}\">{}</ac:parameter>",
        escape_html(value)
    )
}

impl Writer {
    fn lose(&mut self, node: impl Into<String>, at: &str, detail: &str) {
        let at = if at.is_empty() { "/" } else { at };
        self.lost.push(Loss::new(node, at, detail));
    }

    fn blocks(&mut self, nodes: &[Value], base: &str) {
        let mut i = 0;
        while i < nodes.len() {
            if is_inline(&nodes[i]) {
                let start = i;
                while i < nodes.len() && is_inline(&nodes[i]) {
                    i += 1;
                }
                self.out.push_str("<p>");
                self.inline(&nodes[start..i], &format!("{base}/{start}"));
                self.out.push_str("</p>");
                continue;
            }
            self.block(&nodes[i], &format!("{base}/{i}"));
            i += 1;
        }
    }

    fn wrap(&mut self, tag: &str, f: impl FnOnce(&mut Self)) {
        self.out.push_str(&format!("<{tag}>"));
        f(self);
        let name = tag.split_whitespace().next().unwrap_or(tag);
        self.out.push_str(&format!("</{name}>"));
    }

    /// Container whose single (or leading) paragraph is written unwrapped.
    fn unwrap_first(&mut self, nodes: &[Value], only_if_single: bool, base: &str) {
        match nodes.split_first() {
            Some((first, rest))
                if kind(first) == "paragraph" && (!only_if_single || rest.is_empty()) =>
            {
                self.block_marks(first, &format!("{base}/0"));
                self.inline(content(first), &format!("{base}/0/content"));
                // `rest` starts at index 1; inline runs there are rare enough
                // that their pointer is approximate.
                self.blocks(rest, base);
            }
            _ => self.blocks(nodes, base),
        }
    }

    fn block_marks(&mut self, n: &Value, at: &str) {
        for m in n["marks"].as_array().into_iter().flatten() {
            if !matches!(kind(m), "breakout" | "border") {
                self.lose(
                    format!("mark:{}", kind(m)),
                    at,
                    "this block formatting is not written to storage format",
                );
            }
        }
    }

    fn macro_body(&mut self, name: &str, params: &str, body: &[Value], at: &str) {
        self.out.push_str(&format!(
            "<ac:structured-macro ac:name=\"{name}\">{params}<ac:rich-text-body>"
        ));
        self.blocks(body, &format!("{at}/content"));
        self.out
            .push_str("</ac:rich-text-body></ac:structured-macro>");
    }

    fn block(&mut self, n: &Value, at: &str) {
        let c = content(n);
        let kids = format!("{at}/content");
        self.block_marks(n, at);
        match kind(n) {
            "paragraph" => self.wrap("p", |w| w.inline(c, &kids)),
            "heading" => {
                let l = n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
                self.wrap(&format!("h{l}"), |w| w.inline(c, &kids));
            }
            "bulletList" => self.wrap("ul", |w| w.items(c, &kids)),
            "orderedList" => {
                let tag = match n["attrs"]["order"].as_u64() {
                    Some(o) if o != 1 => format!("ol start=\"{o}\""),
                    _ => "ol".into(),
                };
                self.wrap(&tag, |w| w.items(c, &kids));
            }
            "taskList" => self.task_list(c, &kids),
            "codeBlock" => {
                let code: String = c.iter().filter_map(|t| t["text"].as_str()).collect();
                let lang = match n["attrs"]["language"].as_str() {
                    Some(l) if !l.is_empty() => param("language", l),
                    _ => String::new(),
                };
                self.out.push_str(&format!(
                    "<ac:structured-macro ac:name=\"code\">{lang}<ac:plain-text-body><![CDATA[{}]]></ac:plain-text-body></ac:structured-macro>",
                    code.replace("]]>", "]]]]><![CDATA[>")
                ));
            }
            "blockquote" => self.wrap("blockquote", |w| w.blocks(c, &kids)),
            "panel" => {
                let t = n["attrs"]["panelType"].as_str().unwrap_or("info");
                let custom = ["panelIcon", "panelIconId", "panelIconText", "panelColor"]
                    .iter()
                    .any(|k| !n["attrs"][*k].is_null());
                match PANELS.iter().find(|(p, _)| *p == t) {
                    Some((_, name)) if !custom => self.macro_body(name, "", c, at),
                    _ => {
                        self.lose(
                            format!("panel:{t}"),
                            at,
                            "this panel type has no storage format macro",
                        );
                        self.wrap("blockquote", |w| w.blocks(c, &kids));
                    }
                }
            }
            "expand" | "nestedExpand" => {
                let title = n["attrs"]["title"].as_str().unwrap_or("");
                let params = if title.is_empty() {
                    String::new()
                } else {
                    param("title", title)
                };
                self.macro_body("expand", &params, c, at);
            }
            "rule" => self.out.push_str("<hr/>"),
            "table" => self.wrap("table", |w| {
                w.wrap("tbody", |w| {
                    for (ri, row) in c.iter().enumerate() {
                        w.wrap("tr", |w| {
                            for (ci, cell) in content(row).iter().enumerate() {
                                let tag = if kind(cell) == "tableHeader" {
                                    "th"
                                } else {
                                    "td"
                                };
                                // Column widths and backgrounds are layout
                                // only; spans are kept.
                                let mut open = tag.to_string();
                                for k in ["colspan", "rowspan"] {
                                    if let Some(v) = cell["attrs"][k].as_u64().filter(|&v| v > 1) {
                                        open.push_str(&format!(" {k}=\"{v}\""));
                                    }
                                }
                                let cp = format!("{kids}/{ri}/content/{ci}/content");
                                w.wrap(&open, |w| w.unwrap_first(content(cell), true, &cp));
                            }
                        });
                    }
                });
            }),
            "mediaSingle" => {
                for (i, m) in c.iter().enumerate() {
                    self.media(m, &format!("{kids}/{i}"));
                }
            }
            "blockCard" | "embedCard" => {
                let url = n["attrs"]["url"].as_str().unwrap_or("");
                if url.is_empty() {
                    self.lose(kind(n), at, "a smart link without a URL");
                    return;
                }
                let look = if kind(n) == "blockCard" {
                    "block"
                } else {
                    "embed"
                };
                let u = escape_html(url);
                self.out.push_str(&format!(
                    "<p><a href=\"{u}\" data-card-appearance=\"{look}\">{u}</a></p>"
                ));
            }
            other => {
                let label = match n["attrs"]["extensionKey"].as_str() {
                    Some(k) => format!("{other}:{k}"),
                    None => other.to_string(),
                };
                self.lose(label, at, "this node is not written to storage format");
                // Keep any text it holds.
                self.blocks(c, &kids);
            }
        }
    }

    fn items(&mut self, items: &[Value], base: &str) {
        for (i, it) in items.iter().enumerate() {
            let p = format!("{base}/{i}/content");
            self.wrap("li", |w| w.unwrap_first(content(it), false, &p));
        }
    }

    fn task_list(&mut self, items: &[Value], base: &str) {
        self.out.push_str("<ac:task-list>");
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            let p = format!("{base}/{i}");
            i += 1;
            if kind(it) == "taskList" {
                self.task_list(content(it), &format!("{p}/content"));
                continue;
            }
            self.task_id += 1;
            let status = if it["attrs"]["state"] == "DONE" {
                "complete"
            } else {
                "incomplete"
            };
            self.out.push_str(&format!(
                "<ac:task><ac:task-id>{}</ac:task-id><ac:task-status>{status}</ac:task-status><ac:task-body>",
                self.task_id
            ));
            self.inline(content(it), &format!("{p}/content"));
            // Nested task lists live inside the parent task's body.
            while i < items.len() && kind(&items[i]) == "taskList" {
                let np = format!("{base}/{i}/content");
                self.task_list(content(&items[i]), &np);
                i += 1;
            }
            self.out.push_str("</ac:task-body></ac:task>");
        }
        self.out.push_str("</ac:task-list>");
    }

    fn media(&mut self, m: &Value, at: &str) {
        let a = &m["attrs"];
        match a["url"].as_str() {
            Some(url) if kind(m) == "media" && a["type"] != "file" => {
                let alt = a["alt"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" ac:alt=\"{}\"", escape_html(s)))
                    .unwrap_or_default();
                self.out.push_str(&format!(
                    "<ac:image{alt}><ri:url ri:value=\"{}\"/></ac:image>",
                    escape_html(url)
                ));
            }
            _ => self.lose(
                if kind(m) == "media" {
                    format!("media:{}", a["type"].as_str().unwrap_or("unknown"))
                } else {
                    kind(m).to_string()
                },
                at,
                "only images with a URL are written to storage format",
            ),
        }
    }

    fn inline(&mut self, nodes: &[Value], base: &str) {
        let mut stack: Vec<(String, String)> = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            let at = format!("{base}/{i}");
            let want = self.marks(n, &at);
            let common = stack.iter().zip(&want).take_while(|(a, b)| a == b).count();
            while stack.len() > common {
                let (_, close) = stack.pop().unwrap();
                self.out.push_str(&close);
            }
            for m in &want[common..] {
                self.out.push_str(&m.0);
                stack.push(m.clone());
            }
            let attr = |k: &str| n["attrs"][k].as_str().unwrap_or("");
            match kind(n) {
                "text" => self
                    .out
                    .push_str(&escape_html(n["text"].as_str().unwrap_or(""))),
                "hardBreak" => self.out.push_str("<br/>"),
                "mention" => self.out.push_str(&format!(
                    "<ac:link><ri:user ri:account-id=\"{}\"/></ac:link>",
                    escape_html(attr("id"))
                )),
                // An emoji is written as its character or `:shortname:`, which
                // is the text it was parsed from.
                "emoji" => {
                    let t = attr("text");
                    self.out.push_str(&escape_html(if t.is_empty() {
                        attr("shortName")
                    } else {
                        t
                    }));
                }
                "inlineCard" => {
                    let u = escape_html(attr("url"));
                    self.out.push_str(&format!(
                        "<a href=\"{u}\" data-card-appearance=\"inline\">{u}</a>"
                    ));
                }
                "status" => {
                    let color = attr("color").to_ascii_lowercase();
                    let colour = COLOURS
                        .iter()
                        .find(|(c, _)| *c == color)
                        .map_or("Grey", |(_, s)| *s);
                    self.out.push_str(&format!(
                        "<ac:structured-macro ac:name=\"status\">{}{}</ac:structured-macro>",
                        param("title", attr("text")),
                        param("colour", colour)
                    ));
                }
                "date" => match attr("timestamp").parse::<i64>() {
                    Ok(ms) if ms.rem_euclid(86_400_000) == 0 => self.out.push_str(&format!(
                        "<time datetime=\"{}\"/>",
                        super::markdown::iso_date(ms)
                    )),
                    _ => {
                        self.lose("date", &at, "storage dates are whole days");
                        self.out.push_str(&escape_html(attr("timestamp")));
                    }
                },
                other => {
                    let label = match n["attrs"]["extensionKey"].as_str() {
                        Some(k) => format!("{other}:{k}"),
                        None => other.to_string(),
                    };
                    self.lose(
                        label,
                        &at,
                        "this inline node is not written to storage format",
                    );
                    self.out.push_str(&escape_html(attr("text")));
                }
            }
        }
        while let Some((_, close)) = stack.pop() {
            self.out.push_str(&close);
        }
    }

    /// (open tag, close tag) per mark, links outermost.
    fn marks(&mut self, n: &Value, at: &str) -> Vec<(String, String)> {
        let mut ms: Vec<(u8, String, String)> = Vec::new();
        for m in n["marks"].as_array().into_iter().flatten() {
            let (rank, open, close) = match kind(m) {
                "link" => {
                    let title = m["attrs"]["title"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .map(|t| format!(" title=\"{}\"", escape_html(t)))
                        .unwrap_or_default();
                    (
                        0,
                        format!(
                            "<a href=\"{}\"{title}>",
                            escape_html(m["attrs"]["href"].as_str().unwrap_or(""))
                        ),
                        "</a>",
                    )
                }
                "strong" => (1, "<strong>".into(), "</strong>"),
                "em" => (2, "<em>".into(), "</em>"),
                "strike" => (3, "<s>".into(), "</s>"),
                "underline" => (4, "<u>".into(), "</u>"),
                "subsup" if m["attrs"]["type"] == "sub" => (5, "<sub>".into(), "</sub>"),
                "subsup" => (5, "<sup>".into(), "</sup>"),
                "code" => (6, "<code>".into(), "</code>"),
                other => {
                    self.lose(
                        format!("mark:{other}"),
                        at,
                        "this formatting is not written to storage format",
                    );
                    continue;
                }
            };
            ms.push((rank, open, close.to_string()));
        }
        ms.sort_by_key(|m| m.0);
        ms.into_iter().map(|(_, o, c)| (o, c)).collect()
    }
}

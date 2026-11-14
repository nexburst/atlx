//! ADF -> Confluence storage format (XHTML with `ac:`/`ri:` elements).

use serde_json::Value;

use super::adf::{content, is_inline, kind};
use super::escape_html;

/// ADF document -> Confluence storage format.
pub fn adf_to_storage(v: &Value) -> String {
    let mut w = Writer::default();
    if kind(v) == "doc" {
        w.blocks(content(v));
    } else {
        w.blocks(std::slice::from_ref(v));
    }
    w.out
}

#[derive(Default)]
struct Writer {
    out: String,
    task_id: usize,
}

impl Writer {
    fn blocks(&mut self, nodes: &[Value]) {
        let mut i = 0;
        while i < nodes.len() {
            if is_inline(&nodes[i]) {
                let start = i;
                while i < nodes.len() && is_inline(&nodes[i]) {
                    i += 1;
                }
                self.out.push_str("<p>");
                self.inline(&nodes[start..i]);
                self.out.push_str("</p>");
                continue;
            }
            self.block(&nodes[i]);
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
    fn unwrap_first(&mut self, nodes: &[Value], only_if_single: bool) {
        match nodes.split_first() {
            Some((first, rest))
                if kind(first) == "paragraph" && (!only_if_single || rest.is_empty()) =>
            {
                self.inline(content(first));
                self.blocks(rest);
            }
            _ => self.blocks(nodes),
        }
    }

    fn block(&mut self, n: &Value) {
        let c = content(n);
        match kind(n) {
            "paragraph" => self.wrap("p", |w| w.inline(c)),
            "heading" => {
                let l = n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
                self.wrap(&format!("h{l}"), |w| w.inline(c));
            }
            "bulletList" => self.wrap("ul", |w| w.items(c)),
            "orderedList" => {
                let tag = match n["attrs"]["order"].as_u64() {
                    Some(o) if o != 1 => format!("ol start=\"{o}\""),
                    _ => "ol".into(),
                };
                self.wrap(&tag, |w| w.items(c));
            }
            "taskList" => self.task_list(c),
            "codeBlock" => {
                let code: String = c.iter().filter_map(|t| t["text"].as_str()).collect();
                let lang = match n["attrs"]["language"].as_str() {
                    Some(l) if !l.is_empty() => format!(
                        "<ac:parameter ac:name=\"language\">{}</ac:parameter>",
                        escape_html(l)
                    ),
                    _ => String::new(),
                };
                self.out.push_str(&format!(
                    "<ac:structured-macro ac:name=\"code\">{lang}<ac:plain-text-body><![CDATA[{}]]></ac:plain-text-body></ac:structured-macro>",
                    code.replace("]]>", "]]]]><![CDATA[>")
                ));
            }
            "blockquote" | "panel" => self.wrap("blockquote", |w| w.blocks(c)),
            "rule" => self.out.push_str("<hr/>"),
            "table" => self.wrap("table", |w| {
                w.wrap("tbody", |w| {
                    for row in c {
                        w.wrap("tr", |w| {
                            for cell in content(row) {
                                let tag = if kind(cell) == "tableHeader" {
                                    "th"
                                } else {
                                    "td"
                                };
                                let mut open = tag.to_string();
                                for k in ["colspan", "rowspan"] {
                                    if let Some(v) = cell["attrs"][k].as_u64().filter(|&v| v > 1) {
                                        open.push_str(&format!(" {k}=\"{v}\""));
                                    }
                                }
                                w.wrap(&open, |w| w.unwrap_first(content(cell), true));
                            }
                        });
                    }
                });
            }),
            "mediaSingle" | "mediaGroup" => {
                for m in c {
                    self.media(m);
                }
            }
            "media" => self.media(n),
            _ => self.blocks(c),
        }
    }

    fn items(&mut self, items: &[Value]) {
        for it in items {
            self.wrap("li", |w| w.unwrap_first(content(it), false));
        }
    }

    fn task_list(&mut self, items: &[Value]) {
        self.out.push_str("<ac:task-list>");
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            i += 1;
            if kind(it) == "taskList" {
                self.task_list(content(it));
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
            self.inline(content(it));
            // Nested task lists live inside the parent task's body.
            while i < items.len() && kind(&items[i]) == "taskList" {
                self.task_list(content(&items[i]));
                i += 1;
            }
            self.out.push_str("</ac:task-body></ac:task>");
        }
        self.out.push_str("</ac:task-list>");
    }

    fn media(&mut self, m: &Value) {
        let a = &m["attrs"];
        if let Some(url) = a["url"].as_str() {
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
    }

    fn inline(&mut self, nodes: &[Value]) {
        let mut stack: Vec<(String, String)> = Vec::new();
        for n in nodes {
            let want = marks(n);
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
                    self.out.push_str(&format!("<a href=\"{u}\">{u}</a>"));
                }
                "status" | "placeholder" => self.out.push_str(&escape_html(attr("text"))),
                "date" => self.out.push_str(&escape_html(attr("timestamp"))),
                "mediaInline" => self.media(n),
                _ => {}
            }
        }
        while let Some((_, close)) = stack.pop() {
            self.out.push_str(&close);
        }
    }
}

/// (open tag, close tag) per mark, links outermost.
fn marks(n: &Value) -> Vec<(String, String)> {
    let mut ms: Vec<(u8, String, String)> = Vec::new();
    for m in n["marks"].as_array().into_iter().flatten() {
        let (rank, open, close) = match kind(m) {
            "link" => (
                0,
                format!(
                    "<a href=\"{}\">",
                    escape_html(m["attrs"]["href"].as_str().unwrap_or(""))
                ),
                "</a>",
            ),
            "strong" => (1, "<strong>".into(), "</strong>"),
            "em" => (2, "<em>".into(), "</em>"),
            "strike" => (3, "<s>".into(), "</s>"),
            "underline" => (4, "<u>".into(), "</u>"),
            "subsup" if m["attrs"]["type"] == "sub" => (5, "<sub>".into(), "</sub>"),
            "subsup" => (5, "<sup>".into(), "</sup>"),
            "code" => (6, "<code>".into(), "</code>"),
            _ => continue,
        };
        ms.push((rank, open, close.to_string()));
    }
    ms.sort_by_key(|m| m.0);
    ms.into_iter().map(|(_, o, c)| (o, c)).collect()
}

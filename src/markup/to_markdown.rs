//! ADF -> Markdown, for reads.
//!
//! The output parses back to the same ADF through [`super::markdown_to_adf`]
//! for everything the dialect can express (see the golden tests). Anything
//! else is recorded as a [`Loss`] and marked in the text with an
//! `<!-- atlx: ... -->` comment where the node was; container nodes keep
//! their content after the marker so a reader still sees it.

use serde_json::Value;

use super::adf::{content, is_inline, kind};
use super::markdown::{ALERTS, CARDS, SCHEMES, STATUS_COLORS, iso_date, next_emoji};
use super::{Loss, Rendered};

/// ADF document (or a single node) -> Markdown, with what it could not show.
pub fn render_markdown(v: &Value) -> Rendered {
    let mut r = R { lossy: Vec::new() };
    let text = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ if kind(v) == "doc" => r.blocks(content(v), "/content", Ctx::default()),
        _ => r.blocks(std::slice::from_ref(v), "", Ctx::default()),
    };
    Rendered {
        text,
        lossy: r.lossy,
    }
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    table: bool,
    /// Use the alternate list markers (`*`, `1)`) so a list directly after
    /// another list of the same kind stays a separate list.
    alt: bool,
}

/// Marks that only affect layout (width, borders) and carry no content.
const PRESENTATIONAL_MARKS: &[&str] = &["breakout", "border", "fragment", "dataConsumer"];

struct R {
    lossy: Vec<Loss>,
}

/// A marker comment. `--` cannot appear inside an HTML comment.
fn marker(label: &str, shown: bool) -> String {
    let label = label.replace("--", "- -").replace('>', "");
    if shown {
        format!("<!-- atlx: {label} -->")
    } else {
        format!("<!-- atlx: {label} (not shown) -->")
    }
}

/// `extension:jira`, `bodiedExtension:details`, `layoutSection`, ...
fn label(n: &Value) -> String {
    let k = kind(n);
    let k = if k.is_empty() { "unknown" } else { k };
    match n["attrs"]["extensionKey"].as_str() {
        Some(key) if !key.is_empty() => format!("{k}:{key}"),
        _ => k.to_string(),
    }
}

/// A short hint for an extension marker: the first `key`/`title`/`url`
/// macro parameter, when there is one.
fn extension_hint(n: &Value) -> String {
    let params = &n["attrs"]["parameters"]["macroParams"];
    for k in ["key", "title", "url"] {
        if let Some(v) = params[k]["value"].as_str().filter(|v| !v.is_empty()) {
            return format!(" {v}");
        }
    }
    String::new()
}

impl R {
    fn lose(&mut self, node: impl Into<String>, at: &str, detail: impl Into<String>) {
        let at = if at.is_empty() { "/" } else { at };
        self.lossy.push(Loss::new(node, at, detail));
    }

    fn blocks(&mut self, nodes: &[Value], base: &str, ctx: Ctx) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut run: Vec<(usize, &Value)> = Vec::new();
        let mut prev = "";
        let mut alt = false;
        for (i, n) in nodes.iter().enumerate() {
            if is_inline(n) {
                run.push((i, n));
                prev = "";
                continue;
            }
            if !run.is_empty() {
                let s = self.inline_run(&run, base, ctx);
                parts.push(s);
                run.clear();
            }
            alt = kind(n) == prev && !alt;
            prev = kind(n);
            parts.push(self.block(n, &format!("{base}/{i}"), Ctx { alt, ..ctx }));
        }
        if !run.is_empty() {
            let s = self.inline_run(&run, base, ctx);
            parts.push(s);
        }
        // Empty paragraphs carry no content.
        parts.retain(|p| !p.is_empty());
        parts.join(if ctx.table { "<br>" } else { "\n\n" })
    }

    /// Inline nodes found directly in a block container (not valid ADF, but
    /// rendered as a paragraph rather than dropped).
    fn inline_run(&mut self, run: &[(usize, &Value)], base: &str, ctx: Ctx) -> String {
        let at = format!("{base}/{}", run[0].0);
        let nodes: Vec<Value> = run.iter().map(|(_, n)| (*n).clone()).collect();
        self.inline(&nodes, &at, ctx)
    }

    fn block_marks(&mut self, n: &Value, at: &str) {
        for m in n["marks"].as_array().into_iter().flatten() {
            let k = kind(m);
            if !PRESENTATIONAL_MARKS.contains(&k) {
                self.lose(
                    format!("mark:{k}"),
                    at,
                    format!("Markdown has no {k}; the block is shown without it"),
                );
            }
        }
    }

    fn in_table(&mut self, n: &Value, at: &str) {
        self.lose(
            kind(n),
            at,
            format!(
                "a Markdown table cell holds one line of text; this {} was flattened",
                kind(n)
            ),
        );
    }

    fn block(&mut self, n: &Value, at: &str, ctx: Ctx) -> String {
        let attr = |k: &str| n["attrs"][k].as_str().unwrap_or("");
        let kids = format!("{at}/content");
        self.block_marks(n, at);
        match kind(n) {
            "paragraph" => self.inline(content(n), &kids, ctx),
            "heading" if ctx.table => {
                self.in_table(n, at);
                format!("**{}**", self.inline(content(n), &kids, ctx))
            }
            "heading" => {
                if content(n).iter().any(|c| kind(c) == "hardBreak") {
                    self.lose(
                        "hardBreak",
                        at,
                        "a Markdown heading is one line; its line break was lost",
                    );
                }
                let level = n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
                format!(
                    "{} {}",
                    "#".repeat(level),
                    self.inline(content(n), &kids, ctx)
                )
                .trim_end()
                .to_string()
            }
            "bulletList" | "orderedList" | "taskList" | "decisionList" => {
                if kind(n) == "decisionList" {
                    self.lose(
                        "decisionList",
                        at,
                        "Markdown has no decisions; they are shown as a task list",
                    );
                }
                if ctx.table {
                    self.in_table(n, at);
                }
                let s = self.list(n, at, ctx.alt);
                let s = if kind(n) == "decisionList" {
                    format!("{}\n\n{s}", marker("decisionList", true))
                } else {
                    s
                };
                if ctx.table {
                    s.replace('\n', "<br>")
                } else {
                    s
                }
            }
            "codeBlock" => {
                let mut code = String::new();
                for c in content(n) {
                    code.push_str(c["text"].as_str().unwrap_or(""));
                }
                if ctx.table {
                    self.in_table(n, at);
                    return code_span(&code.replace('\n', " "), true);
                }
                let fence = "`".repeat(longest_run(&code, '`').max(2) + 1);
                format!("{fence}{}\n{code}\n{fence}", attr("language"))
            }
            "blockquote" | "panel" => {
                let mut head = None;
                if kind(n) == "panel" {
                    let custom = ["panelIcon", "panelIconId", "panelIconText", "panelColor"]
                        .iter()
                        .any(|k| !n["attrs"][*k].is_null());
                    match ALERTS.iter().find(|(_, t)| *t == attr("panelType")) {
                        Some((label, _)) if !custom => head = Some(format!("[!{label}]")),
                        _ => {
                            let t = attr("panelType");
                            let name = format!("panel:{}", if t.is_empty() { "custom" } else { t });
                            self.lose(
                                &name,
                                at,
                                "this panel's type, icon or colour has no Markdown form; it is shown as a quote",
                            );
                            head = Some(marker(&name, true));
                        }
                    }
                }
                let inner = self.blocks(content(n), &kids, ctx);
                if ctx.table {
                    self.in_table(n, at);
                    return inner;
                }
                let body = inner
                    .lines()
                    .map(|l| {
                        if l.is_empty() {
                            ">".into()
                        } else {
                            format!("> {l}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                match head {
                    Some(h) if h.starts_with("[!") => format!("> {h}\n{body}"),
                    Some(h) => format!("{h}\n{body}"),
                    None => body,
                }
            }
            "rule" => {
                if ctx.table {
                    self.in_table(n, at);
                    String::new()
                } else {
                    "---".into()
                }
            }
            "table" => {
                if ctx.table {
                    self.in_table(n, at);
                }
                self.table(n, at)
            }
            "mediaSingle" => {
                let media = content(n);
                match media {
                    [m] if kind(m) == "media" => self.media(m, &format!("{kids}/0")),
                    _ => {
                        let label = media
                            .iter()
                            .find(|c| kind(c) == "caption")
                            .map_or("mediaSingle", |_| "caption");
                        self.lose(
                            label,
                            at,
                            "an image with a caption or several parts has no Markdown form",
                        );
                        let shown: Vec<String> = media
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| kind(c) == "media")
                            .map(|(i, c)| self.media(c, &format!("{kids}/{i}")))
                            .collect();
                        let mut out = marker(label, true);
                        for s in shown {
                            out.push(' ');
                            out.push_str(&s);
                        }
                        out
                    }
                }
            }
            "mediaGroup" | "media" => {
                let names: Vec<String> = if kind(n) == "media" {
                    vec![media_name(n)]
                } else {
                    content(n).iter().map(media_name).collect()
                };
                self.lose(kind(n), at, "attached files are not shown in Markdown");
                let names: Vec<String> = names.into_iter().filter(|s| !s.is_empty()).collect();
                let l = if names.is_empty() {
                    kind(n).to_string()
                } else {
                    format!("{} {}", kind(n), names.join(", "))
                };
                marker(&l, false)
            }
            "expand" | "nestedExpand" => {
                if ctx.table {
                    self.in_table(n, at);
                }
                let title = attr("title");
                let body = self.blocks(content(n), &kids, Ctx::default());
                let mut out = String::from("<details>");
                if !title.is_empty() {
                    out.push_str(&format!(
                        "\n<summary>{}</summary>",
                        super::escape_html(title)
                    ));
                }
                out.push_str("\n\n");
                if !body.is_empty() {
                    out.push_str(&body);
                    out.push_str("\n\n");
                }
                out.push_str("</details>");
                if ctx.table {
                    out.replace('\n', " ")
                } else {
                    out
                }
            }
            "blockCard" | "embedCard" => {
                let url = attr("url");
                if url.is_empty() {
                    self.lose(
                        kind(n),
                        at,
                        "a smart link without a URL has no Markdown form",
                    );
                    return marker(kind(n), false);
                }
                let title = if kind(n) == "blockCard" {
                    "card:block"
                } else {
                    "card:embed"
                };
                format!(
                    "[{}]({} \"{title}\")",
                    label_text(url, ctx.table),
                    href(url)
                )
            }
            "layoutSection"
            | "layoutColumn"
            | "bodiedExtension"
            | "multiBodiedExtension"
            | "extensionFrame"
            | "syncBlock"
            | "bodiedSyncBlock" => {
                let l = label(n);
                self.lose(
                    &l,
                    at,
                    "Markdown has no form for this; its content is shown without it",
                );
                let body = self.blocks(content(n), &kids, ctx);
                if body.is_empty() {
                    marker(&l, true)
                } else {
                    format!("{}\n\n{body}", marker(&l, true))
                }
            }
            _ => {
                let l = format!("{}{}", label(n), extension_hint(n));
                self.lose(
                    label(n),
                    at,
                    "Markdown has no form for this node; it is not shown",
                );
                let body = self.blocks(content(n), &kids, ctx);
                if body.is_empty() {
                    marker(&l, false)
                } else {
                    format!("{}\n\n{body}", marker(&l, true))
                }
            }
        }
    }

    /// An external image as `![alt](url)`; anything else as a marker.
    fn media(&mut self, m: &Value, at: &str) -> String {
        let a = &m["attrs"];
        match (a["type"].as_str(), a["url"].as_str()) {
            (Some("external") | None, Some(url)) => {
                let alt = a["alt"].as_str().unwrap_or("");
                format!("![{}]({})", label_text(alt, false), href(url))
            }
            _ => {
                self.lose(
                    format!("media:{}", a["type"].as_str().unwrap_or("unknown")),
                    at,
                    "only images with a URL can be shown in Markdown",
                );
                let name = media_name(m);
                let t = a["type"].as_str().unwrap_or("unknown");
                if name.is_empty() {
                    marker(&format!("media:{t}"), false)
                } else {
                    marker(&format!("media:{t} {name}"), false)
                }
            }
        }
    }

    fn list(&mut self, n: &Value, at: &str, alt: bool) -> String {
        let bullet = if alt { "* " } else { "- " };
        let ordered = kind(n) == "orderedList";
        let mut num = n["attrs"]["order"].as_u64().unwrap_or(1);
        let mut out = String::new();
        for (i, item) in content(n).iter().enumerate() {
            let ip = format!("{at}/content/{i}");
            let body;
            let marker;
            match kind(item) {
                "taskList" | "bulletList" | "orderedList" => {
                    // Nested task list: indent under the previous item.
                    if !out.is_empty() {
                        out.push_str("\n  ");
                    }
                    let s = self.list(item, &ip, false);
                    out.push_str(&indent(&s, 2));
                    continue;
                }
                "taskItem" | "decisionItem" => {
                    let check = if item["attrs"]["state"] == "DONE" {
                        "[x] "
                    } else {
                        "[ ] "
                    };
                    marker = format!("{bullet}{check}");
                    body = self.inline(content(item), &format!("{ip}/content"), Ctx::default());
                }
                "listItem" => {
                    marker = match (ordered, alt) {
                        (true, false) => format!("{num}. "),
                        (true, true) => format!("{num}) "),
                        _ => bullet.to_string(),
                    };
                    num += 1;
                    body = self.item_body(item, &ip);
                }
                _ => {
                    // Not valid in a list: render it as an item of its own.
                    marker = bullet.to_string();
                    body = self.block(item, &ip, Ctx::default());
                }
            }
            if !out.is_empty() {
                out.push('\n');
            }
            let width = if marker.ends_with("] ") {
                2
            } else {
                marker.len()
            };
            let line = format!("{marker}{}", indent(&body, width));
            out.push_str(line.trim_end());
        }
        out
    }

    fn item_body(&mut self, item: &Value, at: &str) -> String {
        let mut s = String::new();
        for (i, c) in content(item).iter().enumerate() {
            let r = self.block(c, &format!("{at}/content/{i}"), Ctx::default());
            if i > 0 {
                // An ordered list may only interrupt a paragraph when it starts at 1.
                let nested = matches!(kind(c), "bulletList" | "taskList")
                    || (kind(c) == "orderedList" && c["attrs"]["order"].as_u64().unwrap_or(1) == 1);
                s.push_str(if nested { "\n" } else { "\n\n" });
            }
            s.push_str(&r);
        }
        s
    }

    fn table(&mut self, n: &Value, at: &str) -> String {
        let ctx = Ctx {
            table: true,
            alt: false,
        };
        let rows_in = content(n);
        let is_header_row = |row: &Value| {
            !content(row).is_empty() && content(row).iter().all(|c| kind(c) == "tableHeader")
        };
        let header = rows_in.first().is_some_and(is_header_row);
        let mut widths = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for (ri, row) in rows_in.iter().enumerate() {
            let rp = format!("{at}/content/{ri}");
            if (ri > 0 || !header) && content(row).iter().any(|c| kind(c) == "tableHeader") {
                self.lose(
                    "tableHeader",
                    &rp,
                    "Markdown tables have header cells in the first row only; these became plain cells",
                );
            }
            widths.push(content(row).len());
            let mut cells = Vec::new();
            for (ci, cell) in content(row).iter().enumerate() {
                let cp = format!("{rp}/content/{ci}");
                for k in ["colspan", "rowspan"] {
                    if cell["attrs"][k].as_u64().is_some_and(|v| v > 1) {
                        self.lose(
                            k,
                            &cp,
                            "Markdown tables cannot merge cells; the merged cell is shown once",
                        );
                    }
                }
                let blocks = content(cell);
                if blocks.len() > 1 {
                    self.lose(
                        "tableCell",
                        &cp,
                        "a Markdown table cell holds one paragraph; several were joined with <br>",
                    );
                }
                cells.push(self.blocks(blocks, &format!("{cp}/content"), ctx));
            }
            rows.push(cells);
        }
        if widths.iter().any(|w| Some(w) != widths.first()) {
            self.lose(
                "tableRow",
                at,
                "rows have different numbers of cells; short rows were padded",
            );
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
        if header && rows.first().is_some_and(|r| r.iter().all(String::is_empty)) {
            self.lose(
                "tableHeader",
                at,
                "an empty header row reads back as a table without one",
            );
        }
        let line = |cells: &[String]| {
            let mut l = String::from("|");
            for i in 0..cols {
                let c = cells.get(i).map_or("", String::as_str);
                l.push(' ');
                l.push_str(c);
                l.push_str(if c.is_empty() { "|" } else { " |" });
            }
            l
        };
        let mut out = Vec::new();
        if !header {
            // A blank header row stands for "no header".
            out.push(line(&[]));
            out.push(format!("|{}", " --- |".repeat(cols)));
        }
        for (i, r) in rows.iter().enumerate() {
            out.push(line(r));
            if i == 0 && header {
                out.push(format!("|{}", " --- |".repeat(cols)));
            }
        }
        out.join("\n")
    }

    fn marks_of(&mut self, n: &Value, at: &str) -> (Vec<Mark>, bool) {
        let mut out = Vec::new();
        let mut code = false;
        for m in n["marks"].as_array().into_iter().flatten() {
            match kind(m) {
                "link" => {
                    let h = m["attrs"]["href"].as_str().unwrap_or("").to_string();
                    let title = m["attrs"]["title"].as_str().unwrap_or("").to_string();
                    if SCHEMES.iter().any(|s| h.starts_with(s))
                        || CARDS.iter().any(|(t, _)| *t == title)
                    {
                        self.lose(
                            "mark:link",
                            at,
                            "this link's address or title is reserved for mentions, statuses, dates or cards",
                        );
                    }
                    out.push(Mark::Link(h, title));
                }
                "strong" => out.push(Mark::Strong),
                "em" => out.push(Mark::Em),
                "underline" => out.push(Mark::Underline),
                "strike" => out.push(Mark::Strike),
                "subsup" if m["attrs"]["type"] == "sub" => out.push(Mark::Sub),
                "subsup" => out.push(Mark::Sup),
                "code" => code = true,
                k if PRESENTATIONAL_MARKS.contains(&k) => {}
                k => self.lose(
                    format!("mark:{k}"),
                    at,
                    format!("Markdown has no {k}; the text is shown without it"),
                ),
            }
        }
        // Links outermost so `[**a**](u)` rather than `**[a](u)**` splits.
        out.sort_by_key(|m| match m {
            Mark::Link(..) => 0,
            Mark::Strong => 1,
            Mark::Em => 2,
            Mark::Strike => 3,
            Mark::Underline => 4,
            Mark::Sub | Mark::Sup => 5,
        });
        (out, code)
    }

    fn pieces(&mut self, nodes: &[Value], base: &str, table: bool) -> Vec<(Vec<Mark>, Piece)> {
        let mut marks_all = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            marks_all.push(self.marks_of(n, &format!("{base}/{i}")));
        }
        let mut out: Vec<(Vec<Mark>, Piece)> = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            let at = format!("{base}/{i}");
            let (marks, code) = marks_all[i].clone();
            let attr = |k: &str| n["attrs"][k].as_str().unwrap_or("");
            let piece = match kind(n) {
                "text" => {
                    let t = n["text"].as_str().unwrap_or("").to_string();
                    // A lone link whose text is its URL becomes an autolink.
                    if let [Mark::Link(h, title)] = marks.as_slice()
                        && title.is_empty()
                        && !code
                        && is_autolink(&t, h)
                    {
                        let same = |j: Option<usize>| {
                            j.and_then(|j| marks_all.get(j))
                                .is_some_and(|(ms, _)| ms.iter().any(|m| m == &marks[0]))
                        };
                        if !same(i.checked_sub(1)) && !same(Some(i + 1)) {
                            out.push((Vec::new(), Piece::Raw(format!("<{t}>"))));
                            continue;
                        }
                    }
                    if code { Piece::Code(t) } else { Piece::Text(t) }
                }
                "hardBreak" => Piece::Break,
                "mention" => {
                    let name = attr("text");
                    let name = name.strip_prefix('@').unwrap_or(name);
                    let id = attr("id");
                    if id.is_empty() {
                        self.lose("mention", &at, "a mention without an account id");
                    }
                    Piece::Raw(format!(
                        "@[{}]({})",
                        label_text(name, table),
                        href(&format!("accountid:{id}"))
                    ))
                }
                "emoji" => {
                    let short = attr("shortName");
                    match next_emoji(short) {
                        Some(("", s, "")) => Piece::Raw(s.to_string()),
                        _ => {
                            self.lose(
                                "emoji",
                                &at,
                                "this emoji has no :shortname: form; it is shown as text",
                            );
                            let t = attr("text");
                            Piece::Text(if t.is_empty() { short } else { t }.to_string())
                        }
                    }
                }
                "status" => {
                    let color = attr("color");
                    let color = if color.is_empty() { "neutral" } else { color };
                    if !STATUS_COLORS.contains(&color) {
                        self.lose(
                            "status",
                            &at,
                            format!("status colour '{color}' has no Markdown form"),
                        );
                    }
                    Piece::Raw(format!(
                        "[{}]({})",
                        label_text(attr("text"), table),
                        href(&format!("status:{color}"))
                    ))
                }
                "date" => {
                    let ts = attr("timestamp");
                    match ts.parse::<i64>() {
                        Ok(ms) => {
                            let day = iso_date(ms);
                            let dest = if ms.rem_euclid(86_400_000) == 0 {
                                day.clone()
                            } else {
                                ms.to_string()
                            };
                            Piece::Raw(format!("[{day}](date:{dest})"))
                        }
                        Err(_) => {
                            self.lose("date", &at, "the date has no valid timestamp");
                            Piece::Text(ts.to_string())
                        }
                    }
                }
                "inlineCard" => {
                    let url = attr("url");
                    if url.is_empty() {
                        self.lose(
                            "inlineCard",
                            &at,
                            "a smart link without a URL has no Markdown form",
                        );
                        Piece::Raw(marker("inlineCard", false))
                    } else {
                        Piece::Raw(format!(
                            "[{}]({} \"card\")",
                            label_text(url, table),
                            href(url)
                        ))
                    }
                }
                "placeholder" => {
                    self.lose(
                        "placeholder",
                        &at,
                        "placeholder text is shown as ordinary text",
                    );
                    Piece::Text(attr("text").to_string())
                }
                _ => {
                    let l = format!("{}{}", label(n), extension_hint(n));
                    self.lose(
                        label(n),
                        &at,
                        "Markdown has no form for this inline node; it is not shown",
                    );
                    Piece::Raw(marker(&l, false))
                }
            };
            out.push((marks, piece));
        }
        out
    }

    fn inline(&mut self, nodes: &[Value], base: &str, ctx: Ctx) -> String {
        let mut out = String::new();
        let mut stack: Vec<Mark> = Vec::new();
        let mut pending = String::new();
        let mut line_start = true;
        let close_to = |stack: &mut Vec<Mark>, keep: usize, out: &mut String| {
            while stack.len() > keep {
                out.push_str(&stack.pop().unwrap().close());
            }
        };
        for (marks, piece) in self.pieces(nodes, base, ctx.table) {
            if let Piece::Break = piece {
                pending.clear();
                out.push_str(if ctx.table { "<br>" } else { "\\\n" });
                line_start = true;
                continue;
            }
            let (lead, core, trail) = match &piece {
                Piece::Text(t) => {
                    let core = t.trim();
                    if core.is_empty() {
                        pending.push_str(t);
                        continue;
                    }
                    let start = t.len() - t.trim_start().len();
                    (&t[..start], core, &t[start + core.len()..])
                }
                Piece::Code(t) | Piece::Raw(t) => ("", t.as_str(), ""),
                Piece::Break => unreachable!(),
            };
            let common = stack.iter().zip(&marks).take_while(|(a, b)| a == b).count();
            if common < stack.len() {
                close_to(&mut stack, common, &mut out);
                line_start = false;
            }
            out.push_str(&std::mem::take(&mut pending));
            out.push_str(lead);
            if !lead.is_empty() {
                line_start = false;
            }
            for m in &marks[common..] {
                out.push_str(m.open());
                stack.push(m.clone());
                line_start = false;
            }
            let in_link = stack.iter().any(|m| matches!(m, Mark::Link(..)));
            match &piece {
                Piece::Text(_) if in_link => {
                    out.push_str(&escape_text(core, line_start, ctx.table).replace(']', "\\]"))
                }
                Piece::Text(_) => out.push_str(&escape_text(core, line_start, ctx.table)),
                Piece::Code(_) => out.push_str(&code_span(core, ctx.table)),
                _ => out.push_str(core),
            }
            line_start = false;
            pending.push_str(trail);
        }
        close_to(&mut stack, 0, &mut out);
        out
    }
}

fn media_name(m: &Value) -> String {
    let a = &m["attrs"];
    a["alt"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(a["id"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Indent every line after the first by `n` spaces (blank lines stay blank).
fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 || l.is_empty() {
                l.to_string()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn longest_run(s: &str, ch: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for c in s.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

fn code_span(s: &str, table: bool) -> String {
    let fence = "`".repeat(longest_run(s, '`') + 1);
    let pad = if s.starts_with('`')
        || s.ends_with('`')
        || (s.starts_with(' ') && s.ends_with(' ') && !s.trim().is_empty())
    {
        " "
    } else {
        ""
    };
    let s = if table {
        s.replace('|', "\\|")
    } else {
        s.to_string()
    };
    format!("{fence}{pad}{s}{pad}{fence}")
}

/// Link destinations: wrap in `<>` when they contain characters that would
/// end an inline destination early.
fn href(url: &str) -> String {
    if url.contains([' ', '(', ')']) || url.is_empty() {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
}

// ---------------------------------------------------------------------------
// Inline rendering with properly nested emphasis delimiters.

#[derive(Clone, PartialEq, Debug)]
enum Mark {
    /// href, title
    Link(String, String),
    Strong,
    Em,
    Underline,
    Strike,
    Sub,
    Sup,
}

impl Mark {
    fn open(&self) -> &'static str {
        match self {
            Mark::Link(..) => "[",
            Mark::Strong => "**",
            Mark::Em => "*",
            Mark::Underline => "<u>",
            Mark::Strike => "~~",
            Mark::Sub => "<sub>",
            Mark::Sup => "<sup>",
        }
    }

    fn close(&self) -> String {
        match self {
            Mark::Link(h, t) if t.is_empty() => format!("]({})", href(h)),
            Mark::Link(h, t) => format!(
                "]({} \"{}\")",
                href(h),
                t.replace('\\', "\\\\").replace('"', "\\\"")
            ),
            Mark::Strong => "**".into(),
            Mark::Em => "*".into(),
            Mark::Underline => "</u>".into(),
            Mark::Strike => "~~".into(),
            Mark::Sub => "</sub>".into(),
            Mark::Sup => "</sup>".into(),
        }
    }
}

enum Piece {
    Text(String),
    Code(String),
    Raw(String),
    Break,
}

fn is_autolink(text: &str, href: &str) -> bool {
    let plain = href.strip_prefix("mailto:").unwrap_or(href);
    text == plain
        && (href.contains("://") || href.starts_with("mailto:"))
        && !href.contains([' ', '<', '>'])
}

/// Escape text so it reads back as the same text (not as markup), while
/// leaving ordinary prose untouched.
fn escape_text(s: &str, line_start: bool, table: bool) -> String {
    // Colons that would start a `:shortname:` emoji code.
    let mut emoji_at = Vec::new();
    let mut from = 0;
    while let Some((before, code, _)) = next_emoji(&s[from..]) {
        emoji_at.push(from + before.len());
        from += before.len() + code.len();
    }
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let ws = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace());
    for (i, &(byte, c)) in chars.iter().enumerate() {
        let prev = if i == 0 {
            None
        } else {
            chars.get(i - 1).map(|(_, c)| c)
        };
        let next = chars.get(i + 1).map(|(_, c)| c);
        let esc = match c {
            '\\' => next.is_none_or(|n| n.is_ascii_punctuation()),
            '`' | '[' => true,
            ']' => false,
            // Delimiter runs only act when they touch non-whitespace; edges
            // of a text node may touch other markup, so treat them as such.
            '*' | '~' => !(prev.is_some() && next.is_some() && ws(prev) && ws(next)),
            '_' => {
                let alnum = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
                !(alnum(prev) && alnum(next))
                    && !(prev.is_some() && next.is_some() && ws(prev) && ws(next))
            }
            '<' => next.is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?')),
            '&' => {
                let rest: String = chars[i + 1..].iter().map(|(_, c)| c).take(12).collect();
                rest.find(';').is_some_and(|e| {
                    e > 0
                        && rest[..e]
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '#')
                })
            }
            ':' => emoji_at.contains(&byte),
            '|' => table,
            _ => false,
        };
        if esc {
            out.push('\\');
        }
        out.push(c);
    }
    if line_start {
        out = escape_line_start(&out);
    }
    out
}

fn escape_line_start(s: &str) -> String {
    let first = s.chars().next();
    match first {
        Some('#' | '>' | '+' | '=' | '-') => format!("\\{s}"),
        Some(c) if c.is_ascii_digit() => {
            let digits = s.bytes().take_while(u8::is_ascii_digit).count();
            let rest = &s[digits..];
            if digits <= 9
                && (rest.starts_with(". ") || rest.starts_with(") ") || rest == "." || rest == ")")
            {
                format!("{}\\{}", &s[..digits], rest)
            } else {
                s.to_string()
            }
        }
        _ => s.to_string(),
    }
}

/// Escape the text of a link label (`[...]`), where `]` would end it.
fn label_text(s: &str, table: bool) -> String {
    escape_text(s, false, table).replace(']', "\\]")
}

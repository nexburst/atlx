//! ADF -> GitHub-flavoured Markdown, for reads.
//!
//! The output parses back to the same ADF through [`super::markdown_to_adf`]
//! for everything Markdown can express (see the golden tests).

use serde_json::Value;

use super::adf::{content, is_inline, kind};

/// ADF document (or a single node) -> Markdown.
pub fn adf_to_markdown(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ if kind(v) == "doc" => blocks(content(v), Ctx::default()),
        _ => blocks(std::slice::from_ref(v), Ctx::default()),
    }
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    table: bool,
    /// Use the alternate list markers (`*`, `1)`) so a list directly after
    /// another list of the same kind stays a separate list.
    alt: bool,
}

fn blocks(nodes: &[Value], ctx: Ctx) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut run: Vec<Value> = Vec::new();
    let flush = |run: &mut Vec<Value>, parts: &mut Vec<String>| {
        if !run.is_empty() {
            parts.push(inline(run, ctx));
            run.clear();
        }
    };
    let mut prev = "";
    let mut alt = false;
    for n in nodes {
        if is_inline(n) {
            run.push(n.clone());
            prev = "";
            continue;
        }
        flush(&mut run, &mut parts);
        alt = kind(n) == prev && !alt;
        prev = kind(n);
        parts.push(block(n, Ctx { alt, ..ctx }));
    }
    flush(&mut run, &mut parts);
    parts.retain(|p| !p.is_empty());
    parts.join(if ctx.table { "<br>" } else { "\n\n" })
}

fn block(n: &Value, ctx: Ctx) -> String {
    let attr = |k: &str| n["attrs"][k].as_str().unwrap_or("");
    match kind(n) {
        "paragraph" => inline(content(n), ctx),
        "heading" if ctx.table => format!("**{}**", inline(content(n), ctx)),
        "heading" => {
            let level = n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), inline(content(n), ctx))
                .trim_end()
                .to_string()
        }
        "bulletList" | "orderedList" | "taskList" | "decisionList" => {
            let s = list(n, ctx.alt);
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
                return code_span(&code.replace('\n', " "), true);
            }
            let fence = "`".repeat(longest_run(&code, '`').max(2) + 1);
            format!("{fence}{}\n{code}\n{fence}", attr("language"))
        }
        "blockquote" | "panel" => {
            let inner = blocks(content(n), ctx);
            if ctx.table {
                return inner;
            }
            inner
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".into()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "rule" => {
            if ctx.table {
                String::new()
            } else {
                "---".into()
            }
        }
        "table" => table(n),
        "mediaSingle" | "mediaGroup" => content(n).iter().map(media).collect::<Vec<_>>().join(" "),
        "media" => media(n),
        "expand" | "nestedExpand" => {
            let title = attr("title");
            let body = blocks(content(n), ctx);
            if title.is_empty() {
                body
            } else {
                format!("**{}**\n\n{body}", escape(title, true, ctx.table))
            }
        }
        "blockCard" | "embedCard" => format!("<{}>", attr("url")),
        "extension" | "bodiedExtension" if content(n).is_empty() => String::new(),
        _ => blocks(content(n), ctx),
    }
}

fn media(n: &Value) -> String {
    let a = &n["attrs"];
    let alt = a["alt"].as_str().unwrap_or("");
    match a["url"].as_str() {
        Some(url) => format!("![{}]({})", escape(alt, false, false), href(url)),
        None => {
            let id = a["id"].as_str().unwrap_or("");
            format!("[attachment: {}]", if alt.is_empty() { id } else { alt })
        }
    }
}

fn list(n: &Value, alt: bool) -> String {
    let bullet = if alt { "* " } else { "- " };
    let ordered = kind(n) == "orderedList";
    let mut num = n["attrs"]["order"].as_u64().unwrap_or(1);
    let mut out = String::new();
    for item in content(n) {
        let body;
        let marker;
        match kind(item) {
            "taskList" | "bulletList" | "orderedList" => {
                // Nested task list: indent under the previous item.
                if !out.is_empty() {
                    out.push_str("\n  ");
                }
                out.push_str(&indent(&list(item, false), 2));
                continue;
            }
            "taskItem" | "decisionItem" => {
                let check = if item["attrs"]["state"] == "DONE" {
                    "[x] "
                } else {
                    "[ ] "
                };
                marker = format!("{bullet}{check}");
                body = inline(content(item), Ctx::default());
            }
            _ => {
                marker = match (ordered, alt) {
                    (true, false) => format!("{num}. "),
                    (true, true) => format!("{num}) "),
                    _ => bullet.to_string(),
                };
                num += 1;
                body = item_body(item);
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

fn item_body(item: &Value) -> String {
    let mut s = String::new();
    for (i, c) in content(item).iter().enumerate() {
        let r = block(c, Ctx::default());
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

fn table(n: &Value) -> String {
    let ctx = Ctx {
        table: true,
        alt: false,
    };
    let rows: Vec<Vec<String>> = content(n)
        .iter()
        .map(|row| {
            content(row)
                .iter()
                .map(|cell| blocks(content(cell), ctx))
                .collect()
        })
        .collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
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
    for (i, r) in rows.iter().enumerate() {
        out.push(line(r));
        if i == 0 {
            out.push(format!("|{}", " --- |".repeat(cols)));
        }
    }
    out.join("\n")
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
    Link(String),
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
            Mark::Link(_) => "[",
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
            Mark::Link(h) => format!("]({})", href(h)),
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

fn marks_of(n: &Value) -> (Vec<Mark>, bool) {
    let mut out = Vec::new();
    let mut code = false;
    for m in n["marks"].as_array().into_iter().flatten() {
        match kind(m) {
            "link" => out.push(Mark::Link(
                m["attrs"]["href"].as_str().unwrap_or("").to_string(),
            )),
            "strong" => out.push(Mark::Strong),
            "em" => out.push(Mark::Em),
            "underline" => out.push(Mark::Underline),
            "strike" => out.push(Mark::Strike),
            "subsup" if m["attrs"]["type"] == "sub" => out.push(Mark::Sub),
            "subsup" => out.push(Mark::Sup),
            "code" => code = true,
            _ => {}
        }
    }
    // Links outermost so `[**a**](u)` rather than `**[a](u)**` splits.
    out.sort_by_key(|m| match m {
        Mark::Link(_) => 0,
        Mark::Strong => 1,
        Mark::Em => 2,
        Mark::Strike => 3,
        Mark::Underline => 4,
        Mark::Sub | Mark::Sup => 5,
    });
    (out, code)
}

fn is_autolink(text: &str, href: &str) -> bool {
    let plain = href.strip_prefix("mailto:").unwrap_or(href);
    text == plain
        && (href.contains("://") || href.starts_with("mailto:"))
        && !href.contains([' ', '<', '>'])
}

fn pieces(nodes: &[Value]) -> Vec<(Vec<Mark>, Piece)> {
    let mut out: Vec<(Vec<Mark>, Piece)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let (marks, code) = marks_of(n);
        let attr = |k: &str| n["attrs"][k].as_str().unwrap_or("");
        let piece = match kind(n) {
            "text" => {
                let t = n["text"].as_str().unwrap_or("").to_string();
                // A lone link whose text is its URL becomes an autolink.
                if let [Mark::Link(h)] = marks.as_slice()
                    && !code
                    && is_autolink(&t, h)
                {
                    let same = |j: Option<&Value>| {
                        j.is_some_and(|o| marks_of(o).0.iter().any(|m| m == &Mark::Link(h.clone())))
                    };
                    if !same(i.checked_sub(1).and_then(|j| nodes.get(j))) && !same(nodes.get(i + 1))
                    {
                        out.push((Vec::new(), Piece::Raw(format!("<{t}>"))));
                        continue;
                    }
                }
                if code { Piece::Code(t) } else { Piece::Text(t) }
            }
            "hardBreak" => Piece::Break,
            "mention" => {
                let t = attr("text");
                let t = if t.is_empty() { attr("id") } else { t };
                Piece::Raw(if t.starts_with('@') {
                    t.to_string()
                } else {
                    format!("@{t}")
                })
            }
            "emoji" => {
                let t = attr("text");
                Piece::Raw(if t.is_empty() { attr("shortName") } else { t }.to_string())
            }
            "status" => Piece::Text(format!("[{}]", attr("text"))),
            "date" => Piece::Text(date(attr("timestamp"))),
            "inlineCard" => Piece::Raw(format!("<{}>", attr("url"))),
            "mediaInline" | "media" => Piece::Raw(media(n)),
            "placeholder" => Piece::Text(attr("text").to_string()),
            _ => Piece::Text(n["text"].as_str().unwrap_or(attr("text")).to_string()),
        };
        out.push((marks, piece));
    }
    out
}

fn date(ts: &str) -> String {
    let Ok(ms) = ts.parse::<i64>() else {
        return ts.to_string();
    };
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn inline(nodes: &[Value], ctx: Ctx) -> String {
    let mut out = String::new();
    let mut stack: Vec<Mark> = Vec::new();
    let mut pending = String::new();
    let mut line_start = true;
    let close_to = |stack: &mut Vec<Mark>, keep: usize, out: &mut String| {
        while stack.len() > keep {
            out.push_str(&stack.pop().unwrap().close());
        }
    };
    for (marks, piece) in pieces(nodes) {
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
        match &piece {
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

/// Escape text so it reads back as the same text (not as markup), while
/// leaving ordinary prose untouched.
fn escape_text(s: &str, line_start: bool, table: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let ws = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace());
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i == 0 { None } else { chars.get(i - 1) };
        let next = chars.get(i + 1);
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
                let rest: String = chars[i + 1..].iter().take(12).collect();
                rest.find(';').is_some_and(|e| {
                    e > 0
                        && rest[..e]
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '#')
                })
            }
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

/// Escape free text (titles) without line context.
fn escape(s: &str, line_start: bool, table: bool) -> String {
    escape_text(s, line_start, table)
}

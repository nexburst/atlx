//! Markdown (CommonMark + GFM tables, strikethrough and task lists, plus the
//! atlx extensions listed in the module docs) -> ADF.
//!
//! Anything without an ADF mapping is recorded as a [`Loss`]; callers that
//! write refuse the body in that case.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};

use super::adf::{Builder, IMAGE, node};
use super::{Converted, Loss, html};

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Alert labels (`> [!NOTE]`) and the ADF panel types they stand for. The
/// first label for a type is the one the renderer writes.
pub(crate) const ALERTS: &[(&str, &str)] = &[
    ("NOTE", "info"),
    ("TIP", "tip"),
    ("IMPORTANT", "note"),
    ("WARNING", "warning"),
    ("CAUTION", "error"),
    ("SUCCESS", "success"),
    // Accepted on input only.
    ("INFO", "info"),
    ("ERROR", "error"),
];

pub(crate) const STATUS_COLORS: &[&str] = &["neutral", "purple", "blue", "red", "yellow", "green"];

/// Link titles that turn a link into a smart link card.
pub(crate) const CARDS: &[(&str, &str)] = &[
    ("card", "inlineCard"),
    ("card:block", "blockCard"),
    ("card:embed", "embedCard"),
];

/// Link destinations that stand for inline nodes rather than links.
pub(crate) const SCHEMES: &[&str] = &["accountid:", "status:", "date:"];

/// A link that becomes a node; its text is collected until the link ends.
struct Special {
    kind: &'static str,
    dest: String,
    text: String,
    at: String,
}

struct State<'s> {
    src: &'s str,
    b: Builder,
    in_head: bool,
    html_block: String,
    html_at: String,
    /// Alt text for images being collected: (url, alt).
    image: Option<(String, String)>,
    special: Option<Special>,
    /// A blockquote just opened: its first line may be an alert label.
    alert: AlertState,
}

#[derive(PartialEq)]
enum AlertState {
    None,
    /// Blockquote opened, waiting for its first paragraph.
    Quote,
    /// First paragraph opened, waiting for its first text.
    Para,
    /// Label consumed; skip events up to this source offset and one soft break.
    Skip(usize),
}

/// Markdown -> ADF document, with every construct that has no ADF mapping.
pub fn markdown_to_adf(s: &str) -> Converted {
    let s = s.replace("\r\n", "\n");
    let mut st = State {
        src: &s,
        b: Builder::new(),
        in_head: false,
        html_block: String::new(),
        html_at: String::new(),
        image: None,
        special: None,
        alert: AlertState::None,
    };
    for (ev, range) in Parser::new_ext(&s, options()).into_offset_iter() {
        st.event(ev, range);
    }
    let (adf, lost) = st.b.finish();
    Converted { adf, lost }
}

fn line_of(src: &str, offset: usize) -> String {
    let line = src[..offset.min(src.len())].matches('\n').count() + 1;
    format!("line {line}")
}

impl State<'_> {
    fn at(&self, range: &Range<usize>) -> String {
        line_of(self.src, range.start)
    }

    fn lose(&mut self, node: &str, range: &Range<usize>, detail: impl Into<String>) {
        let at = self.at(range);
        self.b.lose(Loss::new(node, at, detail));
    }

    fn event(&mut self, ev: Event, range: Range<usize>) {
        if self.alert_step(&ev, &range) {
            return;
        }
        if let Some((_, alt)) = self.image.as_mut() {
            match &ev {
                Event::Text(t) | Event::Code(t) => {
                    alt.push_str(t);
                    return;
                }
                Event::End(TagEnd::Image) => {}
                // Formatting inside alt text: the alt text keeps the words.
                Event::Start(_) | Event::End(_) => return,
                _ => {
                    self.lose("image", &range, "only text is kept in image alt text");
                    return;
                }
            }
        }
        if let Some(sp) = self.special.as_mut() {
            match &ev {
                Event::Text(t) | Event::Code(t) => {
                    sp.text.push_str(t);
                    return;
                }
                Event::End(TagEnd::Link) => {}
                _ => {
                    let k = sp.kind;
                    self.lose(
                        k,
                        &range,
                        format!("a {k} holds plain text; formatting inside it was dropped"),
                    );
                    return;
                }
            }
        }
        match ev {
            Event::Start(tag) => self.start(tag, &range),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t, &range),
            Event::Code(t) => {
                self.b.push_mark("code", json!({"type": "code"}));
                self.b.text(&t);
                self.b.pop_mark("code");
            }
            Event::Html(t) => self.html_block.push_str(&t),
            Event::InlineHtml(t) => self.inline_html(&t, &range),
            Event::SoftBreak => self.b.text(" "),
            Event::HardBreak => self.b.push(json!({"type": "hardBreak"})),
            Event::Rule => self.b.push(json!({"type": "rule"})),
            Event::TaskListMarker(done) => {
                if let Some(parent) = self.b.parent_of_mut("listItem") {
                    parent["type"] = "taskList".into();
                    if let Some(o) = parent.get_mut("attrs").and_then(Value::as_object_mut) {
                        o.remove("order");
                    }
                }
                if let Some(item) = self.b.open_node_mut("listItem") {
                    item["type"] = "taskItem".into();
                    item["attrs"] = json!({"state": if done { "DONE" } else { "TODO" }});
                }
            }
            // Math and footnotes are not enabled in `options()`, so these do
            // not occur; if they ever do, the text is kept and reported.
            Event::InlineMath(t) | Event::DisplayMath(t) | Event::FootnoteReference(t) => {
                self.lose("math", &range, "math and footnotes are kept as plain text");
                self.b.text(&t)
            }
        }
    }

    /// Recognize `> [!NOTE]` as the first line of a blockquote. Returns true
    /// when the event was consumed.
    fn alert_step(&mut self, ev: &Event, range: &Range<usize>) -> bool {
        match (&self.alert, ev) {
            (AlertState::None, _) => false,
            (AlertState::Quote, Event::Start(Tag::Paragraph)) => {
                self.alert = AlertState::Para;
                false
            }
            (AlertState::Para, Event::Text(_)) => {
                self.alert = AlertState::None;
                let rest = &self.src[range.start..];
                let Some((panel, len)) = alert_label(rest) else {
                    return false;
                };
                if let Some(q) = self.b.open_node_mut("blockquote") {
                    q["type"] = "panel".into();
                    q["attrs"] = json!({"panelType": panel});
                }
                self.alert = AlertState::Skip(range.start + len);
                self.alert_skip(ev, range)
            }
            (AlertState::Skip(_), _) => self.alert_skip(ev, range),
            _ => {
                self.alert = AlertState::None;
                false
            }
        }
    }

    fn alert_skip(&mut self, ev: &Event, range: &Range<usize>) -> bool {
        let AlertState::Skip(end) = self.alert else {
            return false;
        };
        match ev {
            Event::Text(_) if range.end <= end => true,
            Event::SoftBreak | Event::HardBreak => {
                self.alert = AlertState::None;
                true
            }
            _ => {
                self.alert = AlertState::None;
                false
            }
        }
    }

    fn start(&mut self, tag: Tag, range: &Range<usize>) {
        let b = &mut self.b;
        match tag {
            Tag::Paragraph => b.open("p", node("paragraph", Map::new())),
            Tag::Heading { level, .. } => {
                let mut n = node("heading", Map::new());
                n["attrs"] = json!({"level": level as usize});
                b.open("h", n);
            }
            Tag::BlockQuote(_) => {
                b.open("blockquote", node("blockquote", Map::new()));
                self.alert = AlertState::Quote;
            }
            Tag::CodeBlock(k) => {
                let mut n = node("codeBlock", Map::new());
                if let CodeBlockKind::Fenced(info) = k {
                    let mut words = info.split_whitespace();
                    if let Some(lang) = words.next() {
                        n["attrs"] = json!({"language": lang});
                    }
                    if words.next().is_some() {
                        self.lose(
                            "codeBlock",
                            range,
                            "only the first word of a code fence's info string (the language) is kept",
                        );
                    }
                }
                self.b.open("pre", n);
            }
            Tag::HtmlBlock => {
                self.html_block.clear();
                self.html_at = self.at(range);
            }
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
            Tag::Table(align) => {
                b.open("table", node("table", Map::new()));
                if align.iter().any(|a| *a != pulldown_cmark::Alignment::None) {
                    self.lose("table", range, "column alignment is not kept");
                }
            }
            Tag::TableHead => {
                self.in_head = true;
                b.open("tr", node("tableRow", Map::new()));
            }
            Tag::TableRow => b.open("tr", node("tableRow", Map::new())),
            Tag::TableCell => {
                let ty = if self.in_head {
                    "tableHeader"
                } else {
                    "tableCell"
                };
                b.open("td", node(ty, Map::new()))
            }
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
                title,
                ..
            } => self.link_start(link_type, &dest_url, &title, range),
            Tag::Image {
                dest_url, title, ..
            } => {
                if !title.is_empty() {
                    self.lose("image", range, "image titles are not kept");
                }
                self.image = Some((dest_url.to_string(), String::new()));
            }
            // Not enabled in `options()`; reported if they ever appear.
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => self.lose(
                "markdown",
                range,
                "this Markdown extension is not supported",
            ),
        }
    }

    fn link_start(&mut self, link_type: LinkType, dest: &str, title: &str, range: &Range<usize>) {
        let card = CARDS.iter().find(|(t, _)| *t == title).map(|(_, k)| *k);
        let scheme = SCHEMES.iter().find(|s| dest.starts_with(**s));
        let kind = match (card, scheme) {
            (Some(k), _) => Some(k),
            (None, Some(&"accountid:")) => Some("mention"),
            (None, Some(&"status:")) => Some("status"),
            (None, Some(&"date:")) => Some("date"),
            _ => None,
        };
        if let Some(kind) = kind {
            if self.b.has_marks() {
                self.lose(kind, range, format!("a {kind} cannot carry formatting"));
            }
            self.special = Some(Special {
                kind,
                dest: dest.to_string(),
                text: String::new(),
                at: self.at(range),
            });
            return;
        }
        let href = if link_type == LinkType::Email && !dest.starts_with("mailto:") {
            format!("mailto:{dest}")
        } else {
            dest.to_string()
        };
        let mut mark = json!({"type": "link", "attrs": {"href": href}});
        if !title.is_empty() {
            mark["attrs"]["title"] = title.into();
        }
        self.b.push_mark("a", mark);
    }

    fn special_end(&mut self, sp: Special) {
        let lose = |b: &mut Builder, detail: String| b.lose(Loss::new(sp.kind, &sp.at, detail));
        let n = match sp.kind {
            "mention" => {
                let id = &sp.dest["accountid:".len()..];
                // `@[Name](accountid:…)`: the `@` before the link belongs to it.
                if let Some(last) = self.b.last_mut()
                    && last["type"] == "text"
                    && last.get("marks").is_none()
                    && let Some(t) = last["text"].as_str()
                    && let Some(rest) = t.strip_suffix('@')
                {
                    last["text"] = rest.into();
                }
                let mut a = json!({"id": id});
                if !sp.text.is_empty() {
                    a["text"] = format!("@{}", sp.text).into();
                }
                json!({"type": "mention", "attrs": a})
            }
            "status" => {
                let color = sp.dest["status:".len()..].to_ascii_lowercase();
                if !STATUS_COLORS.contains(&color.as_str()) {
                    lose(
                        &mut self.b,
                        format!(
                            "unknown status colour '{color}' ({}); neutral was used",
                            STATUS_COLORS.join(", ")
                        ),
                    );
                }
                let color = if STATUS_COLORS.contains(&color.as_str()) {
                    color
                } else {
                    "neutral".into()
                };
                json!({"type": "status", "attrs": {"text": sp.text, "color": color}})
            }
            "date" => {
                let v = &sp.dest["date:".len()..];
                match parse_date(v) {
                    Some(ms) => json!({"type": "date", "attrs": {"timestamp": ms.to_string()}}),
                    None => {
                        lose(
                            &mut self.b,
                            format!(
                                "'{v}' is not a date (YYYY-MM-DD or epoch milliseconds); kept as text"
                            ),
                        );
                        self.b.text(&sp.text);
                        return;
                    }
                }
            }
            card => {
                if sp.text != sp.dest {
                    lose(
                        &mut self.b,
                        "a smart link shows its URL; the link text was dropped".into(),
                    );
                }
                json!({"type": card, "attrs": {"url": sp.dest}})
            }
        };
        self.b.push(n);
    }

    fn end(&mut self, tag: TagEnd) {
        let b = &mut self.b;
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Table
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::CodeBlock => b.close(),
            TagEnd::TableHead => {
                self.in_head = false;
                b.close();
                // A header row of empty cells stands for a table without one.
                if b.top_kind() == "table"
                    && let Some(row) = b.pop_last()
                {
                    let empty = row["content"]
                        .as_array()
                        .is_some_and(|cells| cells.iter().all(|c| c["content"] == json!([])));
                    if !empty {
                        b.push(row);
                    }
                }
            }
            TagEnd::HtmlBlock => {
                let html = std::mem::take(&mut self.html_block);
                self.html_block_end(&html);
            }
            TagEnd::Emphasis => b.pop_mark("em"),
            TagEnd::Strong => b.pop_mark("strong"),
            TagEnd::Strikethrough => b.pop_mark("s"),
            TagEnd::Superscript => b.pop_mark("sup"),
            TagEnd::Subscript => b.pop_mark("sub"),
            TagEnd::Link => {
                if let Some(sp) = self.special.take() {
                    self.special_end(sp);
                } else {
                    b.pop_mark("a");
                }
            }
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    b.push(json!({"type": IMAGE, "attrs": {"url": url, "alt": alt}}));
                }
            }
            // Their starts were reported as losses.
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    /// Plain text, with `:shortname:` emoji split out.
    fn text(&mut self, t: &str, range: &Range<usize>) {
        // Only text written literally (no escapes or entities) can hold an
        // emoji code, so `\:smile:` stays text.
        let literal = self.src.get(range.clone()) == Some(t);
        if !literal || self.b.has_marks() || !t.contains(':') {
            self.b.text(t);
            return;
        }
        // An escaped character (`\:`) starts a new text event right after
        // the backslash.
        let backslashes = self.src[..range.start]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count();
        let mut rest = t;
        if backslashes % 2 == 1 {
            let first = rest.chars().next().map_or(0, char::len_utf8);
            self.b.text(&rest[..first]);
            rest = &rest[first..];
        }
        while let Some((before, short, after)) = next_emoji(rest) {
            self.b.text(before);
            self.b
                .push(json!({"type": "emoji", "attrs": {"shortName": short}}));
            rest = after;
        }
        self.b.text(rest);
    }

    fn html_block_end(&mut self, html: &str) {
        let at = std::mem::take(&mut self.html_at);
        let t = html.trim();
        match details_tag(t) {
            Some(Details::Open(title)) => {
                let ty = if self.b.is_open("details") {
                    "nestedExpand"
                } else {
                    "expand"
                };
                let mut n = node(ty, Map::new());
                if let Some(title) = title {
                    n["attrs"] = json!({"title": title});
                }
                self.b.open("details", n);
            }
            Some(Details::Close) if self.b.is_open("details") => self.b.close_tag("details"),
            _ => {
                let detail = if let Some(marker) = atlx_marker(t) {
                    format!(
                        "this marks content atlx could not show ({marker}); writing it back would delete that content"
                    )
                } else {
                    "raw HTML has no ADF mapping".to_string()
                };
                self.b.lose(Loss::new("html", at, detail));
                // Best effort for the lossy compatibility path (`to_adf`).
                for n in html::html_blocks(html) {
                    self.b.push(n);
                }
            }
        }
    }

    /// The small set of inline HTML tags Markdown writers use for formatting
    /// Markdown lacks (underline, sub/superscript, line breaks). Anything
    /// else stays literal text and is reported.
    fn inline_html(&mut self, raw: &str, range: &Range<usize>) {
        let t = raw.trim();
        let parsed = t
            .strip_prefix('<')
            .and_then(|t| t.strip_suffix('>'))
            .filter(|i| !i.starts_with('!'));
        let Some(inner) = parsed else {
            let detail = match atlx_marker(t) {
                Some(m) => format!(
                    "this marks content atlx could not show ({m}); writing it back would delete that content"
                ),
                None => "inline HTML has no ADF mapping; it was kept as text".into(),
            };
            self.lose("html", range, detail);
            self.b.text(raw);
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
                self.b.push(json!({"type": "hardBreak"}));
                return;
            }
            "u" | "ins" => json!({"type": "underline"}),
            "s" | "del" | "strike" => json!({"type": "strike"}),
            "b" | "strong" => json!({"type": "strong"}),
            "i" | "em" => json!({"type": "em"}),
            "sub" => json!({"type": "subsup", "attrs": {"type": "sub"}}),
            "sup" => json!({"type": "subsup", "attrs": {"type": "sup"}}),
            _ => {
                self.lose(
                    "html",
                    range,
                    format!("inline HTML <{name}> has no ADF mapping; it was kept as text"),
                );
                self.b.text(raw);
                return;
            }
        };
        let tag = format!("html:{name}");
        if closing {
            self.b.pop_mark(&tag);
        } else {
            self.b.push_mark(&tag, mark);
        }
    }
}

/// `[!LABEL]` at the start of `s`, alone on its line: (panel type, length
/// up to the end of the line, excluding the newline).
fn alert_label(s: &str) -> Option<(&'static str, usize)> {
    let inner = s.strip_prefix("[!")?;
    let close = inner.find(']')?;
    let label = &inner[..close];
    let (_, panel) = ALERTS.iter().find(|(l, _)| l.eq_ignore_ascii_case(label))?;
    let after = &inner[close + 1..];
    let line_end = after.find('\n').unwrap_or(after.len());
    if !after[..line_end].trim().is_empty() {
        return None;
    }
    Some((panel, 2 + close + 1 + line_end))
}

enum Details {
    Open(Option<String>),
    Close,
}

/// `<details>` (optionally followed by `<summary>Title</summary>`) or
/// `</details>`, alone in an HTML block.
fn details_tag(t: &str) -> Option<Details> {
    if t.eq_ignore_ascii_case("</details>") {
        return Some(Details::Close);
    }
    let rest = t.strip_prefix("<details>")?.trim_start();
    if rest.is_empty() {
        return Some(Details::Open(None));
    }
    let inner = rest.strip_prefix("<summary>")?.strip_suffix("</summary>")?;
    if inner.contains('<') {
        return None;
    }
    Some(Details::Open(Some(html::decode(inner.trim()))))
}

/// The text of an `<!-- atlx: ... -->` marker, if `t` is one.
fn atlx_marker(t: &str) -> Option<&str> {
    t.strip_prefix("<!--")?
        .strip_suffix("-->")?
        .trim()
        .strip_prefix("atlx:")
        .map(str::trim)
}

fn is_short_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-')
}

/// The next `:shortname:` in `s` that stands alone (not inside a word or a
/// time like `10:30:00`): (text before, `:shortname:`, text after).
pub(crate) fn next_emoji(s: &str) -> Option<(&str, &str, &str)> {
    let mut from = 0;
    while let Some(i) = s[from..].find(':').map(|i| i + from) {
        let name_len = s[i + 1..]
            .find(|c: char| !is_short_char(c))
            .unwrap_or(s.len() - i - 1);
        let end = i + 1 + name_len;
        let name = &s[i + 1..end];
        let before_ok = s[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != ':');
        let closes = s[end..].starts_with(':');
        let after_ok = closes
            && s[end + 1..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != ':');
        if before_ok && after_ok && !name.is_empty() && !name.bytes().all(|b| b.is_ascii_digit()) {
            return Some((&s[..i], &s[i..end + 1], &s[end + 1..]));
        }
        from = i + 1;
    }
    None
}

/// `YYYY-MM-DD` (UTC midnight) or epoch milliseconds -> epoch milliseconds.
pub(crate) fn parse_date(v: &str) -> Option<i64> {
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse().ok();
    }
    let mut it = v.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if v.len() != 10 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (proleptic Gregorian).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000)
}

/// Epoch milliseconds -> `YYYY-MM-DD` (UTC).
pub(crate) fn iso_date(ms: i64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_codes() {
        assert_eq!(next_emoji("hi :smile: x"), Some(("hi ", ":smile:", " x")));
        assert_eq!(next_emoji("10:30:00"), None);
        assert_eq!(next_emoji("a:b:c"), None);
        assert_eq!(next_emoji(":+1: :x:"), Some(("", ":+1:", " :x:")));
        assert_eq!(next_emoji("::"), None);
        assert_eq!(next_emoji("(:tada:)"), Some(("(", ":tada:", ")")));
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("2026-01-01"), Some(1_767_225_600_000));
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("1767225600000"), Some(1_767_225_600_000));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("soon"), None);
        for d in ["2024-02-29", "1999-12-31", "2026-11-22"] {
            assert_eq!(iso_date(parse_date(d).unwrap()), d);
        }
    }

    #[test]
    fn alert_labels() {
        assert_eq!(alert_label("[!NOTE]\nbody"), Some(("info", 7)));
        assert_eq!(alert_label("[!tip]  "), Some(("tip", 8)));
        assert_eq!(alert_label("[!NOTE] inline"), None);
        assert_eq!(alert_label("[!OTHER]"), None);
    }
}

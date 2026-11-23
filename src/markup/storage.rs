//! Local checks for Confluence storage format input.
//!
//! Storage is passed through unchanged, never converted, so the only thing
//! atlx can do is make sure Confluence will not drop part of it: the input
//! must be well-formed XML (balanced, correctly nested tags, quoted
//! attributes, valid entities) and use only elements of the storage format
//! vocabulary (XHTML plus the `ac:` and `ri:` elements Atlassian documents).
//! Anything else is refused before a request is made.

use super::{BodyError, Loss};

/// XHTML elements accepted in storage format.
const HTML: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "hr",
    "strong",
    "b",
    "em",
    "i",
    "u",
    "s",
    "del",
    "ins",
    "strike",
    "sub",
    "sup",
    "code",
    "pre",
    "blockquote",
    "cite",
    "small",
    "big",
    "span",
    "div",
    "a",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "colgroup",
    "col",
    "time",
];

/// Confluence-specific elements (`ac:` structure, `ri:` resource identifiers).
const ATLASSIAN: &[&str] = &[
    "ac:structured-macro",
    "ac:parameter",
    "ac:rich-text-body",
    "ac:plain-text-body",
    "ac:link",
    "ac:link-body",
    "ac:plain-text-link-body",
    "ac:image",
    "ac:caption",
    "ac:emoticon",
    "ac:task-list",
    "ac:task",
    "ac:task-id",
    "ac:task-uuid",
    "ac:task-status",
    "ac:task-body",
    "ac:layout",
    "ac:layout-section",
    "ac:layout-cell",
    "ac:placeholder",
    "ac:inline-comment-marker",
    "ac:adf-extension",
    "ac:adf-node",
    "ac:adf-attribute",
    "ac:adf-content",
    "ac:adf-fallback",
    "ac:adf-mark",
    "ac:adf-leaf",
    "ri:page",
    "ri:blog-post",
    "ri:attachment",
    "ri:url",
    "ri:user",
    "ri:space",
    "ri:content-entity",
    "ri:shortcut",
];

const HINT: &str = "storage format must be well-formed XHTML using storage elements only (e.g. <br/>, not <br>); or pass the body as Markdown or ADF";

/// Refuse `s` unless it is well-formed storage that uses only storage
/// elements. Returns every problem found, not just the first.
pub fn check(s: &str) -> Result<(), BodyError> {
    let mut problems = Vec::new();
    if let Err(l) = well_formed(s) {
        problems.push(l);
    }
    for (name, line) in unknown_elements(s) {
        if !problems.iter().any(|p| p.node == format!("<{name}>")) {
            problems.push(Loss::new(
                format!("<{name}>"),
                format!("line {line}"),
                "not a storage format element; Confluence would drop it",
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(BodyError::refused(
            "the body is not valid Confluence storage format",
            problems,
            HINT,
        ))
    }
}

fn line_at(s: &str, i: usize) -> usize {
    s[..i.min(s.len())].matches('\n').count() + 1
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')
}

/// Elements outside the storage vocabulary, with the line of their first use.
pub fn unknown_elements(s: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut rest = s;
    let mut offset = 0;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i + 1..];
        let pos = offset + i;
        // Skip comments and CDATA whole so their text is not read as tags.
        let skip = if tail.starts_with("!--") {
            tail.find("-->").map(|e| e + 3)
        } else if tail.starts_with("![CDATA[") {
            tail.find("]]>").map(|e| e + 3)
        } else {
            None
        };
        if let Some(e) = skip {
            offset = pos + 1 + e;
            rest = &s[offset..];
            continue;
        }
        let name: String = tail
            .trim_start_matches('/')
            .chars()
            .take_while(|c| name_char(*c))
            .collect();
        if !name.is_empty()
            && !HTML.contains(&name.as_str())
            && !ATLASSIAN.contains(&name.as_str())
            && !out.iter().any(|(n, _)| *n == name)
        {
            out.push((name, line_at(s, pos)));
        }
        offset = pos + 1;
        rest = &s[offset..];
    }
    out
}

/// Check XML well-formedness: every tag closed in order, attributes quoted
/// and unique, `&` only in entities, `<` only in markup.
pub fn well_formed(s: &str) -> Result<(), Loss> {
    let err = |i: usize, what: &str, detail: String| {
        Err(Loss::new(what, format!("line {}", line_at(s, i)), detail))
    };
    let mut stack: Vec<(&str, usize)> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'&' => {
                let rest = &s[i + 1..];
                let end = rest.find(';').filter(|&e| e <= 32);
                let ok = end.is_some_and(|e| valid_entity(&rest[..e]));
                if !ok {
                    return err(i, "&", "a bare '&' must be written as &amp;".into());
                }
                i += 1 + end.unwrap() + 1;
            }
            b'<' => {
                let rest = &s[i..];
                if let Some(r) = rest.strip_prefix("<!--") {
                    let Some(e) = r.find("-->") else {
                        return err(i, "comment", "an unclosed <!-- comment".into());
                    };
                    i += 4 + e + 3;
                } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
                    let Some(e) = r.find("]]>") else {
                        return err(i, "CDATA", "an unclosed CDATA section".into());
                    };
                    i += 9 + e + 3;
                } else if rest.starts_with("<!") || rest.starts_with("<?") {
                    return err(
                        i,
                        "declaration",
                        "doctype and processing instructions are not storage format".into(),
                    );
                } else if let Some(r) = rest.strip_prefix("</") {
                    let name_len = r.find(|c: char| !name_char(c)).unwrap_or(r.len());
                    let name = &r[..name_len];
                    let after = r[name_len..].trim_start();
                    if name.is_empty() || !after.starts_with('>') {
                        return err(i, "tag", "a malformed closing tag".into());
                    }
                    match stack.pop() {
                        Some((open, _)) if open == name => {}
                        Some((open, at)) => {
                            return err(
                                i,
                                &format!("<{open}>"),
                                format!(
                                    "</{name}> closes <{open}> opened on line {}; tags must be closed in order",
                                    line_at(s, at)
                                ),
                            );
                        }
                        None => {
                            return err(
                                i,
                                &format!("</{name}>"),
                                format!("</{name}> has no matching opening tag"),
                            );
                        }
                    }
                    let ws = r[name_len..].len() - after.len();
                    i += 2 + name_len + ws + 1;
                } else {
                    let (name, len, self_closing) = match start_tag(rest) {
                        Ok(t) => t,
                        Err(detail) => return err(i, "tag", detail),
                    };
                    if !self_closing {
                        stack.push((name, i));
                    }
                    i += len;
                }
            }
            _ => {
                // Advance one UTF-8 character.
                i += s[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    if let Some((open, at)) = stack.pop() {
        return err(
            at,
            &format!("<{open}>"),
            format!("<{open}> is never closed (void elements are written <{open}/>)"),
        );
    }
    Ok(())
}

fn valid_entity(e: &str) -> bool {
    if let Some(n) = e.strip_prefix('#') {
        return match n.strip_prefix(['x', 'X']) {
            Some(h) => !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()),
            None => !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()),
        };
    }
    // Named entities: XML's five plus the HTML set storage format accepts.
    !e.is_empty()
        && e.starts_with(|c: char| c.is_ascii_alphabetic())
        && e.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Parse a start tag at the beginning of `s` (which starts with `<`):
/// (name, byte length, self-closing).
fn start_tag(s: &str) -> Result<(&str, usize, bool), String> {
    let r = &s[1..];
    let name_len = r.find(|c: char| !name_char(c)).unwrap_or(r.len());
    if name_len == 0 || !r.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err("a '<' that does not start a tag must be written as &lt;".into());
    }
    let name = &r[..name_len];
    let mut i = 1 + name_len;
    let mut seen: Vec<&str> = Vec::new();
    loop {
        let rest = &s[i..];
        let trimmed = rest.trim_start();
        let ws = rest.len() - trimmed.len();
        i += ws;
        if trimmed.starts_with("/>") {
            return Ok((name, i + 2, true));
        }
        if trimmed.starts_with('>') {
            return Ok((name, i + 1, false));
        }
        if trimmed.is_empty() {
            return Err(format!("<{name}> is not closed with '>'"));
        }
        if ws == 0 {
            return Err(format!("<{name}>: attributes must be separated by spaces"));
        }
        let an_len = trimmed
            .find(|c: char| !name_char(c))
            .unwrap_or(trimmed.len());
        if an_len == 0 {
            return Err(format!("<{name}>: a malformed attribute"));
        }
        let an = &trimmed[..an_len];
        if seen.contains(&an) {
            return Err(format!("<{name}>: attribute '{an}' is repeated"));
        }
        seen.push(an);
        let after = trimmed[an_len..].trim_start();
        let Some(v) = after.strip_prefix('=') else {
            return Err(format!("<{name}>: attribute '{an}' needs a quoted value"));
        };
        let v = v.trim_start();
        let Some(q) = v.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
            return Err(format!("<{name}>: the value of '{an}' must be quoted"));
        };
        let Some(end) = v[1..].find(q) else {
            return Err(format!("<{name}>: the value of '{an}' is not closed"));
        };
        let value = &v[1..1 + end];
        if value.contains('<') {
            return Err(format!(
                "<{name}>: '<' in the value of '{an}' must be written as &lt;"
            ));
        }
        let mut rest_v = value;
        while let Some(a) = rest_v.find('&') {
            let tail = &rest_v[a + 1..];
            match tail.find(';') {
                Some(e) if valid_entity(&tail[..e]) => rest_v = &tail[e + 1..],
                _ => return Err(format!("<{name}>: a bare '&' in the value of '{an}'")),
            }
        }
        // Advance past: name, spaces, '=', spaces, quoted value.
        let consumed = trimmed.len() - v.len() + 1 + end + 1;
        i += consumed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_storage() {
        let ok = [
            "<p>a &amp; b &nbsp; &#160; &#xA0;</p>",
            "<p>x<br/>y</p><hr />",
            "<ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[a < b && c]]></ac:plain-text-body></ac:structured-macro>",
            "<p><ac:link><ri:user ri:account-id='1:2'/></ac:link></p><!-- note <x> -->",
            "",
            "plain text > fine",
        ];
        for s in ok {
            assert!(check(s).is_ok(), "{s}: {:?}", check(s));
        }
    }

    #[test]
    fn refuses_broken_or_foreign_markup() {
        let bad = [
            ("<p>unclosed", "<p>"),
            ("<p>a<br>b</p>", "<br>"),
            ("<b><i>x</b></i>", "<i>"),
            ("</p>", "</p>"),
            ("<p>a & b</p>", "&"),
            ("<p>1 < 2</p>", "tag"),
            ("<p class=x>a</p>", "tag"),
            ("<p a=\"1\" a=\"2\">a</p>", "tag"),
            ("<custom-panel>x</custom-panel>", "<custom-panel>"),
            ("<details><summary>x</summary></details>", "<details>"),
            ("<img src=\"x.png\"/>", "<img>"),
            ("<!DOCTYPE html><p>x</p>", "declaration"),
        ];
        for (s, node) in bad {
            let e = check(s).unwrap_err();
            assert!(e.is_refused());
            assert!(
                e.losses.iter().any(|l| l.node == node),
                "{s}: expected {node} in {:?}",
                e.losses
            );
        }
    }
}

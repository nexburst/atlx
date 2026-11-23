//! Building ADF documents and normalizing them into schema-valid shape.
//!
//! Parsers (Markdown and HTML) push loosely nested nodes into a [`Builder`];
//! [`normalize`] then repairs the tree so it satisfies the ADF schema rules
//! Jira enforces: inline runs are wrapped in paragraphs, list items start with
//! a paragraph, table cells hold blocks, `code` marks only combine with
//! links, and there are no empty text nodes.

use serde_json::{Map, Value, json};

use super::Loss;

/// Internal placeholder for an image; becomes `mediaSingle` or a link.
pub(crate) const IMAGE: &str = "__image";
/// Internal placeholder for a block boundary from a transparent HTML block.
pub(crate) const BREAK: &str = "__break";

const INLINE: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "inlineCard",
    "date",
    "status",
    "placeholder",
    "inlineExtension",
    "mediaInline",
    IMAGE,
    BREAK,
];

const MARK_ORDER: &[&str] = &[
    "link",
    "strong",
    "em",
    "underline",
    "strike",
    "subsup",
    "textColor",
    "code",
];

pub(crate) fn kind(v: &Value) -> &str {
    v["type"].as_str().unwrap_or("")
}

pub(crate) fn is_inline(v: &Value) -> bool {
    INLINE.contains(&kind(v))
}

fn is_list(v: &Value) -> bool {
    matches!(kind(v), "bulletList" | "orderedList" | "taskList")
}

pub(crate) fn content(v: &Value) -> &[Value] {
    v["content"].as_array().map_or(&[], Vec::as_slice)
}

fn take_content(v: &mut Value) -> Vec<Value> {
    match v.get_mut("content").map(Value::take) {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

/// Accumulates nodes from an event stream. Each open node carries a tag (the
/// source element name) so tolerant parsers can close by tag. Parsers record
/// what they could not map with [`Builder::lose`]; normalization adds what it
/// had to drop or flatten.
pub(crate) struct Builder {
    stack: Vec<(String, Value)>,
    marks: Vec<(String, Value)>,
    pub losses: Vec<Loss>,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            stack: vec![(
                String::new(),
                json!({"type": "doc", "version": 1, "content": []}),
            )],
            marks: Vec::new(),
            losses: Vec::new(),
        }
    }

    pub fn lose(&mut self, loss: Loss) {
        self.losses.push(loss);
    }

    pub fn open(&mut self, tag: &str, mut node: Value) {
        if node.get("content").is_none() {
            node["content"] = json!([]);
        }
        self.stack.push((tag.to_string(), node));
    }

    /// Close the innermost open node.
    pub fn close(&mut self) {
        if self.stack.len() > 1 {
            let (_, node) = self.stack.pop().unwrap();
            self.push(node);
        }
    }

    fn position(&self, tag: &str) -> Option<usize> {
        self.stack
            .iter()
            .rposition(|(t, _)| t == tag)
            .filter(|&i| i > 0)
    }

    /// Whether a node opened with `tag` is still open.
    pub fn is_open(&self, tag: &str) -> bool {
        self.position(tag).is_some()
    }

    /// Close everything up to and including the innermost node opened with
    /// `tag`. Does nothing when no such node is open.
    pub fn close_tag(&mut self, tag: &str) {
        if let Some(i) = self.position(tag) {
            while self.stack.len() > i {
                self.close();
            }
        }
    }

    /// The tag of the innermost open node among `tags`, if any.
    pub fn nearest(&self, tags: &[&str]) -> Option<&str> {
        self.stack
            .iter()
            .skip(1)
            .rev()
            .find(|(t, _)| tags.contains(&t.as_str()))
            .map(|(t, _)| t.as_str())
    }

    pub fn top_tag(&self) -> &str {
        &self.stack.last().unwrap().0
    }

    pub fn top_kind(&self) -> &str {
        kind(&self.stack.last().unwrap().1)
    }

    /// Mutable access to the innermost open node of the given ADF type.
    pub fn open_node_mut(&mut self, ty: &str) -> Option<&mut Value> {
        self.stack
            .iter_mut()
            .skip(1)
            .rev()
            .map(|(_, v)| v)
            .find(|v| kind(v) == ty)
    }

    /// Mutable access to the innermost open node.
    pub fn top_mut(&mut self) -> &mut Value {
        &mut self.stack.last_mut().unwrap().1
    }

    /// Mutable access to the parent of the innermost open node of type `ty`.
    pub fn parent_of_mut(&mut self, ty: &str) -> Option<&mut Value> {
        let i = self.stack.iter().rposition(|(_, v)| kind(v) == ty)?;
        (i > 0).then(|| &mut self.stack[i - 1].1)
    }

    pub fn push(&mut self, node: Value) {
        let top = &mut self.stack.last_mut().unwrap().1;
        if let Some(a) = top["content"].as_array_mut() {
            a.push(node);
        }
    }

    /// Remove and return the last node pushed into the innermost open node.
    pub fn pop_last(&mut self) -> Option<Value> {
        self.stack.last_mut().unwrap().1["content"]
            .as_array_mut()
            .and_then(Vec::pop)
    }

    /// The last node pushed into the innermost open node.
    pub fn last_mut(&mut self) -> Option<&mut Value> {
        self.stack.last_mut().unwrap().1["content"]
            .as_array_mut()
            .and_then(|a| a.last_mut())
    }

    pub fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let mut node = json!({"type": "text", "text": s});
        if !self.marks.is_empty() {
            node["marks"] = Value::Array(self.marks.iter().map(|(_, m)| m.clone()).collect());
        }
        self.push(node);
    }

    pub fn has_marks(&self) -> bool {
        !self.marks.is_empty()
    }

    pub fn push_mark(&mut self, tag: &str, mark: Value) {
        self.marks.push((tag.to_string(), mark));
    }

    pub fn pop_mark(&mut self, tag: &str) {
        if let Some(i) = self.marks.iter().rposition(|(t, _)| t == tag) {
            self.marks.remove(i);
        }
    }

    pub fn finish(mut self) -> (Value, Vec<Loss>) {
        while self.stack.len() > 1 {
            self.close();
        }
        let doc = self.stack.pop().unwrap().1;
        let mut losses = self.losses;
        let doc = finalize(doc, &mut losses);
        (doc, losses)
    }
}

/// Normalize a whole document and assign task `localId`s.
pub(crate) fn finalize(doc: Value, losses: &mut Vec<Loss>) -> Value {
    let mut doc = normalize(doc, losses)
        .into_iter()
        .next()
        .unwrap_or_else(|| json!({"type": "doc", "version": 1, "content": []}));
    let mut n = 0;
    assign_ids(&mut doc, &mut n);
    doc
}

fn assign_ids(v: &mut Value, n: &mut usize) {
    if matches!(kind(v), "taskList" | "taskItem") {
        *n += 1;
        v["attrs"]["localId"] = format!("task-{n}").into();
    }
    if let Some(a) = v.get_mut("content").and_then(Value::as_array_mut) {
        for c in a {
            assign_ids(c, n);
        }
    }
}

fn paragraph(inline: Vec<Value>) -> Value {
    json!({"type": "paragraph", "content": inline})
}

fn empty_paragraph() -> Value {
    paragraph(Vec::new())
}

fn is_blank_run(run: &[Value]) -> bool {
    run.iter().all(|n| match kind(n) {
        "text" => n["text"].as_str().unwrap_or("").trim().is_empty(),
        BREAK => true,
        _ => false,
    })
}

/// Normalize children of a block container: wrap inline runs in paragraphs
/// (splitting at `__break`), drop blank runs (whitespace only, so nothing is
/// lost), and normalize block children.
fn blocks(children: Vec<Value>, adopt_orphans: bool, l: &mut Vec<Loss>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut run: Vec<Value> = Vec::new();
    let flush = |run: &mut Vec<Value>, out: &mut Vec<Value>, l: &mut Vec<Loss>| {
        if !is_blank_run(run) {
            out.extend(normalize(paragraph(std::mem::take(run)), l));
        }
        run.clear();
    };
    for c in children {
        if kind(&c) == BREAK {
            flush(&mut run, &mut out, l);
        } else if is_inline(&c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out, l);
            out.extend(normalize(if adopt_orphans { adopt(c) } else { c }, l));
        }
    }
    flush(&mut run, &mut out, l);
    out
}

/// Give a node that appears outside its required parent (an `<li>` outside a
/// list, a `<td>` outside a table) the parent it needs. Nothing is dropped.
fn adopt(c: Value) -> Value {
    match kind(&c) {
        "listItem" => json!({"type": "bulletList", "content": [c]}),
        "taskItem" => json!({"type": "taskList", "content": [c]}),
        "tableCell" | "tableHeader" => adopt(json!({"type": "tableRow", "content": [c]})),
        "tableRow" => json!({"type": "table", "content": [c]}),
        _ => c,
    }
}

/// Normalize a node; may return zero nodes (dropped) or several (split).
/// Anything dropped or flattened that carried content is recorded in `l`.
pub(crate) fn normalize(mut node: Value, l: &mut Vec<Loss>) -> Vec<Value> {
    let ty = kind(&node).to_string();
    match ty.as_str() {
        "doc" | "blockquote" | "listItem" | "tableCell" | "tableHeader" | "panel" | "expand"
        | "nestedExpand" | "bodiedExtension" | "layoutColumn" => {
            let mut kids = blocks(take_content(&mut node), true, l);
            match ty.as_str() {
                "blockquote" | "panel" | "listItem" => {
                    kids = kids
                        .into_iter()
                        .flat_map(|c| container_child(&ty, c, l))
                        .collect();
                }
                // The other containers accept any block.
                _ => {}
            }
            match ty.as_str() {
                // An empty quote or panel holds nothing worth keeping.
                "blockquote" | "panel" if kids.is_empty() => return Vec::new(),
                "listItem" => {
                    if kids.first().is_none_or(|k| kind(k) != "paragraph") {
                        kids.insert(0, empty_paragraph());
                    }
                }
                "tableCell" | "tableHeader" if kids.is_empty() => kids.push(empty_paragraph()),
                // Other containers keep their (possibly empty) content as is.
                _ => {}
            }
            node["content"] = Value::Array(kids);
            vec![node]
        }
        "paragraph" | "heading" => inline_block(node, l),
        "bulletList" | "orderedList" => {
            let mut items: Vec<Value> = Vec::new();
            for c in blocks(take_content(&mut node), false, l) {
                match kind(&c) {
                    "listItem" => items.push(c),
                    // A list nested directly in a list belongs to the previous item.
                    _ if is_list(&c) && !items.is_empty() => {
                        items.last_mut().unwrap()["content"]
                            .as_array_mut()
                            .unwrap()
                            .push(c);
                    }
                    _ => items.extend(normalize(json!({"type": "listItem", "content": [c]}), l)),
                }
            }
            // Only reachable when every child was blank.
            if items.is_empty() {
                return Vec::new();
            }
            node["content"] = Value::Array(items);
            vec![node]
        }
        "taskList" => {
            let mut items: Vec<Value> = Vec::new();
            for c in take_content(&mut node) {
                match kind(&c) {
                    "taskItem" => items.extend(normalize(c, l)),
                    "listItem" => {
                        let mut c = c;
                        c["type"] = "taskItem".into();
                        items.extend(normalize(c, l));
                    }
                    "taskList" => items.extend(normalize(c, l)),
                    _ if is_list(&c) => {
                        l.push(Loss::new(
                            kind(&c),
                            "taskList",
                            "a plain list inside a task list became a task list",
                        ));
                        items.extend(normalize(to_task_list(c), l));
                    }
                    _ if is_inline(&c) => {
                        // Blank inline runs (whitespace between elements) carry nothing.
                        if !is_blank_run(std::slice::from_ref(&c)) {
                            items.extend(normalize(json!({"type": "taskItem", "content": [c]}), l));
                        }
                    }
                    _ => items.extend(normalize(json!({"type": "taskItem", "content": [c]}), l)),
                }
            }
            // A taskList must start with a taskItem.
            if items.first().is_some_and(|i| kind(i) != "taskItem") {
                items.insert(0, task_item(Vec::new(), false));
            }
            if items.is_empty() {
                return Vec::new();
            }
            node["content"] = Value::Array(items);
            node["attrs"]["localId"] = "".into();
            vec![node]
        }
        "taskItem" => {
            let done = node["attrs"]["state"] == "DONE";
            let mut inline = Vec::new();
            let mut hoisted = Vec::new();
            let mut blocks_seen = 0;
            flatten_task(
                take_content(&mut node),
                &mut inline,
                &mut hoisted,
                &mut blocks_seen,
                l,
            );
            if blocks_seen > 1 {
                l.push(Loss::new(
                    "paragraph",
                    "taskItem",
                    "a task holds one line of text; its paragraphs were joined",
                ));
            }
            let inline = clean_inline(inline);
            let mut out = vec![task_item(inline, done)];
            for h in hoisted {
                if kind(&h) != "taskList" {
                    l.push(Loss::new(
                        kind(&h),
                        "taskItem",
                        "a plain list inside a task became a task list",
                    ));
                }
                out.extend(normalize(to_task_list(h), l));
            }
            out
        }
        "table" => {
            let mut rows: Vec<Value> = Vec::new();
            for c in take_content(&mut node) {
                let row = match kind(&c) {
                    "tableRow" => c,
                    "tableCell" | "tableHeader" => json!({"type": "tableRow", "content": [c]}),
                    // Whitespace between rows.
                    _ if is_inline(&c) && is_blank_run(std::slice::from_ref(&c)) => continue,
                    other => {
                        l.push(Loss::new(
                            other,
                            "table",
                            "content outside a table row was dropped",
                        ));
                        continue;
                    }
                };
                rows.extend(normalize(row, l));
            }
            if rows.is_empty() {
                return Vec::new();
            }
            node["content"] = Value::Array(rows);
            vec![node]
        }
        "tableRow" => {
            let mut cells = Vec::new();
            for c in take_content(&mut node) {
                match kind(&c) {
                    "tableCell" | "tableHeader" => cells.extend(normalize(c, l)),
                    // Whitespace between cells.
                    _ if is_inline(&c) && is_blank_run(std::slice::from_ref(&c)) => {}
                    _ => cells.extend(normalize(json!({"type": "tableCell", "content": [c]}), l)),
                }
            }
            if cells.is_empty() {
                return Vec::new();
            }
            node["content"] = Value::Array(cells);
            vec![node]
        }
        "codeBlock" => {
            if content(&node)
                .iter()
                .any(|c| !matches!(kind(c), "text" | "hardBreak") || c.get("marks").is_some())
            {
                l.push(Loss::new(
                    "codeBlock",
                    "codeBlock",
                    "a code block holds plain text; formatting and inline nodes inside it were reduced to text",
                ));
            }
            let mut text = String::new();
            collect_text(&node, &mut text);
            let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
            node["content"] = if text.is_empty() {
                json!([])
            } else {
                json!([{"type": "text", "text": text}])
            };
            if node["attrs"]["language"]
                .as_str()
                .is_some_and(|l| l.is_empty())
            {
                node.as_object_mut().unwrap().remove("attrs");
            }
            vec![node]
        }
        IMAGE => vec![media_single(&node)],
        _ if is_inline(&node) => normalize(paragraph(vec![node]), l),
        // Every other node is kept unchanged.
        _ => vec![node],
    }
}

/// Blocks a blockquote, panel or list item may hold; others are adapted or
/// flattened, and each change is recorded.
fn container_child(parent: &str, c: Value, l: &mut Vec<Loss>) -> Vec<Value> {
    let ok: &[&str] = match parent {
        "panel" => &[
            "paragraph",
            "heading",
            "bulletList",
            "orderedList",
            "taskList",
            "codeBlock",
            "mediaSingle",
            "mediaGroup",
            "rule",
            "blockCard",
            "decisionList",
            "extension",
        ],
        "listItem" => &[
            "paragraph",
            "bulletList",
            "orderedList",
            "taskList",
            "codeBlock",
            "mediaSingle",
            "extension",
        ],
        _ => &[
            "paragraph",
            "bulletList",
            "orderedList",
            "codeBlock",
            "mediaSingle",
            "mediaGroup",
            "extension",
        ],
    };
    let k = kind(&c).to_string();
    if ok.contains(&k.as_str()) {
        return vec![c];
    }
    match k.as_str() {
        "heading" => {
            l.push(Loss::new(
                "heading",
                parent,
                format!("a {parent} cannot hold a heading; it became a paragraph"),
            ));
            vec![json!({"type": "paragraph", "content": c["content"]})]
        }
        "blockquote" | "panel" => {
            l.push(Loss::new(
                &k,
                parent,
                format!("a {k} inside a {parent} was flattened into it"),
            ));
            content(&c)
                .iter()
                .cloned()
                .flat_map(|g| container_child(parent, g, l))
                .collect()
        }
        "rule" => {
            l.push(Loss::new(
                "rule",
                parent,
                format!("a {parent} cannot hold a horizontal rule; it was dropped"),
            ));
            Vec::new()
        }
        // Kept as is; the target schema decides.
        _ => vec![c],
    }
}

fn task_item(inline: Vec<Value>, done: bool) -> Value {
    json!({
        "type": "taskItem",
        "attrs": {"localId": "", "state": if done { "DONE" } else { "TODO" }},
        "content": inline,
    })
}

fn to_task_list(mut list: Value) -> Value {
    list["type"] = "taskList".into();
    if let Some(o) = list.get_mut("attrs").and_then(Value::as_object_mut) {
        o.remove("order");
    }
    list
}

/// Task items hold inline content only: paragraphs are joined with hard
/// breaks and nested lists are hoisted out as sibling task lists.
fn flatten_task(
    children: Vec<Value>,
    inline: &mut Vec<Value>,
    hoisted: &mut Vec<Value>,
    blocks_seen: &mut usize,
    l: &mut Vec<Loss>,
) {
    let mut had_inline = false;
    for c in children {
        if is_inline(&c) {
            // A block boundary marker carries no content.
            if kind(&c) == BREAK {
                continue;
            }
            if !had_inline {
                had_inline = true;
                *blocks_seen += 1;
            }
            inline.push(c);
        } else if is_list(&c) {
            hoisted.push(c);
        } else if kind(&c) == "codeBlock" {
            l.push(Loss::new(
                "codeBlock",
                "taskItem",
                "a task cannot hold a code block; it became inline code",
            ));
            let mut t = String::new();
            collect_text(&c, &mut t);
            push_break(inline);
            inline.push(json!({"type": "text", "text": t, "marks": [{"type": "code"}]}));
        } else {
            if kind(&c) != "paragraph" {
                l.push(Loss::new(
                    kind(&c),
                    "taskItem",
                    "a task holds inline content only; this block was flattened",
                ));
            }
            push_break(inline);
            flatten_task(
                c["content"].as_array().cloned().unwrap_or_default(),
                inline,
                hoisted,
                blocks_seen,
                l,
            );
        }
    }
}

fn push_break(inline: &mut Vec<Value>) {
    if inline
        .iter()
        .any(|n| kind(n) != "text" || !n["text"].as_str().unwrap_or("").trim().is_empty())
    {
        inline.push(json!({"type": "hardBreak"}));
    }
}

fn collect_text(v: &Value, out: &mut String) {
    match kind(v) {
        "text" => out.push_str(v["text"].as_str().unwrap_or("")),
        "hardBreak" => out.push('\n'),
        _ => {
            for c in content(v) {
                collect_text(c, out);
            }
        }
    }
}

fn media_single(img: &Value) -> Value {
    let mut attrs = json!({"type": "external", "url": img["attrs"]["url"]});
    if let Some(alt) = img["attrs"]["alt"].as_str().filter(|a| !a.is_empty()) {
        attrs["alt"] = alt.into();
    }
    json!({
        "type": "mediaSingle",
        "attrs": {"layout": "center"},
        "content": [{"type": "media", "attrs": attrs}],
    })
}

/// Image placeholder used inline (mixed with text): a link to the image.
fn image_link(img: &Value) -> Value {
    let url = img["attrs"]["url"].as_str().unwrap_or("");
    let alt = img["attrs"]["alt"].as_str().filter(|a| !a.is_empty());
    let mut marks = img["marks"].as_array().cloned().unwrap_or_default();
    marks.retain(|m| kind(m) != "link");
    marks.push(json!({"type": "link", "attrs": {"href": url}}));
    json!({"type": "text", "text": alt.unwrap_or(url), "marks": marks})
}

/// Normalize a paragraph or heading: clean inline content, hoist block
/// children out (splitting the node), and turn lone images into media.
fn inline_block(mut node: Value, l: &mut Vec<Loss>) -> Vec<Value> {
    let heading = kind(&node) == "heading";
    let mut out = Vec::new();
    let mut run = Vec::new();
    let children = take_content(&mut node);
    let shell = |run: Vec<Value>| {
        let mut n = node.clone();
        n["content"] = Value::Array(run);
        n
    };
    let mut segments: Vec<Vec<Value>> = Vec::new();
    let mut blocks_between: Vec<Vec<Value>> = Vec::new();
    let mut pending_blocks = Vec::new();
    for c in children {
        if kind(&c) == BREAK && !heading {
            segments.push(std::mem::take(&mut run));
            blocks_between.push(std::mem::take(&mut pending_blocks));
        } else if kind(&c) == BREAK {
            run.push(json!({"type": "text", "text": " "}));
        } else if is_inline(&c) {
            run.push(c);
        } else {
            segments.push(std::mem::take(&mut run));
            blocks_between.push(std::mem::take(&mut pending_blocks));
            pending_blocks.extend(normalize(c, l));
        }
    }
    segments.push(run);
    blocks_between.push(pending_blocks);
    for (seg, before) in segments.into_iter().zip(blocks_between) {
        out.extend(before);
        let non_blank: Vec<&Value> = seg
            .iter()
            .filter(|n| !is_blank_run(std::slice::from_ref(n)))
            .collect();
        if !heading && !non_blank.is_empty() && non_blank.iter().all(|n| kind(n) == IMAGE) {
            out.extend(non_blank.into_iter().map(media_single));
            continue;
        }
        let seg: Vec<Value> = seg
            .into_iter()
            .map(|n| {
                if kind(&n) == IMAGE {
                    l.push(Loss::new(
                        "image",
                        if heading { "heading" } else { "paragraph" },
                        "an image inside a line of text became a link to the image",
                    ));
                    image_link(&n)
                } else {
                    n
                }
            })
            .collect();
        let inline = clean_inline(seg);
        if !inline.is_empty() {
            out.push(shell(inline));
        }
    }
    out
}

/// Marks in canonical order. ADF lets `code` combine only with `link`, so
/// other marks on code text are dropped here; every ADF editor applies the
/// same rule, and the text itself is kept.
fn canonical_marks(marks: &[Value]) -> Vec<Value> {
    let mut seen: Vec<Value> = Vec::new();
    for m in marks {
        if !seen.iter().any(|s| kind(s) == kind(m)) {
            seen.push(m.clone());
        }
    }
    if seen.iter().any(|m| kind(m) == "code") {
        seen.retain(|m| matches!(kind(m), "code" | "link"));
    }
    seen.sort_by_key(|m| {
        MARK_ORDER
            .iter()
            .position(|k| *k == kind(m))
            .unwrap_or(MARK_ORDER.len())
    });
    seen
}

/// Clean inline content: canonical marks, merged adjacent text, no empty
/// text, trimmed edges and whitespace around hard breaks. Only whitespace and
/// internal block markers are removed.
pub(crate) fn clean_inline(nodes: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for mut n in nodes {
        match kind(&n) {
            BREAK => continue,
            "text" => {
                let marks = canonical_marks(n["marks"].as_array().map_or(&[], Vec::as_slice));
                let obj = n.as_object_mut().unwrap();
                if marks.is_empty() {
                    obj.remove("marks");
                } else {
                    obj.insert("marks".into(), Value::Array(marks));
                }
                if let Some(prev) = out.last_mut()
                    && kind(prev) == "text"
                    && prev.get("marks") == n.get("marks")
                {
                    let t = format!(
                        "{}{}",
                        prev["text"].as_str().unwrap_or(""),
                        n["text"].as_str().unwrap_or("")
                    );
                    prev["text"] = t.into();
                    continue;
                }
                out.push(n);
            }
            _ => out.push(n),
        }
    }
    // Trim whitespace at the edges and around hard breaks.
    let len = out.len();
    for i in 0..len {
        if kind(&out[i]) != "text" || has_mark(&out[i], "code") {
            continue;
        }
        let start = i == 0 || kind(&out[i - 1]) == "hardBreak";
        let end = i + 1 == len || kind(&out[i + 1]) == "hardBreak";
        let t = out[i]["text"].as_str().unwrap_or("");
        let t = match (start, end) {
            (true, true) => t.trim(),
            (true, false) => t.trim_start(),
            (false, true) => t.trim_end(),
            _ => t,
        }
        .to_string();
        out[i]["text"] = t.into();
    }
    out.retain(|n| kind(n) != "text" || !n["text"].as_str().unwrap_or("").is_empty());
    while out.last().is_some_and(|n| kind(n) == "hardBreak") {
        out.pop();
    }
    while out.first().is_some_and(|n| kind(n) == "hardBreak") {
        out.remove(0);
    }
    out
}

pub(crate) fn has_mark(n: &Value, mark: &str) -> bool {
    n["marks"]
        .as_array()
        .is_some_and(|ms| ms.iter().any(|m| kind(m) == mark))
}

/// A new node `{"type": ty, "attrs": attrs}` with empty content.
pub(crate) fn node(ty: &str, attrs: Map<String, Value>) -> Value {
    let mut n = json!({"type": ty, "content": []});
    if !attrs.is_empty() {
        n["attrs"] = Value::Object(attrs);
    }
    n
}

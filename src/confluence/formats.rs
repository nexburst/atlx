//! `content body-formats`: an offline guide to the body formats atlx reads
//! and writes. The text is atlx's own and describes atlx's converters.

use crate::client::ApiError;
use anyhow::Result;
use clap::ValueEnum;
use serde_json::{Value, json};

/// `--content-type` on `body-formats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum GuideType {
    Page,
    Blogpost,
    #[value(name = "live_doc")]
    LiveDoc,
    Whiteboard,
    Database,
}

/// Formats accepted by `--format` on `content create` and `content update`.
const WRITE_FORMATS: [&str; 4] = ["html", "md", "markdown", "plain"];

const INDEX: &str = "\
# Body formats in atlx

atlx talks to Confluence through its public REST API, where a page body is
stored as XHTML \"storage format\". Every body you send is turned into
storage format before it is written, and every body you read starts from it.

## Writing: `content create` and `content update` (`--format`)

| Format | What atlx does with it |
| --- | --- |
| `html` (recommended) | Sent unchanged. Use it for macros, layouts, panels and anything else Markdown cannot express. |
| `md` / `markdown` | Parsed as GitHub-flavoured Markdown and converted to storage format. |
| `plain` | Each blank-line-separated block becomes a paragraph; single line breaks are kept. Markup characters are escaped. |

## Reading: `content get` (`--format`)

| Format | What you get |
| --- | --- |
| `html` (default) | The storage-format XHTML exactly as Confluence returns it. Nothing is lost. |
| `md` / `markdown` | A Markdown rendering for reading and quoting. Lossy (see below). |
| `adf` (atlx extra) | The page as an ADF JSON document, including node `localId`s for `content update --edits`. |
| `url` | Only the page's web address. |

## What Markdown loses

Markdown covers headings, paragraphs, emphasis, links, lists, task lists,
tables, block quotes, rules and code blocks. Macros, panels, layouts,
expand blocks, mentions, status lozenges, emoji and media are dropped or
reduced to their text when a page is read as Markdown. Writing that
Markdown back would remove them from the page, so:

- read with `--format html` (or edit with `--edits`) when a page holds
  rich content you need to keep;
- use Markdown for new pages, simple pages, and for reading.

## Related commands

- `atlx confluence content body-formats <format>` explains one format.
- `atlx confluence content body-formats --content-type <type>` shows what a
  content type supports.
- `content update --edits` changes individual nodes and leaves the rest of
  the page untouched.
";

const HTML: &str = "\
# `html`: Confluence storage format

Storage format is XHTML with Confluence's own elements for macros
(`<ac:structured-macro>`), links to pages and users (`<ac:link>`,
`<ri:page>`, `<ri:user>`), images (`<ac:image>`) and layouts
(`<ac:layout>`).

- Writing: atlx sends the text as is. It must be well-formed XHTML: close
  every tag and write void elements as `<br/>`. Confluence rejects bodies
  it cannot parse.
- Reading: `content get --detail full --format html` returns the stored
  body without conversion, so a read followed by an update keeps every
  macro and attribute.
- Leave the page title out of the body; Confluence shows `--title` above
  it. Start with a paragraph or an `<h2>`.
";

const MARKDOWN: &str = "\
# `md` / `markdown`: GitHub-flavoured Markdown

- Writing: atlx parses the Markdown and converts it to storage format.
  Supported: `#` to `######` headings, `**bold**`, `*italic*`,
  `~~strike~~`, `` `code` ``, links, bullet and numbered lists (nested),
  `- [ ]` task lists, pipe tables, `>` quotes, `---` rules and fenced code
  blocks (the language becomes the code block's language).
- Reading: atlx renders the stored body as Markdown. Elements with no
  Markdown form (macros, panels, layouts, mentions, media) are dropped or
  kept only as text, so the result is meant for reading. Before rewriting
  a page that has them, read it as `html` or use `--edits`.
- `--section <heading>` reads return Markdown unless `--format html` is
  given.
";

const PLAIN: &str = "\
# `plain`: plain text

- Writing only. Blocks separated by a blank line become paragraphs, single
  newlines become line breaks, and `<`, `>`, `&` and quotes are escaped, so
  nothing in the text is treated as markup.
- To read a page as text, use `--format md`.
";

const ADF: &str = "\
# `adf`: Atlassian Document Format (atlx extra)

- Reading only: `content get --detail full --format adf` returns the page
  body as an ADF JSON document. Every node keeps its `localId`, which is
  what `content update --edits` operations refer to.
- `--detail outline` lists the headings with their `localId`s when that is
  all you need.
";

fn type_doc(t: GuideType) -> String {
    let name = t.to_possible_value().map(|v| v.get_name().to_string());
    let name = name.unwrap_or_default();
    match t {
        GuideType::Page | GuideType::Blogpost | GuideType::LiveDoc => {
            let extra = match t {
                GuideType::Blogpost => {
                    "\nBlog posts have no parent page; `--parent-id` is rejected.\n"
                }
                GuideType::LiveDoc => {
                    "\nLive docs are pages in REST v2, so atlx reads and writes them \
                     through the page endpoints, like any other page.\n"
                }
                _ => "",
            };
            format!(
                "# Body formats for `{name}`\n\n\
                 Write with `--format html`, `md`/`markdown` or `plain`.\n\
                 Read with `--format html` (default), `md`, `adf` (atlx extra) or `url`.\n\n\
                 Use `html` or `--edits` to change pages with macros or layouts \
                 without losing them; Markdown reads are lossy.\n{extra}\n\
                 Run `atlx confluence content body-formats <format>` for details on \
                 one format.\n"
            )
        }
        GuideType::Whiteboard | GuideType::Database => format!(
            "# Body formats for `{name}`\n\n\
             Confluence's public REST API does not return or accept the contents of \
             a {name}, so atlx cannot read or write its body. Metadata commands \
             (`content labels list --content-type {name}`) still work with its content ID.\n"
        ),
    }
}

/// `content body-formats [format] [--content-type]`:
/// `{format, validFormats, doc}` or `{contentType, validFormats, doc}`. An
/// unknown format shows the index.
pub fn guide(format: Option<&str>, content_type: Option<GuideType>) -> Result<Value> {
    if format.is_some() && content_type.is_some() {
        return Err(
            ApiError::usage("pass either a body format or --content-type, not both").into(),
        );
    }
    if let Some(t) = content_type {
        let valid: &[&str] = match t {
            GuideType::Whiteboard | GuideType::Database => &[],
            _ => &WRITE_FORMATS,
        };
        let name = t.to_possible_value().map(|v| v.get_name().to_string());
        return Ok(json!({"contentType": name, "validFormats": valid, "doc": type_doc(t)}));
    }
    let (name, doc) = match format.map(|f| f.trim().to_ascii_lowercase()).as_deref() {
        Some("html" | "storage") => ("html", HTML),
        Some("md" | "markdown") => ("md", MARKDOWN),
        Some("plain" | "text") => ("plain", PLAIN),
        Some("adf") => ("adf", ADF),
        _ => ("index", INDEX),
    };
    Ok(json!({"format": name, "validFormats": WRITE_FORMATS, "doc": doc}))
}

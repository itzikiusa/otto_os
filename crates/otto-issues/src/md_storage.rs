//! Markdown → Confluence storage-format XHTML.
//!
//! The block tree from [`crate::markdown`] (the same parser the Jira ADF
//! emitter uses) is written as storage XHTML: `<h1>`–`<h6>`, `<p>` with
//! `<br />` hard breaks, `<strong>` / `<em>` / `<s>` / `<code>` / `<a href>`,
//! nested `<ul>` / `<ol start>` (a list item's first paragraph sits inline in
//! the `<li>`, so no `<p>`-in-`<li>` spacing), native `<ac:task-list>` tasks,
//! `<blockquote>`, the `code` macro with its `language` parameter and a CDATA
//! body (a `]]>` inside the code is split across two sections),
//! `info` / `tip` / `note` / `warning` panel macros for `> [!NOTE]`-style
//! callouts, `<hr />`, and `<table><tbody>` with a `<th>` header row (rows
//! padded to the widest, empty cells kept). Text and attribute values are
//! XML-escaped. [`markdown_to_storage`] runs the result through
//! [`crate::storage_check::validate_storage`] and refuses anything invalid.

use otto_core::{Error, Result};

use crate::markdown::{inlines, normalize, para_spans, parse_blocks, Block, Inl, List, Mark};

/// Convert Markdown to validated Confluence storage XHTML.
pub fn markdown_to_storage(md: &str) -> Result<String> {
    let xhtml = markdown_to_storage_unchecked(md);
    crate::storage_check::validate_storage(&xhtml).map_err(|e| {
        Error::Invalid(format!(
            "the Markdown could not be converted to valid Confluence storage XHTML ({e}); \
             simplify the formatting around that element"
        ))
    })?;
    Ok(xhtml)
}

/// [`markdown_to_storage`] without the validation pass.
pub fn markdown_to_storage_unchecked(md: &str) -> String {
    let blocks = parse_blocks(&normalize(md), 0);
    let mut e = Emit::default();
    let mut out = String::new();
    e.blocks(&blocks, &mut out);
    out
}

/// Escape text content (`&`, `<`, `>`).
pub(crate) fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            c => o.push(c),
        }
    }
    o
}

/// Escape an attribute value (text escapes plus both quotes).
fn esc_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;").replace('\'', "&apos;")
}

/// A CDATA section holding `s`; a `]]>` inside is split across two sections.
fn cdata(s: &str) -> String {
    format!("<![CDATA[{}]]>", s.replace("]]>", "]]]]><![CDATA[>"))
}

/// Callout kind → Confluence panel macro (`info` blue, `tip` green, `note`
/// yellow, `warning` red).
fn panel_macro(kind: &str) -> &'static str {
    match kind {
        "TIP" | "SUCCESS" => "tip",
        "IMPORTANT" => "note",
        "WARNING" | "CAUTION" | "DANGER" | "ERROR" => "warning",
        _ => "info",
    }
}

/// Spans → inline XHTML. Marks nest link-outermost: `<a><strong><em><s><code>`.
fn inline(spans: &[Inl], extra: &[Mark], out: &mut String) {
    for s in spans {
        match s {
            Inl::Break => out.push_str("<br />"),
            Inl::Text(t, marks) => {
                if t.is_empty() {
                    continue;
                }
                let mut marks = marks.clone();
                if !marks.contains(&Mark::Code) {
                    marks = crate::markdown::with(&marks, extra);
                }
                let mut close = Vec::new();
                if let Some(Mark::Link(href)) = marks.iter().find(|m| matches!(m, Mark::Link(_))) {
                    out.push_str(&format!("<a href=\"{}\">", esc_attr(href)));
                    close.push("</a>");
                }
                for (m, open, end) in [
                    (Mark::Strong, "<strong>", "</strong>"),
                    (Mark::Em, "<em>", "</em>"),
                    (Mark::Strike, "<s>", "</s>"),
                    (Mark::Code, "<code>", "</code>"),
                ] {
                    if marks.contains(&m) {
                        out.push_str(open);
                        close.push(end);
                    }
                }
                out.push_str(&esc(t));
                for c in close.iter().rev() {
                    out.push_str(c);
                }
            }
        }
    }
}

#[derive(Default)]
struct Emit {
    tasks: usize,
}

impl Emit {
    fn blocks(&mut self, blocks: &[Block], out: &mut String) {
        for b in blocks {
            self.block(b, out);
        }
    }

    fn block(&mut self, b: &Block, out: &mut String) {
        match b {
            Block::Heading(level, text) => {
                out.push_str(&format!("<h{level}>"));
                inline(&inlines(text), &[], out);
                out.push_str(&format!("</h{level}>"));
            }
            Block::Para(lines) => {
                let spans = para_spans(lines);
                if spans
                    .iter()
                    .any(|s| matches!(s, Inl::Text(t, _) if !t.is_empty()))
                {
                    out.push_str("<p>");
                    inline(&spans, &[], out);
                    out.push_str("</p>");
                }
            }
            Block::Code(lang, text) => {
                out.push_str("<ac:structured-macro ac:name=\"code\">");
                if !lang.is_empty() {
                    out.push_str(&format!(
                        "<ac:parameter ac:name=\"language\">{}</ac:parameter>",
                        esc(&lang.to_ascii_lowercase())
                    ));
                }
                out.push_str("<ac:plain-text-body>");
                out.push_str(&cdata(text));
                out.push_str("</ac:plain-text-body></ac:structured-macro>");
            }
            Block::Rule => out.push_str("<hr />"),
            Block::Quote(children) => {
                let mut inner = String::new();
                self.blocks(children, &mut inner);
                if !inner.is_empty() {
                    out.push_str(&format!("<blockquote>{inner}</blockquote>"));
                }
            }
            Block::Panel(kind, children) => {
                let mut inner = String::new();
                self.blocks(children, &mut inner);
                out.push_str(&format!(
                    "<ac:structured-macro ac:name=\"{}\"><ac:rich-text-body>{inner}</ac:rich-text-body></ac:structured-macro>",
                    panel_macro(kind)
                ));
            }
            Block::List(list) => self.list(list, out),
            Block::Table(rows) => {
                out.push_str("<table><tbody>");
                for (n, row) in rows.iter().enumerate() {
                    let tag = if n == 0 { "th" } else { "td" };
                    out.push_str("<tr>");
                    for cell in row {
                        let spans = inlines(cell);
                        if spans.is_empty() {
                            out.push_str(&format!("<{tag}><p /></{tag}>"));
                        } else {
                            out.push_str(&format!("<{tag}><p>"));
                            inline(&spans, &[], out);
                            out.push_str(&format!("</p></{tag}>"));
                        }
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table>");
            }
        }
    }

    /// An item's content: a leading paragraph inline (no `<p>` inside the
    /// `<li>` / task body), every later block as a block.
    fn item_body(&mut self, blocks: &[Block], prefix: Option<&str>, out: &mut String) {
        let rest = match blocks.first() {
            Some(Block::Para(lines)) => {
                if let Some(p) = prefix {
                    out.push_str(p);
                }
                inline(&para_spans(lines), &[], out);
                &blocks[1..]
            }
            _ => {
                if let Some(p) = prefix {
                    out.push_str(p);
                }
                blocks
            }
        };
        self.blocks(rest, out);
    }

    fn list(&mut self, list: &List, out: &mut String) {
        let all_tasks = !list.items.is_empty() && list.items.iter().all(|it| it.task.is_some());
        if all_tasks && !list.ordered {
            out.push_str("<ac:task-list>");
            for it in &list.items {
                self.tasks += 1;
                let status = if it.task == Some(true) {
                    "complete"
                } else {
                    "incomplete"
                };
                out.push_str(&format!(
                    "<ac:task><ac:task-id>{}</ac:task-id><ac:task-status>{status}</ac:task-status><ac:task-body>",
                    self.tasks
                ));
                self.item_body(&it.blocks, None, out);
                out.push_str("</ac:task-body></ac:task>");
            }
            out.push_str("</ac:task-list>");
            return;
        }
        if list.ordered {
            if list.start == 1 {
                out.push_str("<ol>");
            } else {
                out.push_str(&format!("<ol start=\"{}\">", list.start));
            }
        } else {
            out.push_str("<ul>");
        }
        for it in &list.items {
            out.push_str("<li>");
            let tick = it.task.map(|done| if done { "☑ " } else { "☐ " });
            self.item_body(&it.blocks, tick, out);
            out.push_str("</li>");
        }
        out.push_str(if list.ordered { "</ol>" } else { "</ul>" });
    }
}

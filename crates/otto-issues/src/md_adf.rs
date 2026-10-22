//! Markdown → Atlassian Document Format (ADF) for Jira Cloud (REST v3).
//!
//! Jira Cloud takes descriptions and comments as ADF, not Markdown. The block
//! tree from [`crate::markdown`] is emitted as ADF that respects the schema's
//! nesting rules — a construct a
//! container cannot hold is DEGRADED (a heading inside a list item becomes a
//! bold paragraph, a table inside a quote becomes ` | `-joined rows) instead
//! of producing a body Jira rejects. [`markdown_to_adf`] runs the result
//! through [`crate::adf_check::validate_adf`] and refuses to return anything
//! invalid.
//!
//! Covered: ATX + setext headings (h1–h6), paragraphs (a single newline, two
//! trailing spaces, a trailing `\` or `<br>` is a `hardBreak`), `**strong**`
//! / `__strong__`, `*em*` / `_em_`, `~~strike~~`, `` `code` ``, links,
//! autolinks and bare URLs (images become links), bullet / ordered / nested
//! lists, `- [ ]` task lists (`taskList` at the top level, `☐`/`☑` prefixes
//! where ADF allows no `taskList`), blockquotes, `> [!NOTE]`-style callouts
//! as panels, fenced code with a language, horizontal rules and GFM tables
//! (header row → `tableHeader`, ragged rows padded to the widest row,
//! alignment ignored, `\|` is a literal pipe). Unicode emoji pass through as
//! text; `:shortcodes:` and `@mentions` stay literal text.

use otto_core::{Error, Result};
use serde_json::{json, Value};

use crate::markdown::{inlines, normalize, para_spans, parse_blocks, with, Block, Inl, List, Mark};

/// Convert Markdown to a validated ADF `doc`. An `Err` (never expected — the
/// emitter degrades instead) names the offending node so nothing broken is
/// ever sent to Jira.
pub fn markdown_to_adf(md: &str) -> Result<Value> {
    let doc = markdown_to_adf_unchecked(md);
    crate::adf_check::validate_adf(&doc).map_err(|e| {
        Error::Invalid(format!(
            "the Markdown could not be converted to valid Jira ADF ({e}); simplify the \
             formatting around that element"
        ))
    })?;
    Ok(doc)
}

/// [`markdown_to_adf`] without the validation pass (for tests of the validator).
pub fn markdown_to_adf_unchecked(md: &str) -> Value {
    let lines = normalize(md);
    let blocks = parse_blocks(&lines, 0);
    let mut st = Emit::default();
    let mut content = Vec::new();
    st.blocks(&blocks, Ctx::Doc, &mut content);
    json!({ "type": "doc", "version": 1, "content": content })
}

// ─────────────────────────────── emission ────────────────────────────────

fn mark_json(m: &Mark) -> Value {
    match m {
        Mark::Strong => json!({"type": "strong"}),
        Mark::Em => json!({"type": "em"}),
        Mark::Strike => json!({"type": "strike"}),
        Mark::Code => json!({"type": "code"}),
        Mark::Link(href) => json!({"type": "link", "attrs": {"href": href}}),
    }
}

/// Spans → ADF inline nodes: adjacent same-mark text merged, empty text dropped.
fn inline_nodes(spans: Vec<Inl>, extra: &[Mark]) -> Vec<Value> {
    let mut merged: Vec<Inl> = Vec::new();
    for s in spans {
        let s = match s {
            Inl::Text(t, m) if !extra.is_empty() && !m.contains(&Mark::Code) => {
                Inl::Text(t, with(&m, extra))
            }
            other => other,
        };
        match (merged.last_mut(), s) {
            (Some(Inl::Text(t, m)), Inl::Text(t2, m2)) if *m == m2 => t.push_str(&t2),
            (_, Inl::Text(t, _)) if t.is_empty() => {}
            (_, s) => merged.push(s),
        }
    }
    merged
        .into_iter()
        .map(|s| match s {
            Inl::Break => json!({"type": "hardBreak"}),
            Inl::Text(t, m) if m.is_empty() => json!({"type": "text", "text": t}),
            Inl::Text(t, m) => json!({
                "type": "text", "text": t,
                "marks": m.iter().map(mark_json).collect::<Vec<_>>()
            }),
        })
        .collect()
}

fn paragraph(nodes: Vec<Value>) -> Value {
    if nodes.is_empty() {
        json!({"type": "paragraph"})
    } else {
        json!({"type": "paragraph", "content": nodes})
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Doc,
    ListItem,
    Quote,
    Panel,
}

#[derive(Default)]
struct Emit {
    ids: usize,
}

impl Emit {
    fn id(&mut self, kind: &str) -> String {
        self.ids += 1;
        format!("otto-{kind}-{}", self.ids)
    }

    fn blocks(&mut self, blocks: &[Block], ctx: Ctx, out: &mut Vec<Value>) {
        for b in blocks {
            self.block(b, ctx, out);
        }
    }

    fn block(&mut self, b: &Block, ctx: Ctx, out: &mut Vec<Value>) {
        match b {
            Block::Heading(level, text) => match ctx {
                Ctx::Doc | Ctx::Panel => {
                    let nodes = inline_nodes(inlines(text), &[]);
                    let mut h = json!({"type": "heading", "attrs": {"level": level}});
                    if !nodes.is_empty() {
                        h["content"] = json!(nodes);
                    }
                    out.push(h);
                }
                // No headings in quotes / list items: a bold line instead.
                _ => out.push(paragraph(inline_nodes(inlines(text), &[Mark::Strong]))),
            },
            Block::Para(lines) => {
                let nodes = inline_nodes(para_spans(lines), &[]);
                if !nodes.is_empty() {
                    out.push(paragraph(nodes));
                }
            }
            Block::Code(lang, text) => {
                if ctx == Ctx::Panel {
                    // Panels take no code block: monospace lines instead.
                    let spans: Vec<Inl> = text
                        .split('\n')
                        .enumerate()
                        .flat_map(|(n, l)| {
                            let mut v = Vec::new();
                            if n > 0 {
                                v.push(Inl::Break);
                            }
                            v.push(Inl::Text(l.to_string(), vec![Mark::Code]));
                            v
                        })
                        .collect();
                    out.push(paragraph(inline_nodes(spans, &[])));
                    return;
                }
                let mut node = json!({"type": "codeBlock"});
                if !lang.is_empty() {
                    node["attrs"] = json!({"language": lang});
                }
                if !text.is_empty() {
                    node["content"] = json!([{"type": "text", "text": text}]);
                }
                out.push(node);
            }
            Block::Rule => {
                if ctx == Ctx::Doc {
                    out.push(json!({"type": "rule"}));
                }
            }
            Block::Quote(children) => match ctx {
                Ctx::Doc => {
                    let mut inner = Vec::new();
                    self.blocks(children, Ctx::Quote, &mut inner);
                    if !inner.is_empty() {
                        out.push(json!({"type": "blockquote", "content": inner}));
                    }
                }
                _ => self.blocks(children, ctx, out),
            },
            Block::Panel(kind, children) => {
                let kind = adf_panel(kind);
                let mut inner = Vec::new();
                let inner_ctx = if ctx == Ctx::Doc { Ctx::Panel } else { ctx };
                self.blocks(children, inner_ctx, &mut inner);
                if ctx == Ctx::Doc {
                    if inner.is_empty() {
                        inner.push(paragraph(Vec::new()));
                    }
                    out.push(
                        json!({"type": "panel", "attrs": {"panelType": kind}, "content": inner}),
                    );
                } else {
                    out.extend(inner);
                }
            }
            Block::List(list) => self.list(list, ctx, out),
            Block::Table(rows) => {
                if ctx == Ctx::Doc {
                    out.push(self.table(rows));
                    return;
                }
                // No tables outside the top level: one ` | `-joined line per row.
                for (n, row) in rows.iter().enumerate() {
                    let mut spans = Vec::new();
                    for (k, cell) in row.iter().enumerate() {
                        if k > 0 {
                            spans.push(Inl::Text(" | ".into(), Vec::new()));
                        }
                        spans.extend(inlines(cell));
                    }
                    let extra: &[Mark] = if n == 0 { &[Mark::Strong] } else { &[] };
                    let nodes = inline_nodes(spans, extra);
                    if !nodes.is_empty() {
                        out.push(paragraph(nodes));
                    }
                }
            }
        }
    }

    fn table(&mut self, rows: &[Vec<String>]) -> Value {
        let rows: Vec<Value> = rows
            .iter()
            .enumerate()
            .map(|(n, row)| {
                let cell_type = if n == 0 { "tableHeader" } else { "tableCell" };
                let cells: Vec<Value> = row
                    .iter()
                    .map(|text| {
                        let nodes = inline_nodes(inlines(text), &[]);
                        json!({"type": cell_type, "attrs": {}, "content": [paragraph(nodes)]})
                    })
                    .collect();
                json!({"type": "tableRow", "content": cells})
            })
            .collect();
        json!({
            "type": "table",
            "attrs": {"isNumberColumnEnabled": false, "layout": "default"},
            "content": rows
        })
    }

    fn list(&mut self, list: &List, ctx: Ctx, out: &mut Vec<Value>) {
        let all_tasks = list.items.iter().all(|it| it.task.is_some());
        if all_tasks && !list.ordered && ctx == Ctx::Doc {
            out.push(self.task_list(list));
            return;
        }
        let items: Vec<Value> = list
            .items
            .iter()
            .map(|it| {
                let mut inner = Vec::new();
                self.blocks(&it.blocks, Ctx::ListItem, &mut inner);
                // ADF: a list item opens with a paragraph or a code block.
                let opens_ok = inner
                    .first()
                    .is_some_and(|f| f["type"] == "paragraph" || f["type"] == "codeBlock");
                if !opens_ok {
                    inner.insert(0, paragraph(Vec::new()));
                }
                if let Some(done) = it.task {
                    let tick = json!({"type": "text", "text": if done { "☑ " } else { "☐ " }});
                    if inner[0]["type"] == "paragraph" {
                        let first = &mut inner[0];
                        let mut content = first["content"].as_array().cloned().unwrap_or_default();
                        content.insert(0, tick);
                        first["content"] = json!(content);
                    } else {
                        inner.insert(0, paragraph(vec![tick]));
                    }
                }
                json!({"type": "listItem", "content": inner})
            })
            .collect();
        let mut node = json!({
            "type": if list.ordered { "orderedList" } else { "bulletList" },
            "content": items
        });
        if list.ordered && list.start != 1 {
            node["attrs"] = json!({"order": list.start});
        }
        out.push(node);
    }

    /// A top-level `- [ ]` list. A task item holds inline content only, so
    /// extra paragraphs join with `hardBreak`s and nested non-task blocks
    /// flatten to text lines; a nested task list stays a nested `taskList`.
    fn task_list(&mut self, list: &List) -> Value {
        let list_id = self.id("tasks");
        let mut content = Vec::new();
        for it in &list.items {
            let mut spans: Vec<Inl> = Vec::new();
            let mut nested = Vec::new();
            for b in &it.blocks {
                match b {
                    Block::List(l) if l.items.iter().all(|x| x.task.is_some()) && !l.ordered => {
                        nested.push(self.task_list(l));
                    }
                    other => {
                        for line in plain_lines(other) {
                            if !spans.is_empty() {
                                spans.push(Inl::Break);
                            }
                            spans.extend(line);
                        }
                    }
                }
            }
            let mut item = json!({
                "type": "taskItem",
                "attrs": {"localId": self.id("task"), "state": if it.task == Some(true) { "DONE" } else { "TODO" }}
            });
            let nodes = inline_nodes(spans, &[]);
            if !nodes.is_empty() {
                item["content"] = json!(nodes);
            }
            content.push(item);
            content.extend(nested);
        }
        json!({"type": "taskList", "attrs": {"localId": list_id}, "content": content})
    }
}

/// A block flattened to inline lines (for task items).
fn plain_lines(b: &Block) -> Vec<Vec<Inl>> {
    match b {
        Block::Heading(_, t) => vec![inlines(t)],
        Block::Para(lines) => lines.iter().map(|l| inlines(l.trim())).collect(),
        Block::Code(_, t) => t
            .split('\n')
            .map(|l| vec![Inl::Text(l.to_string(), vec![Mark::Code])])
            .collect(),
        Block::Rule => Vec::new(),
        Block::Quote(c) | Block::Panel(_, c) => c.iter().flat_map(plain_lines).collect(),
        Block::List(l) => l
            .items
            .iter()
            .flat_map(|it| {
                let lines: Vec<Vec<Inl>> = it.blocks.iter().flat_map(plain_lines).collect();
                lines.into_iter().enumerate().map(|(n, mut line)| {
                    if n == 0 {
                        let bullet = match it.task {
                            Some(true) => "☑ ",
                            Some(false) => "☐ ",
                            None => "• ",
                        };
                        line.insert(0, Inl::Text(bullet.into(), Vec::new()));
                    }
                    line
                })
            })
            .collect(),
        Block::Table(rows) => rows
            .iter()
            .map(|r| vec![Inl::Text(r.join(" | "), Vec::new())])
            .collect(),
    }
}

/// Callout kind → ADF `panelType`, colour-matched to GitHub's alerts (NOTE is
/// blue → `info`, IMPORTANT purple → `note`, TIP green → `success`, WARNING
/// yellow → `warning`, CAUTION red → `error`).
fn adf_panel(kind: &str) -> &'static str {
    match kind {
        "IMPORTANT" => "note",
        "TIP" | "SUCCESS" => "success",
        "WARNING" => "warning",
        "CAUTION" | "DANGER" | "ERROR" => "error",
        _ => "info",
    }
}

//! Structural checks on Atlassian Document Format (ADF).
//!
//! - [`validate_adf`] — the schema rules Jira enforces on the nodes Otto
//!   emits (allowed children per node, no empty text, marks only on text and
//!   never in a code block, `code` only alongside `link`, required attrs).
//!   Unknown node types are an error: Otto must never send what it can't vouch for.
//! - [`adf_outline`] / [`adf_plain_text`] — the block structure and the text
//!   of a document, so a description read back after a create can be compared
//!   with what was sent (Jira adds attrs such as `localId`; those are ignored).

use serde_json::Value;

const INLINE: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "inlineCard",
    "date",
    "status",
];
const DOC_BLOCKS: &[&str] = &[
    "paragraph",
    "heading",
    "bulletList",
    "orderedList",
    "taskList",
    "codeBlock",
    "blockquote",
    "panel",
    "rule",
    "table",
];
const LIST_ITEM: &[&str] = &["paragraph", "bulletList", "orderedList", "codeBlock"];
const QUOTE: &[&str] = &["paragraph", "bulletList", "orderedList", "codeBlock"];
const PANEL: &[&str] = &["paragraph", "heading", "bulletList", "orderedList"];
const CELL: &[&str] = &[
    "paragraph",
    "heading",
    "bulletList",
    "orderedList",
    "codeBlock",
    "blockquote",
    "panel",
    "rule",
];
const PANEL_TYPES: &[&str] = &["info", "note", "warning", "success", "error"];
const MARKS: &[&str] = &["strong", "em", "strike", "code", "link", "underline"];

/// `Ok` when `doc` is a structurally valid ADF document; otherwise the path
/// of the first offending node and why (e.g. `content[2].content[0]: …`).
pub fn validate_adf(doc: &Value) -> Result<(), String> {
    if doc["type"] != "doc" {
        return Err("root: type must be \"doc\"".into());
    }
    if doc["version"] != 1 {
        return Err("root: version must be 1".into());
    }
    let content = doc["content"]
        .as_array()
        .ok_or("root: content must be an array")?;
    children(content, "content", DOC_BLOCKS, "doc", false)
}

fn children(
    nodes: &[Value],
    path: &str,
    allowed: &[&str],
    parent: &str,
    allow_marks: bool,
) -> Result<(), String> {
    for (n, node) in nodes.iter().enumerate() {
        let p = format!("{path}[{n}]");
        let ty = node["type"].as_str().unwrap_or("");
        if !allowed.contains(&ty) {
            return Err(format!("{p}: a {parent} cannot contain \"{ty}\""));
        }
        check(node, ty, &p, allow_marks)?;
    }
    Ok(())
}

fn content<'a>(node: &'a Value, p: &str, min: usize) -> Result<&'a [Value], String> {
    let c = match node.get("content") {
        None => &[][..],
        Some(Value::Array(a)) => a.as_slice(),
        Some(_) => return Err(format!("{p}: content must be an array")),
    };
    if c.len() < min {
        return Err(format!("{p}: needs at least {min} child node(s)"));
    }
    Ok(c)
}

fn attr_str<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node["attrs"][key].as_str().filter(|s| !s.is_empty())
}

fn check(node: &Value, ty: &str, p: &str, allow_marks: bool) -> Result<(), String> {
    if ty != "text" && node.get("marks").is_some() {
        return Err(format!("{p}: only text nodes carry marks"));
    }
    let cp = format!("{p}.content");
    match ty {
        "text" => {
            let text = node["text"].as_str().unwrap_or("");
            if text.is_empty() {
                return Err(format!("{p}: empty text node"));
            }
            if let Some(marks) = node.get("marks") {
                if !allow_marks {
                    return Err(format!("{p}: marks are not allowed here"));
                }
                check_marks(marks, p)?;
            }
        }
        "hardBreak" | "rule" => {
            content(node, p, 0)?
                .first()
                .map_or(Ok(()), |_| Err(format!("{p}: {ty} has no content")))?;
        }
        "mention" | "emoji" | "inlineCard" | "date" | "status" => {}
        "paragraph" => children(content(node, p, 0)?, &cp, INLINE, ty, true)?,
        "heading" => {
            let level = node["attrs"]["level"].as_u64().unwrap_or(0);
            if !(1..=6).contains(&level) {
                return Err(format!("{p}: heading level must be 1-6"));
            }
            children(content(node, p, 0)?, &cp, INLINE, ty, true)?;
        }
        "bulletList" | "orderedList" => {
            if let Some(order) = node["attrs"].get("order") {
                if order.as_u64().is_none() {
                    return Err(format!("{p}: order must be a non-negative integer"));
                }
            }
            children(content(node, p, 1)?, &cp, &["listItem"], ty, false)?;
        }
        "listItem" => {
            let c = content(node, p, 1)?;
            let first = c[0]["type"].as_str().unwrap_or("");
            if first != "paragraph" && first != "codeBlock" {
                return Err(format!(
                    "{cp}[0]: a listItem must open with a paragraph or codeBlock"
                ));
            }
            children(c, &cp, LIST_ITEM, ty, false)?;
        }
        "taskList" => {
            if attr_str(node, "localId").is_none() {
                return Err(format!("{p}: taskList needs attrs.localId"));
            }
            let c = content(node, p, 1)?;
            if c[0]["type"] != "taskItem" {
                return Err(format!("{cp}[0]: a taskList must open with a taskItem"));
            }
            children(c, &cp, &["taskItem", "taskList"], ty, false)?;
        }
        "taskItem" => {
            if attr_str(node, "localId").is_none() {
                return Err(format!("{p}: taskItem needs attrs.localId"));
            }
            if !matches!(attr_str(node, "state"), Some("TODO" | "DONE")) {
                return Err(format!("{p}: taskItem state must be TODO or DONE"));
            }
            children(content(node, p, 0)?, &cp, INLINE, ty, true)?;
        }
        "codeBlock" => {
            if let Some(l) = node["attrs"].get("language") {
                if !l.is_string() {
                    return Err(format!("{p}: codeBlock language must be a string"));
                }
            }
            children(content(node, p, 0)?, &cp, &["text"], ty, false)?;
        }
        "blockquote" => children(content(node, p, 1)?, &cp, QUOTE, ty, false)?,
        "panel" => {
            let kind = attr_str(node, "panelType").unwrap_or("");
            if !PANEL_TYPES.contains(&kind) {
                return Err(format!("{p}: unknown panelType \"{kind}\""));
            }
            children(content(node, p, 1)?, &cp, PANEL, ty, false)?;
        }
        "table" => {
            let rows = content(node, p, 1)?;
            children(rows, &cp, &["tableRow"], ty, false)?;
            let width = |r: &Value| r["content"].as_array().map_or(0, Vec::len);
            let w0 = width(&rows[0]);
            if let Some(n) = rows.iter().position(|r| width(r) != w0) {
                return Err(format!(
                    "{cp}[{n}]: row has {} cells, the header row has {w0}",
                    width(&rows[n])
                ));
            }
        }
        "tableRow" => children(
            content(node, p, 1)?,
            &cp,
            &["tableHeader", "tableCell"],
            ty,
            false,
        )?,
        "tableHeader" | "tableCell" => children(content(node, p, 1)?, &cp, CELL, ty, false)?,
        other => return Err(format!("{p}: unknown node type \"{other}\"")),
    }
    Ok(())
}

fn check_marks(marks: &Value, p: &str) -> Result<(), String> {
    let marks = marks
        .as_array()
        .ok_or_else(|| format!("{p}: marks must be an array"))?;
    let mut seen: Vec<&str> = Vec::new();
    for m in marks {
        let ty = m["type"].as_str().unwrap_or("");
        if !MARKS.contains(&ty) {
            return Err(format!("{p}: unknown mark \"{ty}\""));
        }
        if seen.contains(&ty) {
            return Err(format!("{p}: duplicate {ty} mark"));
        }
        if ty == "link" && attr_str(m, "href").is_none() {
            return Err(format!("{p}: link mark needs attrs.href"));
        }
        seen.push(ty);
    }
    if seen.contains(&"code") && seen.iter().any(|t| *t != "code" && *t != "link") {
        return Err(format!("{p}: the code mark combines only with link"));
    }
    Ok(())
}

/// The block skeleton of a document, one entry per block node in document
/// order: `heading:2`, `bulletList[3]`, `codeBlock:rust`, `panel:info`,
/// `table[3x4,header]`, `taskItem:DONE`, … Inline content and Jira-added
/// attrs (`localId`, table `layout`, …) are ignored.
pub fn adf_outline(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(c) = doc["content"].as_array() {
        for n in c {
            outline(n, &mut out);
        }
    }
    out
}

fn outline(node: &Value, out: &mut Vec<String>) {
    let ty = node["type"].as_str().unwrap_or("");
    let kids = node["content"].as_array().map_or(&[][..], Vec::as_slice);
    let entry = match ty {
        "text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "date" | "status" => return,
        "heading" => format!("heading:{}", node["attrs"]["level"]),
        "bulletList" | "orderedList" | "taskList" => format!("{ty}[{}]", kids.len()),
        "codeBlock" => format!(
            "codeBlock:{}",
            node["attrs"]["language"].as_str().unwrap_or("")
        ),
        "panel" => format!(
            "panel:{}",
            node["attrs"]["panelType"].as_str().unwrap_or("")
        ),
        "taskItem" => format!("taskItem:{}", node["attrs"]["state"].as_str().unwrap_or("")),
        "table" => {
            let cols = kids
                .first()
                .and_then(|r| r["content"].as_array())
                .map_or(0, Vec::len);
            let header = kids
                .first()
                .and_then(|r| r["content"].as_array())
                .is_some_and(|cells| {
                    !cells.is_empty() && cells.iter().all(|c| c["type"] == "tableHeader")
                });
            format!(
                "table[{}x{cols}{}]",
                kids.len(),
                if header { ",header" } else { "" }
            )
        }
        other => other.to_string(),
    };
    out.push(entry);
    for k in kids {
        outline(k, out);
    }
}

/// Every text node's text, whitespace-collapsed (a `hardBreak` is a space).
pub fn adf_plain_text(doc: &Value) -> String {
    fn walk(n: &Value, out: &mut String) {
        if let Some(t) = n["text"].as_str() {
            out.push_str(t);
        }
        if n["type"] == "hardBreak" {
            out.push(' ');
        }
        if let Some(c) = n["content"].as_array() {
            for k in c {
                walk(k, out);
            }
            out.push(' ');
        }
    }
    let mut s = String::new();
    walk(doc, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }

    fn para(text: &str) -> Value {
        json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn accepts_a_well_formed_document() {
        let d = doc(json!([
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "H"}]},
            para("p"),
            {"type": "bulletList", "content": [{"type": "listItem", "content": [para("a"),
                {"type": "orderedList", "attrs": {"order": 3}, "content": [{"type": "listItem", "content": [para("b")]}]}]}]},
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn x() {}"}]},
            {"type": "codeBlock"},
            {"type": "rule"},
            {"type": "panel", "attrs": {"panelType": "warning"}, "content": [para("w")]},
            {"type": "blockquote", "content": [para("q")]},
            {"type": "taskList", "attrs": {"localId": "l"}, "content": [
                {"type": "taskItem", "attrs": {"localId": "t", "state": "DONE"}, "content": [{"type": "text", "text": "x"}]}]},
            {"type": "table", "content": [
                {"type": "tableRow", "content": [{"type": "tableHeader", "content": [para("h")]}]},
                {"type": "tableRow", "content": [{"type": "tableCell", "content": [{"type": "paragraph"}]}]}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "c", "marks": [{"type": "code"}, {"type": "link", "attrs": {"href": "https://x"}}]}]}
        ]));
        assert_eq!(validate_adf(&d), Ok(()));
    }

    #[test]
    fn rejects_each_structural_violation_with_its_path() {
        let cases = [
            (
                doc(json!([{"type": "paragraph", "content": [{"type": "text", "text": ""}]}])),
                "content[0].content[0]: empty text",
            ),
            (
                doc(json!([{"type": "text", "text": "loose"}])),
                "doc cannot contain \"text\"",
            ),
            (
                doc(json!([{"type": "heading", "attrs": {"level": 7}}])),
                "heading level",
            ),
            (
                doc(json!([{"type": "bulletList", "content": []}])),
                "at least 1",
            ),
            (
                doc(
                    json!([{"type": "bulletList", "content": [{"type": "listItem", "content": [
                {"type": "heading", "attrs": {"level": 1}}]}]}]),
                ),
                "must open with a paragraph",
            ),
            (
                doc(json!([{"type": "blockquote", "content": [{"type": "table", "content": []}]}])),
                "blockquote cannot contain \"table\"",
            ),
            (
                doc(
                    json!([{"type": "codeBlock", "content": [{"type": "text", "text": "x", "marks": [{"type": "strong"}]}]}]),
                ),
                "marks are not allowed",
            ),
            (
                doc(
                    json!([{"type": "paragraph", "content": [{"type": "text", "text": "x", "marks": [{"type": "code"}, {"type": "strong"}]}]}]),
                ),
                "code mark combines only with link",
            ),
            (
                doc(
                    json!([{"type": "paragraph", "content": [{"type": "text", "text": "x", "marks": [{"type": "link"}]}]}]),
                ),
                "attrs.href",
            ),
            (
                doc(
                    json!([{"type": "panel", "attrs": {"panelType": "tip"}, "content": [para("x")]}]),
                ),
                "panelType",
            ),
            (
                doc(json!([{"type": "table", "content": [
                {"type": "tableRow", "content": [{"type": "tableHeader", "content": [para("a")]}, {"type": "tableHeader", "content": [para("b")]}]},
                {"type": "tableRow", "content": [{"type": "tableCell", "content": [para("c")]}]}]}])),
                "row has 1 cells, the header row has 2",
            ),
            (
                doc(
                    json!([{"type": "table", "content": [{"type": "tableRow", "content": [{"type": "tableCell", "content": []}]}]}]),
                ),
                "at least 1",
            ),
            (
                doc(
                    json!([{"type": "taskList", "attrs": {"localId": "l"}, "content": [
                {"type": "taskItem", "attrs": {"localId": "t", "state": "MAYBE"}}]}]),
                ),
                "TODO or DONE",
            ),
            (
                doc(json!([{"type": "paragraph", "marks": [{"type": "strong"}]}])),
                "only text nodes carry marks",
            ),
            (
                doc(json!([{"type": "mediaSingle"}])),
                "doc cannot contain \"mediaSingle\"",
            ),
            (
                json!({"type": "doc", "version": 2, "content": []}),
                "version",
            ),
        ];
        for (d, needle) in cases {
            let err = validate_adf(&d).expect_err(needle);
            assert!(err.contains(needle), "want {needle:?}, got {err:?}");
        }
    }

    #[test]
    fn outline_ignores_jira_added_attrs_and_inline_detail() {
        let sent = doc(json!([
            {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "T"}]},
            {"type": "table", "attrs": {"layout": "default"}, "content": [
                {"type": "tableRow", "content": [{"type": "tableHeader", "content": [para("a")]}, {"type": "tableHeader", "content": [para("b")]}]},
                {"type": "tableRow", "content": [{"type": "tableCell", "content": [para("1")]}, {"type": "tableCell", "content": [para("2")]}]}]}
        ]));
        let mut stored = sent.clone();
        stored["content"][1]["attrs"] = json!({"layout": "center", "localId": "abc"});
        stored["content"][0]["content"][0]["marks"] = json!([{"type": "strong"}]);
        assert_eq!(adf_outline(&sent), adf_outline(&stored));
        assert_eq!(adf_outline(&sent)[1], "table[2x2,header]");
        assert_eq!(adf_plain_text(&sent), "T a b 1 2");
    }
}

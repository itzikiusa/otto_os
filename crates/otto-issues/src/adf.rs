//! Atlassian Document Format (ADF) helpers.
//!
//! Provides two pure conversion helpers:
//! - [`text_to_adf`]: converts plain text (with `- ` bullet lines) to an ADF JSON document.
//! - [`adf_to_markdown`]: converts an ADF JSON document to a Markdown string.

use serde_json::{json, Value};

/// Convert plain text to an ADF document.
///
/// Paragraphs are split on blank lines (`\n\n`). Within a paragraph block,
/// lines starting with `"- "` are grouped into a `bulletList`. Other lines
/// are collected into `paragraph` nodes (joined with `hardBreak`s if multiple
/// non-bullet lines appear together).
pub fn text_to_adf(text: &str) -> Value {
    // Split into paragraph chunks on blank lines.
    let mut content: Vec<Value> = Vec::new();

    for chunk in text.split("\n\n") {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }

        // Check whether the chunk is purely bullet lines or mixed.
        let lines: Vec<&str> = chunk.lines().collect();

        // Collect consecutive bullet lines into bullet lists, non-bullet lines
        // into paragraph nodes.
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim();
            if line.starts_with("- ") {
                // Collect run of bullet lines.
                let mut items: Vec<Value> = Vec::new();
                while i < lines.len() && lines[i].trim().starts_with("- ") {
                    let bullet_text = lines[i].trim().strip_prefix("- ").unwrap_or("").trim();
                    items.push(json!({
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": bullet_text}]
                        }]
                    }));
                    i += 1;
                }
                content.push(json!({
                    "type": "bulletList",
                    "content": items
                }));
            } else {
                // Collect run of non-bullet lines into a single paragraph.
                let mut para_content: Vec<Value> = Vec::new();
                while i < lines.len() && !lines[i].trim().starts_with("- ") {
                    if !para_content.is_empty() {
                        para_content.push(json!({"type": "hardBreak"}));
                    }
                    para_content.push(json!({"type": "text", "text": lines[i].trim()}));
                    i += 1;
                }
                if !para_content.is_empty() {
                    content.push(json!({
                        "type": "paragraph",
                        "content": para_content
                    }));
                }
            }
        }
    }

    json!({
        "type": "doc",
        "version": 1,
        "content": content
    })
}

/// Convert an ADF JSON document to a Markdown string.
///
/// Handles: `paragraph`, `text` (with marks: `strong`, `em`, `code`, `link`),
/// `heading` (with `level` attr), `bulletList`, `orderedList`, `listItem`,
/// `codeBlock`, `hardBreak`. Unknown nodes are recursed into (best-effort).
pub fn adf_to_markdown(adf: &Value) -> String {
    let mut out = String::new();
    render_node(adf, &mut out, 0, false);
    // Trim trailing whitespace / newlines.
    out.trim_end().to_string()
}

/// Recursive node renderer. `list_index` is 1-based for ordered lists (0 = not in list).
fn render_node(node: &Value, out: &mut String, list_index: usize, ordered: bool) {
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let children = node.get("content").and_then(|v| v.as_array());

    match node_type {
        "doc" => {
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, 0, false);
                }
            }
        }
        "paragraph" => {
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, 0, false);
                }
            }
            out.push_str("\n\n");
        }
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as usize;
            let hashes = "#".repeat(level.min(6));
            out.push_str(&hashes);
            out.push(' ');
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, 0, false);
                }
            }
            out.push_str("\n\n");
        }
        "text" => {
            let text = node.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let marks = node
                .get("marks")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            // Build wrapped text from innermost to outermost mark.
            let mut result = text.to_string();
            let mut link_href: Option<String> = None;

            for mark in &marks {
                let mark_type = mark.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match mark_type {
                    "strong" => result = format!("**{result}**"),
                    "em" => result = format!("_{result}_"),
                    "code" => result = format!("`{result}`"),
                    "link" => {
                        let href = mark
                            .get("attrs")
                            .and_then(|a| a.get("href"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        link_href = Some(href);
                    }
                    _ => {}
                }
            }

            if let Some(href) = link_href {
                result = format!("[{result}]({href})");
            }

            out.push_str(&result);
        }
        "hardBreak" => {
            out.push('\n');
        }
        "bulletList" => {
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, 0, false);
                }
            }
            out.push('\n');
        }
        "orderedList" => {
            if let Some(children) = children {
                for (idx, child) in children.iter().enumerate() {
                    render_ordered_item(child, out, idx + 1);
                }
            }
            out.push('\n');
        }
        "listItem" => {
            // Called from bulletList context.
            out.push_str("- ");
            if let Some(children) = children {
                for child in children {
                    render_list_item_child(child, out);
                }
            }
            out.push('\n');
        }
        "codeBlock" => {
            let lang = node
                .get("attrs")
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str("```");
            out.push_str(lang);
            out.push('\n');
            if let Some(children) = children {
                for child in children {
                    let text = child.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    out.push_str(text);
                }
            }
            out.push_str("\n```\n\n");
        }
        _ => {
            // Unknown node: recurse into children best-effort.
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, list_index, ordered);
                }
            }
        }
    }
}

/// Render a list item's children without the surrounding paragraph double-newlines.
fn render_list_item_child(node: &Value, out: &mut String) {
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let children = node.get("content").and_then(|v| v.as_array());
    match node_type {
        "paragraph" => {
            if let Some(children) = children {
                for child in children {
                    render_node(child, out, 0, false);
                }
            }
            // Don't add double newline — just a single newline is handled by listItem
        }
        _ => render_node(node, out, 0, false),
    }
}

fn render_ordered_item(node: &Value, out: &mut String, index: usize) {
    let children = node.get("content").and_then(|v| v.as_array());
    out.push_str(&format!("{index}. "));
    if let Some(children) = children {
        for child in children {
            render_list_item_child(child, out);
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- text_to_adf tests ----

    #[test]
    fn test_text_to_adf_structure() {
        let doc = text_to_adf("Hello world");
        assert_eq!(doc["type"], "doc");
        assert_eq!(doc["version"], 1);
        assert!(doc["content"].is_array());
    }

    #[test]
    fn test_text_to_adf_two_paragraphs() {
        let input = "First paragraph\n\nSecond paragraph";
        let doc = text_to_adf(input);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "Expected 2 content nodes, got: {content:?}");
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[1]["type"], "paragraph");
        // Check text content
        assert_eq!(content[0]["content"][0]["text"], "First paragraph");
        assert_eq!(content[1]["content"][0]["text"], "Second paragraph");
    }

    #[test]
    fn test_text_to_adf_bullet_lines() {
        let input = "- Item one\n- Item two\n- Item three";
        let doc = text_to_adf(input);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content.len(), 1, "Expected 1 bulletList node");
        assert_eq!(content[0]["type"], "bulletList");
        let items = content[0]["content"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["type"], "listItem");
        assert_eq!(items[0]["content"][0]["content"][0]["text"], "Item one");
        assert_eq!(items[1]["content"][0]["content"][0]["text"], "Item two");
        assert_eq!(items[2]["content"][0]["content"][0]["text"], "Item three");
    }

    #[test]
    fn test_text_to_adf_paragraph_plus_bullet() {
        // Two blank-line-separated blocks: a paragraph, then bullets.
        let input = "Intro text\n\n- Bullet A\n- Bullet B";
        let doc = text_to_adf(input);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "Expected paragraph + bulletList: {content:?}");
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[1]["type"], "bulletList");
        let items = content[1]["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["content"][0]["content"][0]["text"], "Bullet A");
        assert_eq!(items[1]["content"][0]["content"][0]["text"], "Bullet B");
    }

    #[test]
    fn test_text_to_adf_mixed_paragraph_and_bullets_in_one_chunk() {
        // Within a single chunk (no blank line) mix text + bullets.
        let input = "Header line\n- Bullet one\n- Bullet two";
        let doc = text_to_adf(input);
        let content = doc["content"].as_array().unwrap();
        // Should produce a paragraph node + a bulletList node
        assert!(content.len() >= 2, "Expected at least 2 nodes: {content:?}");
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[1]["type"], "bulletList");
    }

    // ---- adf_to_markdown tests ----

    #[test]
    fn test_adf_to_markdown_heading() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "heading",
                "attrs": {"level": 2},
                "content": [{"type": "text", "text": "My Heading"}]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("## My Heading"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_paragraph() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "Hello world"}]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("Hello world"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_link() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "click here",
                    "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]
                }]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("[click here](https://example.com)"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_bullet_list() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "bulletList",
                "content": [
                    {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Alpha"}]}]},
                    {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Beta"}]}]}
                ]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("- Alpha"), "Got: {md:?}");
        assert!(md.contains("- Beta"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_strong_and_em() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "bold", "marks": [{"type": "strong"}]},
                    {"type": "text", "text": " and "},
                    {"type": "text", "text": "italic", "marks": [{"type": "em"}]}
                ]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("**bold**"), "Got: {md:?}");
        assert!(md.contains("_italic_"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_code_block() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "codeBlock",
                "attrs": {"language": "rust"},
                "content": [{"type": "text", "text": "fn main() {}"}]
            }]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("```rust"), "Got: {md:?}");
        assert!(md.contains("fn main() {}"), "Got: {md:?}");
        assert!(md.contains("```"), "Got: {md:?}");
    }

    #[test]
    fn test_adf_to_markdown_unknown_node_skips_gracefully() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "someUnknownNode",
                "content": [{"type": "text", "text": "inner text"}]
            }]
        });
        // Should not panic, should recurse and get the inner text
        let md = adf_to_markdown(&adf);
        assert!(md.contains("inner text"), "Got: {md:?}");
    }

    #[test]
    fn test_round_trip_paragraphs_and_bullets() {
        // text_to_adf then adf_to_markdown should preserve paragraph text and bullet text.
        let input = "First para\n\nSecond para\n\n- Alpha\n- Beta";
        let adf = text_to_adf(input);
        let md = adf_to_markdown(&adf);
        assert!(md.contains("First para"), "round-trip lost 'First para': {md:?}");
        assert!(md.contains("Second para"), "round-trip lost 'Second para': {md:?}");
        assert!(md.contains("- Alpha"), "round-trip lost '- Alpha': {md:?}");
        assert!(md.contains("- Beta"), "round-trip lost '- Beta': {md:?}");
    }

    #[test]
    fn test_complex_adf_heading_paragraph_link_bulletlist() {
        // A realistic ADF doc: heading + paragraph with link + bullet list.
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "heading",
                    "attrs": {"level": 1},
                    "content": [{"type": "text", "text": "Summary"}]
                },
                {
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "See "},
                        {
                            "type": "text",
                            "text": "the docs",
                            "marks": [{"type": "link", "attrs": {"href": "https://docs.example.com"}}]
                        },
                        {"type": "text", "text": " for details."}
                    ]
                },
                {
                    "type": "bulletList",
                    "content": [
                        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Point one"}]}]},
                        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Point two"}]}]}
                    ]
                }
            ]
        });
        let md = adf_to_markdown(&adf);
        assert!(md.contains("# Summary"), "Missing heading: {md:?}");
        assert!(md.contains("[the docs](https://docs.example.com)"), "Missing link: {md:?}");
        assert!(md.contains("for details."), "Missing paragraph text: {md:?}");
        assert!(md.contains("- Point one"), "Missing bullet: {md:?}");
        assert!(md.contains("- Point two"), "Missing bullet: {md:?}");
    }
}

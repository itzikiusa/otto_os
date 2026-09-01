//! HTML → text/markdown extraction. Pure-Rust, regex-free tag walk over the
//! `scraper` (html5ever) parse tree — no headless engine needed for pages
//! that don't require running scripts.

use ego_tree::NodeRef;
use scraper::{Html, Node};

/// Tags `readability` drops entirely (element + all descendants): noise that
/// is never part of the article body.
const NOISE_TAGS: &[&str] = &["script", "style", "nav", "footer", "header", "aside"];

/// Depth cap on the recursive tag walk. Untrusted HTML can nest thousands of
/// elements deep (accidentally or to attack a naive recursive parser); this
/// bounds stack growth instead of trusting the input. Cheap insurance — real
/// article markup never nests anywhere near this deep.
const MAX_WALK_DEPTH: usize = 256;

/// Convert HTML to a markdown-ish rendering: `<h1>`-`<h6>` become `#`..`######`
/// headings, `<li>` becomes a `- ` bullet, block elements get their own line.
/// `<script>`/`<style>` content is always dropped, independent of whether the
/// caller already ran `readability` first.
pub fn html_to_markdown(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();
    walk_markdown(document.tree.root(), &mut out, &["script", "style"], 0);
    normalize(&out)
}

/// Strip script/style/nav/footer/header/aside subtrees and reconstruct the
/// remaining article-ish content as plain HTML (attributes dropped) — feed
/// the result to `html_to_markdown`, or display it directly.
pub fn readability(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();
    walk_html(document.tree.root(), &mut out, NOISE_TAGS, 0);
    out.trim().to_string()
}

fn walk_markdown(node: NodeRef<'_, Node>, out: &mut String, skip: &[&str], depth: usize) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    match node.value() {
        Node::Element(el) => {
            let tag = el.name();
            if skip.contains(&tag) {
                return;
            }
            let heading_level = match tag {
                "h1" => Some(1),
                "h2" => Some(2),
                "h3" => Some(3),
                "h4" => Some(4),
                "h5" => Some(5),
                "h6" => Some(6),
                _ => None,
            };
            if let Some(level) = heading_level {
                out.push('\n');
                out.push_str(&"#".repeat(level));
                out.push(' ');
            } else if tag == "li" {
                out.push_str("\n- ");
            }
            for child in node.children() {
                walk_markdown(child, out, skip, depth + 1);
            }
            if heading_level.is_some() || matches!(tag, "p" | "div" | "li" | "tr") {
                out.push('\n');
            }
            if tag == "br" {
                out.push('\n');
            }
        }
        Node::Text(text) => out.push_str(text),
        _ => {
            for child in node.children() {
                walk_markdown(child, out, skip, depth + 1);
            }
        }
    }
}

fn walk_html(node: NodeRef<'_, Node>, out: &mut String, skip: &[&str], depth: usize) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    match node.value() {
        Node::Element(el) => {
            let tag = el.name();
            if skip.contains(&tag) {
                return;
            }
            out.push('<');
            out.push_str(tag);
            out.push('>');
            for child in node.children() {
                walk_html(child, out, skip, depth + 1);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        Node::Text(text) => out.push_str(text),
        _ => {
            for child in node.children() {
                walk_html(child, out, skip, depth + 1);
            }
        }
    }
}

/// Collapse the ragged whitespace the tag walk leaves behind: trim each
/// line, drop repeated blank lines, drop leading/trailing blanks.
fn normalize(s: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for raw in s.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push("");
            }
        } else {
            lines.push(trimmed);
        }
    }
    while lines.first() == Some(&"") {
        lines.remove(0);
    }
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_strips_scripts_and_keeps_headings() {
        let html = "<html><head><script>evil()</script></head><body><h1>Title</h1><p>Body</p></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title") && md.contains("Body") && !md.contains("evil"));
    }

    #[test]
    fn markdown_keeps_list_items() {
        let html = "<ul><li>One</li><li>Two</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- One") && md.contains("- Two"));
    }

    #[test]
    fn readability_drops_nav_and_footer() {
        let html = "<body><nav>Home | About</nav><article><h1>Story</h1><p>Text</p></article><footer>© 2026</footer></body>";
        let cleaned = readability(html);
        assert!(cleaned.contains("Story") && cleaned.contains("Text"));
        assert!(!cleaned.contains("Home") && !cleaned.contains("2026"));
    }

    #[test]
    fn readability_output_still_converts_to_markdown() {
        let html = "<body><header>Nav</header><article><h2>Headline</h2><p>Paragraph text</p></article></body>";
        let cleaned = readability(html);
        let md = html_to_markdown(&cleaned);
        assert!(md.contains("## Headline") && md.contains("Paragraph text") && !md.contains("Nav"));
    }

    #[test]
    fn deeply_nested_html_does_not_blow_the_stack() {
        // Well past MAX_WALK_DEPTH; both walks must bail out instead of
        // recursing arbitrarily deep on untrusted input.
        let mut html = String::from("<body>");
        for _ in 0..5_000 {
            html.push_str("<div>");
        }
        html.push_str("<h1>Deep</h1>");
        for _ in 0..5_000 {
            html.push_str("</div>");
        }
        html.push_str("</body>");
        // Must simply return without panicking/overflowing the stack.
        let _ = html_to_markdown(&html);
        let _ = readability(&html);
    }
}

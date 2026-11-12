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

/// Most links [`page_markdown`] keeps per page; later ones keep their text
/// only. Bounds what a link-farm page adds to the reader text and to every
/// agent prompt built from it.
const MAX_LINKS: usize = 200;

/// Link state for one page walk: the URL relative `href`s resolve against and
/// how many links were kept so far.
struct Links {
    base: reqwest::Url,
    kept: usize,
}

impl Links {
    /// The absolute http(s) URL an `href` points to, or `None`: empty or
    /// fragment-only (`#top`), a non-web scheme (`javascript:`, `data:`,
    /// `file:`, `mailto:`…), unparseable, or the page's cap already reached.
    fn resolve(&mut self, href: &str) -> Option<String> {
        let href = href.trim();
        if href.is_empty() || href.starts_with('#') || self.kept >= MAX_LINKS {
            return None;
        }
        let url = self.base.join(href).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        self.kept += 1;
        Some(url.into())
    }
}

/// Convert HTML to a markdown-ish rendering: `<h1>`-`<h6>` become `#`..`######`
/// headings, `<li>` becomes a `- ` bullet, block elements get their own line.
/// `<script>`/`<style>` content is always dropped, independent of whether the
/// caller already ran `readability` first.
pub fn html_to_markdown(html: &str) -> String {
    markdown_with(html, &mut None)
}

fn markdown_with(html: &str, links: &mut Option<Links>) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();
    walk_markdown(
        document.tree.root(),
        &mut out,
        &["script", "style"],
        0,
        links,
    );
    normalize(&out)
}

/// The reader text of a page: [`readability`] then [`html_to_markdown`], but
/// keeping the page's links as `[text](<url>)` so documentation can be
/// clicked through. Relative `href`s resolve against `page_url`; a
/// fragment-only link, any non-http(s) target and every link past
/// [`MAX_LINKS`] keeps its text only.
pub fn page_markdown(html: &str, page_url: &str) -> String {
    let Ok(base) = reqwest::Url::parse(page_url) else {
        return html_to_markdown(&readability(html));
    };
    let fresh = |base: &reqwest::Url| {
        Some(Links {
            base: base.clone(),
            kept: 0,
        })
    };
    let cleaned = readability_with(html, &mut fresh(&base));
    markdown_with(&cleaned, &mut fresh(&base))
}

/// Strip script/style/nav/footer/header/aside subtrees and reconstruct the
/// remaining article-ish content as plain HTML (attributes dropped) — feed
/// the result to `html_to_markdown`, or display it directly.
pub fn readability(html: &str) -> String {
    readability_with(html, &mut None)
}

/// [`readability`], keeping each kept link's resolved `href` (see [`Links`]).
fn readability_with(html: &str, links: &mut Option<Links>) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();
    walk_html(document.tree.root(), &mut out, NOISE_TAGS, 0, links);
    out.trim().to_string()
}

fn walk_markdown(
    node: NodeRef<'_, Node>,
    out: &mut String,
    skip: &[&str],
    depth: usize,
    links: &mut Option<Links>,
) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    match node.value() {
        Node::Element(el) => {
            let tag = el.name();
            if skip.contains(&tag) {
                return;
            }
            let href = match (tag, links.as_mut()) {
                ("a", Some(links)) => el.attr("href").and_then(|h| links.resolve(h)),
                _ => None,
            };
            if let Some(href) = href {
                // `[text](<url>)`: the text on one line, its brackets escaped;
                // a link with no text (an image-only link) is dropped.
                let mut text = String::new();
                for child in node.children() {
                    walk_markdown(child, &mut text, skip, depth + 1, links);
                }
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    let text = text.replace('[', "\\[").replace(']', "\\]");
                    out.push_str(&format!("[{text}](<{href}>)"));
                }
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
            } else if matches!(tag, "td" | "th") && follows_cell(node) {
                // Cells of one row stay on one line, separated — not run
                // together ("NameAge").
                out.push_str(" | ");
            }
            for child in node.children() {
                walk_markdown(child, out, skip, depth + 1, links);
            }
            if heading_level.is_some() || is_block(tag) {
                out.push('\n');
            }
            if tag == "br" {
                out.push('\n');
            }
        }
        Node::Text(text) => out.push_str(text),
        _ => {
            for child in node.children() {
                walk_markdown(child, out, skip, depth + 1, links);
            }
        }
    }
}

fn walk_html(
    node: NodeRef<'_, Node>,
    out: &mut String,
    skip: &[&str],
    depth: usize,
    links: &mut Option<Links>,
) {
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
            // The one attribute kept: a link's resolved, web-only `href`.
            let href = match (tag, links.as_mut()) {
                ("a", Some(links)) => el.attr("href").and_then(|h| links.resolve(h)),
                _ => None,
            };
            if let Some(href) = href {
                out.push_str(" href=\"");
                push_escaped(out, &href.replace('"', "%22"));
                out.push('"');
            }
            out.push('>');
            for child in node.children() {
                walk_html(child, out, skip, depth + 1, links);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        // Re-escape: the text node holds DECODED characters, and this output
        // is parsed as HTML again — a tutorial's literal `&lt;div&gt;` must
        // stay text, not turn into (and vanish as) markup.
        Node::Text(text) => push_escaped(out, text),
        _ => {
            for child in node.children() {
                walk_html(child, out, skip, depth + 1, links);
            }
        }
    }
}

/// Append `text` with the three HTML-significant characters escaped.
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

/// Block-level tags that end their own line in the markdown rendering.
fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "li"
            | "tr"
            | "ul"
            | "ol"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "section"
            | "article"
            | "main"
            | "blockquote"
            | "pre"
            | "figure"
            | "figcaption"
            | "hr"
    )
}

/// Whether a `<td>`/`<th>` has an earlier cell in the same row.
fn follows_cell(node: NodeRef<'_, Node>) -> bool {
    node.prev_siblings().any(|sibling| {
        sibling
            .value()
            .as_element()
            .is_some_and(|el| matches!(el.name(), "td" | "th"))
    })
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
    fn escaped_markup_in_page_text_survives_readability() {
        let html = "<body><p>Use &lt;div&gt;hi&lt;/div&gt; &amp; &lt;script&gt;x()&lt;/script&gt;</p></body>";
        let md = html_to_markdown(&readability(html));
        assert!(md.contains("<div>hi</div>"), "{md}");
        assert!(md.contains("<script>x()</script>"), "{md}");
        assert!(md.contains(" & "), "{md}");
    }

    #[test]
    fn table_cells_stay_separated_and_blocks_break() {
        let html =
            "<table><tr><th>Name</th><th>Age</th></tr><tr><td>Ada</td><td>36</td></tr></table>\
                    <blockquote>Quote</blockquote><section>Next</section>";
        let md = html_to_markdown(html);
        assert!(md.contains("Name | Age"), "{md}");
        assert!(md.contains("Ada | 36"), "{md}");
        assert!(md.contains("Quote\nNext"), "{md}");
    }

    const PAGE: &str = "https://docs.example/guide/intro.html";

    #[test]
    fn page_markdown_keeps_absolute_and_resolves_relative_links() {
        let html = "<body><p>See <a href=\"https://other.example/a?b=1&amp;c=2\">the API</a>, \
                    <a href=\"../ref/\">the reference</a> and <a href=\"//cdn.example/x\">a CDN</a>.</p></body>";
        let md = page_markdown(html, PAGE);
        assert!(
            md.contains("[the API](<https://other.example/a?b=1&c=2>)"),
            "{md}"
        );
        assert!(
            md.contains("[the reference](<https://docs.example/ref/>)"),
            "{md}"
        );
        assert!(md.contains("[a CDN](<https://cdn.example/x>)"), "{md}");
    }

    #[test]
    fn page_markdown_drops_dangerous_and_fragment_links_but_keeps_their_text() {
        let html = "<body><p><a href=\"javascript:alert(1)\">run</a> \
                    <a href=\"data:text/html,x\">data</a> <a href=\"file:///etc/passwd\">file</a> \
                    <a href=\"#install\">jump</a> <a href=\"mailto:a@b.example\">mail</a> \
                    <a>bare</a></p></body>";
        let md = page_markdown(html, PAGE);
        assert!(!md.contains("]("), "no link survives: {md}");
        for text in ["run", "data", "file", "jump", "mail", "bare"] {
            assert!(md.contains(text), "{text} kept as text: {md}");
        }
        assert!(!md.contains("javascript") && !md.contains("passwd"), "{md}");
    }

    #[test]
    fn page_markdown_caps_links_and_escapes_link_text() {
        let mut html = String::from("<body><p><a href=\"/x\">[a] <b>b</b>\n c</a></p>");
        for i in 0..(MAX_LINKS + 20) {
            html.push_str(&format!("<p><a href=\"/p{i}\">p{i}</a></p>"));
        }
        let md = page_markdown(&html, PAGE);
        assert!(
            md.contains("[\\[a\\] b c](<https://docs.example/x>)"),
            "{md}"
        );
        assert_eq!(md.matches("](<").count(), MAX_LINKS, "capped");
        assert!(
            md.contains(&format!("p{}", MAX_LINKS + 19)),
            "text past the cap stays"
        );
        // Plain conversion (no page url) keeps link text only, as before.
        assert!(!html_to_markdown(&html).contains("]("));
        assert!(!readability(&html).contains("href"));
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
        let _ = page_markdown(&html, PAGE);
    }
}

//! Markdown note parser: YAML frontmatter, wikilinks + markdown links (with
//! Obsidian tolerance: aliases, `#heading` / `#^block` anchors, embeds),
//! inline `#tags`, headings, word count. Pure functions — no I/O.
//!
//! Code exclusion: links/tags inside fenced code blocks (``` / ~~~) and inline
//! code spans are NOT extracted, matching Obsidian and the OKF link rules.

use crate::types::{Heading, OutgoingLink};

/// Everything the scanner needs from one note's content.
#[derive(Debug, Default)]
pub struct ParsedNote {
    pub title: Option<String>,
    pub okf_type: Option<String>,
    pub description: Option<String>,
    /// Full frontmatter as JSON (unknown keys preserved). `Value::Null` when
    /// there is no frontmatter block.
    pub frontmatter: serde_json::Value,
    pub has_frontmatter: bool,
    /// YAML parse failure of an existing `---` block (fail-soft).
    pub parse_error: bool,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub headings: Vec<Heading>,
    pub links: Vec<OutgoingLink>,
    pub word_count: usize,
}

/// Split a document into `(frontmatter_yaml, body)`. The frontmatter block is
/// a `---` line at byte 0 closed by a `---`/`...` line.
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let rest = content.strip_prefix("---").map(|r| r.strip_prefix('\r').unwrap_or(r));
    let Some(rest) = rest.and_then(|r| r.strip_prefix('\n')) else {
        return (None, content);
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (Some(yaml), body);
        }
        offset += line.len();
    }
    (None, content)
}

fn yaml_to_json(v: serde_yaml::Value) -> serde_json::Value {
    serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)
}

/// A frontmatter value that may be a scalar or a list → Vec<String>.
fn str_list(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::String(s)) => s
            .split(',')
            .map(|t| t.trim().trim_start_matches('#').to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                serde_json::Value::String(s) => Some(s.trim().trim_start_matches('#').to_string()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn str_field(v: Option<&serde_json::Value>) -> Option<String> {
    match v {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// Parse one note. `content` is the full file.
pub fn parse_note(content: &str) -> ParsedNote {
    let mut out = ParsedNote::default();
    let (yaml, body) = split_frontmatter(content);
    if let Some(y) = yaml {
        out.has_frontmatter = true;
        match serde_yaml::from_str::<serde_yaml::Value>(y) {
            Ok(v) if v.is_mapping() => {
                let fm = yaml_to_json(v);
                out.title = str_field(fm.get("title"));
                out.okf_type = str_field(fm.get("type"));
                out.description = str_field(fm.get("description"));
                out.tags = str_list(fm.get("tags"));
                out.aliases = str_list(fm.get("aliases").or_else(|| fm.get("alias")));
                out.frontmatter = fm;
            }
            Ok(_) => {
                // A non-mapping frontmatter (scalar/list) is malformed for our
                // purposes (and E1 under OKF).
                out.parse_error = true;
                out.frontmatter = serde_json::Value::Null;
            }
            Err(_) => {
                out.parse_error = true;
                out.frontmatter = serde_json::Value::Null;
            }
        }
    } else {
        out.frontmatter = serde_json::Value::Null;
    }

    scan_body(body, &mut out);
    out.word_count = body.split_whitespace().count();
    out
}

/// Scan the body for headings, links and inline tags, honoring fenced code
/// blocks and inline code spans.
fn scan_body(body: &str, out: &mut ParsedNote) {
    let mut in_fence: Option<char> = None; // Some('`') / Some('~')
    for (line_no, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        // Fence open/close (``` or ~~~, any info string).
        if let Some(fc) = in_fence {
            if trimmed.starts_with(&fc.to_string().repeat(3)) {
                in_fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = Some('`');
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = Some('~');
            continue;
        }
        // Heading?
        if let Some(h) = parse_heading(line, line_no as u32) {
            out.headings.push(h);
            // Headings still may contain links in the wild; OKF forbids them but
            // Obsidian resolves them — we index them (permissive consumption).
        }
        scan_line(line, line_no as u32, out);
    }
}

fn parse_heading(line: &str, line_no: u32) -> Option<Heading> {
    let s = line.trim_start();
    let hashes = s.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &s[hashes..];
    let text = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t'))?;
    Some(Heading { level: hashes as u8, text: text.trim().to_string(), line: line_no })
}

/// One line: extract wikilinks, markdown links and `#tags`, skipping inline
/// code spans.
fn scan_line(line: &str, _line_no: u32, out: &mut ParsedNote) {
    let b = line.as_bytes();
    let mut i = 0usize;
    let mut in_code = false;
    while i < b.len() {
        let c = b[i];
        if c == b'`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if in_code {
            i += 1;
            continue;
        }
        // %%comment%% — skip to the closing marker (Obsidian comments).
        if c == b'%' && b.get(i + 1) == Some(&b'%') {
            if let Some(end) = line[i + 2..].find("%%") {
                i += 2 + end + 2;
                continue;
            }
            break; // unterminated comment — rest of line is commented
        }
        // Embed / wikilink.
        if c == b'[' && b.get(i + 1) == Some(&b'[') {
            let embed = i > 0 && b[i - 1] == b'!';
            if let Some(end) = line[i + 2..].find("]]") {
                let inner = &line[i + 2..i + 2 + end];
                if let Some(link) = wikilink(inner, embed) {
                    out.links.push(link);
                }
                i += 2 + end + 2;
                continue;
            }
        }
        // Markdown link [text](target)
        if c == b'[' {
            if let Some((link, consumed)) = md_link(&line[i..]) {
                if let Some(l) = link {
                    out.links.push(l);
                }
                i += consumed;
                continue;
            }
        }
        // Inline tag #tag (not a heading — headings start the line and are
        // followed by whitespace; those never reach here with i at a '#'
        // preceded by start-of-line + '#').
        if c == b'#' {
            let at_ok = i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'(' | b',' | b'[');
            if at_ok {
                let rest = &line[i + 1..];
                let tag: String = rest
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '/'))
                    .collect();
                // A pure-numeric token is not a tag (Obsidian rule: needs a letter).
                if !tag.is_empty() && tag.chars().any(|ch| ch.is_alphabetic()) {
                    let taglen = tag.len();
                    if !out.tags.contains(&tag) {
                        out.tags.push(tag);
                    }
                    i += 1 + taglen;
                    continue;
                }
            }
        }
        // Advance one CHARACTER (not byte) to stay on UTF-8 boundaries.
        let ch_len = line[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
        i += ch_len;
    }
}

/// Parse the inside of a `[[...]]`: `target`, `target|alias`,
/// `target#heading`, `target#^block`.
fn wikilink(inner: &str, embed: bool) -> Option<OutgoingLink> {
    if inner.trim().is_empty() {
        return None;
    }
    let (target_part, alias) = match inner.split_once('|') {
        Some((t, a)) => (t, Some(a.trim().to_string())),
        None => (inner, None),
    };
    let (target, anchor) = match target_part.split_once('#') {
        Some((t, a)) => (t, Some(a.trim().to_string())),
        None => (target_part, None),
    };
    let target = target.trim();
    // `[[#heading]]` — self link to a heading: no cross-note edge.
    if target.is_empty() {
        return None;
    }
    Some(OutgoingLink {
        raw_target: target.to_string(),
        dst_path: None,
        kind: if embed { "embed" } else { "wiki" }.to_string(),
        anchor: anchor.filter(|a| !a.is_empty()),
        alias: alias.filter(|a| !a.is_empty()),
    })
}

/// Try to parse a markdown link starting at `[`. Returns
/// `Some((Some(link)|None, consumed_bytes))` when the `[text](target)` shape
/// matched (link None when the target is external/non-note), or `None` when it
/// isn't a markdown link (caller advances one char).
fn md_link(s: &str) -> Option<(Option<OutgoingLink>, usize)> {
    // Find the closing ']' allowing no nested '['.
    let close = s[1..].find(']')? + 1;
    if s.as_bytes().get(close + 1) != Some(&b'(') {
        return None;
    }
    let rest = &s[close + 2..];
    let end = rest.find(')')?;
    let target_raw = rest[..end].trim();
    let text = &s[1..close];
    let consumed = close + 2 + end + 1;

    // External / non-file targets are ignored (not vault edges).
    let lower = target_raw.to_ascii_lowercase();
    if target_raw.is_empty()
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.contains("://")
    {
        return Some((None, consumed));
    }
    // Strip optional angle brackets and a title suffix (`path "title"`).
    let mut t = target_raw.trim_start_matches('<').trim_end_matches('>').trim();
    if let Some(sp) = t.find(" \"") {
        t = t[..sp].trim();
    }
    // Anchor-only link (same file).
    if t.starts_with('#') {
        return Some((None, consumed));
    }
    let (path_part, anchor) = match t.split_once('#') {
        Some((p, a)) => (p, Some(a.to_string())),
        None => (t, None),
    };
    let decoded = percent_decode(path_part);
    if decoded.trim().is_empty() {
        return Some((None, consumed));
    }
    Some((
        Some(OutgoingLink {
            raw_target: decoded,
            dst_path: None,
            kind: "md".to_string(),
            anchor: anchor.filter(|a| !a.is_empty()),
            alias: Some(text.to_string()).filter(|a| !a.is_empty()),
        }),
        consumed,
    ))
}

/// Minimal %XX decoder (enough for `%20` and UTF-8 escapes in note links).
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Rewrite link targets in a note, preserving everything else byte-for-byte.
/// `f(kind, raw_target) -> Option<new_raw>` — `None` keeps the link unchanged.
/// Walks with the same fence/inline-code/comment rules as the extractor, so a
/// "link" inside code is never rewritten. `kind` is `wiki`/`embed`/`md`.
pub fn rewrite_links(content: &str, mut f: impl FnMut(&str, &str) -> Option<String>) -> String {
    let (yaml, body) = split_frontmatter(content);
    let mut out = String::with_capacity(content.len() + 64);
    if let Some(y) = yaml {
        out.push_str("---\n");
        out.push_str(y);
        out.push_str("---\n");
    }
    let mut in_fence: Option<char> = None;
    for line in body.split_inclusive('\n') {
        let (text, nl) = match line.strip_suffix('\n') {
            Some(t) => (t, "\n"),
            None => (line, ""),
        };
        let trimmed = text.trim_start();
        let mut skip_line = false;
        if let Some(fc) = in_fence {
            if trimmed.starts_with(&fc.to_string().repeat(3)) {
                in_fence = None;
            }
            skip_line = true;
        } else if trimmed.starts_with("```") {
            in_fence = Some('`');
            skip_line = true;
        } else if trimmed.starts_with("~~~") {
            in_fence = Some('~');
            skip_line = true;
        }
        if skip_line {
            out.push_str(text);
            out.push_str(nl);
            continue;
        }
        out.push_str(&rewrite_line(text, &mut f));
        out.push_str(nl);
    }
    out
}

fn rewrite_line(line: &str, f: &mut impl FnMut(&str, &str) -> Option<String>) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 32);
    let mut i = 0usize;
    let mut in_code = false;
    while i < b.len() {
        let c = b[i];
        if c == b'`' {
            in_code = !in_code;
            out.push('`');
            i += 1;
            continue;
        }
        if in_code {
            let ch = line[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if c == b'%' && b.get(i + 1) == Some(&b'%') {
            if let Some(end) = line[i + 2..].find("%%") {
                out.push_str(&line[i..i + 2 + end + 2]);
                i += 2 + end + 2;
                continue;
            }
            out.push_str(&line[i..]);
            break;
        }
        if c == b'[' && b.get(i + 1) == Some(&b'[') {
            let embed = i > 0 && b[i - 1] == b'!';
            if let Some(end) = line[i + 2..].find("]]") {
                let inner = &line[i + 2..i + 2 + end];
                out.push_str("[[");
                out.push_str(&rewrite_wikilink_inner(inner, embed, f));
                out.push_str("]]");
                i += 2 + end + 2;
                continue;
            }
        }
        if c == b'[' {
            if let Some((maybe, consumed)) = md_link(&line[i..]) {
                let chunk = &line[i..i + consumed];
                if let Some(link) = maybe {
                    if let Some(new_raw) = f("md", &link.raw_target) {
                        // Rebuild `[text](new)` keeping text + anchor.
                        let close = chunk[1..].find(']').unwrap() + 1;
                        let text = &chunk[1..close];
                        let enc = percent_encode_spaces(&new_raw);
                        let anchor = link.anchor.as_deref().map(|a| format!("#{a}")).unwrap_or_default();
                        out.push('[');
                        out.push_str(text);
                        out.push_str("](");
                        out.push_str(&enc);
                        out.push_str(&anchor);
                        out.push(')');
                        i += consumed;
                        continue;
                    }
                }
                out.push_str(chunk);
                i += consumed;
                continue;
            }
        }
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn rewrite_wikilink_inner(
    inner: &str,
    embed: bool,
    f: &mut impl FnMut(&str, &str) -> Option<String>,
) -> String {
    let kind = if embed { "embed" } else { "wiki" };
    let (target_part, alias) = match inner.split_once('|') {
        Some((t, a)) => (t, Some(a)),
        None => (inner, None),
    };
    let (target, anchor) = match target_part.split_once('#') {
        Some((t, a)) => (t, Some(a)),
        None => (target_part, None),
    };
    let t = target.trim();
    let new = if t.is_empty() { None } else { f(kind, t) };
    let target_out = new.unwrap_or_else(|| target.to_string());
    let mut s = target_out;
    if let Some(a) = anchor {
        s.push('#');
        s.push_str(a);
    }
    if let Some(a) = alias {
        s.push('|');
        s.push_str(a);
    }
    s
}

/// Encode spaces for markdown link targets (the one escape that matters for
/// portability; other bytes are left as typed).
pub fn percent_encode_spaces(s: &str) -> String {
    s.replace(' ', "%20")
}

/// Display title for a note: frontmatter `title`, else the first H1 heading,
/// else — for reserved `index.md`/`log.md`, which OKF forbids frontmatter on —
/// the parent directory name (`log` keeps a " log" suffix so a bundle's index
/// and log nodes stay distinguishable), else the filename stem.
pub fn derive_title(parsed: &ParsedNote, rel: &str) -> String {
    if let Some(t) = &parsed.title {
        return t.clone();
    }
    if let Some(h1) = parsed.headings.iter().find(|h| h.level == 1 && !h.text.trim().is_empty()) {
        return h1.text.trim().to_string();
    }
    let base = rel.rsplit('/').next().unwrap_or(rel);
    let stem = base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base);
    let lower = stem.to_ascii_lowercase();
    if lower == "index" || lower == "log" {
        if let Some(parent) = rel.rsplit('/').nth(1).filter(|p| !p.is_empty()) {
            return if lower == "log" { format!("{parent} log") } else { parent.to_string() };
        }
    }
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_splits_and_parses() {
        let n = parse_note("---\ntype: Service\ntitle: Auth API\ndescription: Issues JWTs.\ntags: [auth, security]\naliases: [Auth]\ncustom_key: kept\n---\n\nBody here.\n");
        assert!(n.has_frontmatter);
        assert!(!n.parse_error);
        assert_eq!(n.okf_type.as_deref(), Some("Service"));
        assert_eq!(n.title.as_deref(), Some("Auth API"));
        assert_eq!(n.description.as_deref(), Some("Issues JWTs."));
        assert_eq!(n.tags, vec!["auth", "security"]);
        assert_eq!(n.aliases, vec!["Auth"]);
        assert_eq!(n.frontmatter.get("custom_key").and_then(|v| v.as_str()), Some("kept"));
        assert_eq!(n.word_count, 2);
    }

    #[test]
    fn frontmatter_multiline_scalar_and_block_tags() {
        let n = parse_note("---\ntype: BigQuery Table\ndescription: This table contains information\n  wrapped onto a second line.\ntags:\n- one\n- two\n---\nx\n");
        assert!(!n.parse_error);
        assert_eq!(n.tags, vec!["one", "two"]);
        assert!(n.description.unwrap().contains("second line"));
    }

    #[test]
    fn broken_yaml_is_fail_soft() {
        let n = parse_note("---\n: : : nope [\n---\nbody\n");
        assert!(n.has_frontmatter);
        assert!(n.parse_error);
    }

    #[test]
    fn no_frontmatter() {
        let n = parse_note("# Just a heading\n\ntext\n");
        assert!(!n.has_frontmatter);
        assert!(!n.parse_error);
        assert_eq!(n.headings.len(), 1);
    }

    #[test]
    fn wikilink_forms() {
        let n = parse_note("See [[Notes/Target]] and [[Other|the alias]] and [[Third#Heading]] and [[Fourth#^blk42]] plus ![[img.png]].\n");
        let l = &n.links;
        assert_eq!(l.len(), 5);
        assert_eq!(l[0].raw_target, "Notes/Target");
        assert_eq!(l[1].alias.as_deref(), Some("the alias"));
        assert_eq!(l[2].anchor.as_deref(), Some("Heading"));
        assert_eq!(l[3].anchor.as_deref(), Some("^blk42"));
        assert_eq!(l[4].kind, "embed");
    }

    #[test]
    fn markdown_links_local_and_external() {
        let n = parse_note("[customers](/tables/customers.md) then [users](users.md) then [ext](https://example.com) and [enc](my%20note.md) and [anchored](other.md#sec).\n");
        let l = &n.links;
        assert_eq!(l.len(), 4, "external link must be skipped: {l:?}");
        assert_eq!(l[0].raw_target, "/tables/customers.md");
        assert_eq!(l[0].kind, "md");
        assert_eq!(l[0].alias.as_deref(), Some("customers"));
        assert_eq!(l[1].raw_target, "users.md");
        assert_eq!(l[2].raw_target, "my note.md");
        assert_eq!(l[3].anchor.as_deref(), Some("sec"));
    }

    #[test]
    fn code_is_excluded() {
        let n = parse_note("```\n[[NotALink]] #nottag\n```\nInline `[[also not]]` but [[Real]] and #real.\n~~~\n[[fence2]]\n~~~\n");
        assert_eq!(n.links.len(), 1);
        assert_eq!(n.links[0].raw_target, "Real");
        assert_eq!(n.tags, vec!["real"]);
    }

    #[test]
    fn comments_are_excluded() {
        let n = parse_note("%%[[hidden]]%% visible [[Shown]] %% trailing [[gone]]\n");
        assert_eq!(n.links.len(), 1);
        assert_eq!(n.links[0].raw_target, "Shown");
    }

    #[test]
    fn tags_inline_nested_and_not_headings() {
        let n = parse_note("# Heading not tag\nwork on #project/alpha and (#beta) but not#inline nor #123\n");
        assert_eq!(n.tags, vec!["project/alpha", "beta"]);
    }

    #[test]
    fn headings_collected() {
        let n = parse_note("# One\ntext\n## Two\n```\n# not a heading\n```\n### Three\n");
        assert_eq!(
            n.headings.iter().map(|h| (h.level, h.text.as_str())).collect::<Vec<_>>(),
            vec![(1, "One"), (2, "Two"), (3, "Three")]
        );
    }

    #[test]
    fn rewrite_preserves_everything_else() {
        let src = "---\ntitle: T\n---\nA [[Old Note|alias]] and [[Old Note#H]] and [txt](old%20note.md#s) here.\n```\n[[Old Note]] untouched\n```\nInline `[[Old Note]]` untouched, ![[Old Note]] embed.\n";
        let out = rewrite_links(src, |kind, raw| {
            if raw.eq_ignore_ascii_case("old note") || raw.eq_ignore_ascii_case("old note.md") {
                Some(if kind == "md" { "new note.md".into() } else { "New Note".into() })
            } else {
                None
            }
        });
        assert!(out.contains("[[New Note|alias]]"), "{out}");
        assert!(out.contains("[[New Note#H]]"), "{out}");
        assert!(out.contains("[txt](new%20note.md#s)"), "{out}");
        assert!(out.contains("```\n[[Old Note]] untouched\n```"), "{out}");
        assert!(out.contains("`[[Old Note]]` untouched"), "{out}");
        assert!(out.contains("![[New Note]] embed"), "{out}");
        assert!(out.starts_with("---\ntitle: T\n---\n"), "{out}");
    }

    #[test]
    fn rewrite_noop_is_bytes_identical() {
        let src = "---\na: 1\n---\nBody [[Keep]] and [x](keep.md) and #tag.\n\nmore\n";
        let out = rewrite_links(src, |_, _| None);
        assert_eq!(src, out);
    }

    #[test]
    fn unicode_safe() {
        let n = parse_note("עברית [[קישור|כינוי]] וגם #תג-עברי טקסט\n");
        assert_eq!(n.links[0].raw_target, "קישור");
        assert_eq!(n.tags, vec!["תג-עברי"]);
    }

    #[test]
    fn derive_title_frontmatter_wins() {
        let n = parse_note("---\ntitle: Auth API\n---\n# Ignored H1\n");
        assert_eq!(derive_title(&n, "services/auth.md"), "Auth API");
    }

    #[test]
    fn derive_title_h1_fallback_for_reserved_index() {
        let n = parse_note("# koala-smartsoft-go\n\nGroove reverse-integration host.\n");
        assert_eq!(derive_title(&n, "koala-smartsoft-go/index.md"), "koala-smartsoft-go");
        let nested = parse_note("# Endpoints\n\n* [x](x.md)\n");
        assert_eq!(derive_title(&nested, "koala-smartsoft-go/endpoints/index.md"), "Endpoints");
    }

    #[test]
    fn derive_title_parent_dir_for_reserved_without_h1() {
        let log = parse_note("## 2026-07-20\n\n* entry\n");
        assert_eq!(derive_title(&log, "koala-smartsoft-go/log.md"), "koala-smartsoft-go log");
        let idx = parse_note("just text, no headings\n");
        assert_eq!(derive_title(&idx, "koala-smartsoft-go/index.md"), "koala-smartsoft-go");
    }

    #[test]
    fn derive_title_stem_last_resort() {
        let root = parse_note("no headings\n");
        assert_eq!(derive_title(&root, "index.md"), "index");
        let concept = parse_note("no frontmatter, no h1\n");
        assert_eq!(derive_title(&concept, "services/auth.md"), "auth");
    }
}

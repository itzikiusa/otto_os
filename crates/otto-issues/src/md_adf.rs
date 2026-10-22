//! Markdown → Atlassian Document Format (ADF) for Jira Cloud (REST v3).
//!
//! Jira Cloud takes descriptions and comments as ADF, not Markdown. This is a
//! small GFM-flavoured parser (no external crate) that builds a block tree,
//! then emits ADF that respects the schema's nesting rules — a construct a
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

// ───────────────────────────── block parsing ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading(u8, String),
    /// Raw lines; each line break is a `hardBreak`.
    Para(Vec<String>),
    Code(String, String),
    Rule,
    Quote(Vec<Block>),
    /// ADF `panelType` + body.
    Panel(&'static str, Vec<Block>),
    List(List),
    /// Rows of raw cell text; row 0 is the header. Already rectangular.
    Table(Vec<Vec<String>>),
}

#[derive(Debug, Clone, PartialEq)]
struct List {
    ordered: bool,
    start: u64,
    items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
struct Item {
    /// `Some(done)` for a `- [ ]` / `- [x]` task item.
    task: Option<bool>,
    blocks: Vec<Block>,
}

/// CRLF → LF, leading tabs → 4 spaces, control characters dropped.
fn normalize(md: &str) -> Vec<String> {
    md.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|l| {
            let lead = l.len() - l.trim_start_matches([' ', '\t']).len();
            let (head, rest) = l.split_at(lead);
            let mut s = head.replace('\t', "    ");
            s.extend(rest.chars().filter(|c| !c.is_control() || *c == '\t'));
            s
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// `(fence char run, info string)` for an opening code fence.
fn fence_open(line: &str) -> Option<(String, String)> {
    if indent(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = t.chars().take_while(|c| *c == ch).count();
    if run < 3 {
        return None;
    }
    let info = t[run..].trim();
    if ch == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().unwrap_or("").to_string();
    Some((t[..run].to_string(), lang))
}

fn heading(line: &str) -> Option<(u8, String)> {
    if indent(line) > 3 {
        return None;
    }
    let t = line.trim();
    let level = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    // Drop an optional closing `###` sequence.
    let rest = rest.trim();
    let trimmed = rest.trim_end_matches('#');
    let text = if trimmed.is_empty() || trimmed.ends_with(' ') {
        trimmed.trim_end()
    } else {
        rest
    };
    Some((level as u8, text.to_string()))
}

fn is_rule(line: &str) -> bool {
    if indent(line) > 3 {
        return false;
    }
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && ['-', '*', '_'].iter().any(|ch| t.chars().all(|c| c == *ch))
}

struct Marker {
    ordered: bool,
    start: u64,
    /// Column where the item's content starts.
    content_col: usize,
}

fn list_marker(line: &str) -> Option<Marker> {
    let ind = indent(line);
    let rest = &line[ind..];
    let (ordered, start, mlen) = if rest.starts_with(['-', '*', '+']) {
        (false, 1, 1)
    } else {
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let delim = rest[digits..].chars().next()?;
        if delim != '.' && delim != ')' {
            return None;
        }
        (true, rest[..digits].parse().unwrap_or(1), digits + 1)
    };
    let after = &rest[mlen..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let spaces = after.len() - after.trim_start_matches(' ').len();
    let pad = if after.trim().is_empty() || spaces > 4 {
        1
    } else {
        spaces
    };
    Some(Marker {
        ordered,
        start,
        content_col: ind + mlen + pad,
    })
}

/// Split a table row into raw cells; `\|` is a literal pipe (GFM), every
/// other escape is left for the inline parser.
fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") {
        &t[..t.len() - 1]
    } else {
        t
    };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn is_table_delim(line: &str) -> bool {
    if !line.contains('|') || !line.contains('-') {
        return false;
    }
    split_cells(line).iter().all(|c| {
        let c = c.trim();
        let inner = c.strip_prefix(':').unwrap_or(c);
        let inner = inner.strip_suffix(':').unwrap_or(inner);
        !inner.is_empty() && inner.chars().all(|ch| ch == '-')
    })
}

fn table_start(lines: &[String], i: usize) -> bool {
    lines[i].contains('|') && lines.get(i + 1).is_some_and(|l| is_table_delim(l))
}

fn starts_block(lines: &[String], i: usize) -> bool {
    let l = &lines[i];
    fence_open(l).is_some()
        || heading(l).is_some()
        || is_rule(l)
        || (indent(l) <= 3 && l.trim_start().starts_with('>'))
        || list_marker(l).is_some()
        || table_start(lines, i)
}

/// GitHub alert marker → ADF panel type, colour-matched (NOTE is blue →
/// `info`, IMPORTANT purple → `note`, TIP green → `success`, WARNING yellow →
/// `warning`, CAUTION red → `error`).
fn callout(first: &str) -> Option<(&'static str, String)> {
    let t = first.trim();
    let close = t.find(']')?;
    let kind = t.strip_prefix("[!")?.get(..close - 2)?;
    let panel = match kind.to_ascii_uppercase().as_str() {
        "NOTE" | "INFO" => "info",
        "IMPORTANT" => "note",
        "TIP" | "SUCCESS" => "success",
        "WARNING" => "warning",
        "CAUTION" | "DANGER" | "ERROR" => "error",
        _ => return None,
    };
    Some((panel, t[close + 1..].trim().to_string()))
}

/// Container / span nesting followed before the rest is kept as plain text —
/// a `>>>>…` or `*a *b …` thousands deep must not overflow the stack.
const DEPTH_MAX: usize = 32;

fn parse_blocks(lines: &[String], depth: usize) -> Vec<Block> {
    if depth > DEPTH_MAX {
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![Block::Para(text)]
        };
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if is_blank(line) {
            i += 1;
            continue;
        }
        if let Some((fence, lang)) = fence_open(line) {
            let strip = indent(line);
            let ch = fence.chars().next().unwrap_or('`');
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim_start();
                let run = t.chars().take_while(|c| *c == ch).count();
                if run >= fence.len() && t[run..].trim().is_empty() {
                    i += 1;
                    break;
                }
                let l = &lines[i];
                let cut = indent(l).min(strip);
                body.push(l[cut..].to_string());
                i += 1;
            }
            out.push(Block::Code(lang, body.join("\n")));
            continue;
        }
        if let Some((level, text)) = heading(line) {
            out.push(Block::Heading(level, text));
            i += 1;
            continue;
        }
        if is_rule(line) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if table_start(lines, i) {
            let mut rows = vec![split_cells(line)];
            i += 2;
            while i < lines.len() && !is_blank(&lines[i]) && lines[i].contains('|') {
                rows.push(split_cells(&lines[i]));
                i += 1;
            }
            let width = rows.iter().map(Vec::len).max().unwrap_or(1);
            for r in &mut rows {
                r.resize(width, String::new());
            }
            out.push(Block::Table(rows));
            continue;
        }
        if indent(line) <= 3 && line.trim_start().starts_with('>') {
            let mut inner = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let t = lines[i].trim_start()[1..].to_string();
                inner.push(t.strip_prefix(' ').map(str::to_string).unwrap_or(t));
                i += 1;
            }
            match inner.first().and_then(|f| callout(f)) {
                Some((panel, rest)) => {
                    let mut body = vec![rest];
                    body.extend_from_slice(&inner[1..]);
                    out.push(Block::Panel(panel, parse_blocks(&body, depth + 1)));
                }
                None => out.push(Block::Quote(parse_blocks(&inner, depth + 1))),
            }
            continue;
        }
        if list_marker(line).is_some() {
            let (list, next) = parse_list(lines, i, depth);
            out.push(Block::List(list));
            i = next;
            continue;
        }
        // Paragraph (or a setext heading).
        let mut para = vec![line.trim().to_string()];
        i += 1;
        while i < lines.len() && !is_blank(&lines[i]) {
            let t = lines[i].trim();
            if !t.is_empty() && t.chars().all(|c| c == '=') {
                out.push(Block::Heading(1, para.join(" ")));
                para.clear();
                i += 1;
                break;
            }
            if t.len() >= 2 && t.chars().all(|c| c == '-') {
                out.push(Block::Heading(2, para.join(" ")));
                para.clear();
                i += 1;
                break;
            }
            if starts_block(lines, i) {
                break;
            }
            para.push(t.to_string());
            i += 1;
        }
        if !para.is_empty() {
            out.push(Block::Para(para));
        }
    }
    out
}

/// Parse a list starting at `lines[start]`; returns it and the next line index.
fn parse_list(lines: &[String], start: usize, depth: usize) -> (List, usize) {
    let first = list_marker(&lines[start]).expect("caller checked the marker");
    let base = indent(&lines[start]);
    let mut list = List {
        ordered: first.ordered,
        start: first.start,
        items: Vec::new(),
    };
    let mut i = start;
    while let Some(m) = list_marker(&lines[i]) {
        let ind = indent(&lines[i]);
        if m.ordered != list.ordered || ind > base + 1 || ind + 1 < base {
            break;
        }
        let head = lines[i].get(m.content_col..).unwrap_or("").to_string();
        let mut body = vec![head];
        let mut j = i + 1;
        while j < lines.len() {
            let l = &lines[j];
            if is_blank(l) {
                let next = (j..lines.len()).find(|&k| !is_blank(&lines[k]));
                match next {
                    Some(k) if indent(&lines[k]) > base => {
                        body.extend((j..k).map(|_| String::new()));
                        j = k;
                        continue;
                    }
                    _ => break,
                }
            }
            let ind_j = indent(l);
            if ind_j > base {
                body.push(l[ind_j.min(m.content_col)..].to_string());
            } else if !starts_block(lines, j) && body.last().is_some_and(|b| !b.trim().is_empty()) {
                // Lazy paragraph continuation.
                body.push(l.trim().to_string());
            } else {
                break;
            }
            j += 1;
        }
        let task = task_marker(&body[0]);
        if let Some((_, rest)) = &task {
            body[0] = rest.clone();
        }
        list.items.push(Item {
            task: task.map(|(done, _)| done),
            blocks: parse_blocks(&body, depth + 1),
        });
        // Blank lines between sibling items keep the list going.
        let mut k = j;
        while k < lines.len() && is_blank(&lines[k]) {
            k += 1;
        }
        if k < lines.len() && indent(&lines[k]) <= base + 1 && list_marker(&lines[k]).is_some() {
            i = k;
            continue;
        }
        i = j;
        break;
    }
    (list, i.max(start + 1))
}

fn task_marker(head: &str) -> Option<(bool, String)> {
    let t = head.trim_start();
    for (p, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(rest) = t.strip_prefix(p) {
            if rest.is_empty() || rest.starts_with(' ') {
                return Some((done, rest.trim_start().to_string()));
            }
        }
    }
    None
}

// ───────────────────────────── inline parsing ────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Mark {
    Strong,
    Em,
    Strike,
    Code,
    Link(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Inl {
    Text(String, Vec<Mark>),
    Break,
}

fn with(marks: &[Mark], add: &[Mark]) -> Vec<Mark> {
    let mut m = marks.to_vec();
    for a in add {
        // One link per span: an inner link replaces an outer one.
        if matches!(a, Mark::Link(_)) {
            m.retain(|x| !matches!(x, Mark::Link(_)));
        }
        if !m.contains(a) {
            m.push(a.clone());
        }
    }
    m.sort();
    m
}

fn has_link(marks: &[Mark]) -> bool {
    marks.iter().any(|m| matches!(m, Mark::Link(_)))
}

/// Inline Markdown → spans.
fn inlines(s: &str) -> Vec<Inl> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    parse_inline(&chars, &[], &mut out, 0);
    out
}

/// End of the backtick code span opening at `i` (index of its closing run),
/// with the run length, or None when it never closes.
fn code_span(c: &[char], i: usize) -> Option<(usize, usize)> {
    let run = c[i..].iter().take_while(|&&x| x == '`').count();
    let mut j = i + run;
    while j < c.len() {
        if c[j] == '`' {
            let r = c[j..].iter().take_while(|&&x| x == '`').count();
            if r == run {
                return Some((j, run));
            }
            j += r;
        } else {
            j += 1;
        }
    }
    None
}

/// Index of the `close` char matching the `open` at `i`, skipping escapes and
/// code spans and honouring nesting.
fn matching(c: &[char], i: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = i;
    while j < c.len() {
        match c[j] {
            '\\' => j += 1,
            '`' => {
                if let Some((end, run)) = code_span(c, j) {
                    j = end + run - 1;
                }
            }
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Start of the delimiter run that closes a `size`-long `ch` opener whose
/// content starts at `from`. A closing run longer than `size` closes with its
/// first `size` chars (the rest close an outer span, as in
/// `**bold *and it***`); a nested opener's whole span is skipped so its
/// closer is not mistaken for ours.
///
/// `memo` caches each `(from, size, ch)` answer and nesting stops at
/// [`NEST_MAX`]: without them a run of unmatched openers (`*a *b *c …`)
/// re-scanned every suffix and took exponential time.
fn closer(
    c: &[char],
    from: usize,
    ch: char,
    size: usize,
    memo: &mut Memo,
    depth: usize,
) -> Option<usize> {
    if let Some(hit) = memo.get(&(from, size, ch)) {
        return *hit;
    }
    let found = closer_scan(c, from, ch, size, memo, depth);
    memo.insert((from, size, ch), found);
    found
}

/// Closer lookups already answered, keyed by `(from, size, delimiter)`.
type Memo = std::collections::HashMap<(usize, usize, char), Option<usize>>;

/// How deep nested emphasis spans are followed while looking for a closer.
const NEST_MAX: usize = 8;

fn closer_scan(
    c: &[char],
    from: usize,
    ch: char,
    size: usize,
    memo: &mut Memo,
    depth: usize,
) -> Option<usize> {
    let mut j = from;
    while j < c.len() {
        match c[j] {
            '\\' => j += 2,
            '`' => match code_span(c, j) {
                Some((end, run)) => j = end + run,
                None => j += 1,
            },
            x if x == ch => {
                let r = c[j..].iter().take_while(|&&y| y == ch).count();
                let next = c.get(j + r);
                let closes = j > from
                    && !c[j - 1].is_whitespace()
                    && (ch != '_' || next.is_none_or(|n| !n.is_alphanumeric()));
                if closes && r >= size {
                    return Some(j);
                }
                if depth < NEST_MAX && next.is_some_and(|n| !n.is_whitespace()) {
                    if let Some(e) = closer(c, j + r, ch, r, memo, depth + 1) {
                        j = e + r;
                        continue;
                    }
                }
                j += r;
            }
            _ => j += 1,
        }
    }
    None
}

/// `[text](dest "title")` starting at `i` (the `[`): (text, dest, end index).
fn link_at(c: &[char], i: usize) -> Option<(Vec<char>, String, usize)> {
    let close = matching(c, i, '[', ']')?;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = matching(c, close + 1, '(', ')')?;
    let raw: String = c[close + 2..end].iter().collect();
    let raw = raw.trim();
    let dest = match raw.find([' ', '\t']) {
        Some(p) => &raw[..p],
        None => raw,
    };
    let dest = dest.trim_start_matches('<').trim_end_matches('>');
    Some((c[i + 1..close].to_vec(), dest.to_string(), end))
}

fn bare_url_len(c: &[char], i: usize) -> Option<usize> {
    let rest: String = c[i..c.len().min(i + 8)].iter().collect();
    if !(rest.starts_with("http://") || rest.starts_with("https://")) {
        return None;
    }
    if i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '/') {
        return None;
    }
    let mut n = c[i..]
        .iter()
        .take_while(|x| !x.is_whitespace() && **x != '<')
        .count();
    // Trailing punctuation belongs to the sentence; a `)` only when unbalanced.
    while n > 0 {
        let last = c[i + n - 1];
        let url = &c[i..i + n];
        let unbalanced = last == ')'
            && url.iter().filter(|&&x| x == ')').count()
                > url.iter().filter(|&&x| x == '(').count();
        if ".,;:!?\"'".contains(last) || unbalanced {
            n -= 1;
        } else {
            break;
        }
    }
    (n > "https://".len() - 1).then_some(n)
}

fn parse_inline(c: &[char], marks: &[Mark], out: &mut Vec<Inl>, depth: usize) {
    if depth > DEPTH_MAX {
        out.push(Inl::Text(c.iter().collect(), marks.to_vec()));
        return;
    }
    let mut memo = Memo::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Inl>| {
        if !buf.is_empty() {
            out.push(Inl::Text(std::mem::take(buf), marks.to_vec()));
        }
    };
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        // Escapes.
        if ch == '\\' && c.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            buf.push(c[i + 1]);
            i += 2;
            continue;
        }
        // Code span: only a link may share its marks (ADF rule).
        if ch == '`' {
            if let Some((end, run)) = code_span(c, i) {
                let mut code: String = c[i + run..end].iter().collect();
                code = code.replace('\n', " ");
                if code.len() > 2 && code.starts_with(' ') && code.ends_with(' ') {
                    code = code[1..code.len() - 1].to_string();
                }
                flush(&mut buf, out);
                let mut m: Vec<Mark> = marks
                    .iter()
                    .filter(|x| matches!(x, Mark::Link(_)))
                    .cloned()
                    .collect();
                m.insert(0, Mark::Code);
                out.push(Inl::Text(code, m));
                i = end + run;
                continue;
            }
            let run = c[i..].iter().take_while(|&&x| x == '`').count();
            buf.extend(&c[i..i + run]);
            i += run;
            continue;
        }
        // `<br>` and `<https://autolink>`.
        if ch == '<' {
            if let Some(end) = c[i..].iter().position(|&x| x == '>') {
                let tag: String = c[i + 1..i + end].iter().collect();
                let t = tag.trim().trim_end_matches('/').trim().to_ascii_lowercase();
                if t == "br" {
                    flush(&mut buf, out);
                    out.push(Inl::Break);
                    i += end + 1;
                    continue;
                }
                let is_url = ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|p| tag.starts_with(p));
                if is_url && !tag.contains(char::is_whitespace) {
                    flush(&mut buf, out);
                    out.push(Inl::Text(tag.clone(), with(marks, &[Mark::Link(tag)])));
                    i += end + 1;
                    continue;
                }
            }
        }
        // Links and images (an image has no upload path: it becomes a link).
        if ch == '[' || (ch == '!' && c.get(i + 1) == Some(&'[')) {
            let at = if ch == '!' { i + 1 } else { i };
            if let Some((text, dest, end)) = link_at(c, at) {
                flush(&mut buf, out);
                let text = if text.is_empty() {
                    dest.chars().collect()
                } else {
                    text
                };
                if dest.is_empty() {
                    parse_inline(&text, marks, out, depth + 1);
                } else {
                    parse_inline(&text, &with(marks, &[Mark::Link(dest)]), out, depth + 1);
                }
                i = end + 1;
                continue;
            }
        }
        // Emphasis / strong / strike.
        if ch == '*' || ch == '_' || ch == '~' {
            let run = c[i..].iter().take_while(|&&x| x == ch).count();
            let opens = c.get(i + run).is_some_and(|n| !n.is_whitespace())
                && (ch != '_' || i == 0 || !c[i - 1].is_alphanumeric());
            let sizes: &[usize] = match (ch, run) {
                ('~', 2) => &[2],
                ('~', _) => &[],
                (_, 1) => &[1],
                (_, 2) => &[2, 1],
                _ => &[3, 2, 1],
            };
            if opens {
                let found = sizes
                    .iter()
                    .find_map(|&s| closer(c, i + run, ch, s, &mut memo, 0).map(|end| (s, end)));
                if let Some((s, end)) = found {
                    // Opener chars beyond the matched size stay literal.
                    buf.extend(&c[i..i + run - s]);
                    flush(&mut buf, out);
                    let add: &[Mark] = match (ch, s) {
                        ('~', _) => &[Mark::Strike],
                        (_, 1) => &[Mark::Em],
                        (_, 2) => &[Mark::Strong],
                        _ => &[Mark::Strong, Mark::Em],
                    };
                    parse_inline(&c[i + run..end], &with(marks, add), out, depth + 1);
                    i = end + s;
                    continue;
                }
            }
            buf.extend(&c[i..i + run]);
            i += run;
            continue;
        }
        if !has_link(marks) {
            if let Some(n) = bare_url_len(c, i) {
                flush(&mut buf, out);
                let url: String = c[i..i + n].iter().collect();
                out.push(Inl::Text(url.clone(), with(marks, &[Mark::Link(url)])));
                i += n;
                continue;
            }
        }
        buf.push(ch);
        i += 1;
    }
    flush(&mut buf, out);
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

/// A paragraph's lines → spans joined by `hardBreak`s (a trailing `\` or two
/// trailing spaces is just a marker for the same break).
fn para_spans(lines: &[String]) -> Vec<Inl> {
    let mut spans = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        if n > 0 {
            spans.push(Inl::Break);
        }
        let l = l.trim_end();
        let l = l.strip_suffix('\\').unwrap_or(l);
        spans.extend(inlines(l.trim_start()));
    }
    spans
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

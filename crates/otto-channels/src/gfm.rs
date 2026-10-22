//! GitHub-flavoured Markdown → channel dialects (integrations-2-03).
//!
//! Claude / Codex answer in GFM (`**bold**`, `[label](url)`, `## headings`,
//! pipe tables, fenced code). Neither channel renders that: Slack mrkdwn shows
//! the markup literally, and Telegram's legacy `Markdown` parse mode rejects
//! common agent text (an unpaired `_` in `snake_case`, `**`) so the reply fell
//! back to raw markup. `send_formatted` runs the text through one of these
//! converters first:
//!
//! - [`to_slack_mrkdwn`]: `**x**`/`*x*` → `*x*`, `_x_` → `_x_`, `~~x~~` →
//!   `~x~`, `[l](u)` → `<u|l>`, headings → bold lines, bullets → `•`, tables →
//!   an aligned code block. No escaping, so a caller that already writes
//!   Slack's `<url|label>` keeps working.
//! - [`to_telegram_html`]: the same structure as `parse_mode: "HTML"` with
//!   every text run HTML-escaped and every tag balanced (HTML mode is far more
//!   forgiving than legacy Markdown). A single `*x*` renders bold, as legacy
//!   Markdown did for the callers that wrote Telegram-style text.
//!
//! `_` only opens/closes emphasis at a word boundary, so `snake_case` and
//! `__init__`-style identifiers stay literal. Code spans and fenced blocks are
//! never reformatted.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Slack,
    TelegramHtml,
}

#[derive(Clone, Copy)]
enum Style {
    Bold,
    Italic,
    Strike,
}

impl Dialect {
    fn esc(self, s: &str) -> String {
        match self {
            Dialect::Slack => s.to_string(),
            Dialect::TelegramHtml => html_escape(s),
        }
    }
    fn wrap(self, style: Style, inner: &str) -> String {
        match (self, style) {
            (Dialect::Slack, Style::Bold) => format!("*{inner}*"),
            (Dialect::Slack, Style::Italic) => format!("_{inner}_"),
            (Dialect::Slack, Style::Strike) => format!("~{inner}~"),
            (Dialect::TelegramHtml, Style::Bold) => format!("<b>{inner}</b>"),
            (Dialect::TelegramHtml, Style::Italic) => format!("<i>{inner}</i>"),
            (Dialect::TelegramHtml, Style::Strike) => format!("<s>{inner}</s>"),
        }
    }
    fn code(self, code: &str) -> String {
        match self {
            Dialect::Slack => format!("`{code}`"),
            Dialect::TelegramHtml => format!("<code>{}</code>", html_escape(code)),
        }
    }
    fn link(self, url: &str, label: &[char]) -> String {
        match self {
            // Slack can't format inside a link label, and `|` / `>` would end
            // the `<url|label>` token early.
            Dialect::Slack => {
                let label: String = label
                    .iter()
                    .filter(|c| !matches!(c, '*' | '`' | '|' | '<' | '>'))
                    .collect();
                if label.trim().is_empty() || label == url {
                    format!("<{url}>")
                } else {
                    format!("<{url}|{label}>")
                }
            }
            Dialect::TelegramHtml => format!(
                "<a href=\"{}\">{}</a>",
                html_escape(url).replace('"', "&quot;"),
                inline(self, label)
            ),
        }
    }
    fn block(self, lines: &[String]) -> String {
        let body = lines.join("\n");
        match self {
            Dialect::Slack => format!("```\n{body}\n```"),
            Dialect::TelegramHtml => format!("<pre>{}</pre>", html_escape(&body)),
        }
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// GFM → Slack mrkdwn (for `chat.postMessage` with `mrkdwn: true`).
pub fn to_slack_mrkdwn(md: &str) -> String {
    convert(Dialect::Slack, md)
}

/// GFM → Telegram HTML (for `sendMessage` with `parse_mode: "HTML"`).
pub fn to_telegram_html(md: &str) -> String {
    convert(Dialect::TelegramHtml, md)
}

fn convert(d: Dialect, md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        // Fenced code block: verbatim (the language tag is dropped — Slack
        // would print it as the first line).
        if t.starts_with("```") || t.starts_with("~~~") {
            let fence = &t[..3];
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                body.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // the closing fence (absent at EOF is fine)
            out.push(d.block(&body));
            continue;
        }
        // Pipe table: a header row followed by a `|---|` separator row.
        if t.contains('|') && lines.get(i + 1).is_some_and(|n| is_table_sep(n.trim())) {
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(lines[i]);
                i += 1;
            }
            out.push(d.block(&render_table(&rows)));
            continue;
        }
        out.push(convert_line(d, lines[i]));
        i += 1;
    }
    out.join("\n")
}

fn is_table_sep(t: &str) -> bool {
    t.contains('|')
        && t.contains('-')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Pipe-table rows → aligned plain-text lines (the separator row becomes a
/// dashed rule). Emphasis/backticks are stripped: a code block shows them raw.
fn render_table(rows: &[&str]) -> Vec<String> {
    let cells: Vec<Option<Vec<String>>> = rows
        .iter()
        .map(|r| {
            let t = r.trim();
            if is_table_sep(t) {
                return None;
            }
            let t = t.strip_prefix('|').unwrap_or(t);
            let t = t.strip_suffix('|').unwrap_or(t);
            Some(
                t.split('|')
                    .map(|c| c.trim().replace("**", "").replace('`', ""))
                    .collect(),
            )
        })
        .collect();
    let cols = cells.iter().flatten().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for row in cells.iter().flatten() {
        for (k, c) in row.iter().enumerate() {
            widths[k] = widths[k].max(c.chars().count());
        }
    }
    cells
        .iter()
        .map(|row| match row {
            None => widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("-+-"),
            Some(row) => (0..cols)
                .map(|k| {
                    let c = row.get(k).map(String::as_str).unwrap_or("");
                    let pad = widths[k].saturating_sub(c.chars().count());
                    format!("{c}{}", " ".repeat(pad))
                })
                .collect::<Vec<_>>()
                .join(" | ")
                .trim_end()
                .to_string(),
        })
        .collect()
}

fn convert_line(d: Dialect, line: &str) -> String {
    let t = line.trim_start();
    let indent = &line[..line.len() - t.len()];
    // ATX heading → a bold line.
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && (t.len() == hashes || t[hashes..].starts_with(' ')) {
        let text = t[hashes..]
            .trim()
            .trim_end_matches('#')
            .trim()
            .replace("**", "");
        if text.is_empty() {
            return String::new();
        }
        let chars: Vec<char> = text.chars().collect();
        return d.wrap(Style::Bold, &inline(d, &chars));
    }
    // Thematic break.
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
    {
        return "──────────".into();
    }
    // Bullet → `•` (a leading `* ` would otherwise read as emphasis).
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(marker) {
            let chars: Vec<char> = rest.chars().collect();
            return format!("{indent}• {}", inline(d, &chars));
        }
    }
    let chars: Vec<char> = line.chars().collect();
    inline(d, &chars)
}

fn inline(d: Dialect, s: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == '`' {
            let run = s[i..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_backtick_close(s, i + run, run) {
                let code: String = s[i + run..end].iter().collect();
                out.push_str(&d.code(code.trim()));
                i = end + run;
            } else {
                out.push_str(&"`".repeat(run));
                i += run;
            }
            continue;
        }
        if c == '[' || (c == '!' && s.get(i + 1) == Some(&'[')) {
            let start = if c == '!' { i + 1 } else { i };
            if let Some((label, url, next)) = parse_link(s, start) {
                out.push_str(&d.link(&url, label));
                i = next;
                continue;
            }
        }
        if let Some((style, dl)) = delim_at(s, i) {
            if let Some(end) = find_close(s, i, dl) {
                let inner = inline(d, &s[i + dl.len()..end]);
                out.push_str(&d.wrap(style, &inner));
                i = end + dl.len();
                continue;
            }
            // Unmatched: emit the whole delimiter literally.
            out.push_str(&d.esc(&dl.iter().collect::<String>()));
            i += dl.len();
            continue;
        }
        out.push_str(&d.esc(c.encode_utf8(&mut [0; 4])));
        i += 1;
    }
    out
}

const BOLD: &[char] = &['*', '*'];
const STRIKE: &[char] = &['~', '~'];
const STAR: &[char] = &['*'];
const UNDER: &[char] = &['_'];

fn delim_at(s: &[char], i: usize) -> Option<(Style, &'static [char])> {
    let rest = &s[i..];
    if rest.starts_with(BOLD) {
        Some((Style::Bold, BOLD))
    } else if rest.starts_with(STRIKE) {
        Some((Style::Strike, STRIKE))
    } else if rest.starts_with(STAR) {
        // Bold, not italic: Slack's `*x*` is bold, and Telegram legacy
        // Markdown callers wrote `*x*` for bold too.
        Some((Style::Bold, STAR))
    } else if rest.starts_with(UNDER) {
        Some((Style::Italic, UNDER))
    } else {
        None
    }
}

/// The index of the delimiter closing the one opened at `i`, if the run is a
/// real emphasis: non-empty, no whitespace just inside either delimiter, and
/// (for `_`) word boundaries outside both.
fn find_close(s: &[char], i: usize, dl: &[char]) -> Option<usize> {
    let open_end = i + dl.len();
    if s.get(open_end).is_none_or(|c| c.is_whitespace()) {
        return None;
    }
    let underscore = dl == UNDER;
    if underscore && i > 0 && s[i - 1].is_alphanumeric() {
        return None;
    }
    let mut j = open_end + 1;
    while j + dl.len() <= s.len() {
        if s[j] == '`' {
            // Never close inside a code span.
            let run = s[j..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_backtick_close(s, j + run, run) {
                j = end + run;
                continue;
            }
        }
        if s[j..].starts_with(dl)
            && !s[j - 1].is_whitespace()
            && !(dl == STAR && (s.get(j + 1) == Some(&'*') || s[j - 1] == '*'))
            && !(underscore && s.get(j + 1).is_some_and(|c| c.is_alphanumeric()))
        {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_backtick_close(s: &[char], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < s.len() {
        if s[j] == '`' {
            let n = s[j..].iter().take_while(|c| **c == '`').count();
            if n == run {
                return Some(j);
            }
            j += n;
        } else {
            j += 1;
        }
    }
    None
}

/// `[label](url "title")` starting at `s[start] == '['` → (label, url, index
/// after the `)`).
fn parse_link(s: &[char], start: usize) -> Option<(&[char], String, usize)> {
    let mut depth = 0usize;
    let mut k = start;
    let close = loop {
        match s.get(k)? {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break k;
                }
            }
            _ => {}
        }
        k += 1;
    };
    if s.get(close + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut k = close + 1;
    let end = loop {
        match s.get(k)? {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break k;
                }
            }
            _ => {}
        }
        k += 1;
    };
    let target: String = s[close + 2..end].iter().collect();
    let url = target
        .split_whitespace()
        .next()?
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string();
    if url.is_empty() {
        return None;
    }
    Some((&s[start + 1..close], url, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical agent answer: heading, bold, a link, snake_case, inline and
    /// fenced code, a bullet list and a pipe table.
    const ANSWER: &str = "## Summary\n\
I fixed **two bugs** in `parse_args` — see [PR #12](https://github.com/o/r/pull/12).\n\
\n\
- the `max_rows` cap & retention\n\
* a <tag> in snake_case_name\n\
\n\
| File | Change |\n\
|------|--------|\n\
| **a.rs** | `fix` |\n\
| bb.rs | tests |\n\
\n\
```rust\n\
let x = a < b && c;\n\
```";

    #[test]
    fn slack_converts_a_typical_agent_answer() {
        let out = to_slack_mrkdwn(ANSWER);
        let want = [
            "*Summary*",
            "I fixed *two bugs* in `parse_args` — see <https://github.com/o/r/pull/12|PR #12>.",
            "",
            "• the `max_rows` cap & retention",
            "• a <tag> in snake_case_name",
            "",
            "```",
            "File  | Change",
            "------+-------",
            "a.rs  | fix",
            "bb.rs | tests",
            "```",
            "",
            "```",
            "let x = a < b && c;",
            "```",
        ]
        .join("\n");
        assert_eq!(out, want);
    }

    #[test]
    fn telegram_html_escapes_and_balances() {
        let out = to_telegram_html(ANSWER);
        assert!(out.starts_with("<b>Summary</b>\n"), "{out}");
        assert!(out.contains("I fixed <b>two bugs</b> in <code>parse_args</code>"));
        assert!(out.contains("<a href=\"https://github.com/o/r/pull/12\">PR #12</a>"));
        assert!(out.contains("• the <code>max_rows</code> cap &amp; retention"));
        // Raw `<tag>` is escaped; snake_case stays literal (no <i>).
        assert!(out.contains("• a &lt;tag&gt; in snake_case_name"));
        assert!(out.contains("<pre>File  | Change\n"), "{out}");
        assert!(out.contains("<pre>let x = a &lt; b &amp;&amp; c;</pre>"));
        // Every tag is closed.
        for tag in ["b", "i", "s", "code", "pre", "a"] {
            let opens =
                out.matches(&format!("<{tag}>")).count() + out.matches(&format!("<{tag} ")).count();
            assert_eq!(
                opens,
                out.matches(&format!("</{tag}>")).count(),
                "{tag}: {out}"
            );
        }
    }

    #[test]
    fn emphasis_edge_cases_stay_literal() {
        // Unpaired / spaced delimiters and arithmetic are not emphasis.
        assert_eq!(to_telegram_html("2 * 3 * 4"), "2 * 3 * 4");
        assert_eq!(to_telegram_html("a **b"), "a **b");
        assert_eq!(to_telegram_html("call foo_bar_baz()"), "call foo_bar_baz()");
        assert_eq!(
            to_telegram_html("_it_ and ~~gone~~"),
            "<i>it</i> and <s>gone</s>"
        );
        assert_eq!(to_slack_mrkdwn("_it_ and ~~gone~~"), "_it_ and ~gone~");
        // Slack-style input that callers already write survives.
        assert_eq!(
            to_slack_mrkdwn("*Done* <https://x|x>"),
            "*Done* <https://x|x>"
        );
        // Markup inside a code span is not converted.
        assert_eq!(to_slack_mrkdwn("`**x**`"), "`**x**`");
        assert_eq!(to_telegram_html("`**x**`"), "<code>**x**</code>");
    }

    #[test]
    fn links_with_parentheses_and_bare_labels() {
        assert_eq!(
            to_slack_mrkdwn("[wiki](https://en.wikipedia.org/wiki/Rust_(language))"),
            "<https://en.wikipedia.org/wiki/Rust_(language)|wiki>"
        );
        assert_eq!(
            to_telegram_html("[a & b](https://x.test/?a=1&b=2)"),
            "<a href=\"https://x.test/?a=1&amp;b=2\">a &amp; b</a>"
        );
        // Not a link: literal brackets.
        assert_eq!(to_slack_mrkdwn("[x] done"), "[x] done");
    }
}

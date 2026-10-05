//! Shared text helpers: char-safe clipping and ANSI stripping.
//!
//! The daemon used to carry ~17 private `truncate`/`clip` copies that disagreed
//! on bytes-vs-chars and on the ellipsis, plus three `strip_ansi` variants. The
//! semantics here are explicit in the name:
//!
//! - [`clip_chars`] — at most `max` CHARS of `s`, plus `…` when cut.
//! - [`clip_chars_with`] — same, with a caller-chosen marker.
//! - [`prefix_chars`] — the first `max` chars, borrowed, no marker.
//! - [`clip_bytes_on_char_boundary`] — the longest prefix of at most `max`
//!   BYTES that ends on a char boundary (never panics on multi-byte text).
//! - [`strip_ansi`] — remove CSI / OSC / two-char escape sequences only;
//!   every other character (newlines, `\r`, tabs) is preserved so callers
//!   choose their own whitespace policy.
//! - [`clip_bytes`] — at most `max` BYTES (char-boundary safe), plus `…`
//!   when cut.
//! - [`extract_json`] / [`find_json_object`] — the one tolerant
//!   JSON-from-LLM-reply extractor (```json fence, else the first balanced
//!   `{…}` that parses). Four divergent copies (product, swarm recruiter,
//!   workflow engine, goal loop) used to disagree on fences, retries and
//!   string escapes.

/// Ellipsis appended by [`clip_chars`].
pub const ELLIPSIS: &str = "…";

/// The first `max` chars of `s` (borrowed; no marker). Single pass.
pub fn prefix_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

/// `s` unchanged when it has at most `max` chars, else its first `max` chars
/// followed by `marker`.
pub fn clip_chars_with(s: &str, max: usize, marker: &str) -> String {
    let head = prefix_chars(s, max);
    if head.len() == s.len() {
        return s.to_string();
    }
    let mut out = String::with_capacity(head.len() + marker.len());
    out.push_str(head);
    out.push_str(marker);
    out
}

/// `s` unchanged when it has at most `max` chars, else its first `max` chars
/// followed by [`ELLIPSIS`]. (The result can therefore be `max + 1` chars.)
pub fn clip_chars(s: &str, max: usize) -> String {
    clip_chars_with(s, max, ELLIPSIS)
}

/// The longest prefix of `s` that is at most `max` bytes AND ends on a char
/// boundary — for byte-budgeted payloads (logs, JSON caps) holding UTF-8.
pub fn clip_bytes_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `s` unchanged when it is at most `max` BYTES, else its longest
/// char-boundary prefix of at most `max` bytes followed by [`ELLIPSIS`].
pub fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}{ELLIPSIS}", clip_bytes_on_char_boundary(s, max))
}

/// Balanced top-level `{…}` spans of `text`, in order. String literals and
/// their escapes are respected, so a `}` (or `{`) inside a JSON string does
/// not end (or open) a span. An unclosed trailing `{` yields nothing.
fn balanced_object_spans(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    std::iter::from_fn(move || {
        let mut depth = 0usize;
        let mut start = None;
        let mut in_str = false;
        let mut escaped = false;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            if in_str {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_str = false;
                }
                continue;
            }
            match b {
                // Quotes only matter inside a candidate: prose apostrophes
                // and stray `"` before the first `{` must not swallow it.
                b'"' if start.is_some() => in_str = true,
                b'{' => {
                    if depth == 0 {
                        start = Some(i - 1);
                    }
                    depth += 1;
                }
                b'}' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        // `{`/`}` are ASCII, so both ends are char boundaries.
                        return start.map(|s| &text[s..i]);
                    }
                }
                _ => {}
            }
        }
        None
    })
}

/// The first balanced top-level `{…}` substring of `text` (string-aware), or
/// `None`. Does not check that it parses — see [`extract_json`] for that.
pub fn find_json_object(text: &str) -> Option<&str> {
    balanced_object_spans(text).next()
}

/// Tolerantly extract a JSON value from an agent / LLM reply.
///
/// 1. The first ```` ```json ```` fenced block, when its body parses (any JSON
///    value, so a fenced array works). Wins over a bare `{` earlier in prose.
/// 2. Otherwise the first balanced `{…}` span that parses as JSON — a span
///    that does not parse (`{placeholder}` in prose) is skipped, not fatal.
/// 3. `None` for prose-only input.
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
    if let Some(fence) = text.find("```json") {
        let body = &text[fence + "```json".len()..];
        if let Some(end) = body.find("```") {
            if let Ok(v) = serde_json::from_str(body[..end].trim()) {
                return Some(v);
            }
        }
    }
    balanced_object_spans(text).find_map(|span| serde_json::from_str(span).ok())
}

/// Remove ANSI/VT escape sequences: CSI (`ESC [` … final byte `0x40..=0x7E`),
/// OSC (`ESC ]` … BEL or ST `ESC \`) and any other two-char `ESC x`. All other
/// characters — including `\n`, `\r`, `\t` and other C0 controls — pass
/// through untouched.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for f in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&f) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(f) = chars.next() {
                    if f == '\x07' {
                        break;
                    }
                    if f == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            // Lone/other escape: the ESC and its one follower are dropped.
            _ => {}
        }
    }
    out
}

/// [`strip_ansi`] over raw PTY bytes (invalid UTF-8 → U+FFFD).
pub fn strip_ansi_bytes(raw: &[u8]) -> String {
    strip_ansi(&String::from_utf8_lossy(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_bytes_cuts_on_a_char_boundary_and_marks_the_cut() {
        assert_eq!(clip_bytes("hello", 5), "hello");
        assert_eq!(clip_bytes("hello", 3), "hel…");
        // "é" is 2 bytes: a cut inside it backs off to the boundary.
        assert_eq!(clip_bytes("héllo", 2), "h…");
        assert_eq!(clip_bytes("", 0), "");
    }

    #[test]
    fn extract_json_prefers_a_fence_over_an_earlier_bare_brace() {
        let t = "Use {x} here.\n```json\n{\"a\": 1}\n```";
        assert_eq!(extract_json(t).unwrap()["a"], 1);
        // A fenced ARRAY is returned as-is.
        assert_eq!(extract_json("```json\n[1, 2]\n```").unwrap()[1], 2);
    }

    #[test]
    fn extract_json_skips_spans_that_do_not_parse() {
        let t = "Fill in {placeholder} then: {\"ok\": true} trailing }";
        assert_eq!(extract_json(t).unwrap()["ok"], true);
        // The span finder alone returns the first balanced span, parse or not.
        assert_eq!(find_json_object(t), Some("{placeholder}"));
    }

    #[test]
    fn extract_json_respects_braces_and_escapes_inside_strings() {
        let t = r#"note: {"msg": "a } brace and \" quote {", "n": {"b": 2}} done"#;
        let v = extract_json(t).unwrap();
        assert_eq!(v["msg"], "a } brace and \" quote {");
        assert_eq!(v["n"]["b"], 2);
    }

    #[test]
    fn extract_json_ignores_quotes_in_prose_before_the_object() {
        let t = "The agent said \"done and here's the result: {\"a\": 1}";
        assert_eq!(extract_json(t).unwrap()["a"], 1);
    }

    #[test]
    fn extract_json_none_for_prose_or_unclosed_objects() {
        assert!(extract_json("").is_none());
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("{\"a\": 1").is_none());
        assert!(find_json_object("{ never closed").is_none());
        // A fence whose body does not parse falls back to the balanced scan.
        assert_eq!(
            extract_json("```json\n{bad}\n``` {\"a\":3}").unwrap()["a"],
            3
        );
    }

    #[test]
    fn clip_chars_counts_chars_not_bytes() {
        assert_eq!(clip_chars("héllo", 5), "héllo");
        assert_eq!(clip_chars("héllo", 2), "hé…");
        assert_eq!(clip_chars("日本語テキスト", 3), "日本語…");
        assert_eq!(clip_chars("", 0), "");
        assert_eq!(clip_chars("ab", 0), "…");
    }

    #[test]
    fn clip_chars_with_uses_the_marker_only_when_cut() {
        assert_eq!(
            clip_chars_with("abcdef", 3, "\n…[truncated]"),
            "abc\n…[truncated]"
        );
        assert_eq!(clip_chars_with("abc", 3, "!"), "abc");
    }

    #[test]
    fn prefix_chars_borrows_without_a_marker() {
        assert_eq!(prefix_chars("🦀🦀🦀", 2), "🦀🦀");
        assert_eq!(prefix_chars("ab", 5), "ab");
    }

    #[test]
    fn clip_bytes_never_splits_a_char() {
        // "é" is 2 bytes; a 2-byte budget after "a" must back off to "a".
        assert_eq!(clip_bytes_on_char_boundary("aé", 2), "a");
        assert_eq!(clip_bytes_on_char_boundary("aé", 3), "aé");
        assert_eq!(clip_bytes_on_char_boundary("🦀x", 3), "");
        assert_eq!(clip_bytes_on_char_boundary("abc", 10), "abc");
    }

    #[test]
    fn strip_ansi_removes_csi_osc_and_two_char_escapes_only() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[?25lhide"), "hide");
        assert_eq!(strip_ansi("\x1b]0;title\x07body"), "body");
        assert_eq!(strip_ansi("\x1b]8;;http://x\x1b\\link"), "link");
        assert_eq!(strip_ansi("a\x1b=b"), "ab");
        assert_eq!(strip_ansi("l1\r\nl2\tx"), "l1\r\nl2\tx");
        assert_eq!(strip_ansi("trailing\x1b"), "trailing");
        assert_eq!(strip_ansi_bytes(b"ok\xff\x1b[1m!"), "ok\u{fffd}!");
    }
}

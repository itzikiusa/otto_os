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
    fn clip_chars_counts_chars_not_bytes() {
        assert_eq!(clip_chars("héllo", 5), "héllo");
        assert_eq!(clip_chars("héllo", 2), "hé…");
        assert_eq!(clip_chars("日本語テキスト", 3), "日本語…");
        assert_eq!(clip_chars("", 0), "");
        assert_eq!(clip_chars("ab", 0), "…");
    }

    #[test]
    fn clip_chars_with_uses_the_marker_only_when_cut() {
        assert_eq!(clip_chars_with("abcdef", 3, "\n…[truncated]"), "abc\n…[truncated]");
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

//! Shared line-ending (`\r\n` vs `\n`) and UTF-16 column <-> UTF-8 byte offset
//! conversions for IDE text edits, quick fixes, and lexical symbol scanners.

/// Detect the dominant line ending in `content` (`"\r\n"` when CRLF newlines
/// outnumber or tie bare LF newlines, else `"\n"`).
pub fn detect_eol(content: &str) -> &'static str {
    let bytes = content.as_bytes();
    let mut crlf = 0usize;
    let mut bare_lf = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            crlf += 1;
            i += 2;
        } else if bytes[i] == b'\n' {
            bare_lf += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    if crlf > 0 && crlf >= bare_lf {
        "\r\n"
    } else {
        "\n"
    }
}

/// Strip a single trailing `'\r'` from a line slice.
pub fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// Convert a `0`-based UTF-16 code-unit column on `line` into a valid UTF-8 byte
/// offset within `strip_cr(line)`.
///
/// If `utf16_col_0based` lands in the middle of a 2-unit surrogate pair (such as
/// a 4-byte emoji `🚀`), clamps cleanly to the start byte of that character.
pub fn utf16_col_to_byte_idx(line: &str, utf16_col_0based: usize) -> usize {
    let clean = strip_cr(line);
    let mut u16_off = 0usize;
    for (b_idx, ch) in clean.char_indices() {
        if u16_off >= utf16_col_0based {
            return b_idx;
        }
        let w = ch.len_utf16();
        if utf16_col_0based < u16_off + w {
            return b_idx;
        }
        u16_off += w;
    }
    clean.len()
}

/// Convert a UTF-8 byte offset on `line` into a `0`-based UTF-16 code-unit column
/// within `strip_cr(line)`.
pub fn byte_idx_to_utf16_col(line: &str, byte_idx: usize) -> usize {
    let clean = strip_cr(line);
    let mut end = byte_idx.min(clean.len());
    while end > 0 && !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].chars().map(char::len_utf16).sum()
}

/// Convert a UTF-8 byte offset in `text` into a `1`-based `(line, utf16_col)` pair,
/// treating `\r\n` as a single line break without counting `\r` as a visible column.
pub fn byte_offset_to_line_utf16_col(text: &str, byte_offset: usize) -> (u32, u32) {
    let mut clamped = byte_offset.min(text.len());
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }
    let bytes = text.as_bytes();
    let mut line = 1u32;
    let mut col = 1u32;
    for (idx, ch) in text[..clamped].char_indices() {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else if ch == '\r' {
            if bytes.get(idx + 1) == Some(&b'\n') {
                // Part of CRLF line terminator; do not advance column.
            } else {
                // Bare CR line terminator (LSP 3.17 / CodeMirror).
                line += 1;
                col = 1;
            }
        } else {
            col += ch.len_utf16() as u32;
        }
    }
    (line, col)
}

/// Convert a `1`-based `(target_line, target_utf16_col)` pair in `text` into a
/// valid UTF-8 byte offset, clamping past-end-of-line columns right before `\r\n`, `\n`, or bare `\r`.
pub fn line_utf16_col_to_byte_offset(text: &str, target_line: u32, target_col: u32) -> usize {
    let target_line = target_line.max(1);
    let target_col = target_col.max(1);
    let bytes = text.as_bytes();
    let mut line = 1u32;
    let mut col = 1u32;
    for (idx, ch) in text.char_indices() {
        if line == target_line && col >= target_col {
            return idx;
        }
        if ch == '\r' && bytes.get(idx + 1) == Some(&b'\n') {
            if line == target_line {
                return idx;
            }
            continue;
        }
        if ch == '\n' || ch == '\r' {
            if line == target_line {
                return idx;
            }
            line += 1;
            col = 1;
        } else {
            let w = ch.len_utf16() as u32;
            if line == target_line && target_col < col + w {
                return idx;
            }
            col += w;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_crlf_conversions_roundtrip_with_emojis() {
        let line = "const tag = \"🚀🎉\"; let target = 42;\r";
        let target_byte = line.find("target").unwrap();
        // "const tag = \"" (13) + "🚀🎉" (4 UTF-16 units) + "\"; let " (7) = 24 (0-based)
        let u16_col = byte_idx_to_utf16_col(line, target_byte);
        assert_eq!(u16_col, 24);
        assert_eq!(utf16_col_to_byte_idx(line, u16_col), target_byte);
        // Mid-surrogate clamps safely to start of emoji:
        let rocket_byte = line.find('🚀').unwrap();
        assert_eq!(utf16_col_to_byte_idx(line, 14), rocket_byte);

        let doc = "first\r\nconst s = \"🚀\"; call()\r\nthird\r\n";
        assert_eq!(detect_eol(doc), "\r\n");
        let call_off = doc.find("call").unwrap();
        let (ln, col) = byte_offset_to_line_utf16_col(doc, call_off);
        // "const s = \"" (11) + "🚀" (2 UTF-16 units) + "\"; " (3) = 16 preceding units -> 1-based col 17
        assert_eq!((ln, col), (2, 17));
        assert_eq!(line_utf16_col_to_byte_offset(doc, ln, col), call_off);
    }
}

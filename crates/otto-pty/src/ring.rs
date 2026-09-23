//! Line-aware ring buffer for PTY scrollback.
//!
//! Stores output as lines (chunks split inclusively on `\n`); evicts whole
//! lines from the front when either the line cap or the byte cap is exceeded.

use std::collections::VecDeque;

/// Default maximum number of retained lines.
pub const DEFAULT_MAX_LINES: usize = 10_000;
/// Default maximum number of retained bytes (2 MiB).
pub const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Line-aware byte ring buffer.
pub struct RingBuffer {
    lines: VecDeque<Vec<u8>>,
    bytes: usize,
    max_lines: usize,
    max_bytes: usize,
    /// True when the last stored line ended with a newline (i.e. the next
    /// chunk starts a fresh line).
    last_complete: bool,
}

impl RingBuffer {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
            max_lines,
            max_bytes,
            last_complete: true,
        }
    }

    /// Append raw PTY output, splitting into lines on `\n`.
    pub fn push(&mut self, data: &[u8]) {
        for chunk in data.split_inclusive(|&b| b == b'\n') {
            let ends_line = chunk.ends_with(b"\n");
            if !self.last_complete {
                if let Some(last) = self.lines.back_mut() {
                    last.extend_from_slice(chunk);
                } else {
                    self.lines.push_back(chunk.to_vec());
                }
            } else {
                self.lines.push_back(chunk.to_vec());
            }
            self.bytes += chunk.len();
            self.last_complete = ends_line;
            self.evict();
        }
    }

    fn evict(&mut self) {
        while self.lines.len() > 1
            && (self.lines.len() > self.max_lines || self.bytes > self.max_bytes)
        {
            if let Some(front) = self.lines.pop_front() {
                self.bytes -= front.len();
            }
        }
        // One line with no newline in sight (a minified bundle / JSON blob, a
        // `\r` progress bar, a cursor-addressing TUI) must be bounded too —
        // whole-line eviction never touches the last line, so it grew without
        // limit (and `tail`/`search` cloned all of it). Drop its oldest bytes.
        if self.bytes > self.max_bytes {
            if let Some(only) = self.lines.front_mut() {
                let excess = (self.bytes - self.max_bytes).min(only.len());
                *only = only.split_off(excess);
                self.bytes -= excess;
            }
        }
    }

    /// Concatenated bytes of the last `lines` lines (all lines when larger).
    pub fn tail(&self, lines: usize) -> Vec<u8> {
        let n = lines.min(self.lines.len());
        let start = self.lines.len() - n;
        let mut out = Vec::new();
        for line in self.lines.iter().skip(start) {
            out.extend_from_slice(line);
        }
        out
    }

    /// Number of retained lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Search the ring for lines containing `query` (plain substring,
    /// case-insensitive after stripping ANSI escape sequences).
    ///
    /// Returns up to `limit` matching lines in buffer order (oldest → newest),
    /// each as a `(line_index, text)` pair where `line_index` is 0-based from
    /// the oldest retained line. `text` is the stripped (plain-text) content.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, String)> {
        let needle = query.to_lowercase();
        let mut results = Vec::new();
        for (idx, raw) in self.lines.iter().enumerate() {
            if results.len() >= limit {
                break;
            }
            // Strip ANSI escape sequences: sequences starting with ESC [ and
            // ending with a letter (CSI), OSC (ESC ]), and plain ESC X runs.
            let text = strip_ansi(raw);
            if text.to_lowercase().contains(&needle) {
                results.push((idx, text));
            }
        }
        results
    }
}

/// Strip ANSI/VT escape sequences from a raw PTY byte slice, returning plain
/// text. Handles CSI sequences (`ESC [ … <letter>`), OSC sequences (`ESC ]
/// … BEL/ST`), and bare `ESC X` two-byte sequences. Non-UTF-8 bytes are
/// replaced with U+FFFD.
fn strip_ansi(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        // ESC: look at the next byte to classify the sequence.
        match chars.peek().copied() {
            Some('[') => {
                // CSI sequence: ESC [ … final-byte (A-Za-z@~)
                chars.next(); // consume '['
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '@' || c == '~' {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC sequence: ESC ] … BEL or ESC \.
                chars.next(); // consume ']'
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            Some(_) => {
                // Other two-byte ESC sequences — skip the next character.
                chars.next();
            }
            None => {}
        }
    }
    out
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output without a newline used to grow the last line without bound.
    #[test]
    fn a_single_unterminated_line_is_bounded_by_max_bytes() {
        let mut ring = RingBuffer::new(100, 64);
        for i in 0..1000u32 {
            ring.push(format!("\r{i:05}%").as_bytes());
        }
        assert_eq!(ring.len(), 1);
        let tail = ring.tail(1);
        assert!(tail.len() <= 64, "line kept {} bytes", tail.len());
        assert!(
            String::from_utf8_lossy(&tail).ends_with("\r00999%"),
            "the newest bytes must be the ones kept"
        );
        // A later newline still starts a fresh line and eviction resumes.
        ring.push(b"\ndone\n");
        assert!(String::from_utf8_lossy(&ring.tail(1)).contains("done"));
        assert!(ring.tail(10).len() <= 64);
    }

    #[test]
    fn whole_lines_still_evict_first() {
        let mut ring = RingBuffer::new(3, 1024);
        ring.push(b"a\nb\nc\nd\n");
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.tail(10), b"b\nc\nd\n".to_vec());
    }
}

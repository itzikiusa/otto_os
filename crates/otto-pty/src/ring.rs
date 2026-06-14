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
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)
    }
}

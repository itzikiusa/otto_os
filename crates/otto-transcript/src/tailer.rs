//! `Tailer` — a plain byte-offset reader for one append-only transcript file
//! (design §4.1). Transcripts only grow (a live-growth prefix sha256 is stable
//! and re-emitted sidecars like `ai-title` are appended, never rewritten), so
//! there is no rescan logic: read from `offset`, keep the trailing partial
//! line for the next poll, and treat a SHRINKING file as replaced → restart at
//! 0. The server's supervisor (`otto-server/src/transcript_tail.rs`) polls
//! this every 700 ms per live session.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::records::parse_line;

#[derive(Debug)]
pub struct Tailer {
    pub path: PathBuf,
    /// Bytes consumed so far (up to and including the last `\n` handed out).
    pub offset: u64,
    /// Bytes after the last newline — an in-flight write; prepended on the
    /// next poll.
    pub partial_line: Vec<u8>,
}

/// Outcome of one poll.
#[derive(Debug, Default)]
pub struct TailDelta {
    /// New complete records, in file order.
    pub records: Vec<Value>,
    /// The file got shorter (replaced/truncated) — the caller must throw away
    /// everything it folded and start over from the returned records.
    pub restarted: bool,
}

impl Tailer {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            partial_line: Vec::new(),
        }
    }

    /// Start from an already-consumed offset (e.g. the caller folded the whole
    /// file first and only wants what appears next).
    pub fn at(path: impl Into<PathBuf>, offset: u64) -> Self {
        Self {
            path: path.into(),
            offset,
            partial_line: Vec::new(),
        }
    }

    /// Read whatever appeared since the last poll. A missing file is not an
    /// error — it yields an empty delta (the CLI may not have created it yet).
    pub fn poll(&mut self) -> std::io::Result<TailDelta> {
        let mut out = TailDelta::default();
        let mut f = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        let len = f.metadata()?.len();
        if len < self.offset {
            // Replaced or truncated: restart from the top.
            self.offset = 0;
            self.partial_line.clear();
            out.restarted = true;
        }
        if len == self.offset {
            return Ok(out);
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut fresh = Vec::with_capacity((len - self.offset) as usize);
        f.read_to_end(&mut fresh)?;
        // The offset tracks every byte READ; the partial line is carried in
        // memory, not re-read.
        self.offset += fresh.len() as u64;
        let mut buf = std::mem::take(&mut self.partial_line);
        buf.extend_from_slice(&fresh);
        match buf.iter().rposition(|b| *b == b'\n') {
            Some(nl) => {
                let text = String::from_utf8_lossy(&buf[..=nl]);
                out.records = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(parse_line)
                    .collect();
                self.partial_line = buf[nl + 1..].to_vec();
            }
            None => self.partial_line = buf,
        }
        Ok(out)
    }

    /// Byte length of the file at `path` (0 when missing) — the offset to hand
    /// [`Tailer::at`] after a whole-file fold.
    pub fn current_len(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn tail_reads_complete_lines_and_carries_partials() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"{\"type\":\"a\"}\n{\"type\":\"b\"").unwrap();
        let mut t = Tailer::new(&p);
        let d = t.poll().unwrap();
        assert_eq!(d.records.len(), 1);
        assert_eq!(t.partial_line, b"{\"type\":\"b\"");
        // Nothing new → nothing.
        assert!(t.poll().unwrap().records.is_empty());
        f.write_all(b"}\n{\"type\":\"c\"}\n").unwrap();
        let d = t.poll().unwrap();
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[0]["type"], "b");
        assert_eq!(d.records[1]["type"], "c");
        assert!(t.partial_line.is_empty());
        assert_eq!(t.offset, Tailer::current_len(&p));
    }

    #[test]
    fn shrinking_file_restarts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        std::fs::write(&p, "{\"type\":\"a\"}\n{\"type\":\"b\"}\n").unwrap();
        let mut t = Tailer::new(&p);
        assert_eq!(t.poll().unwrap().records.len(), 2);
        std::fs::write(&p, "{\"type\":\"z\"}\n").unwrap();
        let d = t.poll().unwrap();
        assert!(d.restarted);
        assert_eq!(d.records.len(), 1);
        assert_eq!(d.records[0]["type"], "z");
    }

    #[test]
    fn missing_file_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Tailer::new(dir.path().join("nope.jsonl"));
        assert!(t.poll().unwrap().records.is_empty());
    }
}

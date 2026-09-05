//! Reading a transcript file into records. A transcript is JSONL and
//! append-only, so `records[i]` is stable across live growth — that index IS
//! the paging cursor (design §3). Only COMPLETE lines (terminated by `\n`) are
//! parsed: a trailing partial line belongs to a write still in flight and is
//! left for the next read, otherwise it would show up as a transient
//! "unknown record" and shift every later index by one.

use std::io::Read;
use std::path::Path;

use serde_json::Value;

/// Placeholder `type` for a complete line that is not valid JSON. It keeps the
/// index stable and folds into an `unknown` notice (so nothing is silently
/// dropped) without aborting the whole file.
pub const UNPARSEABLE_TYPE: &str = "__unparseable__";

/// Parse `bytes` (a whole file or a tail chunk of complete lines) into records.
pub fn parse_records(bytes: &[u8]) -> Vec<Value> {
    let complete = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(nl) => &bytes[..=nl],
        None => return Vec::new(),
    };
    let text = String::from_utf8_lossy(complete);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_line)
        .collect()
}

/// Parse one line; malformed JSON → the [`UNPARSEABLE_TYPE`] placeholder.
pub fn parse_line(line: &str) -> Value {
    serde_json::from_str::<Value>(line.trim())
        .unwrap_or_else(|_| serde_json::json!({ "type": UNPARSEABLE_TYPE }))
}

/// Read every complete record in `path`.
pub fn read_records(path: &Path) -> std::io::Result<Vec<Value>> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(parse_records(&buf))
}

/// Read only the first `head` bytes and the last `tail` bytes of `path` (the
/// history index's cheap peek). Returns `(head_records, tail_records)`; when
/// the file is smaller than `head + tail` the tail is empty and the head holds
/// everything. Tail parsing starts after the first newline in the chunk so a
/// half-line at the seam is skipped.
pub fn read_head_tail(path: &Path, head: u64, tail: u64) -> std::io::Result<(Vec<Value>, Vec<Value>)> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len <= head + tail {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        return Ok((parse_records(&buf), Vec::new()));
    }
    let mut hb = vec![0u8; head as usize];
    f.read_exact(&mut hb)?;
    f.seek(SeekFrom::Start(len - tail))?;
    let mut tb = Vec::new();
    f.read_to_end(&mut tb)?;
    let tail_records = match tb.iter().position(|b| *b == b'\n') {
        Some(nl) => parse_records(&tb[nl + 1..]),
        None => Vec::new(),
    };
    Ok((parse_records(&hb), tail_records))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_trailing_line_is_left_out() {
        let recs = parse_records(b"{\"type\":\"a\"}\n{\"type\":\"b\"}\n{\"type\":\"c\"");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1]["type"], "b");
        assert!(parse_records(b"{\"type\":\"a\"}").is_empty());
    }

    #[test]
    fn bad_json_keeps_its_index() {
        let recs = parse_records(b"{\"type\":\"a\"}\nnot json\n{\"type\":\"c\"}\n");
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[1]["type"], UNPARSEABLE_TYPE);
    }

    #[test]
    fn head_tail_reads_both_ends() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut s = String::new();
        for i in 0..200 {
            s.push_str(&format!("{{\"type\":\"r\",\"i\":{i},\"pad\":\"{}\"}}\n", "x".repeat(50)));
        }
        std::fs::write(&p, &s).unwrap();
        let (head, tail) = read_head_tail(&p, 300, 300).unwrap();
        assert!(!head.is_empty() && head.len() < 10);
        assert_eq!(head[0]["i"], 0);
        assert!(!tail.is_empty());
        assert_eq!(tail.last().unwrap()["i"], 199);
        // Small file: everything in the head.
        std::fs::write(&p, "{\"type\":\"r\",\"i\":1}\n").unwrap();
        let (head, tail) = read_head_tail(&p, 300, 300).unwrap();
        assert_eq!(head.len(), 1);
        assert!(tail.is_empty());
    }
}

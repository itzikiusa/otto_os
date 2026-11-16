//! Bounded, lossy line reading for child-process pipes.
//!
//! Tokio's `Lines::next_line` returns `InvalidData` on the first non-UTF-8 byte
//! and grows without bound on a line with no newline. A reader loop written as
//! `while let Ok(Some(l)) = lines.next_line().await` then stops, drops the pipe's
//! read end, and the child dies of SIGPIPE (or blocks, or loses all further
//! output) on its next write. [`LossyLines`] never stops early: invalid UTF-8 is
//! replaced with `U+FFFD`, a line longer than its cap keeps only the first
//! `max_line` bytes (the rest is read and dropped), and only EOF or a real I/O
//! error ends it — so the pipe is always drained.

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::task::JoinHandle;

/// Per-line cap for console output (run consoles, adapter stderr, server logs).
pub const MAX_OUTPUT_LINE_BYTES: usize = 16 * 1024;
/// Per-line cap for line-oriented control protocols (shell bridge FIFO).
pub const MAX_PROTOCOL_LINE_BYTES: usize = 256 * 1024;
/// Marker appended to a line that was cut at its cap.
pub const TRUNCATED_MARKER: &str = "... [truncated]";

/// Cancel-safe line reader: every partial line lives in `self`, and the only
/// await point is `fill_buf`, which loses nothing when dropped.
pub struct LossyLines<R> {
    reader: BufReader<R>,
    buf: Vec<u8>,
    max_line: usize,
    truncated: bool,
    done: bool,
}

impl<R: AsyncRead + Unpin> LossyLines<R> {
    pub fn new(reader: R, max_line: usize) -> Self {
        Self {
            reader: BufReader::new(reader),
            buf: Vec::new(),
            max_line: max_line.max(1),
            truncated: false,
            done: false,
        }
    }

    /// The next line without its `\n` / `\r\n`, or `None` at EOF or on an I/O
    /// error. A final line with no trailing newline is still returned.
    pub async fn next_line(&mut self) -> Option<String> {
        if self.done {
            return None;
        }
        loop {
            let avail = match self.reader.fill_buf().await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.done = true;
                    return self.take_pending();
                }
            };
            if avail.is_empty() {
                self.done = true;
                return self.take_pending();
            }
            let (chunk_len, newline) = match avail.iter().position(|&b| b == b'\n') {
                Some(i) => (i, true),
                None => (avail.len(), false),
            };
            let room = self.max_line.saturating_sub(self.buf.len());
            let keep = chunk_len.min(room);
            self.buf.extend_from_slice(&avail[..keep]);
            if keep < chunk_len {
                self.truncated = true;
            }
            let consumed = if newline { chunk_len + 1 } else { chunk_len };
            self.reader.consume(consumed);
            if newline {
                return Some(self.finish_line());
            }
        }
    }

    fn take_pending(&mut self) -> Option<String> {
        (!self.buf.is_empty() || self.truncated).then(|| self.finish_line())
    }

    fn finish_line(&mut self) -> String {
        if !self.truncated && self.buf.last() == Some(&b'\r') {
            self.buf.pop();
        }
        let mut line = String::from_utf8_lossy(&self.buf).into_owned();
        if self.truncated {
            line.push_str(TRUNCATED_MARKER);
        }
        self.buf.clear();
        if self.buf.capacity() > self.max_line.min(64 * 1024) {
            self.buf.shrink_to(4096);
        }
        self.truncated = false;
        line
    }
}

/// Drain `reader` to EOF on a spawned task, calling `on_line` for each line.
pub fn spawn_line_drain<R>(
    reader: R,
    max_line: usize,
    mut on_line: impl FnMut(String) + Send + 'static,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = LossyLines::new(reader, max_line);
        while let Some(line) = lines.next_line().await {
            on_line(line);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn collect(input: &[u8], max: usize) -> Vec<String> {
        let mut lines = LossyLines::new(input, max);
        let mut out = Vec::new();
        while let Some(l) = lines.next_line().await {
            out.push(l);
        }
        out
    }

    #[tokio::test]
    async fn invalid_utf8_is_replaced_and_reading_continues() {
        let out = collect(b"before\n\xff\xfe raw\nafter\r\nlast", 1024).await;
        assert_eq!(out, vec!["before", "\u{fffd}\u{fffd} raw", "after", "last"]);
    }

    #[tokio::test]
    async fn long_line_is_cut_at_the_cap_and_the_rest_is_drained() {
        let mut input = vec![b'x'; 100_000];
        input.extend_from_slice(b"\nnext\n");
        let mut lines = LossyLines::new(&input[..], 64);
        let first = lines.next_line().await.unwrap();
        assert_eq!(first, format!("{}{TRUNCATED_MARKER}", "x".repeat(64)));
        assert_eq!(lines.next_line().await.as_deref(), Some("next"));
        assert_eq!(lines.next_line().await, None);
    }

    #[tokio::test]
    async fn unterminated_flood_never_buffers_more_than_the_cap() {
        let (mut tx, rx) = tokio::io::duplex(4096);
        let writer = tokio::spawn(async move {
            let chunk = vec![b'y'; 64 * 1024];
            for _ in 0..160 {
                tx.write_all(&chunk).await.unwrap();
            }
            tx.write_all(b"\ndone\n").await.unwrap();
        });
        let mut lines = LossyLines::new(rx, 1024);
        let first = lines.next_line().await.unwrap();
        assert!(first.len() <= 1024 + TRUNCATED_MARKER.len());
        assert!(lines.buf.capacity() <= 64 * 1024);
        assert_eq!(lines.next_line().await.as_deref(), Some("done"));
        writer.await.unwrap();
    }

    /// The child keeps writing after an invalid byte: with tokio's `lines()` the
    /// reader stopped and the child died of SIGPIPE on its next write.
    #[cfg(unix)]
    #[tokio::test]
    async fn child_writing_invalid_utf8_keeps_running_and_exits_zero() {
        let mut child = tokio::process::Command::new("/bin/sh")
            .args([
                "-c",
                "printf '\\377\\n'; i=0; while [ $i -lt 2000 ]; do echo line-$i; i=$((i+1)); done; echo end",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);
        let task = spawn_line_drain(out, MAX_OUTPUT_LINE_BYTES, move |l| {
            sink.lock().unwrap().push(l);
        });
        let status = child.wait().await.unwrap();
        task.await.unwrap();
        assert!(status.success(), "child died: {status:?}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first().map(String::as_str), Some("\u{fffd}"));
        assert_eq!(seen.last().map(String::as_str), Some("end"));
        assert_eq!(seen.len(), 2002);
    }
}

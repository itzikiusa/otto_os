//! The embedded server's console output, captured to a small capped file.
//!
//! The server logs at `error` level to the console only (a Poco FileChannel
//! wedges after ENOSPC — see `write_server_config`). Its stdout/stderr used to
//! go to /dev/null, so a start that failed under memory pressure left nothing
//! but "server did not answer /ping". The output now goes to
//! `<data dir>/server/clickhouse-console.log`, rotated to `.1` at spawn once it
//! passes [`LOG_CAP`], and the lines a failed start wrote are quoted into the
//! engine's `start_error`. A plain file descriptor that hits ENOSPC only fails
//! the write; nothing in the server retries it.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Rotate the log at spawn once it is this large (one `.1` generation kept).
pub(super) const LOG_CAP: u64 = 256 * 1024;
/// How much of the file a failed start quotes from (the tail of its output).
const TAIL_BYTES: u64 = 4 * 1024;
/// At most this many lines of that tail go into the error message.
const TAIL_LINES: usize = 6;

/// Where the server's console output goes (one file per data dir).
pub(super) struct ServerLog {
    path: PathBuf,
}

impl ServerLog {
    pub(super) fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("server").join("clickhouse-console.log"),
        }
    }

    /// Rotate when over the cap, then open for append. Returns the stdout and
    /// stderr handles for the child and the offset this run starts at. Any
    /// failure falls back to /dev/null: a missing log never blocks a start.
    pub(super) fn open(&self) -> (Stdio, Stdio, u64) {
        self.try_open().unwrap_or_else(|e| {
            tracing::debug!("usage: clickhouse console log unavailable: {e}");
            (Stdio::null(), Stdio::null(), 0)
        })
    }

    fn try_open(&self) -> std::io::Result<(Stdio, Stdio, u64)> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > LOG_CAP) {
            std::fs::rename(&self.path, self.path.with_extension("log.1"))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let start = file.metadata()?.len();
        let err = file.try_clone()?;
        Ok((Stdio::from(file), Stdio::from(err), start))
    }

    /// The last non-empty lines written since `since`, joined with ` | `
    /// (empty when the server wrote nothing or the file is unreadable).
    pub(super) fn tail_since(&self, since: u64) -> String {
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return String::new();
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        // A rotation between open and now restarts the file at 0.
        let since = if since > len { 0 } else { since };
        let from = since.max(len.saturating_sub(TAIL_BYTES));
        let mut buf = Vec::new();
        if f.seek(SeekFrom::Start(from)).is_err() || f.read_to_end(&mut buf).is_err() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&buf);
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        lines[lines.len().saturating_sub(TAIL_LINES)..].join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_through(stdio_file: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(stdio_file)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn tail_quotes_only_what_this_run_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let log = ServerLog::new(dir.path());
        let (_o, _e, first) = log.open();
        assert_eq!(first, 0);
        write_through(&log.path, "old run: fine\n");
        let (_o, _e, start) = log.open();
        write_through(
            &log.path,
            "\nCode: 241. DB::Exception: Memory limit exceeded\nshutting down\n",
        );
        let tail = log.tail_since(start);
        assert_eq!(
            tail,
            "Code: 241. DB::Exception: Memory limit exceeded | shutting down"
        );
        assert!(!tail.contains("old run"));
    }

    #[test]
    fn tail_keeps_the_last_lines_only() {
        let dir = tempfile::tempdir().unwrap();
        let log = ServerLog::new(dir.path());
        let (_o, _e, start) = log.open();
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        write_through(&log.path, &body);
        let tail = log.tail_since(start);
        assert_eq!(tail.split(" | ").count(), TAIL_LINES);
        assert!(tail.ends_with("line 19"), "{tail}");
    }

    #[test]
    fn open_rotates_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let log = ServerLog::new(dir.path());
        drop(log.open());
        write_through(&log.path, &"x".repeat(LOG_CAP as usize + 1));
        let (_o, _e, start) = log.open();
        assert_eq!(start, 0, "an over-cap log starts over");
        let rotated = log.path.with_extension("log.1");
        assert!(std::fs::metadata(rotated).unwrap().len() > LOG_CAP);
    }

    #[test]
    fn missing_log_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ServerLog::new(dir.path()).tail_since(0), "");
    }
}

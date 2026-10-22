//! Stat-gated polling of a file an agent is editing, for the live previews of
//! the assist engines (Design Hall, canvas). A turn can run for minutes while
//! the file stays unchanged; re-reading up to 25 MiB every poll only to find
//! the same bytes cost tens of MB/s. [`StatGate`] remembers the file's
//! `(len, modified)` and reads it only when that changes.

use std::path::Path;
use std::time::SystemTime;

/// What one [`StatGate::poll`] saw.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    /// Same `(len, modified)` as the previous poll: not read.
    Unchanged,
    /// The file is missing or unreadable.
    Missing,
    /// Changed, but larger than the cap (bytes): not read.
    TooLarge(u64),
    /// Changed: its full contents. (They can still equal an earlier read —
    /// callers keep their own content compare.)
    Changed(Vec<u8>),
}

/// Remembers the `(len, modified)` of the last poll of one file.
#[derive(Debug, Default)]
pub struct StatGate {
    last: Option<(u64, Option<SystemTime>)>,
}

impl StatGate {
    /// Stat `path`; read it (at most `cap` bytes) only when its signature
    /// changed since the previous call. The signature is taken BEFORE the
    /// read, so a write that lands during the read changes it again and the
    /// next poll re-reads.
    pub async fn poll(&mut self, path: &Path, cap: usize) -> Poll {
        let Ok(meta) = tokio::fs::metadata(path).await else {
            self.last = None;
            return Poll::Missing;
        };
        let sig = (meta.len(), meta.modified().ok());
        if self.last == Some(sig) {
            return Poll::Unchanged;
        }
        self.last = Some(sig);
        if sig.0 > cap as u64 {
            return Poll::TooLarge(sig.0);
        }
        match tokio::fs::read(path).await {
            Ok(bytes) => Poll::Changed(bytes),
            Err(_) => {
                self.last = None;
                Poll::Missing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_only_when_len_or_mtime_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("design.html");
        let mut gate = StatGate::default();
        assert_eq!(gate.poll(&p, 100).await, Poll::Missing);
        std::fs::write(&p, "one").unwrap();
        assert_eq!(gate.poll(&p, 100).await, Poll::Changed(b"one".to_vec()));
        assert_eq!(gate.poll(&p, 100).await, Poll::Unchanged);
        std::fs::write(&p, "three").unwrap();
        assert_eq!(gate.poll(&p, 100).await, Poll::Changed(b"three".to_vec()));
        // Same length, newer mtime: still a change.
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        std::fs::write(&p, "THREE").unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(gate.poll(&p, 100).await, Poll::Changed(b"THREE".to_vec()));
        std::fs::write(&p, "x".repeat(200)).unwrap();
        assert_eq!(gate.poll(&p, 100).await, Poll::TooLarge(200));
        assert_eq!(gate.poll(&p, 100).await, Poll::Unchanged, "warned once");
    }
}

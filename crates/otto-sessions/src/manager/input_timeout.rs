//! PTY input write deadlines. Split out of `manager.rs` (LOC ratchet).

use std::time::Duration;

/// How long [`super::SessionManager::input`] waits for its bytes to drain into the
/// PTY before reporting "not accepting input". The write itself runs on the
/// PTY's own writer thread (never a tokio worker) and stays queued in order,
/// so a timeout loses nothing — a 60 KB prompt paste into a TUI that is still
/// booting drains once it starts reading.
pub(super) const INPUT_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// [`INPUT_WRITE_TIMEOUT`] for a write of `len` bytes: a big paste drains
/// at the TUI's reading pace, so it gets one more second per 4 KiB, capped
/// at two minutes. A fixed 15 s failed a ~100 KB review-summarizer paste on a
/// loaded daemon, and the caller then abandons the session.
pub(super) fn input_write_timeout(len: usize) -> Duration {
    let extra = Duration::from_secs((len / 4096) as u64);
    (INPUT_WRITE_TIMEOUT + extra).min(Duration::from_secs(120))
}

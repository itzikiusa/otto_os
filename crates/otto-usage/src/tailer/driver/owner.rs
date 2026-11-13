//! Owner grace for transcripts whose Otto session is not known yet.
//!
//! Codex and Antigravity session ids are learnt AFTER the CLI starts: the
//! session manager scans the provider's files for the conversation it spawned
//! (and refuses to guess while several same-cwd spawns are unclaimed — exactly
//! what parallel review agents in one repo look like). A transcript the tailer
//! consumed before that landed as EXTERNAL and was never moved back, so the
//! first turns of those sessions were missing from the Otto totals, the
//! per-session row and the `review` bucket.
//!
//! So a YOUNG transcript whose owner lookup misses is left unconsumed: its
//! cursor, Codex baseline and Antigravity mark do not move and no event is
//! queued, so the retry on a later pass reads the same records exactly once.
//! Once the grace has passed, a transcript nobody claimed is external as
//! before. The wait clock is the file's birth time (it survives a restart)
//! capped by the first miss this daemon saw (a filesystem without birth
//! times, or a birth time in the future, still waits at most the grace).

use std::path::Path;
use std::time::{Duration, Instant};

/// The session manager keeps scanning for a spawn's rollout for 180 s after
/// its first input (`WINDOW_AFTER_INPUT`), and the rollout is born after that
/// input — an id the manager can still learn arrives within this long of the
/// file's birth.
pub(super) const OWNER_GRACE: Duration = Duration::from_secs(180);

impl super::UsageTailer {
    /// `file`'s owner lookup missed: should it stay unconsumed for now?
    /// Records the first miss; once the answer is no, forgets the file (the
    /// caller records it as external).
    pub(super) async fn wait_for_owner(&mut self, file: &Path) -> bool {
        let first_miss = *self
            .owner_wait
            .entry(file.to_path_buf())
            .or_insert_with(Instant::now);
        let grace = self.owner_grace;
        let born_young = tokio::fs::metadata(file)
            .await
            .and_then(|m| m.created())
            .ok()
            .and_then(|born| born.elapsed().ok())
            .is_none_or(|age| age < grace);
        let wait = born_young && first_miss.elapsed() < grace;
        if !wait {
            self.owner_wait.remove(file);
        }
        wait
    }

    /// A waiting rollout must survive a restart unread: a tracked cursor and
    /// a (zero) session baseline make the boot seed leave it alone instead of
    /// skipping it to EOF as pre-existing history. A zero baseline bills the
    /// first snapshot in full, exactly like no baseline.
    pub(super) fn pin_codex_for_owner(&mut self, file: &Path, session_id: &str) {
        if !self.cursors.contains(file) {
            self.cursors.set(file, 0);
        }
        if !self.codex_counters.contains(session_id) {
            self.codex_counters
                .seed(session_id, &crate::ParsedUsage::default());
            self.codex_counters_dirty = true;
        }
    }

    /// Drop waits whose transcript is gone, or that nothing has retried for
    /// twice the grace (a read that keeps failing) — an entry keeps the
    /// event-driven pass re-visiting its path.
    pub(super) fn forget_stale_owner_waits(&mut self) {
        if self.owner_wait.is_empty() {
            return;
        }
        let limit = self.owner_grace.saturating_mul(2);
        let waits = std::mem::take(&mut self.owner_wait);
        self.owner_wait = super::blocking_io(|| {
            waits
                .into_iter()
                .filter(|(f, since)| since.elapsed() < limit && f.exists())
                .collect()
        });
    }
}

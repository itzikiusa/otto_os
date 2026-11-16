//! Keep the Mac awake while an agent works (T3 Code #34128): one
//! `caffeinate -i -w <ottod pid>` is held while any Threads turn runs or any
//! terminal agent session is `working`, and released [`GRACE`] after the last
//! one ends. `-i` stops idle sleep only (closing the lid still sleeps), and
//! `-w` makes caffeinate exit with the daemon, so a crash never leaves the
//! Mac awake. One owner for the whole daemon, polled; never one per turn.

use std::time::{Duration, Instant};

use super::AgentRuntime;

const POLL: Duration = Duration::from_secs(15);
/// How long the hold outlives the last running turn.
const GRACE: Duration = Duration::from_secs(60);

/// Whether the hold should be on: something was busy within [`GRACE`] of
/// `now` (a busy poll sets `last_busy` to `now`).
fn want_hold(last_busy: Option<Instant>, now: Instant) -> bool {
    last_busy.is_some_and(|t| now.duration_since(t) < GRACE)
}

impl AgentRuntime {
    /// Start the daemon's keep-awake owner (macOS only; a no-op elsewhere).
    /// `sessions` answers whether a terminal agent session is `working`.
    pub fn spawn_keep_awake(&self, sessions: otto_state::SessionsRepo) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let rt = self.clone();
        tokio::spawn(async move {
            let pid = std::process::id().to_string();
            let mut hold: Option<tokio::process::Child> = None;
            let mut last_busy: Option<Instant> = None;
            loop {
                let now = Instant::now();
                if rt.any_turn_running() || sessions.any_working().await.unwrap_or(false) {
                    last_busy = Some(now);
                }
                // A caffeinate that exited on its own is gone; take a new one.
                if hold
                    .as_mut()
                    .is_some_and(|c| !matches!(c.try_wait(), Ok(None)))
                {
                    hold = None;
                }
                match (want_hold(last_busy, now), hold.is_some()) {
                    (true, false) => {
                        match tokio::process::Command::new("/usr/bin/caffeinate")
                            .args(["-i", "-w", &pid])
                            .kill_on_drop(true)
                            .spawn()
                        {
                            Ok(c) => hold = Some(c),
                            Err(e) => {
                                tracing::warn!("keep-awake: caffeinate did not start: {e}");
                                return;
                            }
                        }
                    }
                    (false, true) => hold = None, // dropping kills it
                    _ => {}
                }
                tokio::time::sleep(POLL).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hold_outlives_the_last_turn_by_the_grace_only() {
        let t0 = Instant::now();
        assert!(!want_hold(None, t0), "never busy: no hold");
        assert!(want_hold(Some(t0), t0), "busy now: hold");
        assert!(want_hold(Some(t0), t0 + Duration::from_secs(59)));
        assert!(!want_hold(Some(t0), t0 + GRACE));
    }
}

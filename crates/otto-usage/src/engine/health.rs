//! Notification-centre notices for a usage outage.
//!
//! The engine already recovers on its own (`recovery`) and the Usage page
//! shows `outage_since` / `journal_fill` — but only to someone who opens it.
//! On 2026-11-12 usage stayed down for six hours with the journal filling
//! toward its stop, and nothing said so anywhere else. This watcher raises
//! one warning in the bell per episode:
//! - the usage database down for more than [`OUTAGE_NOTICE_AFTER`];
//! - the ingestion journal more than [`JOURNAL_NOTICE_FILL`] full;
//!
//! and refreshes the same row (same `source_key`) with an info notice when
//! the episode ends. It reads two in-memory numbers once a minute; it never
//! wakes the ClickHouse server.

use std::sync::{Arc, Weak};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::domain::{NoticeAction, NoticeKind, NoticeSeverity};
use otto_core::event::Event;
use tokio::sync::broadcast;

use super::UsageEngine;

/// Down this long before the bell says so (the first retries run at 1 and
/// 3 minutes; a blip they fix stays quiet).
pub(super) const OUTAGE_NOTICE_AFTER: Duration = Duration::from_secs(5 * 60);
/// Journal fill that raises the notice. At 1.0 the tailer stops reading
/// transcripts, and what the CLIs prune meanwhile is lost.
pub(super) const JOURNAL_NOTICE_FILL: f64 = 0.5;
/// Fill below which a raised journal notice clears (a margin, so a journal
/// hovering at one half does not flip the notice every minute).
pub(super) const JOURNAL_CLEAR_FILL: f64 = 0.4;
const CHECK_EVERY: Duration = Duration::from_secs(60);
const OUTAGE_KEY: &str = "usage:outage";
const JOURNAL_KEY: &str = "usage:journal";

/// What changed since the last check.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Change {
    /// Down since this time, past [`OUTAGE_NOTICE_AFTER`].
    Down(DateTime<Utc>),
    /// Back up after a [`Change::Down`].
    Up,
    /// The journal passed [`JOURNAL_NOTICE_FILL`] (the fill, 0–1).
    JournalHigh(f64),
    /// Back under [`JOURNAL_CLEAR_FILL`] after a [`Change::JournalHigh`].
    JournalOk,
}

/// Which episodes have been announced. Pure: the caller passes the readings
/// and the clock.
#[derive(Debug, Default)]
pub(super) struct HealthWatch {
    outage: bool,
    journal: bool,
}

impl HealthWatch {
    pub(super) fn observe(
        &mut self,
        outage_since: Option<DateTime<Utc>>,
        journal_fill: Option<f64>,
        now: DateTime<Utc>,
    ) -> Vec<Change> {
        let mut out = Vec::new();
        match outage_since {
            Some(since)
                if !self.outage
                    && (now - since)
                        .to_std()
                        .is_ok_and(|d| d >= OUTAGE_NOTICE_AFTER) =>
            {
                self.outage = true;
                out.push(Change::Down(since));
            }
            None if self.outage => {
                self.outage = false;
                out.push(Change::Up);
            }
            _ => {}
        }
        // An unknown fill (journal not open yet) changes nothing.
        if let Some(fill) = journal_fill {
            if !self.journal && fill > JOURNAL_NOTICE_FILL {
                self.journal = true;
                out.push(Change::JournalHigh(fill));
            } else if self.journal && fill < JOURNAL_CLEAR_FILL {
                self.journal = false;
                out.push(Change::JournalOk);
            }
        }
        out
    }
}

/// The notice for `change`: (`source_key`, severity, title, body).
pub(super) fn notice_text(
    change: &Change,
    now: DateTime<Utc>,
    why: Option<&str>,
) -> (&'static str, NoticeSeverity, String, String) {
    match change {
        Change::Down(since) => {
            let mins = (now - *since).num_minutes().max(1);
            let why = why.map(|w| format!(" ({w})")).unwrap_or_default();
            (
                OUTAGE_KEY,
                NoticeSeverity::Warn,
                "Usage is not being recorded".into(),
                format!(
                    "The usage database has been down for {mins} minutes{why}. Otto keeps \
                     retrying in the background; token usage waits in the journal meanwhile."
                ),
            )
        }
        Change::Up => (
            OUTAGE_KEY,
            NoticeSeverity::Info,
            "Usage recording resumed".into(),
            "The usage database is back; usage held in the journal is being delivered.".into(),
        ),
        Change::JournalHigh(fill) => {
            let pct = (fill * 100.0).round();
            (
                JOURNAL_KEY,
                NoticeSeverity::Warn,
                format!("Usage journal {pct:.0}% full"),
                format!(
                    "Usage waiting for the usage database fills {pct:.0}% of the journal. At \
                     100% Otto stops reading new transcripts, and usage the agent CLIs prune \
                     meanwhile is lost."
                ),
            )
        }
        Change::JournalOk => (
            JOURNAL_KEY,
            NoticeSeverity::Info,
            "Usage journal drained".into(),
            "The usage journal is below 40% again.".into(),
        ),
    }
}

impl UsageEngine {
    /// Watch for usage outages and post them to the notification centre
    /// (persisted in `pool`, pushed live on `events`). The daemon calls it
    /// once at boot; the task holds the engine weakly and ends with it.
    pub fn spawn_health_notices(
        self: &Arc<Self>,
        pool: otto_state::DbPool,
        events: broadcast::Sender<Event>,
    ) {
        let weak: Weak<Self> = Arc::downgrade(self);
        tokio::spawn(async move {
            let repo = otto_state::NotificationsRepo::new(pool);
            let mut watch = HealthWatch::default();
            loop {
                tokio::time::sleep(CHECK_EVERY).await;
                let Some(engine) = weak.upgrade() else { return };
                if engine
                    .recovery
                    .shut_down
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
                let now = Utc::now();
                let since = engine.recovery.outage_started();
                let changes = watch.observe(since, engine.journal_fill().await, now);
                let why = engine.start_error();
                drop(engine);
                for change in changes {
                    let (key, severity, title, body) = notice_text(&change, now, why.as_deref());
                    let new = otto_state::NewNotice {
                        kind: NoticeKind::System,
                        severity,
                        title,
                        body,
                        source_key: Some(key.into()),
                        action: Some(NoticeAction::OpenRoute {
                            route: "usage".into(),
                            workspace_id: None,
                        }),
                        user_id: None,
                    };
                    match repo.create(new).await {
                        Ok(notice) => {
                            let _ = events.send(Event::Notification {
                                notice,
                                user_id: None,
                            });
                        }
                        Err(e) => tracing::warn!("usage health notice: {e}"),
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mins: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap() + chrono::Duration::minutes(mins)
    }

    #[test]
    fn an_outage_is_announced_once_after_five_minutes_and_its_end_once() {
        let mut w = HealthWatch::default();
        let since = at(0);
        assert!(w.observe(None, None, at(0)).is_empty(), "healthy");
        assert!(w.observe(Some(since), None, at(1)).is_empty(), "a blip");
        assert!(w.observe(Some(since), None, at(4)).is_empty());
        assert_eq!(
            w.observe(Some(since), None, at(5)),
            vec![Change::Down(since)]
        );
        assert!(w.observe(Some(since), None, at(30)).is_empty(), "only once");
        assert_eq!(w.observe(None, None, at(31)), vec![Change::Up]);
        assert!(w.observe(None, None, at(32)).is_empty());
        // A short outage that heals before five minutes says nothing at all.
        assert!(w.observe(Some(at(40)), None, at(42)).is_empty());
        assert!(w.observe(None, None, at(43)).is_empty());
    }

    #[test]
    fn the_journal_notice_rises_above_half_and_clears_below_forty_percent() {
        let mut w = HealthWatch::default();
        let now = at(0);
        assert!(
            w.observe(None, Some(0.5), now).is_empty(),
            "exactly half is fine"
        );
        assert_eq!(
            w.observe(None, Some(0.62), now),
            vec![Change::JournalHigh(0.62)]
        );
        assert!(w.observe(None, Some(0.9), now).is_empty(), "only once");
        assert!(
            w.observe(None, Some(0.45), now).is_empty(),
            "inside the margin"
        );
        assert!(
            w.observe(None, None, now).is_empty(),
            "unknown changes nothing"
        );
        assert_eq!(w.observe(None, Some(0.1), now), vec![Change::JournalOk]);
    }

    #[test]
    fn notices_share_a_key_per_episode_and_say_what_happened() {
        let since = at(0);
        let (k1, s1, t1, b1) =
            notice_text(&Change::Down(since), at(7), Some("server start failed"));
        assert_eq!((k1, s1), (OUTAGE_KEY, NoticeSeverity::Warn));
        assert_eq!(t1, "Usage is not being recorded");
        assert!(
            b1.contains("down for 7 minutes (server start failed)"),
            "{b1}"
        );
        let (k2, s2, ..) = notice_text(&Change::Up, at(8), None);
        assert_eq!(
            (k2, s2),
            (OUTAGE_KEY, NoticeSeverity::Info),
            "the recovery refreshes the row"
        );
        let (k3, _, t3, _) = notice_text(&Change::JournalHigh(0.734), at(0), None);
        assert_eq!(k3, JOURNAL_KEY);
        assert_eq!(t3, "Usage journal 73% full");
        assert_eq!(notice_text(&Change::JournalOk, at(0), None).0, JOURNAL_KEY);
    }
}

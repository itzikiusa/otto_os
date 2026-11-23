//! Embedded threads close themselves: a thread a person opened inside a PR
//! review or a connection page (`meta.source` in
//! [`EMBEDDED_THREAD_SOURCES`]) is closed after
//! `embedded_thread_auto_close_hours` (default 12, `0` = never) without
//! activity — the same Close a person clicks: the agent stops, the thread
//! stays in History and reopens on demand. Never deleted.
//!
//! A thread is left alone while a turn or background work runs, or while an
//! approval / question waits on the person (closing it would throw their
//! answer away). One pass closes at most [`AUTO_CLOSE_MAX_PER_PASS`]; the
//! rest wait for the next hour.
//!
//! Idle agent processes are released (T3 Code #32325, #34934): every minute,
//! a loaded thread whose agent process has been quiet for
//! `thread_idle_release_minutes` (default 30, `0` = never; a finished child
//! thread after at most 5) has that process — and its MCP servers — stopped,
//! as a Close stops it, but the thread stays open: its next message respawns
//! the agent, resuming the provider's conversation. The same guards hold,
//! plus nothing starting or queued.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{
    embedded_auto_close_hours, thread_idle_release_minutes, ProviderSessionStatus,
    RuntimeSessionStatus, CHILD_THREAD_IDLE_RELEASE_MINUTES, EMBEDDED_THREAD_AUTO_CLOSE_KEY,
    EMBEDDED_THREAD_SOURCES, THREAD_IDLE_RELEASE_KEY,
};
use otto_core::{Id, Result};

use super::{
    close_locked, has_pending, retire, set_session_status, stop_child, AgentRuntime, Inner, Live,
    State,
};

/// Candidate rows read per page.
const PAGE: u32 = 64;
/// Threads one pass closes at most.
const AUTO_CLOSE_MAX_PER_PASS: usize = 50;
/// The first pass after boot (a daemon that restarts more often than hourly
/// still sweeps), then hourly.
const FIRST_PASS_AFTER: Duration = Duration::from_secs(5 * 60);
const PASS_EVERY: Duration = Duration::from_secs(60 * 60);
const RELEASE_EVERY: Duration = Duration::from_secs(60);

/// `now - hours`; `None` = never (`0`, or a count past chrono's range).
fn idle_cutoff(now: DateTime<Utc>, hours: u64) -> Option<DateTime<Utc>> {
    if hours == 0 {
        return None;
    }
    let hours = chrono::TimeDelta::try_hours(i64::try_from(hours).ok()?)?;
    now.checked_sub_signed(hours)
}

/// Whether a loaded thread may be closed now: still open, nothing running,
/// nothing waiting on the person, and quiet since before `cutoff`.
fn idle_enough(st: &State, cutoff: DateTime<Utc>) -> bool {
    st.session.status != RuntimeSessionStatus::Closed
        && st.turn.is_none()
        && st.session.background_tasks.is_empty()
        && !has_pending(st)
        && st.session.last_active_at < cutoff
}

/// Whether a thread's agent process may be released now: it has one, nothing
/// is starting or queued, and [`idle_enough`] for its cutoff (`minutes` idle,
/// a child thread at most [`CHILD_THREAD_IDLE_RELEASE_MINUTES`]).
fn releasable(st: &State, minutes: u64, now: DateTime<Utc>) -> bool {
    let minutes = if st.session.child_of().is_some() {
        minutes.min(CHILD_THREAD_IDLE_RELEASE_MINUTES)
    } else {
        minutes
    };
    let cutoff = (minutes > 0)
        .then(|| i64::try_from(minutes).ok())
        .flatten()
        .and_then(chrono::TimeDelta::try_minutes)
        .and_then(|d| now.checked_sub_signed(d));
    st.proc.is_some()
        && st.starting == 0
        && st.proj.queue.is_empty()
        && cutoff.is_some_and(|c| idle_enough(st, c))
}

/// Stop the agent process the way a Close does (stdin EOF, then SIGTERM /
/// SIGKILL past the grace; a held child's holder exits with it, so a restart
/// finds nothing to adopt) and revoke its per-spawn credential, leaving the
/// thread open and `dormant`.
fn release_locked(inner: &Arc<Inner>, live: &Live, st: &mut State) {
    let Some(p) = st.proc.take() else {
        return;
    };
    live.proc_ended.notify_waiters();
    super::client_calls::drop_session(&st.session.id);
    stop_child(p, inner.config.close_grace);
    st.released = true;
    retire(inner, &st.session);
    set_session_status(inner, live, st, ProviderSessionStatus::Stopped, None);
}

impl AgentRuntime {
    /// One idle-release pass over the loaded threads (only those can hold a
    /// process) as of `now`. Returns the ids whose process it stopped.
    pub async fn release_idle_processes(&self, minutes: u64, now: DateTime<Utc>) -> Vec<Id> {
        let candidates: Vec<(Id, Arc<Live>)> = self
            .inner
            .live
            .iter()
            .filter(|e| releasable(&e.value().lock(), minutes, now))
            .map(|e| (e.key().clone(), Arc::clone(e.value())))
            .collect();
        let mut released = Vec::new();
        for (id, live) in candidates {
            // Under the prompt lock: a message arriving now either lands
            // first (and the process stays) or respawns it.
            let _prompt = live.prompt_lock.lock().await;
            let mut st = live.lock();
            if releasable(&st, minutes, now) {
                release_locked(&self.inner, &live, &mut st);
                released.push(id);
            }
        }
        if !released.is_empty() {
            tracing::info!(n = released.len(), "released idle thread agent processes");
        }
        released
    }

    /// One auto-close pass: close every open embedded thread idle for
    /// `hours` as of `now` (injected, so tests need no clock), at most
    /// [`AUTO_CLOSE_MAX_PER_PASS`]. Returns the ids it closed.
    pub async fn auto_close_idle_embedded(
        &self,
        hours: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<Id>> {
        let Some(cutoff) = idle_cutoff(now, hours) else {
            return Ok(Vec::new());
        };
        let mut closed = Vec::new();
        let mut after = String::new();
        loop {
            let page = self
                .inner
                .repo
                .idle_open_ids_with_source(&EMBEDDED_THREAD_SOURCES, cutoff, &after, PAGE)
                .await?;
            let full = page.len() >= PAGE as usize;
            let Some(last) = page.last() else {
                break;
            };
            after = last.clone();
            for id in page {
                if closed.len() >= AUTO_CLOSE_MAX_PER_PASS {
                    return Ok(closed);
                }
                match self.close_if_idle(&id, cutoff).await {
                    Ok(true) => {
                        tracing::info!(session = %id, hours, "closed an embedded thread idle past its auto-close time");
                        closed.push(id);
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!(session = %id, "embedded thread auto-close: {e}"),
                }
            }
            if !full {
                break;
            }
        }
        Ok(closed)
    }

    /// Close `id` when [`idle_enough`] holds — checked under the prompt lock
    /// and the session lock, so a prompt arriving now either lands first (and
    /// the thread stays) or finds it closed.
    async fn close_if_idle(&self, id: &Id, cutoff: DateTime<Utc>) -> Result<bool> {
        let closed = {
            let live = self.live(id).await?;
            let _prompt = live.prompt_lock.lock().await;
            let mut st = live.lock();
            if !idle_enough(&st, cutoff) {
                false
            } else {
                close_locked(&self.inner, &live, &mut st);
                true
            }
        };
        if closed {
            super::evict_if_dead_after_flush(&self.inner, id).await;
        } else {
            super::try_evict_dead(&self.inner, id);
        }
        Ok(closed)
    }

    /// The daemon's auto-close loop: a pass shortly after boot, then hourly,
    /// each reading `embedded_thread_auto_close_hours` afresh.
    /// Also starts the idle-release loop (every minute, reading
    /// `thread_idle_release_minutes` afresh).
    pub fn spawn_embedded_auto_close(&self, settings: otto_state::SettingsRepo) {
        let rt = self.clone();
        let release_settings = settings.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RELEASE_EVERY).await;
                match release_settings.get(THREAD_IDLE_RELEASE_KEY).await {
                    Ok(v) => {
                        let minutes = thread_idle_release_minutes(v.as_ref());
                        rt.release_idle_processes(minutes, Utc::now()).await;
                    }
                    Err(e) => tracing::warn!("idle release: settings unreadable: {e}"),
                }
            }
        });
        let rt = self.clone();
        tokio::spawn(async move {
            let mut wait = FIRST_PASS_AFTER;
            loop {
                tokio::time::sleep(wait).await;
                wait = PASS_EVERY;
                let hours = match settings.get(EMBEDDED_THREAD_AUTO_CLOSE_KEY).await {
                    Ok(v) => embedded_auto_close_hours(v.as_ref()),
                    Err(e) => {
                        tracing::warn!("embedded thread auto-close: settings unreadable: {e}");
                        continue;
                    }
                };
                if let Err(e) = rt.auto_close_idle_embedded(hours, Utc::now()).await {
                    tracing::warn!("embedded thread auto-close failed: {e}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_cutoff_is_never_for_zero_and_out_of_range_hours() {
        let now = Utc::now();
        assert_eq!(idle_cutoff(now, 0), None);
        assert_eq!(
            idle_cutoff(now, 12),
            Some(now - chrono::TimeDelta::hours(12))
        );
        assert_eq!(idle_cutoff(now, u64::MAX), None);
        assert_eq!(idle_cutoff(now, i64::MAX as u64), None);
    }
}

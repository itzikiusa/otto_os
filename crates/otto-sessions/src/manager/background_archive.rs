//! Auto-archive for engine-owned BACKGROUND agent sessions (review agents,
//! workflow steps, summarizers, scheduled-task runs, … — the
//! [`otto_core::domain::BACKGROUND_SESSION_SOURCES`] the user never opened
//! themselves). Split out of `manager.rs` (LOC ratchet).
//!
//! They arrive by the thousand and are never looked at again, yet stayed
//! unarchived ("reconnectable") forever unless the user opted into
//! `session_auto_archive_days`, which treats every session alike. This rule
//! is independent and ON by default: `session_auto_archive_background_days`
//! (absent = [`DEFAULT_BACKGROUND_ARCHIVE_DAYS`], 0 = off). Foreground
//! sessions keep following `session_auto_archive_days` only. Archiving is
//! non-destructive: the row and transcript stay and the session can be
//! unarchived and resumed.

use super::*;

/// Settings key: days of inactivity before a background agent session is
/// archived. Absent = [`DEFAULT_BACKGROUND_ARCHIVE_DAYS`]; 0 (or a value that
/// is not a non-negative integer) = off.
pub(super) const BACKGROUND_ARCHIVE_DAYS_KEY: &str = "session_auto_archive_background_days";

/// [`BACKGROUND_ARCHIVE_DAYS_KEY`] when the setting is absent.
pub(super) const DEFAULT_BACKGROUND_ARCHIVE_DAYS: u64 = 7;

/// Most sessions one hourly pass archives. The first pass on a long-lived
/// install meets thousands of candidates; each archive is a few short writes,
/// so a bounded batch keeps the pass from monopolising the database (the rest
/// follow on later passes, oldest first).
pub(super) const BACKGROUND_ARCHIVE_BATCH: usize = 200;

/// Candidate ids read per page (ids + timestamps only, never `meta_json`).
const CANDIDATE_PAGE: u32 = 100;

impl SessionManager {
    /// The background rule's day count. A settings read error skips this pass
    /// (the next hourly pass retries) rather than falling back to the default.
    async fn background_archive_days(&self) -> u64 {
        let Some(sr) = &self.settings else {
            return DEFAULT_BACKGROUND_ARCHIVE_DAYS;
        };
        match sr.get(BACKGROUND_ARCHIVE_DAYS_KEY).await {
            Ok(Some(v)) => v.as_u64().unwrap_or(0),
            Ok(None) => DEFAULT_BACKGROUND_ARCHIVE_DAYS,
            Err(e) => {
                tracing::warn!(
                    "background auto-archive: reading {BACKGROUND_ARCHIVE_DAYS_KEY}: {e}"
                );
                0
            }
        }
    }

    /// One pass of the background rule (driven by the hourly
    /// [`Self::auto_archive_stale`]). Returns the number archived.
    pub(super) async fn auto_archive_background(&self) -> usize {
        let days = self.background_archive_days().await;
        if days == 0 {
            return 0;
        }
        let Some(cutoff) = auto_archive_cutoff(chrono::Utc::now(), days) else {
            tracing::warn!(
                days,
                "background auto-archive: day count out of range; treating as off"
            );
            return 0;
        };
        let n = self
            .archive_background_before(cutoff, BACKGROUND_ARCHIVE_BATCH)
            .await;
        if n > 0 {
            tracing::info!("auto-archived {n} background session(s) idle for over {days} day(s)");
        }
        n
    }

    /// Archive up to `cap` background agent sessions last active before
    /// `cutoff`, oldest first. Live, attached and `keep_alive`-pinned sessions
    /// are skipped (re-checked under each session's resume lock, like
    /// [`Self::archive_if_stale`]); the keyset cursor advances over skipped
    /// rows so they never starve later candidates.
    pub(super) async fn archive_background_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        cap: usize,
    ) -> usize {
        let (mut after_at, mut after_id) = (String::new(), String::new());
        let mut archived = 0;
        while archived < cap {
            let page = match self
                .repo
                .background_archive_candidates(cutoff, &after_at, &after_id, CANDIDATE_PAGE)
                .await
            {
                Ok(page) => page,
                Err(e) => {
                    tracing::warn!("background auto-archive: candidate page failed: {e}");
                    break;
                }
            };
            let Some((last_id, last_at)) = page.last() else {
                break;
            };
            after_id.clone_from(last_id);
            after_at.clone_from(last_at);
            for (id, _) in page {
                if archived >= cap {
                    break;
                }
                if self.is_live(&id) || self.is_attached(&id) {
                    continue;
                }
                match self.archive_background_if_stale(&id, cutoff).await {
                    Ok(true) => archived += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!(session = %id, "background auto-archive failed: {e}"),
                }
            }
        }
        archived
    }

    /// Archive one background session if it is STILL an eligible, stale,
    /// dormant background session under its resume lock. `Ok(false)` = no
    /// longer eligible (opened, pinned or re-sourced since the page was read).
    ///
    /// Quiet on purpose: [`Self::archive_locked`] sends `session_status` and
    /// `session_archive_changed` (and records a trail entry, which sends
    /// `trail_appended`) per session — a first pass over thousands of rows
    /// would push every one of them to every connected owner/admin client.
    /// Background sessions are not in the sidebar list, and the pages that
    /// show them (reviews, workflow runs, …) re-read on their own; the row
    /// has no PTY (checked under the lock), so there is no lifecycle change
    /// for a live consumer to observe.
    async fn archive_background_if_stale(
        &self,
        id: &Id,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        let session = self.repo.get(id).await?;
        if session.is_foreground_agent()
            || !is_auto_archive_candidate(&session, cutoff)
            || self.is_live(id)
            || self.is_attached(id)
        {
            return Ok(false);
        }
        self.networks.clear(id);
        self.retire_credentials(&session).await;
        self.repo.set_archived(id, true).await?;
        // Stamps `last_active_at` like every archive does: the archived-channel
        // purge deletes rows by that age, so keeping the old stamp would turn
        // this archive into an immediate delete of `channel` sessions.
        self.repo.update_status(id, SessionStatus::Exited).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn manager() -> (Arc<SessionManager>, SessionsRepo, Id, Id) {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        std::mem::forget(dir);
        let (user, ws) = (otto_core::new_id(), otto_core::new_id());
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'u', 'x', 'U', 0, ?)")
            .bind(&user).bind(&now).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)",
        )
        .bind(&ws)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let repo = SessionsRepo::new(pool);
        let (events, _rx) = broadcast::channel(64);
        let mgr = SessionManager::new(repo.clone(), events, ProviderRegistry::new(None));
        (Arc::new(mgr), repo, ws, user)
    }

    /// Insert an unarchived, dormant agent row with `meta` last active at `at`.
    async fn seed(
        repo: &SessionsRepo,
        ws: &Id,
        user: &Id,
        id: &str,
        at: &str,
        meta: serde_json::Value,
    ) {
        sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at,archived,meta_json) VALUES(?,?,'agent','claude','t','reconnectable','/tmp',?,?,?,0,?)")
            .bind(id).bind(ws).bind(user).bind(at).bind(at).bind(meta.to_string())
            .execute(&repo.pool()).await.unwrap();
    }

    async fn archived(repo: &SessionsRepo, id: &str) -> bool {
        repo.get(&id.to_string()).await.unwrap().archived
    }

    const OLD: &str = "2000-01-01T00:00:00+00:00";

    #[tokio::test]
    async fn old_background_sessions_are_archived_quietly_and_everything_else_kept() {
        let (mgr, repo, ws, user) = manager().await;
        let bg = serde_json::json!({"source": "review"});
        let recent = chrono::Utc::now().to_rfc3339();
        seed(&repo, &ws, &user, "bg-old", OLD, bg.clone()).await;
        seed(&repo, &ws, &user, "bg-recent", &recent, bg.clone()).await;
        seed(&repo, &ws, &user, "fg-old", OLD, serde_json::json!({})).await;
        seed(
            &repo,
            &ws,
            &user,
            "bg-pinned",
            OLD,
            serde_json::json!({"source": "workflow", "keep_alive": true}),
        )
        .await;
        seed(&repo, &ws, &user, "bg-attached", OLD, bg.clone()).await;
        seed(&repo, &ws, &user, "bg-live", OLD, bg.clone()).await;
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            cwd: None,
            env: vec![],
        };
        let handle = Arc::new(PtyHandle::spawn(&spec).expect("spawn"));
        mgr.live.insert("bg-live".into(), handle);
        let guard = mgr.attach(&"bg-attached".to_string());
        let mut rx = mgr.events.subscribe();

        // No settings store → the background default (7 days) applies, and the
        // foreground `session_auto_archive_days` rule stays off.
        assert_eq!(mgr.auto_archive_stale().await, 1);
        assert!(archived(&repo, "bg-old").await);
        for kept in ["bg-recent", "fg-old", "bg-pinned", "bg-attached", "bg-live"] {
            assert!(!archived(&repo, kept).await, "{kept} must stay unarchived");
        }
        assert_eq!(
            repo.get(&"bg-old".to_string()).await.unwrap().status,
            SessionStatus::Exited
        );
        // The bulk path sends no per-session event.
        while let Ok(ev) = rx.try_recv() {
            assert!(
                !matches!(
                    ev,
                    Event::SessionStatus { .. }
                        | Event::SessionArchiveChanged { .. }
                        | Event::TrailAppended { .. }
                ),
                "unexpected per-session event {ev:?}"
            );
        }
        drop(guard);
        mgr.live.remove(&"bg-live".to_string());
    }

    /// One pass archives at most `cap`, oldest first; the rest follow later.
    #[tokio::test]
    async fn a_pass_archives_at_most_the_batch_oldest_first() {
        let (mgr, repo, ws, user) = manager().await;
        for i in 0..5 {
            let at = format!("2000-01-0{}T00:00:00+00:00", i + 1);
            seed(
                &repo,
                &ws,
                &user,
                &format!("bg-{i}"),
                &at,
                serde_json::json!({"source": "review_summarizer"}),
            )
            .await;
        }
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::days(7);
        assert_eq!(mgr.archive_background_before(cutoff, 3).await, 3);
        for i in 0..5 {
            assert_eq!(archived(&repo, &format!("bg-{i}")).await, i < 3, "bg-{i}");
        }
        assert_eq!(mgr.archive_background_before(cutoff, 3).await, 2);
        assert_eq!(mgr.archive_background_before(cutoff, 3).await, 0);
    }

    /// `0` turns the background rule off; a foreground session is archived
    /// only by the foreground setting.
    #[tokio::test]
    async fn the_settings_switch_each_rule_independently() {
        let (mut mgr, repo, ws, user) = manager().await;
        let settings = otto_state::SettingsRepo::new(repo.pool());
        Arc::get_mut(&mut mgr).unwrap().settings = Some(settings.clone());
        seed(
            &repo,
            &ws,
            &user,
            "bg",
            OLD,
            serde_json::json!({"source": "scheduled_task"}),
        )
        .await;
        seed(&repo, &ws, &user, "fg", OLD, serde_json::json!({})).await;
        settings
            .put(BACKGROUND_ARCHIVE_DAYS_KEY, &serde_json::json!(0))
            .await
            .unwrap();
        assert_eq!(mgr.auto_archive_stale().await, 0);
        assert!(!archived(&repo, "bg").await && !archived(&repo, "fg").await);
        settings
            .put(BACKGROUND_ARCHIVE_DAYS_KEY, &serde_json::json!(30))
            .await
            .unwrap();
        assert_eq!(mgr.auto_archive_stale().await, 1);
        assert!(archived(&repo, "bg").await && !archived(&repo, "fg").await);
        settings
            .put("session_auto_archive_days", &serde_json::json!(30))
            .await
            .unwrap();
        assert_eq!(mgr.auto_archive_stale().await, 1);
        assert!(archived(&repo, "fg").await);
    }
}

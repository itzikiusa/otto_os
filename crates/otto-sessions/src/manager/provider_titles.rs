//! The provider-title auto-namer sweep. Split out of `manager.rs` (LOC ratchet).

use super::*;

impl SessionManager {
    /// One sweep of the provider-title auto-namer. For every LIVE foreground
    /// agent session whose name the user hasn't claimed, read the provider's own
    /// session title (claude/codex first user prompt) and, when found and
    /// different, rename the session to it and broadcast `SessionRenamed`.
    ///
    /// Cheap + robust: only the live, not-yet-resolved ids are read from the
    /// table (a primary-key lookup — thousands of dormant rows match the column
    /// filter alone); dead/archived/background/user-named sessions are skipped
    /// without any disk work; an unchanged transcript (same mtime) is not
    /// re-parsed; a resolved session is never read again; and a per-session
    /// error never stops the sweep. All file reads run on the blocking pool.
    /// Returns the number of sessions renamed this sweep. Driven by a ~20s loop
    /// in ottod.
    pub async fn refresh_provider_titles(&self) -> usize {
        // Live ids the in-memory probe cache has not settled: at most the
        // handful of PTYs this daemon runs (SG-06 + the dormant-row fix: the
        // SQL used to decode every non-exited "reconnectable" row every 20 s).
        let ids: Vec<Id> = self
            .live
            .iter()
            .map(|e| e.key().clone())
            .filter(|id| {
                !self
                    .title_probe
                    .get(id)
                    .is_some_and(|p| p.resolved || p.gave_up)
            })
            .collect();
        if ids.is_empty() {
            return 0;
        }
        let sessions = match self.repo.list_title_candidates(&ids).await {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::warn!("provider-title sweep: reading candidates failed: {e}");
                return 0;
            }
        };
        let mut renamed = 0;
        for session in sessions {
            // Live only (the doc's contract): the id list was live when read,
            // but a session can exit while an earlier one is probed.
            if !title_eligible(&session) || !self.is_live(&session.id) {
                continue;
            }
            match self.probe_and_apply_title(&session).await {
                Ok(true) => renamed += 1,
                Ok(false) => {}
                Err(e) => tracing::debug!(session = %session.id, "provider-title probe: {e}"),
            }
        }
        renamed
    }

    /// Probe one session's transcript and adopt its provider title when found.
    /// Returns `Ok(true)` when the session was actually renamed. Uses the
    /// in-memory [`TitleProbe`] cache to short-circuit resolved sessions and
    /// unchanged transcripts; does the file read on the blocking pool.
    async fn probe_and_apply_title(&self, s: &Session) -> Result<bool> {
        // Already resolved in this daemon's lifetime — never re-read.
        if self
            .title_probe
            .get(&s.id)
            .is_some_and(|p| p.resolved || p.gave_up)
        {
            return Ok(false);
        }
        let Some(path) = self.activity_artifact(&s.id).await else {
            // Transcript not on disk yet (id just captured, first turn pending).
            return Ok(false);
        };
        // Async stat: this runs every ~20 s per candidate on a runtime worker.
        let mtime = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .ok();
        // Nothing new since the last look — skip the re-parse.
        if let Some(prev) = self.title_probe.get(&s.id) {
            if mtime.is_some() && prev.mtime == mtime {
                return Ok(false);
            }
        }
        let provider = s.provider.clone();
        let path_owned = path.clone();
        let read = tokio::task::spawn_blocking(move || read_provider_title(&provider, &path_owned))
            .await
            .unwrap_or_default();
        let title = read.title;
        // Record the mtime we just parsed at so the next sweep can skip it.
        {
            let mut probe = self.title_probe.entry(s.id.clone()).or_default();
            probe.mtime = mtime;
            // A found title means the (stable) first prompt exists — done either
            // way, whether it differs from the current name or already matches.
            if title.is_some() {
                probe.resolved = true;
            } else if read.capped {
                probe.gave_up = true;
            }
        }
        let Some(title) = title else {
            return Ok(false); // no user prompt yet — retry on a later sweep
        };
        if title == s.title {
            return Ok(false);
        }
        self.repo.set_title(&s.id, &title).await?;
        // Without the durable marker the session stays a candidate and is
        // re-probed after every daemon restart (the probe cache is in-memory).
        if let Err(e) = self
            .repo
            .merge_meta(&s.id, &serde_json::json!({ "title_source": "provider" }))
            .await
        {
            tracing::warn!(session = %s.id, "provider-title: recording title_source failed: {e}");
        }
        let _ = self.events.send(Event::SessionRenamed {
            session_id: s.id.clone(),
            workspace_id: s.workspace_id.clone(),
            title: title.clone(),
        });
        self.record_lifecycle(s, format!("Auto-named from {} session", s.provider));
        Ok(true)
    }
}

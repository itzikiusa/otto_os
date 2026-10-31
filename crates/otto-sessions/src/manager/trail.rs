//! Activity-trail recording helpers for [`super::SessionManager`].

use otto_core::domain::{Session, SessionKind, TrailKind, TrailLevel, TrailSource};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::NewTrail;

use super::SessionManager;

/// Truncate `s` to at most `max` chars (char-boundary safe), appending `…`.
/// Used for one-line trail summaries.
fn trail_clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

impl SessionManager {
    /// Best-effort: persist a trail entry and broadcast it. Fire-and-forget so
    /// callers (lifecycle methods) never block on the DB. No-op without an
    /// activity store.
    pub(super) fn record_trail(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        source: TrailSource,
        kind: TrailKind,
        level: TrailLevel,
        summary: String,
    ) {
        self.record_trail_with_detail(session_id, workspace_id, source, kind, level, summary, None);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_trail_with_detail(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        source: TrailSource,
        kind: TrailKind,
        level: TrailLevel,
        summary: String,
        detail: Option<serde_json::Value>,
    ) {
        let Some(repo) = self.activity.clone() else {
            return;
        };
        let events = self.events.clone();
        let (sid, wid) = (session_id.clone(), workspace_id.clone());
        tokio::spawn(async move {
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: wid.clone(),
                source,
                kind,
                level,
                summary,
                detail,
            };
            match repo.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: wid,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!(session = %sid, "record trail: {e}"),
            }
        });
    }

    /// Record an Otto-side lifecycle entry (spawn/suspend/archive/…) for an
    /// agent session. Skips connection sessions to keep the trail agent-focused.
    pub(super) fn record_lifecycle(&self, session: &Session, summary: impl Into<String>) {
        if session.kind != SessionKind::Agent {
            return;
        }
        self.record_trail(
            &session.id,
            &session.workspace_id,
            TrailSource::Otto,
            TrailKind::Session,
            TrailLevel::Info,
            summary.into(),
        );
    }

    /// Record a user-authored message relayed into a session (channel relay,
    /// orchestrator command). Surfaces the "by user" side of the trail for every
    /// provider. Best-effort; loads the session to resolve its workspace.
    pub async fn record_user_message(&self, session_id: &Id, text: &str) {
        self.record_relayed_message(session_id, TrailSource::User, None, text)
            .await;
    }

    /// Record a message another agent session sent into this one with its own
    /// credential (`/message`, broadcast, relay — S11-305). Attributed to the
    /// agent and the sending session, never to the person who owns both.
    pub async fn record_agent_message(&self, session_id: &Id, from: &Id, text: &str) {
        self.record_relayed_message(session_id, TrailSource::Agent, Some(from), text)
            .await;
    }

    async fn record_relayed_message(
        &self,
        session_id: &Id,
        source: TrailSource,
        from: Option<&Id>,
        text: &str,
    ) {
        if self.activity.is_none() {
            return;
        }
        let Ok(session) = self.repo.get(session_id).await else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let (summary, detail) = match from {
            Some(from) => {
                let sender_title = self
                    .repo
                    .get(from)
                    .await
                    .ok()
                    .map(|s| s.title.trim().to_string())
                    .filter(|t| !t.is_empty());
                let summary = match &sender_title {
                    Some(title) => trail_clip(&format!("from {title} ({from}): {trimmed}"), 200),
                    None => trail_clip(&format!("from session {from}: {trimmed}"), 200),
                };
                let detail = serde_json::json!({
                    "from_session_id": from,
                    "from_title": sender_title,
                    "text": trimmed,
                });
                (summary, Some(detail))
            }
            None => (trail_clip(trimmed, 200), None),
        };
        self.record_trail_with_detail(
            session_id,
            &session.workspace_id,
            source,
            TrailKind::Prompt,
            TrailLevel::Info,
            summary,
            detail,
        );
    }

    /// Record an auto-approved prompt-guard action on the session activity trail.
    /// Called by [`crate::prompt_guard::PromptGuard`] after injecting approval
    /// keys. Best-effort; no-op when the activity store is absent.
    pub fn record_approval_trail(&self, session_id: &Id, provider: &str) {
        let summary = format!("Auto-approved trust/permission prompt for {provider}");
        let repo = self.repo.clone();
        let sid = session_id.clone();
        let activity = self.activity.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let Ok(session) = repo.get(&sid).await else {
                return;
            };
            let Some(activity) = activity else {
                return;
            };
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: session.workspace_id.clone(),
                source: TrailSource::Otto,
                kind: TrailKind::Session,
                level: TrailLevel::Info,
                summary,
                detail: None,
            };
            match activity.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: session.workspace_id,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!("prompt-guard record trail: {e}"),
            }
        });
    }
}

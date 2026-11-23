//! "Resume at reset": a thread whose turn ended on the provider's usage
//! limit continues ONCE when the window resets (T3 Code's
//! UsageLimitRecoveryWorker, MIT). The time lives in the session's meta
//! (`resume_at`, RFC 3339), so it survives a daemon restart (`recover`
//! re-arms it); clearing it cancels, and a resume is skipped when the time
//! was changed or cleared meanwhile or the thread was closed.

use chrono::{DateTime, Utc};
use otto_core::agent_runtime::RuntimeSession;
use otto_core::{Id, Result};
use serde_json::Value;

use super::super::AgentRuntime;

/// The session-meta key holding the armed resume time (sent to clients
/// with the thread meta, `provider_session.updated` `thread`).
pub use otto_core::agent_runtime::RESUME_AT;
/// What the agent is told when the limit has reset.
pub const RESUME_TEXT: &str = "Your usage limit has reset. Continue where you left off.";

/// The armed resume time of `s`, if any.
pub fn resume_at_of(s: &RuntimeSession) -> Option<DateTime<Utc>> {
    resume_at_in(s.meta.as_ref())
}

/// The armed resume time in a session `meta`, if any.
pub fn resume_at_in(meta: Option<&Value>) -> Option<DateTime<Utc>> {
    let v = meta?.get(RESUME_AT)?.as_str()?;
    DateTime::parse_from_rfc3339(v)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl AgentRuntime {
    /// Arm the thread's one resume at `at`, or cancel it (`None`).
    pub async fn set_resume_at(
        &self,
        id: &Id,
        at: Option<DateTime<Utc>>,
    ) -> Result<RuntimeSession> {
        let mut patch = serde_json::Map::new();
        patch.insert(
            RESUME_AT.into(),
            at.map_or(Value::Null, |t| Value::String(t.to_rfc3339())),
        );
        let s = self.update_thread_meta(id, patch).await?;
        if let Some(t) = at {
            self.arm_resume(id.clone(), t);
        }
        Ok(s)
    }

    /// Sleep until `at`, then resume if it is still the armed time.
    fn arm_resume(&self, id: Id, at: DateTime<Utc>) {
        let rt = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep((at - Utc::now()).to_std().unwrap_or_default()).await;
            let Ok(rec) = rt.inner.repo.get(&id).await else {
                return;
            };
            if rec.closed || resume_at_of(&rec.session) != Some(at) {
                return; // cancelled, re-armed for another time, or closed
            }
            // Only a thread still stopped on its limit (the person may have
            // sent something themselves meanwhile).
            let limited = rt.get(&id).await.is_ok_and(|s| {
                s.last_error_class.as_deref()
                    == Some(otto_core::agent_runtime::ERROR_CLASS_USAGE_LIMIT)
            });
            if rt.set_resume_at(&id, None).await.is_ok() && limited {
                let _ = rt.prompt(&id, RESUME_TEXT).await;
            }
        });
    }

    /// After `recover`: re-arm every stored resume (one already due fires
    /// now). Returns `touched` unchanged, so `recover` can hand it on.
    pub(in crate::runtime) fn arm_resumes(&self, touched: usize) -> usize {
        let rt = self.clone();
        tokio::spawn(async move {
            let Ok(ids) = rt.inner.repo.session_ids().await else {
                return;
            };
            for sid in ids {
                if let Ok(rec) = rt.inner.repo.get(&sid).await {
                    if let (false, Some(at)) = (rec.closed, resume_at_of(&rec.session)) {
                        rt.arm_resume(sid, at);
                    }
                }
            }
        });
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_armed_time_from_the_meta() {
        assert_eq!(resume_at_in(None), None);
        let at = serde_json::json!({ RESUME_AT: "2026-11-23T14:00:00Z" });
        assert_eq!(
            resume_at_in(Some(&at)).map(|t| t.to_rfc3339()),
            Some("2026-11-23T14:00:00+00:00".into())
        );
        assert_eq!(
            resume_at_in(Some(&serde_json::json!({ RESUME_AT: "soon" }))),
            None
        );
    }
}

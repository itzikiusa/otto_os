//! Engine threads are kept a while after they close, then deleted: a step's
//! close stamps `meta.retain_until` on the thread and on the sub-threads
//! its agent opened, the engine moves the date when the run ends, and one
//! sweep deletes closed threads past it. A thread without the key (every
//! person's thread) is never touched; a previous build ignores the key.

use chrono::{DateTime, Utc};

use super::*;

pub const RETAIN_UNTIL_KEY: &str = "retain_until";
/// Rows one sweep deletes at most.
const SWEEP_MAX: u32 = 200;

/// `retain_until` as stored: RFC 3339, milliseconds, `Z` — the form
/// `AgentRuntimeRepo::retained_due` compares as text.
pub fn retain_stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl AgentRuntime {
    /// The thread and, when it leads a linked group (an engine thread
    /// always does), every sub-thread of the group, closed ones too.
    async fn with_sub_threads(&self, id: &Id) -> Result<Vec<RuntimeSession>> {
        let me = self.get(id).await?;
        if me.parent_id.is_some() {
            return Ok(vec![me]);
        }
        self.linked(id).await
    }

    async fn stamp(&self, id: &Id, retain_until: DateTime<Utc>) -> Result<()> {
        let mut patch = serde_json::Map::new();
        patch.insert(RETAIN_UNTIL_KEY.into(), retain_stamp(retain_until).into());
        self.update_thread_meta(id, patch).await.map(|_| ())
    }

    /// Stamp `retain_until` on the thread and its sub-threads, then close
    /// them all (an open sub-thread stops with its step).
    pub async fn close_retained(
        &self,
        id: &Id,
        retain_until: DateTime<Utc>,
    ) -> Result<RuntimeSession> {
        for s in self.with_sub_threads(id).await? {
            self.stamp(&s.id, retain_until).await?;
            if s.id != *id && s.status != RuntimeSessionStatus::Closed {
                self.close(&s.id).await?;
            }
        }
        self.close(id).await
    }

    /// Move `retain_until` on each thread and its sub-threads, open or
    /// closed; an id that is already gone is skipped.
    pub async fn set_retain_until(&self, ids: &[Id], retain_until: DateTime<Utc>) -> Result<()> {
        for id in ids {
            let all = match self.with_sub_threads(id).await {
                Ok(all) => all,
                Err(Error::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            for s in all {
                self.stamp(&s.id, retain_until).await?;
            }
        }
        Ok(())
    }

    /// Delete the closed threads past their `retain_until` (at most 200 a
    /// pass). Returns how many went.
    pub async fn sweep_retained(&self, now: DateTime<Utc>) -> Result<usize> {
        let due = self.inner.repo.retained_due(now, SWEEP_MAX).await?;
        let mut n = 0;
        for id in due {
            match self.delete(&id).await {
                Ok(()) => n += 1,
                Err(e) => tracing::warn!(session = %id, "retention: could not delete: {e}"),
            }
        }
        Ok(n)
    }
}

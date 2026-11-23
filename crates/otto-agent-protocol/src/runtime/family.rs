//! One Stop for a thread and its subagents (T3 Code: "Stop on a thread also
//! stops the subagents it delegated to", `task_cancel`, MIT, Copyright (c)
//! 2026 T3 Tools Inc.): the thread's running turn, and every open subagent
//! opened from it — and theirs — with its turn and its background work; with
//! `background`, the thread's own background work too (the waiting line's
//! Stop). A subagent stopped this way does not wake the thread again: its
//! "was stopped" reply is dropped (the notice still says it), and replies
//! already queued for the thread are removed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use otto_core::agent_runtime::{FamilyStopResp, RuntimeSessionStatus};

use super::*;

/// How long after a family stop a subagent's "stopped" reply is dropped
/// (its interrupted turn ends asynchronously).
const QUIET: Duration = Duration::from_secs(20);

fn stopped() -> &'static Mutex<HashMap<Id, Instant>> {
    static S: std::sync::OnceLock<Mutex<HashMap<Id, Instant>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// Whether `parent` was family-stopped within [`QUIET`] (no wake then).
pub(super) fn recently_stopped(parent: &Id) -> bool {
    let mut m = stopped().lock().unwrap_or_else(|e| e.into_inner());
    m.retain(|_, at| at.elapsed() < QUIET);
    m.contains_key(parent)
}

fn mark_stopped(ids: &[Id]) {
    let mut quiet = stopped().lock().unwrap_or_else(|e| e.into_inner());
    for sid in ids {
        quiet.insert(sid.clone(), Instant::now());
    }
}

/// `id`, then every open thread opened from one already listed, in order.
pub(super) fn family_ids(id: &Id, rows: &[RuntimeSession]) -> Vec<Id> {
    let mut ids = vec![id.clone()];
    let mut i = 0;
    while i < ids.len() {
        let parent = ids[i].clone();
        for r in rows {
            let child = r.child_of().is_some_and(|o| o.id == parent);
            if child && r.status != RuntimeSessionStatus::Closed && !ids.contains(&r.id) {
                ids.push(r.id.clone());
            }
        }
        i += 1;
    }
    ids
}

impl AgentRuntime {
    /// `POST /runtime-sessions/{id}/stop-family`.
    pub async fn stop_family(&self, id: &Id, background: bool) -> Result<FamilyStopResp> {
        let me = self.get(id).await?;
        let rows = self.list(&me.workspace_id, Some(&me.created_by)).await?;
        let ids = family_ids(id, &rows);
        mark_stopped(&ids);
        let mut out = FamilyStopResp {
            subagents: ids.len().saturating_sub(1) as u32,
            ..FamilyStopResp::default()
        };
        for (n, sid) in ids.iter().enumerate() {
            if self.interrupt(sid).await.unwrap_or(false) {
                out.turns += 1;
            }
            let bg = (n > 0 || background) && self.background_count(sid).await.unwrap_or(0) > 0;
            if bg {
                if let Ok(r) = self.stop_all_tasks(sid).await {
                    out.tasks += r.results.len() as u32;
                }
            }
        }
        self.drop_wakes(id).await;
        Ok(out)
    }

    /// Remove the subagent replies waiting in `id`'s queue.
    async fn drop_wakes(&self, id: &Id) {
        let Ok(live) = self.live(id).await else {
            return;
        };
        let mut st = live.lock();
        if st.proj.queue.iter().any(|q| q.wake.is_some()) {
            st.proj.queue.retain(|q| q.wake.is_none());
            emit_queue(&self.inner, &live, &mut st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::agent_runtime::{ChildOpener, RuntimeAccessMode, RuntimeProvider};

    fn row(id: &str, of: Option<&str>, status: RuntimeSessionStatus) -> RuntimeSession {
        RuntimeSession {
            title_auto: false,
            id: id.into(),
            workspace_id: "w".into(),
            provider: RuntimeProvider::Codex,
            title: id.into(),
            cwd: "/tmp".into(),
            model: None,
            effort: None,
            native_thread_id: None,
            status,
            access_mode: RuntimeAccessMode::ApprovalRequired,
            created_by: "u".into(),
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            last_sequence: 0,
            parent_id: of.map(|_| "lead".into()),
            ui_control: None,
            background_tasks: Vec::new(),
            capabilities: Default::default(),
            meta: of.map(|o| children::child_meta(&o.into(), ChildOpener::Agent)),
            last_turn_status: None,
            last_error_class: None,
        }
    }

    #[test]
    fn the_family_is_the_thread_its_open_subagents_and_theirs() {
        use RuntimeSessionStatus::{Closed, Idle, Working};
        let rows = [
            row("lead", None, Working),
            row("a", Some("lead"), Working),
            row("b", Some("lead"), Closed),
            row("a1", Some("a"), Idle),
            row("x", Some("other"), Working),
        ];
        let ids: Vec<String> = family_ids(&"lead".into(), &rows)
            .iter()
            .map(|i| i.to_string())
            .collect();
        assert_eq!(ids, ["lead", "a", "a1"]);
    }

    #[test]
    fn a_stopped_family_stays_quiet_for_a_while() {
        let id: Id = "quiet-test".into();
        assert!(!recently_stopped(&id));
        stopped().lock().unwrap().insert(id.clone(), Instant::now());
        assert!(recently_stopped(&id));
    }
}

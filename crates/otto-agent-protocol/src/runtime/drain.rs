//! The thread's queue going out one message at a time once no turn runs
//! (split out of `runtime.rs` to keep it under the size ratchet).

use super::*;

impl AgentRuntime {
    /// Send the next queued message (subagent replies waiting together go
    /// as one, `children::merge_wakes`); a failure stays queued, marked.
    pub(super) async fn drain_queue(&self, id: &Id) {
        let Ok(live) = self.live(id).await else {
            return;
        };
        let _one = live.drain_lock.lock().await;
        let _prompt = live.prompt_lock.lock().await;
        let next = {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed || st.turn.is_some() {
                return;
            }
            children::merge_wakes(&self.inner, &live, &mut st);
            // A child thread's reply never respawns a stopped agent: it waits
            // for the person (their next message, or Revive). A queue with a
            // person's message in it goes out in order.
            // A message whose delivery failed waits for a person's retry.
            let mut ready = st.proj.queue.iter().filter(|q| q.error.is_none());
            if st.proc.is_none() && ready.clone().all(|q| q.wake.is_some()) {
                return;
            }
            ready.next().cloned()
        };
        let Some(rec) = next else {
            return;
        };
        let res = self.prompt_now(id, &live, &rec.req).await;
        let mut st = live.lock();
        let failed = res
            .as_ref()
            .is_err_and(|e| !matches!(e, Error::Conflict(m) if m == TURN_RUNNING));
        match res {
            // A turn started first: this one waits for its end.
            Err(Error::Conflict(m)) if m == TURN_RUNNING => return,
            Ok(_) => st.proj.queue.retain(|q| q.id != rec.id),
            Err(e) => {
                // Never lost: it stays queued, marked, for Retry / Edit.
                if let Some(q) = st.proj.queue.iter_mut().find(|q| q.id == rec.id) {
                    q.error = Some(e.to_string());
                }
                let mut n = item(
                    new_id(),
                    TurnItemStatus::Failed,
                    None,
                    TurnItemKind::SystemNotice {
                        message: format!(
                            "A queued message could not be sent: {e}. It is still queued — retry, edit or remove it."
                        ),
                        task_settlement: None,
                    },
                );
                n.turn_id = None;
                upsert_item(&self.inner, &live, &mut st, n);
            }
        }
        emit_queue(&self.inner, &live, &mut st);
        drop(st);
        if failed {
            // The ones behind it still go out (each failure is marked once).
            self.kick_queue(id);
        }
    }
}

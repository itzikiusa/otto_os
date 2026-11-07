//! Explicit provider replacement inside a visible thread. Admission is serialized
//! with ordinary prompts and queue drains; history is never rebuilt or deleted.
use super::*;

impl AgentRuntime {
    /// Caller holds `prompt_lock`. Taking the old proc under the state lock also
    /// prevents its pump from starting background work after the idle check.
    pub(super) async fn prompt_fresh(
        &self,
        id: &Id,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
    ) -> Result<RuntimePromptResp> {
        let (old_native, old_thread, was_closed) = {
            let mut st = live.lock();
            if is_busy(&st) || has_pending(&st) || !st.proj.queue.is_empty() {
                return Err(Error::Conflict(
                    "Finish or stop active work, answer pending requests and send or remove queued messages before starting fresh.".into(),
                ));
            }
            if let Some(p) = st.proc.take() {
                stop_child(p, self.inner.config.close_grace);
            }
            let old_native = st.session.native_thread_id.take();
            let old_thread = st.proj.thread.take();
            let was_closed = st.session.status == RuntimeSessionStatus::Closed;
            if was_closed {
                st.session.status = RuntimeSessionStatus::Dormant;
                let _ = live.persist.send(PersistOp::Opened);
            }
            st.fresh_starting = true;
            (old_native, old_thread, was_closed)
        };
        let result = self.prompt_now(id, live, req).await;
        if result.is_err() {
            let mut st = live.lock();
            // Keep the earlier resumable conversation if the first send failed.
            if let Some(p) = st.proc.take() {
                stop_child(p, self.inner.config.close_grace);
            }
            st.fresh_starting = false;
            st.session.native_thread_id = old_native.clone();
            let restored = old_thread.unwrap_or(ProviderThread {
                native_thread_id: old_native,
                context_usage: None,
                updated_at: Utc::now(),
            });
            emit(
                &self.inner,
                live,
                &mut st,
                RuntimeEventBody::ProviderThreadUpdated {
                    provider_thread: restored,
                },
            );
            let closed = was_closed || st.session.status == RuntimeSessionStatus::Closed;
            set_session_status(
                &self.inner,
                live,
                &mut st,
                ProviderSessionStatus::Stopped,
                None,
            );
            if closed {
                st.session.status = RuntimeSessionStatus::Closed;
                let _ = live.persist.send(PersistOp::Closed);
            }
        }
        result
    }

    /// Fresh sends wait for initialization instead of accepting a prompt into a
    /// driver queue whose handshake might fail. A timeout leaves the draft armed.
    pub(super) async fn fresh_process_ready(&self, live: &Arc<Live>) -> bool {
        self.wait_started(live).await;
        let st = live.lock();
        st.proc.is_some()
            && st.proj.session.as_ref().is_some_and(|p| {
                !matches!(
                    p.status,
                    ProviderSessionStatus::Starting | ProviderSessionStatus::Stopped
                )
            })
    }

    /// Called after the driver accepts the first prompt, under the state lock.
    /// An invalid/unsupported prompt therefore never creates a divider.
    pub(super) fn commit_fresh(&self, live: &Live, st: &mut State, turn_id: &Id) {
        st.fresh_starting = false;
        let op = match &st.session.native_thread_id {
            Some(n) => PersistOp::NativeThreadId(n.clone()),
            None => PersistOp::ClearNativeThreadId,
        };
        let _ = live.persist.send(op);
        let mut boundary = item(
            format!("{turn_id}:session-boundary"),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::SessionBoundary,
        );
        boundary.turn_id = Some(turn_id.clone());
        upsert_item(&self.inner, live, st, boundary);
    }
}

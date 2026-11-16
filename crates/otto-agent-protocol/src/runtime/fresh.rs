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
        let (old_native, old_thread, was_closed, dropped) = {
            let mut st = live.lock();
            // Only background work left, and the person said to stop it:
            // replacing the process below stops and settles it.
            let stop_bg = req.stop_background && only_background(&st);
            if !stop_bg && (is_busy(&st) || has_pending(&st) || !st.proj.queue.is_empty()) {
                return Err(Error::Conflict(busy_message(&st, "starting fresh")));
            }
            let grace = self.inner.config.close_grace;
            let dropped =
                selection::drop_proc(&self.inner, live, &mut st, grace).then(|| st.session.clone());
            // A list no process reports any more is settled all the same.
            settle_background(&self.inner, live, &mut st, TurnItemStatus::Interrupted);
            let old_native = st.session.native_thread_id.take();
            let old_thread = st.proj.thread.take();
            let was_closed = st.session.status == RuntimeSessionStatus::Closed;
            if was_closed {
                st.session.status = RuntimeSessionStatus::Dormant;
                let _ = live.persist.send(PersistOp::Opened);
            }
            st.fresh_starting = true;
            (old_native, old_thread, was_closed, dropped)
        };
        if let Some(session) = dropped {
            // Before the fresh spawn mints its own (the retire is keyed by session).
            self.inner.host.retire(&session).await;
        }
        let result = self.prompt_now(id, live, req).await;
        if result.is_err() {
            let mut st = live.lock();
            // Keep the earlier resumable conversation if the first send failed.
            if selection::drop_proc(&self.inner, live, &mut st, self.inner.config.close_grace) {
                retire(&self.inner, &st.session);
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

/// Nothing but background work keeps the thread busy: no turn, no pending
/// request, no queued message.
pub(super) fn only_background(st: &State) -> bool {
    st.turn.is_none()
        && !has_pending(st)
        && st.proj.queue.is_empty()
        && !st.session.background_tasks.is_empty()
}

/// The 409 for an action that needs the thread idle (`what`: "starting
/// fresh"), naming the option when only background work is in the way.
pub(super) fn busy_message(st: &State, what: &str) -> String {
    if only_background(st) {
        let n = st.session.background_tasks.len();
        format!(
            "{n} background task(s) still run — stop them first, or send again with \
             `stop_background: true` to stop them before {what}."
        )
    } else {
        format!(
            "Finish or stop active work, answer pending requests and send or remove queued \
             messages before {what}."
        )
    }
}

impl AgentRuntime {
    /// Stop the leftover background work of a thread with nothing else
    /// running (`stop_background`): its process is replaced the way a switch
    /// restart does it (the work settled, its credentials retired); the next
    /// prompt respawns and resumes. `false` when anything else is in the way.
    pub(super) async fn stop_background_only(&self, id: &Id) -> Result<bool> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        let dropped = {
            let mut st = live.lock();
            if !only_background(&st) {
                return Ok(!is_busy(&st) && !has_pending(&st));
            }
            let grace = self.inner.config.close_grace;
            let dropped = selection::drop_proc(&self.inner, &live, &mut st, grace)
                .then(|| st.session.clone());
            settle_background(&self.inner, &live, &mut st, TurnItemStatus::Interrupted);
            dropped
        };
        if let Some(session) = dropped {
            self.inner.host.retire(&session).await;
        }
        Ok(true)
    }
}

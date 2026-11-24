//! Explicit provider replacement inside a visible thread. Admission is serialized
//! with ordinary prompts and queue drains; history is never rebuilt or deleted.
//! A provider switch is the same replacement onto another provider, whose
//! first prompt carries a context handoff (`handoff.rs`; T3 Code's
//! `ProviderSwitchService` `create_with_handoff`). Everything provider-specific
//! (MCP, access mapping, sandbox, usage) is re-derived by the normal spawn.
use super::*;
use otto_core::agent_runtime::{
    ContextHandoff, RuntimeCapabilities, RuntimeProvider, PROVIDER_SLUG_META_KEY,
};

/// A prompt's `provider` naming another provider than the thread runs.
pub(super) struct Switch {
    provider: RuntimeProvider,
    slug: String,
    /// `meta.provider_slug` for a named account; `None` for a built-in.
    meta_slug: Option<String>,
}

/// What a switch replaced, put back when its first prompt fails.
struct Before {
    provider: RuntimeProvider,
    /// `meta.provider_slug` before (only that key is put back: a pin or
    /// "seen" patched meanwhile stays).
    slug: Option<serde_json::Value>,
    model: Option<String>,
    effort: Option<String>,
    model_from_provider: bool,
    capabilities: RuntimeCapabilities,
    shown_model: Option<Option<String>>,
    /// The old agent's slash commands, put aside until the new one says its
    /// own (a new process never re-sends the old list).
    commands: Vec<otto_core::agent_runtime::ProviderCommand>,
}

impl Before {
    fn restore(self, st: &mut State) {
        st.session.provider = self.provider;
        set_slug(st, self.slug);
        st.session.model = self.model;
        st.session.effort = self.effort;
        st.model_from_provider = self.model_from_provider;
        st.session.capabilities = self.capabilities;
        if let (Some(ps), Some(m)) = (st.proj.session.as_mut(), self.shown_model) {
            ps.model = m;
        }
        st.proj.commands = self.commands;
    }
}

/// Set (or with `None` remove) `meta.provider_slug`, returning what it was.
fn set_slug(st: &mut State, to: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let mut meta = match st.session.meta.take() {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let was = match to {
        Some(v) => meta.insert(PROVIDER_SLUG_META_KEY.into(), v),
        None => meta.remove(PROVIDER_SLUG_META_KEY),
    };
    st.session.meta = (!meta.is_empty()).then_some(serde_json::Value::Object(meta));
    was
}

/// Move the thread onto `sw` in memory (persisted once the first prompt is
/// accepted, `commit_fresh`), with `model` (the prompt's pick) or the new
/// provider's default, and the handoff its first prompt carries.
fn switch_to(st: &mut State, sw: Switch, model: Option<String>) -> Before {
    let from = st.session.provider_slug().to_string();
    // A named account reads as its own slug ("claudework").
    let from_name = if from == st.session.provider.as_str() {
        crate::driver::provider_name(st.session.provider)
    } else {
        &from
    };
    let h = handoff::build(
        st.proj.items.values(),
        st.proj.plans.values(),
        &from,
        from_name,
        &sw.slug,
        handoff::BUDGET_TOKENS,
    );
    let slug = set_slug(st, sw.meta_slug.clone().map(Into::into));
    let before = Before {
        provider: std::mem::replace(&mut st.session.provider, sw.provider),
        slug,
        model: std::mem::replace(&mut st.session.model, model.clone()),
        effort: st.session.effort.take(),
        model_from_provider: std::mem::replace(&mut st.model_from_provider, false),
        capabilities: std::mem::replace(
            &mut st.session.capabilities,
            RuntimeCapabilities::for_provider(sw.provider),
        ),
        shown_model: st.proj.session.as_mut().map(|ps| ps.model.take()),
        commands: std::mem::take(&mut st.proj.commands),
    };
    st.handoff = Some(ContextHandoff {
        from_model: before.model.clone(),
        to_model: model,
        ..h
    });
    before
}

impl AgentRuntime {
    /// Caller holds `prompt_lock`. Taking the old proc under the state lock also
    /// prevents its pump from starting background work after the idle check.
    /// With `switch`, the new process runs on that provider.
    pub(super) async fn prompt_fresh(
        &self,
        id: &Id,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
        switch: Option<Switch>,
    ) -> Result<RuntimePromptResp> {
        // A switch sets the model itself (`apply_selection` then sees none).
        let model = req
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty() && switch.is_some())
            .map(str::to_string);
        let switched_req;
        let req = match &switch {
            Some(_) => {
                switched_req = RuntimePromptReq {
                    fresh_context: true,
                    provider: None,
                    model: None,
                    ..req.clone()
                };
                &switched_req
            }
            None => req,
        };
        let (old_native, old_thread, was_closed, dropped, before) = {
            let mut st = live.lock();
            // Only background work left, and the person said to stop it:
            // replacing the process below stops and settles it.
            let stop_bg = req.stop_background && only_background(&st);
            if !stop_bg && (is_busy(&st) || has_pending(&st) || !st.proj.queue.is_empty()) {
                let what = if switch.is_some() {
                    "switching provider"
                } else {
                    "starting fresh"
                };
                return Err(Error::Conflict(busy_message(&st, what)));
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
            let before = switch.map(|sw| switch_to(&mut st, sw, model));
            (old_native, old_thread, was_closed, dropped, before)
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
            // A switch that did not take: back on the provider it ran.
            st.handoff = None;
            if let Some(b) = before {
                let shown = st.proj.commands.clone();
                b.restore(&mut st);
                if st.proj.commands != shown {
                    let commands = st.proj.commands.clone();
                    emit(
                        &self.inner,
                        live,
                        &mut st,
                        RuntimeEventBody::CommandsUpdated { commands },
                    );
                }
            }
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

    /// The prompt's `provider`, when it names another provider than the
    /// thread runs (enabled, with a structured driver); `None` otherwise.
    pub(super) fn provider_switch(
        &self,
        live: &Live,
        req: &RuntimePromptReq,
    ) -> Result<Option<Switch>> {
        let Some(slug) = req
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        let current = live.lock().session.provider;
        if slug == live.lock().session.provider_slug() {
            return Ok(None);
        }
        let (provider, meta_slug) = self.resolve_target_provider(current, Some(slug))?;
        if self.inner.drivers.create(provider).is_none() {
            return Err(Error::Invalid(format!(
                "no structured driver for provider '{}' yet",
                provider.as_str()
            )));
        }
        Ok(Some(Switch {
            provider,
            slug: slug.to_string(),
            meta_slug,
        }))
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
        let handoff = st.handoff.take();
        if handoff.is_some() {
            // The switch took: the thread runs on this provider from now on.
            let s = &st.session;
            let slug = (s.provider_slug() != s.provider.as_str()).then(|| s.provider_slug().into());
            let _ = live.persist.send(PersistOp::Provider(s.provider, slug));
            let _ = live.persist.send(PersistOp::Model(s.model.clone()));
            let _ = live.persist.send(PersistOp::Effort(None));
            // The old agent's commands go unless the new one listed its own.
            if st.proj.commands.is_empty() {
                let commands = Vec::new();
                emit(
                    &self.inner,
                    live,
                    st,
                    RuntimeEventBody::CommandsUpdated { commands },
                );
            }
        }
        let op = match &st.session.native_thread_id {
            Some(n) => PersistOp::NativeThreadId(n.clone()),
            None => PersistOp::ClearNativeThreadId,
        };
        let _ = live.persist.send(op);
        let mut boundary = item(
            format!("{turn_id}:session-boundary"),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::SessionBoundary { handoff },
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
        // Name what is in the way, and how to clear it.
        let mut why = Vec::new();
        if st.turn.is_some() {
            why.push("the agent is still working (wait for it or Stop it)".to_string());
        }
        if has_pending(st) {
            why.push("a request waits for your answer (answer or skip it)".to_string());
        }
        let queued = st.proj.queue.len();
        if queued > 0 {
            why.push(format!("{queued} queued message(s) (send or remove them)"));
        }
        let bg = st.session.background_tasks.len();
        if bg > 0 {
            why.push(format!("{bg} background task(s) still run (stop them)"));
        }
        format!("Not {what} yet: {}.", why.join("; "))
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

/// A provider switch's first prompt as the driver reads it: the handoff, then
/// the message (the timeline keeps the message alone).
pub(super) fn with_handoff(st: &State, req: &RuntimePromptReq) -> Option<RuntimePromptReq> {
    let h = st.handoff.as_ref().filter(|_| req.fresh_context)?;
    Some(RuntimePromptReq {
        text: handoff::prompt_text(h, &req.text),
        ..req.clone()
    })
}

/// A fresh (or switched) agent did not come up: say why when it said
/// (signed out, an unsupported account), so the composer can show it.
pub(super) fn not_started(st: &State) -> Error {
    Error::Invalid(
        match st.proj.session.as_ref().and_then(|s| s.last_error.clone()) {
            Some(w) => format!("the fresh agent could not start: {w}"),
            None => "the fresh agent could not start — retry the message".into(),
        },
    )
}

impl AgentRuntime {
    /// [`Self::prompt_with`] run to its end even when the caller goes away
    /// (a page reload drops the HTTP request): a provider switch or fresh
    /// start is otherwise left half-done in memory, neither committed nor
    /// restored.
    pub async fn prompt_to_end(
        &self,
        id: &Id,
        req: &RuntimePromptReq,
    ) -> Result<RuntimePromptResp> {
        let (rt, id, req) = (self.clone(), id.clone(), req.clone());
        tokio::spawn(async move { rt.prompt_with(&id, &req).await })
            .await
            .map_err(|e| Error::Internal(format!("the prompt task failed: {e}")))?
    }
}

//! What a thread runs on: the model, reasoning effort and access mode a
//! person (or an opening agent) picked. They are session state, persisted
//! in the order they take effect, re-applied on every respawn, and each
//! accepted change leaves one quiet timeline row ("Model: opus → sonnet"),
//! so the person can see what ran. A live switch the agent refuses puts the
//! thread back on the model it actually runs.
use super::*;
use otto_core::agent_runtime::{RuntimeAccessMode, MODEL_SWITCH_REFUSED};

/// What a prompt's `model` / `effort` asks for. `""` is the documented way
/// to drop a pin ("Default model"): the provider's own default runs next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pick {
    Keep,
    Set(String),
    Clear,
}

pub(super) fn pick(raw: Option<&str>) -> Pick {
    match raw.map(str::trim) {
        None => Pick::Keep,
        Some("") => Pick::Clear,
        Some(v) => Pick::Set(v.to_string()),
    }
}

/// `req` as a driver takes it: no empty model / effort (a clear is the
/// runtime's business — the driver sees no override).
pub(super) fn for_driver(req: &RuntimePromptReq) -> RuntimePromptReq {
    fn keep(v: &Option<String>) -> Option<String> {
        v.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    }
    RuntimePromptReq {
        model: keep(&req.model),
        effort: keep(&req.effort),
        ..req.clone()
    }
}

/// The session runs a pinned model (not one the agent merely reported).
fn pinned(st: &State) -> Option<&String> {
    st.session
        .model
        .as_ref()
        .filter(|_| !st.model_from_provider)
}

/// A clear that needs a restart: the pin (model or effort) this process was
/// launched with goes away. No driver can unset either live.
pub(super) fn clear_needs_restart(st: &State, req: &RuntimePromptReq) -> bool {
    (pick(req.model.as_deref()) == Pick::Clear && pinned(st).is_some())
        || (pick(req.effort.as_deref()) == Pick::Clear && st.session.effort.is_some())
}

/// One timeline row for an accepted change of `field` ("Model", "Effort",
/// "Access"). `title` names the field so a client can style it; the message
/// is the readable line.
pub(super) fn note_change(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    field: &str,
    from: Option<&str>,
    to: Option<&str>,
) {
    let message = format!(
        "{field}: {} → {}",
        from.unwrap_or("default"),
        to.unwrap_or("default")
    );
    note(inner, live, st, field, message);
}

fn note(inner: &Inner, live: &Live, st: &mut State, field: &str, message: String) {
    let mut n = item(
        format!("selection:{}", new_id()),
        TurnItemStatus::Completed,
        Some(field.to_string()),
        TurnItemKind::SystemNotice {
            message,
            task_settlement: None,
        },
    );
    n.turn_id = st.turn.as_ref().map(|t| t.id.clone());
    upsert_item(inner, live, st, n);
}

/// A model change takes effect now: in memory, persisted in order, noted.
/// `live_switch` arms the rollback a refusal triggers.
pub(super) fn set_model(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    to: Option<String>,
    live_switch: bool,
) {
    let from = pinned(st).cloned();
    st.switch_from = live_switch.then(|| st.session.model.clone());
    st.session.model = to.clone();
    st.model_from_provider = false;
    let _ = live.persist.send(PersistOp::Model(to.clone()));
    if from != to {
        note_change(inner, live, st, "Model", from.as_deref(), to.as_deref());
    }
}

pub(super) fn set_access(inner: &Inner, live: &Live, st: &mut State, to: RuntimeAccessMode) {
    let from = std::mem::replace(&mut st.session.access_mode, to);
    let _ = live.persist.send(PersistOp::Access(to));
    note_change(
        inner,
        live,
        st,
        "Access",
        Some(from.as_str()),
        Some(to.as_str()),
    );
}

pub(super) fn set_effort(inner: &Inner, live: &Live, st: &mut State, to: Option<String>) {
    if st.session.effort == to {
        return;
    }
    let from = std::mem::replace(&mut st.session.effort, to.clone());
    let _ = live.persist.send(PersistOp::Effort(to.clone()));
    note_change(inner, live, st, "Effort", from.as_deref(), to.as_deref());
}

/// The agent took a prompt carrying `effort`: it is the thread's now.
pub(super) fn accept_effort(inner: &Inner, live: &Live, st: &mut State, req: &RuntimePromptReq) {
    if let Pick::Set(e) = pick(req.effort.as_deref()) {
        // Only a level this agent lists: one it dropped (an ACP agent with no
        // effort option takes the turn at its default) is not the thread's.
        let takes = &st.session.capabilities.efforts;
        if takes.iter().any(|t| t.eq_ignore_ascii_case(&e)) {
            set_effort(inner, live, st, Some(e));
        }
    }
}

/// A driver error. A refused live model switch rolls the thread back to the
/// model it runs (memory, DB, the running turn's record) and says so; every
/// error is also an `error` row, as before.
pub(super) fn on_error(inner: &Inner, live: &Live, st: &mut State, failure: ProviderFailure) {
    if failure.code.as_deref() == Some(MODEL_SWITCH_REFUSED) {
        if let Some(prev) = st.switch_from.take() {
            let refused = std::mem::replace(&mut st.session.model, prev.clone());
            st.model_from_provider = false;
            let _ = live.persist.send(PersistOp::Model(prev.clone()));
            let turn = st
                .turn
                .as_ref()
                .and_then(|t| st.proj.turns.get(&t.id))
                .cloned();
            if let Some(mut turn) = turn {
                turn.model = prev.clone();
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::ProviderTurnUpdated {
                        provider_turn: turn,
                    },
                );
            }
            let message = format!(
                "Model: {} was refused — still on {}",
                refused.as_deref().unwrap_or("default"),
                prev.as_deref().unwrap_or("default")
            );
            note(inner, live, st, "Model", message);
            let (status, last_error) = current_status(st);
            set_session_status(inner, live, st, status, last_error);
        }
    }
    let mut e = item(
        new_id(),
        TurnItemStatus::Failed,
        None,
        TurnItemKind::Error { failure },
    );
    e.turn_id = st.turn.as_ref().map(|t| t.id.clone());
    upsert_item(inner, live, st, e);
}

/// Take the running process out for good (a switch restart, fresh context,
/// a fatal driver error): it stops, and the background work it owned is
/// settled — a new process never reports the old one's list, so a stale
/// one would keep the thread "busy" forever. `false` when none ran. The
/// caller retires the spawn's credentials (before any respawn mints new).
pub(super) fn drop_proc(inner: &Inner, live: &Live, st: &mut State, grace: Duration) -> bool {
    let Some(p) = st.proc.take() else {
        return false;
    };
    live.proc_ended.notify_waiters();
    stop_child(p, grace);
    st.switch_from = None;
    settle_background(inner, live, st, TurnItemStatus::Interrupted);
    true
}

impl AgentRuntime {
    /// Apply the prompt's model / effort / access picks before its turn
    /// starts (caller: `prompt_now`, no turn running). A driver that cannot
    /// switch live, or a pin being dropped, restarts in the new selection and
    /// resumes its thread. Returns the request as the driver should get it,
    /// with the thread's stored effort filled in when the prompt names none
    /// (so every respawn runs at the effort the person picked).
    pub(super) async fn apply_selection(
        &self,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
    ) -> Result<RuntimePromptReq> {
        let model = pick(req.model.as_deref());
        let wants_switch = model != Pick::Keep
            || pick(req.effort.as_deref()) == Pick::Clear
            || req.access_mode.is_some();
        let mut out = for_driver(req);
        if wants_switch {
            let _spawning = live.spawn_lock.lock().await;
            let stale = {
                let mut st = live.lock();
                check_can_prompt(&st)?;
                let restart_clear = clear_needs_restart(&st, req);
                let mode = req.access_mode.filter(|m| *m != st.session.access_mode);
                let new_model = match &model {
                    Pick::Set(m) if st.session.model.as_ref() != Some(m) => Some(Some(m.clone())),
                    // The model the agent reported, now pinned: no switch.
                    Pick::Set(m) if st.model_from_provider => {
                        st.model_from_provider = false;
                        let _ = live.persist.send(PersistOp::Model(Some(m.clone())));
                        None
                    }
                    Pick::Clear if pinned(&st).is_some() => Some(None),
                    _ => None,
                };
                let live_switch = !restart_clear
                    && st.proc.as_ref().is_some_and(|p| {
                        (new_model.is_none() || p.driver.switches_model_live())
                            && mode.is_none_or(|m| p.driver.switches_access_live(m))
                    });
                if let Some(m) = new_model.clone() {
                    set_model(&self.inner, live, &mut st, m, live_switch);
                }
                if pick(req.effort.as_deref()) == Pick::Clear {
                    set_effort(&self.inner, live, &mut st, None);
                }
                if let Some(m) = mode {
                    set_access(&self.inner, live, &mut st, m);
                }
                let changed = new_model.is_some() || mode.is_some() || restart_clear;
                let grace = self.inner.config.interrupt_grace;
                if changed && !live_switch && drop_proc(&self.inner, live, &mut st, grace) {
                    Some(st.session.clone())
                } else {
                    None
                }
            };
            if let Some(session) = stale {
                // Before the respawn mints its own (the retire is keyed by session).
                self.inner.host.retire(&session).await;
            }
        }
        if out.effort.is_none() {
            out.effort = live.lock().session.effort.clone();
        }
        Ok(out)
    }

    /// Merge a client-owned thread-meta patch (`otto_core::agent_runtime::
    /// THREAD_META_KEYS`, already validated) into the session's `meta` and
    /// tell every client through `provider_session.updated` (its `thread`).
    /// Allowed on a `closed` session too (History, pins).
    pub async fn update_thread_meta(
        &self,
        id: &Id,
        patch: serde_json::Map<String, serde_json::Value>,
    ) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let patch = serde_json::Value::Object(patch);
        self.inner.repo.patch_meta(id, &patch).await?;
        let mut st = live.lock();
        let mut meta = match st.session.meta.take() {
            Some(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        for (k, v) in patch.as_object().into_iter().flatten() {
            if v.is_null() {
                meta.remove(k);
            } else {
                meta.insert(k.clone(), v.clone());
            }
        }
        st.session.meta = (!meta.is_empty()).then_some(serde_json::Value::Object(meta));
        let (status, last_error) = current_status(&st);
        set_session_status(&self.inner, &live, &mut st, status, last_error);
        Ok(st.session.clone())
    }

    /// Store `effort` as the thread's (create, an opening agent's pick with
    /// no prompt). Checked against what the provider takes.
    pub async fn set_effort(&self, id: &Id, effort: Option<&str>) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let effort = effort.map(str::trim).filter(|e| !e.is_empty());
        if let Some(e) = effort {
            check_effort(&st.session.capabilities, st.session.provider_slug(), e)?;
        }
        set_effort(&self.inner, &live, &mut st, effort.map(str::to_string));
        Ok(st.session.clone())
    }
}

/// `effort` is one of the levels `caps` lists (a 400 naming `who` if not).
pub fn check_effort(
    caps: &otto_core::agent_runtime::RuntimeCapabilities,
    who: &str,
    effort: &str,
) -> Result<()> {
    if caps.efforts.iter().any(|t| t.eq_ignore_ascii_case(effort)) {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "{who} has no '{effort}' reasoning effort"
    )))
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;

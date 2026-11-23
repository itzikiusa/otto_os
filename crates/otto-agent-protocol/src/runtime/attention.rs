//! "Agent needs attention" for structured threads: the PTY path's notice
//! (`otto-server` `routes/activity.rs` `notify_waiting`, `monitor.rs`) for a
//! thread that starts waiting on its person — an approval, a question form
//! (`provider_session.status` → `waiting`), or a private secret request.
//!
//! One notice per waiting EPISODE: raised when the status enters `waiting`,
//! settled when it leaves it (answered, withdrawn, interrupted, closed). A
//! second request while the first still waits is the same episode and raises
//! nothing. The host owns the notice itself ([`RuntimeHost::attention`]):
//! the `session_events` switch, de-duplication on `session:<id>:waiting`,
//! the bell and the native banner. Background threads (`meta.source` in
//! [`BACKGROUND_SESSION_SOURCES`] — workflow steps, reviews, scheduled tasks,
//! …) run unattended and stay quiet, as their terminal sessions do.
//!
//! [`RuntimeHost::attention`]: crate::host::RuntimeHost::attention

use otto_core::agent_runtime::{
    ProviderSessionStatus, RequestKind, RuntimeRequestStatus, RuntimeSession, TurnItemKind,
};
use otto_core::domain::BACKGROUND_SESSION_SOURCES;

use super::{Inner, State};

/// Called by `set_session_status` before `next` is emitted (the projection
/// still holds the previous status).
pub(super) fn on_status(inner: &Inner, st: &State, next: ProviderSessionStatus) {
    let was = st
        .proj
        .session
        .as_ref()
        .is_some_and(|p| p.status == ProviderSessionStatus::Waiting);
    let now = next == ProviderSessionStatus::Waiting;
    if was != now {
        let what = now.then(|| describe(st));
        signal(inner, &st.session, what.as_deref());
        if now {
            super::child_question::on_waiting(inner, st);
        }
    }
}

/// Raise (`Some(what)`) or settle (`None`) `session`'s notice, unless it is a
/// background thread.
pub(super) fn signal(inner: &Inner, session: &RuntimeSession, what: Option<&str>) {
    if !is_background(session) {
        inner.host.attention(session, what);
    }
}

pub(super) fn is_background(s: &RuntimeSession) -> bool {
    s.meta
        .as_ref()
        .and_then(|m| m.get("source"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|src| BACKGROUND_SESSION_SOURCES.contains(&src))
}

/// What the newest pending request asks, in the notice's words.
pub(super) fn describe(st: &State) -> String {
    let Some(r) = st
        .proj
        .requests
        .values()
        .filter(|r| r.status == RuntimeRequestStatus::Pending)
        .max_by_key(|r| r.created_at)
    else {
        return "is waiting on you".into();
    };
    let nonempty = |s: &String| !s.trim().is_empty();
    if r.kind == RequestKind::Question {
        let q = r
            .question_title
            .clone()
            .filter(nonempty)
            .or_else(|| r.questions.first().map(|q| q.prompt.clone()))
            .filter(nonempty);
        return match q {
            Some(q) => format!("asks: {}", q.trim()),
            None => "asks you a question".into(),
        };
    }
    let asked = st
        .proj
        .request_items
        .get(&r.id)
        .and_then(|i| st.proj.items.get(i))
        .and_then(|i| match &i.kind {
            TurnItemKind::ApprovalRequest { prompt, .. } => prompt.clone().or(i.title.clone()),
            _ => None,
        })
        .filter(nonempty);
    match asked {
        Some(p) => format!("wants approval: {}", p.trim()),
        None => "wants your approval".into(),
    }
}

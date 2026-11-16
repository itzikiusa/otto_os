//! Keeps a live thread's `last_turn_status` / `last_error_class` in step
//! with its projection (`RuntimeSession::set_outcome`): the turn and error
//! events that can change them refresh the row before the event is
//! published, so every `get` / list after it reads the new outcome. A cold
//! thread gets the same values from the store (`otto-state` derives them
//! from the persisted latest turn).

use otto_core::agent_runtime::{RuntimeEventBody, RuntimeSession, TurnItemKind};

use super::Projection;

/// Called by `emit` after `body` is projected.
pub(super) fn on_event(session: &mut RuntimeSession, proj: &Projection, body: &RuntimeEventBody) {
    let relevant = match body {
        RuntimeEventBody::ProviderTurnUpdated { .. } => true,
        RuntimeEventBody::TurnItemUpdated { turn_item } => {
            matches!(turn_item.kind, TurnItemKind::Error { .. })
        }
        _ => false,
    };
    if relevant {
        refresh(session, proj);
    }
}

/// The outcome of `proj`'s latest turn (by ordinal) and its error item.
pub(super) fn refresh(session: &mut RuntimeSession, proj: &Projection) {
    let latest = proj.turns.values().max_by_key(|t| t.ordinal);
    let failure = latest
        .and_then(|t| proj.items.get(&format!("{}:error", t.id)))
        .and_then(|i| match &i.kind {
            TurnItemKind::Error { failure } => Some(failure),
            _ => None,
        });
    session.set_outcome(latest, failure);
}

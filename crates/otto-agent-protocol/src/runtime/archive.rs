//! Settling archived history (split out of runtime.rs).

use chrono::Utc;
use otto_core::agent_runtime::{
    ProviderSession, ProviderSessionStatus, ProviderTurn, ProviderTurnStatus, ResponseCapability,
    RuntimeRequest, RuntimeRequestStatus, TurnItem, TurnItemStatus,
};

use super::{settle_item, K_ITEM, K_QUEUE, K_REQUEST, K_SESSION, K_TURN};

/// Settle an archived `agent_runtime_entities` JSON payload so exported or
/// restored history never preserves mid-turn spinners, running provider
/// sessions, or pending approvals. Returns `Some(updated_json)` only when the
/// entity required settling.
pub fn settle_archived_entity_json(kind: &str, payload_json: &str) -> Option<String> {
    let now = Utc::now();
    let raw: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let updated = match kind {
        K_TURN => serde_json::from_value::<ProviderTurn>(raw)
            .ok()
            .filter(|t| !t.status.is_terminal())
            .and_then(|mut t| {
                t.status = ProviderTurnStatus::Interrupted;
                t.completed_at = Some(now);
                serde_json::to_value(t).ok()
            }),
        K_ITEM => serde_json::from_value::<TurnItem>(raw)
            .ok()
            .filter(|i| i.status.is_open() || i.is_streaming())
            .and_then(|mut i| {
                settle_item(&mut i, TurnItemStatus::Interrupted, now);
                serde_json::to_value(i).ok()
            }),
        K_REQUEST => serde_json::from_value::<RuntimeRequest>(raw)
            .ok()
            .filter(|q| q.status == RuntimeRequestStatus::Pending)
            .and_then(|mut q| {
                q.status = RuntimeRequestStatus::Expired;
                q.response_capability = ResponseCapability::NotResumable {
                    reason: "Imported history; pending request was not resumed.".into(),
                };
                serde_json::to_value(q).ok()
            }),
        K_SESSION => serde_json::from_value::<ProviderSession>(raw)
            .ok()
            .filter(|s| {
                s.status != ProviderSessionStatus::Stopped || !s.background_tasks.is_empty()
            })
            .and_then(|mut s| {
                s.background_tasks.clear();
                s.status = ProviderSessionStatus::Stopped;
                s.updated_at = now;
                serde_json::to_value(s).ok()
            }),
        // Imported prompts never send themselves: boot recovery would dispatch a
        // person's queued message on the next start.
        K_QUEUE => raw
            .as_array()
            .filter(|q| !q.is_empty())
            .map(|_| serde_json::json!([])),
        _ => None,
    }?;
    serde_json::to_string(&updated).ok()
}

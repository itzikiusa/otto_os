//! Personal-agent **activity** — the live "what is it doing now" feed.
//!
//! Built on events the daemon already sees: every governed `otto.*` call a
//! personal agent's session makes passes [`crate::personal_agent_policy`],
//! which records it here (allowed / blocked / needs approval), and every
//! approval the call files is noted with its id so the timeline can show it as
//! *waiting*. Runs (history) come from `personal_agent_runs`; the current
//! session's live state from the session manager — the route merges the three
//! (`GET /personal-agents/{id}/activity`).
//!
//! The tool-call ring is in memory (newest [`CAP`] per agent): it is a live
//! view, not an audit trail — `mcp_call_log` stays the durable audit, and run
//! history survives restarts.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use otto_core::event::Event;
use serde::Serialize;

use crate::personal_agent_policy::AgentGate;
use crate::state::ServerCtx;

/// Newest entries kept per agent.
const CAP: usize = 200;

/// One live activity entry.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityItem {
    /// Monotonic per-process sequence (stable ordering + dedupe in the UI).
    pub seq: u64,
    pub at: String,
    /// `tool_call` | `blocked` | `approval_required` | `approval_waiting`.
    pub kind: String,
    /// Bare tool name (`create_pr`).
    pub tool: String,
    pub detail: String,
    pub session_id: Option<String>,
    /// The approval this entry is waiting on (`approval_waiting`).
    pub approval_id: Option<String>,
}

type Ring = HashMap<String, VecDeque<ActivityItem>>;

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn push(agent_id: &str, item: ActivityItem) {
    let mut map = ring().lock().unwrap_or_else(|e| e.into_inner());
    let q = map.entry(agent_id.to_string()).or_default();
    q.push_back(item);
    while q.len() > CAP {
        q.pop_front();
    }
}

/// The entry a gate decision becomes.
pub fn item_for(bare: &str, session_id: &str, gate: &AgentGate) -> ActivityItem {
    let (kind, detail) = match gate {
        AgentGate::Pass => ("tool_call", format!("called otto.{bare}")),
        AgentGate::Deny(reason) => ("blocked", reason.clone()),
        AgentGate::ForceApproval { reason, .. } => ("approval_required", reason.clone()),
    };
    ActivityItem {
        seq: next_seq(),
        at: chrono::Utc::now().to_rfc3339(),
        kind: kind.into(),
        tool: bare.to_string(),
        detail,
        session_id: Some(session_id.to_string()),
        approval_id: None,
    }
}

fn announce(ctx: &ServerCtx, workspace_id: &str, agent_id: &str, kind: &str) {
    // Ids only, like every other personal-agent event; scoped to the
    // workspace's members (ws_events).
    let _ = ctx.events.send(Event::PersonalAgentActivity {
        workspace_id: workspace_id.to_string(),
        agent_id: agent_id.to_string(),
        kind: kind.to_string(),
    });
}

/// Record a governed tool call by an agent's session (called by the policy).
pub fn record_tool_call(
    ctx: &ServerCtx,
    workspace_id: &str,
    agent_id: &str,
    session_id: &str,
    bare: &str,
    gate: &AgentGate,
) {
    let item = item_for(bare, session_id, gate);
    let kind = item.kind.clone();
    push(agent_id, item);
    announce(ctx, workspace_id, agent_id, &kind);
}

/// Note that an agent's call filed a human approval and is waiting on it.
pub fn record_approval_waiting(
    ctx: &ServerCtx,
    workspace_id: &str,
    agent_id: &str,
    session_id: Option<&str>,
    bare: &str,
    approval_id: &str,
) {
    push(
        agent_id,
        ActivityItem {
            seq: next_seq(),
            at: chrono::Utc::now().to_rfc3339(),
            kind: "approval_waiting".into(),
            tool: bare.to_string(),
            detail: format!("waiting for your approval to run otto.{bare}"),
            session_id: session_id.map(str::to_string),
            approval_id: Some(approval_id.to_string()),
        },
    );
    announce(ctx, workspace_id, agent_id, "approval_waiting");
}

/// The agent's live entries, newest first.
pub fn recent(agent_id: &str, limit: usize) -> Vec<ActivityItem> {
    let map = ring().lock().unwrap_or_else(|e| e.into_inner());
    map.get(agent_id)
        .map(|q| q.iter().rev().take(limit).cloned().collect())
        .unwrap_or_default()
}

/// Entries newer than `after_seq`, newest first, at most `limit` (perf W4:
/// the Activity tab appends only what is new instead of re-reading the ring).
pub fn recent_after(agent_id: &str, after_seq: u64, limit: usize) -> Vec<ActivityItem> {
    let map = ring().lock().unwrap_or_else(|e| e.into_inner());
    map.get(agent_id)
        .map(|q| {
            q.iter()
                .rev()
                .take_while(|i| i.seq > after_seq)
                .take(limit)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Approval ids of the agent's `approval_waiting` entries still in the ring
/// (newest first, deduped).
pub fn waiting_approval_ids(agent_id: &str) -> Vec<String> {
    let map = ring().lock().unwrap_or_else(|e| e.into_inner());
    let mut seen = std::collections::HashSet::new();
    map.get(agent_id)
        .map(|q| {
            q.iter()
                .rev()
                .filter(|i| i.kind == "approval_waiting")
                .filter_map(|i| i.approval_id.clone())
                .filter(|id| seen.insert(id.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Forget an agent's live entries ("reset agent", delete).
pub fn clear(agent_id: &str) {
    ring()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_capped_newest_first_and_clearable() {
        let agent = "agent-ring-test";
        clear(agent);
        for i in 0..(CAP + 5) {
            push(agent, item_for(&format!("t{i}"), "s1", &AgentGate::Pass));
        }
        let got = recent(agent, CAP + 10);
        assert_eq!(got.len(), CAP);
        assert_eq!(got[0].tool, format!("t{}", CAP + 4));
        assert!(got[0].seq > got[1].seq);
        clear(agent);
        assert!(recent(agent, 10).is_empty());
    }

    #[test]
    fn recent_after_returns_only_newer_entries() {
        let agent = "agent-after-test";
        clear(agent);
        for i in 0..5 {
            push(agent, item_for(&format!("t{i}"), "s1", &AgentGate::Pass));
        }
        let all = recent(agent, 10);
        let cursor = all[2].seq; // t2
        let newer = recent_after(agent, cursor, 10);
        assert_eq!(
            newer.iter().map(|i| i.tool.as_str()).collect::<Vec<_>>(),
            ["t4", "t3"]
        );
        assert!(recent_after(agent, all[0].seq, 10).is_empty());
        assert_eq!(recent_after(agent, 0, 10).len(), 5);
        clear(agent);
    }

    #[test]
    fn gate_kinds_map_to_entries() {
        let blocked = item_for("create_pr", "s", &AgentGate::Deny("read-only".into()));
        assert_eq!(blocked.kind, "blocked");
        assert_eq!(blocked.detail, "read-only");
        let ask = item_for(
            "k8s_action",
            "s",
            &AgentGate::ForceApproval {
                reason: "rule".into(),
                risk: "agent_rule",
            },
        );
        assert_eq!(ask.kind, "approval_required");
    }
}

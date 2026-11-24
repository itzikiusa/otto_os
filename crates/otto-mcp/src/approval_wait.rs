//! Waiting (bounded) for a person's decision on an `mcp_approvals` card —
//! shared by the `otto.*` choke point and the live-agent gateway, so an
//! approval-gated call made from a thread waits for the answer given in the
//! thread's attention dock and then runs, instead of telling the agent to
//! resubmit.

use std::time::Duration;

use otto_core::event::Event;
use otto_state::McpApprovalRepo;
use tokio::sync::broadcast;

/// The hold of a caller nobody can answer in time: a headless session (a
/// routine run, a workflow step, a background review) keeps the short wait and
/// gets `pending_approval` back.
pub const HEADLESS_WAIT_SECS: u64 = 15;

/// The hold for an agent CLI whose own per-call MCP timeout is not known: its
/// MCP client may give up at the MCP SDK's 60 s default, so the card must be
/// answered (and the call run) before that.
pub const UNKNOWN_CLIENT_WAIT_SECS: u64 = 45;
const _: () = assert!(
    UNKNOWN_CLIENT_WAIT_SECS < 60,
    "under the MCP SDK's 60 s default"
);

/// How long a governed call from a session may hold for a person. Only an
/// interactive session (a thread or terminal session its person works in)
/// holds long, and never past its CLI's own per-call MCP timeout:
/// - claude: `MCP_TOOL_TIMEOUT` default far above it (Otto sets none);
/// - codex: Otto passes `tool_timeout_sec` = `CLIENT_TOOL_TIMEOUT_SECS`;
/// - gemini: 10 min default (`DEFAULT_MCP_TIMEOUT_MSEC`);
/// - grok 1.0.92: `tool_timeout_sec` default 6000 s (its own config docs);
/// - agy, opencode, muse, pi: not established → [`UNKNOWN_CLIENT_WAIT_SECS`].
pub fn hold_secs(cli_family: &str, interactive: bool) -> u64 {
    if !interactive {
        return HEADLESS_WAIT_SECS;
    }
    match cli_family {
        "claude" | "codex" | "gemini" | "grok" => otto_core::mcp_budgets::GOVERNED_WAIT_SECS,
        _ => UNKNOWN_CLIENT_WAIT_SECS,
    }
}

/// `Some(true)` = approved, `Some(false)` = denied / expired / cancelled,
/// `None` = still pending after `budget` (the caller resubmits later).
///
/// Event-driven: it subscribes to the bus BEFORE the first status read, then
/// re-reads only when an `mcp_approval_changed` names this approval (or a bulk
/// expiry, `approval_id: None`) — so a decision resumes the call immediately,
/// and a waiting call issues a few reads instead of one per second. The
/// `fallback` re-read covers a writer that bypassed the hook (another
/// process) and a lagged or closed receiver.
pub async fn wait_for_decision(
    approvals: &McpApprovalRepo,
    events: &broadcast::Sender<Event>,
    approval_id: &str,
    budget: Duration,
    fallback: Duration,
) -> Option<bool> {
    let mut rx = events.subscribe();
    let deadline = tokio::time::Instant::now() + budget;
    let id = approval_id.to_string();
    loop {
        if let Ok(a) = approvals.get(&id).await {
            match a.status.as_str() {
                // `consumed` = a sibling call waiting on the same shared card
                // already spent the approval. Report it as decided so the
                // caller's single-use `consume` refuses it ("approval already
                // used") instead of this call idling to its deadline and then
                // telling the agent to resubmit a card that can never run.
                "approved" | "consumed" => return Some(true),
                "denied" | "expired" | "cancelled" => return Some(false),
                _ => {}
            }
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let tick = (now + fallback).min(deadline);
        // Wait for a relevant change, the fallback tick, or the deadline —
        // whichever is first — then re-read.
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(Event::McpApprovalChanged { approval_id, .. })
                        if approval_id.as_deref().is_none_or(|a| a == id) => break,
                    Ok(_) => continue,
                    // Lagged: a change may have been dropped — re-read now.
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    // Bus gone: plain polling at the fallback period.
                    Err(broadcast::error::RecvError::Closed) => {
                        tokio::time::sleep_until(tick).await;
                        break;
                    }
                },
                _ = tokio::time::sleep_until(tick) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_person_s_session_holds_long_and_never_past_its_cli_timeout() {
        let long = otto_core::mcp_budgets::GOVERNED_WAIT_SECS;
        for f in ["claude", "codex", "gemini", "grok"] {
            assert_eq!(hold_secs(f, true), long, "{f}");
            assert_eq!(hold_secs(f, false), HEADLESS_WAIT_SECS, "{f} headless");
        }
        for f in ["agy", "opencode", "muse", "pi", "custom"] {
            assert_eq!(hold_secs(f, true), UNKNOWN_CLIENT_WAIT_SECS, "{f}");
        }
    }
    use otto_state::NewApproval;

    async fn pending(repo: &McpApprovalRepo) -> String {
        repo.create(NewApproval {
            workspace_id: None,
            kind: "tool_call".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("create_pr".into()),
            title: "otto → create_pr".into(),
            detail: None,
            args_redacted_json: "{}".into(),
            args_hash: None,
            risk_label: None,
            requested_by: Some("agent-user".into()),
            requested_by_kind: Some("mcp_server".into()),
            requested_by_session_id: Some("thread-1".into()),
            expires_at: None,
        })
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn a_decision_wakes_the_wait_and_an_unanswered_card_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::open(&dir.path().join("t.db"))
            .await
            .unwrap();
        let repo = McpApprovalRepo::new(pool);
        let (tx, _keep) = broadcast::channel(8);
        let slow = Duration::from_secs(60);

        let id = pending(&repo).await;
        assert_eq!(
            wait_for_decision(&repo, &tx, &id, Duration::ZERO, slow).await,
            None
        );

        let (r2, tx2, id2) = (repo.clone(), tx.clone(), id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            r2.decide(&id2, true, "person", None).await.unwrap();
            let _ = tx2.send(Event::McpApprovalChanged {
                approval_id: Some(id2),
                workspace_id: None,
                status: "approved".into(),
            });
        });
        // Long budget and fallback: only the event can end it this fast.
        let started = std::time::Instant::now();
        let decided = wait_for_decision(&repo, &tx, &id, slow, slow).await;
        assert_eq!(decided, Some(true));
        assert!(started.elapsed() < Duration::from_secs(5));

        let denied = pending(&repo).await;
        repo.decide(&denied, false, "person", None).await.unwrap();
        assert_eq!(
            wait_for_decision(&repo, &tx, &denied, slow, slow).await,
            Some(false)
        );
    }
}

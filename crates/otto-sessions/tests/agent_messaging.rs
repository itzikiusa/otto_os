//! Agents talk to each other (`agent_message_rule`): an agent session's own
//! credential may MESSAGE (`/message`, broadcast, relay) any sibling agent CLI
//! session, but never a sibling plain shell or connection PTY — and raw
//! `/input` / lifecycle stay on the strict `agent_input_rule`.

use otto_core::domain::{Session, SessionKind, SessionStatus};
use otto_core::Id;
use otto_sessions::http::{agent_input_rule, agent_message_rule, DELEGATED_BY_META};
use serde_json::json;

fn session(id: &str, provider: &str, meta: serde_json::Value) -> Session {
    Session {
        id: Id::from(id),
        workspace_id: Id::from("ws"),
        kind: if provider == "ssh" {
            SessionKind::Connection
        } else {
            SessionKind::Agent
        },
        provider: provider.into(),
        title: id.into(),
        status: SessionStatus::Idle,
        cwd: "/tmp".into(),
        provider_session_id: None,
        connection_id: None,
        created_by: Id::from("u"),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta,
    }
}

#[test]
fn agent_token_messages_sibling_agents_but_not_sibling_shells() {
    let me = Id::from("lead");
    let sibling_agent = session("messi", "codex", json!({}));
    let sibling_shell = session("zsh", "shell", json!({}));
    let sibling_ssh = session("box", "ssh", json!({}));
    let my_shell_worker = session("w", "shell", json!({ DELEGATED_BY_META: "lead" }));
    assert!(agent_message_rule(Some(&me), &sibling_agent).is_ok());
    assert!(agent_message_rule(Some(&me), &sibling_shell).is_err());
    assert!(agent_message_rule(Some(&me), &sibling_ssh).is_err());
    // Its own worker (even a shell) and a person's credential: unchanged.
    assert!(agent_message_rule(Some(&me), &my_shell_worker).is_ok());
    assert!(agent_message_rule(None, &sibling_shell).is_ok());
    // Raw input / lifecycle stay strict for the sibling agent.
    assert!(agent_input_rule(Some(&me), &sibling_agent.id, &sibling_agent.meta, true).is_err());
}

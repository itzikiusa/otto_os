//! Parsing of the planner claude's reply text into an `ActionPlan`, plus
//! plan validation against the workspace context. The reply text comes
//! from the session JSONL transcript (see [`crate::claude_pty`]).

use otto_core::api::{Action, ActionPlan};
use otto_core::{Error, Result};

use crate::OrchestratorContext;

/// Maximum number of actions a plan may contain.
pub const MAX_ACTIONS: usize = 10;

/// Find the first complete JSON array embedded in `text` (brackets matched
/// outside of string literals) that parses as JSON.
pub fn find_json_array(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    for (start, &b) in bytes.iter().enumerate() {
        if b != b'[' {
            continue;
        }
        if let Some(end) = matching_bracket(bytes, start) {
            let candidate = &text[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Byte index of the `]` matching the `[` at `open`, skipping string
/// literals and escapes. ASCII-only scanning is safe on UTF-8 bytes.
fn matching_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse an `ActionPlan` out of assistant text (the first JSON array found).
pub fn parse_plan(text: &str) -> Result<ActionPlan> {
    let array = find_json_array(text)
        .ok_or_else(|| Error::Invalid("no JSON action array found in model output".into()))?;
    serde_json::from_str::<ActionPlan>(array)
        .map_err(|e| Error::Invalid(format!("action plan does not match schema: {e}")))
}

/// Validate a parsed plan: 1..=10 actions, providers in the allowed set,
/// referenced session/connection ids exist in the workspace context.
pub fn validate_plan(
    plan: &ActionPlan,
    ctx: &OrchestratorContext,
    allowed_providers: &[String],
) -> Result<()> {
    if plan.is_empty() || plan.len() > MAX_ACTIONS {
        return Err(Error::Invalid(format!(
            "plan must contain between 1 and {MAX_ACTIONS} actions (got {})",
            plan.len()
        )));
    }
    for (i, action) in plan.iter().enumerate() {
        match action {
            Action::SpawnSessions { provider, count } => {
                if !allowed_providers.iter().any(|p| p == provider) {
                    return Err(Error::Invalid(format!(
                        "action {i}: unknown provider '{provider}'"
                    )));
                }
                if *count == 0 {
                    return Err(Error::Invalid(format!("action {i}: count must be >= 1")));
                }
            }
            Action::Broadcast { text } => {
                if text.is_empty() {
                    return Err(Error::Invalid(format!("action {i}: empty broadcast text")));
                }
            }
            Action::OpenConnection { connection_id } => {
                if !ctx.connections.iter().any(|c| &c.id == connection_id) {
                    return Err(Error::Invalid(format!(
                        "action {i}: unknown connection id '{connection_id}'"
                    )));
                }
            }
            Action::RunCommand { session_id, text } => {
                if !ctx.sessions.iter().any(|s| &s.id == session_id) {
                    return Err(Error::Invalid(format!(
                        "action {i}: unknown session id '{session_id}'"
                    )));
                }
                if text.is_empty() {
                    return Err(Error::Invalid(format!("action {i}: empty command text")));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::domain::{Connection, ConnectionKind, Session, SessionKind, SessionStatus};

    fn ctx() -> OrchestratorContext {
        OrchestratorContext {
            sessions: vec![Session {
                id: "S1".into(),
                workspace_id: "W1".into(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "claude #1".into(),
                status: SessionStatus::Idle,
                cwd: "/tmp".into(),
                provider_session_id: Some("PSID".into()),
                connection_id: None,
                created_by: "U1".into(),
                created_at: Utc::now(),
                last_active_at: Utc::now(),
                archived: false,
                meta: serde_json::json!({}),
            }],
            connections: vec![Connection {
                id: "C1".into(),
                workspace_id: Some("W1".into()),
                name: "prod redis".into(),
                kind: ConnectionKind::Redis,
                params: serde_json::json!({"host":"r1"}),
                secret_ref: None,
                first_command: None,
                section_id: None,
                created_by: "U1".into(),
                created_at: Utc::now(),
            }],
            cwd: "/tmp".into(),
            default_provider: "claude".into(),
        }
    }

    const ALLOWED: &[&str] = &["claude", "codex", "shell"];

    fn allowed() -> Vec<String> {
        ALLOWED.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn valid_array_parses_and_validates() {
        let text = r#"[
            {"action":"spawn_sessions","provider":"claude","count":2},
            {"action":"broadcast","text":"run the tests"},
            {"action":"open_connection","connection_id":"C1"},
            {"action":"run_command","session_id":"S1","text":"cargo test"}
        ]"#;
        let plan = parse_plan(text).expect("parse");
        assert_eq!(plan.len(), 4);
        validate_plan(&plan, &ctx(), &allowed()).expect("validate");
    }

    #[test]
    fn array_embedded_in_prose_is_found() {
        let text = "Sure! Here is the plan you asked for [notice]:\n```json\n[{\"action\":\"run_command\",\"session_id\":\"S1\",\"text\":\"ls [a-z]*\"}]\n```\nLet me know.";
        let plan = parse_plan(text).expect("parse embedded");
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0],
            otto_core::api::Action::RunCommand {
                session_id: "S1".into(),
                text: "ls [a-z]*".into()
            }
        );
        validate_plan(&plan, &ctx(), &allowed()).expect("validate");
    }

    #[test]
    fn malformed_json_is_an_error() {
        let text = r#"[{"action":"broadcast","text": "unterminated"#;
        let err = parse_plan(text).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got: {err}");
    }

    #[test]
    fn unknown_action_is_an_error() {
        let text = r#"[{"action":"delete_everything","target":"/"}]"#;
        let err = parse_plan(text).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got: {err}");
    }

    #[test]
    fn validation_rejects_bad_refs_counts_and_providers() {
        let c = ctx();
        let a = allowed();

        let plan = vec![otto_core::api::Action::SpawnSessions {
            provider: "gpt9".into(),
            count: 1,
        }];
        assert!(validate_plan(&plan, &c, &a).is_err());

        let plan = vec![otto_core::api::Action::OpenConnection {
            connection_id: "NOPE".into(),
        }];
        assert!(validate_plan(&plan, &c, &a).is_err());

        let plan = vec![otto_core::api::Action::RunCommand {
            session_id: "NOPE".into(),
            text: "ls".into(),
        }];
        assert!(validate_plan(&plan, &c, &a).is_err());

        let empty: ActionPlan = vec![];
        assert!(validate_plan(&empty, &c, &a).is_err());

        let too_many: ActionPlan = (0..11)
            .map(|_| otto_core::api::Action::Broadcast { text: "x".into() })
            .collect();
        assert!(validate_plan(&too_many, &c, &a).is_err());
    }
}

//! Turn a session's claude JSONL transcript into a compact digest the analysis
//! agent can read, and surface which skills were invoked (candidates).

use std::path::Path;

use otto_core::domain::Session;
use otto_orchestrator::claude_pty::session_jsonl_path;

/// Total transcript text budget per session (chars) to keep prompts bounded.
const PER_SESSION_TEXT_CAP: usize = 4000;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDigest {
    pub session_id: String,
    pub title: String,
    pub turns: usize,
    pub skills_used: Vec<String>,
    pub tool_errors: usize,
    /// Truncated concatenation of user + assistant text turns.
    pub text: String,
}

/// Build a digest from a session, reading its transcript from disk. Returns
/// `None` when no transcript exists yet (no `provider_session_id`, or file
/// not written). Pure parsing of the JSONL body is in `digest_from_jsonl` so
/// it can be unit-tested without the filesystem.
pub fn build_digest(session: &Session) -> Option<SessionDigest> {
    let psid = session.provider_session_id.as_deref()?;
    let path = session_jsonl_path(&session.cwd, psid);
    let body = std::fs::read_to_string(&path).ok()?;
    Some(digest_from_jsonl(&session.id, &session.title, &body))
}

/// Parse a transcript body into a digest (filesystem-free; testable).
pub fn digest_from_jsonl(session_id: &str, title: &str, body: &str) -> SessionDigest {
    let mut turns = 0usize;
    let mut skills_used: Vec<String> = Vec::new();
    let mut tool_errors = 0usize;
    let mut text = String::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Older shape: a top-level `toolUseResult.is_error` flag. Real claude
        // transcripts carry the flag on the `tool_result` content block instead
        // (see below) — we tolerate both.
        if v.get("toolUseResult")
            .and_then(|r| r.get("is_error"))
            .and_then(|b| b.as_bool())
            == Some(true)
        {
            tool_errors += 1;
        }
        let Some(msg) = v.get("message") else { continue };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let Some(content) = msg.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        if role == "user" || role == "assistant" {
            turns += 1;
        }
        for block in content {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        push_capped(&mut text, role, t);
                    }
                }
                Some("tool_use") => {
                    if block.get("name").and_then(|n| n.as_str()) == Some("Skill") {
                        if let Some(name) = block
                            .get("input")
                            .and_then(|i| i.get("skill"))
                            .and_then(|s| s.as_str())
                        {
                            if !skills_used.iter().any(|s| s == name) {
                                skills_used.push(name.to_string());
                            }
                        }
                    }
                }
                Some("tool_result") => {
                    // Real claude JSONL records tool failures here.
                    if block.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
                        tool_errors += 1;
                    }
                }
                _ => {}
            }
        }
    }

    SessionDigest {
        session_id: session_id.to_string(),
        title: title.to_string(),
        turns,
        skills_used,
        tool_errors,
        text,
    }
}

fn push_capped(buf: &mut String, role: &str, t: &str) {
    if buf.len() >= PER_SESSION_TEXT_CAP {
        return;
    }
    let remaining = PER_SESSION_TEXT_CAP - buf.len();
    let snippet: String = t.chars().take(remaining).collect();
    buf.push_str(if role == "user" { "\nUSER: " } else { "\nASSISTANT: " });
    buf.push_str(snippet.trim());
}

/// Convenience: does a transcript file exist for this session?
pub fn has_transcript(session: &Session) -> bool {
    match session.provider_session_id.as_deref() {
        Some(psid) => Path::new(&session_jsonl_path(&session.cwd, psid)).exists(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors the real claude JSONL shape: `Skill` tool_use carries
    // `input.skill`, and a failed tool surfaces `is_error: true` on the
    // `tool_result` content block of a user message.
    const JSONL: &str = concat!(
        r#"{"message":{"role":"user","content":[{"type":"text","text":"refund please"}]}}"#, "\n",
        r#"{"message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","name":"Skill","input":{"skill":"support-triage-router"}}]}}"#, "\n",
        r#"{"message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#, "\n",
        r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"routed to billing"}]}}"#, "\n"
    );

    #[test]
    fn extracts_skills_turns_errors_text() {
        let d = digest_from_jsonl("sess_1", "Slack chat", JSONL);
        assert_eq!(d.skills_used, vec!["support-triage-router"]);
        assert_eq!(d.tool_errors, 1);
        assert!(d.turns >= 2);
        assert!(d.text.contains("refund please"));
        assert!(d.text.contains("routed to billing"));
    }

    #[test]
    fn text_is_capped() {
        let big = "x".repeat(10_000);
        let line = format!(
            r#"{{"message":{{"role":"assistant","content":[{{"type":"text","text":"{big}"}}]}}}}"#
        );
        let d = digest_from_jsonl("s", "t", &line);
        assert!(d.text.len() <= PER_SESSION_TEXT_CAP + 32);
    }
}

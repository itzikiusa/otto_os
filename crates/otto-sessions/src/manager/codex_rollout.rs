//! Reading a codex rollout's first user prompt for provider-id capture
//! ([`super::pick_codex_rollout`]'s content arbitration). Split out of
//! `manager.rs` (LOC ratchet).

/// First user prompt recorded in a rollout, if any; `None` means "no prompt
/// reached this conversation yet". Two layouts:
///
/// - older codex: an `event_msg` whose payload is
///   `{"type": "user_message", "message": …}`;
/// - codex 0.167+ (seen on the field install, 2026-11): no `user_message`
///   event at all — the prompt is an `event_msg` `item_completed` whose
///   `item` is `{"type": "UserMessage", "content": [{"type": "text",
///   "text": …}]}` (it also appears as a `response_item` user message, after
///   the injected `<environment_context>` one, so that form is not used).
///   The rollout is also created eagerly at startup (meta line first), not
///   on the first prompt.
///
/// Reading only the old layout made every candidate "inconclusive", so two
/// or more same-cwd captures in flight (a review fan-out) stayed Ambiguous
/// until the window closed — "no matching session found; won't
/// auto-resume" — and a prompt that clearly differed no longer excluded
/// another session's rollout.
pub(super) fn codex_rollout_first_user_message(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(200) {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let Some(p) = v.get("payload") else {
            continue;
        };
        match p.get("type").and_then(|t| t.as_str()) {
            Some("user_message") => {
                return p.get("message").and_then(|m| m.as_str()).map(str::to_owned);
            }
            Some("item_completed") => {
                let Some(item) = p
                    .get("item")
                    .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("UserMessage"))
                else {
                    continue;
                };
                let text: String = item
                    .get("content")
                    .and_then(|c| c.as_array())
                    .into_iter()
                    .flatten()
                    .filter(|part| part.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect();
                return Some(text);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rollout(lines: &[serde_json::Value]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        f
    }

    fn meta() -> serde_json::Value {
        serde_json::json!({"type": "session_meta", "payload": {"session_id": "S",
            "cwd": "/w", "originator": "codex-tui", "thread_source": "user"}})
    }

    /// The codex 0.167 layout: the environment wrapper and the prompt as
    /// `response_item` user messages, then the `UserMessage` item. The item's
    /// text is the prompt (the wrapper is never mistaken for it).
    #[test]
    fn reads_the_user_message_item_of_current_codex() {
        let user = |text: &str| {
            serde_json::json!({"type": "response_item", "payload": {"type": "message",
                "role": "user", "content": [{"type": "input_text", "text": text}]}})
        };
        let f = rollout(&[
            meta(),
            serde_json::json!({"type": "event_msg", "payload": {"type": "task_started"}}),
            user("<environment_context>\n  <cwd>/w</cwd>\n</environment_context>"),
            user("Review the diff for correctness"),
            serde_json::json!({"type": "event_msg", "payload": {"type": "item_completed",
                "item": {"type": "UserMessage", "id": "i", "client_id": null,
                    "content": [{"type": "text", "text": "Review the diff for correctness",
                        "text_elements": []}]}}}),
        ]);
        assert_eq!(
            codex_rollout_first_user_message(f.path()).as_deref(),
            Some("Review the diff for correctness")
        );
    }

    /// Eagerly created (meta + startup events, no prompt yet) → `None`; the
    /// older `user_message` event still reads.
    #[test]
    fn no_prompt_yet_is_none_and_the_old_event_still_reads() {
        let started = serde_json::json!({"type": "event_msg", "payload": {"type": "task_started"}});
        let other_item = serde_json::json!({"type": "event_msg", "payload": {"type": "item_completed",
            "item": {"type": "AgentMessage", "content": [{"type": "text", "text": "hi"}]}}});
        let f = rollout(&[meta(), started, other_item]);
        assert_eq!(codex_rollout_first_user_message(f.path()), None);
        let f = rollout(&[
            meta(),
            serde_json::json!({"type": "event_msg", "payload": {"type": "user_message",
                "message": "Scan the repository"}}),
        ]);
        assert_eq!(
            codex_rollout_first_user_message(f.path()).as_deref(),
            Some("Scan the repository")
        );
    }
}

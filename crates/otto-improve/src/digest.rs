//! Provider-neutral, recent transcript evidence. Parsing delegates to the same
//! normalized adapters used by the conversation view.

use otto_transcript::{Block, FoldOpts, Provider, Role};
use serde_json::Value;

pub(crate) const PER_SESSION_TEXT_CAP: usize = 4000;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDigest {
    pub session_id: String,
    pub title: String,
    pub turns: usize,
    pub skills_used: Vec<String>,
    pub tool_errors: usize,
    /// Recent user/assistant evidence, bounded in Unicode characters.
    pub text: String,
}

pub fn digest_from_jsonl(session_id: &str, title: &str, body: &str) -> SessionDigest {
    digest_for_provider(session_id, title, Provider::Claude, body)
}

pub fn digest_for_provider(
    session_id: &str,
    title: &str,
    provider: Provider,
    body: &str,
) -> SessionDigest {
    let records: Vec<Value> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|mut v| {
            // Older Claude exports omit the redundant top-level type.
            if provider == Provider::Claude && v.get("type").is_none() {
                if let Some(role) = v
                    .pointer("/message/role")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    v["type"] = role.into();
                }
            }
            v
        })
        .collect();
    let folded = otto_transcript::fold(provider, &records, FoldOpts::default());
    let mut d = SessionDigest {
        session_id: session_id.into(),
        title: title.into(),
        turns: folded.turns.len(),
        skills_used: vec![],
        tool_errors: 0,
        text: String::new(),
    };
    let mut user_text = String::new();
    let mut assistant_text = String::new();
    for turn in folded.turns {
        let role = if turn.turn.role == Role::User {
            "USER"
        } else {
            "ASSISTANT"
        };
        for block in turn.turn.blocks {
            match block {
                Block::Text { md } => {
                    if turn.turn.role == Role::User {
                        collect_skill_mentions(&md, &mut d.skills_used);
                    }
                    append_recent(
                        if turn.turn.role == Role::User {
                            &mut user_text
                        } else {
                            &mut assistant_text
                        },
                        &format!("\n{role}: {md}"),
                    );
                }
                Block::ToolCall {
                    name,
                    input,
                    result,
                    ..
                } => {
                    if name == "Skill" {
                        if let Some(skill) = input.get("skill").and_then(Value::as_str) {
                            add_skill(&mut d.skills_used, skill);
                        }
                    }
                    if name == "ToolSearch" {
                        if let Some(rest) = input
                            .get("query")
                            .and_then(Value::as_str)
                            .and_then(|q| q.strip_prefix("select:"))
                        {
                            for name in rest.split(',') {
                                add_skill(&mut d.skills_used, name.trim());
                            }
                        }
                    }
                    if result.is_some_and(|r| !r.ok) {
                        d.tool_errors += 1;
                    }
                }
                _ => {}
            }
        }
    }
    // Legacy exports may omit tool IDs, so their results cannot attach to a
    // folded call. Count Claude failures from records once, not both formats.
    if provider == Provider::Claude {
        d.tool_errors = records
            .iter()
            .map(|v| {
                let blocks = v.pointer("/message/content").and_then(Value::as_array);
                let count = blocks
                    .map(|b| {
                        b.iter()
                            .filter(|b| {
                                b.get("type").and_then(Value::as_str) == Some("tool_result")
                                    && b.get("is_error").and_then(Value::as_bool) == Some(true)
                            })
                            .count()
                    })
                    .unwrap_or(0);
                count.max(usize::from(
                    v.pointer("/toolUseResult/is_error")
                        .and_then(Value::as_bool)
                        == Some(true),
                ))
            })
            .sum();
        // Skill calls without IDs are also tolerated by historical exports.
        for v in &records {
            if let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) {
                for b in blocks {
                    if b.get("name").and_then(Value::as_str) == Some("Skill") {
                        if let Some(skill) = b.pointer("/input/skill").and_then(Value::as_str) {
                            add_skill(&mut d.skills_used, skill);
                        }
                    }
                }
            }
        }
    }
    // A verbose assistant response must not evict the user's correction.
    // Keep independent recent role windows, giving direct user evidence most
    // of the budget when both are present.
    d.text = if user_text.is_empty() {
        assistant_text
    } else if assistant_text.is_empty() {
        user_text
    } else {
        format!(
            "{}{}",
            tail_chars(&user_text, 2500),
            tail_chars(&assistant_text, 1500)
        )
    };
    d
}

fn tail_chars(text: &str, cap: usize) -> String {
    text.chars()
        .rev()
        .take(cap)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub(crate) fn add_skill(skills: &mut Vec<String>, name: &str) {
    if !name.is_empty() && !skills.iter().any(|s| s == name) {
        skills.push(name.into());
    }
}

pub(crate) fn collect_skill_mentions(text: &str, skills: &mut Vec<String>) {
    for word in text.split_whitespace() {
        if let Some(name) = word.strip_prefix('/').or_else(|| word.strip_prefix('$')) {
            let name =
                name.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
            if name.len() > 2 && !name.contains('/') {
                add_skill(skills, name);
            }
        }
    }
}

/// Keep the newest evidence, including the end of a long correction. Unicode
/// characters, labels and separators all count against the same strict budget.
pub(crate) fn append_recent(buf: &mut String, text: &str) {
    let tail: String = text
        .chars()
        .rev()
        .take(PER_SESSION_TEXT_CAP)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    buf.push_str(&tail);
    let excess = buf.chars().count().saturating_sub(PER_SESSION_TEXT_CAP);
    if excess > 0 {
        let byte = buf
            .char_indices()
            .nth(excess)
            .map(|(i, _)| i)
            .unwrap_or(buf.len());
        buf.drain(..byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors the real claude JSONL shape: `Skill` tool_use carries
    // `input.skill`, and a failed tool surfaces `is_error: true` on the
    // `tool_result` content block of a user message.
    const JSONL: &str = concat!(
        r#"{"message":{"role":"user","content":[{"type":"text","text":"refund please"}]}}"#,
        "\n",
        r#"{"message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","name":"Skill","input":{"skill":"support-triage-router"}}]}}"#,
        "\n",
        r#"{"message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#,
        "\n",
        r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"routed to billing"}]}}"#,
        "\n"
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
    fn latest_correction_survives_long_history() {
        let body = format!(
            "{}\n{}",
            serde_json::json!({"message":{"role":"user","content":[{"type":"text","text":"old ".repeat(3000)}]}}),
            serde_json::json!({"message":{"role":"user","content":[{"type":"text","text":"Correction: never drop review comments for brevity"}]}})
        );
        let d = digest_from_jsonl("s", "title", &body);
        assert!(d.text.contains("Correction: never drop review comments"));
        assert!(d.text.chars().count() <= PER_SESSION_TEXT_CAP);
    }

    #[test]
    fn codex_deduplicates_event_and_response_messages() {
        let body = concat!(
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"Please use $correctness-review for this fix"}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Please use $correctness-review for this fix"}]}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"I will verify the failure"}}"#,
            "\n"
        );
        let d = digest_for_provider("codex", "work", Provider::Codex, body);
        assert_eq!(d.text.matches("Please use").count(), 1);
        assert!(d.text.contains("I will verify"));
        assert_eq!(d.skills_used, vec!["correctness-review"]);
    }

    #[test]
    fn unicode_budget_includes_labels_and_keeps_latest_evidence() {
        let mut text = String::new();
        append_recent(&mut text, &"א".repeat(5000));
        append_recent(&mut text, "\nUSER: preserve the final correction ✅");
        assert_eq!(text.chars().count(), PER_SESSION_TEXT_CAP);
        assert!(text.ends_with("correction ✅"));
    }

    #[test]
    fn correction_survives_a_verbose_assistant_response() {
        let body = format!(
            "{}\n{}",
            serde_json::json!({"type":"user","message":{"role":"user","content":"Correction: preserve all findings"}}),
            serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"verbose ".repeat(3000)}]}})
        );
        let d = digest_from_jsonl("s", "t", &body);
        assert!(d.text.contains("Correction: preserve all findings"));
        assert!(d.text.contains("verbose"));
        assert!(d.text.chars().count() <= PER_SESSION_TEXT_CAP);
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

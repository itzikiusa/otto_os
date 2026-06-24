//! Deterministic, offline agent replies for Playwright E2E (`OTTO_E2E=1`).
//!
//! Feature flows route their `run_agent` prompt through here by embedding a
//! stable `OTTO_TASK: <name>` sentinel, so a single stub can serve both the
//! discovery-chat (needs an `actions` JSON block) and canvas-assist (needs a
//! mermaid block) specs without a real `claude` PTY or any network.

/// Return a canned reply chosen by the `OTTO_TASK:` sentinel in `prompt`.
pub fn canned_reply(prompt: &str) -> String {
    if prompt.contains("OTTO_TASK: discovery_chat") {
        return discovery_chat_reply();
    }
    if prompt.contains("OTTO_TASK: canvas_assist") {
        return canvas_assist_reply();
    }
    // Generic fallback for any other agent call under E2E.
    "OK".to_string()
}

fn discovery_chat_reply() -> String {
    // Prose followed by a single fenced `actions` JSON block. The handler splits
    // markdown from the JSON and renders each action as a card.
    let actions = serde_json::json!({
        "actions": [
            {
                "type": "apply_draft",
                "title": "E2E Discovery Story",
                "body_md": "## Summary\nAs a user, I want X so that Y.\n\n## Acceptance\n- [ ] does X"
            },
            {
                "type": "add_questions",
                "questions": [
                    { "text": "What is the expected latency SLA?", "rationale": "non-functional", "category": "nfr" },
                    { "text": "Which user roles can access this?", "rationale": "auth", "category": "security" }
                ]
            }
        ]
    });
    format!(
        "Here's what I found from your draft and mockups. I drafted a story and a couple of questions to lock down.\n\n```json\n{}\n```",
        serde_json::to_string_pretty(&actions).unwrap_or_default()
    )
}

fn canvas_assist_reply() -> String {
    // A mermaid sequence diagram whose fan-out (B doing several steps) is the
    // golden example. The handler detects the ```mermaid fence first.
    "Here is the sequence you described — service A calls B, and B's steps are spelled out.\n\n\
```mermaid\n\
sequenceDiagram\n\
  participant A as Service A\n\
  participant B as Service B\n\
  A->>B: request\n\
  B->>B: validate input\n\
  B->>B: load record\n\
  B->>B: apply rules\n\
  B->>B: persist\n\
  B-->>A: response\n\
```"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::canned_reply;

    #[test]
    fn routes_by_sentinel() {
        assert!(canned_reply("foo OTTO_TASK: discovery_chat bar").contains("\"actions\""));
        assert!(canned_reply("x OTTO_TASK: canvas_assist y").contains("```mermaid"));
        assert_eq!(canned_reply("no sentinel"), "OK");
    }
}

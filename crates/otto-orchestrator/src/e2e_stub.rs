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
        return canvas_assist_reply(prompt);
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

fn canvas_assist_reply(prompt: &str) -> String {
    // Format-aware: the Excalidraw-mode prompt targets `canvas.json` → reply with a
    // simplified Excalidraw scene (the app expands it). Otherwise (Mermaid mode) →
    // a colour-coded flowchart. The handler prefers the file edit and falls back to
    // this reply's fenced block.
    if prompt.contains("canvas.json") || prompt.contains("EXCALIDRAW canvas") {
        return "Drew the order flow as Excalidraw shapes with a validation decision.\n\n\
```json\n\
{\"type\":\"excalidraw\",\"elements\":[\
{\"type\":\"rectangle\",\"id\":\"a\",\"x\":40,\"y\":40,\"width\":160,\"height\":60,\
\"backgroundColor\":\"#dcfce7\",\"strokeColor\":\"#16a34a\",\"label\":{\"text\":\"🚀 Start\"}},\
{\"type\":\"diamond\",\"id\":\"b\",\"x\":40,\"y\":160,\"width\":180,\"height\":80,\
\"backgroundColor\":\"#fef9c3\",\"strokeColor\":\"#ca8a04\",\"label\":{\"text\":\"❓ Valid?\"}},\
{\"type\":\"rectangle\",\"id\":\"c\",\"x\":300,\"y\":160,\"width\":160,\"height\":60,\
\"backgroundColor\":\"#eef2ff\",\"strokeColor\":\"#6366f1\",\"label\":{\"text\":\"⚙️ Process\"}},\
{\"type\":\"rectangle\",\"id\":\"d\",\"x\":40,\"y\":300,\"width\":160,\"height\":60,\
\"backgroundColor\":\"#fee2e2\",\"strokeColor\":\"#dc2626\",\"label\":{\"text\":\"❌ Reject\"}},\
{\"type\":\"arrow\",\"id\":\"e1\",\"start\":{\"id\":\"a\"},\"end\":{\"id\":\"b\"}},\
{\"type\":\"arrow\",\"id\":\"e2\",\"start\":{\"id\":\"b\"},\"end\":{\"id\":\"c\"},\"label\":{\"text\":\"yes\"}},\
{\"type\":\"arrow\",\"id\":\"e3\",\"start\":{\"id\":\"b\"},\"end\":{\"id\":\"d\"},\"label\":{\"text\":\"no\"}}\
]}\n\
```"
            .to_string();
    }
    "Here is the order flow you described, with a validation decision and a reject path.\n\n\
```mermaid\n\
flowchart TD\n\
  A([\"🚀 Start\"]) --> B{\"❓ Valid?\"}\n\
  B -->|yes| C[\"⚙️ Process order\"]\n\
  B -->|no| D[\"❌ Reject\"]\n\
  C --> E([\"✅ Done\"])\n\
  classDef start fill:#dcfce7,stroke:#16a34a,color:#064e3b;\n\
  classDef error fill:#fee2e2,stroke:#dc2626,color:#7f1d1d;\n\
  class A,E start;\n\
  class D error;\n\
```"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::canned_reply;

    #[test]
    fn routes_by_sentinel() {
        assert!(canned_reply("foo OTTO_TASK: discovery_chat bar").contains("\"actions\""));
        // Mermaid mode (no canvas.json in the prompt) → a mermaid fence.
        assert!(canned_reply("x OTTO_TASK: canvas_assist y").contains("```mermaid"));
        assert_eq!(canned_reply("no sentinel"), "OK");
    }

    #[test]
    fn canvas_reply_is_format_aware() {
        // Excalidraw mode (prompt edits canvas.json) → an excalidraw JSON scene.
        let ex = canned_reply("OTTO_TASK: canvas_assist edit `canvas.json` please");
        assert!(ex.contains("```json"));
        assert!(ex.contains("\"type\":\"excalidraw\""));
        assert!(!ex.contains("```mermaid"));
    }
}

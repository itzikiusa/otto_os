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
    if prompt.contains("OTTO_TASK: mockup_assist") {
        return mockup_assist_reply(prompt);
    }
    if prompt.contains("OTTO_TASK: db_assist") {
        return db_assist_reply();
    }
    // Generic fallback for any other agent call under E2E.
    "OK".to_string()
}

/// E2E stub for the DB Assistant. The real agent writes its query to `ANSWER.sql`;
/// offline we can't, so we return a one-line note + a fenced read-only query that
/// the db_assist handler extracts as the proposed SQL.
fn db_assist_reply() -> String {
    "Grouped player counts by brand.\n\n```sql\nSELECT brand_id, COUNT(*) AS players \
     FROM player_details GROUP BY brand_id ORDER BY players DESC\n```"
        .to_string()
}

fn mockup_assist_reply(prompt: &str) -> String {
    // Format-aware: the HTML-mode prompt edits `mockup.html` → reply with a tiny
    // self-contained page (marker `E2E mockup`). Otherwise (Mermaid mode) → a
    // flowchart. The handler prefers the file edit and falls back to this fence.
    if prompt.contains("mockup.html") || prompt.contains("HTML mockup") {
        return "Built a settings-page mockup.\n\n\
```html\n\
<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>E2E mockup</title>\
<style>body{font:15px/1.5 system-ui;padding:32px;color:#0f172a;background:#f8fafc}\
.card{max-width:520px;margin:0 auto;background:#fff;border:1px solid #e2e8f0;border-radius:12px;padding:24px}\
h1{font-size:20px;margin:0 0 16px}</style></head>\
<body><div class=\"card\"><h1>E2E mockup — Settings</h1>\
<p>Notifications, security, and billing for the account.</p></div></body></html>\n\
```"
            .to_string();
    }
    "Drew the flow as a Mermaid diagram.\n\n\
```mermaid\n\
flowchart TD\n\
  A([\"Open settings\"]) --> B[\"Edit profile\"]\n\
  B --> C([\"Saved\"])\n\
```"
        .to_string()
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
        // Mockup: HTML mode (edits mockup.html) → an html fence with the marker.
        let mh = canned_reply("OTTO_TASK: mockup_assist edit `mockup.html`");
        assert!(mh.contains("```html") && mh.contains("E2E mockup"));
        // Mockup: Mermaid mode (edits mockup.mmd) → a mermaid fence.
        assert!(canned_reply("OTTO_TASK: mockup_assist edit `mockup.mmd`").contains("```mermaid"));
        // DB Assistant → a read-only SQL fence the handler extracts as the query.
        let db = canned_reply("OTTO_TASK: db_assist read SCHEMA.md");
        assert!(db.contains("```sql") && db.to_uppercase().contains("SELECT"));
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

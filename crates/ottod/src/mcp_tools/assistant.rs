//! Otto Assistant tools (native; only advertised to `source: "assistant"`
//! sessions). Moved out of `mcp_tools.rs` verbatim (architecture-1-05).

use super::*;

// ---------------------------------------------------------------------------
// Otto Assistant tools (native; only advertised to `source: "assistant"`
// sessions). Each posts to `POST /assistant/agent/{tool}` AS THE SESSION
// OWNER with this session's id; the daemon resolves the session to its
// assistant thread (the token's session binding wins over the body), so a
// session can only ever act for its own thread. They write only the owner's
// own assistant data (memories with chips + Undo, tasks, reminders); the
// outward-action tool OPENS an approval, it never performs the action.
// ---------------------------------------------------------------------------

/// `(native tool name, /assistant/agent/{tool} segment)`.
pub(super) const ASSISTANT_TOOLS: [(&str, &str); 8] = [
    ("assistant_remember", "remember"),
    ("assistant_forget", "forget"),
    ("assistant_recall", "recall"),
    ("assistant_create_reminder", "reminder"),
    ("assistant_create_task", "task"),
    ("assistant_update_task", "task_update"),
    ("assistant_delegate", "delegate"),
    ("assistant_request_approval", "approval"),
];

/// The session source that sees the assistant tools.
pub(super) const ASSISTANT_SOURCE: &str = "assistant";

pub(super) fn assistant_tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "assistant_remember",
            "description": "Otto Assistant: save ONE short, atomic fact about the user (a preference, a person, a recurring plan). Never store secrets. The user sees it as a chip with Undo; with memory approval on it waits for their review.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string" },
                "kind": { "type": "string", "description": "fact | decision | learning … (default fact)" },
                "tags": { "type": "array", "items": { "type": "string" } }
            }, "required": ["text"] }
        }),
        json!({
            "name": "assistant_forget",
            "description": "Otto Assistant: forget the user's memories matching `query` (\"forget my old address\"). Up to 10 go; the user can undo. Say what was forgotten.",
            "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] }
        }),
        json!({
            "name": "assistant_recall",
            "description": "Otto Assistant: read the user's profile and the memories matching `query` (omit it for the most recent). Use before answering anything personal.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string" },
                "k": { "type": "integer", "minimum": 1, "maximum": 20 }
            } }
        }),
        json!({
            "name": "assistant_create_reminder",
            "description": "Otto Assistant: remind the user at `run_at` (RFC3339, or local `YYYY-MM-DDTHH:MM` in `timezone`, default the Mac's). Delivered into this thread and as a notification.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string" },
                "run_at": { "type": "string" },
                "timezone": { "type": "string", "description": "IANA name, e.g. Asia/Jerusalem" }
            }, "required": ["text", "run_at"] }
        }),
        json!({
            "name": "assistant_create_task",
            "description": "Otto Assistant: open a task card for a longer job (it starts `running`). Keep it current with assistant_update_task.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "detail": { "type": "string" }
            }, "required": ["title"] }
        }),
        json!({
            "name": "assistant_update_task",
            "description": "Otto Assistant: move a task you created to `running`, `needs_you` (with a `question`, optional `options`), `done` or `failed` (optional `result` object).",
            "inputSchema": { "type": "object", "properties": {
                "task_id": { "type": "string" },
                "state": { "type": "string", "enum": ["running", "needs_you", "done", "failed"] },
                "question": { "type": "string" },
                "options": { "type": "array", "items": { "type": "string" } },
                "result": { "type": "object" }
            }, "required": ["task_id", "state"] }
        }),
        json!({
            "name": "assistant_delegate",
            "description": "Otto Assistant: hand a directive to one of the user's Personal Agents (`agent` = its id or exact name). It runs in the background; its summary is posted back into this thread. Tell the user who you asked.",
            "inputSchema": { "type": "object", "properties": {
                "agent": { "type": "string" },
                "directive": { "type": "string" }
            }, "required": ["agent", "directive"] }
        }),
        json!({
            "name": "assistant_request_approval",
            "description": "Otto Assistant: REQUIRED before anything outward (send, post, publish, purchase, delete, submit a form, anything touching production). State `where` it goes, `what` is sent, `who_sees` it and your `reason`; add `tool` + `destination` when known and a `category`. Returns `approved`, `denied` or `pending` (pass `wait_seconds` ≤ 30 to wait). Proceed ONLY on approved.",
            "inputSchema": { "type": "object", "properties": {
                "where": { "type": "string" },
                "what": { "type": "string" },
                "who_sees": { "type": "string" },
                "reason": { "type": "string" },
                "tool": { "type": "string" },
                "destination": { "type": "string" },
                "category": { "type": "string", "enum": ["send", "post", "publish", "purchase", "delete", "submit", "prod", "other"] },
                "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 30 }
            }, "required": ["where", "what", "who_sees", "reason"] }
        }),
    ]
}

/// The `/assistant/agent/{tool}` segment of a native assistant tool name.
pub(super) fn assistant_segment(name: &str) -> Option<&'static str> {
    ASSISTANT_TOOLS
        .iter()
        .find_map(|(n, seg)| (*n == name).then_some(*seg))
}

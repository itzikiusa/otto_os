//! agy (Antigravity Gemini CLI) conversation discovery for the session
//! manager's provider-id capture.

/// Root directory agy (Antigravity Gemini CLI) writes conversations to:
/// `~/.gemini/antigravity-cli`. Holds `conversations/<id>.db|.pb` (one per
/// conversation, named by its UUID) plus a `cache/last_conversations.json`
/// map of `cwd -> most-recent conversation id`.
pub(crate) fn agy_cli_root() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".gemini")
        .join("antigravity-cli")
}

/// One scan-and-pick pass: read `cache/last_conversations.json`, look up `cwd`,
/// and accept the mapped conversation id when its `conversations/<id>.{db,pb}`
/// file exists, is modified at/after `floor`, and isn't already `claimed`. Split
/// out from the capture loop (`spawn_session_id_capture`) so it's synchronously
/// testable. agy mints its own conversation id (like codex); the cwd-keyed map
/// makes this inherently sole-candidate.
pub(crate) fn scan_agy_conversation(
    cli_root: &std::path::Path,
    cwd: &str,
    floor: std::time::SystemTime,
    claimed: &std::collections::HashSet<&str>,
) -> Option<String> {
    let map_path = cli_root.join("cache").join("last_conversations.json");
    let raw = std::fs::read_to_string(&map_path).ok()?;
    let map: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let id = map.get(cwd).and_then(|v| v.as_str())?;
    if claimed.contains(id) {
        return None;
    }
    if !agy_conversation_fresh(cli_root, id, floor) {
        return None;
    }
    Some(id.to_string())
}

/// True when agy's conversation file `conversations/<id>.db|.pb` exists and was
/// modified at/after `floor` — i.e. created/touched by THIS launch, not a stale
/// pre-existing conversation that happened to share the same cwd.
fn agy_conversation_fresh(
    cli_root: &std::path::Path,
    id: &str,
    floor: std::time::SystemTime,
) -> bool {
    let dir = cli_root.join("conversations");
    ["db", "pb"].iter().any(|ext| {
        std::fs::metadata(dir.join(format!("{id}.{ext}")))
            .and_then(|m| m.modified())
            .map(|m| m >= floor)
            .unwrap_or(false)
    })
}

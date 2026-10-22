//! The per-session identity + policy context handed to the first-party
//! `ottod mcp-tools` server: the env Claude/agy/grok children inherit, and the
//! 0600 creds file Codex's `--config` names (moved out of `manager.rs`).

use super::*;

use crate::http::MCP_PROFILES_META;

/// The session's MCP profile ids as the bridge receives them
/// (`OTTO_MCP_PROFILES` / the creds file's `profiles`): comma-separated, or
/// `default` when none is stored (existing sessions use the default).
pub(crate) fn session_mcp_profiles(meta: &serde_json::Value) -> String {
    let ids: Vec<&str> = meta
        .get(MCP_PROFILES_META)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.contains(','))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        "default".to_string()
    } else {
        ids.join(",")
    }
}

/// Write the per-session Codex creds file (0600) carrying the token + routing the
/// `ottod mcp-tools --config <path>` process reads. Returns the path on success.
pub(super) fn write_codex_creds(
    session_id: &Id,
    token: &str,
    base: &str,
    workspace_id: &str,
    source: Option<&str>,
    profiles: &str,
) -> std::io::Result<std::path::PathBuf> {
    let path = codex_creds_path(session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "token": token,
        "base": base,
        "session_id": session_id.to_string(),
        "workspace_id": workspace_id,
        "source": source,
        "profiles": profiles,
    })
    .to_string();
    std::fs::write(&path, body)?;
    // Lock down to owner-only — it holds a bearer token.
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&path, perms)?;
    Ok(path)
}

/// Build the environment consumed by the first-party MCP subprocess for
/// Claude/agy/grok sessions. Keep the session source in this pure builder: the
/// docs-review read-only policy depends on it, and every env-based provider
/// must receive the same policy context — the MCP profile included, so every
/// provider's `otto` server lists and dispatches the same narrowed tool set.
pub(super) fn otto_tools_env(
    session: &Session,
    token: &str,
    base: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    env.insert("OTTO_MCP_TOKEN".to_string(), token.to_string());
    env.insert("OTTO_MCP_BASE".to_string(), base.to_string());
    env.insert("OTTO_SESSION_ID".to_string(), session.id.to_string());
    env.insert(
        "OTTO_WORKSPACE_ID".to_string(),
        session.workspace_id.to_string(),
    );
    if let Some(source) = session
        .meta
        .get("source")
        .and_then(serde_json::Value::as_str)
    {
        env.insert("OTTO_SESSION_SOURCE".to_string(), source.to_string());
    }
    env.insert(
        "OTTO_MCP_PROFILES".to_string(),
        session_mcp_profiles(&session.meta),
    );
    env
}

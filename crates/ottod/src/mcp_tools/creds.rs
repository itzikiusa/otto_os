//! `ottod mcp-tools` credentials + routing: the per-session env (the Claude /
//! agy / grok path) or the per-session creds file (the Codex path) — moved
//! out of `mcp_tools.rs`; now also carries the session's MCP profile ids.

use serde_json::Value;

/// Resolved credentials + routing for the tools — from env (the Claude path) or a
/// per-session creds file (the Codex path).
pub(super) struct Creds {
    pub(super) token: String,
    pub(super) base: Option<String>,
    pub(super) session_id: Option<String>,
    pub(super) workspace_id: Option<String>,
    pub(super) source: Option<String>,
    /// The session's MCP profile ids (`OTTO_MCP_PROFILES` / the creds file's
    /// `profiles`), comma-separated; `None` ⇒ no profile filter at all.
    pub(super) profiles: Option<String>,
}

/// Find the creds-file path: `--config <path>` / `--config=<path>` in `args`, else
/// the `OTTO_MCP_CONFIG` env. Pure over `args` for testability.
pub(super) fn config_path_in(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--config" {
            return it.next().cloned();
        }
        if let Some(p) = a.strip_prefix("--config=") {
            return Some(p.to_string());
        }
    }
    std::env::var("OTTO_MCP_CONFIG")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Parse a per-session creds JSON document (`{token, base?, session_id?,
/// workspace_id?}`). Pure for testability; errors if the `token` is missing.
pub(super) fn parse_creds(body: &str) -> Result<Creds, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("parse creds: {e}"))?;
    let token = v
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if token.is_empty() {
        return Err("creds file has no `token`".into());
    }
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Ok(Creds {
        token,
        base: s("base"),
        session_id: s("session_id"),
        workspace_id: s("workspace_id"),
        source: s("source"),
        profiles: s("profiles"),
    })
}

/// Resolve credentials: env first (Claude — `OTTO_MCP_TOKEN` & friends), then a
/// per-session creds file (Codex — `--config <path>` / `OTTO_MCP_CONFIG`).
pub(super) fn load_creds(args: &[String]) -> Result<Creds, String> {
    if let Ok(token) = std::env::var("OTTO_MCP_TOKEN") {
        if !token.is_empty() {
            let s = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
            return Ok(Creds {
                token,
                base: s("OTTO_MCP_BASE"),
                session_id: s("OTTO_SESSION_ID"),
                workspace_id: s("OTTO_WORKSPACE_ID"),
                source: s("OTTO_SESSION_SOURCE"),
                profiles: s("OTTO_MCP_PROFILES"),
            });
        }
    }
    let path = config_path_in(args).ok_or_else(|| {
        "no OTTO_MCP_TOKEN and no --config/OTTO_MCP_CONFIG creds file (the first-party \
         tools require a per-session token)"
            .to_string()
    })?;
    let body = std::fs::read_to_string(&path).map_err(|e| format!("read creds {path}: {e}"))?;
    parse_creds(&body)
}

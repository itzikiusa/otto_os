//! Browser tools via MCP. Otto can give an agent session a browser by writing
//! a `<workspace>/.mcp.json` entry that claude/codex load on launch. We
//! preserve every other key in the file and only manage the `otto-browser`
//! server entry.
//!
//! The browser MCP binary is discovered in this order:
//!   1. `OTTO_BROWSER_MCP` env (explicit command, shell-split)
//!   2. loom's `loom-mcp-browser` next to the daemon / on PATH
//!   3. fallback: `npx -y @playwright/mcp@latest`

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

const SERVER_KEY: &str = "otto-browser";

/// The `.mcp.json` key for Otto's own first-party read-only tool server
/// (Task B2b). Opt-in per workspace (`otto_mcp_enabled`); never auto-injected.
const OTTO_TOOLS_KEY: &str = "otto";

/// The command Otto uses to launch the browser MCP server.
pub fn browser_command() -> (String, Vec<String>) {
    if let Ok(cmd) = std::env::var("OTTO_BROWSER_MCP") {
        let parts = shell_words::split(&cmd).unwrap_or_default();
        if let Some((program, args)) = parts.split_first() {
            return (program.clone(), args.to_vec());
        }
    }
    if let Some(bin) = discover_loom_browser() {
        return (bin, vec![]);
    }
    (
        "npx".to_string(),
        vec!["-y".into(), "@playwright/mcp@latest".into()],
    )
}

fn discover_loom_browser() -> Option<String> {
    // Next to the running daemon binary.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("loom-mcp-browser");
            if sibling.is_file() {
                return Some(sibling.to_string_lossy().into_owned());
            }
        }
    }
    // On PATH.
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = Path::new(dir).join("loom-mcp-browser");
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn mcp_path(workspace_root: &str) -> PathBuf {
    Path::new(workspace_root).join(".mcp.json")
}

/// Add (or refresh) the browser MCP entry in the workspace `.mcp.json`,
/// preserving all other content. Best-effort: errors are returned for logging.
pub fn enable_browser(workspace_root: &str) -> Result<(), String> {
    let path = mcp_path(workspace_root);
    let mut doc = read_doc(&path)?;
    let servers = doc
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;
    let (command, args) = browser_command();
    servers.insert(
        SERVER_KEY.to_string(),
        json!({ "command": command, "args": args }),
    );
    write_doc(&path, &doc)
}

/// A user-configured MCP server to merge into the workspace `.mcp.json`. Mirrors
/// the persisted `otto_core::domain::McpServer` (name/command/args/env), kept as
/// a plain struct so `otto-sessions` needn't depend on `otto-state`.
#[derive(Debug, Clone)]
pub struct UserMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Merge the user's enabled MCP servers into the workspace `.mcp.json`,
/// preserving every other key — including Otto's own `otto-browser` entry. Each
/// server is written under its `name`; an `env` map is only emitted when
/// non-empty. Best-effort: errors are returned for logging.
///
/// This does NOT remove servers the user has since disabled/deleted: a stale
/// entry written on a prior spawn is the user's `.mcp.json` to manage, and we
/// avoid silently dropping a key they may have hand-edited. We never auto-enable
/// — callers pass only the rows the user flipped on.
pub fn merge_user_servers(workspace_root: &str, servers: &[UserMcpServer]) -> Result<(), String> {
    if servers.is_empty() {
        return Ok(());
    }
    let path = mcp_path(workspace_root);
    let mut doc = read_doc(&path)?;
    let map = doc
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;
    for s in servers {
        // Reserved for Otto's managed servers — don't let a user entry named
        // "otto-browser" or "otto" clobber them via this merge path.
        if s.name == SERVER_KEY || s.name == OTTO_TOOLS_KEY {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("command".into(), Value::String(s.command.clone()));
        entry.insert(
            "args".into(),
            Value::Array(s.args.iter().cloned().map(Value::String).collect()),
        );
        if !s.env.is_empty() {
            let env: Map<String, Value> = s
                .env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            entry.insert("env".into(), Value::Object(env));
        }
        map.insert(s.name.clone(), Value::Object(entry));
    }
    write_doc(&path, &doc)
}

/// The command + env for Otto's first-party read-only MCP tool server
/// (Task B2b). The command runs the `ottod mcp-tools` subcommand; the env
/// carries the per-session **read-only** credential the tools authorize with
/// (the same per-session token Otto already mints for `/ingest/*`), the daemon
/// base URL the tools call back into, and the session/workspace identifiers used
/// for RBAC scoping + audit. `env` values are secrets-in-transit only — they
/// live in the spawned child's environment, never persisted to the repo.
#[derive(Debug, Clone)]
pub struct OttoToolsServer {
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Add (or refresh) Otto's first-party `otto` MCP tool server in the workspace
/// `.mcp.json`, preserving every other key (including `otto-browser` and the
/// user's own servers). Called on agent spawn **only** when `otto_mcp_enabled`
/// is on for the workspace. Best-effort: errors are returned for logging and
/// never block a spawn.
pub fn enable_otto_tools(workspace_root: &str, server: &OttoToolsServer) -> Result<(), String> {
    let path = mcp_path(workspace_root);
    let mut doc = read_doc(&path)?;
    let servers = doc
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;
    let mut entry = Map::new();
    entry.insert("command".into(), Value::String(server.command.clone()));
    entry.insert(
        "args".into(),
        Value::Array(server.args.iter().cloned().map(Value::String).collect()),
    );
    if !server.env.is_empty() {
        let env: Map<String, Value> = server
            .env
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        entry.insert("env".into(), Value::Object(env));
    }
    servers.insert(OTTO_TOOLS_KEY.to_string(), Value::Object(entry));
    write_doc(&path, &doc)
}

/// Remove Otto's first-party `otto` MCP tool server, preserving everything else.
/// No-op if absent. Mirrors [`disable_browser`].
pub fn disable_otto_tools(workspace_root: &str) -> Result<(), String> {
    let path = mcp_path(workspace_root);
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_doc(&path)?;
    if let Some(servers) = doc.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove(OTTO_TOOLS_KEY);
    }
    write_doc(&path, &doc)
}

/// Remove the browser MCP entry, preserving everything else. No-op if absent.
pub fn disable_browser(workspace_root: &str) -> Result<(), String> {
    let path = mcp_path(workspace_root);
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_doc(&path)?;
    if let Some(servers) = doc.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove(SERVER_KEY);
    }
    write_doc(&path, &doc)
}

fn read_doc(path: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => {
            serde_json::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display()))
        }
        _ => Ok(Map::new()),
    }
}

fn write_doc(path: &Path, doc: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn read_servers(root: &str) -> Map<String, Value> {
        let doc = read_doc(&mcp_path(root)).unwrap();
        doc.get("mcpServers")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default()
    }

    /// Enabling the first-party `otto` server writes a command+args+env entry and
    /// preserves every other key already in the file (browser + a user server).
    #[test]
    fn enable_otto_tools_preserves_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();

        // Pre-seed a user server and Otto's browser entry.
        enable_browser(root).unwrap();
        merge_user_servers(
            root,
            &[UserMcpServer {
                name: "myserver".into(),
                command: "node".into(),
                args: vec!["x.js".into()],
                env: BTreeMap::new(),
            }],
        )
        .unwrap();

        let mut env = BTreeMap::new();
        env.insert("OTTO_MCP_TOKEN".to_string(), "secret-token".to_string());
        env.insert("OTTO_SESSION_ID".to_string(), "sess-1".to_string());
        enable_otto_tools(
            root,
            &OttoToolsServer {
                command: "/usr/local/bin/ottod".into(),
                args: vec!["mcp-tools".into()],
                env,
            },
        )
        .unwrap();

        let servers = read_servers(root);
        // Otto's own entry is present and shaped right.
        let otto = servers.get("otto").and_then(|v| v.as_object()).unwrap();
        assert_eq!(otto["command"], json!("/usr/local/bin/ottod"));
        assert_eq!(otto["args"], json!(["mcp-tools"]));
        assert_eq!(otto["env"]["OTTO_MCP_TOKEN"], json!("secret-token"));
        // Pre-existing servers are untouched.
        assert!(servers.contains_key("otto-browser"));
        assert!(servers.contains_key("myserver"));
    }

    /// A user server named "otto" can never clobber the managed first-party entry
    /// via the merge path.
    #[test]
    fn user_server_named_otto_is_skipped_by_merge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        merge_user_servers(
            root,
            &[UserMcpServer {
                name: "otto".into(),
                command: "evil".into(),
                args: vec![],
                env: BTreeMap::new(),
            }],
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(
            !servers.contains_key("otto"),
            "a user server named 'otto' must be dropped by merge_user_servers"
        );
    }

    /// Disabling removes only the `otto` key.
    #[test]
    fn disable_otto_tools_removes_only_otto() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        enable_browser(root).unwrap();
        enable_otto_tools(
            root,
            &OttoToolsServer {
                command: "ottod".into(),
                args: vec!["mcp-tools".into()],
                env: BTreeMap::new(),
            },
        )
        .unwrap();
        disable_otto_tools(root).unwrap();
        let servers = read_servers(root);
        assert!(!servers.contains_key("otto"));
        assert!(servers.contains_key("otto-browser"));
    }
}

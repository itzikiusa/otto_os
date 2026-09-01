//! MCP launcher config for agent sessions. Otto wires browser tools, the
//! user's enabled MCP servers, and its own first-party `otto` tool server into
//! the config each agent CLI actually reads:
//!
//!   - claude/agy: `<workspace>/.mcp.json`
//!   - grok:       `<workspace>/.grok/config.toml` (never `.mcp.json`)
//!   - codex:      per-spawn `-c mcp_servers.*` overrides (never `.mcp.json`)
//!
//! Everything Otto writes is **reconciled, not accumulated**: each file carries
//! a marker key listing the server names Otto manages there, and every spawn
//! removes marker entries no longer in the enabled set, upserts the current
//! set, and rewrites the marker. Entries the user hand-added (names absent from
//! the marker) are never touched. The whole read→reconcile→write is a single
//! RMW guarded by a per-cwd lock and lands via temp-file + rename, so
//! concurrent spawns sharing a cwd (e.g. PR-review sessions) can't tear or
//! lose each other's updates.
//!
//! The browser MCP binary is discovered in this order:
//!   1. `OTTO_BROWSER_MCP` env (explicit command, shell-split)
//!   2. loom's `loom-mcp-browser` next to the daemon / on PATH
//!   3. fallback: `npx -y @playwright/mcp@latest`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Map, Value};

const SERVER_KEY: &str = "otto-browser";

/// The `.mcp.json` key for Otto's own first-party read-only tool server
/// (Task B2b). Opt-in per workspace (`otto_mcp_enabled`); never auto-injected.
const OTTO_TOOLS_KEY: &str = "otto";

/// Top-level `.mcp.json` marker listing the `mcpServers` names Otto manages.
/// Reconciliation removes stale names from this set and never touches names
/// outside it, so user hand-edits survive while Otto's own entries track the
/// enabled state exactly.
const MANAGED_KEY: &str = "ottoManagedServers";

/// The grok `config.toml` counterpart of [`MANAGED_KEY`] (snake_case to match
/// the surrounding TOML keys).
const MANAGED_KEY_TOML: &str = "otto_managed_servers";

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

/// Per-cwd write lock: several sessions can share one cwd (PR-review sessions
/// all spawn in the repo root), and each spawn does a read→reconcile→write of
/// the same files. A process-global registry keyed by workspace root serializes
/// them; the guard is only ever held across synchronous fs work (no awaits), so
/// a plain std mutex is enough and every caller — create, restart, tests — is
/// covered without plumbing state through `SessionManager`.
fn cwd_lock(workspace_root: &str) -> std::sync::Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, std::sync::Arc<Mutex<()>>>>> = OnceLock::new();
    let registry = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(workspace_root.to_string())
        .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
        .clone()
}

/// A user-configured MCP server to reconcile into the workspace launcher
/// configs. Mirrors the persisted `otto_core::domain::McpServer`
/// (name/command/args/env), kept as a plain struct so `otto-sessions` needn't
/// depend on `otto-state`.
#[derive(Debug, Clone)]
pub struct UserMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// The command plus per-session environment for Otto's first-party MCP tool
/// server. The shared project launchers persist only command/args; callers put
/// `env` on the individual agent process so its MCP child inherits the correct
/// credential and policy without contaminating another session in the same cwd.
#[derive(Debug, Clone)]
pub struct OttoToolsServer {
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Everything Otto manages in a workspace's `.mcp.json`, resolved once per
/// spawn. `None`/`false`/empty means "not enabled" — reconciliation then
/// REMOVES the corresponding managed entries, which is how a disable or delete
/// propagates to the file.
#[derive(Debug, Clone, Default)]
pub struct ManagedMcpConfig {
    /// Session opted into browser tools (`meta.browser == true`).
    pub browser: bool,
    /// The workspace's *enabled* user-configured servers. Never auto-enabled —
    /// callers pass only the rows the user flipped on.
    pub user_servers: Vec<UserMcpServer>,
    /// Otto's first-party tool server, when the workspace has it enabled.
    /// Per-session env is deliberately NOT written to the shared file: several
    /// sessions can share this cwd, and persisting one session's token here
    /// would make the last spawner's capabilities win for every later resume.
    pub otto_tools: Option<OttoToolsServer>,
}

/// Reconcile the workspace `.mcp.json` against `cfg` — the ONE write path for
/// everything Otto manages there (browser, user servers, `otto` tools). Runs
/// even when the config is empty so disables/deletes propagate as removals.
/// Preserves every key Otto doesn't manage; best-effort — errors are returned
/// for logging. Returns the managed server names now in the file (the marker),
/// so callers can snapshot what the session actually got.
pub fn reconcile_managed_servers(
    workspace_root: &str,
    cfg: &ManagedMcpConfig,
) -> Result<Vec<String>, String> {
    let path = mcp_path(workspace_root);
    let lock = cwd_lock(workspace_root);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    // Desired managed entries, in file order: browser, otto tools, user servers.
    let mut desired: Vec<(String, Value)> = Vec::new();
    if cfg.browser {
        let (command, args) = browser_command();
        desired.push((
            SERVER_KEY.to_string(),
            json!({ "command": command, "args": args }),
        ));
    }
    if let Some(t) = &cfg.otto_tools {
        // Identity-neutral: command/args only, never the session env.
        desired.push((
            OTTO_TOOLS_KEY.to_string(),
            json!({ "command": t.command, "args": t.args }),
        ));
    }
    for s in &cfg.user_servers {
        // Reserved for Otto's own entries — a user server named "otto-browser"
        // or "otto" can't clobber them.
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
        desired.push((s.name.clone(), Value::Object(entry)));
    }
    let desired_names: Vec<String> = desired.iter().map(|(n, _)| n.clone()).collect();

    // Nothing to manage and nothing on disk: don't create a file just to hold
    // an empty map.
    if desired.is_empty() && !path.exists() {
        return Ok(Vec::new());
    }

    let mut doc = read_doc(&path)?;

    // The previously managed set. Files written before the marker existed
    // (the old accumulate-only merge) get an adoption pass: Otto's reserved
    // names ("otto", "otto-browser") are always Otto's, and an existing entry
    // whose name matches a currently-enabled user server was — with
    // overwhelming likelihood — written by a prior spawn (the old merge
    // overwrote same-named hand-edits anyway). Entries matching neither are
    // treated as hand-added and left alone forever; a server that was disabled
    // BEFORE this marker shipped is indistinguishable from a hand-edit, so its
    // stale entry stays until the user removes it (documented residual).
    let prior: Vec<String> = match doc.get(MANAGED_KEY).and_then(Value::as_array) {
        Some(names) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        None => {
            let existing: Vec<String> = doc
                .get("mcpServers")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            existing
                .into_iter()
                .filter(|n| {
                    n == SERVER_KEY || n == OTTO_TOOLS_KEY || desired_names.contains(n)
                })
                .collect()
        }
    };

    let servers = doc
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;
    // Remove what Otto wrote before and no longer wants; upsert the rest.
    for name in &prior {
        if !desired_names.contains(name) {
            servers.remove(name);
        }
    }
    for (name, entry) in desired {
        servers.insert(name, entry);
    }
    if desired_names.is_empty() {
        doc.remove(MANAGED_KEY);
    } else {
        doc.insert(
            MANAGED_KEY.to_string(),
            Value::Array(desired_names.iter().cloned().map(Value::String).collect()),
        );
    }
    write_doc(&path, &doc)?;
    Ok(desired_names)
}

/// TOML-quote a string value for a Codex `-c key=value` override (Codex parses the
/// value as TOML). Wraps in double quotes, escaping `\` and `"`.
fn toml_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// A bare (unquoted) TOML key segment — what we can safely splice into a Codex
/// `-c mcp_servers.<name>.…` dotted path without knowing how strictly Codex's
/// override parser handles quoted segments.
fn is_bare_toml_key(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The Codex `-c` config overrides that attach Otto's `otto` MCP server to a Codex
/// session. Codex doesn't read the workspace `.mcp.json`, so instead of editing its
/// global `~/.codex/config.toml` (shared, and unable to carry a per-session token)
/// we pass per-spawn overrides that run `<ottod> mcp-tools --config <creds_path>`.
/// The per-session token lives in the (0600) creds file at `creds_path` — never on
/// argv. Returns the args to append to the Codex launch command; non-destructive.
pub fn codex_mcp_inject_args(ottod: &str, creds_path: &str) -> Vec<String> {
    vec![
        "-c".to_string(),
        format!("mcp_servers.otto.command={}", toml_str(ottod)),
        "-c".to_string(),
        format!(
            "mcp_servers.otto.args=[{},{},{}]",
            toml_str("mcp-tools"),
            toml_str("--config"),
            toml_str(creds_path)
        ),
    ]
}

/// The Codex `-c` overrides that attach the user's enabled MCP servers to a
/// Codex session — the codex counterpart of the `.mcp.json` user entries, using
/// the same per-spawn mechanism as [`codex_mcp_inject_args`]. Per-spawn, so
/// disables propagate for free (a disabled server simply isn't emitted).
/// Servers whose name can't be spliced into a bare TOML key path are skipped
/// with a debug log rather than failing the spawn.
pub fn codex_user_server_args(servers: &[UserMcpServer]) -> Vec<String> {
    let mut out = Vec::new();
    for s in servers {
        if s.name == SERVER_KEY || s.name == OTTO_TOOLS_KEY {
            continue; // reserved for Otto's managed entries
        }
        if !is_bare_toml_key(&s.name) {
            tracing::debug!(server = %s.name, "codex MCP: name not a bare TOML key, skipping");
            continue;
        }
        out.push("-c".to_string());
        out.push(format!(
            "mcp_servers.{}.command={}",
            s.name,
            toml_str(&s.command)
        ));
        out.push("-c".to_string());
        let args: Vec<String> = s.args.iter().map(|a| toml_str(a)).collect();
        out.push(format!("mcp_servers.{}.args=[{}]", s.name, args.join(",")));
        if !s.env.is_empty() {
            let env: Vec<String> = s
                .env
                .iter()
                .map(|(k, v)| format!("{}={}", toml_str(k), toml_str(v)))
                .collect();
            out.push("-c".to_string());
            out.push(format!("mcp_servers.{}.env={{{}}}", s.name, env.join(",")));
        }
    }
    out
}

/// Add (or refresh) Otto's `otto` MCP server in grok's PROJECT-scoped config
/// (`<workspace>/.grok/config.toml`) — grok reads that plus `~/.grok/config.toml`
/// but NOT the workspace `.mcp.json`, so without this a grok session has no
/// otto tools at all ("Otto MCP is blocked"). Format-preserving upsert
/// (toml_edit): only the `[mcp_servers.otto]` table is managed; everything
/// else in the file survives. Best-effort.
///
/// Shape (verified against `grok mcp add -s project`). Credentials are inherited
/// from the individual Grok process environment, so this shared file stays
/// identity-neutral:
/// ```toml
/// [mcp_servers.otto]
/// command = "<ottod>"
/// args = ["mcp-tools"]
/// enabled = true
/// ```
pub fn enable_otto_tools_grok(workspace_root: &str, server: &OttoToolsServer) -> Result<(), String> {
    use toml_edit::{value, Array, Item, Table};

    let lock = cwd_lock(workspace_root);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (dir, path, mut doc) = read_grok_doc(workspace_root)?;

    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers is not a table")?;
    // Implicit so it renders as [mcp_servers.otto], not a bare [mcp_servers].
    servers.set_implicit(true);

    let mut entry = Table::new();
    entry["command"] = value(server.command.clone());
    let mut args = Array::new();
    for a in &server.args {
        args.push(a.clone());
    }
    entry["args"] = value(args);
    entry["enabled"] = value(true);
    servers.insert(OTTO_TOOLS_KEY, Item::Table(entry));

    write_grok_doc(&dir, &path, &doc)
}

/// Reconcile the user's enabled MCP servers into grok's project config — the
/// `.grok/config.toml` counterpart of [`reconcile_managed_servers`]'s user
/// entries. A top-level `otto_managed_servers` array tracks which
/// `[mcp_servers.*]` tables Otto wrote so a later disable removes them without
/// ever touching tables the user added (the `otto` table stays owned by
/// [`enable_otto_tools_grok`] and is excluded from this marker). Best-effort.
pub fn reconcile_user_servers_grok(
    workspace_root: &str,
    user_servers: &[UserMcpServer],
) -> Result<(), String> {
    use toml_edit::{value, Array, Item, Table};

    let lock = cwd_lock(workspace_root);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (dir, path, mut doc) = read_grok_doc(workspace_root)?;

    let desired: Vec<&UserMcpServer> = user_servers
        .iter()
        .filter(|s| s.name != SERVER_KEY && s.name != OTTO_TOOLS_KEY)
        .collect();
    if desired.is_empty() && !path.exists() {
        return Ok(());
    }

    let prior: Vec<String> = doc
        .get(MANAGED_KEY_TOML)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let desired_names: Vec<String> = desired.iter().map(|s| s.name.clone()).collect();

    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers is not a table")?;
    servers.set_implicit(true);
    for name in &prior {
        if !desired_names.contains(name) && name != OTTO_TOOLS_KEY {
            servers.remove(name);
        }
    }
    for s in &desired {
        let mut entry = Table::new();
        entry["command"] = value(s.command.clone());
        let mut args = Array::new();
        for a in &s.args {
            args.push(a.clone());
        }
        entry["args"] = value(args);
        if !s.env.is_empty() {
            let mut env = toml_edit::InlineTable::new();
            for (k, v) in &s.env {
                env.insert(k, v.clone().into());
            }
            entry["env"] = value(env);
        }
        entry["enabled"] = value(true);
        servers.insert(&s.name, Item::Table(entry));
    }
    if desired_names.is_empty() {
        doc.remove(MANAGED_KEY_TOML);
    } else {
        let mut marker = Array::new();
        for n in &desired_names {
            marker.push(n.clone());
        }
        doc[MANAGED_KEY_TOML] = value(marker);
    }
    write_grok_doc(&dir, &path, &doc)
}

fn read_grok_doc(
    workspace_root: &str,
) -> Result<(PathBuf, PathBuf, toml_edit::DocumentMut), String> {
    let dir = Path::new(workspace_root).join(".grok");
    let path = dir.join("config.toml");
    let doc = match std::fs::read_to_string(&path) {
        Ok(s) => s
            .parse()
            .map_err(|e| format!("parse {}: {e}", path.display()))?,
        Err(_) => toml_edit::DocumentMut::new(),
    };
    Ok((dir, path, doc))
}

fn write_grok_doc(dir: &Path, path: &Path, doc: &toml_edit::DocumentMut) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    atomic_write(path, doc.to_string().as_bytes())
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
    atomic_write(path, body.as_bytes())
}

/// Temp-file + rename in the target's own directory: a reader (an agent CLI
/// launching mid-reconcile) sees either the old or the new file, never a torn
/// half-write. The pid suffix keeps a concurrent daemon (dev + installed) from
/// colliding on the temp name; same-process writers are already serialized by
/// [`cwd_lock`].
fn atomic_write(path: &Path, body: &[u8]) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("bad path {}", path.display()))?;
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {} -> {}: {e}", tmp.display(), path.display())
    })
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

    fn marker(root: &str) -> Vec<String> {
        read_doc(&mcp_path(root))
            .unwrap()
            .get(MANAGED_KEY)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn user(name: &str) -> UserMcpServer {
        UserMcpServer {
            name: name.into(),
            command: "node".into(),
            args: vec!["x.js".into()],
            env: BTreeMap::new(),
        }
    }

    fn otto_tools() -> OttoToolsServer {
        let mut env = BTreeMap::new();
        env.insert("OTTO_MCP_TOKEN".to_string(), "secret-token".to_string());
        OttoToolsServer {
            command: "/usr/local/bin/ottod".into(),
            args: vec!["mcp-tools".into()],
            env,
        }
    }

    /// The full managed set lands in one reconcile: browser + otto + user
    /// entries, the marker records exactly those names, and the cwd-shared
    /// launcher stays identity-neutral (per-session env never hits the file).
    #[test]
    fn reconcile_writes_managed_set_and_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let names = reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                browser: true,
                user_servers: vec![user("myserver")],
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        assert_eq!(names, vec!["otto-browser", "otto", "myserver"]);
        let servers = read_servers(root);
        let otto = servers.get("otto").and_then(|v| v.as_object()).unwrap();
        assert_eq!(otto["command"], json!("/usr/local/bin/ottod"));
        assert_eq!(otto["args"], json!(["mcp-tools"]));
        assert!(otto.get("env").is_none(), "shared config leaked session env: {otto:?}");
        assert!(servers.contains_key("otto-browser"));
        assert!(servers.contains_key("myserver"));
        assert_eq!(marker(root), names);
    }

    /// Disabling a user server (it stops appearing in the enabled set)
    /// propagates as a REMOVAL on the next reconcile — the original
    /// accumulate-forever bug.
    #[test]
    fn disable_propagates_removal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let cfg = ManagedMcpConfig {
            browser: false,
            user_servers: vec![user("jira"), user("linear")],
            otto_tools: Some(otto_tools()),
        };
        reconcile_managed_servers(root, &cfg).unwrap();
        assert!(read_servers(root).contains_key("jira"));

        // User disables "jira": next spawn's enabled set no longer carries it.
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: vec![user("linear")],
                ..cfg
            },
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(!servers.contains_key("jira"), "disabled server survived: {servers:?}");
        assert!(servers.contains_key("linear"));
        assert!(servers.contains_key("otto"));
        assert_eq!(marker(root), vec!["otto", "linear"]);
    }

    /// An empty enabled set removes every managed entry — but never a server
    /// the user hand-added to the file (its name is outside the marker).
    #[test]
    fn empty_set_removes_managed_preserves_hand_added() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                browser: true,
                user_servers: vec![user("jira")],
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        // User hand-adds an entry directly to the file.
        let path = mcp_path(root);
        let mut doc = read_doc(&path).unwrap();
        doc.get_mut("mcpServers")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("mine".into(), json!({ "command": "deno" }));
        write_doc(&path, &doc).unwrap();

        reconcile_managed_servers(root, &ManagedMcpConfig::default()).unwrap();
        let servers = read_servers(root);
        assert_eq!(
            servers.keys().collect::<Vec<_>>(),
            vec!["mine"],
            "only the hand-added entry may survive: {servers:?}"
        );
        assert!(marker(root).is_empty());
        assert!(
            read_doc(&path).unwrap().get(MANAGED_KEY).is_none(),
            "empty marker key should be dropped"
        );
    }

    /// Marker migration: a `.mcp.json` written by the old accumulate-only merge
    /// (no marker) adopts Otto's reserved names plus entries matching the
    /// currently-enabled user set; everything else is treated as hand-added.
    #[test]
    fn missing_marker_adopts_reserved_and_matching_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let path = mcp_path(root);
        let legacy = json!({
            "mcpServers": {
                "otto-browser": { "command": "npx" },
                "otto": { "command": "ottod", "args": ["mcp-tools"] },
                "jira": { "command": "node", "args": ["old.js"] },
                "mine": { "command": "deno" }
            }
        });
        write_doc(&path, legacy.as_object().unwrap()).unwrap();

        // "jira" is still enabled → adopted + refreshed; otto/browser adopted
        // as reserved and then removed (browser off, tools off here).
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: vec![user("jira")],
                ..Default::default()
            },
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(!servers.contains_key("otto"));
        assert!(!servers.contains_key("otto-browser"));
        assert_eq!(servers["jira"]["args"], json!(["x.js"]), "adopted entry refreshed");
        assert!(servers.contains_key("mine"), "hand-added entry adopted by mistake");
        assert_eq!(marker(root), vec!["jira"]);

        // And a later disable of jira now removes it.
        reconcile_managed_servers(root, &ManagedMcpConfig::default()).unwrap();
        let servers = read_servers(root);
        assert_eq!(servers.keys().collect::<Vec<_>>(), vec!["mine"]);
    }

    /// Concurrent spawns sharing one cwd (the PR-review case): the per-cwd lock
    /// makes each reconcile a full RMW, so no writer loses another's update and
    /// a hand-added entry always survives.
    #[test]
    fn concurrent_reconciles_do_not_lose_updates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        // Seed a hand-added entry that must survive every iteration.
        let path = mcp_path(&root);
        write_doc(
            &path,
            json!({ "mcpServers": { "mine": { "command": "deno" } } })
                .as_object()
                .unwrap(),
        )
        .unwrap();

        let cfg = ManagedMcpConfig {
            browser: true,
            user_servers: vec![user("jira")],
            otto_tools: Some(otto_tools()),
        };
        std::thread::scope(|s| {
            for _ in 0..2 {
                let root = root.clone();
                let cfg = cfg.clone();
                s.spawn(move || {
                    for _ in 0..50 {
                        reconcile_managed_servers(&root, &cfg).unwrap();
                    }
                });
            }
        });
        let servers = read_servers(&root);
        for key in ["mine", "otto", "otto-browser", "jira"] {
            assert!(servers.contains_key(key), "lost {key}: {servers:?}");
        }
        assert_eq!(marker(&root), vec!["otto-browser", "otto", "jira"]);
    }

    /// A user server named "otto" or "otto-browser" can never clobber the
    /// managed first-party entries.
    #[test]
    fn user_server_with_reserved_name_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: vec![UserMcpServer {
                    name: "otto".into(),
                    command: "evil".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !read_servers(root).contains_key("otto"),
            "a user server named 'otto' must be dropped by reconcile"
        );
    }

    /// grok project config: upsert is format-preserving and idempotent — a
    /// user's own `[mcp_servers.other]` table and top-level keys survive, and
    /// re-enabling replaces (not duplicates) the managed `otto` table.
    #[test]
    fn grok_project_config_upsert_preserves_user_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let grok_cfg = dir.path().join(".grok").join("config.toml");
        std::fs::create_dir_all(grok_cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &grok_cfg,
            "# my config\nmodel = \"grok-4\"\n\n[mcp_servers.other]\ncommand = \"node\"\nargs = [\"x.js\"]\n",
        )
        .unwrap();

        let server = otto_tools();
        enable_otto_tools_grok(root, &server).unwrap();
        enable_otto_tools_grok(root, &server).unwrap(); // idempotent

        let s = std::fs::read_to_string(&grok_cfg).unwrap();
        assert!(s.contains("# my config"), "{s}");
        assert!(s.contains("model = \"grok-4\""), "{s}");
        assert!(s.contains("[mcp_servers.other]"), "{s}");
        assert_eq!(s.matches("[mcp_servers.otto]").count(), 1, "{s}");
        let doc: toml_edit::DocumentMut = s.parse().unwrap();
        let otto = &doc["mcp_servers"]["otto"];
        assert_eq!(otto["command"].as_str(), Some("/usr/local/bin/ottod"));
        assert_eq!(otto["enabled"].as_bool(), Some(true));
        assert!(otto.get("env").is_none(), "shared Grok config leaked session env: {otto:?}");
    }

    /// grok project config: created from scratch when absent.
    #[test]
    fn grok_project_config_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        enable_otto_tools_grok(
            root,
            &OttoToolsServer {
                command: "ottod".into(),
                args: vec!["mcp-tools".into()],
                env: BTreeMap::new(),
            },
        )
        .unwrap();
        let s = std::fs::read_to_string(dir.path().join(".grok/config.toml")).unwrap();
        assert!(s.contains("[mcp_servers.otto]"), "{s}");
        assert!(s.contains("args = [\"mcp-tools\"]"), "{s}");
    }

    /// grok user-server reconcile: enabled servers land as tables tracked by
    /// the `otto_managed_servers` marker; a disable removes them while the
    /// user's own tables and the managed `otto` table survive.
    #[test]
    fn grok_user_servers_reconcile_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let grok_cfg = dir.path().join(".grok").join("config.toml");
        std::fs::create_dir_all(grok_cfg.parent().unwrap()).unwrap();
        std::fs::write(&grok_cfg, "[mcp_servers.other]\ncommand = \"node\"\n").unwrap();
        enable_otto_tools_grok(root, &otto_tools()).unwrap();

        let mut jira = user("jira");
        jira.env.insert("BASE".into(), "https://x".into());
        reconcile_user_servers_grok(root, &[jira]).unwrap();
        let s = std::fs::read_to_string(&grok_cfg).unwrap();
        let doc: toml_edit::DocumentMut = s.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["jira"]["command"].as_str(), Some("node"));
        assert!(s.contains("otto_managed_servers"), "{s}");

        reconcile_user_servers_grok(root, &[]).unwrap();
        let s = std::fs::read_to_string(&grok_cfg).unwrap();
        assert!(!s.contains("[mcp_servers.jira]"), "{s}");
        assert!(s.contains("[mcp_servers.other]"), "{s}");
        assert!(s.contains("[mcp_servers.otto]"), "{s}");
        assert!(!s.contains("otto_managed_servers"), "{s}");
    }

    #[test]
    fn codex_mcp_inject_args_builds_c_overrides() {
        let args = codex_mcp_inject_args("/usr/local/bin/ottod", "/tmp/otto-mcp/s-1.json");
        assert_eq!(
            args,
            vec![
                "-c".to_string(),
                "mcp_servers.otto.command=\"/usr/local/bin/ottod\"".to_string(),
                "-c".to_string(),
                "mcp_servers.otto.args=[\"mcp-tools\",\"--config\",\"/tmp/otto-mcp/s-1.json\"]"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn codex_mcp_inject_args_escapes_quotes_and_backslashes() {
        let args = codex_mcp_inject_args(r#"/p"q"#, r#"/p\a"#);
        // command value: embedded double-quote is escaped.
        assert_eq!(args[1], r#"mcp_servers.otto.command="/p\"q""#);
        // backslash in the creds path is escaped inside the args array.
        assert!(args[3].contains(r#"/p\\a"#), "got {}", args[3]);
    }

    /// codex user overrides mirror the .mcp.json entries; reserved and
    /// non-bare-key names are skipped rather than failing the spawn.
    #[test]
    fn codex_user_server_args_builds_and_skips() {
        let mut jira = user("jira");
        jira.env.insert("TOKEN".into(), "t\"1".into());
        let mut weird = user("has space");
        weird.command = "x".into();
        let args = codex_user_server_args(&[jira, weird, user("otto")]);
        assert_eq!(
            args,
            vec![
                "-c".to_string(),
                "mcp_servers.jira.command=\"node\"".to_string(),
                "-c".to_string(),
                "mcp_servers.jira.args=[\"x.js\"]".to_string(),
                "-c".to_string(),
                "mcp_servers.jira.env={\"TOKEN\"=\"t\\\"1\"}".to_string(),
            ]
        );
    }
}

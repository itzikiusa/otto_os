//! MCP launcher config for agent sessions. Otto wires browser tools, the
//! user's enabled MCP servers, and its own first-party `otto` tool server into
//! the config each agent CLI actually reads:
//!
//!   - claude:     `<workspace>/.mcp.json`
//!   - agy:        `<workspace>/.agents/mcp_config.json` (never `.mcp.json`)
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
//!   3. fallback: `npx -y @playwright/mcp@<PLAYWRIGHT_MCP_VERSION>` (pinned)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Map, Value};

/// Session launches hold a read guard from native configuration lookup through
/// live insertion. Governance activation holds the write guard through cleanup
/// and policy commit, so no pending launcher can resurrect retired credentials.
pub fn activation_gate() -> &'static tokio::sync::RwLock<()> {
    static GATE: OnceLock<tokio::sync::RwLock<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::RwLock::new(()))
}

/// The launcher key for Otto's browser MCP server (every provider's config).
pub(crate) const SERVER_KEY: &str = "otto-browser";

/// The `.mcp.json` key for Otto's own first-party read-only tool server
/// (Task B2b). Opt-in per workspace (`otto_mcp_enabled`); never auto-injected.
pub(crate) const OTTO_TOOLS_KEY: &str = "otto";

/// Top-level `.mcp.json` marker listing the `mcpServers` names Otto manages.
/// Reconciliation removes stale names from this set and never touches names
/// outside it, so user hand-edits survive while Otto's own entries track the
/// enabled state exactly.
const MANAGED_KEY: &str = "ottoManagedServers";

/// The grok `config.toml` counterpart of [`MANAGED_KEY`] (snake_case to match
/// the surrounding TOML keys).
const MANAGED_KEY_TOML: &str = "otto_managed_servers";

/// The exact `@playwright/mcp` release the fallback launches. Pinned — never
/// `@latest` — so every agent spawn runs a known package instead of whatever
/// the registry serves that minute (a supply-chain hop on every session start).
/// Bump deliberately, after reviewing the release.
pub const PLAYWRIGHT_MCP_VERSION: &str = "0.0.82";

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
        vec![
            "-y".into(),
            format!("@playwright/mcp@{PLAYWRIGHT_MCP_VERSION}"),
        ],
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

/// agy (Antigravity CLI)'s workspace MCP config. agy ignores `.mcp.json`; it
/// discovers `.agents/` walking up from its cwd to the repo root and loads that
/// `mcp_config.json` — same `mcpServers` shape, unknown top-level keys (our
/// marker) tolerated, and the agy process env is inherited by stdio children.
fn agy_mcp_path(workspace_root: &str) -> PathBuf {
    Path::new(workspace_root)
        .join(".agents")
        .join("mcp_config.json")
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

/// Resolves a workspace's *enabled* user-configured MCP servers just before an
/// agent spawn. Fallible on purpose: "the lookup failed" must stay distinct
/// from "nothing is enabled" — the latter removes every managed user entry
/// from the shared launcher files, so a transient DB/Keychain fault mapped to
/// an empty list would strip the user's servers for every session in that cwd.
/// Implementations are sync (the spawn path runs them on the blocking pool).
pub trait McpServerSource: Send + Sync {
    fn enabled_servers(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<otto_core::hooks::McpServerSpec>, String>;
}

/// Everything Otto manages in a workspace's `.mcp.json`, resolved once per
/// spawn. `None`/`false`/empty means "not enabled" — reconciliation then
/// REMOVES the corresponding managed entries, which is how a disable or delete
/// propagates to the file. The one exception is `user_servers: None` ("the
/// enabled set is unknown"), which preserves the managed user entries as-is.
#[derive(Debug, Clone)]
pub struct ManagedMcpConfig {
    /// Session opted into browser tools (`meta.browser == true`).
    pub browser: bool,
    /// The workspace's *enabled* user-configured servers. Never auto-enabled —
    /// callers pass only the rows the user flipped on. `None` means the lookup
    /// failed: the previously managed user entries (and their marker names)
    /// are kept untouched while Otto's own entries are still reconciled.
    pub user_servers: Option<Vec<UserMcpServer>>,
    /// Otto's first-party tool server, when the workspace has it enabled.
    /// Per-session env is deliberately NOT written to the shared file: several
    /// sessions can share this cwd, and persisting one session's token here
    /// would make the last spawner's capabilities win for every later resume.
    pub otto_tools: Option<OttoToolsServer>,
}

/// Defaults to "nothing enabled" (`user_servers: Some(vec![])`), not to the
/// preserve-on-unknown `None`: removal is the normal reconcile, preservation
/// must be asked for explicitly.
impl Default for ManagedMcpConfig {
    fn default() -> Self {
        Self {
            browser: false,
            user_servers: Some(Vec::new()),
            otto_tools: None,
        }
    }
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
    reconcile_json_launcher(workspace_root, &mcp_path(workspace_root), cfg)
}

/// The agy counterpart of [`reconcile_managed_servers`]: the same managed set
/// and marker semantics, written to `<workspace>/.agents/mcp_config.json`
/// (agy never reads `.mcp.json`, so without this an agy session gets no
/// `otto`, browser or user servers at all).
pub fn reconcile_managed_servers_agy(
    workspace_root: &str,
    cfg: &ManagedMcpConfig,
) -> Result<Vec<String>, String> {
    reconcile_json_launcher(workspace_root, &agy_mcp_path(workspace_root), cfg)
}

/// One `mcpServers`-shaped JSON launcher file (`.mcp.json` or agy's
/// `.agents/mcp_config.json`) reconciled against `cfg` under the cwd lock.
fn reconcile_json_launcher(
    workspace_root: &str,
    path: &Path,
    cfg: &ManagedMcpConfig,
) -> Result<Vec<String>, String> {
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
    for s in cfg.user_servers.iter().flatten() {
        // Reserved for Otto's own entries — a user server named "otto-browser"
        // or "otto" can't clobber them.
        if s.name == SERVER_KEY || s.name == OTTO_TOOLS_KEY {
            continue;
        }
        // The server's env (Keychain secrets included) never lands in the
        // work tree: it goes to a private env file the entry execs through.
        let Some((command, args)) = launch_via_env_file(workspace_root, s) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("command".into(), Value::String(command));
        entry.insert(
            "args".into(),
            Value::Array(args.into_iter().map(Value::String).collect()),
        );
        desired.push((s.name.clone(), Value::Object(entry)));
    }
    let desired_names: Vec<String> = desired.iter().map(|(n, _)| n.clone()).collect();

    // Nothing to manage and nothing on disk: don't create a file just to hold
    // an empty map.
    if desired.is_empty() && !path.exists() {
        return Ok(Vec::new());
    }

    let mut doc = read_doc(path)?;

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
                .filter(|n| n == SERVER_KEY || n == OTTO_TOOLS_KEY || desired_names.contains(n))
                .collect()
        }
    };

    // Unknown enabled set (lookup failed): every previously managed user entry
    // stays exactly as it is and keeps its marker slot, so the next healthy
    // spawn can still reconcile it. Only Otto's reserved entries are managed.
    let mut managed_names = desired_names.clone();
    if cfg.user_servers.is_none() {
        for name in &prior {
            if name != SERVER_KEY && name != OTTO_TOOLS_KEY && !managed_names.contains(name) {
                managed_names.push(name.clone());
            }
        }
    }

    let servers = doc
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers is not an object")?;
    // Remove what Otto wrote before and no longer wants; upsert the rest.
    for name in &prior {
        if !managed_names.contains(name) {
            servers.remove(name);
            // Its launcher env file holds Keychain-resolved values; a server
            // that is gone (deleted, disabled, renamed) must not leave them on
            // disk. Names preserved for a failed lookup stay in
            // `managed_names`, so their env files are kept.
            if name != SERVER_KEY && name != OTTO_TOOLS_KEY {
                remove_launcher_env(workspace_root, name);
            }
        }
    }
    // A preserved name whose entry is gone (hand-deleted) is not managed any more.
    managed_names.retain(|n| desired_names.contains(n) || servers.contains_key(n));
    for (name, entry) in desired {
        servers.insert(name, entry);
    }
    let desired_names = managed_names;
    if desired_names.is_empty() {
        doc.remove(MANAGED_KEY);
    } else {
        doc.insert(
            MANAGED_KEY.to_string(),
            Value::Array(desired_names.iter().cloned().map(Value::String).collect()),
        );
    }
    write_doc(path, &doc)?;
    exclude_launcher_artifacts(workspace_root);
    Ok(desired_names)
}

/// The command/args a shared launcher file (JSON or grok TOML) carries for a
/// user server. With env, that is `/bin/sh -c ENV_EXEC_SCRIPT <env file>
/// <command> <args…>` — the codex mechanism — with the env in a 0600 file
/// under `<tmp>/otto-mcp`, so no secret value is ever written under the cwd
/// (where an agent's `git add -A`, or a tracked team `.mcp.json`, would carry
/// it into a commit). `None` = not deliverable (an env key that isn't a shell
/// name, or the env file couldn't be written): skipped with a warning, like
/// codex, rather than falling back to inline secrets.
fn launch_via_env_file(workspace_root: &str, s: &UserMcpServer) -> Option<(String, Vec<String>)> {
    if s.env.is_empty() {
        // The env (and with it any secret) was removed since the last spawn:
        // the old file is no longer referenced, so it must not linger.
        remove_launcher_env(workspace_root, &s.name);
        return Some((s.command.clone(), s.args.clone()));
    }
    if let Some(bad) = s.env.keys().find(|k| !is_shell_name(k)) {
        tracing::warn!(
            server = %s.name,
            key = %bad,
            "MCP launcher: env key is not a shell variable name; server not attached"
        );
        return None;
    }
    let path = launcher_env_path(workspace_root, &s.name);
    if let Err(e) = write_env_file_atomic(&path, &s.env) {
        tracing::warn!(server = %s.name, "MCP launcher: write env file failed: {e}");
        return None;
    }
    let mut args = vec![
        "-c".to_string(),
        ENV_EXEC_SCRIPT.to_string(),
        path.to_string_lossy().into_owned(),
        s.command.clone(),
    ];
    args.extend(s.args.iter().cloned());
    Some(("/bin/sh".to_string(), args))
}

/// `<tmp>/otto-mcp/launch-<cwd hash>.<server>-<name hash>.env`: one file per
/// (cwd, server), because the launcher it backs is shared by every session in
/// that cwd and rewritten (same path) on each spawn. The `launch-` prefix keeps
/// it clear of the per-session `<session_id>.*` files the session cleanup
/// removes, so its lifetime is owned here instead: reconcile deletes it when
/// the server leaves the launcher or loses its env, and
/// [`sweep_launcher_env_files`] removes every cwd's copy when the server is
/// deleted or disabled. A user server name is arbitrary text, so it is reduced
/// to a safe segment.
fn launcher_env_path(workspace_root: &str, server: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workspace_root.hash(&mut h);
    std::env::temp_dir().join("otto-mcp").join(format!(
        "launch-{:016x}.{}",
        h.finish(),
        launcher_env_suffix(server)
    ))
}

/// `<safe name>-<name hash>.env`, the server-specific tail of every
/// [`launcher_env_path`] for `server`, whatever the cwd.
fn launcher_env_suffix(server: &str) -> String {
    use std::hash::{Hash, Hasher};
    let safe: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let mut nh = std::collections::hash_map::DefaultHasher::new();
    server.hash(&mut nh);
    format!("{safe}-{:08x}.env", nh.finish() as u32)
}

/// Delete one cwd's launcher env file for `server` (absent is fine; any other
/// failure is logged — the caller has already dropped the entry using it).
fn remove_launcher_env(workspace_root: &str, server: &str) {
    let path = launcher_env_path(workspace_root, server);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(server, "MCP launcher: remove stale env file failed: {e}"),
    }
}

/// Remove `server`'s launcher env file in EVERY cwd (`<tmp>/otto-mcp/launch-*
/// .<safe name>-<name hash>.env`): called when a server is deleted or disabled,
/// so its Keychain-resolved values don't outlive it on disk in cwds no session
/// reconciles again. A same-named server in another workspace loses its file
/// too; that workspace's next spawn rewrites it. Returns how many were removed.
/// Blocking: callers run it on the blocking pool (`spawn_blocking`).
#[allow(clippy::disallowed_methods)] // sync helper: callers use spawn_blocking
pub fn sweep_launcher_env_files(server: &str) -> usize {
    let suffix = format!(".{}", launcher_env_suffix(server));
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir().join("otto-mcp")) else {
        return 0;
    };
    let mut removed = 0;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("launch-") && name.ends_with(&suffix) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(server, "MCP launcher: sweep env file failed: {e}"),
            }
        }
    }
    removed
}

/// [`write_env_file`] to a temp sibling, then rename: a CLI starting from
/// another session in the same cwd reads the old or the new env, never a
/// half-written one.
fn write_env_file_atomic(
    path: &Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let tmp = path.with_extension(format!("env.{}.tmp", std::process::id()));
    write_env_file(&tmp, env)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {} -> {}: {e}", tmp.display(), path.display())
    })
}

/// Keep the launcher files out of git: append the root-anchored
/// [`otto_core::paths::LAUNCHER_ARTIFACTS`] (anchored at `workspace_root`'s
/// place in its work tree) to `<git-common-dir>/info/exclude`, idempotently.
/// The common dir's exclude applies to every linked worktree, and nothing is
/// added to a tracked file. Best-effort (not a repo, read-only `.git`: logged).
///
/// Sync on purpose — every reconcile runs synchronously under [`cwd_lock`] —
/// so the git dir is resolved from the `.git` dir/file (and a worktree's
/// `commondir`) the way `git rev-parse --git-common-dir` does, without
/// spawning git inside a possibly untrusted repo.
fn exclude_launcher_artifacts(workspace_root: &str) {
    let Some((exclude, prefix)) = git_exclude_target(Path::new(workspace_root)) else {
        return;
    };
    let wanted: Vec<String> = otto_core::paths::LAUNCHER_ARTIFACTS
        .iter()
        .map(|a| {
            if prefix.is_empty() {
                format!("/{a}")
            } else {
                format!("/{prefix}/{a}")
            }
        })
        .collect();
    if let Err(e) = append_exclude_lines(&exclude, &wanted) {
        tracing::warn!(path = %exclude.display(), "MCP launcher: update git exclude: {e}");
    }
}

/// `(<git-common-dir>/info/exclude, workspace_root relative to its work-tree
/// root)`; `None` outside a git work tree.
fn git_exclude_target(cwd: &Path) -> Option<(PathBuf, String)> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    for top in cwd.ancestors() {
        let dotgit = top.join(".git");
        let Ok(meta) = std::fs::metadata(&dotgit) else {
            continue;
        };
        let gitdir = if meta.is_dir() {
            dotgit
        } else {
            // Linked worktree / submodule: `.git` is a `gitdir: <path>` file.
            let text = std::fs::read_to_string(&dotgit).ok()?;
            let p = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let p = Path::new(p);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                top.join(p)
            }
        };
        let common = match std::fs::read_to_string(gitdir.join("commondir")) {
            Ok(c) => {
                let c = Path::new(c.trim());
                if c.is_absolute() {
                    c.to_path_buf()
                } else {
                    gitdir.join(c)
                }
            }
            Err(_) => gitdir,
        };
        let prefix = cwd
            .strip_prefix(top)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        return Some((common.join("info").join("exclude"), prefix));
    }
    None
}

/// Append each of `lines` not already present (exact line match) to the
/// exclude file, creating `info/` and the file as needed.
fn append_exclude_lines(exclude: &Path, lines: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let existing = match std::fs::read_to_string(exclude) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let have: std::collections::HashSet<&str> = existing.lines().map(str::trim).collect();
    let missing: Vec<&String> = lines
        .iter()
        .filter(|l| !have.contains(l.as_str()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(parent) = exclude.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("# Otto MCP launchers (runtime state; never commit)\n");
    for l in missing {
        body.push_str(l);
        body.push('\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(exclude)?
        .write_all(body.as_bytes())
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

/// The Codex `-c` overrides that attach Otto's browser MCP server — the codex
/// counterpart of the `otto-browser` entry the JSON launchers get. Emitted only
/// for a session that opted in (`meta.browser`); per-spawn, so nothing to undo.
pub fn codex_browser_args() -> Vec<String> {
    let (command, args) = browser_command();
    let args: Vec<String> = args.iter().map(|a| toml_str(a)).collect();
    vec![
        "-c".to_string(),
        format!("mcp_servers.{SERVER_KEY}.command={}", toml_str(&command)),
        "-c".to_string(),
        format!("mcp_servers.{SERVER_KEY}.args=[{}]", args.join(",")),
    ]
}

/// What [`codex_user_server_args`] attached: the `-c` overrides plus the names
/// of the servers they actually carry (the session's MCP snapshot must list
/// only what the agent really gets, never a server codex silently skipped).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CodexUserServers {
    pub args: Vec<String>,
    pub delivered: Vec<String>,
}

/// `sh -c` program that loads a server's env file and execs the real server:
/// `$0` is the env file, `"$@"` the server's command and args. `set -a` exports
/// every assignment the file makes; a missing/unreadable file is a fatal `.`
/// error, so the server never starts without its configured environment.
const ENV_EXEC_SCRIPT: &str = "set -a; . \"$0\"; set +a; exec \"$@\"";

/// A POSIX shell variable name — the only env keys an `sh`-sourced env file can
/// assign.
fn is_shell_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The Codex `-c` overrides that attach the user's enabled MCP servers to a
/// Codex session — the codex counterpart of the `.mcp.json` user entries, using
/// the same per-spawn mechanism as [`codex_mcp_inject_args`]. Per-spawn, so
/// disables propagate for free (a disabled server simply isn't emitted).
///
/// A server's `env` (Keychain-resolved secrets included) never touches argv:
/// `env_file` writes it to a per-session 0600 file and the server is launched
/// as `/bin/sh -c ENV_EXEC_SCRIPT <file> <command> <args…>`, the same
/// "secrets live in a private file" rule as the `otto` creds file.
///
/// Servers that can't be delivered — a name that isn't a bare TOML key, an env
/// key that isn't a shell name, an env file that couldn't be written — are
/// skipped with a warning rather than failing the spawn, and are left out of
/// [`CodexUserServers::delivered`].
pub fn codex_user_server_args(
    servers: &[UserMcpServer],
    mut env_file: impl FnMut(&UserMcpServer) -> Result<PathBuf, String>,
) -> CodexUserServers {
    let mut out = CodexUserServers::default();
    for s in servers {
        if s.name == SERVER_KEY || s.name == OTTO_TOOLS_KEY {
            continue; // reserved for Otto's managed entries
        }
        if !is_bare_toml_key(&s.name) {
            tracing::warn!(
                server = %s.name,
                "codex MCP: server name is not a bare TOML key (letters, digits, '_' and '-' only); \
                 not attached to this codex session"
            );
            continue;
        }
        let (command, args) = if s.env.is_empty() {
            (s.command.clone(), s.args.clone())
        } else {
            if let Some(bad) = s.env.keys().find(|k| !is_shell_name(k)) {
                tracing::warn!(
                    server = %s.name,
                    key = %bad,
                    "codex MCP: env key is not a shell variable name; not attached to this codex session"
                );
                continue;
            }
            let path = match env_file(s) {
                Ok(path) => path,
                Err(e) => {
                    tracing::warn!(server = %s.name, "codex MCP: write env file failed: {e}");
                    continue;
                }
            };
            let mut args = vec![
                "-c".to_string(),
                ENV_EXEC_SCRIPT.to_string(),
                path.to_string_lossy().into_owned(),
                s.command.clone(),
            ];
            args.extend(s.args.iter().cloned());
            ("/bin/sh".to_string(), args)
        };
        out.args.push("-c".to_string());
        out.args.push(format!(
            "mcp_servers.{}.command={}",
            s.name,
            toml_str(&command)
        ));
        out.args.push("-c".to_string());
        let args: Vec<String> = args.iter().map(|a| toml_str(a)).collect();
        out.args
            .push(format!("mcp_servers.{}.args=[{}]", s.name, args.join(",")));
        out.delivered.push(s.name.clone());
    }
    out
}

/// Single-quote `v` for a POSIX shell assignment (`'` becomes `'\''`).
fn shell_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// Write a server's env as `KEY='value'` lines to `path`, owner-only (0600)
/// before any byte lands — it carries Keychain-resolved secrets. Keys must
/// already be shell names ([`codex_user_server_args`] checks).
pub fn write_env_file(
    path: &Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut body = String::new();
    for (k, v) in env {
        body.push_str(k);
        body.push('=');
        body.push_str(&shell_quote(v));
        body.push('\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    // `mode` only applies on create: tighten a pre-existing file too, before
    // the secrets are written into it.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod {}: {e}", path.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
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
pub fn enable_otto_tools_grok(
    workspace_root: &str,
    server: &OttoToolsServer,
) -> Result<(), String> {
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
            remove_launcher_env(workspace_root, name);
        }
    }
    for s in &desired {
        // Same rule as the JSON launchers: env goes to a private file, never
        // into `.grok/config.toml`. An undeliverable server is dropped.
        let Some((command, argv)) = launch_via_env_file(workspace_root, s) else {
            servers.remove(&s.name);
            continue;
        };
        let mut entry = Table::new();
        entry["command"] = value(command);
        let mut args = Array::new();
        for a in argv {
            args.push(a);
        }
        entry["args"] = value(args);
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
    write_grok_doc(&dir, &path, &doc)?;
    exclude_launcher_artifacts(workspace_root);
    Ok(())
}

fn read_grok_doc(
    workspace_root: &str,
) -> Result<(PathBuf, PathBuf, toml_edit::DocumentMut), String> {
    let dir = Path::new(workspace_root).join(".grok");
    let path = dir.join("config.toml");
    // Same rule as `read_doc`: only a missing/blank file starts fresh.
    let doc = match read_launcher(&path)? {
        Some(s) => s
            .parse()
            .map_err(|e| format!("parse {}: {e}", path.display()))?,
        None => toml_edit::DocumentMut::new(),
    };
    Ok((dir, path, doc))
}

fn write_grok_doc(dir: &Path, path: &Path, doc: &toml_edit::DocumentMut) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    atomic_write(path, doc.to_string().as_bytes())
}

/// Read a launcher file for a read→reconcile→write. Only a missing or blank
/// file reads as empty: any other read failure (EACCES, non-UTF-8 such as a
/// UTF-16 save, an I/O error) is propagated exactly like a parse error, so the
/// reconcile is skipped and logged instead of atomically writing a fresh
/// document over the user's hand-added servers.
fn read_doc(path: &Path) -> Result<Map<String, Value>, String> {
    match read_launcher(path)? {
        Some(s) => serde_json::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display())),
        None => Ok(Map::new()),
    }
}

/// The launcher file's text, `None` when it is absent or whitespace-only.
fn read_launcher(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(None),
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {}: {e}", path.display())),
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
    // Owner-only (0600) from the first byte, like the codex env/creds files:
    // a launcher names every enabled server and the local `ottod` path.
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.write_all(body)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {} -> {}: {e}", tmp.display(), path.display())
    })
}

/// Retire a formerly direct server without disturbing other launcher entries.
/// A preview is read-only. Untracked entries require an explicit manual cleanup
/// because Otto cannot claim ownership of somebody else's launcher config.
/// Every launcher (`.mcp.json`, agy's `.agents/mcp_config.json`, grok's
/// `.grok/config.toml`) is checked BEFORE any is written, so a refusal in one
/// never leaves the server half-retired in another.
pub fn retire_user_server(workspace_root: &str, name: &str, apply: bool) -> Result<(), String> {
    let lock = cwd_lock(workspace_root);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let untracked = |path: &Path| {
        format!(
            "remove the untracked direct MCP entry '{name}' from {} before enforcement",
            path.display()
        )
    };
    let mut json_docs: Vec<(PathBuf, Map<String, Value>)> = Vec::new();
    for path in [mcp_path(workspace_root), agy_mcp_path(workspace_root)] {
        if !path.exists() {
            continue;
        }
        let doc = read_doc(&path)?;
        if doc
            .get("mcpServers")
            .and_then(Value::as_object)
            .is_some_and(|s| s.contains_key(name))
        {
            let tracked = doc
                .get(MANAGED_KEY)
                .and_then(Value::as_array)
                .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(name)));
            if !tracked {
                return Err(untracked(&path));
            }
            json_docs.push((path, doc));
        }
    }
    let (dir, grok_path, mut grok_doc) = read_grok_doc(workspace_root)?;
    let grok_has = grok_path.exists()
        && grok_doc
            .get("mcp_servers")
            .and_then(toml_edit::Item::as_table)
            .is_some_and(|s| s.contains_key(name));
    if grok_has {
        let tracked = grok_doc
            .get(MANAGED_KEY_TOML)
            .and_then(toml_edit::Item::as_array)
            .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(name)));
        if !tracked {
            return Err(untracked(&grok_path));
        }
    }
    if !apply {
        return Ok(());
    }

    for (path, mut doc) in json_docs {
        if let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) {
            servers.remove(name);
        }
        if let Some(names) = doc.get_mut(MANAGED_KEY).and_then(Value::as_array_mut) {
            names.retain(|n| n.as_str() != Some(name));
        }
        write_doc(&path, &doc)?;
    }
    if grok_has {
        if let Some(servers) = grok_doc
            .get_mut("mcp_servers")
            .and_then(toml_edit::Item::as_table_mut)
        {
            servers.remove(name);
        }
        if let Some(names) = grok_doc
            .get_mut(MANAGED_KEY_TOML)
            .and_then(toml_edit::Item::as_array_mut)
        {
            names.retain(|n| n.as_str() != Some(name));
        }
        write_grok_doc(&dir, &grok_path, &grok_doc)?;
    }
    Ok(())
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
                user_servers: Some(vec![user("myserver")]),
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        assert_eq!(names, vec!["otto-browser", "otto", "myserver"]);
        let servers = read_servers(root);
        let otto = servers.get("otto").and_then(|v| v.as_object()).unwrap();
        assert_eq!(otto["command"], json!("/usr/local/bin/ottod"));
        assert_eq!(otto["args"], json!(["mcp-tools"]));
        assert!(
            otto.get("env").is_none(),
            "shared config leaked session env: {otto:?}"
        );
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
            user_servers: Some(vec![user("jira"), user("linear")]),
            otto_tools: Some(otto_tools()),
        };
        reconcile_managed_servers(root, &cfg).unwrap();
        assert!(read_servers(root).contains_key("jira"));

        // User disables "jira": next spawn's enabled set no longer carries it.
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: Some(vec![user("linear")]),
                ..cfg
            },
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(
            !servers.contains_key("jira"),
            "disabled server survived: {servers:?}"
        );
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
                user_servers: Some(vec![user("jira")]),
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
                user_servers: Some(vec![user("jira")]),
                ..Default::default()
            },
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(!servers.contains_key("otto"));
        assert!(!servers.contains_key("otto-browser"));
        assert_eq!(
            servers["jira"]["args"],
            json!(["x.js"]),
            "adopted entry refreshed"
        );
        assert!(
            servers.contains_key("mine"),
            "hand-added entry adopted by mistake"
        );
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
            user_servers: Some(vec![user("jira")]),
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
                user_servers: Some(vec![UserMcpServer {
                    name: "otto".into(),
                    command: "evil".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                }]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !read_servers(root).contains_key("otto"),
            "a user server named 'otto' must be dropped by reconcile"
        );
    }

    /// agy ignores `.mcp.json`: its reconcile lands the same managed set (and
    /// marker) in `.agents/mcp_config.json`, leaves `.mcp.json` alone, keeps
    /// hand-added agy entries, and propagates a disable as a removal.
    #[test]
    fn agy_config_reconciles_into_agents_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let path = agy_mcp_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"mine":{"command":"x"}}}"#).unwrap();

        let names = reconcile_managed_servers_agy(
            root,
            &ManagedMcpConfig {
                browser: false,
                user_servers: Some(vec![user("jira")]),
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        assert_eq!(names, vec!["otto", "jira"]);
        assert!(!mcp_path(root).exists(), "agy reconcile wrote .mcp.json");
        let doc = read_doc(&path).unwrap();
        let servers = doc["mcpServers"].as_object().unwrap();
        assert_eq!(servers["otto"]["args"], json!(["mcp-tools"]));
        assert!(servers["otto"].get("env").is_none(), "session env leaked");
        assert!(servers.contains_key("jira") && servers.contains_key("mine"));
        assert_eq!(doc[MANAGED_KEY], json!(["otto", "jira"]));

        reconcile_managed_servers_agy(
            root,
            &ManagedMcpConfig {
                otto_tools: Some(otto_tools()),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = read_doc(&path).unwrap();
        let servers = doc["mcpServers"].as_object().unwrap();
        assert!(!servers.contains_key("jira"), "disabled server survived");
        assert!(servers.contains_key("otto") && servers.contains_key("mine"));
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
        assert!(
            otto.get("env").is_none(),
            "shared Grok config leaked session env: {otto:?}"
        );
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

    /// Every file under `dir`, recursively (`.git` included).
    #[allow(clippy::disallowed_methods)] // sync test helper over a tempdir
    fn all_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                all_files(&p, out);
            } else {
                out.push(p);
            }
        }
    }

    /// A reconcile with a secret-bearing user server (claude, agy and grok
    /// launchers) never writes the value anywhere under the cwd: the launchers
    /// exec through a 0600 env file in `<tmp>/otto-mcp`, are themselves 0600,
    /// and the repo's `info/exclude` keeps all three out of `git add -A`.
    #[test]
    fn secret_env_never_lands_under_the_cwd_and_launchers_are_git_excluded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("info")).unwrap();
        std::fs::write(
            dir.path().join(".git").join("info").join("exclude"),
            "# user rules\n*.log",
        )
        .unwrap();
        let ws = dir.path().join("svc");
        std::fs::create_dir_all(&ws).unwrap();
        let root = ws.to_str().unwrap();
        let secret = "sk-live-do-not-commit-1234";
        let mut jira = user("jira");
        jira.env.insert("API_TOKEN".into(), secret.into());
        let cfg = ManagedMcpConfig {
            user_servers: Some(vec![jira.clone()]),
            ..Default::default()
        };
        for _ in 0..2 {
            reconcile_managed_servers(root, &cfg).unwrap();
            reconcile_managed_servers_agy(root, &cfg).unwrap();
            reconcile_user_servers_grok(root, std::slice::from_ref(&jira)).unwrap();
        }

        let mut files = Vec::new();
        all_files(dir.path(), &mut files);
        for f in &files {
            let body = std::fs::read(f).unwrap();
            assert!(
                !String::from_utf8_lossy(&body).contains(secret),
                "secret written under the cwd: {}",
                f.display()
            );
        }
        let servers = read_servers(root);
        assert_eq!(servers["jira"]["command"], "/bin/sh");
        assert!(servers["jira"].get("env").is_none());
        let env_file = PathBuf::from(servers["jira"]["args"][2].as_str().unwrap());
        assert!(env_file.starts_with(std::env::temp_dir().join("otto-mcp")));
        assert!(std::fs::read_to_string(&env_file).unwrap().contains(secret));
        for f in [&env_file, &mcp_path(root), &agy_mcp_path(root)] {
            let mode = std::fs::metadata(f).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} must be owner-only", f.display());
        }

        // Root-anchored at the cwd's place in the repo, added once, user rules kept.
        let exclude =
            std::fs::read_to_string(dir.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.starts_with("# user rules\n*.log\n"), "{exclude}");
        for a in otto_core::paths::LAUNCHER_ARTIFACTS {
            let line = format!("/svc/{a}");
            assert_eq!(
                exclude.lines().filter(|l| *l == line).count(),
                1,
                "{line} in {exclude}"
            );
        }
        let _ = std::fs::remove_file(env_file);
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
        // Env-carrying server: exec'd through its private env file.
        assert_eq!(
            doc["mcp_servers"]["jira"]["command"].as_str(),
            Some("/bin/sh")
        );
        assert!(s.contains("\"node\""), "{s}");
        assert!(!s.contains("https://x"), "env value in the work tree: {s}");
        assert!(doc["mcp_servers"]["jira"].get("env").is_none(), "{s}");
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
    /// non-bare-key names are skipped rather than failing the spawn, and only
    /// the servers actually emitted are reported as delivered.
    #[test]
    fn codex_user_server_args_builds_and_skips() {
        let mut weird = user("has space");
        weird.command = "x".into();
        let out = codex_user_server_args(&[user("jira"), weird, user("otto")], |_| {
            panic!("no env, no env file")
        });
        assert_eq!(
            out.args,
            vec![
                "-c".to_string(),
                "mcp_servers.jira.command=\"node\"".to_string(),
                "-c".to_string(),
                "mcp_servers.jira.args=[\"x.js\"]".to_string(),
            ]
        );
        assert_eq!(out.delivered, vec!["jira"]);
    }

    /// A user server's env (Keychain secrets included) never reaches codex's
    /// argv: it lands in a 0600 env file the server is exec'd through, and the
    /// file round-trips every value — quotes, spaces, `$`, newlines — exactly.
    #[test]
    #[allow(clippy::disallowed_methods)] // sync test: runs the wrapper shell to completion
    fn codex_user_server_env_stays_off_argv() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let secret = "s3cr'et $HOME \"q\"\nline2";
        let mut jira = user("jira");
        jira.env.insert("API_TOKEN".into(), secret.into());
        jira.env.insert("BASE".into(), "https://x".into());
        let mut bad_key = user("badkey");
        bad_key.env.insert("NOT-A-NAME".into(), "v".into());
        let env_path = dir.path().join("jira.env");
        let out = codex_user_server_args(&[jira, bad_key], |s| {
            assert_eq!(s.name, "jira", "env file only for deliverable servers");
            write_env_file(&env_path, &s.env)?;
            Ok(env_path.clone())
        });
        assert_eq!(out.delivered, vec!["jira"]);
        for arg in &out.args {
            assert!(!arg.contains("s3cr"), "secret on argv: {arg}");
            assert!(!arg.contains("https://x"), "env value on argv: {arg}");
            assert!(!arg.contains(".env="), "literal env override: {arg}");
        }
        assert_eq!(out.args[1], "mcp_servers.jira.command=\"/bin/sh\"");
        assert!(out.args[3].contains(&env_path.to_string_lossy().into_owned()));
        let mode = std::fs::metadata(&env_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "env file must be owner-only");

        // Run the exact wrapper codex would: the server sees the env verbatim.
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                ENV_EXEC_SCRIPT,
                &env_path.to_string_lossy(),
                "/usr/bin/printenv",
                "API_TOKEN",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{secret}\n")
        );
    }

    /// The browser toggle reaches codex as `-c mcp_servers.otto-browser.*`
    /// (codex never reads `.mcp.json`).
    #[test]
    fn codex_browser_args_attach_otto_browser() {
        let args = codex_browser_args();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "-c");
        assert!(
            args[1].starts_with("mcp_servers.otto-browser.command=\""),
            "{args:?}"
        );
        assert!(
            args[3].starts_with("mcp_servers.otto-browser.args=["),
            "{args:?}"
        );
    }

    /// A failed server lookup (`user_servers: None`) still reconciles Otto's own
    /// entries but keeps every previously managed user entry, and its marker
    /// slot, so the next healthy spawn can reconcile it normally.
    #[test]
    fn unknown_user_set_preserves_managed_user_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                browser: true,
                user_servers: Some(vec![user("jira"), user("linear")]),
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        let names = reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                browser: false,
                user_servers: None,
                otto_tools: Some(otto_tools()),
            },
        )
        .unwrap();
        let servers = read_servers(root);
        assert!(servers.contains_key("jira") && servers.contains_key("linear"));
        assert!(
            !servers.contains_key("otto-browser"),
            "Otto's own entries still reconcile"
        );
        assert_eq!(names, vec!["otto", "jira", "linear"]);
        assert_eq!(marker(root), names);

        // The next healthy spawn with only "linear" enabled removes "jira".
        reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: Some(vec![user("linear")]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!read_servers(root).contains_key("jira"));
    }

    /// Only a missing or blank launcher reads as empty: an unreadable one
    /// (here: not UTF-8, e.g. a UTF-16 save) fails the reconcile and leaves the
    /// user's file byte-identical instead of being replaced by a fresh doc.
    #[test]
    fn unreadable_launcher_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let path = mcp_path(root);
        let utf16: Vec<u8> = [0xFF, 0xFE, b'{', 0, b'}', 0].to_vec();
        std::fs::write(&path, &utf16).unwrap();
        let err = reconcile_managed_servers(
            root,
            &ManagedMcpConfig {
                user_servers: Some(vec![user("jira")]),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.starts_with("read "), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), utf16);

        let grok = dir.path().join(".grok").join("config.toml");
        std::fs::create_dir_all(grok.parent().unwrap()).unwrap();
        std::fs::write(&grok, &utf16).unwrap();
        assert!(reconcile_user_servers_grok(root, &[user("jira")]).is_err());
        assert_eq!(std::fs::read(&grok).unwrap(), utf16);

        // Blank still reads as empty.
        std::fs::write(&path, "  \n").unwrap();
        assert!(read_doc(&path).unwrap().is_empty());
    }

    fn retirement_config() -> ManagedMcpConfig {
        let named = |name: &str, command: &str| UserMcpServer {
            name: name.into(),
            command: command.into(),
            args: vec![],
            env: Default::default(),
        };
        ManagedMcpConfig {
            user_servers: Some(vec![named("private", "server"), named("keep", "other")]),
            ..Default::default()
        }
    }

    /// Retirement walks every JSON launcher (`.mcp.json` AND agy's
    /// `.agents/mcp_config.json`): a preview leaves both byte-identical, apply
    /// removes the retired server from both and keeps the others.
    #[test]
    fn retiring_direct_credentials_preserves_other_servers_and_preview_is_readonly() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let config = retirement_config();
        reconcile_managed_servers(root, &config).unwrap();
        reconcile_managed_servers_agy(root, &config).unwrap();
        let before_json = std::fs::read(mcp_path(root)).unwrap();
        let before_agy = std::fs::read(agy_mcp_path(root)).unwrap();

        retire_user_server(root, "private", false).unwrap();
        assert_eq!(std::fs::read(mcp_path(root)).unwrap(), before_json);
        assert_eq!(std::fs::read(agy_mcp_path(root)).unwrap(), before_agy);

        retire_user_server(root, "private", true).unwrap();
        for path in [mcp_path(root), agy_mcp_path(root)] {
            let doc = read_doc(&path).unwrap();
            assert!(
                doc["mcpServers"].get("private").is_none(),
                "{}",
                path.display()
            );
            assert!(
                doc["mcpServers"].get("keep").is_some(),
                "{}",
                path.display()
            );
            assert_eq!(doc[MANAGED_KEY], json!(["keep"]), "{}", path.display());
        }
    }

    /// A hand-added (untracked) entry with the retired name in agy's launcher
    /// is refused — Otto can't claim it — and nothing is written anywhere, not
    /// even the tracked `.mcp.json` entry that would otherwise be removable.
    #[test]
    fn retiring_refuses_untracked_agy_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        reconcile_managed_servers(root, &retirement_config()).unwrap();
        let agy = agy_mcp_path(root);
        std::fs::create_dir_all(agy.parent().unwrap()).unwrap();
        std::fs::write(&agy, r#"{"mcpServers":{"private":{"command":"mine"}}}"#).unwrap();
        let before_json = std::fs::read(mcp_path(root)).unwrap();

        for apply in [false, true] {
            let err = retire_user_server(root, "private", apply).unwrap_err();
            assert!(err.contains("mcp_config.json"), "{err}");
            assert_eq!(std::fs::read(mcp_path(root)).unwrap(), before_json);
            assert!(read_doc(&agy).unwrap()["mcpServers"]
                .get("private")
                .is_some());
        }
    }

    /// integrations-3-03: a server's Keychain-resolved env file goes when the
    /// server leaves the launcher (deleted/disabled) or loses its env, and the
    /// delete/disable sweep removes every cwd's copy — but a failed lookup
    /// (`user_servers: None`) keeps the preserved entry's file.
    #[test]
    fn reconcile_and_sweep_remove_stale_launcher_env_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let name = format!("env-gone-{}", std::process::id());
        let mut s = user(&name);
        s.env.insert("TOKEN".into(), "leaked-secret".into());
        let with = |servers: Option<Vec<UserMcpServer>>| ManagedMcpConfig {
            browser: false,
            user_servers: servers,
            otto_tools: None,
        };
        let env_path = launcher_env_path(root, &name);

        reconcile_managed_servers(root, &with(Some(vec![s.clone()]))).unwrap();
        assert!(env_path.exists(), "the env file backs the launcher entry");
        reconcile_managed_servers(root, &with(None)).unwrap();
        assert!(
            env_path.exists(),
            "a failed lookup preserves the entry and its env"
        );
        reconcile_managed_servers(root, &with(Some(vec![]))).unwrap();
        assert!(!env_path.exists(), "a removed server's env file is deleted");
        assert!(!read_servers(root).contains_key(&name));

        reconcile_managed_servers(root, &with(Some(vec![s.clone()]))).unwrap();
        assert!(env_path.exists());
        let mut no_env = s.clone();
        no_env.env.clear();
        reconcile_managed_servers(root, &with(Some(vec![no_env]))).unwrap();
        assert!(!env_path.exists(), "an env-less server keeps no env file");

        let other = tempfile::tempdir().unwrap();
        let other_root = other.path().to_str().unwrap();
        reconcile_managed_servers(root, &with(Some(vec![s.clone()]))).unwrap();
        reconcile_managed_servers(other_root, &with(Some(vec![s.clone()]))).unwrap();
        let other_path = launcher_env_path(other_root, &name);
        let keep = launcher_env_path(root, &format!("{name}-other"));
        std::fs::write(&keep, "X=1\n").unwrap();
        assert_eq!(sweep_launcher_env_files(&name), 2);
        assert!(!env_path.exists() && !other_path.exists());
        assert!(keep.exists(), "another server's env file is left alone");
        let _ = std::fs::remove_file(&keep);
    }
}

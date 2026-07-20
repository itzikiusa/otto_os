//! SessionManager — owns live PTYs, the sessions DB rows and per-session
//! status tasks (working/idle/exited detection + events).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{
    Session, SessionKind, SessionStatus, TrailKind, TrailLevel, TrailSource, Workspace,
};
use otto_core::event::Event;
use otto_core::hooks::{McpServerProvider, PreSpawnHook};
use otto_core::{new_id, Error, Id, Result};
use otto_pty::{resolve_grid, CommandSpec, PtyHandle};
use otto_rbac::AuthRepo;
use otto_state::{ActivityRepo, NewSession, NewTrail, SessionsRepo};
use tokio::sync::{broadcast, Mutex};

use crate::providers::ProviderRegistry;

/// Build `["--add-dir", path, ...]` args for providers that support `--add-dir`
/// (claude, codex, agy — NOT shell).  Returns an empty vec for unknown/shell
/// providers or when `meta` has no `extra_dirs` array.
///
/// NOTE: this is provider-agnostic on purpose (it just grants dir access). It is
/// NOT a skill-registration mechanism: only claude first-class-loads skills from
/// an added dir's `.claude/skills`. If you put a skill bundle in `extra_dirs`,
/// gate it to claude at the CALL SITE (see
/// `otto_server::review_session::review_skills_extra_dirs`) — handing a
/// `.claude/skills` bundle to codex makes it scavenge and run the wrong skill.
fn add_dir_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if provider == "shell" {
        return vec![];
    }
    let Some(arr) = meta.get("extra_dirs").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut out = Vec::with_capacity(arr.len() * 2);
    for item in arr {
        if let Some(dir) = item.as_str() {
            if !dir.is_empty() {
                out.push("--add-dir".to_string());
                out.push(dir.to_string());
            }
        }
    }
    out
}

/// Build `["--model", name]` args when `meta.model` is set and the provider
/// supports an explicit `--model` flag (claude — same flag codex accepts).
/// Returns an empty vec for shell or unknown providers, or when `meta.model`
/// is absent/empty.  The model flag is provider-specific:
///   - claude/codex: `--model <name>`
///   - agy, shell: unsupported — silently omitted (agy has no model flag).
fn model_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if !matches!(provider, "claude" | "codex") {
        return vec![];
    }
    let Some(model) = meta.get("model").and_then(|v| v.as_str()) else {
        return vec![];
    };
    let model = model.trim();
    if model.is_empty() {
        return vec![];
    }
    vec!["--model".to_string(), model.to_string()]
}

/// Per-session creds file for the Codex `otto` MCP server: a daemon-private temp
/// path (`<tmp>/otto-mcp/<session_id>.json`, mode 0600). Holds the per-session
/// token so it never appears on Codex's argv; removed when the session is removed.
fn codex_creds_path(session_id: &Id) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("otto-mcp")
        .join(format!("{session_id}.json"))
}

/// Write the per-session Codex creds file (0600) carrying the token + routing the
/// `ottod mcp-tools --config <path>` process reads. Returns the path on success.
fn write_codex_creds(
    session_id: &Id,
    token: &str,
    base: &str,
    workspace_id: &str,
    source: Option<&str>,
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
/// must receive the same policy context.
fn otto_tools_env(
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
    env
}

#[derive(Default)]
struct OttoToolsInjection {
    args: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
}

/// One row of the live process table: (pid, ppid, cumulative CPU ms).
type ProcRow = (u32, u32, u64);

/// Snapshot the OS process table via one `ps -axo pid=,ppid=,time=` pass.
/// Used by the idle-suspend sweep; a failed/absent `ps` yields an empty table
/// (the sweep then behaves exactly as before the guard existed).
fn process_table() -> Vec<ProcRow> {
    let out = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,time="])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let cpu = parse_ps_time_ms(it.next()?)?;
            Some((pid, ppid, cpu))
        })
        .collect()
}

/// Parse `ps` cumulative CPU time (`MM:SS.ss`, `HH:MM:SS`, or `D-HH:MM:SS`)
/// into milliseconds.
fn parse_ps_time_ms(s: &str) -> Option<u64> {
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<u64>().ok()?,
            m.parse::<u64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        [m, s] => (0, m.parse::<u64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some((days * 86_400_000) + (h * 3_600_000) + (m * 60_000) + (sec * 1000.0) as u64)
}

/// Total cumulative CPU (ms) of every DESCENDANT of `root` — children,
/// grandchildren, … — excluding `root` itself (an idle agent TUI accrues CPU
/// redrawing; its tool subprocesses are what indicate in-flight work).
fn descendant_cpu_ms(root: u32, table: &[ProcRow]) -> u64 {
    use std::collections::{HashMap, HashSet};
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut cpu: HashMap<u32, u64> = HashMap::new();
    for (pid, ppid, ms) in table {
        children.entry(*ppid).or_default().push(*pid);
        cpu.insert(*pid, *ms);
    }
    let mut total = 0u64;
    let mut seen: HashSet<u32> = HashSet::new();
    let mut stack: Vec<u32> = children.get(&root).cloned().unwrap_or_default();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue; // defensive: a malformed table must not loop forever
        }
        total += cpu.get(&pid).copied().unwrap_or(0);
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    total
}

/// Root directory codex writes session rollouts to: `$CODEX_HOME/sessions`,
/// else `~/.codex/sessions`.
fn codex_sessions_root() -> std::path::PathBuf {
    let home = std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex")
        });
    home.join("sessions")
}

/// Capture the codex session UUID for a just-spawned session by scanning its
/// rollout files (`<root>/YYYY/MM/DD/rollout-*.jsonl`). Matches a TOP-LEVEL
/// interactive session (`originator == "codex-tui"`, `thread_source == "user"`)
/// whose recorded `cwd` equals ours, that isn't already `claimed`, and whose
/// file is at/after `spawn_time`. Returns the OLDEST such match — the rollout
/// THIS launch created (a later concurrent same-cwd spawn's rollout is newer and
/// belongs to it). Polls briefly because codex writes the rollout a moment after
/// launch. `None` if nothing matches in the window — caller leaves the session
/// non-resumable rather than guessing and resuming the wrong conversation.
async fn capture_codex_session_id(
    sessions_root: &std::path::Path,
    cwd: &str,
    spawn_time: std::time::SystemTime,
    claimed: &[String],
) -> Option<String> {
    use std::collections::HashSet;
    let claimed: HashSet<&str> = claimed.iter().map(String::as_str).collect();
    // A touch before spawn to tolerate clock/fs skew; a prior session's rollout
    // in this cwd is either older than this floor or already in `claimed`.
    let floor = spawn_time
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(std::time::UNIX_EPOCH);
    // 60s window: codex-tui can take 20s+ to mint its rollout when it boots
    // with heavy context/MCP config, and a concurrent same-cwd capture holding
    // `codex_capture_lock` delays this one's first scan further. A miss here
    // leaves the session permanently non-resumable (it opens DEAD after the
    // next daemon restart), so err long — the loop exits on first match.
    for _ in 0..120 {
        if let Some(sid) = scan_codex_rollout(sessions_root, cwd, floor, &claimed) {
            return Some(sid);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// One scan-and-pick pass: the OLDEST unclaimed top-level codex rollout for `cwd`
/// at/after `floor`. Split out from the polling loop so it's synchronously
/// testable. See [`capture_codex_session_id`].
fn scan_codex_rollout(
    sessions_root: &std::path::Path,
    cwd: &str,
    floor: std::time::SystemTime,
    claimed: &std::collections::HashSet<&str>,
) -> Option<String> {
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for path in recent_codex_rollouts(sessions_root, floor) {
        let Some((session_id, mtime)) = codex_rollout_match(&path, cwd) else {
            continue;
        };
        if claimed.contains(session_id.as_str()) {
            continue;
        }
        let take = match &best {
            Some((t, _)) => mtime < *t,
            None => true,
        };
        if take {
            best = Some((mtime, session_id));
        }
    }
    best.map(|(_, sid)| sid)
}

/// Recursively collect `*.jsonl` rollout files under `root` modified at/after
/// `cutoff`. Bounded depth (the layout is `YYYY/MM/DD/`); the cutoff keeps the
/// scan cheap even with a deep history.
fn recent_codex_rollouts(
    root: &std::path::Path,
    cutoff: std::time::SystemTime,
) -> Vec<std::path::PathBuf> {
    fn walk(
        dir: &std::path::Path,
        cutoff: std::time::SystemTime,
        out: &mut Vec<std::path::PathBuf>,
        depth: usize,
    ) {
        if depth > 5 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => walk(&path, cutoff, out, depth + 1),
                Ok(_)
                    if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                        && entry
                            .metadata()
                            .and_then(|m| m.modified())
                            .map(|m| m >= cutoff)
                            .unwrap_or(false) =>
                {
                    out.push(path);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, cutoff, &mut out, 0);
    out
}

/// If `path` is a TOP-LEVEL codex rollout whose recorded cwd == `cwd`, return its
/// session UUID and file mtime. Reads only the first line (`session_meta`).
fn codex_rollout_match(
    path: &std::path::Path,
    cwd: &str,
) -> Option<(String, std::time::SystemTime)> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    std::io::BufReader::new(file).read_line(&mut first).ok()?;
    let v: serde_json::Value = serde_json::from_str(first.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
        return None;
    }
    let p = v.get("payload")?;
    if p.get("cwd").and_then(|c| c.as_str()) != Some(cwd) {
        return None;
    }
    // Top-level interactive session only — exclude subagent threads, which mint
    // their own rollouts under the same cwd.
    if p.get("originator").and_then(|o| o.as_str()) != Some("codex-tui")
        || p.get("thread_source").and_then(|t| t.as_str()) != Some("user")
    {
        return None;
    }
    let session_id = p.get("session_id").and_then(|s| s.as_str())?.to_string();
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    Some((session_id, mtime))
}

/// Root directory agy (Antigravity Gemini CLI) writes conversations to:
/// `~/.gemini/antigravity-cli`. Holds `conversations/<id>.db|.pb` (one per
/// conversation, named by its UUID) plus a `cache/last_conversations.json`
/// map of `cwd -> most-recent conversation id`.
fn agy_cli_root() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".gemini")
        .join("antigravity-cli")
}

/// Capture the agy conversation UUID for a just-spawned session. agy mints its
/// own conversation id (like codex) and records the most-recent conversation per
/// cwd in `cache/last_conversations.json`; the id names a `conversations/<id>.db`
/// (or `.pb`) file. We poll that map for OUR cwd and accept the mapped id once its
/// conversation file is FRESH (mtime at/after `spawn_time`) and not already
/// `claimed` by another Otto session — so we never capture a stale pre-existing
/// conversation for the same cwd. Polls because agy writes the conversation a
/// moment after launch. `None` if nothing matches in the window — the session
/// stays non-resumable rather than resuming the wrong conversation.
async fn capture_agy_session_id(
    cli_root: &std::path::Path,
    cwd: &str,
    spawn_time: std::time::SystemTime,
    claimed: &[String],
) -> Option<String> {
    use std::collections::HashSet;
    let claimed: HashSet<&str> = claimed.iter().map(String::as_str).collect();
    // A touch before spawn to tolerate clock/fs skew; a prior session's
    // conversation in this cwd is either older than this floor or already claimed.
    let floor = spawn_time
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(std::time::UNIX_EPOCH);
    // agy may only persist the conversation after the first turn, so poll a touch
    // longer than codex (≈20s) before giving up.
    for _ in 0..40 {
        if let Some(sid) = scan_agy_conversation(cli_root, cwd, floor, &claimed) {
            return Some(sid);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// One scan-and-pick pass: read `cache/last_conversations.json`, look up `cwd`,
/// and accept the mapped conversation id when its `conversations/<id>.{db,pb}`
/// file exists, is modified at/after `floor`, and isn't already `claimed`. Split
/// out from the polling loop so it's synchronously testable.
/// See [`capture_agy_session_id`].
fn scan_agy_conversation(
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

/// Max length (chars) of an auto-derived provider title before it is clipped.
const PROVIDER_TITLE_MAX: usize = 60;
/// Ceiling on how many transcript lines the auto-namer parsers scan before
/// giving up on finding a first user prompt — bounds the read on a large
/// transcript that genuinely never carried one (all-system/tool content).
const PROVIDER_TITLE_SCAN_LINES: usize = 4000;

/// Collapse a raw provider prompt into a one-line session title: strip control
/// characters / newlines to single spaces, squeeze runs of whitespace, and clip
/// to [`PROVIDER_TITLE_MAX`] chars (char-boundary safe, `…` suffix). Returns
/// `None` for an empty/blank result so callers skip it.
fn clean_provider_title(s: &str) -> Option<String> {
    let mut collapsed = String::with_capacity(s.len().min(256));
    let mut last_space = true; // leading whitespace is dropped
    for ch in s.chars() {
        if ch.is_whitespace() || ch.is_control() {
            if !last_space {
                collapsed.push(' ');
                last_space = true;
            }
        } else {
            collapsed.push(ch);
            last_space = false;
        }
    }
    let trimmed = collapsed.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= PROVIDER_TITLE_MAX {
        return Some(trimmed.to_string());
    }
    let mut out: String = trimmed.chars().take(PROVIDER_TITLE_MAX).collect();
    // Don't leave a dangling space before the ellipsis.
    while out.ends_with(' ') {
        out.pop();
    }
    out.push('…');
    Some(out)
}

/// True when a claude/codex transcript text line is a wrapper/meta prompt rather
/// than a real user message: command echoes (`<command-name>…`), tool output
/// (`<local-command-stdout>`), injected context (`# AGENTS.md instructions`,
/// `<permissions instructions>`), or a Claude caveat preamble. These are never
/// the human's own words, so the title parsers skip them.
fn is_wrapper_prompt(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty()
        || t.starts_with('<')
        || t.starts_with("# AGENTS.md")
        || t.starts_with("Caveat:")
        || t.starts_with("# CLAUDE.md")
}

/// Pull the first genuine user prompt out of a Claude Code transcript JSONL
/// (`~/.claude/projects/<enc-cwd>/<uuid>.jsonl`). Claude records no dedicated
/// title/summary line in this format, so the first real `type:"user"` message is
/// the best human-meaningful name. Skips meta lines (`isMeta`), tool-result
/// user turns (array content with no text parts), and wrapper prompts
/// (slash-command echoes, injected CLAUDE.md/AGENTS.md). `None` when none is
/// found within [`PROVIDER_TITLE_SCAN_LINES`].
fn parse_claude_first_prompt(contents: &str) -> Option<String> {
    for line in contents.lines().take(PROVIDER_TITLE_SCAN_LINES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        // Sidechain (subagent) turns and injected meta turns aren't the user.
        if v.get("isMeta").and_then(|m| m.as_bool()) == Some(true)
            || v.get("isSidechain").and_then(|m| m.as_bool()) == Some(true)
        {
            continue;
        }
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let text = match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => {
                // Concatenate text parts; a tool_result-only turn yields "".
                let mut buf = String::new();
                for p in parts {
                    if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            if !buf.is_empty() {
                                buf.push(' ');
                            }
                            buf.push_str(t);
                        }
                    }
                }
                buf
            }
            _ => continue,
        };
        if is_wrapper_prompt(&text) {
            continue;
        }
        if let Some(title) = clean_provider_title(&text) {
            return Some(title);
        }
    }
    None
}

/// Pull the first genuine user prompt out of a Codex rollout JSONL
/// (`rollout-<ts>-<uuid>.jsonl`). Codex records the typed prompt as an
/// `event_msg` of `payload.type == "user_message"` (`payload.message`) — the
/// clean human message, distinct from the injected AGENTS.md context that
/// arrives as a `response_item` user turn. Falls back to the first
/// `response_item` user `input_text` that isn't a wrapper prompt for older
/// rollouts that predate the `user_message` event. `None` when none is found
/// within [`PROVIDER_TITLE_SCAN_LINES`].
fn parse_codex_first_prompt(contents: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    for line in contents.lines().take(PROVIDER_TITLE_SCAN_LINES) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let payload = v.get("payload");
        match v.get("type").and_then(|t| t.as_str()) {
            Some("event_msg") => {
                let Some(p) = payload else { continue };
                if p.get("type").and_then(|t| t.as_str()) != Some("user_message") {
                    continue;
                }
                if let Some(msg) = p.get("message").and_then(|m| m.as_str()) {
                    if !is_wrapper_prompt(msg) {
                        if let Some(title) = clean_provider_title(msg) {
                            return Some(title);
                        }
                    }
                }
            }
            // Fallback for pre-`user_message` rollouts: first non-wrapper user
            // input_text. Recorded but not returned until the scan ends, so a
            // later real `user_message` event still wins.
            Some("response_item") if fallback.is_none() => {
                let Some(p) = payload else { continue };
                if p.get("type").and_then(|t| t.as_str()) != Some("message")
                    || p.get("role").and_then(|r| r.as_str()) != Some("user")
                {
                    continue;
                }
                if let Some(parts) = p.get("content").and_then(|c| c.as_array()) {
                    for part in parts {
                        if part.get("type").and_then(|t| t.as_str()) == Some("input_text") {
                            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                                if !is_wrapper_prompt(t) {
                                    fallback = clean_provider_title(t);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fallback
}

/// Gate for [`SessionManager::refresh_provider_titles`]: only LIVE (non-exited),
/// non-archived foreground agent sessions on a title-bearing provider
/// (claude/codex) with a captured provider session id and a
/// not-user-and-not-already-provider `meta.title_source` are worth probing. The
/// durable `title_source` marker is what keeps a resolved session from being
/// re-probed after a daemon restart (the in-memory cache is empty then).
fn title_eligible(s: &Session) -> bool {
    if s.archived
        || s.status == SessionStatus::Exited
        || !s.is_foreground_agent()
        || !matches!(s.provider.as_str(), "claude" | "codex")
        || s.provider_session_id.is_none()
    {
        return false;
    }
    // "user" = the user owns the name; "provider" = already auto-named and the
    // first prompt is stable, so there's nothing new to read.
    !matches!(
        s.meta.get("title_source").and_then(|v| v.as_str()),
        Some("user") | Some("provider")
    )
}

/// Read a provider transcript file and extract its first user prompt as a
/// session title. Dispatches on provider; synchronous (callers run it on the
/// blocking pool). `None` for unsupported providers, an unreadable file, or a
/// transcript with no user prompt yet.
fn read_provider_title(provider: &str, path: &std::path::Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    match provider {
        "claude" => parse_claude_first_prompt(&contents),
        "codex" => parse_codex_first_prompt(&contents),
        _ => None,
    }
}

/// Truncate `s` to at most `max` chars (char-boundary safe), appending `…`.
/// Used for one-line trail summaries.
fn trail_clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Window after the last output chunk during which a session is `working`.
const WORKING_WINDOW: Duration = Duration::from_secs(5);
/// Status poll interval.
const STATUS_TICK: Duration = Duration::from_secs(2);

/// How long a LIVE resumable session must be idle (no output) AND unattached
/// (no WS viewer) before its PTY is suspended to free RAM. The conversation
/// stays resumable, so reopening it auto-resumes via `--resume`.
pub const SUSPEND_GRACE: Duration = Duration::from_secs(5 * 60);

/// Hook that inspects live PTY output for a session, used by otto-server's
/// credential monitor to detect mid-session re-auth prompts (e.g. "run
/// `claude login`", "session expired").
///
/// Lives here (not in otto-core) to avoid a core dependency. **Best-effort:**
/// implementations MUST handle their own errors and never panic — a scan
/// failure must never disturb the session's status task. `chunk` is a raw PTY
/// output slice (may split lines); implementations should keep their own small
/// rolling context and debounce per session.
pub trait OutputScanner: Send + Sync {
    /// Called for each PTY output chunk. `provider` is the session's CLI
    /// provider ("claude", "codex", "shell", …) used as the re-auth target.
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]);
}

/// Await the child exit code without holding the (non-Send) watch guard
/// across an await point. `None` when the watch sender was dropped.
pub(crate) async fn wait_exit_code(
    rx: &mut tokio::sync::watch::Receiver<Option<i32>>,
) -> Option<i32> {
    let res = rx.wait_for(|v| v.is_some()).await;
    match res {
        Ok(guard) => Some((*guard).unwrap_or(-1)),
        Err(_) => None,
    }
}

/// RAII guard for a WS terminal attachment: decrements the session's attached-
/// viewer count when dropped, on every WS `serve_terminal` return path.
pub struct AttachGuard {
    manager: Arc<SessionManager>,
    id: Id,
    /// Daemon-unique id of this WS connection (size-authority tracking).
    conn_id: u64,
}

impl AttachGuard {
    /// This connection's daemon-unique id, passed to the size-authority calls.
    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.manager.detach(&self.id, self.conn_id);
    }
}

/// Default daemon base URL agent hooks post activity back to. Overridden via
/// [`SessionManager::with_ingest_base`] (ottod sets it from its bind port).
const DEFAULT_INGEST_BASE: &str = "http://127.0.0.1:7700";

/// The name theme applied to new agent sessions when a user hasn't chosen one.
/// Single source of truth lives in [`crate::names::DEFAULT_THEME`].
const DEFAULT_NAME_THEME: &str = crate::names::DEFAULT_THEME;

/// Owns live sessions: PTY handles keyed by session id plus persistence.
/// Outcome of a name-addressed [`SessionManager::relay`].
#[derive(Debug, Clone)]
pub struct RelayOutcome {
    /// Sessions the message was delivered to (empty when unaddressed).
    pub session_ids: Vec<Id>,
    /// True when the address was an explicit broadcast keyword ("all"/"everyone").
    pub broadcast: bool,
    /// True when `text` contained no recognizable session address — the caller
    /// should fall back to its normal handling.
    pub unaddressed: bool,
    /// The message actually sent (address prefix stripped).
    pub text: String,
}

/// Per-session bookkeeping for the provider-title auto-namer. Keeps the sweep
/// cheap: once a session is `resolved` (a provider title was adopted, or the
/// user claimed the name) it is never read again; otherwise `mtime` skips a
/// re-parse when the transcript hasn't grown since the last look.
#[derive(Default)]
struct TitleProbe {
    /// Transcript mtime at the last parse. Skip re-reading when unchanged.
    mtime: Option<std::time::SystemTime>,
    /// Terminal state: a title was applied/matched, or the user owns the name.
    /// The first user prompt is stable, so a resolved session never re-reads.
    resolved: bool,
}

pub struct SessionManager {
    /// Shared so the per-session status task can evict an exited handle
    /// (otherwise dead PtyHandles — and their emulator + ring buffer — leak).
    live: Arc<DashMap<Id, Arc<PtyHandle>>>,
    /// In-memory count of attached WS terminal viewers per session. Bumped by
    /// `ws::serve_terminal` on attach/detach; read by the idle-suspend sweep so
    /// it never suspends a session someone is actively watching.
    attached: Arc<DashMap<Id, usize>>,
    /// Attached WS connection ids per session (see [`Self::may_resize`]).
    attached_conns: Arc<DashMap<Id, Vec<u64>>>,
    /// Size-authority owner per session: the conn that most recently typed.
    size_owner: Arc<DashMap<Id, u64>>,
    /// Session ids whose PTY is being deliberately suspended (RAM release, not
    /// a real exit). The per-session status task consults this in its exit
    /// branch so it marks the session `Reconnectable` (still resumable) instead
    /// of `Exited`, winning the kill→exit race deterministically.
    suspending: Arc<DashMap<Id, ()>>,
    /// Last observed cumulative CPU (ms) of each live session's DESCENDANT
    /// process tree (excluding the direct child), sampled by the idle-suspend
    /// sweep. A tree that accrued CPU since the previous sweep is running a
    /// quiet long command (build/test) — not idle — and is skipped.
    suspend_cpu: Arc<DashMap<Id, u64>>,
    repo: SessionsRepo,
    events: broadcast::Sender<Event>,
    providers: ProviderRegistry,
    /// Optional context-provisioning hook, invoked before an agent spawn.
    pre_spawn_hook: Option<Arc<dyn PreSpawnHook>>,
    /// Optional resolver for the workspace's enabled user-configured MCP servers,
    /// merged into `.mcp.json` on agent spawn (alongside Otto's browser entry).
    mcp_servers: Option<Arc<dyn McpServerProvider>>,
    /// Optional live-output scanner (credential monitor's mid-session auth
    /// detection). When set, each session's status task subscribes to its PTY
    /// output and forwards chunks here.
    output_scanner: Option<Arc<dyn OutputScanner>>,
    /// Daemon base URL that injected agent hooks post their activity back to.
    ingest_base: String,
    /// Per-session ingest tokens. An agent's hooks present this token to the
    /// (otherwise unauthenticated) `/ingest/*` endpoints; the route verifies it
    /// against this map. Minted at spawn, dropped when the session is removed.
    ingest_tokens: Arc<DashMap<Id, String>>,
    /// Optional activity store: records Otto-side lifecycle and user actions to
    /// the session trail (so the trail is populated for every provider, not just
    /// ones with native hooks).
    activity: Option<ActivityRepo>,
    /// Per-session forced-disconnect signal. Attached `/ws/term` viewers
    /// subscribe via [`Self::evict_signal`]; [`Self::evict`] fires a unit to all
    /// of them so they immediately send `{"type":"terminated"}` and close.
    /// Created lazily (a session only gets a sender once someone subscribes or
    /// evicts it). A `broadcast` channel is used so every attached viewer is
    /// dropped, not just one — mirrors how `live`/`attached` are keyed by id.
    evict: Arc<DashMap<Id, broadcast::Sender<()>>>,
    /// Optional settings store used to read the configurable idle-suspend grace
    /// period (`idle_suspend_grace_secs`). Falls back to [`SUSPEND_GRACE`] when
    /// absent or when the key is not set.
    settings: Option<otto_state::SettingsRepo>,
    /// Optional auth-token repo. When set (and `otto_mcp_enabled` is on for the
    /// workspace), an agent spawn mints a per-session token for Otto's first-party
    /// read-only MCP tool server (Task B2b) and injects the `otto` entry into
    /// `.mcp.json`. Absent ⇒ the feature is entirely off.
    auth: Option<AuthRepo>,
    /// Absolute path to the `ottod` binary that backs the `otto` MCP tool server
    /// (`<path> mcp-tools`). Defaults to the running executable's own path so the
    /// tools subcommand is always the same build as the daemon.
    mcp_tools_bin: String,
    /// Per-session MCP-token ids (the auth-token row id, NOT the secret), so the
    /// token minted for the `otto` server can be revoked when the session is
    /// removed. Keyed like `ingest_tokens`.
    mcp_tokens: Arc<DashMap<Id, String>>,
    /// Serializes the post-spawn codex session-id capture so two codex sessions
    /// launched in the SAME cwd claim DISTINCT on-disk rollouts: each capture
    /// runs under this lock and persists its claim before releasing, so the next
    /// one sees it in the claimed set. See `spawn_session_id_capture`.
    codex_capture_lock: Arc<Mutex<()>>,
    /// Optional name-themes store. When set, a new agent session whose title is
    /// not explicitly provided is auto-named from the creating user's active
    /// theme (e.g. "Ronaldo"), unique among the workspace's open sessions.
    /// Absent ⇒ the legacy "{provider} #N" numbering.
    name_themes: Option<otto_state::NameThemesRepo>,
    /// Per-session state for the provider-title auto-namer sweep
    /// ([`Self::refresh_provider_titles`]). In-memory only: it's a read-cache,
    /// and the durable `meta.title_source` marker survives restarts.
    title_probe: Arc<DashMap<Id, TitleProbe>>,
}

impl SessionManager {
    pub fn new(
        repo: SessionsRepo,
        events: broadcast::Sender<Event>,
        providers: ProviderRegistry,
    ) -> Self {
        Self {
            live: Arc::new(DashMap::new()),
            attached: Arc::new(DashMap::new()),
            attached_conns: Arc::new(DashMap::new()),
            size_owner: Arc::new(DashMap::new()),
            suspending: Arc::new(DashMap::new()),
            suspend_cpu: Arc::new(DashMap::new()),
            repo,
            events,
            providers,
            pre_spawn_hook: None,
            mcp_servers: None,
            output_scanner: None,
            ingest_base: std::env::var("OTTO_INGEST_BASE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_INGEST_BASE.to_string()),
            ingest_tokens: Arc::new(DashMap::new()),
            activity: None,
            evict: Arc::new(DashMap::new()),
            settings: None,
            auth: None,
            // Default to this daemon's own binary so `mcp-tools` is the same build.
            mcp_tools_bin: std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(str::to_owned))
                .unwrap_or_else(|| "ottod".to_string()),
            mcp_tokens: Arc::new(DashMap::new()),
            codex_capture_lock: Arc::new(Mutex::new(())),
            name_themes: None,
            title_probe: Arc::new(DashMap::new()),
        }
    }

    /// Attach the name-themes store so new agent sessions are auto-named from the
    /// creating user's active theme. Builder-style; without it sessions fall back
    /// to "{provider} #N" numbering.
    pub fn with_name_themes_repo(mut self, repo: otto_state::NameThemesRepo) -> Self {
        self.name_themes = Some(repo);
        self
    }

    /// Attach the activity store so lifecycle + user actions are recorded to the
    /// session trail. Builder-style; without it the recording calls are no-ops.
    pub fn with_activity_repo(mut self, activity: ActivityRepo) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Attach the settings store so the idle-suspend grace period and other
    /// runtime-configurable parameters can be read at sweep time. Builder-style;
    /// without it all parameters fall back to their compiled-in defaults.
    pub fn with_settings_repo(mut self, settings: otto_state::SettingsRepo) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Attach the auth-token repo used to mint the per-session token for the
    /// first-party `otto` MCP tool server (Task B2b). Without it the feature is
    /// off even if `otto_mcp_enabled` is set. Builder-style.
    pub fn with_auth_repo(mut self, auth: AuthRepo) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Override the `ottod` binary path that backs the `otto` MCP tool server
    /// (`<path> mcp-tools`). Defaults to the running executable. Builder-style.
    pub fn with_mcp_tools_bin(mut self, bin: impl Into<String>) -> Self {
        self.mcp_tools_bin = bin.into();
        self
    }

    /// Best-effort: persist a trail entry and broadcast it. Fire-and-forget so
    /// callers (lifecycle methods) never block on the DB. No-op without an
    /// activity store.
    fn record_trail(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        source: TrailSource,
        kind: TrailKind,
        level: TrailLevel,
        summary: String,
    ) {
        let Some(repo) = self.activity.clone() else {
            return;
        };
        let events = self.events.clone();
        let (sid, wid) = (session_id.clone(), workspace_id.clone());
        tokio::spawn(async move {
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: wid.clone(),
                source,
                kind,
                level,
                summary,
                detail: None,
            };
            match repo.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: wid,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!(session = %sid, "record trail: {e}"),
            }
        });
    }

    /// Record an Otto-side lifecycle entry (spawn/suspend/archive/…) for an
    /// agent session. Skips connection sessions to keep the trail agent-focused.
    fn record_lifecycle(&self, session: &Session, summary: impl Into<String>) {
        if session.kind != SessionKind::Agent {
            return;
        }
        self.record_trail(
            &session.id,
            &session.workspace_id,
            TrailSource::Otto,
            TrailKind::Session,
            TrailLevel::Info,
            summary.into(),
        );
    }

    /// Record a user-authored message relayed into a session (channel relay,
    /// orchestrator command). Surfaces the "by user" side of the trail for every
    /// provider. Best-effort; loads the session to resolve its workspace.
    pub async fn record_user_message(&self, session_id: &Id, text: &str) {
        if self.activity.is_none() {
            return;
        }
        let Ok(session) = self.repo.get(session_id).await else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let summary = trail_clip(trimmed, 200);
        self.record_trail(
            session_id,
            &session.workspace_id,
            TrailSource::User,
            TrailKind::Prompt,
            TrailLevel::Info,
            summary,
        );
    }

    /// Record an auto-approved prompt-guard action on the session activity trail.
    /// Called by [`crate::prompt_guard::PromptGuard`] after injecting approval
    /// keys. Best-effort; no-op when the activity store is absent.
    pub fn record_approval_trail(&self, session_id: &Id, provider: &str) {
        let summary = format!("Auto-approved trust/permission prompt for {provider}");
        let repo = self.repo.clone();
        let sid = session_id.clone();
        let activity = self.activity.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let Ok(session) = repo.get(&sid).await else {
                return;
            };
            let Some(activity) = activity else {
                return;
            };
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: session.workspace_id.clone(),
                source: TrailSource::Otto,
                kind: TrailKind::Session,
                level: TrailLevel::Info,
                summary,
                detail: None,
            };
            match activity.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: session.workspace_id,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!("prompt-guard record trail: {e}"),
            }
        });
    }

    /// Submit a message to a live session as if a human typed it and pressed
    /// Enter. Sends the text inside a bracketed-paste pair (so multi-line text
    /// stays intact and interactive TUIs treat it as pasted content rather than
    /// keystrokes), waits briefly for the TUI to finish handling the paste, then
    /// sends a real carriage return to submit.
    ///
    /// This is the reliable "actually send" path: writing `"{text}\n"` in one
    /// burst makes bracketed-paste TUIs (Claude Code, Codex) treat the trailing
    /// newline as pasted content — it inserts a newline instead of submitting,
    /// so the message is pasted but never sent. Mirrors the handover injector.
    pub async fn submit_text(&self, id: &Id, text: &str) -> Result<()> {
        let paste = format!("\x1b[200~{text}\x1b[201~");
        self.input(id, paste.as_bytes()).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.input(id, b"\r").await
    }

    /// Relay `text` verbatim to live agent sessions in workspace `ws`. When
    /// `targets` is `Some`, only those sessions are considered; otherwise every
    /// live agent session is. A session is eligible when it is an agent (not a
    /// connection) and its status is live (`Running | Working | Idle`).
    ///
    /// Each message is submitted via [`Self::submit_text`] (paste + Enter) and
    /// recorded on the session trail. Per-session failures are logged and
    /// skipped — they never abort the rest. Returns the ids that received it.
    ///
    /// Deliberately free of any AI/orchestrator involvement: it sends the literal
    /// text, nothing else.
    pub async fn broadcast_message(
        &self,
        ws: &Id,
        text: &str,
        targets: Option<&[Id]>,
    ) -> Result<Vec<Id>> {
        let sessions = self.list_by_workspace(ws).await?;
        let mut hit = Vec::new();
        for s in sessions {
            let live = matches!(
                s.status,
                SessionStatus::Running | SessionStatus::Working | SessionStatus::Idle
            );
            let targeted = targets.is_none_or(|ids| ids.iter().any(|t| t == &s.id));
            if s.kind == SessionKind::Agent && live && targeted {
                if let Err(e) = self.submit_text(&s.id, text).await {
                    tracing::warn!(session = %s.id, "broadcast failed: {e}");
                    continue;
                }
                self.record_user_message(&s.id, text).await;
                hit.push(s.id);
            }
        }
        Ok(hit)
    }

    /// Resolve a leading **name address** in `text` against this workspace's
    /// live agent sessions and deliver the (address-stripped) message to the
    /// matched session(s) — or broadcast it when addressed to "all".
    ///
    /// Examples: `"ronaldo: do X"` → the session named Ronaldo;
    /// `"ronaldo, messi: ship it"` → both; `"all: stand down"` → broadcast.
    /// When `text` carries no recognizable session address, returns
    /// `unaddressed = true` and delivers nothing, so the caller can fall back to
    /// its normal handling (e.g. AI orchestration). Only LIVE sessions are
    /// addressable (a suspended one has no PTY to receive input).
    pub async fn relay(&self, ws: &Id, text: &str) -> Result<RelayOutcome> {
        let sessions = self.list_by_workspace(ws).await?;
        let addressable: Vec<crate::names::Addressable> = sessions
            .iter()
            .filter(|s| {
                s.kind == SessionKind::Agent
                    && matches!(
                        s.status,
                        SessionStatus::Running | SessionStatus::Working | SessionStatus::Idle
                    )
            })
            .map(|s| {
                let handle = s
                    .meta
                    .get("name_handle")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&s.title)
                    .to_string();
                let full = s
                    .meta
                    .get("name_full")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&s.title)
                    .to_string();
                crate::names::Addressable {
                    id: s.id.clone(),
                    handle,
                    title: s.title.clone(),
                    full,
                }
            })
            .collect();

        let resolved = crate::names::resolve_address(text, &addressable);
        if resolved.targets.is_empty() {
            return Ok(RelayOutcome {
                session_ids: vec![],
                broadcast: false,
                unaddressed: true,
                text: text.to_string(),
            });
        }

        let msg = resolved.text.trim();
        let mut delivered = Vec::new();
        for id in &resolved.targets {
            if let Err(e) = self.submit_text(id, msg).await {
                tracing::warn!(session = %id, "relay failed: {e}");
                continue;
            }
            self.record_user_message(id, msg).await;
            delivered.push(id.clone());
        }
        Ok(RelayOutcome {
            session_ids: delivered,
            broadcast: resolved.broadcast,
            unaddressed: false,
            text: msg.to_string(),
        })
    }

    /// Prune the activity trail to the newest `keep_per_session` rows per
    /// session. No-op without an activity store. Returns rows pruned.
    pub async fn prune_activity_trail(&self, keep_per_session: i64) -> u64 {
        let Some(repo) = self.activity.as_ref() else {
            return 0;
        };
        match repo.prune_trail(keep_per_session).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("prune activity trail: {e}");
                0
            }
        }
    }

    /// Set the daemon base URL agent hooks post activity back to (ottod passes
    /// its actual bind URL). Builder-style.
    pub fn with_ingest_base(mut self, base: impl Into<String>) -> Self {
        let base = base.into();
        if !base.trim().is_empty() {
            self.ingest_base = base;
        }
        self
    }

    /// Attach a pre-spawn hook (context provisioning). Builder-style so existing
    /// `new()` callers (tests, channels) stay unchanged.
    pub fn with_pre_spawn_hook(mut self, hook: Arc<dyn PreSpawnHook>) -> Self {
        self.pre_spawn_hook = Some(hook);
        self
    }

    /// Attach the user-configured MCP-server resolver, merged into `.mcp.json`
    /// on agent spawn. Builder-style; without it no user servers are written.
    pub fn with_mcp_servers(mut self, provider: Arc<dyn McpServerProvider>) -> Self {
        self.mcp_servers = Some(provider);
        self
    }

    /// Attach a live-output scanner (mid-session re-auth detection). Builder-
    /// style so existing `new()` callers stay unchanged.
    pub fn with_output_scanner(mut self, scanner: Arc<dyn OutputScanner>) -> Self {
        self.output_scanner = Some(scanner);
        self
    }

    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// Verify an agent hook's ingest token for `session_id`. Returns false when
    /// the session has no token (not an agent / not spawned by this daemon) or
    /// the token doesn't match. Used by the unauthenticated `/ingest/*` routes.
    pub fn verify_ingest_token(&self, session_id: &Id, token: &str) -> bool {
        !token.is_empty()
            && self
                .ingest_tokens
                .get(session_id)
                .is_some_and(|t| t.as_str() == token)
    }

    /// Mint (or reuse) the ingest token for `session_id` and return the env vars
    /// that wire an agent's injected hooks back to this daemon. Pushed onto the
    /// spawned PTY's environment so hook subprocesses inherit them.
    fn ingest_env(&self, session_id: &Id) -> Vec<(String, String)> {
        let token = self
            .ingest_tokens
            .entry(session_id.clone())
            .or_insert_with(|| uuid::Uuid::new_v4().simple().to_string())
            .clone();
        vec![
            ("OTTO_INGEST_BASE".to_string(), self.ingest_base.clone()),
            ("OTTO_SESSION_ID".to_string(), session_id.to_string()),
            ("OTTO_INGEST_TOKEN".to_string(), token),
        ]
    }

    /// Wrap `spec` in an OS-level sandbox when the `process_sandbox` setting is
    /// enabled. Confines an agent CLI's filesystem **writes** to the workspace
    /// (+ the resolved git dir so commits in a worktree still work + the agent
    /// CLIs' own config/cache dirs + temp), while leaving reads global and
    /// network at the configured posture (default `full`, so the agent still
    /// reaches its model API). No-op for connection sessions, on non-macOS, or
    /// when the setting is absent/disabled. Mirrors how Claude Code / Codex CLI
    /// wrap their tools in Apple Seatbelt.
    async fn apply_sandbox(&self, spec: &mut CommandSpec, session: &Session) {
        if session.kind != SessionKind::Agent || session.cwd.trim().is_empty() {
            return;
        }
        if !otto_sandbox::is_supported() {
            return;
        }
        let Some(sr) = &self.settings else {
            return;
        };
        let cfg = match sr.get("process_sandbox").await {
            Ok(Some(v)) => v,
            _ => return,
        };
        let Some(network) = sandbox_decision(&cfg, session.kind, &session.provider) else {
            return;
        };

        let cwd = std::path::PathBuf::from(&session.cwd);
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let data_dir = home.join("Library/Application Support/Otto");
        // Resolve the git dir so commits in a worktree (whose .git lives outside
        // cwd) still work. Best-effort; absent for non-repos.
        let mut extra: Vec<std::path::PathBuf> = Vec::new();
        if let Some(gitdir) = resolve_git_common_dir(&cwd).await {
            extra.push(gitdir);
        }
        let policy =
            otto_sandbox::SandboxPolicy::for_agent(&cwd, &home, &data_dir, &extra, network);
        let (program, args) = policy.wrap(&spec.program, &spec.args);
        spec.program = program;
        spec.args = args;
        tracing::info!(
            session = %session.id,
            provider = %session.provider,
            "process sandbox enabled (network={network:?})"
        );
    }

    /// Is Otto's first-party MCP tool server enabled for `workspace_id`?
    /// Reads the `otto_mcp_enabled` setting and applies the shared precedence
    /// rules (see [`otto_state::otto_mcp_enabled_for`]); **default ON** when the
    /// setting is unset. Returns `false` only when no settings repo is wired (the
    /// feature is plumbed off entirely, e.g. in a bare test harness).
    async fn otto_mcp_enabled(&self, workspace_id: &str) -> bool {
        let Some(settings) = &self.settings else {
            return false;
        };
        let value = settings
            .get(otto_state::OTTO_MCP_ENABLED_KEY)
            .await
            .ok()
            .flatten();
        otto_state::otto_mcp_enabled_for(value.as_ref(), workspace_id)
    }

    /// When the `otto` MCP server is enabled for the workspace (default on), mint a
    /// per-session token and attach the server to the session. Claude/agy/grok
    /// discover an identity-neutral workspace launcher and inherit the credential
    /// from their own process environment; Codex receives per-spawn `-c` overrides
    /// because it does not read `.mcp.json`.
    ///
    /// Author sessions receive the existing owner API token. Vault reviewer
    /// sessions receive a persisted, session/workspace-bound MCP capability whose
    /// server-enforced scope contains only Vault reads. The row id is recorded in
    /// `mcp_tokens` so it is revoked on restart/removal. Best-effort: any failure
    /// here is logged and never blocks the spawn.
    ///
    /// Returns per-spawn args plus identity-bearing environment. Shared launcher
    /// files remain identity-neutral; every provider's MCP child inherits this
    /// session's own credential from its parent process.
    async fn maybe_enable_otto_tools(&self, session: &Session) -> OttoToolsInjection {
        if !self.otto_mcp_enabled(&session.workspace_id).await {
            return OttoToolsInjection::default();
        }
        let Some(auth) = &self.auth else {
            return OttoToolsInjection::default(); // feature wired off (no token minter)
        };
        // A restart replaces the previous in-process token before minting a new
        // one. (After a daemon restart the old raw secret is gone with the PTY;
        // the fixed TTL remains the final backstop.)
        self.revoke_mcp_token(&session.created_by, &session.id)
            .await;
        // Mint a per-session token for the owner. Labeled so it is identifiable
        // in the token list and revoked on session removal.
        let label = format!("otto-mcp:{}", session.id);
        let reviewer = session
            .meta
            .get("source")
            .and_then(serde_json::Value::as_str)
            == Some("vault-docs-review");
        let issued = if reviewer {
            auth.issue_vault_reviewer_token(
                &session.created_by,
                Some(&label),
                &session.id,
                &session.workspace_id,
            )
            .await
        } else {
            auth.issue_api_token(&session.created_by, Some(&label))
                .await
                .map(|(token, info)| (token, info.id))
        };
        let (token, token_id) = match issued {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("otto MCP tools: mint token failed: {e}");
                return OttoToolsInjection::default();
            }
        };
        self.mcp_tokens.insert(session.id.clone(), token_id);

        let env = otto_tools_env(session, &token, &self.ingest_base);
        let server = crate::mcp::OttoToolsServer {
            command: self.mcp_tools_bin.clone(),
            args: vec!["mcp-tools".to_string()],
            env: env.clone(),
        };
        // Claude / agy read the identity-neutral workspace `.mcp.json`; their
        // session-specific token and routing values come from the spawn env.
        if let Err(e) = crate::mcp::enable_otto_tools(&session.cwd, &server) {
            tracing::warn!("otto MCP tools: write .mcp.json failed: {e}");
        }
        // grok reads its own project-scoped `.grok/config.toml`, never
        // `.mcp.json` — without this a grok session has no otto tools.
        if session.provider == "grok" {
            if let Err(e) = crate::mcp::enable_otto_tools_grok(&session.cwd, &server) {
                tracing::warn!("otto MCP tools: write .grok/config.toml failed: {e}");
            }
        }
        // Codex doesn't read `.mcp.json`: attach via per-spawn `-c` overrides that
        // point at a per-session creds file (the token never touches argv).
        if session.provider == "codex" {
            match write_codex_creds(
                &session.id,
                &token,
                &self.ingest_base,
                &session.workspace_id,
                session
                    .meta
                    .get("source")
                    .and_then(serde_json::Value::as_str),
            ) {
                Ok(path) => {
                    return OttoToolsInjection {
                        args: crate::mcp::codex_mcp_inject_args(
                            &self.mcp_tools_bin,
                            &path.to_string_lossy(),
                        ),
                        env,
                    };
                }
                Err(e) => tracing::warn!("otto MCP tools: write codex creds failed: {e}"),
            }
        }
        OttoToolsInjection {
            args: Vec::new(),
            env,
        }
    }

    /// Revoke the per-session MCP token minted for `session_id` (if any), and
    /// delete the Codex creds file if one was written. Called from the
    /// session-removal path so the `otto` tool server's credential dies with the
    /// session. Best-effort.
    async fn revoke_mcp_token(&self, owner: &Id, session_id: &Id) {
        let _ = std::fs::remove_file(codex_creds_path(session_id));
        if let Some((_, token_id)) = self.mcp_tokens.remove(session_id) {
            if let Some(auth) = &self.auth {
                if let Err(e) = auth.revoke_api_token(owner, &token_id).await {
                    tracing::warn!("otto MCP tools: revoke token failed: {e}");
                }
                if let Err(e) = auth.revoke_vault_reviewer_token(owner, &token_id).await {
                    tracing::warn!("otto reviewer MCP token: revoke failed: {e}");
                }
            }
        }
    }

    /// All `(provider_name, update_command)` pairs for providers that have an
    /// update command configured. Delegates to the registry.
    pub fn provider_update_commands(&self) -> Vec<(String, String)> {
        self.providers.update_commands()
    }

    /// Return the resolved program binary for `name`, or `None` if the
    /// provider is not registered. Delegates to the registry.
    pub fn provider_program(&self, name: &str) -> Option<String> {
        self.providers.program_for(name)
    }

    /// Create a session row, spawn its PTY and start the status task.
    ///
    /// `spec_override` is used by connection sessions (the connections crate
    /// prebuilds the full command, including secret env vars). For
    /// `kind=agent` without an override the command comes from the provider
    /// registry. Title default: `"<provider> #N"`; callers that open
    /// connections should pass `req.title = Some(<connection name>)`.
    pub async fn create(
        &self,
        ws: &Workspace,
        user_id: &Id,
        req: CreateSessionReq,
        spec_override: Option<CommandSpec>,
    ) -> Result<Session> {
        let cwd = req.cwd.clone().unwrap_or_else(|| ws.root_path.clone());

        let (provider, mut spec, provider_session_id) = match spec_override {
            Some(spec) => {
                let provider = req
                    .provider
                    .clone()
                    .ok_or_else(|| Error::Invalid("provider is required".into()))?;
                (provider, spec, None)
            }
            None => {
                if req.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are opened via POST /connections/{id}/open".into(),
                    ));
                }
                let provider = req.provider.clone().ok_or_else(|| {
                    Error::Invalid("provider is required for agent sessions".into())
                })?;
                // claude's --session-id flag requires a UUID, so provider
                // session ids are UUIDs (the Otto session id stays a ULID).
                let sid = uuid::Uuid::new_v4().to_string();
                let mut spec = self.providers.build_spec(&provider, &sid, &cwd, false)?;
                // Append --add-dir args from req.meta.extra_dirs
                let meta_val = req.meta.clone().unwrap_or(serde_json::json!({}));
                spec.args.extend(add_dir_args(&provider, &meta_val));
                spec.args.extend(model_args(&provider, &meta_val));
                // Record the provider_session_id NOW only when Otto assigns it
                // (claude, via `--session-id {sid}`). Providers that mint their
                // own id (codex) start with None and have it captured from disk
                // after spawn — see the capture task below.
                let psid = (self.providers.supports_resume(&provider)
                    && !self.providers.captures_session_id(&provider))
                .then_some(sid);
                (provider, spec, psid)
            }
        };

        // Auto-name from the creating user's active name theme when no explicit
        // title was given (themed agent sessions only). Falls back to the legacy
        // "{provider} #N" numbering when no theme is active or the store is absent.
        let mut name_alloc: Option<crate::names::Allocated> = None;
        let title = match req.title.clone() {
            Some(t) if !t.is_empty() => t,
            _ => {
                if req.kind == SessionKind::Agent {
                    if let Some(alloc) =
                        self.allocate_session_name(&ws.id, user_id, &provider).await
                    {
                        let t = alloc.title.clone();
                        name_alloc = Some(alloc);
                        t
                    } else {
                        let n = self.repo.count_by_provider(&ws.id, &provider).await? + 1;
                        format!("{provider} #{n}")
                    }
                } else {
                    let n = self.repo.count_by_provider(&ws.id, &provider).await? + 1;
                    format!("{provider} #{n}")
                }
            }
        };

        // Record the callable handle + full display name in meta so the address
        // resolver ("ronaldo: do X") and the UI can use them. Explicitly-titled
        // sessions stay addressable by their title (resolver falls back to it).
        let mut meta = req.meta.clone().unwrap_or_else(|| serde_json::json!({}));
        if let (Some(alloc), Some(obj)) = (&name_alloc, meta.as_object_mut()) {
            obj.insert("name_handle".into(), alloc.handle.clone().into());
            obj.insert("name_full".into(), alloc.full.clone().into());
        }
        // Record how this session got its title so the provider-title auto-namer
        // knows whether it may replace it: "user" (explicit at creation — never
        // touched), "theme" (a name-theme allocation), or "auto" (the
        // "{provider} #N" fallback). Theme/auto titles are replaceable by the
        // provider's own session title once it appears.
        if let Some(obj) = meta.as_object_mut() {
            let source = if req.title.as_deref().is_some_and(|t| !t.is_empty()) {
                "user"
            } else if name_alloc.is_some() {
                "theme"
            } else {
                "auto"
            };
            obj.insert("title_source".into(), source.into());
        }

        let session = self
            .repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: req.kind,
                provider,
                title,
                cwd,
                provider_session_id,
                connection_id: req.connection_id.clone(),
                created_by: user_id.clone(),
                meta,
            })
            .await?;

        // The cwd must exist (a missing dir makes the child fall back to
        // $HOME) and agent CLIs should already trust the workspace folder.
        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
            // Browser tools: wire an MCP browser server into the workspace
            // when the session opted in (meta.browser == true).
            if session
                .meta
                .get("browser")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if let Err(e) = crate::mcp::enable_browser(&session.cwd) {
                    tracing::warn!("enable browser MCP: {e}");
                }
            }
            // User-configured MCP servers: merge the workspace's *enabled* ones
            // into `.mcp.json` alongside any managed entries. Opt-in by the user
            // (each server is enabled explicitly); best-effort — never blocks spawn.
            if let Some(provider) = &self.mcp_servers {
                let specs = provider.enabled_servers(&session.workspace_id);
                if !specs.is_empty() {
                    let servers: Vec<crate::mcp::UserMcpServer> = specs
                        .into_iter()
                        .map(|s| crate::mcp::UserMcpServer {
                            name: s.name,
                            command: s.command,
                            args: s.args,
                            env: s.env,
                        })
                        .collect();
                    if let Err(e) = crate::mcp::merge_user_servers(&session.cwd, &servers) {
                        tracing::warn!("merge user MCP servers: {e}");
                    }
                }
            }
            // Otto's first-party read-only tool server: when the workspace has
            // opted in (`otto_mcp_enabled`), mint a per-session token and inject
            // the `otto` MCP entry alongside the user/browser servers. Opt-in,
            // best-effort — never blocks spawn.
            let otto_tools = self.maybe_enable_otto_tools(&session).await;
            spec.args.extend(otto_tools.args);
            spec.env.extend(otto_tools.env);
            // Otto context provisioning: materialize the workspace's active
            // skills + soul + context into this CLI's native form. Best-effort —
            // the hook logs and swallows its own errors, never blocking spawn.
            //
            // Skipped for PR-review sessions: they all share one repo cwd, so
            // concurrent spawns would serialize on this *synchronous* materialize
            // (leaving one agent stuck "pending"); a focused diff review needs no
            // workspace skills/soul; and provisioning also pollutes the repo with
            // .otto-managed.json / CLAUDE.md.
            let is_review = session.meta.get("source").and_then(|v| v.as_str()) == Some("review");
            if !is_review {
                if let Some(hook) = &self.pre_spawn_hook {
                    // Materialize the workspace context into its out-of-tree
                    // bundle and append the launch flags/env that load it
                    // (--add-dir / --append-system-prompt-file / codex
                    // developer_instructions). Nothing is written into the cwd.
                    // Materialize is synchronous disk churn (skill dir copies)
                    // — run it on the blocking pool so it can't stall an async
                    // worker (part of the intermittent create-session 2-3s
                    // latency; see also the PtyHandle spawn below).
                    let hook = Arc::clone(hook);
                    let ws_owned = ws.clone();
                    let cwd = session.cwd.clone();
                    let prov = session.provider.clone();
                    let injection = tokio::task::spawn_blocking(move || {
                        hook.before_spawn(&ws_owned, &cwd, &prov)
                    })
                    .await
                    .unwrap_or_default();
                    spec.args.extend(injection.args);
                    spec.env.extend(injection.env);
                }
            }
            // Wire this session's injected hooks back to the daemon: the
            // provisioner wrote a hooks config that reads these env vars and
            // posts trail/task activity to the per-session ingest endpoint.
            spec.env.extend(self.ingest_env(&session.id));
        }

        // Restore the saved grid from `pty_cols` / `pty_rows` in the session's
        // metadata (written by `resize()`). Falls back to 80×24 when absent or
        // out-of-range so the very first spawn still gets a sane default.
        let saved_cols = session
            .meta
            .get("pty_cols")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16);
        let saved_rows = session
            .meta
            .get("pty_rows")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16);
        let (grid_cols, grid_rows) = resolve_grid(saved_cols, saved_rows);

        // OS-level confinement (opt-in via the `process_sandbox` setting), applied
        // as the very last step before spawn so it wraps the fully-injected spec.
        self.apply_sandbox(&mut spec, &session).await;

        // fork/exec of the agent CLI is a synchronous syscall path that can take
        // tens-hundreds of ms — keep it off the async workers (blocked workers
        // with idle CPU are exactly the intermittent everything-is-slow shape).
        let spawn_spec = spec.clone();
        let handle = match tokio::task::spawn_blocking(move || {
            PtyHandle::spawn_sized(&spawn_spec, grid_cols, grid_rows)
        })
        .await
        .unwrap_or_else(|e| Err(Error::Internal(format!("pty spawn task: {e}"))))
        {
            Ok(h) => Arc::new(h),
            Err(e) => {
                let _ = self.repo.delete(&session.id).await;
                return Err(e);
            }
        };

        self.live.insert(session.id.clone(), Arc::clone(&handle));
        self.start_status_task(
            session.id.clone(),
            session.workspace_id.clone(),
            session.provider.clone(),
            handle,
        );
        // Providers that mint their own session id (codex): capture it from the
        // on-disk rollout now that the CLI is running, so the session becomes
        // resumable across daemon restarts (like claude's `--session-id`).
        if session.kind == SessionKind::Agent
            && session.provider_session_id.is_none()
            && self.providers.captures_session_id(&session.provider)
        {
            self.spawn_session_id_capture(&session);
        }
        let _ = self.events.send(Event::SessionCreated {
            session: session.clone(),
        });
        self.record_lifecycle(&session, format!("Session started · {}", session.provider));
        Ok(session)
    }

    /// Spawn a background task that captures a self-id-minting provider's own
    /// session id from disk and records it as the `provider_session_id`, making
    /// the session resumable after a daemon restart. Handles both providers that
    /// mint their own id: codex (`codex resume <uuid>`, scanned from its rollout)
    /// and agy (`agy --conversation <uuid>`, read from its `last_conversations`
    /// cache).
    ///
    /// Runs under `codex_capture_lock` so two such sessions launched in the same
    /// cwd claim DISTINCT ids (each persists its claim before the next runs).
    /// Best-effort: if no matching session appears within the window the session
    /// simply stays non-resumable (safe — we never guess and resume the wrong
    /// conversation). Never blocks the spawn.
    fn spawn_session_id_capture(&self, session: &Session) {
        let repo = self.repo.clone();
        let lock = Arc::clone(&self.codex_capture_lock);
        let id = session.id.clone();
        // codex/agy record a symlink-RESOLVED cwd (macOS `/var` → `/private/var`,
        // `/tmp` → `/private/tmp`) in their session files; the scans below compare
        // by exact string, so match the canonical form or a session launched from
        // a symlinked path never captures its id and silently stays non-resumable.
        let cwd = std::fs::canonicalize(&session.cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| session.cwd.clone());
        let provider = session.provider.clone();
        // Captured at the spawn moment so we match THIS launch's session, not a
        // later concurrent same-cwd spawn's (whose file mtime is newer).
        let spawn_time = std::time::SystemTime::now();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            let claimed = repo.provider_session_ids().await.unwrap_or_default();
            let captured = match provider.as_str() {
                "codex" => {
                    capture_codex_session_id(&codex_sessions_root(), &cwd, spawn_time, &claimed)
                        .await
                }
                "agy" => capture_agy_session_id(&agy_cli_root(), &cwd, spawn_time, &claimed).await,
                _ => None,
            };
            match captured {
                Some(psid) => match repo.set_provider_session(&id, &psid).await {
                    Ok(()) => tracing::info!(
                        session = %id, provider = %provider, provider_session = %psid,
                        "captured provider session id — session is now resumable"
                    ),
                    Err(e) => {
                        tracing::warn!(session = %id, "provider id capture: persist failed: {e}")
                    }
                },
                None => tracing::warn!(
                    session = %id, provider = %provider,
                    "provider id capture: no matching session found; won't auto-resume"
                ),
            }
        });
    }

    /// Pick a unique display name for a new agent session from the creating
    /// user's active name theme. Returns `None` (caller uses "{provider} #N")
    /// when the themes store is absent or the active theme is the "none"
    /// sentinel. The name is unique among the workspace's OPEN (non-archived)
    /// agent sessions, so addressing it by name is unambiguous.
    async fn allocate_session_name(
        &self,
        ws_id: &Id,
        user_id: &Id,
        _provider: &str,
    ) -> Option<crate::names::Allocated> {
        let repo = self.name_themes.as_ref()?;
        let active = repo
            .active(user_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_NAME_THEME.to_string());
        if active == crate::names::THEME_NONE {
            return None;
        }
        let used: std::collections::HashSet<String> = self
            .repo
            .list_by_workspace(ws_id)
            .await
            .ok()?
            .into_iter()
            .filter(|s| !s.archived && s.kind == SessionKind::Agent)
            .map(|s| s.title.to_lowercase())
            .collect();
        if crate::names::is_builtin(&active) {
            crate::names::allocate_builtin(&active, &used)
        } else {
            // A custom theme id: load the user's list; if it vanished, fall back
            // to the default builtin rather than the bare numbering.
            match repo.get(&active).await {
                Ok(theme) => Some(crate::names::allocate_custom(&theme.names, &used)),
                Err(_) => crate::names::allocate_builtin(DEFAULT_NAME_THEME, &used),
            }
        }
    }

    /// Load one session from the DB.
    pub async fn get(&self, id: &Id) -> Result<Session> {
        self.repo.get(id).await
    }

    /// All sessions of a workspace from the DB.
    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<Session>> {
        self.repo.list_by_workspace(ws).await
    }

    /// Sessions of a workspace owned by `user_id` — the owner-scoped variant
    /// used to list only the caller's own sessions for non-admins (#L1).
    pub async fn list_by_workspace_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Session>> {
        self.repo.list_by_workspace_for_user(ws, user_id).await
    }

    /// True when the session has a live PTY in this daemon process.
    pub fn is_live(&self, id: &Id) -> bool {
        self.live.contains_key(id)
    }

    /// Register a WS terminal viewer for `id` (called on attach). Returns an
    /// [`AttachGuard`] that decrements the count on drop, so every WS exit path
    /// (clean close, error, drop) releases the attachment. Each guard carries a
    /// daemon-unique connection id used by the SIZE-AUTHORITY policy below.
    pub fn attach(self: &Arc<Self>, id: &Id) -> AttachGuard {
        static CONN_SEQ: AtomicU64 = AtomicU64::new(1);
        let conn_id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
        *self.attached.entry(id.clone()).or_insert(0) += 1;
        self.attached_conns
            .entry(id.clone())
            .or_default()
            .push(conn_id);
        AttachGuard {
            manager: Arc::clone(self),
            id: id.clone(),
            conn_id,
        }
    }

    /// Decrement the attached-viewer count for `id`, removing the entry at zero.
    fn detach(&self, id: &Id, conn_id: u64) {
        if let Some(mut e) = self.attached.get_mut(id) {
            *e = e.saturating_sub(1);
            if *e == 0 {
                drop(e);
                self.attached.remove_if(id, |_, &v| v == 0);
            }
        }
        if let Some(mut conns) = self.attached_conns.get_mut(id) {
            conns.retain(|&c| c != conn_id);
            if conns.is_empty() {
                drop(conns);
                self.attached_conns.remove_if(id, |_, v| v.is_empty());
            }
        }
        // A departing size owner releases authority (next typer claims it).
        self.size_owner.remove_if(id, |_, &owner| owner == conn_id);
    }

    /// SIZE AUTHORITY: the connection that most recently TYPED into a session
    /// owns its PTY size while attached. Multiple viewers share one PTY
    /// (open pane + tiled-overview tile + a phone/share tab), each fitting to
    /// its own box — last-writer-wins let a passive small viewer pin a wide
    /// desktop pane's TUI to ~80 cols (observed live: working sessions stuck
    /// at 80×48/60×47 while their pane was 150 cols). Typing claims the size;
    /// resizes from non-owners are ignored while the owner stays attached.
    pub fn note_input_authority(&self, id: &Id, conn_id: u64) {
        self.size_owner.insert(id.clone(), conn_id);
    }

    /// Whether `conn_id` may resize `id` under the size-authority policy:
    /// yes when it IS the owner, when nobody has typed yet, or when the owner
    /// is no longer attached (stale entry — first resizer wins again).
    pub fn may_resize(&self, id: &Id, conn_id: u64) -> bool {
        match self.size_owner.get(id).map(|e| *e) {
            None => true,
            Some(owner) if owner == conn_id => true,
            Some(owner) => !self
                .attached_conns
                .get(id)
                .is_some_and(|conns| conns.contains(&owner)),
        }
    }

    /// Number of WS terminal viewers currently attached to `id`.
    pub fn attached_count(&self, id: &Id) -> usize {
        self.attached.get(id).map(|e| *e).unwrap_or(0)
    }

    /// True when at least one WS viewer is attached to `id`.
    pub fn is_attached(&self, id: &Id) -> bool {
        self.attached_count(id) > 0
    }

    /// Subscribe to the per-session forced-disconnect signal, lazily creating
    /// the broadcast sender for `id` if it doesn't exist yet (mirrors how the
    /// `attached` map's entry is created on demand). The attached `/ws/term`
    /// loop selects on the returned receiver; on [`Self::evict`] it sends a
    /// `{"type":"terminated"}` frame and closes the socket. Capacity is tiny —
    /// the channel only ever carries unit signals.
    pub fn evict_signal(&self, id: &Id) -> broadcast::Receiver<()> {
        self.evict
            .entry(id.clone())
            .or_insert_with(|| broadcast::channel(8).0)
            .subscribe()
    }

    /// Fire the forced-disconnect signal for `id`: every attached viewer that
    /// subscribed via [`Self::evict_signal`] is dropped. A no-op when no sender
    /// exists (no one ever subscribed); the "no receivers" send error is ignored
    /// (all viewers already detached). Used by admin terminate (Task 4.2) and
    /// mobile share-link revoke to kick live `/ws/term` viewers immediately.
    pub fn evict(&self, id: &Id) {
        if let Some(tx) = self.evict.get(id) {
            // Err means no live receivers — harmless, nothing to evict.
            let _ = tx.send(());
        }
    }

    /// Ensure the session is live, resuming it if it is an exited-but-resumable
    /// agent session. A no-op when the session is already live or cannot be
    /// resumed. Errors are logged and suppressed so callers (WS attach) can
    /// proceed optimistically.
    pub async fn ensure_live(&self, id: &Id) -> Result<()> {
        if self.is_live(id) {
            return Ok(());
        }
        let mut session = self.repo.get(id).await?;
        // Second-chance provider-id capture. A codex/agy session whose
        // spawn-time capture timed out (slow first rollout write) carries no
        // provider_session_id and would dead-end below: reconnectable in the
        // UI but impossible to reopen — a live-looking session nobody can
        // reach. Its rollout usually exists on disk by now, so rescan with
        // this session's creation as the floor and claim it late; resume then
        // works as if the spawn-time capture had succeeded.
        if session.kind == SessionKind::Agent
            && session.provider_session_id.is_none()
            && self.providers.captures_session_id(&session.provider)
        {
            if let Some(psid) = self.late_capture_provider_id(&session).await {
                match self.repo.set_provider_session(id, &psid).await {
                    Ok(()) => {
                        tracing::info!(
                            session = %id, provider = %session.provider, provider_session = %psid,
                            "late provider id capture — session is now resumable"
                        );
                        session.provider_session_id = Some(psid);
                    }
                    Err(e) => {
                        tracing::warn!(session = %id, "late provider id capture: persist failed: {e}")
                    }
                }
            }
        }
        let resumable = session.kind == SessionKind::Agent
            && session.provider_session_id.is_some()
            && self.providers.supports_resume(&session.provider);
        if resumable {
            self.restart(id, None).await.map(|_| ())?;
        }
        Ok(())
    }

    /// One synchronous scan for a non-resumable codex/agy session's on-disk
    /// id, run at reopen time instead of spawn time. Same matching rules as
    /// [`Self::spawn_session_id_capture`] (canonical cwd, oldest UNCLAIMED
    /// match at/after the floor, serialized by `codex_capture_lock`) — only
    /// the floor differs: the session's `created_at`, since the spawn moment
    /// is long gone. No polling: by reopen time the file either exists or
    /// never will.
    async fn late_capture_provider_id(&self, session: &Session) -> Option<String> {
        let _guard = self.codex_capture_lock.lock().await;
        let claimed_rows = self.repo.provider_session_ids().await.unwrap_or_default();
        let claimed: std::collections::HashSet<&str> =
            claimed_rows.iter().map(String::as_str).collect();
        let cwd = std::fs::canonicalize(&session.cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| session.cwd.clone());
        let floor = std::time::SystemTime::from(session.created_at)
            .checked_sub(Duration::from_secs(2))
            .unwrap_or(std::time::UNIX_EPOCH);
        match session.provider.as_str() {
            "codex" => scan_codex_rollout(&codex_sessions_root(), &cwd, floor, &claimed),
            "agy" => scan_agy_conversation(&agy_cli_root(), &cwd, floor, &claimed),
            _ => None,
        }
    }

    /// The live PTY handle, when the session has one in this daemon.
    pub fn live_handle(&self, id: &Id) -> Option<Arc<PtyHandle>> {
        self.live.get(id).map(|h| Arc::clone(&h))
    }

    /// Write input bytes to a live session.
    pub async fn input(&self, id: &Id, data: &[u8]) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        handle.write(data)
    }

    /// Resize a live session's terminal.
    pub async fn resize(&self, id: &Id, cols: u16, rows: u16) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        // Same-size resizes are a no-op end to end — no SIGWINCH, no emulator
        // rewrap, no meta write — so clients can re-push their grid freely.
        if handle.size() == (cols, rows) {
            return Ok(());
        }
        handle.resize(cols, rows)?;
        // Persist the last known grid size so resume/reconnect can restore it
        // (prevents reflow flash on reconnect). Best-effort — no await. Uses the
        // atomic merge (single UPDATE): the old read-modify-write raced
        // update_meta and could revert a concurrent keep-alive/issue toggle
        // with its stale snapshot.
        let repo = self.repo.clone();
        let sid = id.clone();
        tokio::spawn(async move {
            let patch = serde_json::json!({ "pty_cols": cols, "pty_rows": rows });
            let _ = repo.merge_meta(&sid, &patch).await;
        });
        Ok(())
    }

    /// Rename a session (the user-driven `PATCH /sessions/{id}` path). Marks the
    /// title **user-owned** (`meta.title_source = "user"`) so the provider-title
    /// auto-namer never overwrites it again, and broadcasts `SessionRenamed` so
    /// every connected client updates in place.
    pub async fn update_title(&self, id: &Id, title: &str) -> Result<Session> {
        self.repo.set_title(id, title).await?;
        let _ = self
            .repo
            .merge_meta(id, &serde_json::json!({ "title_source": "user" }))
            .await;
        // The user owns the name from now on — retire the session from the
        // auto-namer without touching disk again.
        self.title_probe.entry(id.clone()).or_default().resolved = true;
        let session = self.repo.get(id).await?;
        let _ = self.events.send(Event::SessionRenamed {
            session_id: session.id.clone(),
            workspace_id: session.workspace_id.clone(),
            title: session.title.clone(),
        });
        Ok(session)
    }

    /// Shallow-merge `patch` (a JSON object) into the session's existing meta.
    /// Top-level null values in the patch remove that key. Non-object existing
    /// meta is replaced by an empty object before merging.
    ///
    /// Uses the repo's atomic `merge_meta` (single UPDATE via `json_patch`) so a
    /// concurrent writer (e.g. the resize persister) can never be overwritten by
    /// a stale snapshot. `json_patch` deep-merges OBJECT values, but this API's
    /// contract is shallow-replace — so object-valued keys are nulled first,
    /// then set (two atomic merges; the key is briefly absent, never stale).
    pub async fn update_meta(&self, id: &Id, patch: serde_json::Value) -> Result<Session> {
        // Verify the session exists up-front (preserves the NotFound error path).
        let _ = self.repo.get(id).await?;
        if let serde_json::Value::Object(ref patch_map) = patch {
            let nulls: serde_json::Map<String, serde_json::Value> = patch_map
                .iter()
                .filter(|(_, v)| v.is_object())
                .map(|(k, _)| (k.clone(), serde_json::Value::Null))
                .collect();
            if !nulls.is_empty() {
                self.repo
                    .merge_meta(id, &serde_json::Value::Object(nulls))
                    .await?;
            }
            self.repo.merge_meta(id, &patch).await?;
        }
        let updated = self.repo.get(id).await?;
        let _ = self.events.send(Event::SessionMetaUpdated {
            session_id: updated.id.clone(),
            workspace_id: updated.workspace_id.clone(),
            meta: updated.meta.clone(),
        });
        Ok(updated)
    }

    /// Kill the PTY (if live) and mark the session exited.
    pub async fn kill_session(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some(handle) = self.live_handle(id) {
            let _ = handle.kill();
        }
        self.repo.update_status(id, SessionStatus::Exited).await?;
        self.record_lifecycle(&session, "Killed");
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Exited,
        });
        Ok(())
    }

    /// Suspend a session: release its RAM-holding PTY **without losing the
    /// session**. The conversation stays resumable, so reopening it (WS attach
    /// → `ensure_live` → `restart --resume`) brings it right back.
    ///
    /// Only meaningful for resumable agent sessions; callers (the idle-suspend
    /// sweep) gate on `supports_resume`. The row is kept (incl.
    /// `provider_session_id`); the session ends up `Reconnectable`.
    ///
    /// Status-race handling: killing the handle makes the per-session status
    /// task's exit branch fire, which would normally write `Exited`. We mark
    /// the id in `suspending` *before* killing so that branch writes
    /// `Reconnectable` instead. We also set `Reconnectable` here directly, so
    /// the final status is correct regardless of which path wins.
    pub async fn suspend(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        // Mark as suspending so the status task's exit branch chooses
        // Reconnectable over Exited. Cleared by that branch (or below).
        self.suspending.insert(id.clone(), ());
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        // Authoritatively set Reconnectable (idempotent with the status task).
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        // Drop the suspend flag last; the status task only reads it, and a late
        // read after this point is harmless (it would also pick Reconnectable).
        self.suspending.remove(id);
        self.record_lifecycle(&session, "Suspended (idle — freed memory, still resumable)");
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Reconnectable,
        });
        Ok(())
    }

    /// Best-effort "still working" probe: true when the session's DESCENDANT
    /// process tree accrued meaningful CPU across a short sample — i.e. a
    /// command (build, tests, deploy) is running under the agent even though
    /// the PTY is quiet and the status shows Idle. Same descendants-only rule
    /// as the idle-suspend sweep (the agent TUI's own redraws don't count).
    /// Used to suppress the "awaiting input" notice while work is in flight.
    /// A dead/unknown session (or an empty `ps`) reads as inactive.
    pub async fn tree_active(&self, id: &Id) -> bool {
        let Some(pid) = self.live.get(id).and_then(|e| e.value().pid()) else {
            return false;
        };
        let before = descendant_cpu_ms(pid, &process_table());
        tokio::time::sleep(Duration::from_millis(750)).await;
        let after = descendant_cpu_ms(pid, &process_table());
        // ≥30ms over 750ms ≈ a real job burning CPU; idle MCP helpers accrue ~0.
        after > before.saturating_add(30)
    }

    /// Best-effort path of the provider's on-disk ACTIVITY ARTIFACT for a
    /// session — the file that grows while the agent works: claude's transcript
    /// JSONL, codex's rollout JSONL, agy's conversation db. Callers use its
    /// mtime as a truthful progress clock (PTY output lies for agent TUIs: an
    /// idle spinner repaints forever). `None` when the provider has no artifact
    /// (shell/custom/grok), the provider session id isn't captured yet
    /// (codex/agy mint it a few seconds post-spawn), or the file doesn't exist
    /// yet — callers keep their PTY-clock fallback and re-ask later, and must
    /// never read a missing artifact as "no progress".
    pub async fn activity_artifact(&self, id: &Id) -> Option<std::path::PathBuf> {
        let session = self.repo.get(id).await.ok()?;
        let psid = session.provider_session_id.clone()?;
        let path = match session.provider.as_str() {
            // `~/.claude/projects/<enc(cwd)>/<psid>.jsonl`. claude symlink-
            // resolves the spawn cwd for its transcript dir, so canonicalize
            // before encoding (same rule as otto-server's transcript polling;
            // encoding mirrors otto_orchestrator::claude_pty::project_dir).
            "claude" => {
                let cwd = std::fs::canonicalize(&session.cwd)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| session.cwd.clone());
                let enc: String = cwd
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                    .collect();
                std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".claude")
                    .join("projects")
                    .join(enc)
                    .join(format!("{psid}.jsonl"))
            }
            // `<sessions root>/YYYY/MM/DD/rollout-<ts>-<psid>.jsonl`, found by
            // filename suffix. The rollout's mtime only moves forward, so the
            // session-creation floor keeps the walk cheap even with history.
            "codex" => {
                let floor = std::time::SystemTime::from(session.created_at)
                    .checked_sub(Duration::from_secs(2))
                    .unwrap_or(std::time::UNIX_EPOCH);
                let suffix = format!("-{psid}.jsonl");
                recent_codex_rollouts(&codex_sessions_root(), floor)
                    .into_iter()
                    .find(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.ends_with(&suffix))
                    })?
            }
            // `conversations/<psid>.db|.pb` — the most recently touched of the
            // two (agy writes whichever format it's on; either grows per turn).
            "agy" => {
                let dir = agy_cli_root().join("conversations");
                ["db", "pb"]
                    .iter()
                    .filter_map(|ext| {
                        let p = dir.join(format!("{psid}.{ext}"));
                        let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
                        Some((mtime, p))
                    })
                    .max_by_key(|(m, _)| *m)
                    .map(|(_, p)| p)?
            }
            _ => return None,
        };
        std::fs::metadata(&path).is_ok().then_some(path)
    }

    /// One sweep of the provider-title auto-namer. For every LIVE foreground
    /// agent session whose name the user hasn't claimed, read the provider's own
    /// session title (claude/codex first user prompt) and, when found and
    /// different, rename the session to it and broadcast `SessionRenamed`.
    ///
    /// Cheap + robust: dead/archived/background/user-named sessions are skipped
    /// without any disk work; an unchanged transcript (same mtime) is not
    /// re-parsed; a resolved session is never read again; and every per-session
    /// error is swallowed. All file reads run on the blocking pool. Returns the
    /// number of sessions renamed this sweep. Driven by a ~20s loop in ottod.
    pub async fn refresh_provider_titles(&self) -> usize {
        let Ok(sessions) = self.repo.list_all().await else {
            return 0;
        };
        let mut renamed = 0;
        for session in sessions {
            if !title_eligible(&session) {
                continue;
            }
            match self.probe_and_apply_title(&session).await {
                Ok(true) => renamed += 1,
                Ok(false) => {}
                Err(e) => tracing::debug!(session = %session.id, "provider-title probe: {e}"),
            }
        }
        renamed
    }

    /// Probe one session's transcript and adopt its provider title when found.
    /// Returns `Ok(true)` when the session was actually renamed. Uses the
    /// in-memory [`TitleProbe`] cache to short-circuit resolved sessions and
    /// unchanged transcripts; does the file read on the blocking pool.
    async fn probe_and_apply_title(&self, s: &Session) -> Result<bool> {
        // Already resolved in this daemon's lifetime — never re-read.
        if self.title_probe.get(&s.id).is_some_and(|p| p.resolved) {
            return Ok(false);
        }
        let Some(path) = self.activity_artifact(&s.id).await else {
            // Transcript not on disk yet (id just captured, first turn pending).
            return Ok(false);
        };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        // Nothing new since the last look — skip the re-parse.
        if let Some(prev) = self.title_probe.get(&s.id) {
            if mtime.is_some() && prev.mtime == mtime {
                return Ok(false);
            }
        }
        let provider = s.provider.clone();
        let path_owned = path.clone();
        let title = tokio::task::spawn_blocking(move || read_provider_title(&provider, &path_owned))
            .await
            .ok()
            .flatten();
        // Record the mtime we just parsed at so the next sweep can skip it.
        {
            let mut probe = self.title_probe.entry(s.id.clone()).or_default();
            probe.mtime = mtime;
            // A found title means the (stable) first prompt exists — done either
            // way, whether it differs from the current name or already matches.
            if title.is_some() {
                probe.resolved = true;
            }
        }
        let Some(title) = title else {
            return Ok(false); // no user prompt yet — retry on a later sweep
        };
        if title == s.title {
            return Ok(false);
        }
        self.repo.set_title(&s.id, &title).await?;
        let _ = self
            .repo
            .merge_meta(&s.id, &serde_json::json!({ "title_source": "provider" }))
            .await;
        let _ = self.events.send(Event::SessionRenamed {
            session_id: s.id.clone(),
            workspace_id: s.workspace_id.clone(),
            title: title.clone(),
        });
        self.record_lifecycle(s, format!("Auto-named from {} session", s.provider));
        Ok(true)
    }

    /// One sweep of the idle-suspend policy: suspend every LIVE session that is
    /// resumable, idle (no output for ≥ [`SUSPEND_GRACE`]) and has no attached
    /// WS viewer. Working sessions, attached sessions and non-resumable
    /// providers are never touched. Returns the number suspended.
    ///
    /// Resilient: a failure on one session is logged and skipped; the loop
    /// never panics or aborts.
    pub async fn suspend_idle_unattached(&self) -> usize {
        // Read the configurable grace period from settings; fall back to the
        // compiled-in default when not set or when the key is absent.
        let grace = if let Some(ref sr) = self.settings {
            match sr.get("idle_suspend_grace_secs").await {
                Ok(Some(v)) => v.as_u64().map(Duration::from_secs).unwrap_or(SUSPEND_GRACE),
                _ => SUSPEND_GRACE,
            }
        } else {
            SUSPEND_GRACE
        };

        // Snapshot live ids first (don't hold DashMap refs across awaits).
        let candidates: Vec<(Id, std::time::Instant, Option<u32>)> = self
            .live
            .iter()
            .map(|e| (e.key().clone(), e.value().last_output_at(), e.value().pid()))
            .collect();

        // One process-table pass per sweep. "No PTY output" alone is NOT
        // idleness — an agent running a long quiet command (test suite, build)
        // looks silent while its child process tree is hard at work; killing it
        // mid-run loses the in-flight command. We compare each candidate's
        // DESCENDANT-tree cumulative CPU against the previous sweep: accruing
        // CPU ⇒ active ⇒ skip. Descendants only (not the agent CLI itself, whose
        // idle TUI redraws accrue CPU forever) — long-lived idle helpers (MCP
        // servers) accrue ~none, so genuinely idle sessions still suspend.
        let proc_table = process_table();

        let mut suspended = 0;
        for (id, last_output, pid) in candidates {
            // Idle: no PTY output for the full grace window.
            if last_output.elapsed() < grace {
                continue;
            }
            // Unattached: nobody is watching the terminal right now.
            if self.is_attached(&id) {
                continue;
            }
            // Working-but-quiet guard (see the sweep comment above).
            if let Some(pid) = pid {
                let cpu = descendant_cpu_ms(pid, &proc_table);
                let prev = self.suspend_cpu.insert(id.clone(), cpu);
                match prev {
                    // Tree accrued >200ms CPU since the last sweep → in-flight work.
                    Some(prev_cpu) if cpu > prev_cpu.saturating_add(200) => {
                        tracing::debug!(
                            session = %id,
                            "idle-suspend: descendants accrued CPU ({prev_cpu}→{cpu}ms) — skipping"
                        );
                        continue;
                    }
                    Some(_) => {}
                    // No baseline yet and descendants exist: measure this sweep,
                    // decide on the next one (60s later).
                    None if cpu > 0 => {
                        tracing::debug!(session = %id, "idle-suspend: baselining descendant CPU — skipping");
                        continue;
                    }
                    None => {}
                }
            }
            let session = match self.repo.get(&id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(session = %id, "idle-suspend: load failed: {e}");
                    continue;
                }
            };
            // Per-session keep-alive: never auto-suspend sessions pinned by the user.
            if session
                .meta
                .get("keep_alive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            // Only resumable agent sessions — never lose work for a provider
            // that can't be resumed (shell, or a self-id provider whose id we
            // never captured). claude/codex/agy all qualify once resumable.
            let resumable = session.kind == SessionKind::Agent
                && session.provider_session_id.is_some()
                && self.providers.supports_resume(&session.provider);
            if !resumable {
                continue;
            }
            match self.suspend(&id).await {
                Ok(()) => {
                    suspended += 1;
                    self.suspend_cpu.remove(&id);
                    tracing::info!(
                        session = %id,
                        provider = %session.provider,
                        title = %session.title,
                        "suspended idle, unattached session (freed PTY; stays resumable)"
                    );
                }
                Err(e) => tracing::warn!(session = %id, "idle-suspend failed: {e}"),
            }
        }
        suspended
    }

    /// Existence-check pruner: for non-live agent sessions with a
    /// `provider_session_id`, verify the provider's on-disk transcript still
    /// exists. If it is positively gone (un-resumable) → delete the row. If it
    /// still exists, or existence cannot be determined → keep the session.
    ///
    /// **Foreground (Agents-tab) sessions are exempt**: the user's own
    /// sessions are durable — they stay listed until explicitly archived or
    /// deleted, even after the provider CLI cleans its transcript (claude's
    /// `cleanupPeriodDays`). Only background/automation sessions (see
    /// [`otto_core::domain::BACKGROUND_SESSION_SOURCES`]) are pruned; they
    /// arrive at volume (tickets, review agents, workflow steps) and going
    /// stale is their normal end state.
    ///
    /// We only ever delete what we can positively confirm is gone. `$HOME`
    /// locates the transcripts; when unset we keep everything.
    ///
    /// Resilient: per-session failures are logged and skipped. Returns the
    /// number of rows pruned.
    pub async fn prune_dead_sessions(&self) -> usize {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
            _ => {
                tracing::warn!("prune: HOME unset; skipping existence-check prune");
                return 0;
            }
        };
        self.prune_dead_sessions_with_home(&home).await
    }

    /// [`Self::prune_dead_sessions`] with an explicit home dir (test seam).
    pub async fn prune_dead_sessions_with_home(&self, home: &std::path::Path) -> usize {
        let candidates = match self.repo.list_prunable_agent_sessions().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("prune: list prunable sessions failed: {e}");
                return 0;
            }
        };
        let mut pruned = 0;
        for s in candidates {
            // Foreground (Agents-tab) sessions are durable — never auto-delete
            // them, whatever the transcript says (see the method doc).
            if s.is_foreground_agent() {
                continue;
            }
            // Never prune a session that became live again between the query
            // and now (e.g. someone reopened it).
            if self.is_live(&s.id) {
                continue;
            }
            let Some(psid) = s.provider_session_id.as_deref() else {
                continue;
            };
            let verdict = crate::lifecycle::check_resumability(home, &s.provider, &s.cwd, psid);
            match verdict {
                crate::lifecycle::Resumability::Gone => match self.remove(&s.id).await {
                    Ok(()) => {
                        pruned += 1;
                        tracing::info!(
                            session = %s.id,
                            provider = %s.provider,
                            title = %s.title,
                            "pruned un-resumable session (provider transcript gone)"
                        );
                    }
                    Err(e) => tracing::warn!(session = %s.id, "prune remove failed: {e}"),
                },
                // Exists or Unknown → keep. We never prune what we can't verify.
                crate::lifecycle::Resumability::Exists
                | crate::lifecycle::Resumability::Unknown => {}
            }
        }
        pruned
    }

    /// Kill every live PTY and mark the sessions exited. Used when the app
    /// closes (no orphaned agent processes left running) and on daemon
    /// shutdown. Returns the number of sessions terminated.
    pub async fn shutdown_all(&self) -> usize {
        let ids: Vec<Id> = self.live.iter().map(|e| e.key().clone()).collect();
        let count = ids.len();
        for id in ids {
            if let Some((_, handle)) = self.live.remove(&id) {
                let _ = handle.kill();
            }
            // Best-effort status update; ignore errors during shutdown.
            let _ = self.repo.update_status(&id, SessionStatus::Exited).await;
            if let Ok(s) = self.repo.get(&id).await {
                let _ = self.events.send(Event::SessionStatus {
                    session_id: id.clone(),
                    workspace_id: s.workspace_id,
                    status: SessionStatus::Exited,
                });
            }
        }
        count
    }

    /// Number of live (running) sessions.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Archive a session: kill its PTY, mark it archived + exited, keep the
    /// row and history. It disappears from the active list (clients hide it)
    /// but can be restored or deleted later.
    pub async fn archive(&self, id: &Id) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.set_archived(id, true).await?;
        self.repo.update_status(id, SessionStatus::Exited).await?;
        self.record_lifecycle(&session, "Archived");
        // Clients refresh on this event and move the row to the archive.
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Exited,
        });
        self.repo.get(id).await
    }

    /// Auto-archive channel-spawned agent sessions (ticket/chat) idle longer
    /// than `max_idle`, so they don't pile up. A later message in the same
    /// conversation spawns a fresh session. Returns the number archived.
    pub async fn reap_idle_channel_sessions(&self, max_idle: std::time::Duration) -> usize {
        let stale = match self.repo.list_idle_channel_sessions(max_idle).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("reap idle channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            match self.archive(&s.id).await {
                Ok(_) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "reaped idle channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "reap archive failed: {e}"),
            }
        }
        n
    }

    /// Permanently delete archived channel (ticket/chat) sessions whose last
    /// activity is older than `max_age`, so closed tickets don't accumulate in
    /// the DB forever. Uses [`remove`](Self::remove) so clients drop the row
    /// from the Archived view. Returns the number deleted.
    pub async fn purge_old_archived_channel_sessions(&self, max_age: std::time::Duration) -> usize {
        let stale = match self
            .repo
            .list_archived_channel_sessions_older_than(max_age)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("purge old archived channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            // Skip anything that became live again between the query and now.
            if self.live.contains_key(&s.id) {
                continue;
            }
            match self.remove(&s.id).await {
                Ok(()) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "purged old archived channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "purge delete failed: {e}"),
            }
        }
        n
    }

    /// Un-archive a session (it returns to the active list as reconnectable;
    /// agent sessions can then be restarted to resume).
    pub async fn unarchive(&self, id: &Id) -> Result<Session> {
        self.repo.set_archived(id, false).await?;
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        let session = self.repo.get(id).await?;
        self.record_lifecycle(&session, "Unarchived");
        Ok(session)
    }

    /// Kill the PTY, delete the DB row and emit `SessionRemoved`.
    pub async fn remove(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.delete(id).await?;
        self.ingest_tokens.remove(id);
        self.title_probe.remove(id);
        // Revoke the per-session token minted for the `otto` MCP tool server, so
        // its read-only credential dies with the session (best-effort).
        self.revoke_mcp_token(&session.created_by, id).await;
        // Drop the per-session disconnect sender; any attached viewers were
        // already evicted by the terminate path before removal.
        self.evict.remove(id);
        let _ = self.events.send(Event::SessionRemoved {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
        });
        Ok(())
    }

    /// Respawn a session. Agent sessions with a `provider_session_id` use the
    /// provider's resume args. Connection sessions need a `spec_override`
    /// (rebuilt by the connections service) — without one this fails.
    pub async fn restart(&self, id: &Id, spec_override: Option<CommandSpec>) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }

        let mut spec = match spec_override {
            Some(s) => s,
            None => {
                if session.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are reopened via their connection".into(),
                    ));
                }
                let resume = session.provider_session_id.is_some()
                    && self.providers.supports_resume(&session.provider);
                let sid = session.provider_session_id.clone().unwrap_or_else(new_id);
                let mut spec =
                    self.providers
                        .build_spec(&session.provider, &sid, &session.cwd, resume)?;
                // Append --add-dir and --model args from session.meta.
                spec.args
                    .extend(add_dir_args(&session.provider, &session.meta));
                spec.args
                    .extend(model_args(&session.provider, &session.meta));
                // Re-apply the out-of-tree context injection so a resumed session
                // keeps its bundle (no Workspace here — the bundle persists and is
                // read back). Mirrors the create() path's `before_spawn`.
                if let Some(hook) = &self.pre_spawn_hook {
                    let injection = hook.resume_injection(&session.cwd, &session.provider);
                    spec.args.extend(injection.args);
                    spec.env.extend(injection.env);
                }
                spec
            }
        };

        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
            let otto_tools = self.maybe_enable_otto_tools(&session).await;
            spec.args.extend(otto_tools.args);
            spec.env.extend(otto_tools.env);
            // Re-wire the per-session ingest env (the hooks config persists in
            // the workspace from the initial spawn).
            spec.env.extend(self.ingest_env(&session.id));
        }
        // Restore the saved grid — the client will confirm its own size via a
        // Resize frame on connect, but we want the PTY and emulator to agree
        // with what the user last had so the first snapshot is correctly framed.
        let saved_cols = session
            .meta
            .get("pty_cols")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16);
        let saved_rows = session
            .meta
            .get("pty_rows")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16);
        let (grid_cols, grid_rows) = resolve_grid(saved_cols, saved_rows);
        // OS-level confinement on resume too (mirrors create()).
        self.apply_sandbox(&mut spec, &session).await;
        // Blocking-pool fork/exec, mirroring create(): idle-resume runs on the
        // terminal-attach path, so a blocked async worker here is user-visible.
        let spawn_spec = spec.clone();
        let handle = Arc::new(
            tokio::task::spawn_blocking(move || {
                PtyHandle::spawn_sized(&spawn_spec, grid_cols, grid_rows)
            })
            .await
            .unwrap_or_else(|e| Err(Error::Internal(format!("pty spawn task: {e}"))))?,
        );
        self.live.insert(id.clone(), Arc::clone(&handle));
        self.repo.update_status(id, SessionStatus::Running).await?;
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Running,
        });
        self.record_lifecycle(&session, "Session resumed");
        self.start_status_task(id.clone(), session.workspace_id, session.provider, handle);
        self.repo.get(id).await
    }

    /// Daemon-boot restore. We deliberately do NOT respawn agent processes here:
    /// keeping every historical session resident would cost ~200 MB each. Instead
    /// every restorable session is marked `Reconnectable` (0 memory) and resumed
    /// lazily by [`Self::ensure_live`] the moment a client opens it — claude/codex
    /// keep their conversation in the on-disk JSONL, so `--resume` restores it in
    /// full. `_fallback_cwd` is kept for signature stability (used by resume).
    pub async fn restore_all(
        &self,
        _fallback_cwd: &(dyn Fn(&Id) -> Option<String> + Send + Sync),
    ) -> Result<()> {
        for session in self.repo.list_all_restorable().await? {
            self.repo
                .update_status(&session.id, SessionStatus::Reconnectable)
                .await?;
            let _ = self.events.send(Event::SessionStatus {
                session_id: session.id.clone(),
                workspace_id: session.workspace_id.clone(),
                status: SessionStatus::Reconnectable,
            });
        }
        Ok(())
    }

    /// Per-session status task: every 2s classify working/idle from PTY
    /// activity; on exit mark `exited` and stop. When an [`OutputScanner`] is
    /// configured, also spawns a sibling task that streams the PTY's live
    /// output into the scanner (mid-session re-auth detection).
    fn start_status_task(
        &self,
        id: Id,
        workspace_id: Id,
        provider: String,
        handle: Arc<PtyHandle>,
    ) {
        // Mid-session output scan: subscribe to the PTY broadcast and forward
        // chunks to the scanner. Ends when the PTY closes (broadcast Closed).
        if let Some(scanner) = self.output_scanner.clone() {
            let mut rx = handle.subscribe();
            let scan_id = id.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(chunk) => scanner.on_output(&scan_id, &provider, &chunk),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        let repo = self.repo.clone();
        let events = self.events.clone();
        let live = Arc::clone(&self.live);
        let suspending = Arc::clone(&self.suspending);
        tokio::spawn(async move {
            let mut exit_rx = handle.on_exit();
            let mut current = SessionStatus::Running;
            let mut interval = tokio::time::interval(STATUS_TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let next = if handle.last_output_at().elapsed() < WORKING_WINDOW {
                            SessionStatus::Working
                        } else {
                            SessionStatus::Idle
                        };
                        if next != current {
                            current = next;
                            let _ = repo.update_status(&id, next).await;
                            let _ = events.send(Event::SessionStatus {
                                session_id: id.clone(),
                                workspace_id: workspace_id.clone(),
                                status: next,
                            });
                        }
                    }
                    code = wait_exit_code(&mut exit_rx) => {
                        let _ = code;
                        // Evict the dead handle so its emulator + ring buffer
                        // are dropped (no accumulation across many sessions) —
                        // but ONLY if `live` still maps THIS handle. A respawn
                        // (restart/ensure_live) kills this PTY and inserts a
                        // fresh handle under the same id; without the identity
                        // check, this superseded task's exit would evict the new
                        // handle, orphaning its process (alive but untracked, so
                        // suspend/archive never kill it). See `evict_if_same`.
                        evict_if_same(&live, &id, &handle);
                        // If this exit was caused by a deliberate suspend (PTY
                        // killed to free RAM), the session stays resumable: mark
                        // it Reconnectable, not Exited. `suspend()` also writes
                        // Reconnectable authoritatively, so either order is safe.
                        let status = if suspending.contains_key(&id) {
                            SessionStatus::Reconnectable
                        } else {
                            SessionStatus::Exited
                        };
                        let _ = repo.update_status(&id, status).await;
                        let _ = events.send(Event::SessionStatus {
                            session_id: id.clone(),
                            workspace_id: workspace_id.clone(),
                            status,
                        });
                        break;
                    }
                }
            }
        });
    }
}

/// Remove `id`'s entry from `live` **only if it still maps `handle`** (pointer
/// identity). Returns true iff it removed.
///
/// This is the fix for the orphaned-process leak. The per-session status task
/// calls it when its child exits. Removing unconditionally was the bug: a
/// respawn (`restart` / `ensure_live`) kills the old PTY and inserts a NEW
/// handle under the same id; the OLD handle's status task, woken by that kill,
/// could run AFTER the new insert and evict the fresh handle — leaving the new
/// process alive but no longer in `live`, so `suspend` / `archive` (which only
/// kill what is in `live`) never terminated it. The identity check makes a
/// superseded handle's exit a no-op against a replacement entry.
fn evict_if_same(live: &DashMap<Id, Arc<PtyHandle>>, id: &Id, handle: &Arc<PtyHandle>) -> bool {
    live.remove_if(id, |_, h| Arc::ptr_eq(h, handle)).is_some()
}

/// Decide whether a session should be sandboxed and with what network posture,
/// from the `process_sandbox` setting JSON. `None` means "do not sandbox". Pure
/// (no I/O) so the gating is unit-testable: only `Agent` sessions whose provider
/// is in the configured set (default: all agent providers) when `enabled`.
fn sandbox_decision(
    cfg: &serde_json::Value,
    kind: SessionKind,
    provider: &str,
) -> Option<otto_sandbox::NetworkPolicy> {
    if kind != SessionKind::Agent {
        return None;
    }
    if !cfg
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let providers: Vec<String> = cfg
        .get("providers")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            ["claude", "codex", "agy", "shell"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    if !providers.iter().any(|p| p == provider) {
        return None;
    }
    Some(
        match cfg
            .get("network")
            .and_then(|v| v.as_str())
            .unwrap_or("full")
        {
            "none" => otto_sandbox::NetworkPolicy::None,
            "loopback" => otto_sandbox::NetworkPolicy::LoopbackOnly,
            _ => otto_sandbox::NetworkPolicy::Full,
        },
    )
}

/// Resolve a repo's git **common dir** (absolute, canonicalized) for `cwd`, so
/// the sandbox can grant write access to it. For a linked worktree this is the
/// main repo's `.git` (which holds the objects + the worktree's gitdir), which
/// lives OUTSIDE `cwd` — without it a sandboxed agent in a worktree couldn't
/// commit. Best-effort: `None` when `cwd` isn't a git repo.
async fn resolve_git_common_dir(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--git-common-dir"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let p = std::path::Path::new(&s);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    Some(std::fs::canonicalize(&abs).unwrap_or(abs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{SessionKind, Workspace};
    use otto_state::NewSession;

    #[test]
    fn codex_creds_preserve_session_source_for_mcp_policy() {
        let session_id = new_id();
        let path = write_codex_creds(
            &session_id,
            "token",
            "http://127.0.0.1:7700",
            "workspace",
            Some("vault-docs-review"),
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["source"], serde_json::json!("vault-docs-review"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn env_based_mcp_providers_preserve_reviewer_source() {
        let mut session = Session {
            id: new_id(),
            workspace_id: new_id(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "reviewer".into(),
            status: SessionStatus::Idle,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: new_id(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: serde_json::json!({"source": "vault-docs-review"}),
        };
        for provider in ["claude", "agy", "grok"] {
            session.provider = provider.into();
            let env = otto_tools_env(&session, "token", "http://127.0.0.1:7700");
            assert_eq!(
                env.get("OTTO_SESSION_SOURCE").map(String::as_str),
                Some("vault-docs-review"),
                "provider {provider} lost the reviewer MCP policy source"
            );
        }
        session.meta = serde_json::json!({"source": "vault-docs"});
        assert_eq!(
            otto_tools_env(&session, "token", "base")
                .get("OTTO_SESSION_SOURCE")
                .map(String::as_str),
            Some("vault-docs")
        );
    }

    fn agent_session(provider: &str, status: SessionStatus, meta: serde_json::Value) -> Session {
        Session {
            id: new_id(),
            workspace_id: new_id(),
            kind: SessionKind::Agent,
            provider: provider.into(),
            title: "Messi".into(),
            status,
            cwd: "/tmp".into(),
            provider_session_id: Some("019f-abc".into()),
            connection_id: None,
            created_by: new_id(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta,
        }
    }

    #[test]
    fn clean_provider_title_strips_newlines_and_clips() {
        // Newlines/tabs collapse to single spaces; leading/trailing trimmed.
        assert_eq!(
            clean_provider_title("  fix the\n\tpty  redraw  bug\n"),
            Some("fix the pty redraw bug".into())
        );
        // Blank / whitespace-only yields None.
        assert_eq!(clean_provider_title("   \n\t "), None);
        assert_eq!(clean_provider_title(""), None);
        // Clipped to 60 chars + ellipsis, with no dangling space before it.
        let long = "a".repeat(200);
        let out = clean_provider_title(&long).unwrap();
        assert_eq!(out.chars().count(), PROVIDER_TITLE_MAX + 1);
        assert!(out.ends_with('…'));
        // A word boundary landing on the clip point doesn't leave "word …".
        let sentence = format!("{} zzzz", "word ".repeat(20));
        let clipped = clean_provider_title(&sentence).unwrap();
        assert!(!clipped.contains(" …"), "no space before ellipsis: {clipped:?}");
    }

    #[test]
    fn parse_claude_first_prompt_skips_meta_and_wrappers() {
        // Meta line, a slash-command wrapper, a tool_result-only turn, then the
        // real prompt (string content). The first genuine user turn wins.
        let jsonl = concat!(
            r#"{"type":"summary"}"#,
            "\n",
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<system reminder>"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":"i have an issue\nwith the pty session"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":"a later prompt"}}"#,
        );
        assert_eq!(
            parse_claude_first_prompt(jsonl),
            Some("i have an issue with the pty session".into())
        );
    }

    #[test]
    fn parse_claude_first_prompt_reads_array_text_parts() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"refactor the parser"}]}}"#;
        assert_eq!(
            parse_claude_first_prompt(jsonl),
            Some("refactor the parser".into())
        );
    }

    #[test]
    fn parse_claude_first_prompt_none_when_no_user_turn() {
        let jsonl = r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#;
        assert_eq!(parse_claude_first_prompt(jsonl), None);
    }

    #[test]
    fn parse_codex_first_prompt_prefers_user_message_event() {
        // session_meta, the injected AGENTS.md as a response_item user turn, then
        // the real typed prompt as an event_msg/user_message. The event wins.
        let jsonl = concat!(
            r#"{"type":"session_meta","payload":{"id":"019f","cwd":"/x"}}"#,
            "\n",
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions\nblah"}]}}"##,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"scan the repo into the vault"}}"#,
        );
        assert_eq!(
            parse_codex_first_prompt(jsonl),
            Some("scan the repo into the vault".into())
        );
    }

    #[test]
    fn parse_codex_first_prompt_falls_back_to_input_text() {
        // No user_message event (older rollout): the first non-wrapper user
        // input_text is used, and the AGENTS.md wrapper before it is skipped.
        let jsonl = concat!(
            r#"{"type":"session_meta","payload":{"id":"019f","cwd":"/x"}}"#,
            "\n",
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions"}]}}"##,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"add a retry to the client"}]}}"#,
        );
        assert_eq!(
            parse_codex_first_prompt(jsonl),
            Some("add a retry to the client".into())
        );
    }

    #[test]
    fn title_eligible_guards_ownership_and_lifecycle() {
        // Theme-named live claude session → eligible.
        assert!(title_eligible(&agent_session(
            "claude",
            SessionStatus::Idle,
            serde_json::json!({ "title_source": "theme" }),
        )));
        // "auto" (#N fallback) and absent title_source are also eligible.
        assert!(title_eligible(&agent_session(
            "codex",
            SessionStatus::Working,
            serde_json::json!({ "title_source": "auto" }),
        )));
        assert!(title_eligible(&agent_session(
            "claude",
            SessionStatus::Running,
            serde_json::json!({}),
        )));
        // User-owned → never touched.
        assert!(!title_eligible(&agent_session(
            "claude",
            SessionStatus::Idle,
            serde_json::json!({ "title_source": "user" }),
        )));
        // Already provider-named → nothing new to read.
        assert!(!title_eligible(&agent_session(
            "claude",
            SessionStatus::Idle,
            serde_json::json!({ "title_source": "provider" }),
        )));
        // Exited → skipped (a dead session gains no new prompt).
        assert!(!title_eligible(&agent_session(
            "claude",
            SessionStatus::Exited,
            serde_json::json!({ "title_source": "theme" }),
        )));
        // Provider without a human title source (shell/agy) → skipped.
        assert!(!title_eligible(&agent_session(
            "agy",
            SessionStatus::Idle,
            serde_json::json!({ "title_source": "theme" }),
        )));
        // Background (engine-owned) session → skipped.
        assert!(!title_eligible(&agent_session(
            "claude",
            SessionStatus::Idle,
            serde_json::json!({ "source": "review", "title_source": "theme" }),
        )));
        // No captured provider session id yet → skipped.
        let mut no_psid = agent_session("claude", SessionStatus::Idle, serde_json::json!({}));
        no_psid.provider_session_id = None;
        assert!(!title_eligible(&no_psid));
    }

    #[test]
    fn ps_time_parses_all_shapes() {
        assert_eq!(parse_ps_time_ms("0:00.05"), Some(50));
        assert_eq!(parse_ps_time_ms("1:30.00"), Some(90_000));
        assert_eq!(
            parse_ps_time_ms("2:03:04"),
            Some(2 * 3_600_000 + 3 * 60_000 + 4_000)
        );
        assert_eq!(parse_ps_time_ms("1-00:00:01"), Some(86_400_000 + 1_000));
        assert_eq!(parse_ps_time_ms("garbage"), None);
    }

    #[test]
    fn descendant_cpu_excludes_the_root_and_walks_depth() {
        // 100 (root, 9999ms) → 200 (10ms) → 300 (500ms); 400 unrelated.
        let table = vec![
            (100, 1, 9_999),
            (200, 100, 10),
            (300, 200, 500),
            (400, 1, 777),
        ];
        assert_eq!(descendant_cpu_ms(100, &table), 510);
        assert_eq!(descendant_cpu_ms(400, &table), 0);
        // A (bogus) cyclic table must not hang.
        let cyclic = vec![(2, 1, 5), (1, 2, 7)];
        assert_eq!(descendant_cpu_ms(1, &cyclic), 12);
    }

    async fn test_manager() -> (Arc<SessionManager>, SessionsRepo, Workspace, Id) {
        // A migrated on-disk sqlite (in a tempdir) via otto-state's opener.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let pool = otto_state::open(&db).await.unwrap();
        // Keep the tempdir alive for the whole process (leak is fine in tests).
        std::mem::forget(dir);

        let user = new_id();
        let ws_id = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&user).bind("u").bind("x").bind("U").bind(&now)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws_id)
            .bind("w")
            .bind("/tmp")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let repo = SessionsRepo::new(pool);
        let (events, _rx) = broadcast::channel(16);
        let providers = ProviderRegistry::new(None);
        let mgr = Arc::new(SessionManager::new(repo.clone(), events, providers));
        let ws = Workspace {
            id: ws_id,
            name: "w".into(),
            root_path: "/tmp".into(),
            settings: serde_json::json!({}),
            archived: false,
            created_at: chrono::Utc::now(),
        };
        (mgr, repo, ws, user)
    }

    /// Write a codex rollout file with a `session_meta` first line, returning its
    /// path. `thread_source`/`originator` let tests forge subagent / non-codex rows.
    fn write_rollout(
        dir: &std::path::Path,
        name: &str,
        session_id: &str,
        cwd: &str,
        originator: &str,
        thread_source: &str,
    ) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "session_id": session_id,
                "id": session_id,
                "cwd": cwd,
                "originator": originator,
                "thread_source": thread_source,
            }
        });
        std::fs::write(&path, format!("{meta}\n")).unwrap();
        path
    }

    #[test]
    fn codex_rollout_match_filters_to_top_level_in_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let day = tmp.path().join("2026/06/25");
        let top = write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
        let sub = write_rollout(
            &day,
            "b.jsonl",
            "BBB",
            "/work/proj",
            "codex-tui",
            "subagent",
        );
        let other = write_rollout(&day, "c.jsonl", "CCC", "/elsewhere", "codex-tui", "user");

        assert_eq!(
            codex_rollout_match(&top, "/work/proj").map(|(s, _)| s),
            Some("AAA".to_string())
        );
        // Subagent thread and wrong-cwd rollouts are not matched.
        assert_eq!(
            codex_rollout_match(&sub, "/work/proj").map(|(s, _)| s),
            None
        );
        assert_eq!(
            codex_rollout_match(&other, "/work/proj").map(|(s, _)| s),
            None
        );
    }

    #[test]
    fn scan_codex_rollout_picks_unclaimed_top_level_match() {
        use std::collections::HashSet;
        let tmp = tempfile::tempdir().unwrap();
        let day = tmp.path().join("2026/06/25");
        write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
        write_rollout(
            &day,
            "b.jsonl",
            "BBB",
            "/work/proj",
            "codex-tui",
            "subagent",
        );
        write_rollout(&day, "c.jsonl", "CCC", "/elsewhere", "codex-tui", "user");
        let floor = std::time::SystemTime::UNIX_EPOCH;

        // Picks the only top-level rollout for this cwd.
        let none_claimed: HashSet<&str> = HashSet::new();
        assert_eq!(
            scan_codex_rollout(tmp.path(), "/work/proj", floor, &none_claimed),
            Some("AAA".to_string())
        );
        // Once AAA is claimed by another session, there's nothing left to claim.
        let claimed: HashSet<&str> = ["AAA"].into_iter().collect();
        assert_eq!(
            scan_codex_rollout(tmp.path(), "/work/proj", floor, &claimed),
            None
        );
    }

    #[test]
    fn scan_agy_conversation_matches_fresh_unclaimed_cwd() {
        use std::collections::HashSet;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::create_dir_all(root.join("conversations")).unwrap();
        // cwd -> most-recent conversation id (agy's last_conversations cache).
        std::fs::write(
            root.join("cache/last_conversations.json"),
            r#"{"/work/proj":"AAA","/other":"BBB"}"#,
        )
        .unwrap();
        // Only AAA has a conversation file on disk (fresh).
        std::fs::write(root.join("conversations/AAA.db"), b"x").unwrap();
        let floor = std::time::SystemTime::UNIX_EPOCH;
        let none: HashSet<&str> = HashSet::new();

        // The cwd maps to AAA and its file is fresh + unclaimed → captured.
        assert_eq!(
            scan_agy_conversation(root, "/work/proj", floor, &none),
            Some("AAA".to_string())
        );
        // /other maps to BBB but there is no conversation file → not captured.
        assert_eq!(scan_agy_conversation(root, "/other", floor, &none), None);
        // Already claimed by another session → skip.
        let claimed: HashSet<&str> = ["AAA"].into_iter().collect();
        assert_eq!(
            scan_agy_conversation(root, "/work/proj", floor, &claimed),
            None
        );
        // Unknown cwd → nothing.
        assert_eq!(scan_agy_conversation(root, "/nope", floor, &none), None);
    }

    async fn seed_session(
        repo: &SessionsRepo,
        ws: &Workspace,
        user: &Id,
        psid: Option<&str>,
    ) -> Id {
        let s = repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "t".into(),
                cwd: "/tmp".into(),
                provider_session_id: psid.map(|s| s.to_string()),
                connection_id: None,
                created_by: user.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        s.id
    }

    /// Root-cause regression for the orphaned-process leak: a superseded handle's
    /// status-task exit must NOT evict a freshly-respawned handle from `live`.
    /// With the old unconditional `live.remove(id)` the replacement was evicted
    /// (untracked → never killed by suspend/archive → orphan). `evict_if_same`
    /// makes the stale exit a no-op against the replacement, while the
    /// replacement's own exit still evicts it.
    #[tokio::test]
    async fn evict_if_same_ignores_superseded_handle() {
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            cwd: None,
            env: vec![],
        };
        let live: DashMap<Id, Arc<PtyHandle>> = DashMap::new();
        let id: Id = "S1".into();
        let h1 = Arc::new(PtyHandle::spawn(&spec).expect("spawn h1")); // old, superseded
        let h2 = Arc::new(PtyHandle::spawn(&spec).expect("spawn h2")); // the respawn

        // After a restart, `live` maps the NEW handle.
        live.insert(id.clone(), Arc::clone(&h2));

        // The OLD handle's status task fires on its exit → must be a no-op here.
        assert!(
            !evict_if_same(&live, &id, &h1),
            "a superseded handle must not remove the replacement entry"
        );
        assert!(live.contains_key(&id), "replacement must stay tracked");
        assert!(Arc::ptr_eq(live.get(&id).unwrap().value(), &h2));

        // The replacement's OWN exit does evict it.
        assert!(evict_if_same(&live, &id, &h2));
        assert!(!live.contains_key(&id));

        // Drop kills both children (RAII); no orphan left behind.
    }

    #[tokio::test]
    async fn attach_guard_counts_and_releases() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;

        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));

        let g1 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        let g2 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 2);

        drop(g1);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        drop(g2);
        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));
    }

    #[tokio::test]
    async fn size_authority_typing_owner_blocks_passive_resizers() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid-size")).await;

        let pane = mgr.attach(&id); // the user's open pane
        let tile = mgr.attach(&id); // a passive tiled-overview tile

        // Nobody typed yet: first-come resize is allowed for both.
        assert!(mgr.may_resize(&id, pane.conn_id()));
        assert!(mgr.may_resize(&id, tile.conn_id()));

        // Typing in the pane claims authority; the tile may no longer resize.
        mgr.note_input_authority(&id, pane.conn_id());
        assert!(mgr.may_resize(&id, pane.conn_id()));
        assert!(!mgr.may_resize(&id, tile.conn_id()));

        // A focus claim moves authority (e.g. user clicks the other viewer).
        mgr.note_input_authority(&id, tile.conn_id());
        assert!(!mgr.may_resize(&id, pane.conn_id()));
        assert!(mgr.may_resize(&id, tile.conn_id()));

        // Owner detaching releases authority — survivors may resize again.
        drop(tile);
        assert!(mgr.may_resize(&id, pane.conn_id()));
    }

    #[tokio::test]
    async fn suspend_marks_reconnectable_and_keeps_row() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid-keep")).await;

        // No live PTY in this test; suspend still drives the DB-side outcome.
        mgr.suspend(&id).await.unwrap();

        let s = repo.get(&id).await.unwrap();
        assert_eq!(s.status, SessionStatus::Reconnectable);
        // The session is NOT lost — row and resume id are preserved.
        assert_eq!(s.provider_session_id.as_deref(), Some("sid-keep"));
        // Suspend flag is cleared after the operation.
        assert!(!mgr.suspending.contains_key(&id));
    }

    /// Root-cause regression for the dead-on-reopen codex session
    /// (2026-07-16): a session whose spawn-time id capture timed out (codex
    /// wrote its rollout ~20s after launch, past the old 12s window) stayed
    /// non-resumable forever, and `ensure_live` silently no-opped — the
    /// session showed in the UI but could never be reopened. The late capture
    /// must claim the on-disk rollout born after the session's creation, and
    /// must NOT steal an id another session already claimed.
    #[tokio::test]
    async fn late_capture_claims_rollout_written_after_spawn_window() {
        let (mgr, repo, ws, user) = test_manager().await;
        let cwd_dir = tempfile::tempdir().unwrap();
        // Canonical form up front: the capture compares cwd by exact string.
        let cwd = cwd_dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let new_codex = |title: &str| NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "codex".into(),
            title: title.into(),
            cwd: cwd.clone(),
            provider_session_id: None,
            connection_id: None,
            created_by: user.clone(),
            meta: serde_json::json!({}),
        };
        let s = repo.create(new_codex("t")).await.unwrap();

        // The rollout lands only now — after `created_at`, i.e. after the
        // spawn-time window would have expired. CODEX_HOME points the scan at
        // it (safe here: every other test passes its scan root explicitly).
        let codex_home = tempfile::tempdir().unwrap();
        let day = codex_home.path().join("sessions").join("2026/07/16");
        write_rollout(&day, "r.jsonl", "LATE-1", &cwd, "codex-tui", "user");
        std::env::set_var("CODEX_HOME", codex_home.path());

        assert_eq!(
            mgr.late_capture_provider_id(&s).await.as_deref(),
            Some("LATE-1"),
            "late capture must find the rollout the spawn-time window missed"
        );

        // Claim it for `s`; a second non-resumable session in the same cwd
        // must not capture the same conversation.
        repo.set_provider_session(&s.id, "LATE-1").await.unwrap();
        let s2 = repo.create(new_codex("t2")).await.unwrap();
        assert_eq!(
            mgr.late_capture_provider_id(&s2).await,
            None,
            "a claimed rollout id must never be re-claimed by another session"
        );
    }

    #[tokio::test]
    async fn idle_suspend_skips_attached_sessions() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;
        // Mark it as a (fake) live session so the sweep considers it, but with
        // an attachment so it must be skipped. We can't spawn a real PTY here,
        // so assert the guard semantics the sweep relies on: an attached
        // session reports is_attached == true.
        let _g = mgr.attach(&id);
        assert!(mgr.is_attached(&id));
        // The sweep over live sessions is a no-op (no live PTYs), and the
        // attachment registry it consults is correct.
        assert_eq!(mgr.suspend_idle_unattached().await, 0);
    }

    #[tokio::test]
    async fn evict_signal_fires_to_subscribers() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;

        // Two attached viewers each subscribe to the per-session disconnect
        // signal (broadcast so every viewer is dropped, not just one).
        let mut rx1 = mgr.evict_signal(&id);
        let mut rx2 = mgr.evict_signal(&id);

        // Nothing fired yet.
        assert!(rx1.try_recv().is_err());

        // Firing the signal yields a unit to every subscriber.
        mgr.evict(&id);
        assert!(rx1.recv().await.is_ok(), "subscriber 1 must receive evict");
        assert!(rx2.recv().await.is_ok(), "subscriber 2 must receive evict");
    }

    #[tokio::test]
    async fn evict_without_subscribers_is_noop() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, None).await;
        // No receivers exist; evict must not panic or error (no-op).
        mgr.evict(&id);
        // A subscriber created afterwards does not see the earlier (lost) send.
        let mut rx = mgr.evict_signal(&id);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn prune_keeps_session_with_existing_transcript() {
        let (mgr, repo, ws, user) = test_manager().await;
        // Point HOME at a tempdir holding a matching transcript.
        let home = tempfile::tempdir().unwrap();
        let cwd = "/tmp";
        let psid = "exists-1111";
        let proj = home
            .path()
            .join(".claude")
            .join("projects")
            .join(crate::lifecycle::claude_project_dir_name(cwd));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{psid}.jsonl")), b"{}").unwrap();

        let id = seed_session(&repo, &ws, &user, Some(psid)).await;
        repo.update_status(&id, SessionStatus::Reconnectable)
            .await
            .unwrap();

        // Safe in tests: this process owns its env. Set HOME for the check.
        std::env::set_var("HOME", home.path());
        let pruned = mgr.prune_dead_sessions().await;
        assert_eq!(pruned, 0, "existing transcript must be kept");
        assert!(repo.get(&id).await.is_ok());
    }

    // ── Grid-size resolution tests ───────────────────────────────────────────

    /// `resolve_grid` returns the clamped values when both fall in-range.
    #[test]
    fn resolve_grid_in_range() {
        let (c, r) = resolve_grid(Some(132), Some(50));
        assert_eq!(c, 132);
        assert_eq!(r, 50);
    }

    /// Out-of-range cols fall back to the default; rows are accepted when valid.
    #[test]
    fn resolve_grid_cols_out_of_range_falls_back() {
        // cols = 0 is below MIN_COLS (20) → default 80
        let (c, r) = resolve_grid(Some(0), Some(40));
        assert_eq!(c, otto_pty::DEFAULT_COLS, "zero cols should yield default");
        assert_eq!(r, 40);

        // cols = 501 is above MAX_COLS (500) → default 80
        let (c, _) = resolve_grid(Some(501), Some(24));
        assert_eq!(
            c,
            otto_pty::DEFAULT_COLS,
            "oversized cols should yield default"
        );
    }

    /// Rows out-of-range fall back to the default.
    #[test]
    fn resolve_grid_rows_out_of_range_falls_back() {
        let (_, r) = resolve_grid(Some(80), Some(1));
        assert_eq!(
            r,
            otto_pty::DEFAULT_ROWS,
            "rows below MIN_ROWS should yield default"
        );

        let (_, r) = resolve_grid(Some(80), Some(201));
        assert_eq!(
            r,
            otto_pty::DEFAULT_ROWS,
            "rows above MAX_ROWS should yield default"
        );
    }

    /// `None` values yield the defaults.
    #[test]
    fn resolve_grid_none_yields_defaults() {
        let (c, r) = resolve_grid(None, None);
        assert_eq!(c, otto_pty::DEFAULT_COLS);
        assert_eq!(r, otto_pty::DEFAULT_ROWS);
    }

    // ── model_args tests ────────────────────────────────────────────────────

    /// claude with a model set → ["--model", name].
    #[test]
    fn model_args_claude_with_model() {
        let meta = serde_json::json!({ "model": "claude-opus-4-8" });
        let args = model_args("claude", &meta);
        assert_eq!(args, vec!["--model", "claude-opus-4-8"]);
    }

    /// codex also accepts --model.
    #[test]
    fn model_args_codex_with_model() {
        let meta = serde_json::json!({ "model": "gpt-5-codex" });
        let args = model_args("codex", &meta);
        assert_eq!(args, vec!["--model", "gpt-5-codex"]);
    }

    /// agy does not support --model; args must be empty.
    #[test]
    fn model_args_agy_skipped() {
        let meta = serde_json::json!({ "model": "some-model" });
        let args = model_args("agy", &meta);
        assert!(args.is_empty(), "agy has no --model flag");
    }

    /// shell provider → empty regardless of meta.
    #[test]
    fn model_args_shell_skipped() {
        let meta = serde_json::json!({ "model": "some-model" });
        let args = model_args("shell", &meta);
        assert!(args.is_empty(), "shell has no --model flag");
    }

    /// No model in meta → empty vec.
    #[test]
    fn model_args_absent_model_empty() {
        let meta = serde_json::json!({});
        let args = model_args("claude", &meta);
        assert!(args.is_empty(), "no model key should yield no args");
    }

    /// Whitespace-only model is silently skipped.
    #[test]
    fn model_args_blank_model_empty() {
        let meta = serde_json::json!({ "model": "   " });
        let args = model_args("claude", &meta);
        assert!(args.is_empty(), "blank model should yield no args");
    }

    /// Leading/trailing whitespace is trimmed from the model name.
    #[test]
    fn model_args_model_is_trimmed() {
        let meta = serde_json::json!({ "model": "  opus  " });
        let args = model_args("claude", &meta);
        assert_eq!(args, vec!["--model", "opus"]);
    }

    /// `add_dir_args` is provider-agnostic: ANY non-shell provider handed
    /// `extra_dirs` gets `--add-dir`. This is the contract that makes gating a
    /// skill bundle to claude the CALLER's job (see
    /// `otto_server::review_session::review_skills_extra_dirs`) — handing the
    /// bundle to codex here would re-introduce the wrong-skill bug.
    #[test]
    fn add_dir_args_emits_for_any_non_shell_with_extra_dirs() {
        let meta = serde_json::json!({ "extra_dirs": ["/bundle"] });
        assert_eq!(add_dir_args("claude", &meta), vec!["--add-dir", "/bundle"]);
        assert_eq!(add_dir_args("codex", &meta), vec!["--add-dir", "/bundle"]);
        assert_eq!(add_dir_args("agy", &meta), vec!["--add-dir", "/bundle"]);
    }

    /// shell never gets `--add-dir`, and an absent/empty `extra_dirs` yields none.
    #[test]
    fn add_dir_args_empty_for_shell_or_no_dirs() {
        let meta = serde_json::json!({ "extra_dirs": ["/bundle"] });
        assert!(add_dir_args("shell", &meta).is_empty());
        assert!(add_dir_args("codex", &serde_json::json!({})).is_empty());
        // Empty-string entries are skipped.
        let empties = serde_json::json!({ "extra_dirs": ["", " "] });
        assert_eq!(add_dir_args("claude", &empties), vec!["--add-dir", " "]);
    }

    /// A session spawned with saved grid meta reports that size via screen_size().
    #[tokio::test]
    async fn spawn_sized_restores_grid() {
        use otto_pty::{CommandSpec, PtyHandle};
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn_sized(&spec, 132, 50).expect("spawn");
        let (rows, cols) = handle.screen_size();
        assert_eq!(cols, 132, "restored cols");
        assert_eq!(rows, 50, "restored rows");
    }

    #[test]
    fn sandbox_decision_gates_correctly() {
        use otto_sandbox::NetworkPolicy;
        let on = serde_json::json!({"enabled": true, "network": "full"});
        // Agent + a default provider → sandbox with the configured network.
        assert_eq!(
            sandbox_decision(&on, SessionKind::Agent, "claude"),
            Some(NetworkPolicy::Full)
        );
        // Connection sessions are never sandboxed.
        assert_eq!(sandbox_decision(&on, SessionKind::Connection, "ssh"), None);
        // Disabled (or absent) → never.
        assert_eq!(
            sandbox_decision(
                &serde_json::json!({"enabled": false}),
                SessionKind::Agent,
                "claude"
            ),
            None
        );
        assert_eq!(
            sandbox_decision(&serde_json::json!({}), SessionKind::Agent, "claude"),
            None
        );
        // An explicit provider allowlist excludes others.
        let only_codex = serde_json::json!({"enabled": true, "providers": ["codex"]});
        assert_eq!(
            sandbox_decision(&only_codex, SessionKind::Agent, "claude"),
            None
        );
        assert_eq!(
            sandbox_decision(&only_codex, SessionKind::Agent, "codex"),
            Some(NetworkPolicy::Full)
        );
        // Network posture parsing (default = full).
        assert_eq!(
            sandbox_decision(
                &serde_json::json!({"enabled": true, "network": "loopback"}),
                SessionKind::Agent,
                "shell"
            ),
            Some(NetworkPolicy::LoopbackOnly)
        );
        assert_eq!(
            sandbox_decision(
                &serde_json::json!({"enabled": true, "network": "none"}),
                SessionKind::Agent,
                "shell"
            ),
            Some(NetworkPolicy::None)
        );
    }

    /// A FOREGROUND (Agents-tab) session survives the pruner even when its
    /// provider transcript is positively gone — only archive/delete may remove
    /// it. Background sessions keep the existence-check prune.
    #[tokio::test]
    async fn prune_keeps_foreground_sessions_and_prunes_background_ones() {
        let (mgr, repo, ws, user) = test_manager().await;
        let home = tempfile::tempdir().unwrap();

        let mk = |title: &str, psid: &str, meta: serde_json::Value| NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: title.into(),
            cwd: "/tmp/proj".into(),
            provider_session_id: Some(psid.into()),
            connection_id: None,
            created_by: user.clone(),
            meta,
        };

        // Foreground (no meta.source), transcript GONE → must be KEPT.
        let fg = repo
            .create(mk("Messi", "psid-fg", serde_json::json!({})))
            .await
            .unwrap();
        repo.update_status(&fg.id, SessionStatus::Reconnectable)
            .await
            .unwrap();
        // Foreground with an unknown source, transcript GONE → also KEPT.
        let fg2 = repo
            .create(mk(
                "Ronaldo",
                "psid-fg2",
                serde_json::json!({"source": "someday-new"}),
            ))
            .await
            .unwrap();
        repo.update_status(&fg2.id, SessionStatus::Exited)
            .await
            .unwrap();
        // Background (channel ticket), transcript GONE → pruned as before.
        let bg = repo
            .create(mk(
                "ticket",
                "psid-bg",
                serde_json::json!({"source": "channel"}),
            ))
            .await
            .unwrap();
        repo.update_status(&bg.id, SessionStatus::Exited)
            .await
            .unwrap();
        // Background whose transcript still EXISTS → kept (unchanged behavior).
        let bg_live = repo
            .create(mk(
                "review",
                "psid-live",
                serde_json::json!({"source": "review"}),
            ))
            .await
            .unwrap();
        repo.update_status(&bg_live.id, SessionStatus::Exited)
            .await
            .unwrap();
        let proj = home
            .path()
            .join(".claude")
            .join("projects")
            .join("-tmp-proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("psid-live.jsonl"), "{}\n").unwrap();

        let pruned = mgr.prune_dead_sessions_with_home(home.path()).await;
        assert_eq!(pruned, 1, "exactly the gone-transcript background session");
        assert!(
            repo.get(&fg.id).await.is_ok(),
            "foreground survives a gone transcript"
        );
        assert!(
            repo.get(&fg2.id).await.is_ok(),
            "unknown-source foreground survives too"
        );
        assert!(
            repo.get(&bg.id).await.is_err(),
            "channel session with gone transcript is pruned"
        );
        assert!(
            repo.get(&bg_live.id).await.is_ok(),
            "existing transcript keeps a background session"
        );
    }
}

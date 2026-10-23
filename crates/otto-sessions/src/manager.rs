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
use otto_core::hooks::PreSpawnHook;
use otto_core::{Error, Id, Result};
use otto_pty::{resolve_grid, CommandSpec, PtyHandle};
use otto_rbac::AuthRepo;
use otto_state::{ActivityRepo, NewSession, NewTrail, SessionsRepo};
use tokio::sync::{broadcast, Mutex};

use crate::agy::{agy_cli_root, scan_agy_conversation};

use crate::providers::ProviderRegistry;
use crate::titles::{parse_claude_first_prompt, parse_codex_first_prompt};

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

/// Expand a provider's model-flag TEMPLATE (its `ProviderSpec.model_args`,
/// e.g. `["--model","{model}"]` for claude/codex/agy, `["-m","{model}"]` for a
/// custom provider) against `meta.model`. Returns an empty vec when the
/// provider has no template (`None` — shell, template-less custom providers;
/// pickers hide the model control via `/meta.model_flags` so this is never a
/// surprise drop) or when `meta.model` is absent/blank.
fn model_args(template: Option<&[String]>, meta: &serde_json::Value) -> Vec<String> {
    let Some(template) = template else {
        return vec![];
    };
    let Some(model) = meta.get("model").and_then(|v| v.as_str()) else {
        return vec![];
    };
    let model = model.trim();
    if model.is_empty() {
        return vec![];
    }
    template
        .iter()
        .map(|a| a.replace("{model}", model))
        .collect()
}

/// Extra argv for a **lean turn** — a short, mechanical, user-blocking agent
/// turn (drafting a PR title/description, a commit message) whose prompt is
/// entirely self-contained.
///
/// Opted in with `meta.lean_turn = true`. Two costs dominate such a turn and
/// neither buys anything here:
///   * **MCP servers.** Otto injects its own `otto` server into every claude
///     session; `--strict-mcp-config` with no `--mcp-config` loads none at all.
///   * **Tool round trips.** The diff is already in the prompt, so `Bash`,
///     edits and web access can only add latency. Read/Grep/Glob stay allowed
///     as an escape hatch for a truncated diff.
///
/// Claude-only: codex takes neither flag.
fn lean_turn_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if provider != "claude" || meta.get("lean_turn").and_then(|v| v.as_bool()) != Some(true) {
        return vec![];
    }
    vec![
        "--strict-mcp-config".to_string(),
        "--disallowed-tools".to_string(),
        "Bash Edit Write NotebookEdit WebFetch WebSearch Task".to_string(),
    ]
}

/// Extra argv for a **read-only** session (`meta.read_only = true` — a
/// proactive personal-agent run, or a run of a read-only schedule). The CLI's
/// own shell and sub-agents are removed so nothing can run `git push` / `gh` /
/// `curl -X POST`; file edits stay (the agent writes its memory + report) and
/// are confined to its folder by the forced Seatbelt profile
/// ([`SessionManager::apply_sandbox`]). Otto's own tools are refused at the
/// daemon (otto-server `personal_agent_policy`). Claude gets tool flags; codex
/// gets the untrusted-checkout refinements below (and [`read_only_strip`]);
/// other providers rely on the sandbox + the daemon-side policy.
///
/// Review-only refinements (code review runs against PR checkouts whose
/// content an outsider wrote):
/// - `meta.allow_subagents = true` keeps the sub-agent tool (an orchestrator
///   reviewer delegates each lens to one) while the shell stays removed;
/// - `meta.project_settings = false` marks an UNTRUSTED checkout. For claude it
///   adds `--setting-sources user`, so the checkout's `.claude/settings*.json`
///   (hooks, MCP servers, permission rules) never loads — a repo hook would
///   otherwise run on SessionStart — plus `--strict-mcp-config` (no MCP server
///   at all: a reviewer only reads files and writes its findings JSON, and a
///   user-scope MCP is an exfiltration channel) and removes WebFetch /
///   WebSearch (an injected "fetch https://x/?k=<secret>" is the other one).
///   For codex it adds `-c project_doc_max_bytes=0`, so the checkout's
///   AGENTS.md is never loaded as INSTRUCTIONS (it is still readable as code).
fn read_only_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if meta.get("read_only").and_then(|v| v.as_bool()) != Some(true) {
        return vec![];
    }
    let untrusted = meta.get("project_settings").and_then(|v| v.as_bool()) == Some(false);
    match provider {
        "claude" => {
            let subagents = meta.get("allow_subagents").and_then(|v| v.as_bool()) == Some(true);
            let mut denied = vec!["Bash", "NotebookEdit"];
            if !subagents {
                denied.push("Task");
            }
            if untrusted {
                denied.extend(["WebFetch", "WebSearch"]);
            }
            let mut args = vec!["--disallowed-tools".to_string(), denied.join(" ")];
            if untrusted {
                args.push("--setting-sources".to_string());
                args.push("user".to_string());
                args.push("--strict-mcp-config".to_string());
            }
            args
        }
        "codex" if untrusted => vec!["-c".to_string(), "project_doc_max_bytes=0".to_string()],
        _ => vec![],
    }
}

/// Removes provider flags a **read-only** session must not carry, from argv the
/// provider registry built. Codex's `--search` (the model-side live web-search
/// tool) is dropped: a read-only run has no business browsing, and on an
/// untrusted checkout it is an exfiltration channel a prompt injection can
/// steer. (codex keeps its approvals bypass: its own Seatbelt cannot nest
/// inside Otto's forced read-only profile, which confines its whole process
/// tree instead.)
fn read_only_strip(provider: &str, meta: &serde_json::Value, args: &mut Vec<String>) {
    if provider == "codex" && meta.get("read_only").and_then(|v| v.as_bool()) == Some(true) {
        args.retain(|a| a != "--search");
    }
}

/// Per-session creds file for the Codex `otto` MCP server: a daemon-private temp
/// path (`<tmp>/otto-mcp/<session_id>.json`, mode 0600). Holds the per-session
/// token so it never appears on Codex's argv; removed when the session is removed.
fn codex_creds_path(session_id: &Id) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("otto-mcp")
        .join(format!("{session_id}.json"))
}

/// Per-session env file for one of the user's MCP servers on a Codex session
/// (`<tmp>/otto-mcp/<session_id>.<server>.env`, mode 0600): its env values —
/// Keychain secrets included — reach the server through this file instead of
/// codex's argv. Server names are bare TOML keys, so the path segment is safe.
/// Removed with the session's other credentials.
fn codex_server_env_path(session_id: &Id, server: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("otto-mcp")
        .join(format!("{session_id}.{server}.env"))
}

/// Delete every per-server env file written for `session_id`
/// ([`codex_server_env_path`]). Best-effort.
async fn remove_codex_server_env_files(session_id: &Id) {
    let dir = std::env::temp_dir().join("otto-mcp");
    let prefix = format!("{session_id}.");
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".env") {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

#[derive(Default)]
struct OttoToolsInjection {
    args: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
    /// The launcher entry for the workspace `.mcp.json` (identity-neutral —
    /// command/args only end up in the shared file). `None` when the feature
    /// is off for the workspace; the caller folds it into the single per-spawn
    /// MCP reconcile ([`SessionManager::sync_workspace_mcp`]).
    server: Option<crate::mcp::OttoToolsServer>,
}

/// One row of the live process table: (pid, ppid, cumulative CPU ms).
type ProcRow = (u32, u32, u64);

/// `ps` flags for [`process_table`]. On macOS `-S` sums each process's
/// EXITED (reaped) children into it, so a tree's total stays monotonic: without
/// it a tree that churns short processes (test runners, `make`, a shell loop)
/// lost each reaped child's CPU and read as non-increasing — busy background
/// work looked idle to the working-but-quiet guard (review S1-16).
#[cfg(target_os = "macos")]
const PS_ARGS: [&str; 3] = ["-axS", "-o", "pid=,ppid=,time="];
#[cfg(not(target_os = "macos"))]
const PS_ARGS: [&str; 3] = ["-ax", "-o", "pid=,ppid=,time="];

/// Snapshot the OS process table via one `ps` pass ([`PS_ARGS`]).
/// Used by the idle-suspend sweep; a failed/absent `ps` yields an empty table
/// (the sweep then behaves exactly as before the guard existed).
#[allow(clippy::disallowed_methods)] // sync helper: blocking by contract — async callers offload it
fn process_table() -> Vec<ProcRow> {
    let out = match std::process::Command::new("ps").args(PS_ARGS).output() {
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

/// [`process_table`] OFF the runtime. The `ps` spawn + parse blocks for tens of
/// milliseconds (far more under load or an EDR agent scanning every exec), and
/// every caller sits on a tokio worker shared with the terminal sockets — so a
/// sweep would stall whatever keystroke that worker was about to deliver.
/// A join failure yields an empty table, exactly like a failed `ps`.
async fn process_table_async() -> Vec<ProcRow> {
    tokio::task::spawn_blocking(process_table)
        .await
        .unwrap_or_default()
}

/// The process table, taken on FIRST NEED within one sweep (r3-01-09 /
/// r3-08-08): the idle sweep used to `ps` the whole box every minute even with
/// no live session, or none past its grace window.
#[derive(Default)]
struct LazyProcTable(Option<Vec<ProcRow>>);

impl LazyProcTable {
    async fn get(&mut self) -> &[ProcRow] {
        if self.0.is_none() {
            self.0 = Some(process_table_async().await);
        }
        self.0.as_deref().unwrap_or_default()
    }

    #[cfg(test)]
    fn taken(&self) -> bool {
        self.0.is_some()
    }
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

/// Pause between provider-id capture scans, by time since the first input:
/// quick while the CLI is most likely to flush its rollout, then backing off
/// (review S1-28 — a 500 ms rescan for up to 180 s was pure overhead).
fn capture_scan_pause(since_input: Duration) -> Duration {
    if since_input < Duration::from_secs(10) {
        Duration::from_secs(1)
    } else if since_input < Duration::from_secs(60) {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(5)
    }
}

/// The `session_auto_archive_days` cutoff: `now - days`, or `None` when the
/// day count doesn't fit an `i64`, a chrono `TimeDelta`, or the date range
/// (`None` = auto-archive off).
fn auto_archive_cutoff(
    now: chrono::DateTime<chrono::Utc>,
    days: u64,
) -> Option<chrono::DateTime<chrono::Utc>> {
    i64::try_from(days)
        .ok()
        .and_then(chrono::TimeDelta::try_days)
        .and_then(|d| now.checked_sub_signed(d))
}

/// Row-level auto-archive eligibility (`session_auto_archive_days`): an
/// unarchived, unpinned agent session last active before `cutoff`. Liveness
/// and attachment are checked separately (in-memory state).
fn is_auto_archive_candidate(s: &Session, cutoff: chrono::DateTime<chrono::Utc>) -> bool {
    !s.archived
        && s.kind == SessionKind::Agent
        && s.last_active_at < cutoff
        && !s
            .meta
            .get("keep_alive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

/// The descendant-CPU guard's verdict for one session in one sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpuVerdict {
    /// The tree accrued more than [`CPU_GROWTH_MS`] since the last sample:
    /// in-flight work (a quiet build/test run).
    Growing { prev: u64, now: u64 },
    /// No previous sample but descendants exist: decide on the next sweep.
    Baselining,
    /// Nothing accrued (or no descendants): the guard does not hold.
    Idle,
}

/// CPU growth between two sweeps that counts as in-flight work.
const CPU_GROWTH_MS: u64 = 200;

/// Pure descendant-CPU decision for one sample against the previous one.
fn cpu_verdict(prev: Option<u64>, now: u64) -> CpuVerdict {
    match prev {
        Some(prev) if now > prev.saturating_add(CPU_GROWTH_MS) => CpuVerdict::Growing { prev, now },
        Some(_) => CpuVerdict::Idle,
        None if now > 0 => CpuVerdict::Baselining,
        None => CpuVerdict::Idle,
    }
}

/// The CPU verdicts of ONE sweep, shared by the idle pass and the cap pass
/// (review S1-07). Sampling a session updates its `suspend_cpu` baseline, so
/// a second sample in the same sweep — the same lazily-cached process table
/// — always read `prev == now` and voided the guard: the cap pass suspended
/// exactly the sessions the idle pass had just held for "descendant CPU".
#[derive(Default)]
struct SweepCpu {
    table: LazyProcTable,
    verdicts: std::collections::HashMap<Id, CpuVerdict>,
}

impl SweepCpu {
    /// This sweep's verdict for `id` — sampled (and the baseline advanced)
    /// at most once per sweep.
    async fn verdict(&mut self, baselines: &DashMap<Id, u64>, id: &Id, pid: u32) -> CpuVerdict {
        if let Some(v) = self.verdicts.get(id) {
            return *v;
        }
        let now = descendant_cpu_ms(pid, self.table.get().await);
        let v = cpu_verdict(baselines.insert(id.clone(), now), now);
        self.verdicts.insert(id.clone(), v);
        v
    }
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
/// else `~/.codex/sessions` ([`otto_transcript::roots`] owns the rule).
pub fn codex_sessions_root() -> std::path::PathBuf {
    otto_transcript::roots::RootsEnv::from_process()
        .default_codex_root(&otto_transcript::roots::home_dir())
}

/// Locate the rollout file for a codex conversation id: the file named
/// `rollout-<ts>-<psid>.jsonl` somewhere under `YYYY/MM/DD/` (conversation
/// view, design §4.2). A directory walk — callers persist the result on the
/// session row (`sessions.transcript_path`) so this runs once per session.
pub fn codex_rollout_path(psid: &str) -> Option<std::path::PathBuf> {
    codex_rollout_path_under(&codex_sessions_root(), psid)
}

/// [`codex_rollout_path`] under an explicit sessions root.
pub fn codex_rollout_path_under(root: &std::path::Path, psid: &str) -> Option<std::path::PathBuf> {
    #[allow(clippy::disallowed_methods)] // sync helper: blocking by contract — async callers offload it
    fn walk(dir: &std::path::Path, suffix: &str, depth: usize) -> Option<std::path::PathBuf> {
        if depth > 5 {
            return None;
        }
        // Newest date dirs first: the rollout we want is almost always recent.
        let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for entry in entries {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {
                    if let Some(p) = walk(&path, suffix, depth + 1) {
                        return Some(p);
                    }
                }
                Ok(_) => {
                    if path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(suffix))
                    {
                        return Some(path);
                    }
                }
                Err(_) => {}
            }
        }
        None
    }
    if psid.is_empty() {
        return None;
    }
    walk(root, &format!("-{psid}.jsonl"), 0)
}

/// Best-effort: resolve + persist `sessions.transcript_path` right after a
/// provider id is captured (design §4.2), so the conversation view never has to
/// scan `~/.codex/sessions` for this session again. Failures only log.
async fn persist_transcript_path(
    repo: &SessionsRepo,
    id: &Id,
    provider: &str,
    cwd: &str,
    psid: &str,
    provider_home: Option<&std::path::Path>,
) {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    // Transcript resolution walks `~/.codex/sessions` / `~/.claude/projects`
    // with sync fs calls — run it on the blocking pool, not a runtime worker.
    let (provider_s, cwd_s, psid_s) = (provider.to_string(), cwd.to_string(), psid.to_string());
    let provider_home = provider_home.map(std::path::Path::to_path_buf);
    let resolved = match tokio::task::spawn_blocking(move || match provider_home {
        Some(root) => crate::lifecycle::transcript_path_in_roots(
            &root.join("projects"),
            &root.join("sessions"),
            &provider_s,
            &cwd_s,
            Some(&psid_s),
        ),
        None => crate::lifecycle::transcript_path(&home, &provider_s, &cwd_s, Some(&psid_s)),
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(session = %id, "transcript path resolve task failed: {e}");
            return;
        }
    };
    match resolved {
        Ok(path) => {
            if let Err(e) = repo.set_transcript_path(id, &path.to_string_lossy()).await {
                tracing::debug!(session = %id, "transcript path persist failed: {e}");
            }
        }
        Err(why) => tracing::debug!(session = %id, ?why, "transcript path not resolvable yet"),
    }
}

/// Normalize raw PTY input bytes into the plain text codex will record as the
/// session's first `user_message`: strips ESC/CSI sequences (bracketed-paste
/// markers included), applies backspace/DEL editing, folds CR/LF into spaces,
/// collapses whitespace runs, and caps the result. Used to content-match a
/// session's typed prompt against candidate rollouts.
fn normalize_pty_input(bytes: &[u8]) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut i = 0;
    while i < bytes.len() && out.len() < 512 {
        let b = bytes[i];
        match b {
            0x1b => {
                // ESC sequence: CSI (`ESC [ … final 0x40-0x7E`) or a 2-byte one.
                i += 1;
                if bytes.get(i) == Some(&b'[') {
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                }
                i += 1; // consume the final byte (or the single ESC-following char)
                continue;
            }
            0x08 | 0x7f => {
                // Backspace/DEL: the user edited the composed line — mirror it.
                out.pop();
            }
            b'\r' | b'\n' | b'\t' => out.push(' '),
            0x00..=0x1f => {} // other control bytes carry no text
            _ => {
                // Decode the UTF-8 sequence starting here (input is valid UTF-8
                // in practice; a broken byte is simply skipped).
                let len = match b {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    _ => 4,
                };
                if let Ok(s) = std::str::from_utf8(&bytes[i..(i + len).min(bytes.len())]) {
                    out.extend(s.chars());
                }
                i += len;
                continue;
            }
        }
        i += 1;
    }
    // Collapse whitespace runs and trim.
    let mut s = String::with_capacity(out.len());
    let mut in_ws = true; // leading whitespace is dropped
    for c in out {
        if c.is_whitespace() {
            if !in_ws {
                s.push(' ');
            }
            in_ws = true;
        } else {
            s.push(c);
            in_ws = false;
        }
    }
    while s.ends_with(' ') {
        s.pop();
    }
    s
}

/// First `user_message` text recorded in a rollout, if any. codex flushes the
/// rollout lazily — meta line and first user message land together when the
/// first prompt is submitted — so `None` means "no prompt reached this
/// conversation yet".
fn codex_rollout_first_user_message(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(200) {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let p = v.get("payload")?;
        if p.get("type").and_then(|t| t.as_str()) == Some("user_message") {
            return p.get("message").and_then(|m| m.as_str()).map(str::to_owned);
        }
    }
    None
}

/// First input a session's PTY received while its provider-id capture was
/// pending. `first_at` gates and floors the rollout scan; `raw` (capped) is
/// normalized and content-matched against candidate rollouts' first
/// user_message. Registered empty at spawn, fed by [`SessionManager::input`].
#[derive(Default)]
struct CaptureProbe {
    /// Identity of the process generation whose first input this probe records.
    generation: Arc<()>,
    first_at: Option<std::time::SystemTime>,
    raw: Vec<u8>,
}

/// Outcome of one [`pick_codex_rollout`] pass.
#[derive(Debug, PartialEq)]
enum RolloutPick {
    /// Exactly one coherent candidate — claim this provider session id.
    Claim(String),
    /// Multiple candidates and no content evidence to tell them apart. Never
    /// guess: the caller keeps polling (or gives up and leaves the session
    /// non-resumable).
    Ambiguous,
    /// No (acceptable) candidate yet.
    Nothing,
}

/// Minimum normalized characters before typed-prompt content is trusted to
/// confirm or contradict a candidate rollout.
const CAPTURE_PROBE_MIN_CHARS: usize = 16;

/// One scan-and-pick pass over the unclaimed top-level rollouts for `cwd`
/// at/after `floor`.
///
/// `probe` is the normalized text of the first input THIS session's PTY
/// received (None when unknown, or when this is the only in-flight capture for
/// the cwd and content arbitration is unnecessary). Rules:
/// - a candidate whose first `user_message` matches the probe is CONFIRMED —
///   the oldest confirmed candidate wins;
/// - a candidate whose message clearly differs is someone else's conversation
///   and is excluded outright;
/// - with no usable probe, a SOLE candidate is claimed (the common single-spawn
///   case) but multiple candidates are refused as [`RolloutPick::Ambiguous`] —
///   the 2026-07-21 cross-wire incident was the old code claiming the oldest of
///   many and resuming other sessions' conversations.
fn pick_codex_rollout(
    sessions_root: &std::path::Path,
    cwd: &str,
    floor: std::time::SystemTime,
    claimed: &std::collections::HashSet<&str>,
    probe: Option<&str>,
) -> RolloutPick {
    let mut candidates: Vec<(std::time::SystemTime, String, std::path::PathBuf)> = Vec::new();
    for path in recent_codex_rollouts(sessions_root, floor) {
        let Some((session_id, mtime)) = codex_rollout_match(&path, cwd) else {
            continue;
        };
        if claimed.contains(session_id.as_str()) {
            continue;
        }
        candidates.push((mtime, session_id, path));
    }
    if candidates.is_empty() {
        return RolloutPick::Nothing;
    }
    let probe = probe.filter(|p| p.chars().count() >= CAPTURE_PROBE_MIN_CHARS);
    let Some(probe) = probe else {
        return match candidates.len() {
            1 => RolloutPick::Claim(candidates.remove(0).1),
            _ => RolloutPick::Ambiguous,
        };
    };
    // Content arbitration: compare the probe against each candidate's first
    // user_message over their common prefix (bounded, char-safe).
    let mut confirmed: Vec<(std::time::SystemTime, String)> = Vec::new();
    let mut inconclusive: Vec<String> = Vec::new();
    for (mtime, sid, path) in candidates {
        match codex_rollout_first_user_message(&path) {
            Some(msg) => {
                let msg = normalize_pty_input(msg.as_bytes());
                let a: Vec<char> = probe.chars().take(120).collect();
                let b: Vec<char> = msg.chars().take(120).collect();
                let n = a.len().min(b.len());
                if n >= CAPTURE_PROBE_MIN_CHARS && a[..n] == b[..n] {
                    confirmed.push((mtime, sid));
                }
                // else: recorded prompt clearly differs → excluded.
            }
            None => inconclusive.push(sid),
        }
    }
    confirmed.sort_by_key(|(t, _)| *t);
    if let Some((_, sid)) = confirmed.into_iter().next() {
        return RolloutPick::Claim(sid);
    }
    match inconclusive.len() {
        0 => RolloutPick::Nothing,
        // A single not-yet-flushed-message candidate: claim it (transient
        // window; also keeps capture working if codex stops recording
        // user_message events).
        1 => RolloutPick::Claim(inconclusive.remove(0)),
        _ => RolloutPick::Ambiguous,
    }
}

/// True when the rollout file for `psid` is being actively written by some
/// process right now (size/mtime advances across a short settle window).
/// `ensure_live` uses this as a resume-fork guard: `codex resume <psid>` on a
/// conversation that is still live in another PTY forks it — the incident's
/// "duplicated sessions".
async fn rollout_actively_written(
    sessions_root: &std::path::Path,
    psid: &str,
    settle: Duration,
) -> bool {
    #[allow(clippy::disallowed_methods)] // sync helper: blocking by contract — async callers offload it
    fn find(dir: &std::path::Path, psid: &str, depth: usize) -> Option<std::path::PathBuf> {
        if depth > 5 {
            return None;
        }
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {
                    if let Some(p) = find(&path, psid, depth + 1) {
                        return Some(p);
                    }
                }
                Ok(_)
                    if path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains(psid) && n.ends_with(".jsonl")) =>
                {
                    return Some(path);
                }
                _ => {}
            }
        }
        None
    }
    // The recursive walk (up to 5 levels of `~/.codex/sessions`) is sync fs
    // work: keep it off the runtime worker.
    let (root, id) = (sessions_root.to_path_buf(), psid.to_string());
    let Some(path) = tokio::task::spawn_blocking(move || find(&root, &id, 0))
        .await
        .ok()
        .flatten()
    else {
        return false;
    };
    let stat = |p: std::path::PathBuf| async move {
        tokio::fs::metadata(p)
            .await
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    let before = stat(path.clone()).await;
    tokio::time::sleep(settle).await;
    let after = stat(path).await;
    before != after
}

/// Recursively collect `*.jsonl` rollout files under `root` modified at/after
/// `cutoff`. Bounded depth (the layout is `YYYY/MM/DD/`); the cutoff keeps the
/// scan cheap even with a deep history.
fn recent_codex_rollouts(
    root: &std::path::Path,
    cutoff: std::time::SystemTime,
) -> Vec<std::path::PathBuf> {
    #[allow(clippy::disallowed_methods)] // sync helper: blocking by contract — async callers offload it
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

/// The command that puts a reopened `shell` session back into the agent
/// conversation the user had running in it, or `None` when the terminal never
/// had one (a plain shell, which simply respawns empty).
///
/// Per-launch miss backoff for [`SessionManager::capture_nested_agents`]:
/// `(session, nested pid)` → (consecutive misses, next probe at). The first
/// [`NESTED_FREE_MISSES`] misses keep the loop's own cadence (a fresh agent
/// writes its transcript only once prompted); after that the gap doubles from
/// one minute up to [`NESTED_MAX_BACKOFF`].
type NestedMisses = std::collections::HashMap<(Id, u32), (u32, std::time::Instant)>;

const NESTED_FREE_MISSES: u32 = 6;
const NESTED_MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

fn nested_misses() -> &'static std::sync::Mutex<NestedMisses> {
    static M: std::sync::OnceLock<std::sync::Mutex<NestedMisses>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// (session, spawn) → "is an agent-kind `shell`" for the nested-agent sweep.
/// Kind and provider are fixed for a spawn, so each live session costs one row
/// read per launch instead of one per 30 s tick, and a box whose live sessions
/// are all agent CLIs never runs the whole-box `ps` at all (r3-01-09).
type NestedEligible = std::collections::HashMap<(Id, u64), bool>;

fn nested_eligible() -> &'static std::sync::Mutex<NestedEligible> {
    static M: std::sync::OnceLock<std::sync::Mutex<NestedEligible>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

fn nested_probe_due(id: &Id, pid: u32) -> bool {
    let m = nested_misses().lock().unwrap_or_else(|p| p.into_inner());
    m.get(&(id.clone(), pid))
        .is_none_or(|(_, at)| std::time::Instant::now() >= *at)
}

fn nested_probe_missed(id: &Id, pid: u32) {
    let mut m = nested_misses().lock().unwrap_or_else(|p| p.into_inner());
    // A new launch in the same terminal starts from zero.
    m.retain(|(sid, p), _| sid != id || *p == pid);
    let e = m
        .entry((id.clone(), pid))
        .or_insert((0, std::time::Instant::now()));
    e.0 += 1;
    let extra = e.0.saturating_sub(NESTED_FREE_MISSES);
    e.1 = std::time::Instant::now()
        + if extra == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs(60u64.saturating_mul(1 << extra.min(8)) / 2).min(NESTED_MAX_BACKOFF)
        };
}

fn nested_probe_forget(id: &Id) {
    let mut m = nested_misses().lock().unwrap_or_else(|p| p.into_inner());
    m.retain(|(sid, _), _| sid != id);
}

/// Reads what [`SessionManager::capture_nested_agents`] recorded: the provider,
/// its conversation id (stored in the row's `provider_session_id`, exactly like
/// a first-class agent session's) and the directory the agent was launched
/// from — the `cd` is emitted only when that differs from where the shell
/// itself starts, since the transcript is resolved relative to it.
fn nested_resume_command(session: &Session) -> Option<String> {
    let provider = session.meta.get("nested_provider")?.as_str()?;
    let sid = session.provider_session_id.as_deref()?;
    let launched_in = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .filter(|dir| *dir != session.cwd.as_str());
    crate::nested::resume_command(provider, sid, launched_in)
}

/// Ceiling on how many transcript lines the auto-namer parsers scan before
/// giving up on finding a first user prompt — bounds the read on a large
/// transcript that genuinely never carried one (all-system/tool content).
pub(crate) const PROVIDER_TITLE_SCAN_LINES: usize = 4000;

/// Byte cap on the transcript prefix the auto-namer reads (with
/// [`PROVIDER_TITLE_SCAN_LINES`]): agent transcripts grow to hundreds of MB
/// and the sweep runs every 20 s, so only the head is ever read.
const PROVIDER_TITLE_SCAN_BYTES: u64 = 4 * 1024 * 1024;

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
///
/// Reads at most [`PROVIDER_TITLE_SCAN_LINES`] lines / [`PROVIDER_TITLE_SCAN_BYTES`]
/// of the file (lossy UTF-8) — never the whole transcript.
fn read_provider_title(provider: &str, path: &std::path::Path) -> TitleRead {
    let Some((contents, capped)) = read_transcript_head(path) else {
        return TitleRead::default();
    };
    let title = match provider {
        "claude" => parse_claude_first_prompt(&contents),
        "codex" => parse_codex_first_prompt(&contents),
        _ => None,
    };
    TitleRead { title, capped }
}

/// Result of [`read_provider_title`]. `capped` = the scan hit its line/byte cap,
/// i.e. the head the first prompt must live in is complete: no title there
/// means there never will be one, so the session is not probed again.
#[derive(Debug, Default)]
struct TitleRead {
    title: Option<String>,
    capped: bool,
}

/// The first [`PROVIDER_TITLE_SCAN_LINES`] lines of `path`, bounded by
/// [`PROVIDER_TITLE_SCAN_BYTES`], lossily decoded. `.1` is true when either cap
/// was reached. `None` when the file can't be opened.
fn read_transcript_head(path: &std::path::Path) -> Option<(String, bool)> {
    use std::io::{BufRead, Read};
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file.take(PROVIDER_TITLE_SCAN_BYTES));
    let mut head: Vec<u8> = Vec::new();
    let mut lines = 0usize;
    loop {
        if lines >= PROVIDER_TITLE_SCAN_LINES {
            return Some((String::from_utf8_lossy(&head).into_owned(), true));
        }
        match reader.read_until(b'\n', &mut head) {
            Ok(0) => break,
            Ok(_) => lines += 1,
            Err(_) => break,
        }
    }
    let capped = head.len() as u64 >= PROVIDER_TITLE_SCAN_BYTES;
    Some((String::from_utf8_lossy(&head).into_owned(), capped))
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

/// A live terminal nobody has viewed for this long keeps only
/// [`otto_pty::UNVIEWED_SCROLLBACK_LINES`] of emulator history (daemon-core
/// perf F9): with 12 live agents the full 4000-row histories were the
/// biggest steady memory term. The next attach restores the full cap (older
/// rows dropped meanwhile are gone; history regrows from there).
const UNVIEWED_HISTORY_AFTER: Duration = Duration::from_secs(10 * 60);

/// How long [`SessionManager::input`] waits for its bytes to drain into the
/// PTY before reporting "not accepting input". The write itself runs on the
/// PTY's own writer thread (never a tokio worker) and stays queued in order,
/// so a timeout loses nothing — a 60 KB prompt paste into a TUI that is still
/// booting drains once it starts reading.
const INPUT_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a LIVE resumable session must be idle (no output) AND unattached
/// (no WS viewer) before its PTY is suspended to free RAM. The conversation
/// stays resumable, so reopening it auto-resumes via `--resume`.
pub const SUSPEND_GRACE: Duration = Duration::from_secs(5 * 60);

/// How long a chat keep-alive ping (`POST /sessions/{id}/transcript/touch`,
/// sent every 60 s while a conversation view is mounted on a live session)
/// keeps counting as a viewer for the idle-suspend sweep. A chat mounts no
/// terminal, so it never `attach()`es; without this the sweep saw every
/// Chat-mode session as unattached and suspended it after [`SUSPEND_GRACE`]
/// of quiet — the composer went dead until a reload or a terminal attach.
/// Three missed pings (a frozen tab, a network blip) before the hold lapses.
pub const VIEW_HOLD: Duration = Duration::from_secs(3 * 60);

/// How long a LIVE but NON-resumable **background** (engine-owned) agent
/// session must be idle+unattached before the sweep KILLS it. Suspend can
/// never reclaim these (no provider id to resume — e.g. a codex review agent
/// whose rollout pick was ambiguous across a same-cwd fan-out), so before this
/// existed each one held its PTY (~3 fds), its agent process and that
/// process's MCP sidecar FOREVER; review fleets accumulated hundreds of fds
/// and pushed the daemon over launchd's 256 soft cap ("spawn claude: Too many
/// open files (os error 24)", `accept()` failures, seconds-long keystrokes).
/// Generous — every engine turn-watcher trips its stall/quiet windows
/// (≤ 3 min) long before this — and it only ever fires on sessions whose
/// owning engine is done with them. Foreground sessions are never touched.
pub const REAP_UNRESUMABLE_GRACE: Duration = Duration::from_secs(30 * 60);

/// Default for the `manual_idle_suspend_secs` setting: how long a session the
/// USER started (Agents page, [`is_user_started`]) must be quiet — no PTY
/// output, no viewer, no open turn, no descendant CPU — before the idle sweep
/// suspends it too. Engine sessions use the much shorter
/// `idle_suspend_grace_secs` ([`SUSPEND_GRACE`]); the user's own get a long
/// grace instead of the infinite one they had (r3-05-01: every manual session
/// opened since the daemon started kept its CLI — 150–400 MB of Node plus an
/// MCP sidecar — alive forever). `0` in the setting = never (the old rule).
///
/// A full day: the 30 minutes this started at suspended a session you had
/// merely stepped away from over lunch — reopening it worked (`--resume`),
/// but whatever the CLI held in memory (and a plain shell's whole state) was
/// gone. A day of complete silence is a session you are done with.
pub const MANUAL_IDLE_SUSPEND: Duration = Duration::from_secs(24 * 60 * 60);

/// Setting (bool, default `true`): spawn the sessions YOU start
/// ([`is_user_started`] — Agents-page agents and shells) inside a detached
/// PTY holder ([`otto_pty::holder`]) so they keep running across a daemon
/// restart (deploy, crash, app relaunch) and are re-attached — same process,
/// same screen and scrollback — when the daemon comes back. Off = the old
/// behaviour: the PTY lives in the daemon and dies with it (agents come back
/// via `--resume`, shells start fresh). Engine-owned sessions are never held:
/// their driving engine dies with the daemon and recovers its own way.
pub const SESSION_PERSISTENCE_SETTING: &str = "session_persistence";

/// Default for the `max_live_agent_sessions` setting: a soft cap on live
/// agent-CLI sessions. Past it the sweep suspends the least-recently-used
/// quiet ones ([`SessionManager::enforce_live_cap`]) — never one that is
/// mid-turn, watched, pinned or busy. `0` in the setting = no cap.
pub const MAX_LIVE_AGENT_SESSIONS: usize = 12;

/// `meta.suspended.reason` values (review A4): why a session went dormant.
/// The idle sweep's grace ran out.
pub const SUSPEND_REASON_IDLE: &str = "idle";
/// The live-session cap reclaimed the least recently used quiet session.
pub const SUSPEND_REASON_CAP: &str = "cap";
/// The daemon restarted and the process did not survive it.
pub const SUSPEND_REASON_RESTART: &str = "restart";
/// An explicit [`SessionManager::suspend`] call (engines, API).
pub const SUSPEND_REASON_RELEASED: &str = "released";
/// An explicit restart killed the old process and then could not start the
/// new one (account apply, spawn, shutdown, provider-id write). The cause is
/// in `meta.suspended.error` (error-handling-4-05: this used to be stamped
/// `released`, so the pane blamed a memory-saving suspend).
pub const SUSPEND_REASON_RESPAWN_FAILED: &str = "respawn_failed";

/// The agent-trail lifecycle line for a suspend with `reason`.
fn suspend_lifecycle_line(reason: &str) -> &'static str {
    match reason {
        SUSPEND_REASON_IDLE => "Suspended (idle — freed memory, still resumable)",
        SUSPEND_REASON_CAP => "Suspended (live-session cap — freed memory, still resumable)",
        SUSPEND_REASON_RESTART => "Suspended (daemon restart — still resumable)",
        SUSPEND_REASON_RESPAWN_FAILED => "Restart failed (stopped, still resumable)",
        _ => "Suspended (freed memory, still resumable)",
    }
}

/// What [`SessionManager::restore_all`] did at boot: sessions re-adopted from
/// their PTY holders (still running) and sessions that were live but lost
/// their process with the previous daemon (now dormant, resumable).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RestoreSummary {
    pub kept_running: usize,
    pub suspended: usize,
}

/// The cap never suspends a session that printed something more recently than
/// this (or than the idle grace, when that is set lower): a CLI that just
/// streamed is plausibly still working even when its transcript says nothing.
pub const CAP_MIN_QUIET: Duration = Duration::from_secs(2 * 60);

/// Pure decision for the sweep's kill branch (see [`REAP_UNRESUMABLE_GRACE`]):
/// only an **agent** session (never a connection terminal), only an
/// engine-owned **background** one (a foreground session is the user's own
/// live conversation — killing a non-resumable one would destroy it), and only
/// past the reap grace. The caller has already established the session is
/// live, unattached, not `keep_alive`-pinned, CPU-quiet and not resumable.
fn should_reap_unresumable(session: &Session, idle_for: Duration) -> bool {
    session.kind == SessionKind::Agent
        && !session.is_foreground_agent()
        && idle_for >= REAP_UNRESUMABLE_GRACE
}

/// Did the USER start this session by hand — the Agents page — as opposed to
/// an engine starting it in the background?
///
/// A user-started session gets the long `manual_idle_suspend_secs` grace
/// ([`MANUAL_IDLE_SUSPEND`]) instead of the engine one. "No PTY output
/// for 5 minutes" is NOT "done": an agent that hands work to background
/// watchers and `sleep`-polls them is silent, burns no descendant CPU and is
/// very much mid-turn — and yanking its PTY away mid-turn is exactly the bug
/// this guard exists for (observed three times in one afternoon on the user's
/// own interactive claude session). Engine-owned sessions (workflow, channel,
/// review, swarm, delegation, …) keep the old behaviour: their owning engine
/// has already consumed the turn output, so reclaiming their RAM loses nothing.
///
/// Two independent signals mark a session as engine-owned — a background
/// `meta.source` (see [`Session::is_foreground_agent`], which also excludes
/// connection terminals) or a `meta.work.origin` other than `"manual"`. A
/// session with no work ref AT ALL is the plain-create path (older rows,
/// pre-dating the `origin` stamp), i.e. the user's own.
fn is_user_started(session: &Session) -> bool {
    if !session.is_foreground_agent() {
        return false;
    }
    match session
        .meta
        .get("work")
        .and_then(|w| w.get("origin"))
        .and_then(|o| o.as_str())
    {
        Some(origin) => origin == "manual",
        None => true,
    }
}

/// The opaque metadata a session's PTY holder carries (and hands back to the
/// daemon that re-adopts it after a restart): which session it is, plus the
/// per-session ingest token already baked into the child's environment — the
/// new daemon must accept that token, or the agent's hooks go silent.
fn holder_meta(session: &Session, ingest_token: Option<String>) -> serde_json::Value {
    serde_json::json!({
        "session_id": session.id,
        "workspace_id": session.workspace_id,
        "provider": session.provider,
        "ingest_token": ingest_token,
    })
}

/// How much of a provider activity artifact the turn probe reads. Only the
/// TAIL matters (the last few lines decide), and these files run to hundreds
/// of MB on a long conversation.
const TURN_TAIL_BYTES: u64 = 256 * 1024;

/// Last [`TURN_TAIL_BYTES`] of `path` as lossy UTF-8, with the (necessarily
/// partial) first line of the window dropped when the file was bigger than it.
/// Blocking I/O — call from the blocking pool.
fn read_tail(path: &std::path::Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max);
    if start > 0 {
        f.seek(SeekFrom::Start(start))?;
    }
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(match (start > 0, text.find('\n')) {
        (true, Some(i)) => text[i + 1..].to_string(),
        (true, None) => String::new(), // one >256 KiB line: nothing parseable
        (false, _) => text,
    })
}

/// Is the agent in the MIDDLE OF A TURN according to the provider's own
/// on-disk record? `Some(true)` = open turn (treat as ACTIVE even with zero
/// PTY output and zero CPU), `Some(false)` = the last turn closed,
/// `None` = unknown (unreadable/unparseable file, provider without a rule) —
/// callers must fall back to their previous decision and never read `None` as
/// "idle".
///
/// The rule is the turn oracle's own fold, [`otto_transcript::turn::tail_open`]
/// (architecture-6-02 — one parser, not a local mirror):
///
/// - **claude**: the last MESSAGE-bearing parent line must be an assistant
///   `end_turn` for the turn to be closed — and a later delivered
///   `<task-notification>` re-opens it (a harness wake-up is pending), except
///   one stapled onto an `attachment` line or a `remove`d queue entry.
/// - **codex**: the latest `event_msg` `task_started` with no matching
///   `task_complete` / `turn_aborted` for the same `turn_id`.
fn agent_turn_open(provider: &str, artifact: &std::path::Path) -> Option<bool> {
    let tail = read_tail(artifact, TURN_TAIL_BYTES).ok()?;
    otto_transcript::turn::tail_open(provider, &tail)
}

/// The idle sweep's per-session HOLD decision, as a pure function: given the
/// row, the transcript verdict from [`agent_turn_open`] and the age of that
/// transcript, return the name of the guard that keeps the session alive, or
/// `None` to let the sweep reclaim it.
///
/// Factored out so the guards are testable without a PTY — deleting one here
/// fails a test, which is the point (the sweep itself cannot be driven in a
/// unit test: it walks `self.live`, which needs real child processes).
/// The descendant-CPU guard is NOT here: it runs earlier, off the process
/// table, before the row is even loaded.
///
/// `artifact_age` bounds the open-turn hold. A transcript that has not moved
/// for [`REAP_UNRESUMABLE_GRACE`] is a STUCK turn, not a live one: every
/// engine watcher gives up inside 3 minutes, and an agent genuinely polling a
/// background watcher appends a `tool_result` on every poll. Without the bound
/// a single un-answered `<task-notification>` tail pinned an engine session's
/// PTY, agent process and MCP sidecar forever — the exact fd leak
/// [`REAP_UNRESUMABLE_GRACE`] exists to close. Unknown age (`None`, an
/// unreadable artifact) never holds.
fn sweep_hold(
    session: &Session,
    turn_open: Option<bool>,
    artifact_age: Option<Duration>,
    manual: ManualIdle,
) -> Option<&'static str> {
    if is_pinned(session) {
        return Some("keep_alive");
    }
    // The user's OWN sessions (Agents page): silence is not doneness, so they
    // get the long `manual_idle_suspend_secs` grace instead of the engine one.
    // Past it they fall through to the same open-turn guard as everything
    // else. A process that a passive open merely RESUMED (nobody typed into
    // it since) earns no hold at all: it is not doing anything for anyone.
    if is_user_started(session) && !manual.passive_resume {
        let within = manual.grace.is_none_or(|g| manual.quiet_for < g);
        if within {
            return Some("origin=manual");
        }
    }
    turn_hold(turn_open, artifact_age)
}

/// What the idle sweep knows about a session's recent use, for the
/// user-started branch of [`sweep_hold`].
#[derive(Clone, Copy, Debug)]
struct ManualIdle {
    /// How long the PTY has printed nothing.
    quiet_for: Duration,
    /// `manual_idle_suspend_secs` ([`MANUAL_IDLE_SUSPEND`]); `None` = never.
    grace: Option<Duration>,
    /// The live process was spawned by a resume-on-open (`ensure_live` from a
    /// terminal attach or a chat ping) and has received no input since — see
    /// [`SessionManager::passive_resume`].
    passive_resume: bool,
}

/// `meta.keep_alive`: the user pinned the session; no sweep ever touches it.
fn is_pinned(session: &Session) -> bool {
    session
        .meta
        .get("keep_alive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// An OPEN AGENT TURN is activity even at zero output and zero CPU — as long
/// as the transcript backing that claim is still fresh (see [`sweep_hold`]).
fn turn_hold(turn_open: Option<bool>, artifact_age: Option<Duration>) -> Option<&'static str> {
    if turn_open == Some(true) && artifact_age.is_some_and(|age| age < REAP_UNRESUMABLE_GRACE) {
        return Some("turn open");
    }
    None
}

/// Does a live session count toward `max_live_agent_sessions`? Agent CLIs
/// only — the heavy processes the cap exists for. A plain `shell` is cheap
/// and cannot be resumed losslessly, and connection terminals are not agents.
fn counts_toward_cap(session: &Session) -> bool {
    session.kind == SessionKind::Agent && session.provider != "shell"
}

/// The cap's per-session HOLD, as a pure function (see
/// [`SessionManager::enforce_live_cap`]): the cap overrides the user-started
/// grace, but never a pin, a `Working` row, a fresh open turn, or a session
/// that cannot be resumed (suspending one of those would lose work).
fn cap_hold(
    session: &Session,
    resumable: bool,
    turn_open: Option<bool>,
    artifact_age: Option<Duration>,
) -> Option<&'static str> {
    if !resumable {
        return Some("not resumable");
    }
    if is_pinned(session) {
        return Some("keep_alive");
    }
    if session.status == SessionStatus::Working {
        return Some("working");
    }
    turn_hold(turn_open, artifact_age)
}

/// How many sessions the cap must free: live agent CLIs over `cap` (`0` =
/// no cap).
fn cap_excess(live_agents: usize, cap: usize) -> usize {
    if cap == 0 {
        0
    } else {
        live_agents.saturating_sub(cap)
    }
}

/// A session must have printed nothing for this long before a "restart when
/// idle" caller (the nightly CLI auto-update) may re-exec it.
pub const RESTART_QUIET: Duration = Duration::from_secs(30);

/// Pure "would a restart NOW interrupt work?" decision (r3-08-01), so the
/// guards are unit-testable without a PTY. Busy when any of:
/// - the row says `Working`;
/// - an engine turn driver still holds the session ([`SessionManager::hold_for_turn`]);
/// - the PTY printed something inside [`RESTART_QUIET`] (streaming output);
/// - the provider's transcript has an OPEN turn that moved inside
///   [`REAP_UNRESUMABLE_GRACE`] (quiet `sleep`-polling is still mid-turn —
///   same freshness bound as [`sweep_hold`]).
///
/// Unknown transcript state is NOT busy on its own: the other three guards
/// already cover a live turn, and a caller only defers, never skips.
pub fn restart_would_interrupt(
    status: SessionStatus,
    engine_turn_open: bool,
    quiet_for: Duration,
    turn_open: Option<bool>,
    artifact_age: Option<Duration>,
) -> bool {
    status == SessionStatus::Working
        || engine_turn_open
        || quiet_for < RESTART_QUIET
        || (turn_open == Some(true) && artifact_age.is_some_and(|a| a < REAP_UNRESUMABLE_GRACE))
}

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

    /// The PTY behind `session_id`'s output stream closed (its process is gone
    /// and the reader drained): drop any per-session state kept for it. A
    /// respawn starts a new stream. Default: nothing to drop.
    fn on_session_end(&self, _session_id: &Id) {}
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

/// Upper bound a respawn waits for the process it just retired to exit (the
/// CLIs' SIGHUP shutdown takes ~0.5–0.8 s; the kill escalates to SIGTERM /
/// SIGKILL on its own). Past it the respawn proceeds anyway — a wedged child
/// must not make the session unopenable.
const RETIRED_EXIT_WAIT: Duration = Duration::from_secs(5);

/// Longest one session's shutdown waits for its resume lock, and the budget
/// all of them share (see `SessionManager::shutdown_ids`): well inside the
/// daemon's 12 s teardown timeout.
const SHUTDOWN_LOCK_WAIT: Duration = Duration::from_secs(1);
const SHUTDOWN_LOCK_BUDGET: Duration = Duration::from_secs(4);

/// Wait (bounded by [`RETIRED_EXIT_WAIT`]) for a retired process's exit.
async fn await_retired_exit(id: &Id, mut rx: tokio::sync::watch::Receiver<Option<i32>>) {
    if tokio::time::timeout(RETIRED_EXIT_WAIT, wait_exit_code(&mut rx))
        .await
        .is_err()
    {
        tracing::warn!(
            session = %id,
            "retired PTY still alive after {}s; respawning anyway",
            RETIRED_EXIT_WAIT.as_secs()
        );
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

/// RAII guard for an engine turn driver's watch on a session (see
/// [`SessionManager::hold_for_turn`]): released on every return path of the
/// driver, including the error ones, so a failed turn never pins a session.
pub struct TurnHold {
    manager: Arc<SessionManager>,
    id: Id,
}

impl Drop for TurnHold {
    fn drop(&mut self) {
        self.manager.release_turn(&self.id);
    }
}

/// Default daemon base URL agent hooks post activity back to. Overridden via
/// [`SessionManager::with_ingest_base`] (ottod sets it from its bind port).
const DEFAULT_INGEST_BASE: &str = "http://127.0.0.1:7700";

/// The name theme applied to new agent sessions when a user hasn't chosen one.
/// Single source of truth lives in [`crate::names::DEFAULT_THEME`].
const DEFAULT_NAME_THEME: &str = crate::names::DEFAULT_THEME;

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
    /// The capped head of the transcript was read and holds no user prompt:
    /// it never will, so the session is never read again either.
    gave_up: bool,
}

/// Owns live sessions: PTY handles keyed by session id plus persistence.
pub struct SessionManager {
    /// Holders [`Self::adopt_holders`] could not decide on (a transient DB
    /// error on the session lookup), by session id → socket. Left running,
    /// and kept out of the boot dormant pass and credential sweep; retried
    /// by [`Self::retry_deferred_adoptions`] and by [`Self::ensure_live`]
    /// before it would ever start a second CLI (review S1-09).
    deferred_holders: Arc<DashMap<Id, std::path::PathBuf>>,
    /// Daemon shutdown has begun ([`Self::shutdown_all`] /
    /// [`Self::shutdown_for_restart`]): no restart may spawn any more.
    shutting_down: std::sync::atomic::AtomicBool,
    /// Shared so the per-session status task can evict an exited handle
    /// (otherwise dead PtyHandles — and their emulator + ring buffer — leak).
    live: Arc<DashMap<Id, Arc<PtyHandle>>>,
    /// In-memory count of attached WS terminal viewers per session. Bumped by
    /// `ws::serve_terminal` on attach/detach; read by the idle-suspend sweep so
    /// it never suspends a session someone is actively watching.
    attached: Arc<DashMap<Id, usize>>,
    /// Attached WS connection ids per session (see [`Self::may_resize`]).
    attached_conns: Arc<DashMap<Id, Vec<u64>>>,
    /// Last chat keep-alive ping per session (see [`VIEW_HOLD`]): a mounted
    /// conversation view is a viewer even though it holds no terminal WS.
    viewed: Arc<DashMap<Id, std::time::Instant>>,
    /// Engine turn drivers currently WATCHING each session (see
    /// [`Self::hold_for_turn`]). A workflow step / review agent / channel reply
    /// whose owning engine is still polling its transcript is not "done", no
    /// matter how quiet its PTY is — the oracle's bounded holds (handoff /
    /// background-task linger, up to 15 min) are longer than the 5-min idle
    /// grace, and suspending the session under the engine's feet made it
    /// re-run the whole step in a fresh session.
    engine_turns: Arc<DashMap<Id, usize>>,
    /// Size-authority owner per session: the conn that most recently typed.
    size_owner: Arc<DashMap<Id, u64>>,
    /// Grid feed for terminal viewers (crate::grid, the WS `grid` frame).
    grids: Arc<crate::grid::GridFeeds>,
    /// Per-session authority locks issue epochs rechecked by the PTY writer.
    room_authority: crate::room_authority::RoomAuthorityMap,
    /// Last observed cumulative CPU (ms) of each live session's DESCENDANT
    /// process tree (excluding the direct child), sampled by the idle-suspend
    /// sweep. A tree that accrued CPU since the previous sweep is running a
    /// quiet long command (build/test) — not idle — and is skipped.
    suspend_cpu: Arc<DashMap<Id, u64>>,
    /// Last guard that HELD each session in the idle-suspend sweep (see
    /// [`sweep_hold`]). Purely a log de-duplicator: a held session is re-held
    /// every 60 s forever, so only a CHANGE of guard is worth an `info!` line.
    suspend_hold: Arc<DashMap<Id, &'static str>>,
    /// Sessions whose CURRENT process was spawned by [`Self::ensure_live`] (a
    /// terminal attach / chat ping reopening a suspended session) and has had
    /// no input since. Opening a session to read its history must not pin a
    /// 150–400 MB agent CLI: such a process gets the short engine grace, not
    /// the user-started one ([`sweep_hold`]). Cleared by any typed / automated
    /// input, an explicit restart, and every teardown path.
    passive_resume: Arc<DashMap<Id, ()>>,
    /// Exit watch of the process a lifecycle op (suspend / restart) just
    /// killed. A respawn of the same session waits on it (bounded, see
    /// [`RETIRED_EXIT_WAIT`]) before starting the next CLI, so two
    /// incarnations never write one provider conversation at the same time.
    /// Only the tiny watch receiver is kept, never the handle (its emulator
    /// and ring must be freed by the suspend).
    retiring: Arc<DashMap<Id, tokio::sync::watch::Receiver<Option<i32>>>>,
    /// Serializes the best-effort `meta.pty_cols/pty_rows` writes of
    /// [`Self::resize_pty`] so they can never land out of order.
    size_persist: Arc<tokio::sync::Mutex<()>>,
    /// Last time somebody USED each session — typed into it, attached or
    /// detached a terminal, pinged a chat. With the PTY's own last-output time
    /// it orders the live-session cap's least-recently-used eviction.
    last_touch: Arc<DashMap<Id, std::time::Instant>>,
    repo: SessionsRepo,
    networks: Arc<crate::network::SessionNetworks>,
    events: broadcast::Sender<Event>,
    providers: ProviderRegistry,
    /// Optional context-provisioning hook, invoked before an agent spawn.
    pre_spawn_hook: Option<Arc<dyn PreSpawnHook>>,
    /// Optional resolver for the workspace's enabled user-configured MCP servers,
    /// merged into `.mcp.json` on agent spawn (alongside Otto's browser entry).
    mcp_servers: Option<Arc<dyn crate::mcp::McpServerSource>>,
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
    provider_accounts: Option<(
        otto_state::provider_accounts::ProviderAccountsRepo,
        std::path::PathBuf,
    )>,
    /// The daemon's configured data dir (`$OTTO_DATA_DIR` or the default),
    /// which the process sandbox write-denies (and whose secrets / state DB it
    /// read-denies). `None` ⇒ resolved like `ottod`'s config does.
    data_dir: Option<std::path::PathBuf>,
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
    /// First-input probe per session with a PENDING provider-id capture.
    /// Registered (empty) at spawn; [`Self::input`] records the first input
    /// moment and accumulates the initial bytes. The capture task gates its
    /// rollout scan on the input moment (a promptless TUI never mints a rollout,
    /// so scanning before input can only claim someone ELSE's conversation) and
    /// content-matches the bytes against candidate rollouts' first user_message.
    capture_probes: Arc<DashMap<Id, CaptureProbe>>,
    /// In-flight provider-id captures per canonical cwd. When >1, concurrent
    /// same-cwd spawns are racing and a capture only claims a rollout its own
    /// typed prompt confirms; when this session is alone, the sole-candidate
    /// fast path applies (see [`pick_codex_rollout`]).
    captures_in_flight: Arc<DashMap<String, usize>>,
    /// Optional name-themes store. When set, a new agent session whose title is
    /// not explicitly provided is auto-named from the creating user's active
    /// theme (e.g. "Ronaldo"), unique among the workspace's open sessions.
    /// Absent ⇒ the legacy "{provider} #N" numbering.
    name_themes: Option<otto_state::NameThemesRepo>,
    /// Per-session state for the provider-title auto-namer sweep
    /// ([`Self::refresh_provider_titles`]). In-memory only: it's a read-cache,
    /// and the durable `meta.title_source` marker survives restarts.
    title_probe: Arc<DashMap<Id, TitleProbe>>,
    /// Per-session resume/restart serialization. Two concurrent WS attaches to a
    /// reconnectable session (a pane + the tiled overview is a realistic pair)
    /// would otherwise both pass `ensure_live`'s `is_live` check and both spawn:
    /// the second `live.insert` overwrites the first handle (alive but untracked
    /// — the exact orphan class `evict_if_same` exists to prevent) and, for
    /// claude/codex, `--resume` runs twice against one conversation (the
    /// 2026-07-21 fork incident). All resume paths take this lock and re-check
    /// `is_live` under it.
    resume_locks: Arc<DashMap<Id, Arc<Mutex<()>>>>,
    /// Per-session submit mutex ([`Self::submit_lock`]): every "paste, wait,
    /// Enter" submitter — automation (`submit_text`: agent→agent messages,
    /// broadcast, relay, nudges), humans (`human_submit_text_checked`) and the
    /// channel bridge — holds it across the whole pair, so two submissions
    /// within the paste delay can't land as paste A, paste B, Enter, Enter
    /// (one merged prompt plus an empty submit). Evicted in `remove`.
    submit_locks: Arc<DashMap<Id, Arc<Mutex<()>>>>,
    /// Where user-started sessions' PTY holders live and how to launch one
    /// (see [`SESSION_PERSISTENCE_SETTING`]). `None` = every PTY is local (the
    /// pre-holder behaviour; bare test harnesses).
    holders: Option<otto_pty::HolderConfig>,
}

impl SessionManager {
    pub fn new(
        repo: SessionsRepo,
        events: broadcast::Sender<Event>,
        providers: ProviderRegistry,
    ) -> Self {
        let networks = crate::network::SessionNetworks::new(repo.pool());
        Self {
            live: Arc::new(DashMap::new()),
            attached: Arc::new(DashMap::new()),
            attached_conns: Arc::new(DashMap::new()),
            viewed: Arc::new(DashMap::new()),
            engine_turns: Arc::new(DashMap::new()),
            size_owner: Arc::new(DashMap::new()),
            grids: Default::default(),
            room_authority: Default::default(),
            suspend_cpu: Arc::new(DashMap::new()),
            deferred_holders: Arc::new(DashMap::new()),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            suspend_hold: Arc::new(DashMap::new()),
            passive_resume: Arc::new(DashMap::new()),
            retiring: Arc::new(DashMap::new()),
            size_persist: Arc::new(tokio::sync::Mutex::new(())),
            last_touch: Arc::new(DashMap::new()),
            repo,
            networks,
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
            provider_accounts: None,
            data_dir: None,
            // Default to this daemon's own binary so `mcp-tools` is the same build.
            mcp_tools_bin: std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(str::to_owned))
                .unwrap_or_else(|| "ottod".to_string()),
            mcp_tokens: Arc::new(DashMap::new()),
            codex_capture_lock: Arc::new(Mutex::new(())),
            capture_probes: Arc::new(DashMap::new()),
            captures_in_flight: Arc::new(DashMap::new()),
            name_themes: None,
            title_probe: Arc::new(DashMap::new()),
            resume_locks: Arc::new(DashMap::new()),
            submit_locks: Arc::new(DashMap::new()),
            holders: None,
        }
    }

    /// Enable PTY holders: user-started sessions survive daemon restarts (see
    /// [`SESSION_PERSISTENCE_SETTING`]). Builder-style.
    pub fn with_pty_holders(mut self, config: otto_pty::HolderConfig) -> Self {
        self.holders = Some(config);
        self
    }

    /// [`Self::with_pty_holders`] for an optional config (`None` = off).
    pub fn with_pty_holders_opt(mut self, config: Option<otto_pty::HolderConfig>) -> Self {
        self.holders = config;
        self
    }

    /// Is session persistence active right now? Needs a holder config AND the
    /// [`SESSION_PERSISTENCE_SETTING`] (default on).
    pub async fn persistence_enabled(&self) -> bool {
        if self.holders.is_none() {
            return false;
        }
        match &self.settings {
            Some(sr) => sr
                .get(SESSION_PERSISTENCE_SETTING)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            None => true,
        }
    }

    /// Spawn a session's PTY. A session the user started goes into a PTY
    /// holder when persistence is on, so it outlives this daemon; anything
    /// else — and any holder failure — spawns in-process as before (logged:
    /// that session then won't survive a restart). Fork/exec runs on the
    /// blocking pool either way (it can take tens-hundreds of ms).
    async fn spawn_session_pty(
        &self,
        session: &Session,
        spec: CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<PtyHandle> {
        if is_user_started(session) && self.persistence_enabled().await {
            if let Some(config) = self.holders.clone() {
                let meta = holder_meta(
                    session,
                    self.ingest_tokens
                        .get(&session.id)
                        .map(|t| t.value().clone()),
                );
                let held_spec = spec.clone();
                let held = tokio::task::spawn_blocking(move || {
                    PtyHandle::spawn_held(&config, &held_spec, cols, rows, meta)
                })
                .await
                .unwrap_or_else(|e| Err(Error::Internal(format!("pty holder task: {e}"))));
                match held {
                    Ok(handle) => return Ok(handle),
                    Err(e) => tracing::warn!(
                        session = %session.id,
                        "pty holder unavailable — spawning in-process (this session won't survive a daemon restart): {e}"
                    ),
                }
            }
        }
        tokio::task::spawn_blocking(move || PtyHandle::spawn_sized(&spec, cols, rows))
            .await
            .unwrap_or_else(|e| Err(Error::Internal(format!("pty spawn task: {e}"))))
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

    /// The daemon's configured data dir — the one the process sandbox must
    /// protect. Without it the sandbox falls back to `$OTTO_DATA_DIR` / the
    /// default location, which is wrong for a daemon started with a custom
    /// `--data-dir`-style config.
    pub fn with_data_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// [`missing_cwd_refusal_in`] against this daemon's data dir: the
    /// configured one (`with_data_dir` / `$OTTO_DATA_DIR`), never a hard-coded
    /// default (S1-306).
    fn missing_cwd_refusal(&self, session: &Session) -> Option<String> {
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        missing_cwd_refusal_in(session, &self.sandbox_data_dir(&home))
    }

    /// The data dir the sandbox protects: the configured one, else resolved
    /// exactly like `ottod::config` (`$OTTO_DATA_DIR`, then the default).
    fn sandbox_data_dir(&self, home: &std::path::Path) -> std::path::PathBuf {
        if let Some(d) = &self.data_dir {
            return d.clone();
        }
        std::env::var_os("OTTO_DATA_DIR")
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join("Library/Application Support/Otto"))
    }

    pub fn with_provider_accounts(
        mut self,
        repo: otto_state::provider_accounts::ProviderAccountsRepo,
        root: std::path::PathBuf,
    ) -> Self {
        self.provider_accounts = Some((repo, root));
        self
    }

    /// Canonical profile root for transcript/activity lookup. The account ID is
    /// validated at creation, immutable thereafter, and checked again on resume.
    pub fn provider_home(&self, session: &Session) -> Option<std::path::PathBuf> {
        let id = session.meta.get("account_id")?.as_str()?;
        crate::accounts::account_home(&self.provider_accounts.as_ref()?.1, &id.to_owned()).ok()
    }

    /// The Codex sessions root this session's CLI writes to — the shared
    /// [`otto_transcript::roots::TranscriptRoots::for_session`] rule (its
    /// account home, else the explicit / isolated / default root). No root
    /// (an account id without configured accounts) → the default root.
    fn codex_root_for(&self, session: &Session) -> std::path::PathBuf {
        use otto_transcript::roots::{home_dir, RootsEnv};
        let accounts = self.provider_accounts.as_ref().map(|(_, r)| r.as_path());
        let data = self.data_dir.as_deref();
        RootsEnv::from_process()
            .for_session(&session.meta, &home_dir(), accounts, data)
            .codex
            .into_iter()
            .next()
            .map(|root| root.dir)
            .unwrap_or_else(codex_sessions_root)
    }

    async fn resolve_account(
        &self,
        owner: &Id,
        provider: &str,
        meta: &serde_json::Value,
    ) -> Result<
        Option<(
            otto_core::provider_accounts::ProviderAccount,
            std::path::PathBuf,
        )>,
    > {
        let Some(value) = meta.get("account_id").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let id = value.as_str().filter(|s| !s.is_empty()).ok_or_else(|| {
            Error::Invalid("account_id must be a nonempty profile ID or null".into())
        })?;
        let (repo, root) = self
            .provider_accounts
            .as_ref()
            .ok_or_else(|| Error::Invalid("provider account support is unavailable".into()))?;
        let account = repo.get(owner, &id.to_owned()).await?;
        if account.provider != provider {
            return Err(Error::Invalid(
                "selected account belongs to a different provider".into(),
            ));
        }
        let home = crate::accounts::account_home(root, &account.id)?;
        crate::accounts::prepare_home(&home)?;
        Ok(Some((account, home)))
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
        self.record_relayed_message(session_id, TrailSource::User, None, text)
            .await;
    }

    /// Record a message another agent session sent into this one with its own
    /// credential (`/message`, broadcast, relay — S11-305). Attributed to the
    /// agent and the sending session, never to the person who owns both.
    pub async fn record_agent_message(&self, session_id: &Id, from: &Id, text: &str) {
        self.record_relayed_message(session_id, TrailSource::Agent, Some(from), text)
            .await;
    }

    async fn record_relayed_message(
        &self,
        session_id: &Id,
        source: TrailSource,
        from: Option<&Id>,
        text: &str,
    ) {
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
        let summary = match from {
            Some(from) => trail_clip(&format!("from session {from}: {trimmed}"), 200),
            None => trail_clip(trimmed, 200),
        };
        self.record_trail(
            session_id,
            &session.workspace_id,
            source,
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
        // One submission at a time per session (see `submit_locks`).
        let lock = self.submit_lock(id);
        let _submit = lock.lock().await;
        // `bracketed_paste` strips control bytes: an `ESC [ 2 0 1 ~` in a task
        // title would otherwise END the paste and type the rest as keystrokes.
        self.input(id, &crate::submit::bracketed_paste(text))
            .await?;
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
        let sessions = self.live_agent_sessions(ws).await?;
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

    /// Prune the activity trail to the newest `keep_per_session` rows per
    /// session — every session, or with `since` only those that received rows
    /// since then (see [`ActivityRepo::prune_trail`]). No-op without an
    /// activity store. Returns rows pruned.
    pub async fn prune_activity_trail(
        &self,
        keep_per_session: i64,
        since: Option<std::time::SystemTime>,
    ) -> u64 {
        let Some(repo) = self.activity.as_ref() else {
            return 0;
        };
        let since = since.map(chrono::DateTime::<chrono::Utc>::from);
        match repo.prune_trail(keep_per_session, since).await {
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
    pub fn with_mcp_servers(mut self, provider: Arc<dyn crate::mcp::McpServerSource>) -> Self {
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

    /// Whether the opt-in `process_sandbox` setting is on at all (any provider
    /// may then be confined). Unreadable ⇒ `true`: callers use this to RELAX a
    /// check, so an error must keep the strict path.
    async fn process_sandbox_enabled(&self) -> bool {
        if !otto_sandbox::is_supported() {
            return false;
        }
        let Some(sr) = &self.settings else {
            return false;
        };
        match sr.get("process_sandbox").await {
            Ok(Some(cfg)) => cfg.get("enabled").and_then(|v| v.as_bool()) == Some(true),
            Ok(None) => false,
            Err(_) => true,
        }
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
        // A read-only session (`meta.read_only`) is ALWAYS confined, whatever
        // the `process_sandbox` setting says: writes only inside its own
        // folder (+ temp + the CLIs' config dirs). Network stays open so the
        // CLI reaches its model API.
        let forced = session.meta.get("read_only").and_then(|v| v.as_bool()) == Some(true);
        let network = if forced {
            otto_sandbox::NetworkPolicy::Full
        } else {
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
            network
        };

        let cwd = std::path::PathBuf::from(&session.cwd);
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let data_dir = self.sandbox_data_dir(&home);
        // Resolve the git dir so commits in a worktree (whose .git lives outside
        // cwd) still work. Best-effort; absent for non-repos.
        let mut extra: Vec<std::path::PathBuf> = Vec::new();
        if let Some(gitdir) = resolve_git_common_dir(&cwd).await {
            extra.push(gitdir);
        }
        // A named-account session's CLI home lives under the data dir
        // (`provider-accounts/<id>`), which the profile otherwise write-denies;
        // `for_agent` re-opens an extra inside the data dir — only this one.
        if let Some(account_home) = self.provider_home(session) {
            extra.push(account_home);
        }
        let mut policy = sandbox_policy(&cwd, &home, &data_dir, &extra, network, forced);
        // Codex runs with `CODEX_HOME` = the shadow home in its context bundle
        // (`~/.otto/context/codex/<cwd>/codex-home`), which the profile
        // otherwise write-denies with the rest of `~/.otto/context`: re-open
        // that one home, minus its config / skills / instruction files.
        if let Some(shadow) = spec
            .env
            .iter()
            .rev()
            .find(|(k, _)| k == "CODEX_HOME")
            .map(|(_, v)| std::path::PathBuf::from(v))
            .filter(|p| p.is_absolute() && !p.starts_with(&data_dir))
        {
            policy = policy.allow_provider_home(&shadow);
        }
        // Package managers / build tools cache into a private per-session dir
        // (under temp, which is writable): the user's shared caches hold code
        // unsandboxed programs run later, so they are not writable roots.
        let cache = agent_cache_dir(&session.id);
        otto_sandbox::seed_agent_cache(&cache, &home, |k| std::env::var(k).ok());
        spec.env.extend(otto_sandbox::agent_cache_env(&cache));
        let (program, args) = policy.wrap(&spec.program, &spec.args);
        spec.program = program;
        spec.args = args;
        tracing::info!(
            session = %session.id,
            provider = %session.provider,
            "process sandbox enabled (network={network:?})"
        );
    }

    /// The `process_sandbox` policy for a NON-PTY process the daemon runs on a
    /// user's behalf as `provider` in `cwd` — today a scheduled `shell` task's
    /// `/bin/sh -c`. Same decision and profile as a session spawn
    /// ([`Self::apply_sandbox`]): `None` when the setting is off or excludes
    /// `provider`, or on a host without Seatbelt.
    pub async fn process_sandbox_policy(
        &self,
        provider: &str,
        cwd: &std::path::Path,
    ) -> Option<otto_sandbox::SandboxPolicy> {
        if !otto_sandbox::is_supported() {
            return None;
        }
        let cfg = self
            .settings
            .as_ref()?
            .get("process_sandbox")
            .await
            .ok()??;
        let network = sandbox_decision(&cfg, SessionKind::Agent, provider)?;
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let data_dir = self.sandbox_data_dir(&home);
        let extra: Vec<std::path::PathBuf> =
            resolve_git_common_dir(cwd).await.into_iter().collect();
        Some(sandbox_policy(
            cwd, &home, &data_dir, &extra, network, false,
        ))
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
        // Durable session ownership survives daemon restarts. Do not mint a
        // replacement if revocation failed, or credentials would accumulate.
        if let Err(e) = self
            .revoke_mcp_token(&session.created_by, &session.id)
            .await
        {
            tracing::warn!(session = %session.id, "otto MCP token rotation failed: {e}");
            return OttoToolsInjection::default();
        }
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
            auth.issue_session_api_token(&session.created_by, &session.id)
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
        // Claude reads the identity-neutral workspace `.mcp.json`, agy its
        // `.agents/mcp_config.json`; both entries are written by the caller's
        // single per-spawn reconcile (`sync_workspace_mcp`, which receives
        // `server` below). The session-specific token and routing values come
        // from the spawn env (agy passes it through to stdio MCP children).
        //
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
                &session_mcp_profiles(&session.meta),
            ) {
                Ok(path) => {
                    return OttoToolsInjection {
                        args: crate::mcp::codex_mcp_inject_args(
                            &self.mcp_tools_bin,
                            &path.to_string_lossy(),
                        ),
                        env,
                        server: Some(server),
                    };
                }
                Err(e) => tracing::warn!("otto MCP tools: write codex creds failed: {e}"),
            }
        }
        OttoToolsInjection {
            args: Vec::new(),
            env,
            server: Some(server),
        }
    }

    /// Revoke the per-session MCP token minted for `session_id` (if any), and
    /// delete the Codex creds file if one was written. Called from the
    /// session-removal path so the `otto` tool server's credential dies with the
    /// session. Best-effort.
    async fn revoke_mcp_token(&self, owner: &Id, session_id: &Id) -> Result<()> {
        revoke_session_credentials(self.auth.as_ref(), &self.mcp_tokens, owner, session_id).await
    }

    /// A session's process was retired (exit, kill, suspend, archive): its
    /// per-session MCP credential has no live holder any more, so revoke it
    /// now instead of leaving the owner's full permissions valid for the
    /// token's 10-year TTL. Every respawn mints a fresh one
    /// (`maybe_enable_otto_tools`). Best-effort, logged.
    async fn retire_credentials(&self, session: &Session) {
        if let Err(e) = self
            .revoke_mcp_token(&session.created_by, &session.id)
            .await
        {
            tracing::warn!(session = %session.id, "revoke retired session credentials: {e}");
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

    pub fn network_status(&self, id: &str) -> otto_core::network_profiles::SessionNetworkStatus {
        self.networks.status(id)
    }

    /// S11-302: refuse an agent session whose folder is `/`, `$HOME` or one of
    /// its parents, or Otto's data dir (or a non-work-area path inside it) —
    /// the session's cwd becomes a Seatbelt write grant and is pre-trusted,
    /// so `$HOME` would hand a "confined" agent `~/.zshrc` and
    /// `~/Library/LaunchAgents`. The one exception is a workspace a person
    /// rooted at `$HOME` itself, opened at that root (its sandbox profile
    /// simply carries no cwd grant — `SandboxPolicy::for_agent` never grants
    /// a root covering `$HOME`).
    ///
    /// `unconfined`: the session gets no Seatbelt profile (the opt-in
    /// `process_sandbox` setting is off and it is not read-only). Then there
    /// is no write grant to widen, so `$HOME` ITSELF — "start Claude in ~" —
    /// is the creator's call, like opening a terminal there. `/`, the
    /// parents of `$HOME` and Otto's data dir stay refused, and an agent's
    /// own credential is confined separately by [`Self::check_agent_cwd`].
    fn check_session_cwd(&self, ws: &Workspace, cwd: &str, unconfined: bool) -> Result<()> {
        let home = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let data_dir = self.sandbox_data_dir(&home);
        let path = std::path::Path::new(cwd);
        let Some(why) = otto_sandbox::unsafe_session_cwd(path, &home, &data_dir) else {
            return Ok(());
        };
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).ok();
        let is_home =
            !home.as_os_str().is_empty() && canon(path).is_some() && canon(path) == canon(&home);
        let workspace_home_root =
            is_home && canon(path) == canon(std::path::Path::new(&ws.root_path));
        if workspace_home_root || (is_home && unconfined) {
            return Ok(());
        }
        Err(Error::Invalid(format!(
            "refusing to start an agent session in {cwd}: it is {why}"
        )))
    }

    /// S11-302: where an agent's OWN credential (`caller` = its session) may
    /// open another session. Inside the workspace root, inside the caller's
    /// own folder, inside a repo a person registered in this workspace, or in
    /// a worktree of the same repository as any of those (same git common
    /// dir) — never an arbitrary folder, whose tree the new
    /// session's sandbox would make writable and its trust pre-seeding would
    /// trust. `None` (the workspace root) is always fine. A person's
    /// credential never comes here.
    pub async fn check_agent_cwd(
        &self,
        ws: &Workspace,
        caller: &Id,
        cwd: Option<&str>,
    ) -> Result<()> {
        let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) else {
            return Ok(());
        };
        let want = std::path::Path::new(cwd);
        if !want.is_absolute() {
            return Err(Error::Forbidden(
                "an agent may only open sessions at an absolute folder".into(),
            ));
        }
        let want = std::fs::canonicalize(want)
            .map_err(|_| Error::Forbidden(format!("an agent may not open a session in {cwd}")))?;
        let mut bases: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(&ws.root_path)];
        if let Ok(own) = self.get(caller).await {
            if !own.cwd.trim().is_empty() {
                bases.push(std::path::PathBuf::from(own.cwd));
            }
        }
        // A registered repo may live outside the workspace root (sweep
        // 2026-10-11 M2): a person chose it for this workspace.
        match otto_state::GitStore::new(self.repo.pool())
            .list_repos(&ws.id)
            .await
        {
            Ok(repos) => bases.extend(repos.into_iter().map(|r| std::path::PathBuf::from(r.path))),
            Err(e) => tracing::warn!(workspace = %ws.id, "agent cwd check: list repos: {e}"),
        }
        let bases: Vec<std::path::PathBuf> = bases
            .iter()
            .filter_map(|b| std::fs::canonicalize(b).ok())
            .collect();
        if bases.iter().any(|b| want.starts_with(b)) {
            return Ok(());
        }
        if let Some(common) = resolve_git_common_dir(&want).await {
            for b in &bases {
                if resolve_git_common_dir(b).await.as_ref() == Some(&common) {
                    return Ok(());
                }
            }
        }
        Err(Error::Forbidden(format!(
            "an agent may only open sessions inside the workspace, its own folder, \
             a repo registered in this workspace or a worktree of those (not {cwd})"
        )))
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
        // Shutdown has begun (concurrency-async-5-04): a session created now
        // would be published after the teardown iterated `live` — a CLI
        // nobody kills, its MCP token never revoked, its row `running` until
        // boot recovery. Refused here, and re-checked once the spawn returns.
        self.refuse_while_shutting_down()?;
        let _mcp_activation = crate::mcp::activation_gate().read().await;
        let mut req = req;
        // Agent UI control's grant is server-owned and never outlives the
        // session it was given to: no creator (a client, a handover/fork
        // copying meta, an agent calling the create route with its token)
        // may start a session pre-granted.
        if let Some(obj) = req.meta.as_mut().and_then(serde_json::Value::as_object_mut) {
            obj.remove("ui_control");
        }
        // Fold the explicit `model` param into `meta.model` (winning over any
        // model already in `meta`) so ONE meta key drives both the spawn args
        // below and every later resume (`restart_locked` re-reads it).
        if let Some(model) = req
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            let meta = req.meta.get_or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("model".into(), model.into());
            }
        }
        let cwd = req.cwd.clone().unwrap_or_else(|| ws.root_path.clone());
        if req.kind == SessionKind::Agent {
            let read_only = req
                .meta
                .as_ref()
                .and_then(|m| m.get("read_only"))
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            let unconfined = !read_only && !self.process_sandbox_enabled().await;
            self.check_session_cwd(ws, &cwd, unconfined)?;
        }

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
                spec.args.extend(model_args(
                    self.providers.model_args_template(&provider).as_deref(),
                    &meta_val,
                ));
                spec.args.extend(lean_turn_args(&provider, &meta_val));
                read_only_strip(&provider, &meta_val, &mut spec.args);
                spec.args.extend(read_only_args(&provider, &meta_val));
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

        let account = self.resolve_account(user_id, &provider, &meta).await?;
        if let Some((account, _)) = &account {
            crate::accounts::validate_account(&spec, account)?;
            meta["account_label"] = account.label.clone().into();
        }
        let project_context = self.repo.project_context(&ws.id, &meta).await?;
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

        let network = match self.networks.prepare(&session).await {
            Ok(network) => network,
            Err(error) => {
                let _ = self.repo.delete(&session.id).await;
                return Err(error);
            }
        };

        // The cwd must exist (a missing dir makes the child fall back to
        // $HOME) and agent CLIs should already trust the workspace folder.
        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            // The legacy trust helper writes default CLI homes. Named profiles
            // let their native CLI/prompt guard persist trust in their own home.
            if account.is_none() {
                crate::trust::ensure_trusted_async(&session.provider, &session.cwd).await;
            }
            // Otto's first-party read-only tool server: when the workspace has
            // opted in (`otto_mcp_enabled`), mint a per-session token; the
            // launcher entry itself lands via the reconcile below. Opt-in,
            // best-effort — never blocks spawn.
            let otto_tools = self.maybe_enable_otto_tools(&session).await;
            let codex_otto = !otto_tools.args.is_empty();
            spec.args.extend(otto_tools.args);
            spec.env.extend(otto_tools.env);
            // Everything MCP in the shared cwd — the browser opt-in
            // (meta.browser), the workspace's *enabled* user servers, and the
            // `otto` entry above — is written in ONE reconcile per launcher
            // file, so disables propagate as removals and concurrent spawns
            // sharing a cwd can't lose each other's updates. Codex doesn't
            // read `.mcp.json`; its user servers ride per-spawn `-c` overrides.
            let codex_user_args = self
                .sync_workspace_mcp(&session, otto_tools.server, codex_otto)
                .await;
            spec.args.extend(codex_user_args);
            // Otto context provisioning: materialize the workspace's active
            // skills + soul + context into this CLI's native form. Best-effort —
            // the hook logs and swallows its own errors, never blocking spawn.
            //
            // Reviews receive the same workspace knowledge. Keep ordinary
            // provider bundle paths stable (Codex credentials depend on its
            // home identity); explicit accounts/projects retain scoped bundles.
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
                let context = (account.is_some() || project_context.is_some()).then(|| {
                    otto_core::hooks::SessionSpawnContext {
                        namespace: session.id.clone(),
                        provider_home: account.as_ref().map(|(_, home)| home.clone()),
                        extra_context_md: project_context.clone().unwrap_or_default(),
                    }
                });
                let injection = tokio::task::spawn_blocking(move || match context {
                    Some(context) => hook.before_spawn_session(&ws_owned, &cwd, &prov, &context),
                    None => hook.before_spawn(&ws_owned, &cwd, &prov),
                })
                .await
                .unwrap_or_default();
                spec.args.extend(injection.args);
                spec.env.extend(injection.env);
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
        if let Some((account, home)) = &account {
            if let Err(error) = crate::accounts::apply_account(&mut spec, account, home) {
                let _ = self
                    .revoke_mcp_token(&session.created_by, &session.id)
                    .await;
                let _ = self.repo.delete(&session.id).await;
                return Err(error);
            }
        }
        if let Some(network) = &network {
            spec.env
                .retain(|(key, _)| !network.env.iter().any(|(mapped, _)| mapped == key));
            spec.env.extend(network.env.clone());
        }
        self.apply_sandbox(&mut spec, &session).await;

        // fork/exec of the agent CLI is a synchronous syscall path that can take
        // tens-hundreds of ms — `spawn_session_pty` keeps it off the async
        // workers (blocked workers with idle CPU are exactly the intermittent
        // everything-is-slow shape).
        let handle = match self
            .spawn_unless_shutting_down(&session, spec.clone(), grid_cols, grid_rows)
            .await
        {
            Ok(h) => Arc::new(h),
            Err(e) => {
                let _ = self
                    .revoke_mcp_token(&session.created_by, &session.id)
                    .await;
                let _ = self.repo.delete(&session.id).await;
                return Err(e);
            }
        };

        self.networks
            .activate(session.id.clone(), network, handle.on_exit());
        // Providers that mint their own session id (codex): capture it from the
        // on-disk rollout now that the CLI is running, so the session becomes
        // resumable across daemon restarts (like claude's `--session-id`).
        if session.kind == SessionKind::Agent
            && session.provider_session_id.is_none()
            && self.providers.captures_session_id(&session.provider)
        {
            self.spawn_session_id_capture(&session);
        }
        self.live.insert(session.id.clone(), Arc::clone(&handle));
        self.publish_grid(&session.id);
        self.start_status_task(
            session.id.clone(),
            session.workspace_id.clone(),
            session.provider.clone(),
            handle,
            false,
        );
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
    /// The scan is GATED on this session's first PTY input: codex flushes its
    /// rollout lazily (nothing on disk until the first prompt), so a promptless
    /// session has nothing of its own to match and scanning early can only
    /// claim someone else's conversation — the 2026-07-21 cross-wire incident
    /// (12 same-cwd spawns; captures paired "longest-waiting task" with "next
    /// prompt typed anywhere", then suspend/resume forked live conversations).
    /// When several same-cwd captures are in flight, a rollout is only claimed
    /// if its recorded first user_message matches OUR typed input (see
    /// [`pick_codex_rollout`]). Claims are persisted under `codex_capture_lock`
    /// (acquired per scan pass, not across the whole window) so concurrent
    /// captures see each other's claims. Best-effort: no match within the
    /// window leaves the session non-resumable — we never guess and resume the
    /// wrong conversation.
    fn spawn_session_id_capture(&self, session: &Session) {
        /// Give-up horizon for a session that never receives any input — as
        /// long as such a session may sit idle before the sweep suspends it
        /// ([`MANUAL_IDLE_SUSPEND`]): a codex session opened in the morning
        /// and first typed into after lunch must still become resumable.
        /// Waiting costs one map lookup per 500 ms tick.
        const NO_INPUT_GIVEUP: Duration = MANUAL_IDLE_SUSPEND;
        /// Scan window after the first input. Generous: under a many-spawn CPU
        /// storm codex-tui has been observed taking 2min+ to boot and consume
        /// the (kernel-buffered) input, and only then does it flush the rollout.
        const WINDOW_AFTER_INPUT: Duration = Duration::from_secs(180);

        let repo = self.repo.clone();
        let lock = Arc::clone(&self.codex_capture_lock);
        let probes = Arc::clone(&self.capture_probes);
        let in_flight = Arc::clone(&self.captures_in_flight);
        let id = session.id.clone();
        // codex/agy record a symlink-RESOLVED cwd (macOS `/var` → `/private/var`,
        // `/tmp` → `/private/tmp`) in their session files; the scans below compare
        // by exact string, so match the canonical form or a session launched from
        // a symlinked path never captures its id and silently stays non-resumable.
        let cwd = std::fs::canonicalize(&session.cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| session.cwd.clone());
        let provider = session.provider.clone();
        let provider_home = self.provider_home(session);
        let codex_root = self.codex_root_for(session);
        let probe = CaptureProbe::default();
        let generation = Arc::clone(&probe.generation);
        let resume_locks = Arc::clone(&self.resume_locks);
        probes.insert(id.clone(), probe);
        *in_flight.entry(cwd.clone()).or_insert(0) += 1;
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let mut captured: Option<String> = None;
            let mut logged_ambiguous = false;
            // Waiting for the first input costs one map lookup per tick; the
            // scans after it back off (review S1-28): each one walks the
            // rollout dir and reads files, and loads every claimed id.
            let mut pause = Duration::from_millis(500);
            loop {
                tokio::time::sleep(pause).await;
                // Snapshot the probe (guard must not be held across awaits).
                let Some((first_at, probe_text)) = probes
                    .get(&id)
                    .filter(|p| Arc::ptr_eq(&p.generation, &generation))
                    .map(|p| (p.first_at, normalize_pty_input(&p.raw)))
                else {
                    break; // session removed
                };
                let Some(first_at) = first_at else {
                    if started.elapsed() > NO_INPUT_GIVEUP {
                        break;
                    }
                    continue; // no input yet — nothing of ours can be on disk
                };
                let since_input = std::time::SystemTime::now()
                    .duration_since(first_at)
                    .unwrap_or_default();
                if since_input > WINDOW_AFTER_INPUT {
                    break;
                }
                pause = capture_scan_pause(since_input);
                let floor = first_at
                    .checked_sub(Duration::from_secs(2))
                    .unwrap_or(std::time::UNIX_EPOCH);
                // Content arbitration is only needed while OTHER same-cwd
                // captures race us; alone, the sole-candidate path is exact and
                // tolerant of probe-garbling edits (arrow keys etc.).
                let contended = in_flight.get(&cwd).map(|v| *v).unwrap_or(1) > 1;
                let probe = contended.then_some(probe_text.as_str());
                if !matches!(provider.as_str(), "codex" | "agy") {
                    break;
                }
                // Serialize persistence with restart/removal. A replacement
                // probe belongs to a different process even though the Otto
                // session ID is unchanged; never claim its predecessor's ID.
                let resume_lock = session_lock(&resume_locks, &id);
                let _resume_guard = resume_lock.lock().await;
                if !probes
                    .get(&id)
                    .is_some_and(|p| Arc::ptr_eq(&p.generation, &generation))
                {
                    break;
                }
                let _guard = lock.lock().await;
                // Reopen-time capture may have assigned an ID while this task
                // waited for the lifecycle lock, without replacing its probe.
                // Never reinterpret the remaining rollout as our conversation.
                match repo.get(&id).await {
                    Ok(current) if current.provider_session_id.is_some() => {
                        captured = current.provider_session_id;
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(session = %id, "provider id capture: session lookup failed: {error}");
                        break;
                    }
                }
                let claimed_rows = repo.provider_session_ids().await.unwrap_or_default();
                // Directory walks + file reads: off the async workers.
                let pick = {
                    let provider = provider.clone();
                    let codex_root = codex_root.clone();
                    let cwd = cwd.clone();
                    let probe = probe.map(str::to_owned);
                    tokio::task::spawn_blocking(move || {
                        let claimed: std::collections::HashSet<&str> =
                            claimed_rows.iter().map(String::as_str).collect();
                        match provider.as_str() {
                            "codex" => pick_codex_rollout(
                                &codex_root,
                                &cwd,
                                floor,
                                &claimed,
                                probe.as_deref(),
                            ),
                            _ => scan_agy_conversation(&agy_cli_root(), &cwd, floor, &claimed)
                                .map(RolloutPick::Claim)
                                .unwrap_or(RolloutPick::Nothing),
                        }
                    })
                    .await
                    .unwrap_or(RolloutPick::Nothing)
                };
                match pick {
                    RolloutPick::Claim(psid) => {
                        // Persist under the lock so the next capture's claimed
                        // set already contains this id.
                        match repo.set_provider_session(&id, &psid).await {
                            Ok(()) => {
                                persist_transcript_path(
                                    &repo,
                                    &id,
                                    &provider,
                                    &cwd,
                                    &psid,
                                    provider_home.as_deref(),
                                )
                                .await;
                                captured = Some(psid)
                            }
                            Err(e) => tracing::warn!(
                                session = %id, "provider id capture: persist failed: {e}"
                            ),
                        }
                        break;
                    }
                    RolloutPick::Ambiguous => {
                        if !logged_ambiguous {
                            logged_ambiguous = true;
                            tracing::info!(
                                session = %id, cwd = %cwd,
                                "provider id capture: multiple unclaimed candidates, no content match yet — waiting"
                            );
                        }
                    }
                    RolloutPick::Nothing => {}
                }
            }
            // Success drops the probe; a miss KEEPS it so the late (reopen-time)
            // capture can still content-match. Removed with the session either way.
            if captured.is_some() {
                probes.remove_if(&id, |_, p| Arc::ptr_eq(&p.generation, &generation));
            }
            if let Some(mut e) = in_flight.get_mut(&cwd) {
                *e = e.saturating_sub(1);
            }
            in_flight.remove_if(&cwd, |_, v| *v == 0);
            match captured {
                Some(psid) => tracing::info!(
                    session = %id, provider = %provider, provider_session = %psid,
                    "captured provider session id — session is now resumable"
                ),
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
            .open_agent_titles(ws_id)
            .await
            .ok()?
            .into_iter()
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

    /// The workspace's agent sessions that have a live PTY in this daemon,
    /// fetched by id from the in-memory live map — a live-only caller never
    /// decodes the workspace's whole session history
    /// (backend-performance-4-03). Rows keep their stored status; callers
    /// apply their own status rule.
    pub async fn live_agent_sessions(&self, ws: &Id) -> Result<Vec<Session>> {
        let ids: Vec<Id> = self.live.iter().map(|e| e.key().clone()).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let scope = otto_state::SessionScope {
            workspace_id: ws.clone(),
            owner: None,
        };
        let filter = otto_state::SessionListFilter {
            archived: Some(false),
            kind: Some("agent".into()),
            ids: Some(ids),
            ..Default::default()
        };
        self.repo.list_filtered(&[scope], &filter).await
    }

    /// Sessions across one or many workspace scopes with every narrowing
    /// filter applied in SQL (see [`otto_state::SessionsRepo::list_filtered`]) — the list
    /// endpoints' path, so a live-only list never decodes archived history.
    pub async fn list_filtered(
        &self,
        scopes: &[otto_state::SessionScope],
        filter: &otto_state::SessionListFilter,
    ) -> Result<Vec<Session>> {
        self.repo.list_filtered(scopes, filter).await
    }

    /// Live (non-archived, non-exited) sessions whose `meta.<meta_key>`
    /// equals `value`, filtered in SQL (see
    /// [`otto_state::SessionsRepo::list_live_by_meta`]).
    pub async fn list_live_by_meta(
        &self,
        ws: &Id,
        kind: Option<&str>,
        meta_key: &'static str,
        value: &str,
    ) -> Result<Vec<Session>> {
        self.repo.list_live_by_meta(ws, kind, meta_key, value).await
    }

    /// True when the session has a live PTY in this daemon process.
    pub fn is_live(&self, id: &Id) -> bool {
        self.live.contains_key(id)
    }

    /// True when `id`'s live PTY runs in a PTY holder (it is detached, not
    /// killed, by [`Self::shutdown_for_restart`] — when persistence is on).
    pub fn runs_in_holder(&self, id: &Id) -> bool {
        self.live
            .get(id)
            .is_some_and(|e| e.value().holder().is_some())
    }

    /// Register a WS terminal viewer for `id` (called on attach). Returns an
    /// [`AttachGuard`] that decrements the count on drop, so every WS exit path
    /// (clean close, error, drop) releases the attachment. Each guard carries a
    /// daemon-unique connection id used by the SIZE-AUTHORITY policy below.
    pub fn attach(self: &Arc<Self>, id: &Id) -> AttachGuard {
        static CONN_SEQ: AtomicU64 = AtomicU64::new(1);
        let conn_id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
        self.touch(id);
        let first = {
            let mut n = self.attached.entry(id.clone()).or_insert(0);
            *n += 1;
            *n == 1
        };
        if first {
            // A long-unviewed session may run with a shallow history cap
            // (see UNVIEWED_HISTORY_AFTER): let it grow again from now.
            if let Some(h) = self.live_handle(id) {
                h.set_history_cap(otto_pty::EMULATOR_SCROLLBACK_LINES);
            }
        }
        self.attached_conns
            .entry(id.clone())
            .or_default()
            .push(conn_id);
        self.publish_grid(id);
        AttachGuard {
            manager: Arc::clone(self),
            id: id.clone(),
            conn_id,
        }
    }

    /// Decrement the attached-viewer count for `id`, removing the entry at zero.
    fn detach(&self, id: &Id, conn_id: u64) {
        // Leaving a pane is the last moment it was used (the cap's LRU clock).
        self.touch(id);
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
        // The owner leaving un-owns the grid for the viewers that remain (the
        // leaving socket dropped its receiver before this guard).
        self.publish_grid(id);
        self.grids.forget_if_unwatched(id);
        // Authority is STICKY: a departing owner does NOT release it. While
        // the user is away from the pane the session must keep the pane's
        // grid — releasing let a passive tile/preview re-pin the PTY narrow
        // and everything the agent printed meanwhile stayed hard-wrapped at
        // that width forever (the "old parts of the transcript are half
        // width" damage). The next claim/typing (pane re-attach claims on
        // open) simply takes authority over; a daemon restart clears the map.
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
        self.publish_grid(id);
    }

    /// Optimistic [`Self::note_input_authority`] for the terminal socket
    /// (perf 01 N7): typing claims the size the moment the keystroke is
    /// QUEUED, so a `resize` right behind the first keystroke is not judged
    /// against stale authority while the PTY write is still pending. Returns
    /// the previous owner for [`Self::revert_input_authority`].
    pub fn claim_input_authority(&self, id: &Id, conn_id: u64) -> Option<u64> {
        let prev = self.size_owner.insert(id.clone(), conn_id);
        if prev != Some(conn_id) {
            self.publish_grid(id);
        }
        prev
    }

    /// Undo an optimistic claim whose write failed — only while `conn_id`
    /// still holds it (a newer claim by someone else stands).
    pub fn revert_input_authority(&self, id: &Id, conn_id: u64, prev: Option<u64>) {
        let mut reverted = false;
        if let dashmap::mapref::entry::Entry::Occupied(mut e) = self.size_owner.entry(id.clone()) {
            if *e.get() == conn_id {
                reverted = true;
                match prev {
                    Some(p) => {
                        e.insert(p);
                    }
                    None => {
                        e.remove();
                    }
                }
            }
        }
        if reverted {
            self.publish_grid(id);
        }
    }

    /// `id`'s grid + ATTACHED size owner as viewers see it (crate::grid).
    pub fn grid_state(&self, id: &Id) -> crate::grid::GridState {
        crate::grid::grid_state(&self.live, &self.attached_conns, &self.size_owner, id)
    }

    /// Subscribe to `id`'s grid feed (the WS loop sends each change).
    pub fn grid_watch(&self, id: &Id) -> tokio::sync::watch::Receiver<crate::grid::GridState> {
        self.grids.watch(id, || self.grid_state(id))
    }

    /// Push `id`'s grid to its viewers when it changed (every resize and
    /// size-authority change calls this).
    pub fn publish_grid(&self, id: &Id) {
        self.grids.publish(id, || self.grid_state(id));
    }

    /// Whether `conn_id` may resize `id` under the size-authority policy:
    /// yes when it IS the owner, or when nobody has ever claimed/typed (a
    /// session only ever watched by claim-less embeds — e.g. a vault-run or
    /// workflow panel — is sized by whoever views it). Once claimed, only a
    /// newer claim/typing transfers the right — never a passive viewer, not
    /// even while the owner is detached (see the sticky note in `detach`).
    pub fn may_resize(&self, id: &Id, conn_id: u64) -> bool {
        match self.size_owner.get(id).map(|e| *e) {
            None => true,
            Some(owner) => owner == conn_id,
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

    /// Bump `last_active_at` to now without changing the status. The channel
    /// bridge calls this when it hands a follow-up message to an existing
    /// session: the column otherwise only moves on a status transition, so a
    /// thread that keeps getting replies inside one long turn looked idle to
    /// the channel reaper and was archived mid-conversation.
    pub async fn touch_activity(&self, id: &Id) -> Result<()> {
        self.repo.touch(id).await
    }

    /// Record a chat keep-alive ping for `id` (the conversation view's
    /// `POST …/transcript/touch`). Holds the session against the idle-suspend
    /// sweep for [`VIEW_HOLD`], exactly like a terminal attachment would.
    pub fn note_view(&self, id: &Id) {
        self.viewed.insert(id.clone(), std::time::Instant::now());
        self.touch(id);
    }

    /// Stamp `id` as used now (the live-session cap's LRU clock).
    fn touch(&self, id: &Id) {
        self.last_touch
            .insert(id.clone(), std::time::Instant::now());
    }

    /// Real input reached `id` (a person typed, or automation sent a turn):
    /// the process is doing something for someone, so it is no longer a
    /// passive resume ([`Self::passive_resume`]) and it was just used.
    fn note_engaged(&self, id: &Id) {
        self.passive_resume.remove(id);
        self.touch(id);
    }

    /// True while `id`'s live process was spawned by a resume-on-open and has
    /// had no input since (see the field doc).
    pub fn is_passive_resume(&self, id: &Id) -> bool {
        self.passive_resume.contains_key(id)
    }

    /// Drop the sweep's per-session bookkeeping once `id`'s process is gone.
    fn forget_sweep_state(&self, id: &Id) {
        self.suspend_cpu.remove(id);
        self.suspend_hold.remove(id);
        self.passive_resume.remove(id);
    }

    /// Register an engine turn driver as a watcher of `id` for as long as the
    /// returned guard lives. The idle-suspend sweep treats it exactly like an
    /// attached viewer: the session is never suspended or reaped while its
    /// owning engine is still consuming the turn — an engine-owned session
    /// "loses nothing when reclaimed" only AFTER its driver has returned. The
    /// guard is in-memory: a daemon restart drops every hold along with the
    /// drivers themselves.
    pub fn hold_for_turn(self: &Arc<Self>, id: &Id) -> TurnHold {
        *self.engine_turns.entry(id.clone()).or_insert(0) += 1;
        TurnHold {
            manager: Arc::clone(self),
            id: id.clone(),
        }
    }

    /// True while at least one engine turn driver holds `id`.
    pub fn engine_turn_open(&self, id: &Id) -> bool {
        self.engine_turns.get(id).is_some_and(|n| *n > 0)
    }

    /// OS pid of `id`'s live child process (None when not live / unknown).
    /// Lets a deferred "restart when idle" tell whether the process it meant
    /// to replace is still the one running (a user restart already did it).
    pub fn live_pid(&self, id: &Id) -> Option<u32> {
        self.live.get(id).and_then(|e| e.value().pid())
    }

    /// Would restarting `id` right now interrupt an in-flight turn? See
    /// [`restart_would_interrupt`]. A session that is not live is never busy.
    /// Reads the provider transcript tail off the runtime (blocking hop).
    pub async fn busy_for_restart(&self, id: &Id) -> bool {
        let Some(quiet_for) = self
            .live
            .get(id)
            .map(|e| e.value().last_output_at().elapsed())
        else {
            return false;
        };
        let engine_open = self.engine_turn_open(id);
        let Ok(session) = self.repo.get(id).await else {
            // Unreadable row: say busy — a caller only ever defers on it.
            return true;
        };
        // Cheap guards first; only read the transcript when they all pass.
        if restart_would_interrupt(session.status, engine_open, quiet_for, None, None) {
            return true;
        }
        let (turn_open, artifact_age) = match self.activity_artifact(id).await {
            Some(artifact) => {
                let provider = session.provider.clone();
                tokio::task::spawn_blocking(move || {
                    let age = std::fs::metadata(&artifact)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok());
                    (agent_turn_open(&provider, &artifact), age)
                })
                .await
                .unwrap_or((None, None))
            }
            None => (None, None),
        };
        restart_would_interrupt(
            session.status,
            engine_open,
            quiet_for,
            turn_open,
            artifact_age,
        )
    }

    fn release_turn(&self, id: &Id) {
        if let Some(mut n) = self.engine_turns.get_mut(id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                drop(n);
                self.engine_turns.remove(id);
            }
        }
    }

    /// True when `id` was chat-pinged within `hold`.
    pub fn viewed_within(&self, id: &Id, hold: Duration) -> bool {
        self.viewed
            .get(id)
            .map(|at| at.elapsed() <= hold)
            .unwrap_or(false)
    }

    /// Somebody is looking at `id` right now: a terminal WS viewer, a
    /// conversation view that pinged within [`VIEW_HOLD`], OR an engine turn
    /// driver still polling it ([`Self::hold_for_turn`]). The idle-suspend
    /// sweep's "unattached" test.
    pub fn is_watched(&self, id: &Id) -> bool {
        self.is_attached(id) || self.viewed_within(id, VIEW_HOLD) || self.engine_turn_open(id)
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
        // Serialize with every other resume of this session; re-check liveness
        // under the lock (the loser of the race finds the session live and
        // returns instead of double-spawning). The Arc is cloned out of the map
        // entry first so no DashMap shard lock is held across the await.
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        if self.is_live(id) {
            return Ok(());
        }
        // A holder whose boot adoption was deferred may still run this
        // session's agent: adopt it rather than resume a second CLI.
        if let Some(path) = self.deferred_holders.get(id).map(|p| p.clone()) {
            if self.adopt_deferred(id, path).await {
                return Ok(());
            }
            if self.deferred_holders.contains_key(id) {
                return Err(Error::Conflict(
                    "this session's process is still running but could not be reattached yet — try again shortly".into(),
                ));
            }
        }
        let mut session = self.repo.get(id).await?;
        if session.archived {
            return Err(Error::Conflict(
                "session is archived — unarchive it first".into(),
            ));
        }
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
                session.provider_session_id = Some(psid);
            }
        }
        let resumable = session.kind == SessionKind::Agent
            && session.provider_session_id.is_some()
            && self.providers.supports_resume(&session.provider);
        if resumable {
            // Resume-fork guard: if the claimed codex rollout is being written
            // by another live process RIGHT NOW, `codex resume` would fork that
            // conversation into this session (two independent copies of one
            // conversation — the 2026-07-21 "duplicated sessions"). Refuse; the
            // session stays reconnectable and can be reopened once the other
            // process is done (or the claim is corrected).
            if session.provider == "codex" {
                if let Some(psid) = &session.provider_session_id {
                    if rollout_actively_written(
                        &self.codex_root_for(&session),
                        psid,
                        Duration::from_millis(750),
                    )
                    .await
                    {
                        tracing::warn!(
                            session = %id, provider_session = %psid,
                            "refusing resume: conversation is being written by another live process (fork guard)"
                        );
                        return Err(Error::Conflict(
                            "codex conversation is active in another process; refusing to resume a fork of it".into(),
                        ));
                    }
                }
            }
            // A resume-on-open: until real input arrives this process only
            // repainted history for whoever opened it, so it must not earn
            // the user-started idle hold (r3-05-01 — attach-resumed CLIs were
            // pinned forever). Callers that go on to send a turn (channel
            // bridge, agent_session, assistant) clear the flag in `input`.
            // `restart_locked` sets it BEFORE the handle becomes reachable, so
            // input that races the resume is never overwritten as passive.
            self.restart_locked(id, None, true).await.map(|_| ())?;
        } else if session.kind == SessionKind::Agent && session.provider == "shell" {
            // A plain terminal has no provider-side conversation of its own, so
            // the branch above can never bring it back — reopening one used to
            // dead-end on a black screen ("this session doesn't exist any
            // more") even though nothing about it was lost. A shell is cheap
            // and stateless: respawn it. And when the user had an agent CLI
            // running in it, THAT conversation is persistent and was captured
            // by [`Self::capture_nested_agents`] — type its resume command back
            // in so the terminal comes back in the state it was left.
            let resume = nested_resume_command(&session);
            self.restart_locked(id, None, false).await.map(|_| ())?;
            if let Some(cmd) = resume {
                self.type_after_prompt(id, cmd);
            }
        }
        Ok(())
    }

    /// Type a command into a just-respawned shell, once the shell is ready.
    ///
    /// The PTY exists the moment `restart_locked` returns, but the login shell
    /// behind it is still sourcing rc files; bytes written now sit in the line
    /// discipline and are echoed in the middle of the prompt being drawn. Wait
    /// for the shell's first output (its prompt) and a short settle, then type.
    /// Both waits are bounded, so a shell that prints nothing at all still gets
    /// the command. Fire-and-forget: a failure here leaves a working terminal.
    fn type_after_prompt(&self, id: &Id, cmd: String) {
        let Some(handle) = self.live_handle(id) else {
            return;
        };
        let mut rx = handle.subscribe();
        let sid = id.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            match handle
                .write_async(format!("{cmd}\n").as_bytes(), INPUT_WRITE_TIMEOUT)
                .await
            {
                Ok(()) => tracing::info!(
                    session = %sid, %cmd,
                    "typed the nested agent's resume command into the respawned shell"
                ),
                Err(e) => {
                    tracing::warn!(session = %sid, "nested-agent resume input failed: {e}")
                }
            }
        });
    }

    /// One pass of the nested-agent capture: for every LIVE `shell` session,
    /// find the agent CLI the user launched inside it and record that
    /// conversation's id on the session row, making the terminal resumable.
    ///
    /// The scan reads the live process tree, so it only ever sees an agent that
    /// is running right now — hence a periodic sweep rather than a one-shot at
    /// spawn (the user types `claude` whenever they feel like it, and may exit
    /// it and start another). Ids already owned by another session are never
    /// re-claimed, and an ambiguous window is skipped rather than guessed at
    /// (same rule as the codex rollout capture). Returns the number captured.
    ///
    /// Resilient: a failure on one session is logged and skipped.
    pub async fn capture_nested_agents(&self) -> usize {
        // Snapshot live ids first (no DashMap refs held across awaits).
        let snapshot: Vec<(Id, Option<u32>, u64)> = self
            .live
            .iter()
            .map(|e| (e.key().clone(), e.value().pid(), e.value().spawn_seq()))
            .collect();
        {
            // Backoff + eligibility entries die with their session / spawn.
            let mut m = nested_misses().lock().unwrap_or_else(|p| p.into_inner());
            m.retain(|(sid, _), _| snapshot.iter().any(|(id, ..)| id == sid));
            let mut e = nested_eligible().lock().unwrap_or_else(|p| p.into_inner());
            e.retain(|(sid, seq), _| snapshot.iter().any(|(id, _, s)| id == sid && s == seq));
        }
        // Only agent-kind shells can host a nested agent: learn each spawn's
        // kind once, and skip the whole-box `ps` when no live shell exists.
        let mut live: Vec<(Id, Option<u32>)> = Vec::new();
        for (id, pid, seq) in snapshot {
            let known = nested_eligible()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&(id.clone(), seq))
                .copied();
            let eligible = match known {
                Some(v) => v,
                None => {
                    let Ok(s) = self.repo.get(&id).await else {
                        continue; // removed between the snapshot and now
                    };
                    let v = s.kind == SessionKind::Agent && s.provider == "shell";
                    nested_eligible()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert((id.clone(), seq), v);
                    v
                }
            };
            if eligible {
                live.push((id, pid));
            }
        }
        // A shell idling at its prompt has no child process, so it cannot be
        // hosting an agent: skip it without the whole-box `ps` (S9-10).
        live.retain(|(_, pid)| pid.is_some_and(crate::nested::has_children));
        if live.is_empty() {
            return 0;
        }
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
        // `ps -axo …` for the whole box: off the runtime (see `process_table_async`).
        let table = tokio::task::spawn_blocking(crate::nested::process_table)
            .await
            .unwrap_or_default();
        let mut captured = 0;
        for (id, pid) in live {
            let Some(pid) = pid else { continue };
            let session = match self.repo.get(&id).await {
                Ok(s) => s,
                Err(_) => continue, // removed between the snapshot and now
            };
            // Agent-kind shells only: a connection terminal (ssh, a DB
            // client, `k8s exec`) is reopened through its connection, and its
            // PTY's cwd is the daemon's, not the remote's.
            if session.kind != SessionKind::Agent || session.provider != "shell" {
                continue;
            }
            let Some((proc, provider)) = crate::nested::find_nested_agent(pid, &table) else {
                continue;
            };
            // Already captured THIS launch? `nested_pid` pins the capture to one
            // process, so exiting the agent and starting another in the same
            // terminal re-captures instead of resuming the stale conversation.
            let same_launch = session
                .meta
                .get("nested_pid")
                .and_then(|v| v.as_u64())
                .is_some_and(|p| p == proc.pid as u64);
            if same_launch && session.provider_session_id.is_some() {
                continue;
            }
            // An agent that never writes a transcript used to be probed (an
            // `lsof` subprocess + transcript scans) every tick forever; after
            // a few misses the probe backs off (capped), per launch.
            if !nested_probe_due(&id, proc.pid) {
                continue;
            }
            // The agent files its transcript under the directory IT runs in,
            // which is not the session cwd when the user `cd`-ed first.
            // `lsof` (a 30–40 ms subprocess) + canonicalize: off the runtime.
            let (fallback, ppid) = (session.cwd.clone(), proc.pid);
            let cwd = tokio::task::spawn_blocking(move || {
                let cwd = crate::nested::process_cwd(ppid).unwrap_or(fallback);
                std::fs::canonicalize(&cwd)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(cwd)
            })
            .await
            .unwrap_or_else(|_| session.cwd.clone());
            let floor = proc
                .started
                .checked_sub(Duration::from_secs(15))
                .unwrap_or(std::time::UNIX_EPOCH);
            // Serialize with the spawn-time capture so both see each other's
            // claims (a nested agent and a first-class session can race for the
            // same fresh rollout in the same cwd).
            let _guard = self.codex_capture_lock.lock().await;
            let claimed_rows = self.repo.provider_session_ids().await.unwrap_or_default();
            // Transcript / rollout directory scans: off the runtime too.
            let (home_b, cwd_b, started) = (home.clone(), cwd.clone(), proc.started);
            let found = tokio::task::spawn_blocking(move || {
                let claimed: std::collections::HashSet<&str> =
                    claimed_rows.iter().map(String::as_str).collect();
                match provider {
                    "claude" => crate::nested::claude_transcript_in_window(
                        &home_b, &cwd_b, started, &claimed,
                    ),
                    "codex" => match pick_codex_rollout(
                        &codex_sessions_root(),
                        &cwd_b,
                        floor,
                        &claimed,
                        None,
                    ) {
                        RolloutPick::Claim(psid) => Some(psid),
                        RolloutPick::Ambiguous | RolloutPick::Nothing => None,
                    },
                    "agy" => scan_agy_conversation(&agy_cli_root(), &cwd_b, floor, &claimed),
                    _ => None,
                }
            })
            .await
            .ok()
            .flatten();
            let Some(psid) = found else {
                nested_probe_missed(&id, proc.pid);
                continue;
            };
            nested_probe_forget(&id);
            if let Err(e) = self.repo.set_provider_session(&id, &psid).await {
                tracing::warn!(session = %id, "nested-agent capture: persist failed: {e}");
                continue;
            }
            persist_transcript_path(
                &self.repo,
                &id,
                provider,
                &cwd,
                &psid,
                self.provider_home(&session).as_deref(),
            )
            .await;
            let _ = self
                .repo
                .merge_meta(
                    &id,
                    &serde_json::json!({
                        "nested_provider": provider,
                        "nested_cwd": cwd,
                        "nested_pid": proc.pid,
                    }),
                )
                .await;
            captured += 1;
            tracing::info!(
                session = %id, provider, provider_session = %psid, cwd = %cwd,
                "captured a nested agent conversation — this terminal is now resumable"
            );
            self.record_lifecycle(
                &session,
                format!("{provider} started in this terminal — conversation is now resumable"),
            );
        }
        captured
    }

    /// The per-session submit mutex (created lazily; see `submit_locks`). Any
    /// caller that writes a paste and then a separate submit key into the
    /// session's PTY — the channel bridge included — must hold it across both
    /// writes. The entry ref is dropped before the caller awaits the lock.
    pub fn submit_lock(&self, id: &Id) -> Arc<Mutex<()>> {
        self.submit_locks
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// The per-session resume mutex (created lazily). The map entry ref is
    /// dropped before the caller awaits the lock, so no shard lock outlives
    /// this call. Entries are tiny and evicted with the session in `remove`.
    fn resume_lock(&self, id: &Id) -> Arc<Mutex<()>> {
        self.resume_locks
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// One synchronous scan-and-persist for a non-resumable codex/agy session's
    /// on-disk id, run at reopen time instead of spawn time. Same matching
    /// rules as [`Self::spawn_session_id_capture`] (canonical cwd, unclaimed
    /// candidates at/after the floor, serialized by `codex_capture_lock`) —
    /// only the floor differs: the session's `created_at`, since the spawn
    /// moment is long gone. No polling: by reopen time the file either exists
    /// or never will. A retained spawn-time probe (capture missed but daemon
    /// never restarted) still content-matches; otherwise only a SOLE candidate
    /// is claimed — claiming the oldest of several here is how a blank session
    /// could adopt another session's conversation and fork it on resume.
    ///
    /// Both the claimed-id lookup/scan and `set_provider_session` run while
    /// holding the same `codex_capture_lock` guard so two concurrent reopens in
    /// the same cwd cannot both claim the same rollout. Claimed-id lookup
    /// failures fail closed (`None`).
    async fn late_capture_provider_id(&self, session: &Session) -> Option<String> {
        let _guard = self.codex_capture_lock.lock().await;
        let claimed_rows = match self.repo.provider_session_ids().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    session = %session.id,
                    "late provider id capture: claimed-id lookup failed: {e}"
                );
                return None;
            }
        };
        let floor = std::time::SystemTime::from(session.created_at)
            .checked_sub(Duration::from_secs(2))
            .unwrap_or(std::time::UNIX_EPOCH);
        let (provider, sid) = (session.provider.clone(), session.id.clone());
        let (session_cwd, codex_root) = (session.cwd.clone(), self.codex_root_for(session));
        let probe_text =
            (self.capture_probes.get(&session.id)).map(|p| normalize_pty_input(&p.raw));
        // The rollout walk (every `YYYY/MM/DD` dir since `created_at`, plus
        // candidate heads) runs off the runtime like the spawn-time capture;
        // the capture lock stays held so claims still serialize.
        let psid = tokio::task::spawn_blocking(move || {
            let claimed: std::collections::HashSet<&str> =
                claimed_rows.iter().map(String::as_str).collect();
            let cwd = std::fs::canonicalize(&session_cwd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(session_cwd);
            match provider.as_str() {
                "codex" => {
                    match pick_codex_rollout(
                        &codex_root,
                        &cwd,
                        floor,
                        &claimed,
                        probe_text.as_deref(),
                    ) {
                        RolloutPick::Claim(sid) => Some(sid),
                        RolloutPick::Ambiguous => {
                            tracing::warn!(
                                session = %sid, cwd = %cwd,
                                "late provider id capture: multiple unclaimed candidates — refusing to guess"
                            );
                            None
                        }
                        RolloutPick::Nothing => None,
                    }
                }
                "agy" => scan_agy_conversation(&agy_cli_root(), &cwd, floor, &claimed),
                _ => None,
            }
        })
        .await
        .unwrap_or(None)?;
        match self.repo.set_provider_session(&session.id, &psid).await {
            Ok(()) => {
                tracing::info!(
                    session = %session.id,
                    provider = %session.provider,
                    provider_session = %psid,
                    "late provider id capture — session is now resumable"
                );
                persist_transcript_path(
                    &self.repo,
                    &session.id,
                    &session.provider,
                    &session.cwd,
                    &psid,
                    self.provider_home(session).as_deref(),
                )
                .await;
                self.capture_probes.remove(&session.id);
                Some(psid)
            }
            Err(e) => {
                tracing::warn!(
                    session = %session.id,
                    "late provider id capture: persist failed: {e}"
                );
                None
            }
        }
    }

    /// The live PTY handle, when the session has one in this daemon.
    pub fn live_handle(&self, id: &Id) -> Option<Arc<PtyHandle>> {
        self.live.get(id).map(|h| Arc::clone(&h))
    }

    /// Begin exclusive room authority, pinned to the current process.
    pub async fn begin_room_authority(
        &self,
        id: &Id,
        room: &str,
        owner: &Id,
        host_member: &str,
    ) -> Result<()> {
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.begin(room, owner, host_member, handle.spawn_seq())
    }

    pub async fn set_room_driver(&self, id: &Id, room: &str, member: Option<&str>) -> Result<u64> {
        self.room_authority
            .entry(id)
            .lock_owned()
            .await
            .grant(room, member)
    }

    /// Reconnect fences old frames without trusting a possibly stale room snapshot.
    pub async fn fence_room_connection(&self, id: &Id, room: &str, member: &str) -> Result<u64> {
        self.room_authority
            .entry(id)
            .lock_owned()
            .await
            .connect(room, member)
    }

    pub async fn end_room_authority(&self, id: &Id, room: &str) {
        self.room_authority.entry(id).lock_owned().await.end(room);
        self.room_authority.prune_idle(id);
    }

    pub async fn room_authority_snapshot(
        &self,
        id: &Id,
    ) -> Option<crate::room_authority::RoomAuthoritySnapshot> {
        self.room_authority.entry(id).lock_owned().await.snapshot()
    }

    /// A room frame carries the grant epoch observed by its sender. Checking it
    /// under the authority lock and again in the writer rejects stale queued input.
    pub async fn room_input(
        &self,
        id: &Id,
        room: &str,
        member: &str,
        epoch: u64,
        data: &[u8],
    ) -> Result<()> {
        let permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.check_room(room, member, epoch, handle.spawn_seq())?;
        let authorization = permit.authorization();
        drop(permit);
        self.note_engaged(id);
        self.write_pty(id, &handle, data, Some(authorization)).await
    }

    pub async fn room_resize(
        &self,
        id: &Id,
        room: &str,
        member: &str,
        epoch: u64,
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        otto_pty::validate_resize(cols, rows)?;
        let permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.check_room(room, member, epoch, handle.spawn_seq())?;
        self.resize_pty(id, &handle, cols, rows)
    }

    /// Human input has already passed the caller's normal auth checks. Scoped
    /// shares and non-host administrators must not bypass a room's driver seat.
    pub async fn human_input(
        &self,
        id: &Id,
        user: &Id,
        scoped: bool,
        user_initiated: bool,
        data: &[u8],
    ) -> Result<()> {
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.human(user, scoped, !user_initiated, handle.spawn_seq())?;
        let authorization = permit.authorization();
        drop(permit);
        // Emulator replies (DA / cursor-position answers an attach produces
        // on its own) are not the user doing anything.
        if user_initiated {
            self.note_engaged(id);
        }
        self.write_pty(id, &handle, data, Some(authorization)).await
    }

    /// Size-only traffic is passive: it never takes back the guest's driver seat.
    pub async fn human_resize(
        &self,
        id: &Id,
        user: &Id,
        scoped: bool,
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        // Reject a hostile/buggy grid up front (also enforced inside
        // `PtyHandle::resize`, the choke point every path funnels through).
        otto_pty::validate_resize(cols, rows)?;
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.human(user, scoped, true, handle.spawn_seq())?;
        self.resize_pty(id, &handle, cols, rows)
    }

    pub async fn human_claim_size(
        &self,
        id: &Id,
        user: &Id,
        scoped: bool,
        conn_id: u64,
    ) -> Result<()> {
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.human(user, scoped, true, handle.spawn_seq())?;
        self.note_input_authority(id, conn_id);
        Ok(())
    }

    /// Human bracketed paste and delayed Enter share one revocable epoch.
    pub async fn human_submit_text(
        &self,
        id: &Id,
        user: &Id,
        scoped: bool,
        text: &str,
    ) -> Result<()> {
        self.human_submit_text_checked(id, user, scoped, text, || async { Ok(()) })
            .await
    }

    /// Callers with delayed access checks can reauthorize without issuing a new
    /// input grant. A revoke and regrant during the delay still cancels Enter.
    pub async fn human_submit_text_checked<F, Fut>(
        &self,
        id: &Id,
        user: &Id,
        scoped: bool,
        text: &str,
        reauthorize: F,
    ) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        // Serialize with every other paste+Enter submitter (`submit_locks`).
        // Taken before the authority permit, which is never held across it.
        let lock = self.submit_lock(id);
        let _submit = lock.lock().await;
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.human(user, scoped, false, handle.spawn_seq())?;
        let authorization = permit.authorization();
        drop(permit);
        self.note_engaged(id);
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        self.write_pty(
            id,
            &handle,
            format!("\x1b[200~{text}\x1b[201~").as_bytes(),
            Some(authorization.clone()),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        reauthorize().await?;
        if self
            .live_handle(id)
            .is_none_or(|current| current.spawn_seq() != handle.spawn_seq())
        {
            return Err(Error::Conflict(
                "session process changed before submit".into(),
            ));
        }
        self.write_pty(id, &handle, b"\r", Some(authorization))
            .await
    }
    /// Trusted internal automation. HTTP/WebSocket human callers use
    /// `human_input` instead so guest credentials cannot impersonate automation.
    pub async fn input(&self, id: &Id, data: &[u8]) -> Result<()> {
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.automation(handle.spawn_seq())?;
        let authorization = permit.authorization();
        drop(permit);
        self.note_engaged(id);
        self.write_pty(id, &handle, data, Some(authorization)).await
    }

    /// The writer rechecks the captured epoch after queueing. Do not hold the
    /// async permit while a child is hung: the host must still be able to revoke.
    async fn write_pty(
        &self,
        id: &Id,
        handle: &Arc<PtyHandle>,
        data: &[u8],
        authorization: Option<otto_pty::InputAuthorization>,
    ) -> Result<()> {
        // Feed the pending provider-id capture's probe (absent for sessions
        // without one — the common case, a single map lookup).
        if let Some(mut probe) = self.capture_probes.get_mut(id) {
            if probe.first_at.is_none() {
                probe.first_at = Some(std::time::SystemTime::now());
            }
            let room = 4096usize.saturating_sub(probe.raw.len());
            if room > 0 {
                probe.raw.extend_from_slice(&data[..data.len().min(room)]);
            }
        }
        // Never a blocking `write_all` on a tokio worker: a child that stops
        // reading its tty used to park one worker per keystroke/paste until
        // the pool was exhausted and the whole daemon froze.
        handle
            .write_async_authorized(data, INPUT_WRITE_TIMEOUT, authorization)
            .await
    }

    /// Trusted internal terminal resize; normal viewers use `human_resize`.
    pub async fn resize(&self, id: &Id, cols: u16, rows: u16) -> Result<()> {
        otto_pty::validate_resize(cols, rows)?;
        let mut permit = self.room_authority.entry(id).lock_owned().await;
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        permit.automation(handle.spawn_seq())?;
        self.resize_pty(id, &handle, cols, rows)
    }

    fn resize_pty(&self, id: &Id, handle: &Arc<PtyHandle>, cols: u16, rows: u16) -> Result<()> {
        // Same-size resizes are a no-op end to end — no SIGWINCH, no emulator
        // rewrap, no meta write — so clients can re-push their grid freely.
        let (old_cols, old_rows) = handle.size();
        if (old_cols, old_rows) == (cols, rows) {
            return Ok(());
        }
        // Forensic trail for the half-width bug class: every real grid change
        // is logged, so a narrow re-pin can be timed and attributed.
        tracing::info!(session = %id, "pty resize {old_cols}x{old_rows} -> {cols}x{rows}");
        handle.resize(cols, rows)?;
        self.publish_grid(id);
        // Persist the last known grid size so resume/reconnect can restore it
        // (prevents reflow flash on reconnect). Best-effort — no await. Uses the
        // atomic merge (single UPDATE): the old read-modify-write raced
        // update_meta and could revert a concurrent keep-alive/issue toggle
        // with its stale snapshot.
        //
        // Ordering: each resize spawns its own write, and two spawned writes
        // can land out of order (an older grid overwriting the newest). So the
        // writes are serialized on `size_persist`, and each one persists the
        // handle's grid AS OF ITS TURN, not the value it was spawned with —
        // the last write to run always stores the latest size.
        self.persist_grid(id, handle);
        Ok(())
    }

    /// The lifecycle lock covers the generation check AND write: a restart
    /// cannot publish a new handle between them. A delayed predecessor write
    /// must never overwrite the replacement process's saved grid.
    fn persist_grid(&self, id: &Id, handle: &Arc<PtyHandle>) -> tokio::task::JoinHandle<()> {
        let repo = self.repo.clone();
        let sid = id.clone();
        let handle = Arc::clone(handle);
        let live = Arc::clone(&self.live);
        let lifecycle = self.resume_lock(id);
        let order = Arc::clone(&self.size_persist);
        tokio::spawn(async move {
            let _lifecycle = lifecycle.lock().await;
            if live
                .get(&sid)
                .is_none_or(|current| current.spawn_seq() != handle.spawn_seq())
            {
                return;
            }
            let _turn = order.lock().await;
            let (cols, rows) = handle.size();
            let patch = serde_json::json!({ "pty_cols": cols, "pty_rows": rows });
            let _ = repo.merge_meta(&sid, &patch).await;
        })
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
    /// Object-valued keys are replaced in one atomic UPDATE, preserving other
    /// keys and never exposing an absent object between removal and replacement.
    pub async fn update_meta(&self, id: &Id, patch: serde_json::Value) -> Result<Session> {
        if patch.get("account_id").is_some() || patch.get("account_label").is_some() {
            return Err(Error::Invalid(
                "a session keeps its original account; create a new session to choose another"
                    .into(),
            ));
        }
        let _ = self.repo.get(id).await?;
        self.repo.replace_meta_keys(id, &patch).await?;
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
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        self.kill_session_locked(id).await
    }

    /// [`Self::kill_session`] body; caller MUST hold this session's resume lock.
    async fn kill_session_locked(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        // Retire this generation under the resume lock before signalling it.
        // Callers must not see a killed handle as live until the status poller
        // notices its exit; a subsequent restart waits on the retiring child.
        if let Some((_, handle)) = self.live.remove(id) {
            if let Err(error) = handle.kill() {
                // Keep a failed kill reachable for retry instead of declaring
                // the generation gone or tearing down its network transport.
                self.live.insert(id.clone(), handle);
                return Err(error);
            }
            self.retiring.insert(id.clone(), handle.on_exit());
        }
        self.networks.clear(id);
        self.publish_grid(id);
        self.forget_sweep_state(id);
        self.retire_credentials(&session).await;
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
    /// task's exit branch fire ~0.5–0.8 s later (the CLI's SIGHUP shutdown).
    /// That branch only writes a status for the handle `live` still maps, and
    /// only under this session's resume lock (see [`Self::start_status_task`]),
    /// so removing the handle here — under the same lock, BEFORE the kill —
    /// turns its late exit into a no-op and the `Reconnectable` below is final.
    /// (The old `suspending` flag was cleared before the child actually
    /// exited, so the late exit wrote `Exited` over 334 of 337 idle suspends:
    /// channel threads then spawned a fresh, memory-less agent per follow-up.)
    pub async fn suspend(&self, id: &Id) -> Result<()> {
        self.suspend_inner(id, None, SUSPEND_REASON_RELEASED)
            .await
            .map(|_| ())
    }

    /// [`Self::suspend`] for the idle sweep. The sweep evaluates its guards
    /// (quiet PTY, no viewer, no engine turn) and THEN does slow row /
    /// transcript reads; an engine taking a turn hold or a user attaching in
    /// that window must still win. So the cheap guards are re-checked under the
    /// resume lock, right before the kill. `Ok(false)` = no longer idle (or no
    /// longer live) — nothing was done.
    async fn suspend_if_idle(
        &self,
        id: &Id,
        grace: Duration,
        reason: &'static str,
    ) -> Result<bool> {
        self.suspend_inner(id, Some(grace), reason).await
    }

    /// The idle sweep's reap of an UNRESUMABLE background session: the same
    /// cheap guards as [`Self::suspend_if_idle`] (quiet PTY, no viewer, no
    /// open engine turn), re-checked under the resume lock right before the
    /// kill — the sweep's slow transcript reads leave a window in which a user
    /// may attach or an engine may take a turn, and a kill there is not
    /// resumable. `Ok(false)` = no longer idle / no longer live; nothing done.
    async fn kill_if_idle(&self, id: &Id, grace: Duration) -> Result<bool> {
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        if !self.idle_guards_hold(id, grace) {
            return Ok(false);
        }
        self.kill_session_locked(id).await?;
        Ok(true)
    }

    /// Cheap idle guards shared by the sweep's suspend and reap; caller holds
    /// the session's resume lock. False when not live, recently loud, watched
    /// by a viewer, or inside an engine turn.
    fn idle_guards_hold(&self, id: &Id, grace: Duration) -> bool {
        let quiet = self
            .live
            .get(id)
            .map(|h| h.value().last_output_at().elapsed() >= grace);
        quiet == Some(true) && !self.engine_turn_open(id) && !self.is_watched(id)
    }

    /// `reason` is stamped as `meta.suspended = {reason, at}` (review A4) so
    /// the pane can say why and when it went dormant; a resume clears it.
    async fn suspend_inner(
        &self,
        id: &Id,
        idle_grace: Option<Duration>,
        reason: &'static str,
    ) -> Result<bool> {
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        if let Some(grace) = idle_grace {
            if !self.idle_guards_hold(id, grace) {
                return Ok(false);
            }
        }
        let session = self.repo.get(id).await?;
        self.networks.clear(id);
        // Untrack BEFORE the kill: the status task's exit branch then sees a
        // superseded handle and leaves the status alone. A quick resume waits
        // on the recorded exit before spawning the next CLI.
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
            self.retiring.insert(id.clone(), handle.on_exit());
        }
        self.publish_grid(id);
        self.forget_sweep_state(id);
        self.retire_credentials(&session).await;
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        self.record_lifecycle(&session, suspend_lifecycle_line(reason));
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Reconnectable,
        });
        self.stamp_suspended(&session, Some(reason)).await;
        Ok(true)
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
        let before = descendant_cpu_ms(pid, &process_table_async().await);
        tokio::time::sleep(Duration::from_millis(750)).await;
        let after = descendant_cpu_ms(pid, &process_table_async().await);
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
        // The provider session id is joined into file names below; a real one
        // is a UUID-like token, so anything that could name another path
        // (separators, `..`, NUL) is not a session we can locate.
        if psid.is_empty() || psid.contains(['/', '\\', '\0']) || psid.contains("..") {
            return None;
        }
        // Every lookup below is filesystem work (canonicalize, a rollout-tree
        // walk, stats) — resolve the inputs here, then run it off the runtime.
        let provider_home = self.provider_home(&session);
        let codex_root = self.codex_root_for(&session);
        tokio::task::spawn_blocking(move || {
            Self::locate_activity_artifact(&session, &psid, provider_home, &codex_root)
        })
        .await
        .ok()
        .flatten()
    }

    /// [`Self::activity_artifact`]'s filesystem half. Blocking.
    fn locate_activity_artifact(
        session: &Session,
        psid: &str,
        provider_home: Option<std::path::PathBuf>,
        codex_root: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
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
                provider_home
                    .unwrap_or_else(|| {
                        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                            .join(".claude")
                    })
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
                recent_codex_rollouts(codex_root, floor)
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
        // SQL pre-filter (SG-06): the sweep used to decode EVERY session ever
        // (archived too) every 20 s to find a handful of candidates.
        let Ok(sessions) = self.repo.list_title_candidates().await else {
            return 0;
        };
        let mut renamed = 0;
        for session in sessions {
            // Live only (the doc's contract): a dead row's missing transcript
            // was re-probed (row fetch + rollout walk) every sweep, forever.
            if !title_eligible(&session) || !self.is_live(&session.id) {
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
        if self
            .title_probe
            .get(&s.id)
            .is_some_and(|p| p.resolved || p.gave_up)
        {
            return Ok(false);
        }
        let Some(path) = self.activity_artifact(&s.id).await else {
            // Transcript not on disk yet (id just captured, first turn pending).
            return Ok(false);
        };
        // Async stat: this runs every ~20 s per candidate on a runtime worker.
        let mtime = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .ok();
        // Nothing new since the last look — skip the re-parse.
        if let Some(prev) = self.title_probe.get(&s.id) {
            if mtime.is_some() && prev.mtime == mtime {
                return Ok(false);
            }
        }
        let provider = s.provider.clone();
        let path_owned = path.clone();
        let read = tokio::task::spawn_blocking(move || read_provider_title(&provider, &path_owned))
            .await
            .unwrap_or_default();
        let title = read.title;
        // Record the mtime we just parsed at so the next sweep can skip it.
        {
            let mut probe = self.title_probe.entry(s.id.clone()).or_default();
            probe.mtime = mtime;
            // A found title means the (stable) first prompt exists — done either
            // way, whether it differs from the current name or already matches.
            if title.is_some() {
                probe.resolved = true;
            } else if read.capped {
                probe.gave_up = true;
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
    /// WS viewer — and KILL every idle, unattached background session that can
    /// never be suspended (not resumable), once it has sat idle past the much
    /// longer [`REAP_UNRESUMABLE_GRACE`] (see [`should_reap_unresumable`]).
    /// Working sessions, attached sessions and the user's own (foreground)
    /// non-resumable sessions are never touched. A second pass then enforces
    /// the live-session cap ([`Self::enforce_live_cap`]). Returns the number
    /// reclaimed (suspended + killed + capped).
    ///
    /// **"Idle" means "no PTY output", which is not "done".** An agent that
    /// delegates work and then `sleep`-polls its watchers prints nothing, burns
    /// no descendant CPU and is squarely mid-turn; this sweep used to suspend
    /// exactly that. Five guards now hold a session; the first hold (and every
    /// later CHANGE of guard) is logged at `info` with the guard's name, so the
    /// daemon log explains itself without repeating a line a minute forever:
    ///
    /// - `engine turn`    — an engine turn driver is still polling the
    ///   session ([`Self::hold_for_turn`]): a workflow step in one of the
    ///   oracle's bounded holds is quiet for longer than the idle grace, and
    ///   suspending it there re-ran the whole step in a fresh session.
    /// - `keep_alive`     — the user pinned it (`meta.keep_alive`).
    /// - `origin=manual`  — the user started it from the Agents page
    ///   ([`is_user_started`]) and it has been quiet for less than
    ///   `manual_idle_suspend_secs` ([`MANUAL_IDLE_SUSPEND`]). A process a
    ///   passive open merely resumed ([`Self::is_passive_resume`]) gets no
    ///   such hold. Past the grace the remaining guards decide, as for any
    ///   engine session.
    /// - `turn open`      — the provider's own transcript says a turn is still
    ///   open ([`agent_turn_open`]) **and** that transcript moved inside the
    ///   last [`REAP_UNRESUMABLE_GRACE`], whatever the PTY and the CPU say. An
    ///   older "open" tail is a stuck turn, not a live one, and holding it
    ///   would re-open the very fd leak that grace exists to close.
    /// - `descendant CPU` — the process tree accrued CPU since the last sweep
    ///   (a quiet build/test run).
    ///
    /// The first three are decided by the pure [`sweep_hold`]; the CPU one runs
    /// earlier, off the one process-table pass, before the row is loaded.
    ///
    /// Resilient: a failure on one session is logged and skipped; an unreadable
    /// transcript falls back to the pre-existing decision (never "suspend on
    /// error"); the loop never panics or aborts.
    /// Record (and log) the guard that held `id` in this sweep. A hold repeats
    /// every 60 s for as long as it applies, so only a CHANGE of guard earns an
    /// `info!` line; the steady state stays at `debug!`. `detail` is appended
    /// to the message (e.g. the CPU deltas).
    fn note_hold(&self, id: &Id, guard: &'static str, detail: &str) {
        if self.suspend_hold.insert(id.clone(), guard) == Some(guard) {
            tracing::debug!(session = %id, guard, "idle-suspend: keeping session alive{detail}");
        } else {
            tracing::info!(session = %id, guard, "idle-suspend: keeping session alive{detail}");
        }
    }

    /// A `u64` setting, `None` when unset / unreadable / not a number.
    async fn setting_u64(&self, key: &str) -> Option<u64> {
        let sr = self.settings.as_ref()?;
        sr.get(key).await.ok().flatten().and_then(|v| v.as_u64())
    }

    pub async fn suspend_idle_unattached(&self) -> usize {
        // Read the configurable grace period from settings; fall back to the
        // compiled-in default when not set or when the key is absent.
        let grace = self
            .setting_u64("idle_suspend_grace_secs")
            .await
            .map(Duration::from_secs)
            .unwrap_or(SUSPEND_GRACE);
        // The user-started grace (`0` = never, the pre-r3 rule).
        let manual_grace = match self.setting_u64("manual_idle_suspend_secs").await {
            Some(0) => None,
            Some(secs) => Some(Duration::from_secs(secs)),
            None => Some(MANUAL_IDLE_SUSPEND),
        };
        // Live agent-CLI cap (`0` = none).
        let cap = self
            .setting_u64("max_live_agent_sessions")
            .await
            .map(|n| n as usize)
            .unwrap_or(MAX_LIVE_AGENT_SESSIONS);

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
        // Taken only when a candidate passed the cheap checks below; the
        // verdicts are shared with the cap pass.
        let mut sweep_cpu = SweepCpu::default();

        let mut suspended = 0;
        for (id, last_output, pid) in candidates {
            // Idle: no PTY output for the full grace window.
            if last_output.elapsed() < grace {
                continue;
            }
            // Unattached: nobody is watching — no terminal WS viewer, no
            // chat keep-alive ping within VIEW_HOLD (an open conversation
            // view mounts no terminal; its 60 s touch is its attachment),
            // and no engine turn driver still consuming the turn.
            if self.engine_turn_open(&id) {
                self.note_hold(&id, "engine turn", "");
                continue;
            }
            if self.is_watched(&id) {
                continue;
            }
            // Working-but-quiet guard (see the sweep comment above).
            if let Some(pid) = pid {
                match sweep_cpu.verdict(&self.suspend_cpu, &id, pid).await {
                    // Tree accrued >200ms CPU since the last sweep → in-flight work.
                    CpuVerdict::Growing { prev, now } => {
                        self.note_hold(
                            &id,
                            "descendant CPU",
                            &format!(" (descendants accrued {prev}→{now}ms)"),
                        );
                        continue;
                    }
                    // No baseline yet and descendants exist: measure this sweep,
                    // decide on the next one (60s later).
                    CpuVerdict::Baselining => {
                        self.note_hold(&id, "descendant CPU", " (baselining descendant CPU)");
                        continue;
                    }
                    CpuVerdict::Idle => {}
                }
            }
            let session = match self.repo.get(&id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(session = %id, "idle-suspend: load failed: {e}");
                    continue;
                }
            };
            // Row-derived holds ([`sweep_hold`]). The cheap ones first: a
            // pinned or user-started session needs no transcript I/O, and
            // every idle Agents-page session is one of those.
            let manual = ManualIdle {
                quiet_for: last_output.elapsed(),
                grace: manual_grace,
                passive_resume: self.is_passive_resume(&id),
            };
            let mut hold = sweep_hold(&session, None, None, manual);
            if hold.is_none() {
                // An OPEN AGENT TURN is activity even at zero output and zero
                // CPU (the agent is `sleep`-polling a background watcher). Ask
                // the provider's own transcript, and read its mtime in the same
                // blocking hop so the verdict carries its own freshness.
                // No artifact (shell, psid not captured yet, file gone) is
                // unknown, never "idle": decided exactly as before.
                let (turn_open, artifact_age) = self.turn_state(&session).await;
                hold = sweep_hold(&session, turn_open, artifact_age, manual);
            }
            if let Some(guard) = hold {
                self.note_hold(&id, guard, "");
                continue;
            }
            // Only resumable agent sessions — never lose work for a provider
            // that can't be resumed (shell, or a self-id provider whose id we
            // never captured). claude/codex/agy all qualify once resumable.
            let resumable = session.kind == SessionKind::Agent
                && session.provider_session_id.is_some()
                && self.providers.supports_resume(&session.provider);
            if !resumable {
                // Suspend can never reclaim this one. A background
                // (engine-owned) session loses nothing when killed — its
                // engine already consumed the turn output — so reap it after
                // the much longer [`REAP_UNRESUMABLE_GRACE`] instead of
                // leaking its PTY fds, agent process and MCP sidecar forever.
                // Foreground sessions are the user's own live conversation
                // and are left alone, as before.
                if should_reap_unresumable(&session, last_output.elapsed()) {
                    match self.kill_if_idle(&id, REAP_UNRESUMABLE_GRACE).await {
                        Ok(false) => {
                            tracing::debug!(session = %id, "unresumable reap: session became active or was respawned mid-sweep; skipped");
                        }
                        Ok(true) => {
                            suspended += 1;
                            tracing::info!(
                                session = %id,
                                provider = %session.provider,
                                title = %session.title,
                                "killed idle, unattached, non-resumable background session (freed PTY + agent process)"
                            );
                        }
                        Err(e) => tracing::warn!(session = %id, "unresumable reap failed: {e}"),
                    }
                }
                continue;
            }
            match self.suspend_if_idle(&id, grace, SUSPEND_REASON_IDLE).await {
                Ok(false) => {
                    tracing::debug!(session = %id, "idle-suspend: session became active or was respawned mid-sweep; skipped");
                }
                Ok(true) => {
                    suspended += 1;
                    self.viewed.remove(&id);
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
            + self
                .enforce_live_cap(cap, grace.min(CAP_MIN_QUIET), &mut sweep_cpu)
                .await
    }

    /// The live-session cap (`max_live_agent_sessions`, r3-05-01): while more
    /// than `cap` agent CLIs are live, suspend the least-recently-used ones —
    /// oldest of (last PTY output, last attach / detach / input / chat ping)
    /// first. The cap overrides the user-started idle grace, but NEVER takes
    /// a session that is watched (terminal viewer, chat ping, engine turn
    /// driver), pinned, `Working`, printed inside `min_quiet`, whose process
    /// tree is burning CPU, whose provider transcript shows a fresh open turn,
    /// or that cannot be resumed. When every session over the cap is held the
    /// cap simply stays exceeded until the next sweep: it is soft by design.
    async fn enforce_live_cap(
        &self,
        cap: usize,
        min_quiet: Duration,
        sweep_cpu: &mut SweepCpu,
    ) -> usize {
        if cap == 0 || self.live.len() <= cap {
            return 0;
        }
        let snapshot: Vec<(Id, std::time::Instant, Option<u32>)> = self
            .live
            .iter()
            .map(|e| (e.key().clone(), e.value().last_output_at(), e.value().pid()))
            .collect();
        let mut agents = Vec::with_capacity(snapshot.len());
        for (id, last_output, pid) in snapshot {
            let Ok(session) = self.repo.get(&id).await else {
                continue;
            };
            if !counts_toward_cap(&session) {
                continue;
            }
            let last_used = self
                .last_touch
                .get(&id)
                .map(|t| (*t).max(last_output))
                .unwrap_or(last_output);
            agents.push((last_used, last_output, pid, session));
        }
        let mut excess = cap_excess(agents.len(), cap);
        if excess == 0 {
            return 0;
        }
        agents.sort_by_key(|(last_used, ..)| *last_used);
        let mut freed = 0;
        for (_, last_output, pid, session) in agents {
            if excess == 0 {
                break;
            }
            let id = session.id.clone();
            if last_output.elapsed() < min_quiet || self.is_watched(&id) {
                continue;
            }
            // Same descendant-CPU rule as the idle pass — and the SAME
            // verdict when the idle pass already sampled this session.
            if let Some(pid) = pid {
                if sweep_cpu.verdict(&self.suspend_cpu, &id, pid).await != CpuVerdict::Idle {
                    continue;
                }
            }
            let resumable = session.provider_session_id.is_some()
                && self.providers.supports_resume(&session.provider);
            let mut hold = cap_hold(&session, resumable, None, None);
            if hold.is_none() {
                let (turn_open, artifact_age) = self.turn_state(&session).await;
                hold = cap_hold(&session, resumable, turn_open, artifact_age);
            }
            if let Some(guard) = hold {
                tracing::debug!(session = %id, guard, "live-session cap: keeping session alive");
                continue;
            }
            match self
                .suspend_if_idle(&id, min_quiet, SUSPEND_REASON_CAP)
                .await
            {
                Ok(true) => {
                    excess -= 1;
                    freed += 1;
                    self.viewed.remove(&id);
                    tracing::info!(
                        session = %id,
                        provider = %session.provider,
                        title = %session.title,
                        cap,
                        "live-session cap: suspended least-recently-used session (stays resumable)"
                    );
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(session = %id, "live-session cap: suspend failed: {e}"),
            }
        }
        if excess > 0 {
            tracing::info!(
                cap,
                over = excess,
                "live-session cap exceeded; every remaining session is in use"
            );
        }
        freed
    }

    /// The provider transcript's open-turn verdict and the transcript's age,
    /// read in one blocking hop (see [`agent_turn_open`]). `(None, None)` when
    /// the session has no readable artifact.
    async fn turn_state(&self, session: &Session) -> (Option<bool>, Option<Duration>) {
        match self.activity_artifact(&session.id).await {
            Some(artifact) => {
                let provider = session.provider.clone();
                tokio::task::spawn_blocking(move || {
                    let age = std::fs::metadata(&artifact)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok());
                    (agent_turn_open(&provider, &artifact), age)
                })
                .await
                .unwrap_or((None, None))
            }
            None => (None, None),
        }
    }

    /// Opt-in auto-archive: archive every non-archived agent session whose
    /// `last_active_at` is older than the configured number of days
    /// (`session_auto_archive_days` setting; absent or 0 = OFF, the default).
    /// Never touches live PTYs, attached sessions, or `keep_alive`-pinned
    /// rows; archive keeps the row + history, so nothing is lost — the session
    /// just moves to the sidebar's "Archived" section (and stays unarchivable).
    /// Returns the number archived.
    pub async fn auto_archive_stale(&self) -> usize {
        let days = match &self.settings {
            Some(sr) => match sr.get("session_auto_archive_days").await {
                Ok(Some(v)) => v.as_u64().unwrap_or(0),
                _ => 0,
            },
            None => 0,
        };
        if days == 0 {
            return 0;
        }
        // A day count too large for chrono (or for `i64`) means "never":
        // auto-archive stays off rather than panicking or, via an `as i64`
        // wrap, putting the cutoff in the future and archiving everything.
        let Some(cutoff) = auto_archive_cutoff(chrono::Utc::now(), days) else {
            tracing::warn!(
                days,
                "auto-archive: day count out of range; treating as off"
            );
            return 0;
        };
        let mut archived = 0;
        let mut after_id = String::new();
        loop {
            let candidates = match self
                .repo
                .auto_archive_candidate_ids(cutoff, &after_id, 128)
                .await
            {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::warn!("auto-archive: candidate page failed: {e}");
                    break;
                }
            };
            let Some(last) = candidates.last() else {
                break;
            };
            // Advance over skipped or failed rows too, otherwise a page of
            // pinned/attached sessions would starve every later candidate.
            after_id.clone_from(last);
            for id in candidates {
                if self.is_live(&id) || self.is_attached(&id) {
                    continue;
                }
                match self.archive_if_stale(&id, cutoff).await {
                    Ok(false) => {
                        tracing::debug!(session = %id, "auto-archive: session became active mid-sweep; skipped")
                    }
                    Ok(true) => {
                        archived += 1;
                        tracing::info!(session = %id, "auto-archived session idle for over {days} day(s)");
                    }
                    Err(e) => tracing::warn!(session = %id, "auto-archive failed: {e}"),
                }
            }
        }
        archived
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
            // Never use the default CLI's history to declare a named account's
            // conversation missing. Account sessions are retained explicitly.
            if s.meta.get("account_id").and_then(|v| v.as_str()).is_some() {
                continue;
            }
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
            let verdict =
                crate::lifecycle::check_resumability_async(home, &s.provider, &s.cwd, psid).await;
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

    /// Kill `ids`' live PTYs and mark them exited (see [`Self::shutdown_all`]).
    ///
    /// Each session is retired under its resume lock, like every other
    /// lifecycle op: the status task writes only under that lock and only
    /// while `live` still maps its handle, so taking it here orders a tick
    /// that already passed that check BEFORE our `Exited`. Without it, a
    /// tick's `Working`/`Idle` could land after `Exited` and the next boot
    /// counted the killed (engine) session as a suspended live one.
    ///
    /// The lock wait is BOUNDED (review S1-15): a resume in flight can hold
    /// it for seconds (`RETIRED_EXIT_WAIT`, the pre-spawn hook, the spawn),
    /// and the whole shutdown sits under the daemon's 12 s teardown budget —
    /// two slow resumes used to exhaust it, leaving every later session
    /// neither killed nor marked exited. Past [`SHUTDOWN_LOCK_WAIT`] (or the
    /// shared [`SHUTDOWN_LOCK_BUDGET`]) the session is retired without the
    /// lock; the in-flight resume then refuses to spawn (`shutting_down`).
    async fn shutdown_ids(&self, ids: Vec<Id>) -> usize {
        let count = ids.len();
        let budget_end = tokio::time::Instant::now() + SHUTDOWN_LOCK_BUDGET;
        for id in ids {
            let lock = self.resume_lock(&id);
            let wait = budget_end
                .saturating_duration_since(tokio::time::Instant::now())
                .min(SHUTDOWN_LOCK_WAIT);
            let _guard = match tokio::time::timeout(wait, lock.lock()).await {
                Ok(guard) => Some(guard),
                Err(_) => {
                    tracing::warn!(session = %id, "shutdown: resume lock busy — retiring the session without it");
                    None
                }
            };
            self.networks.clear(&id);
            if let Some((_, handle)) = self.live.remove(&id) {
                let _ = handle.kill();
            }
            // Best-effort status update; ignore errors during shutdown.
            let _ = self.repo.update_status(&id, SessionStatus::Exited).await;
            if let Ok(s) = self.repo.get(&id).await {
                self.retire_credentials(&s).await;
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
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        let session = self.repo.get(id).await?;
        self.archive_locked(id, session).await
    }

    /// [`Self::archive`] for the auto-archive sweep: archive only if the
    /// session is STILL stale under its resume lock (review S1-17). The
    /// sweep's candidate query runs outside the lock, so a
    /// session the user opened as the sweep reached it was resumed and then
    /// immediately killed and archived. `Ok(false)` = no longer stale.
    async fn archive_if_stale(
        &self,
        id: &Id,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        let session = self.repo.get(id).await?;
        if !is_auto_archive_candidate(&session, cutoff) || self.is_live(id) || self.is_attached(id)
        {
            return Ok(false);
        }
        self.archive_locked(id, session).await.map(|_| true)
    }

    /// The archive body; the caller holds `id`'s resume lock.
    async fn archive_locked(&self, id: &Id, session: Session) -> Result<Session> {
        self.networks.clear(id);
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.publish_grid(id);
        self.retire_credentials(&session).await;
        self.repo.set_archived(id, true).await?;
        self.repo.update_status(id, SessionStatus::Exited).await?;
        self.record_lifecycle(&session, "Archived");
        // Preserve the status event for lifecycle consumers.
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Exited,
        });
        let session = self.repo.get(id).await?;
        let _ = self.events.send(Event::SessionArchiveChanged {
            session: session.clone(),
        });
        Ok(session)
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
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        self.repo.set_archived(id, false).await?;
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        let session = self.repo.get(id).await?;
        self.record_lifecycle(&session, "Unarchived");
        let _ = self.events.send(Event::SessionArchiveChanged {
            session: session.clone(),
        });
        Ok(session)
    }

    /// Kill the PTY, delete the DB row and emit `SessionRemoved`.
    pub async fn remove(&self, id: &Id) -> Result<()> {
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        let session = self.repo.get(id).await?;
        self.networks.clear(id);
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.delete(id).await?;
        // Its on-disk files (pasted images) go with the row.
        self.remove_session_files(id).await;
        self.room_authority.prune_idle(id);
        self.ingest_tokens.remove(id);
        self.title_probe.remove(id);
        // Per-session sweep / viewer bookkeeping (r3-05-08: these outlived
        // the row forever).
        self.forget_sweep_state(id);
        self.last_touch.remove(id);
        self.viewed.remove(id);
        self.size_owner.remove(id);
        self.grids.remove(id);
        // Also ends a pending provider-id capture task (it exits on the missing
        // probe entry at its next poll).
        self.capture_probes.remove(id);
        // Revoke the per-session token minted for the `otto` MCP tool server, so
        // its read-only credential dies with the session (best-effort).
        if let Err(e) = self.revoke_mcp_token(&session.created_by, id).await {
            tracing::warn!(session = %id, "revoke removed session credentials: {e}");
        }
        // Drop the per-session disconnect sender; any attached viewers were
        // already evicted by the terminate path before removal.
        self.evict.remove(id);
        self.retiring.remove(id);
        // Keep the same lock while another lifecycle operation is queued. A
        // waiter will observe the deleted row instead of spawning after removal.
        if Arc::strong_count(&lock) == 2 {
            self.resume_locks.remove(id);
        }
        // An in-flight submitter keeps its Arc; it fails on the missing PTY.
        self.submit_locks.remove(id);
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
        // Same per-session serialization as `ensure_live` (which calls
        // `restart_locked` under its own guard) — see `resume_locks`.
        let lock = self.resume_lock(id);
        let _guard = lock.lock().await;
        // An explicit restart is deliberate use, never a passive resume.
        self.passive_resume.remove(id);
        self.touch(id);
        self.restart_locked(id, spec_override, false).await
    }

    /// [`Self::restart`] body; caller MUST hold this session's resume lock.
    /// `passive` marks the new process a passive resume
    /// ([`Self::passive_resume`]) before it is published in `live`.
    async fn restart_locked(
        &self,
        id: &Id,
        spec_override: Option<CommandSpec>,
        passive: bool,
    ) -> Result<Session> {
        let _mcp_activation = crate::mcp::activation_gate().read().await;
        let mut session = self.repo.get(id).await?;
        let account = self
            .resolve_account(&session.created_by, &session.provider, &session.meta)
            .await?;
        let project_context = self
            .repo
            .project_context(&session.workspace_id, &session.meta)
            .await?;
        let workspace = if session.kind == SessionKind::Agent {
            Some(self.repo.workspace(&session.workspace_id).await?)
        } else {
            None
        };
        if session.archived {
            return Err(Error::Conflict(
                "session is archived — unarchive it first".into(),
            ));
        }
        // Checked BEFORE the old process is retired: a refusal leaves it as is.
        if session.kind == SessionKind::Agent {
            if let Some(msg) = self.missing_cwd_refusal(&session) {
                return Err(Error::Conflict(msg));
            }
        }
        let mut assigned_provider_id = None;
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
                let sid = session
                    .provider_session_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let mut spec =
                    self.providers
                        .build_spec(&session.provider, &sid, &session.cwd, resume)?;
                // Append --add-dir and --model args from session.meta.
                spec.args
                    .extend(add_dir_args(&session.provider, &session.meta));
                spec.args.extend(model_args(
                    self.providers
                        .model_args_template(&session.provider)
                        .as_deref(),
                    &session.meta,
                ));
                spec.args
                    .extend(lean_turn_args(&session.provider, &session.meta));
                // A fresh restart (e.g. a legacy row or an override-created
                // session) must use and retain the same UUID contract as create.
                // Commit it after spawn succeeds: otherwise a failed spawn
                // would make the next attempt resume a nonexistent conversation.
                if session.provider_session_id.is_none()
                    && self.providers.supports_resume(&session.provider)
                    && !self.providers.captures_session_id(&session.provider)
                {
                    assigned_provider_id = Some(sid);
                }
                read_only_strip(&session.provider, &session.meta, &mut spec.args);
                spec.args
                    .extend(read_only_args(&session.provider, &session.meta));
                spec
            }
        };

        // Always re-materialize from current workspace settings on restart,
        // including spec overrides and review sessions. Reusing the persisted
        // bundle would silently retain pre-edit shared knowledge.
        if let (Some(hook), Some(workspace)) = (&self.pre_spawn_hook, workspace) {
            let hook = Arc::clone(hook);
            let cwd = session.cwd.clone();
            let provider = session.provider.clone();
            let context = (account.is_some() || project_context.is_some()).then(|| {
                otto_core::hooks::SessionSpawnContext {
                    namespace: session.id.clone(),
                    provider_home: account.as_ref().map(|(_, home)| home.clone()),
                    extra_context_md: project_context.clone().unwrap_or_default(),
                }
            });
            let injection = tokio::task::spawn_blocking(move || match context {
                Some(context) => hook.before_spawn_session(&workspace, &cwd, &provider, &context),
                None => hook.before_spawn(&workspace, &cwd, &provider),
            })
            .await
            .unwrap_or_default();
            spec.args.extend(injection.args);
            spec.env.extend(injection.env);
        }

        if let Some((account, _)) = &account {
            crate::accounts::validate_account(&spec, account)?;
        }
        // Keep an existing PTY/tunnel alive if replacement forwarding fails.
        let network = self.networks.prepare(&session).await?;
        self.networks.clear(id);
        // From here on the old process is retired: every failure below must
        // leave the row resumable instead of `running` with nothing live (the
        // old status task sees itself superseded and writes nothing).
        let mut retired = self.retiring.remove(id).map(|(_, rx)| rx);
        let had_live = match self.live.remove(id) {
            Some((_, handle)) => {
                let _ = handle.kill();
                retired = Some(handle.on_exit());
                true
            }
            None => false,
        };
        // Never run two CLIs on one conversation: give the killed one (this
        // one, or a just-suspended one) a bounded window to actually exit.
        if let Some(rx) = retired {
            await_retired_exit(id, rx).await;
        }
        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            if account.is_none() {
                crate::trust::ensure_trusted_async(&session.provider, &session.cwd).await;
            }
            let otto_tools = self.maybe_enable_otto_tools(&session).await;
            let codex_otto = !otto_tools.args.is_empty();
            spec.args.extend(otto_tools.args);
            spec.env.extend(otto_tools.env);
            // Re-run the same MCP reconcile as create(): a resumed session
            // picks up server enable/disable changes made since its original
            // spawn instead of keeping the stale `.mcp.json` forever.
            let codex_user_args = self
                .sync_workspace_mcp(&session, otto_tools.server, codex_otto)
                .await;
            spec.args.extend(codex_user_args);
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
        if let Some((account, home)) = &account {
            if let Err(error) = crate::accounts::apply_account(&mut spec, account, home) {
                let _ = self
                    .revoke_mcp_token(&session.created_by, &session.id)
                    .await;
                if had_live {
                    self.respawn_failed(&session, &error).await;
                }
                return Err(error);
            }
        }
        if let Some(network) = &network {
            spec.env
                .retain(|(key, _)| !network.env.iter().any(|(mapped, _)| mapped == key));
            spec.env.extend(network.env.clone());
        }
        self.apply_sandbox(&mut spec, &session).await;
        // Blocking-pool fork/exec, mirroring create(): idle-resume runs on the
        // terminal-attach path, so a blocked async worker here is user-visible.
        // Shutdown has begun (review S1-15): a respawn completing now would
        // land after the teardown iterated past this session — a live CLI
        // nobody kills, its row later counted as a suspended live one.
        let spawned = if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            Err(Error::Conflict("the daemon is shutting down".into()))
        } else {
            self.spawn_session_pty(&session, spec.clone(), grid_cols, grid_rows)
                .await
        };
        let spawned = match spawned {
            Ok(handle) if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) => {
                let _ = handle.kill();
                Err(Error::Conflict("the daemon is shutting down".into()))
            }
            other => other,
        };
        let handle = match spawned {
            Ok(handle) => Arc::new(handle),
            Err(e) => {
                let _ = self
                    .revoke_mcp_token(&session.created_by, &session.id)
                    .await;
                if had_live {
                    self.respawn_failed(&session, &e).await;
                }
                return Err(e);
            }
        };
        if let Some(sid) = assigned_provider_id {
            if let Err(error) = self.repo.set_provider_session(id, &sid).await {
                let _ = handle.kill();
                self.respawn_failed(&session, &error).await;
                return Err(error);
            }
            session.provider_session_id = Some(sid);
        }
        self.networks
            .activate(id.clone(), network, handle.on_exit());
        if passive {
            self.passive_resume.insert(id.clone(), ());
        }
        // A failed/expired initial capture must not strand a fresh restarted
        // provider forever. Register before publishing live so its first input
        // cannot race ahead of the probe. Generation checks retire older tasks.
        if session.kind == SessionKind::Agent
            && session.provider_session_id.is_none()
            && self.providers.captures_session_id(&session.provider)
        {
            self.spawn_session_id_capture(&session);
        }
        self.live.insert(id.clone(), Arc::clone(&handle));
        self.publish_grid(id);
        self.repo.update_status(id, SessionStatus::Running).await?;
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Running,
        });
        // Live again: the dormant reason no longer applies (review A4).
        if session.meta.get("suspended").is_some() {
            self.stamp_suspended(&session, None).await;
        }
        self.record_lifecycle(&session, "Session resumed");
        self.start_status_task(
            id.clone(),
            session.workspace_id,
            session.provider,
            handle,
            false,
        );
        self.repo.get(id).await
    }

    /// Re-adopt the PTY holders the previous daemon run left behind (see
    /// [`SESSION_PERSISTENCE_SETTING`]): for each live holder whose session
    /// still exists, is not archived and is one the user started, re-attach
    /// (replaying its screen + scrollback), re-register it as live, restore
    /// its ingest token and mark it `running`. Every other holder is ended —
    /// one for a session that is gone, archived or engine-owned, a duplicate
    /// (only the newest holder per session is kept), one whose child already
    /// exited while no daemon was attached (the session then restores like
    /// any other), or one speaking an incompatible protocol version. Stale
    /// socket files are removed. Returns the adopted session ids. Runs
    /// regardless of the setting: switching persistence off must not orphan
    /// sessions that are already running.
    pub async fn adopt_holders(&self) -> Vec<Id> {
        let Some(config) = self.holders.clone() else {
            return Vec::new();
        };
        let sockets = config.sockets();
        if sockets.is_empty() {
            return Vec::new();
        }
        // Handshakes and snapshot replays are blocking I/O + parsing.
        let attached = tokio::task::spawn_blocking(move || {
            let mut out: Vec<PtyHandle> = Vec::new();
            for path in sockets {
                match PtyHandle::adopt(&path) {
                    Ok(handle) => out.push(handle),
                    Err(otto_pty::AdoptError::Stale) => {
                        tracing::debug!(socket = %path.display(), "pty holder gone; removed its stale socket");
                    }
                    Err(otto_pty::AdoptError::Incompatible(info)) => {
                        tracing::warn!(
                            socket = %path.display(),
                            holder = %info.holder_version,
                            "pty holder speaks protocol {}.{} — ending it (its session resumes the old way)",
                            info.proto_major, info.proto_minor
                        );
                        let _ = otto_pty::holder::terminate(&path);
                    }
                    Err(otto_pty::AdoptError::Failed(e)) => {
                        // Unadoptable but maybe alive: end it, or reopening its
                        // session would start a SECOND process (a resumed
                        // agent forking the conversation it is still in).
                        tracing::warn!(socket = %path.display(), "pty holder not adoptable — ending it: {e}");
                        let _ = otto_pty::holder::terminate(&path);
                    }
                }
            }
            out
        })
        .await
        .unwrap_or_default();

        // Newest holder per session wins; the rest are ended below.
        let mut by_session: std::collections::HashMap<Id, Vec<PtyHandle>> = Default::default();
        for handle in attached {
            let sid = handle
                .holder()
                .and_then(|i| i.meta.get("session_id"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            match sid {
                Some(sid) => by_session.entry(sid).or_default().push(handle),
                None => {
                    tracing::warn!("pty holder without a session id — ending it");
                    drop(handle); // not detached: kills the child, releases the holder
                }
            }
        }
        let mut adopted = Vec::new();
        for (sid, mut handles) in by_session {
            handles.sort_by_key(|h| {
                std::cmp::Reverse(h.holder().map(|i| i.started_at_ms).unwrap_or(0))
            });
            let mut iter = handles.into_iter();
            let Some(handle) = iter.next() else { continue };
            for dup in iter {
                tracing::warn!(session = %sid, "duplicate pty holder for one session — ending the older one");
                drop(dup);
            }
            if self.adopt_one(&sid, handle).await {
                adopted.push(sid);
            }
        }
        adopted
    }

    /// Decide on one adoptable holder handle for `sid` (the body of
    /// [`Self::adopt_holders`]): re-register it as live (`true`), end it, or
    /// — on a transient lookup error — detach it and record it in
    /// `deferred_holders` for a retry.
    async fn adopt_one(&self, sid: &Id, handle: PtyHandle) -> bool {
        let sid = sid.clone();
        if handle.has_exited() {
            tracing::info!(session = %sid, "session's process exited while the daemon was down");
            self.deferred_holders.remove(&sid);
            drop(handle);
            return false;
        }
        // A transient DB error gets a couple of short retries before the
        // holder is left alone (see below).
        let mut lookup = self.repo.get(&sid).await;
        for backoff_ms in [100, 500] {
            if matches!(lookup, Ok(_) | Err(Error::NotFound(_))) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            lookup = self.repo.get(&sid).await;
        }
        let session = match lookup {
            Ok(s) => s,
            Err(Error::NotFound(_)) => {
                tracing::warn!(session = %sid, "pty holder for a session that no longer exists — ending it");
                self.deferred_holders.remove(&sid);
                drop(handle);
                return false;
            }
            Err(e) => {
                // A transient DB failure (busy, I/O) is no proof the
                // session is gone: killing here would end a live agent
                // mid-turn. Leave the holder running (detached, not
                // killed on drop) and record it: the boot dormant pass
                // and credential sweep skip it, and a retry (or the
                // first open) adopts it instead of forking a second CLI.
                tracing::warn!(session = %sid, "pty holder: session lookup failed ({e}) — leaving it running for a later adoption");
                if let Some(path) = handle.holder_socket().map(|p| p.to_path_buf()) {
                    self.deferred_holders.insert(sid.clone(), path);
                }
                handle.detach();
                drop(handle);
                return false;
            }
        };
        if session.archived || !is_user_started(&session) || self.is_live(&sid) {
            tracing::info!(session = %sid, "pty holder not re-adoptable (archived / engine-owned / already live) — ending it");
            self.deferred_holders.remove(&sid);
            drop(handle);
            return false;
        }
        if let Some(token) = handle
            .holder()
            .and_then(|i| i.meta.get("ingest_token"))
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
        {
            self.ingest_tokens.insert(sid.clone(), token.to_string());
        }
        let pid = handle.pid();
        let handle = Arc::new(handle);
        self.live.insert(sid.clone(), Arc::clone(&handle));
        self.publish_grid(&sid);
        // Not activity: a restart must not reset the idle clock (A14).
        if let Err(e) = self
            .repo
            .update_status_keep_activity(&sid, SessionStatus::Running)
            .await
        {
            tracing::warn!(session = %sid, "re-adopted session: status update failed: {e}");
        }
        let _ = self.events.send(Event::SessionStatus {
            session_id: sid.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Running,
        });
        self.record_lifecycle(
            &session,
            "Reattached after a daemon restart (process kept running)",
        );
        self.start_status_task(
            sid.clone(),
            session.workspace_id.clone(),
            session.provider.clone(),
            handle,
            true,
        );
        // A codex/agy id capture that was still pending died with the old
        // daemon: re-arm it (it waits for the next input, as at spawn).
        if session.kind == SessionKind::Agent
            && session.provider_session_id.is_none()
            && self.providers.captures_session_id(&session.provider)
        {
            self.spawn_session_id_capture(&session);
        }
        tracing::info!(
            session = %sid,
            provider = %session.provider,
            pid = ?pid,
            "re-adopted live session from its pty holder"
        );
        self.deferred_holders.remove(&sid);
        true
    }

    /// Retry the holders [`Self::adopt_holders`] deferred on a transient DB
    /// error. Returns the ids adopted now. A holder that is gone (or ended)
    /// is forgotten; one still undecidable stays deferred.
    pub async fn retry_deferred_adoptions(&self) -> Vec<Id> {
        let pending: Vec<(Id, std::path::PathBuf)> = self
            .deferred_holders
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        let mut adopted = Vec::new();
        for (sid, _) in pending {
            // Under the session's resume lock, like `ensure_live` (S1-305):
            // otherwise a user opening the session mid-lookup finds no
            // deferred entry and nothing live, resumes a second CLI, and this
            // adoption then "ends" the original holder mid-turn. Re-read the
            // entry under the lock — an open may have decided it already.
            let lock = self.resume_lock(&sid);
            let _guard = lock.lock().await;
            let Some(path) = self.deferred_holders.get(&sid).map(|p| p.clone()) else {
                continue;
            };
            if self.adopt_deferred(&sid, path).await {
                adopted.push(sid);
            }
        }
        adopted
    }

    /// One deferred holder: adopt it if its session can be decided now.
    /// Callers hold the session's resume lock. The `deferred_holders` entry is
    /// kept until the holder is decided (adopted, gone, or ended), so the
    /// dormant pass and the credential sweep keep skipping it meanwhile.
    async fn adopt_deferred(&self, sid: &Id, path: std::path::PathBuf) -> bool {
        if self.is_live(sid) {
            self.deferred_holders.remove(sid);
            return false;
        }
        let res = tokio::task::spawn_blocking(move || PtyHandle::adopt(&path)).await;
        match res {
            Ok(Ok(handle)) => {
                if handle.has_exited() {
                    self.deferred_holders.remove(sid);
                    drop(handle);
                    return false;
                }
                // `adopt_one` drops the entry once it has decided, and keeps
                // it when this lookup fails again.
                self.adopt_one(sid, handle).await
            }
            Ok(Err(otto_pty::AdoptError::Failed(e))) => {
                tracing::debug!(session = %sid, "deferred pty holder still unreachable: {e}");
                false
            }
            Ok(Err(_)) | Err(_) => {
                // Gone (stale socket) or incompatible: nothing to adopt.
                self.deferred_holders.remove(sid);
                false
            }
        }
    }

    /// Retry deferred adoptions in the background, 5 s and 30 s after boot.
    pub fn spawn_deferred_adoption_retries(self: &Arc<Self>) {
        if self.deferred_holders.is_empty() {
            return;
        }
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            for wait in [Duration::from_secs(5), Duration::from_secs(25)] {
                tokio::time::sleep(wait).await;
                let Some(this) = this.upgrade() else { return };
                let ids = this.retry_deferred_adoptions().await;
                if !ids.is_empty() {
                    tracing::info!(count = ids.len(), "re-adopted deferred pty holders");
                }
                if this.deferred_holders.is_empty() {
                    return;
                }
            }
        });
    }

    /// Daemon shutdown. With session persistence on
    /// ([`SESSION_PERSISTENCE_SETTING`]), sessions running in a PTY holder are
    /// DETACHED — left running, status untouched, credentials kept — for the
    /// next daemon run to re-adopt; every other live PTY is killed and marked
    /// exited exactly as [`Self::shutdown_all`] does. With it off, everything
    /// is killed. Returns `(killed, kept_running)`.
    pub async fn shutdown_for_restart(&self) -> (usize, usize) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let persist = self.persistence_enabled().await;
        if persist {
            // No teardown path from here on may end a held session.
            otto_pty::holder::detach_all_on_drop();
        }
        let mut keep = Vec::new();
        let mut kill = Vec::new();
        for entry in self.live.iter() {
            if persist && entry.value().holder().is_some() {
                entry.value().detach();
                keep.push(entry.key().clone());
            } else {
                kill.push(entry.key().clone());
            }
        }
        // Untrack the detached ones: their status tasks then see a superseded
        // handle and leave the row alone — it stays as it is (live) for the
        // next daemon run, which re-adopts the process.
        for id in &keep {
            self.networks.clear(id);
            self.live.remove(id);
        }
        (self.shutdown_ids(kill).await, keep.len())
    }

    /// Daemon-boot restore. We deliberately do NOT respawn agent processes here:
    /// keeping every historical session resident would cost ~200 MB each. Instead
    /// every restorable session is marked `Reconnectable` (0 memory) and resumed
    /// lazily by [`Self::ensure_live`] the moment a client opens it — claude/codex
    /// keep their conversation in the on-disk JSONL, so `--resume` restores it in
    /// full. `_fallback_cwd` is kept for signature stability (used by resume).
    ///
    /// Sessions whose process outlived the previous daemon in a PTY holder
    /// ([`SESSION_PERSISTENCE_SETTING`]) are re-adopted FIRST
    /// ([`Self::adopt_holders`]): they come back `running` — same process,
    /// same screen — and keep their credentials. Only the rest go dormant.
    ///
    /// The dormant pass is ONE set-based statement pair
    /// ([`SessionsRepo::mark_dormant_except`]): rows already `reconnectable`
    /// are untouched and `last_active_at` is never stamped (a restart is not
    /// activity — review P1). Only the rows that changed are broadcast. The
    /// returned [`RestoreSummary`] feeds the UI's "Otto restarted" notice.
    pub async fn restore_all(
        &self,
        _fallback_cwd: &(dyn Fn(&Id) -> Option<String> + Send + Sync),
    ) -> Result<RestoreSummary> {
        let adopted = self.adopt_holders().await;
        // Deferred holders (transient lookup error) still run their agent:
        // neither revoke its credential nor mark its row dormant (review
        // S1-09) — a dormant row's next open would resume a SECOND CLI.
        let mut keep = adopted.clone();
        keep.extend(self.deferred_holders.iter().map(|e| e.key().clone()));
        self.expire_orphaned_session_credentials(&keep).await;
        let at = chrono::Utc::now().to_rfc3339();
        let pass = self.repo.mark_dormant_except(&keep, &at).await?;
        for (id, workspace_id, meta) in &pass.suspended {
            let _ = self.events.send(Event::SessionStatus {
                session_id: id.clone(),
                workspace_id: workspace_id.clone(),
                status: SessionStatus::Reconnectable,
            });
            let _ = self.events.send(Event::SessionMetaUpdated {
                session_id: id.clone(),
                workspace_id: workspace_id.clone(),
                meta: meta.clone(),
            });
        }
        for (id, workspace_id) in &pass.resumable {
            let _ = self.events.send(Event::SessionStatus {
                session_id: id.clone(),
                workspace_id: workspace_id.clone(),
                status: SessionStatus::Reconnectable,
            });
        }
        Ok(RestoreSummary {
            kept_running: adopted.len(),
            suspended: pass.suspended.len(),
        })
    }

    /// A restart / resume retired the old process and then failed to start the
    /// new one: nothing is live any more, so the row must not keep saying
    /// `running`. Agent sessions become `Reconnectable` (stamped like a
    /// release, so the pane offers Resume); connection sessions — reopened
    /// only via their connection — become `Exited`.
    async fn respawn_failed(&self, session: &Session, cause: &Error) {
        let id = &session.id;
        self.forget_sweep_state(id);
        let status = if session.kind == SessionKind::Agent {
            SessionStatus::Reconnectable
        } else {
            SessionStatus::Exited
        };
        // Emit only what was stored: a status event for a write that failed
        // would show the pane a state the row (and the next reload) doesn't.
        if let Err(e) = self.repo.update_status(id, status).await {
            tracing::warn!(session = %id, "respawn failed ({cause}); status write failed too: {e}");
            return;
        }
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status,
        });
        if status == SessionStatus::Reconnectable {
            self.stamp_suspended_with(
                session,
                Some(SUSPEND_REASON_RESPAWN_FAILED),
                Some(&cause.to_string()),
            )
            .await;
        }
    }

    /// Merge `meta.suspended = {reason, at: now}` (or remove it with `None`)
    /// and broadcast the new meta. Best-effort: a failed write only loses the
    /// pane's explanation, never the suspend itself.
    async fn stamp_suspended(&self, session: &Session, reason: Option<&'static str>) {
        self.stamp_suspended_with(session, reason, None).await;
    }

    /// [`Self::stamp_suspended`] that also records `error` (the cause of a
    /// failed restart, clipped) as `meta.suspended.error`.
    async fn stamp_suspended_with(
        &self,
        session: &Session,
        reason: Option<&'static str>,
        error: Option<&str>,
    ) {
        let value = match reason {
            Some(reason) => {
                let mut v = serde_json::json!({
                    "reason": reason,
                    "at": chrono::Utc::now().to_rfc3339(),
                });
                if let Some(error) = error {
                    v["error"] = otto_core::text::clip_chars(error, 300).into();
                }
                v
            }
            None => serde_json::Value::Null,
        };
        let patch = serde_json::json!({ "suspended": value });
        if let Err(e) = self.repo.merge_meta(&session.id, &patch).await {
            tracing::debug!(session = %session.id, "stamp meta.suspended: {e}");
            return;
        }
        if let Ok(updated) = self.repo.get(&session.id).await {
            let _ = self.events.send(Event::SessionMetaUpdated {
                session_id: updated.id.clone(),
                workspace_id: updated.workspace_id.clone(),
                meta: updated.meta.clone(),
            });
        }
    }

    /// Boot-time credential sweep (see [`Self::restore_all`]): apart from the
    /// sessions just re-adopted from their PTY holders (`keep` — their agent
    /// is still running with its credential in its environment), no agent
    /// process survives a daemon restart, so every other per-session MCP
    /// credential still valid at boot has no live holder — revoke them all (a
    /// resume mints a fresh one). Also retires the legacy `otto-mcp:<session>`
    /// label tokens minted before managed ownership (thousands of full-owner,
    /// 10-year credentials that nothing revoked). Best-effort; rows are kept.
    async fn expire_orphaned_session_credentials(&self, keep: &[Id]) {
        let Some(auth) = &self.auth else { return };
        match auth.expire_managed_session_tokens_except(keep).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                count = n,
                "revoked per-session MCP credentials left over from the previous daemon run"
            ),
            Err(e) => tracing::warn!("boot credential sweep (managed): {e}"),
        }
        match auth
            .expire_legacy_session_label_tokens(legacy_session_token_cutover())
            .await
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(count = n, "revoked legacy otto-mcp:<session> tokens"),
            Err(e) => tracing::warn!("boot credential sweep (legacy): {e}"),
        }
    }

    /// Per-session status task: every 2s classify working/idle from PTY
    /// activity; on exit mark `exited` and stop. When an [`OutputScanner`] is
    /// configured, also spawns a sibling task that streams the PTY's live
    /// output into the scanner (mid-session re-auth detection).
    /// `readopted`: `handle` is a held PTY re-adopted after a daemon restart.
    /// Until it prints something new, its status ticks are corrections, not
    /// activity, so they must not stamp `last_active_at` (A14) — the first
    /// tick turns the adoption's `Running` into `Idle`.
    fn start_status_task(
        &self,
        id: Id,
        workspace_id: Id,
        provider: String,
        handle: Arc<PtyHandle>,
        readopted: bool,
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
                scanner.on_session_end(&scan_id);
            });
        }

        // Status-write ownership. This task speaks for ONE process incarnation
        // (`handle`); every lifecycle op (suspend / archive / remove / restart /
        // kill / ensure_live) runs under the session's resume lock and, when it
        // replaces or retires the process, untracks the handle from `live` and
        // writes its own final status. So a write from here is only valid while
        // `live` still maps THIS handle, checked and written under that same
        // lock — a late exit or tick of a superseded incarnation can never
        // overwrite a newer decision (suspend's `Reconnectable`, a respawn's
        // `Running`, or a removed row's `SessionRemoved`).
        let repo = self.repo.clone();
        let events = self.events.clone();
        let live = Arc::clone(&self.live);
        let locks = Arc::clone(&self.resume_locks);
        let suspend_cpu = Arc::clone(&self.suspend_cpu);
        let suspend_hold = Arc::clone(&self.suspend_hold);
        let passive_resume = Arc::clone(&self.passive_resume);
        let auth = self.auth.clone();
        let mcp_tokens = Arc::clone(&self.mcp_tokens);
        let attached = Arc::clone(&self.attached);
        let grid_maps = (Arc::clone(&self.grids), Arc::clone(&self.attached_conns));
        let size_owner = Arc::clone(&self.size_owner);
        tokio::spawn(async move {
            let mut exit_rx = handle.on_exit();
            let mut current = SessionStatus::Running;
            // A re-adopted handle's (back-dated) last-output clock: while it
            // still reads this, nothing new happened since the restart.
            let mut adopted_quiet_at = readopted.then(|| handle.last_output_at());
            // Unviewed-history cap (UNVIEWED_HISTORY_AFTER): one in-memory map
            // read per tick; `attach` restores the full cap.
            let mut unviewed_since: Option<std::time::Instant> = None;
            let mut interval = tokio::time::interval(STATUS_TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if attached.get(&id).is_some_and(|n| *n > 0) {
                            unviewed_since = None;
                        } else {
                            let since = *unviewed_since.get_or_insert_with(std::time::Instant::now);
                            if since.elapsed() >= UNVIEWED_HISTORY_AFTER {
                                handle.set_history_cap(otto_pty::UNVIEWED_SCROLLBACK_LINES);
                            }
                        }
                        let next = if handle.last_output_at().elapsed() < WORKING_WINDOW {
                            SessionStatus::Working
                        } else {
                            SessionStatus::Idle
                        };
                        if next != current {
                            // A lifecycle op in flight owns the status right
                            // now: don't wait behind it (a respawn can take
                            // seconds) — retry on the next tick instead.
                            let lock = session_lock(&locks, &id);
                            let Ok(_guard) = lock.try_lock() else {
                                continue;
                            };
                            if !maps_handle(&live, &id, &handle) {
                                // Superseded: the exit arm finishes this task.
                                continue;
                            }
                            current = next;
                            let quiet = adopted_quiet_at
                                .is_some_and(|at| handle.last_output_at() == at);
                            if quiet {
                                let _ = repo.update_status_keep_activity(&id, next).await;
                            } else {
                                adopted_quiet_at = None;
                                let _ = repo.update_status(&id, next).await;
                            }
                            let _ = events.send(Event::SessionStatus {
                                session_id: id.clone(),
                                workspace_id: workspace_id.clone(),
                                status: next,
                            });
                        }
                    }
                    code = wait_exit_code(&mut exit_rx) => {
                        let lock = session_lock(&locks, &id);
                        let guard = lock.lock().await;
                        // Evict the dead handle so its emulator + ring buffer
                        // are dropped (no accumulation across many sessions) —
                        // but ONLY if `live` still maps THIS handle. A respawn
                        // (restart/ensure_live) kills this PTY and inserts a
                        // fresh handle under the same id; without the identity
                        // check, this superseded task's exit would evict the new
                        // handle, orphaning its process (alive but untracked, so
                        // suspend/archive never kill it). See `evict_if_same`.
                        let owned = evict_if_same(&live, &id, &handle);
                        if owned {
                            // A real exit of the current process (the CLI quit,
                            // crashed, or `kill_session` killed it in place).
                            suspend_cpu.remove(&id);
                            suspend_hold.remove(&id);
                            passive_resume.remove(&id);
                            // Its MCP credential has no live holder any more
                            // (a resume mints a fresh one).
                            if let Ok(s) = repo.get(&id).await {
                                if let Err(e) = revoke_session_credentials(auth.as_ref(), &mcp_tokens, &s.created_by, &id).await {
                                    tracing::warn!(session = %id, "revoke exited session credentials: {e}");
                                }
                            }
                            let _ = repo.update_status(&id, SessionStatus::Exited).await;
                            let _ = events.send(Event::SessionStatus {
                                session_id: id.clone(),
                                workspace_id: workspace_id.clone(),
                                status: SessionStatus::Exited,
                            });
                            // Viewers learn the grid is gone (0×0, ws.md).
                            let (grids, conns) = &grid_maps;
                            grids.publish(&id, || crate::grid::grid_state(&live, conns, &size_owner, &id));
                        } else {
                            // Retired by a lifecycle op that already wrote the
                            // final status (suspend → Reconnectable, restart →
                            // Running, archive/shutdown → Exited, remove → row
                            // gone). Writing `Exited` here was the bug.
                            tracing::debug!(session = %id, code = ?code, "superseded PTY exited; status left to its lifecycle op");
                        }
                        drop(guard);
                        // Don't leak a lock entry for a session nobody else is
                        // serializing on (e.g. one that was just removed).
                        // Strong count 2 = the map + our clone; anyone holding
                        // or queued on it has a clone, so it stays.
                        locks.remove_if(&id, |_, l| Arc::strong_count(l) == 2);
                        break;
                    }
                }
            }
        });
    }
}

/// Otto stopped minting label-only `otto-mcp:<session>` tokens when managed
/// (`session_scope`) ownership shipped on 2026-09-22 (on the author's install
/// the last legacy row is 10:37Z, the first managed one 11:18Z). A label-only
/// token created after this instant is a human's own naming and is never
/// auto-revoked. (Falls back to "nothing is legacy" if the literal is bad.)
fn legacy_session_token_cutover() -> chrono::DateTime<chrono::Utc> {
    chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 23, 0, 0, 0)
        .single()
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
}

/// Revoke the per-session MCP credentials minted for `session_id` (managed
/// rows by durable `session_scope`, plus the row id this daemon recorded in
/// `mcp_tokens`) and delete the Codex creds and server env files if written.
/// Free-standing so the status task can call it without `&self`.
async fn revoke_session_credentials(
    auth: Option<&AuthRepo>,
    mcp_tokens: &DashMap<Id, String>,
    owner: &Id,
    session_id: &Id,
) -> Result<()> {
    let _ = std::fs::remove_file(codex_creds_path(session_id));
    remove_codex_server_env_files(session_id).await;
    if let Some(auth) = auth {
        auth.revoke_session_tokens(owner, session_id).await?;
    }
    if let Some((_, token_id)) = mcp_tokens.remove(session_id) {
        if let Some(auth) = auth {
            if let Err(e) = auth.revoke_api_token(owner, &token_id).await {
                tracing::warn!("otto MCP tools: revoke token failed: {e}");
            }
            if let Err(e) = auth.revoke_vault_reviewer_token(owner, &token_id).await {
                tracing::warn!("otto reviewer MCP token: revoke failed: {e}");
            }
        }
    }
    Ok(())
}

/// The per-session resume lock (see `SessionManager::resume_locks`), created
/// on demand. Free-standing so the status task can take it without `&self`.
fn session_lock(locks: &DashMap<Id, Arc<Mutex<()>>>, id: &Id) -> Arc<Mutex<()>> {
    locks
        .entry(id.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// True iff `live` currently maps `id` to exactly `handle` (pointer identity):
/// the handle is the session's CURRENT process incarnation.
fn maps_handle(live: &DashMap<Id, Arc<PtyHandle>>, id: &Id, handle: &Arc<PtyHandle>) -> bool {
    live.get(id).is_some_and(|h| Arc::ptr_eq(h.value(), handle))
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

/// The providers the legacy `process_sandbox.providers` allow-list defaulted to.
const SANDBOX_BUILTIN_PROVIDERS: &[&str] = &["claude", "codex", "agy", "shell"];

/// Decide whether a session should be sandboxed and with what network posture,
/// from the `process_sandbox` setting JSON. `None` means "do not sandbox". Pure
/// (no I/O) so the gating is unit-testable. When `enabled`, EVERY `Agent`
/// session is confined — a user-added provider (`opencode`, a custom CLI)
/// included — unless a person exempted its provider: `exempt_providers`, or
/// (legacy) a `providers` list that leaves out one of the built-in
/// [`SANDBOX_BUILTIN_PROVIDERS`]. A provider the legacy list never knew about
/// is never exempt by omission (S11-309).
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
    let list = |key: &str| -> Option<Vec<String>> {
        cfg.get(key).and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
    };
    if list("exempt_providers").is_some_and(|ex| ex.iter().any(|p| p == provider)) {
        return None;
    }
    if let Some(legacy) = list("providers") {
        let builtin = SANDBOX_BUILTIN_PROVIDERS.contains(&provider);
        if builtin && !legacy.iter().any(|p| p == provider) {
            return None;
        }
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

/// The Seatbelt policy for one agent session. A read-only session (untrusted
/// input, always confined) additionally may not plant project-scope claude
/// config (`.claude/settings*.json`, hooks, `.mcp.json`) in its own folder —
/// Otto pre-trusts every cwd, so the user's next unconfined `claude` there
/// would load it without a prompt.
fn sandbox_policy(
    cwd: &std::path::Path,
    home: &std::path::Path,
    data_dir: &std::path::Path,
    extra: &[std::path::PathBuf],
    network: otto_sandbox::NetworkPolicy,
    read_only: bool,
) -> otto_sandbox::SandboxPolicy {
    let policy = otto_sandbox::SandboxPolicy::for_agent(cwd, home, data_dir, extra, network);
    if read_only {
        policy.harden_read_only(cwd, home)
    } else {
        policy
    }
}

/// A sandboxed session's private package / build cache dir
/// (`<tmp>/otto-agent-cache/<session id>`; see [`otto_sandbox::agent_cache_env`]).
fn agent_cache_dir(session_id: &Id) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("otto-agent-cache")
        .join(session_id.as_str())
}

/// Resolve a repo's git **common dir** (absolute, canonicalized) for `cwd`, so
/// the sandbox can grant write access to it. For a linked worktree this is the
/// main repo's `.git` (which holds the objects + the worktree's gitdir), which
/// lives OUTSIDE `cwd` — without it a sandboxed agent in a worktree couldn't
/// commit. Best-effort: `None` when `cwd` isn't a git repo.
pub(crate) async fn resolve_git_common_dir(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let out = otto_git_cmd::hardened_command()
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

/// Resume refuses an agent whose folder is gone (S20-04). The respawn used to
/// `create_dir_all` it: the agent then woke in an EMPTY folder, believed the
/// project had been wiped and could start recreating files. A moved/deleted
/// repo is the user's to restore — say so instead (409). Exempt (still
/// recreated on spawn): the scratch ("No workspace") home and Otto-managed
/// folders under its data dir (the CONFIGURED one — S1-306), which are
/// Otto's to rebuild.
fn missing_cwd_refusal_in(session: &Session, otto_data: &std::path::Path) -> Option<String> {
    let cwd = std::path::Path::new(&session.cwd);
    if session.cwd.trim().is_empty()
        || session.workspace_id == otto_core::domain::SCRATCH_WORKSPACE_ID
        || cwd.starts_with(otto_data)
        || cwd.is_dir()
    {
        return None;
    }
    Some(format!(
        "folder {} no longer exists — restore it (or move the session's repo back), or read the conversation in History",
        session.cwd
    ))
}

#[path = "manager/mcp_sync.rs"]
mod mcp_sync;
#[path = "manager/otto_tools_env.rs"]
mod otto_tools_env;
#[path = "manager/session_files.rs"]
mod session_files;
#[path = "manager/shutdown_guard.rs"]
mod shutdown_guard;
use otto_tools_env::{otto_tools_env, session_mcp_profiles, write_codex_creds};

#[cfg(test)]
#[path = "manager/tests/mod.rs"]
mod tests;

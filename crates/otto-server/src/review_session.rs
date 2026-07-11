//! Run a PR-review agent as a real, openable [`SessionManager`] session so the
//! user can watch it live and type into it to unblock it.
//!
//! Each agent is spawned as a normal agent session (tagged `meta.source =
//! "review"`), the review prompt is injected into its PTY (like the channel
//! bridge does), and it is told to write its findings to a temp file we then
//! read. Provider-agnostic: codex/agy write no transcript, so the file is the
//! reliable capture path; claude's JSONL transcript is a fallback.
//!
//! Resilience: each agent is independent — one that never starts, errors, or
//! gets stuck does NOT abort the others. While it runs we persist its live
//! state (running → waiting → done/error) so the UI's poll surfaces progress;
//! "waiting" means it looks blocked on input and the user should Open it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{ReviewAgentState, ReviewFinding, SessionKind, User, Workspace};
use otto_sessions::SessionManager;
use otto_state::ReviewsRepo;
use tokio::sync::Mutex;

use crate::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome, WatchStatus};

// Generous: several CLIs cold-start concurrently for one review, so claude can
// take >30s to draw its TUI; injecting before it's ready loses the prompt.
const TUI_STARTUP_WAIT: Duration = Duration::from_secs(40);
const TUI_POLL: Duration = Duration::from_millis(250);
const TUI_SETTLE: Duration = Duration::from_millis(600);
pub const PASTE_TO_ENTER: Duration = Duration::from_millis(250);
// After submitting, confirm the agent actually started (output advanced); if
// not, re-send Enter once — a freshly-spawned CLI under load can drop the first.
const DISPATCH_WAIT: Duration = Duration::from_secs(6);
const DISPATCH_POLL: Duration = Duration::from_millis(250);
pub const FINDINGS_POLL: Duration = Duration::from_millis(1000);
/// After this much silence with no findings yet, assume the agent may be
/// blocked on a prompt the guard couldn't auto-accept and flag it "waiting".
pub const WAITING_IDLE: Duration = Duration::from_secs(45);
/// After this much TOTAL silence with no findings, treat the agent as stuck and
/// fail fast so the recovery wrapper can kill + retry it — instead of waiting out
/// the full grace `timeout`. Well past `WAITING_IDLE`, so a watching human still
/// has a window to Open + respond before auto-retry kicks in.
const STUCK_IDLE: Duration = Duration::from_secs(180);
/// Total attempts (initial + retries) for a review agent before giving up.
const MAX_REVIEW_ATTEMPTS: u32 = 3;
/// Backoff before each review-agent retry.
const REVIEW_RETRY_BACKOFF: Duration = Duration::from_secs(3);

/// Effective max attempts: config override or the compiled-in default.
pub fn effective_max_attempts(max_attempts: Option<u32>) -> u32 {
    max_attempts.unwrap_or(MAX_REVIEW_ATTEMPTS)
}

/// Absolute temp path an agent writes its findings JSON to (unique per run).
pub fn findings_path(review_id: &str, agent_index: usize) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-review-{review_id}-{agent_index}.json"))
}

/// Absolute temp path the (already-built) prompt for one agent is saved to, so
/// a per-agent Retry can re-run exactly that agent without rebuilding it.
pub fn prompt_path(review_id: &str, agent_index: usize) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-review-{review_id}-{agent_index}.prompt"))
}

/// Append the "write findings to this file" instruction to a reviewer prompt.
pub fn augment_prompt(base_prompt: &str, findings_path: &str) -> String {
    format!(
        "{base_prompt}\n\n---\nWhen you have finished reviewing, write your findings as a JSON \
         array (the exact schema described above) to this absolute file path, overwriting any \
         existing content:\n\n{findings_path}\n\nWrite ONLY the JSON array to that file (no prose, \
         no markdown fence). Writing the file is the last thing you do."
    )
}

/// One finding as an agent actually emits it. Agents are TOLD to emit
/// `{path, line, severity: info|warn|bug, body}`, but review-lens skills (and
/// the models themselves) routinely use their own vocabulary — `file` for the
/// path, `summary`/`description`/`message` for the body, `blocker`/`major`/
/// `minor` severities. Silently defaulting those to `null`/`""` destroyed
/// whole runs (45 real findings → empty husks → the summarizer correctly
/// answered `[]` → 0 comments, no error). Every alternate key is captured
/// explicitly (NOT `#[serde(alias)]` — an object carrying both the canonical
/// and the alias key would then fail the WHOLE array as a duplicate field).
#[derive(serde::Deserialize)]
struct RawFinding {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    issue: Option<String>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    failure_scenario: Option<String>,
    #[serde(default)]
    suggested_fix: Option<String>,
    #[serde(default)]
    fix: Option<String>,
}

fn default_severity() -> String {
    "info".to_string()
}

/// Map agent severity vocabularies onto the engine's `info|warn|bug` scale.
/// Unknown labels pass through unchanged (the UI renders them as text).
fn normalize_severity(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "blocker" | "critical" | "bug" | "error" | "high" => "bug".to_string(),
        "major" | "warn" | "warning" | "medium" => "warn".to_string(),
        "minor" | "nit" | "suggestion" | "low" | "info" | "note" => "info".to_string(),
        _ => raw.to_string(),
    }
}

impl RawFinding {
    fn into_finding(self) -> ReviewFinding {
        let mut body = if self.body.trim().is_empty() {
            // Longest-form candidates first; `title` (a one-liner) is the floor.
            [
                self.summary,
                self.description,
                self.message,
                self.comment,
                self.issue,
                self.details,
                self.detail,
                self.title,
            ]
            .into_iter()
            .flatten()
            .find(|s| !s.trim().is_empty())
            .unwrap_or_default()
        } else {
            self.body
        };
        // Fold the auxiliary detail fields in so their substance survives the
        // summarizer (which only ever sees `body`).
        if let Some(fs) = self.failure_scenario.filter(|s| !s.trim().is_empty()) {
            body = format!("{body}\n\nFailure scenario: {fs}");
        }
        if let Some(fx) = self
            .suggested_fix
            .or(self.fix)
            .filter(|s| !s.trim().is_empty())
        {
            body = format!("{body}\n\nSuggested fix: {fx}");
        }
        ReviewFinding {
            path: self.path.or(self.file).filter(|s| !s.trim().is_empty()),
            line: self.line,
            severity: normalize_severity(&self.severity),
            body,
        }
    }
}

/// Extract the JSON array of findings from arbitrary agent output (tolerates
/// ```` ```json ```` fences + surrounding prose). Returns `[]` on any failure.
pub fn parse_findings(text: &str) -> Vec<ReviewFinding> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = stripped.find('[').unwrap_or(0);
    let end = stripped.rfind(']').map(|i| i + 1).unwrap_or(stripped.len());
    if start >= end {
        return Vec::new();
    }
    serde_json::from_str::<Vec<RawFinding>>(&stripped[start..end])
        .map(|raw| raw.into_iter().map(RawFinding::into_finding).collect())
        .unwrap_or_default()
}

/// Outcome of one review agent run (fed to the summarizer).
pub struct AgentRunResult {
    pub findings: Vec<ReviewFinding>,
    pub errored: bool,
}

/// Shared, persisted live state for all agents in a review.
pub type SharedStates = Arc<Mutex<Vec<ReviewAgentState>>>;

/// The `extra_dirs` value (→ `--add-dir=<bundle>`) that registers the staged
/// review-lens skills as FIRST-CLASS skills for `provider`, or `None` when the
/// bundle must NOT be wired.
///
/// This is a CLAUDE-ONLY mechanism: only claude loads skills from an added dir's
/// `.claude/skills` (the layout `stage_review_skills` writes). codex has no
/// first-class out-of-tree skills — and, spawned with `--search`, it would
/// scavenge the add-dir'd bundle and run the WRONG skill (the reported bug). agy
/// loads `.agents/skills`, not this bundle's claude layout, so it gets nothing
/// from it either. For non-claude providers the lens method is delivered inline
/// in the prompt (see `compose_review_lens_prompt` / `run_review_core`), so the
/// bundle is pure downside and is withheld. An empty/None dir ⇒ `None`.
pub(crate) fn review_skills_extra_dirs(
    provider: &str,
    skills_add_dir: Option<&str>,
) -> Option<serde_json::Value> {
    let dir = skills_add_dir.filter(|d| !d.is_empty())?;
    (provider == "claude").then(|| serde_json::json!([dir]))
}

/// Spawn `provider` as a live session in the repo, inject the (augmented)
/// review prompt, and wait until it writes its findings file (or `timeout`
/// elapses / it exits). Updates + persists this agent's state throughout so the
/// UI shows live progress; archives the session when done.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_session(
    manager: &Arc<SessionManager>,
    reviews: &ReviewsRepo,
    states: &SharedStates,
    ws: &Workspace,
    user: &User,
    provider: &str,
    // Per-reviewer model (e.g. a specific claude/codex model). Empty → the
    // provider's default. Carried into meta so SessionManager injects
    // `--model <name>` for providers that support it — mirrors the summarizer,
    // which already honours its configured model via `model_opt`.
    model: &str,
    // Working directory for the agent (the repo path — where the diff lives).
    cwd: &str,
    review_id: &str,
    agent_index: usize,
    base_prompt: &str,
    timeout: Duration,
    // Shared out-of-tree skills bundle (`<dir>/.claude/skills/<lens>/`) to load
    // as first-class skills via `--add-dir` — CLAUDE-ONLY (see
    // `review_skills_extra_dirs`). codex/agy can't load it and rely on the lens
    // method inlined in the prompt. None → inline only.
    skills_add_dir: Option<&str>,
) -> RunOutcome {
    let path = findings_path(review_id, agent_index);
    let _ = std::fs::remove_file(&path); // clear any stale file
    let prompt = augment_prompt(base_prompt, &path.to_string_lossy());

    let mut meta = serde_json::json!({
        "source": "review",
        "review_id": review_id,
        "agent_index": agent_index,
    });
    // `extra_dirs` becomes `--add-dir=<bundle>` on spawn (and resume), registering
    // the lens skills as first-class — but ONLY for claude, which is the only CLI
    // that loads `.claude/skills` from an added dir. Wiring it for codex (which
    // would scavenge the bundle and run the wrong skill) or agy (which loads
    // `.agents/skills`) is the propagation bug; they get the lens method inline.
    if let Some(dirs) = review_skills_extra_dirs(provider, skills_add_dir) {
        meta["extra_dirs"] = dirs;
    }
    if !model.trim().is_empty() {
        meta["model"] = serde_json::json!(model.trim());
    }
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.to_string()),
        connection_id: None,
        meta: Some(meta),
    };
    let session = match manager.create(ws, &user.id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("review_session: create session ({provider}): {e}");
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let sid = session.id.clone();
    // Persist running + session_id immediately so the UI shows it live + Open
    // works. Terminal (done/error) + findings persistence happens in the recovery
    // wrapper, so intermediate failed attempts aren't recorded as terminal.
    persist_agent(states, reviews, review_id, agent_index, {
        let sid = sid.clone();
        move |s: &mut ReviewAgentState| {
            s.status = "running".into();
            s.session_id = Some(sid);
            s.note = String::new();
        }
    })
    .await;

    // Inject the prompt once the TUI has drawn + settled, then confirm it
    // dispatched (re-sending Enter once if the first submit was dropped).
    if wait_for_tui(manager, &sid).await {
        let _ = manager.input(&sid, &bracketed_paste(&prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = manager.input(&sid, b"\r").await;
        if !dispatched(manager, &sid, before).await {
            let _ = manager.input(&sid, b"\r").await;
        }
    }

    // Watch via the shared runner (out-file / claude transcript; exit / stuck /
    // timeout). It persists the waiting↔running transition; we never kill the
    // session here so it stays openable.
    watch_for_result(
        manager,
        &sid,
        provider,
        session.provider_session_id.as_deref(),
        cwd,
        &path,
        timeout,
        WAITING_IDLE,
        STUCK_IDLE,
        |t| !parse_findings(t).is_empty(),
        |st| async move {
            let (status, note) = match st {
                WatchStatus::Waiting => {
                    ("waiting", "looks blocked on input — Open it to respond".to_string())
                }
                WatchStatus::Resumed => ("running", String::new()),
            };
            persist_agent(states, reviews, review_id, agent_index, move |s: &mut ReviewAgentState| {
                s.status = status.into();
                s.note = note;
            })
            .await;
        },
    )
    .await
}

/// Mutate this agent's state element then persist ONLY that element (never the
/// whole array — concurrent agents each persist their own, and rewriting the full
/// array would let a stale snapshot revert other rows to "pending").
async fn persist_agent<F: FnOnce(&mut ReviewAgentState)>(
    states: &SharedStates,
    reviews: &ReviewsRepo,
    review_id: &str,
    agent_index: usize,
    f: F,
) {
    let row = {
        let mut g = states.lock().await;
        g.get_mut(agent_index).map(|s| {
            f(s);
            s.clone()
        })
    };
    if let Some(row) = row {
        let _ = reviews
            .set_agent_at(&review_id.to_string(), agent_index, &row)
            .await;
    }
}

/// Map a run failure reason to the human note shown on the review agent row.
fn review_error_note(reason: Option<FailReason>) -> String {
    match reason {
        Some(FailReason::Stuck) => "stuck — no output for ~3m",
        Some(FailReason::Timeout) => "timed out (grace period elapsed)",
        Some(FailReason::Exited) => "session exited before writing findings",
        Some(FailReason::SessionGone) => "session is no longer live",
        Some(FailReason::CreateFailed) => "could not start",
        Some(FailReason::Stopped) => "stopped by user",
        None => "unknown error",
    }
    .to_string()
}

/// Run a review agent with bounded auto-recovery: up to `max_attempts`
/// (defaults to `MAX_REVIEW_ATTEMPTS`) total attempts, killing the prior
/// stuck/failed session and backing off between tries. Returns the first
/// successful result, or the last failure. (PR review agents are autonomous
/// — unlike interactive chat sessions, which must NOT be auto-retried.)
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_session_with_recovery(
    manager: &Arc<SessionManager>,
    reviews: &ReviewsRepo,
    states: &SharedStates,
    ws: &Workspace,
    user: &User,
    provider: &str,
    // Per-reviewer model (empty → provider default); threaded into each attempt.
    model: &str,
    cwd: &str,
    review_id: &str,
    agent_index: usize,
    base_prompt: &str,
    timeout: Duration,
    max_attempts: Option<u32>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // Shared out-of-tree skills bundle for `--add-dir` (see `run_agent_session`).
    skills_add_dir: Option<&str>,
) -> AgentRunResult {
    let attempts = effective_max_attempts(max_attempts);
    // Shared retry loop (kills the prior session + backs off between attempts).
    // The `cancel` flag, when set by a Cancel-review request, short-circuits the
    // loop with `Stopped` and is not retried.
    let outcome = run_with_recovery(
        manager,
        attempts,
        &[REVIEW_RETRY_BACKOFF],
        cancel,
        |_attempt| {
            run_agent_session(
                manager, reviews, states, ws, user, provider, model, cwd, review_id, agent_index,
                base_prompt, timeout, skills_add_dir,
            )
        },
    )
    .await;

    // Persist terminal state ONCE (parse findings from the final raw result).
    if let Some(raw) = outcome.raw.as_deref() {
        let findings = parse_findings(raw);
        let count = findings.len();
        let persisted = findings.clone();
        persist_agent(states, reviews, review_id, agent_index, move |s| {
            s.status = "done".into();
            s.note = format!("{count} finding{}", if count == 1 { "" } else { "s" });
            s.comment_count = count as u32;
            s.findings = persisted;
        })
        .await;
        AgentRunResult { findings, errored: false }
    } else {
        let note = review_error_note(outcome.reason);
        persist_agent(states, reviews, review_id, agent_index, move |s| {
            s.status = "error".into();
            s.note = note;
        })
        .await;
        AgentRunResult { findings: Vec::new(), errored: true }
    }
}

pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 16);
    v.extend_from_slice(b"\x1b[200~");
    v.extend_from_slice(text.as_bytes());
    v.extend_from_slice(b"\x1b[201~");
    v
}

/// How long an injected claude prompt gets to LAND in the transcript before the
/// caller re-injects (first window) or fails the attempt (second window).
pub const PROMPT_LAND_WAIT: Duration = Duration::from_secs(45);

/// Byte length of claude's transcript for (cwd, provider session id) — 0 when
/// the file doesn't exist yet. Callers capture this BEFORE injecting so
/// [`claude_prompt_landed`] only scans what THIS turn appended (a reused agent
/// session already has older user records).
pub fn transcript_len(cwd: &str, psid: &str) -> u64 {
    std::fs::metadata(otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Wait until the injected prompt actually LANDED in claude's transcript — a
/// `"type":"user"` record appended past `offset`. TUI output alone is redraw
/// noise: a paste can be swallowed (or the Enter ignored) while the terminal
/// keeps repainting, leaving a LIVE session that was never told to do anything
/// (the operator had to stop it by hand). The provider session id is re-read
/// each poll — fresh sessions only adopt it once claude writes the file.
pub async fn claude_prompt_landed(
    manager: &Arc<SessionManager>,
    sid: &otto_core::Id,
    cwd: &str,
    offset: u64,
    wait: Duration,
) -> bool {
    const NEEDLE: &[u8] = b"\"type\":\"user\"";
    let deadline = Instant::now() + wait;
    loop {
        if let Ok(s) = manager.get(sid).await {
            if let Some(psid) = s.provider_session_id.as_deref() {
                let path = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
                if let Ok(raw) = std::fs::read(&path) {
                    let tail = &raw[raw.len().min(offset as usize)..];
                    if tail.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
                        return true;
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

pub async fn wait_for_tui(manager: &Arc<SessionManager>, sid: &otto_core::Id) -> bool {
    let deadline = Instant::now() + TUI_STARTUP_WAIT;
    loop {
        let Some(handle) = manager.live_handle(sid) else {
            return false;
        };
        if handle.on_exit().borrow().is_some() {
            return false;
        }
        if !handle.scrollback(1).is_empty() && handle.last_output_at().elapsed() >= TUI_SETTLE {
            return true;
        }
        if Instant::now() >= deadline {
            return true;
        }
        tokio::time::sleep(TUI_POLL).await;
    }
}

/// True if the session produced fresh output after `before` within
/// [`DISPATCH_WAIT`] — i.e. the submitted prompt was accepted and the agent
/// started working.
pub async fn dispatched(
    manager: &Arc<SessionManager>,
    sid: &otto_core::Id,
    before: Option<std::time::Instant>,
) -> bool {
    let Some(before) = before else { return false };
    let deadline = Instant::now() + DISPATCH_WAIT;
    loop {
        match manager.live_handle(sid) {
            Some(h) if h.last_output_at() > before => return true,
            None => return false,
            _ => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DISPATCH_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_path_unique_per_agent() {
        assert_ne!(findings_path("r", 0), findings_path("r", 1));
        assert!(findings_path("r", 2).to_string_lossy().ends_with("otto-review-r-2.json"));
    }

    #[test]
    fn augment_prompt_includes_path_and_base() {
        let out = augment_prompt("Review it.", "/tmp/x.json");
        assert!(out.contains("Review it."));
        assert!(out.contains("/tmp/x.json"));
        assert!(out.to_lowercase().contains("json array"));
    }

    #[test]
    fn review_skills_extra_dirs_is_claude_only() {
        // claude first-class-loads `<dir>/.claude/skills` from `--add-dir`, so it
        // gets the staged lens bundle wired as a single-element array.
        assert_eq!(
            review_skills_extra_dirs("claude", Some("/bundle")),
            Some(serde_json::json!(["/bundle"]))
        );
        // codex has no first-class out-of-tree skills (and with `--search` would
        // scavenge the bundle and run the WRONG skill); agy loads `.agents/skills`,
        // not this bundle's claude layout. Both rely on the inlined lens method, so
        // the bundle is NEVER wired for them.
        assert_eq!(review_skills_extra_dirs("codex", Some("/bundle")), None);
        assert_eq!(review_skills_extra_dirs("agy", Some("/bundle")), None);
        // No/empty bundle ⇒ nothing, even for claude.
        assert_eq!(review_skills_extra_dirs("claude", None), None);
        assert_eq!(review_skills_extra_dirs("claude", Some("")), None);
    }

    #[test]
    fn stopped_agent_note_matches_stop_endpoint_wording() {
        // The per-agent Stop endpoint persists "stopped by user" directly; the
        // recovery loop re-persists via this note when it unwinds — the two
        // must agree or the row flickers between wordings.
        assert_eq!(review_error_note(Some(FailReason::Stopped)), "stopped by user");
    }

    #[test]
    fn parse_findings_tolerates_fences_prose_and_garbage() {
        let raw = "ok:\n```json\n[{\"path\":\"a.rs\",\"line\":3,\"severity\":\"bug\",\"body\":\"x\"}]\n```";
        let f = parse_findings(raw);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path.as_deref(), Some("a.rs"));
        assert_eq!(f[0].severity, "bug");

        assert_eq!(parse_findings("[{\"body\":\"n\"}]")[0].severity, "info");
        assert!(parse_findings("not json").is_empty());
        assert!(parse_findings("").is_empty());
    }

    #[test]
    fn parse_findings_accepts_skill_style_keys() {
        // The review-lens skills teach {file, line, severity, summary,
        // failure_scenario, …} — the exact shape that wiped review
        // 01KWYP3Z3N2CRPV1X15ZCAWKAR to empty bodies. It must map cleanly.
        let raw = r#"[{
            "file": "libs/a/b.component.ts",
            "line": 208,
            "severity": "major",
            "confidence": "confirmed",
            "category": "correctness",
            "summary": "Add-mode pricing table never refills",
            "failure_scenario": "Open dialog twice; second open is empty."
        }]"#;
        let f = parse_findings(raw);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path.as_deref(), Some("libs/a/b.component.ts"));
        assert_eq!(f[0].line, Some(208));
        assert_eq!(f[0].severity, "warn"); // major → warn
        assert!(f[0].body.starts_with("Add-mode pricing table never refills"));
        assert!(f[0].body.contains("Failure scenario: Open dialog twice"));
    }

    #[test]
    fn parse_findings_prefers_canonical_keys_and_maps_severities() {
        // Canonical body wins even when a summary is also present, and an
        // object carrying BOTH path and file must not fail the array.
        let raw = r#"[
            {"path":"x.rs","file":"y.rs","line":1,"severity":"blocker","body":"real body","summary":"ignored"},
            {"file":"z.rs","severity":"nit","description":"via description"}
        ]"#;
        let f = parse_findings(raw);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].path.as_deref(), Some("x.rs"));
        assert_eq!(f[0].severity, "bug"); // blocker → bug
        assert_eq!(f[0].body, "real body");
        assert_eq!(f[1].path.as_deref(), Some("z.rs"));
        assert_eq!(f[1].severity, "info"); // nit → info
        assert_eq!(f[1].body, "via description");
    }
}

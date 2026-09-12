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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{
    ReviewAgentState, ReviewFinding, SessionKind, SessionStatus, User, Workspace,
};
use otto_sessions::SessionManager;
use otto_state::ReviewsRepo;
use tokio::sync::Mutex;

use crate::agent_run::{
    run_with_recovery_until, watch_for_result_guarded, FailReason, RunOutcome, WatchGuard,
    WatchStatus,
};

// Generous: several CLIs cold-start concurrently for one review, so claude can
// take >30s to draw its TUI; injecting before it's ready loses the prompt.
const TUI_STARTUP_WAIT: Duration = Duration::from_secs(90);
const TUI_POLL: Duration = Duration::from_millis(250);
const TUI_SETTLE: Duration = Duration::from_millis(600);
pub const PASTE_TO_ENTER: Duration = Duration::from_millis(250);
/// After a paste, how long to wait for the pasted text to actually echo in the
/// TUI before pressing Enter. A late redraw (claude's "N MCP servers need
/// authentication" banner lands well after the prompt box is drawn) can wipe
/// the input box, so an unverified Enter submits an empty line and the agent
/// idles for the whole timeout.
const PASTE_ECHO_WAIT: Duration = Duration::from_secs(8);
const PASTE_ECHO_POLL: Duration = Duration::from_millis(250);
// After submitting, confirm the agent actually started (output advanced); if
// not, re-send Enter once — a freshly-spawned CLI under load can drop the first.
const DISPATCH_WAIT: Duration = Duration::from_secs(6);
const DISPATCH_POLL: Duration = Duration::from_millis(250);
pub const FINDINGS_POLL: Duration = Duration::from_millis(1000);
/// After this much silence with no findings yet, assume the agent may be
/// blocked on a prompt the guard couldn't auto-accept and flag it "waiting".
pub const WAITING_IDLE: Duration = Duration::from_secs(120);
/// After this much TOTAL silence with no findings, treat the agent as stuck and
/// fail fast so the recovery wrapper can kill + retry it — instead of waiting out
/// the full grace `timeout`. Well past `WAITING_IDLE`, so a watching human still
/// has a window to Open + respond before auto-retry kicks in.
const STUCK_IDLE: Duration = Duration::from_secs(900);
/// Total attempts (initial + retries) for a review agent before giving up.
const MAX_REVIEW_ATTEMPTS: u32 = 3;
/// Backoff before each review-agent retry.
const REVIEW_RETRY_BACKOFF: Duration = Duration::from_secs(3);

/// Effective max attempts: config override or the compiled-in default.
pub fn effective_max_attempts(max_attempts: Option<u32>) -> u32 {
    max_attempts.unwrap_or(MAX_REVIEW_ATTEMPTS)
}

/// Directory every review artifact (findings, per-lens files, prompts) lives
/// in — `$TMPDIR`, `/tmp` when unset.
fn findings_dir() -> PathBuf {
    PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string()))
}

/// Absolute temp path an agent writes its findings JSON to (unique per run).
pub fn findings_path(review_id: &str, agent_index: usize) -> PathBuf {
    findings_path_in(&findings_dir(), review_id, agent_index)
}

/// [`findings_path`] rooted at an explicit directory (tests hand it a tempdir
/// instead of mutating the process-wide `TMPDIR`).
pub fn findings_path_in(dir: &Path, review_id: &str, agent_index: usize) -> PathBuf {
    dir.join(format!("otto-review-{review_id}-{agent_index}.json"))
}

/// Absolute temp path ONE LENS of an orchestrator reviewer writes to. Distinct
/// from [`findings_path`] (which is the MERGED array the reviewer writes last)
/// by the extra `-<slug>` segment, so the glob below never eats the merged file.
pub fn lens_findings_path(review_id: &str, agent_index: usize, slug: &str) -> PathBuf {
    lens_findings_path_in(&findings_dir(), review_id, agent_index, slug)
}

/// [`lens_findings_path`] rooted at an explicit directory.
pub fn lens_findings_path_in(
    dir: &Path,
    review_id: &str,
    agent_index: usize,
    slug: &str,
) -> PathBuf {
    dir.join(format!("otto-review-{review_id}-{agent_index}-{slug}.json"))
}

/// Delete every per-lens file of one orchestrator run. Called before each
/// attempt (which does not know the lens slugs — hence the prefix scan) and
/// once the review is summarized.
pub fn remove_lens_findings_files(review_id: &str, agent_index: usize) {
    remove_lens_findings_files_in(&findings_dir(), review_id, agent_index);
}

/// [`remove_lens_findings_files`] rooted at an explicit directory.
pub fn remove_lens_findings_files_in(dir: &Path, review_id: &str, agent_index: usize) {
    let prefix = format!("otto-review-{review_id}-{agent_index}-");
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".json") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Sanitize a lens name into a `[a-z0-9-]{0,40}` slug safe to use as a path
/// component and as the `lens` label on a finding. Empty when nothing survives
/// (callers substitute a positional fallback).
pub fn sanitize_lens_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(40));
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.trim_matches('-').to_string()
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
    /// Lens slug, written by orchestrator-mode sub-agents into their per-lens
    /// file and kept through the parent's merge. Absent for fan-out reviewers.
    #[serde(default)]
    lens: Option<String>,
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
        // Read before `self` is partially moved below; a slug that sanitizes to
        // nothing carries no information, so it reads as absent.
        let lens = self
            .lens
            .as_deref()
            .map(sanitize_lens_slug)
            .filter(|l| !l.is_empty());
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
            lens,
        }
    }
}

/// Extract the JSON array of findings from arbitrary agent output (tolerates
/// ```` ```json ```` fences + surrounding prose). Returns `[]` on any failure.
pub fn parse_findings(text: &str) -> Vec<ReviewFinding> {
    parse_findings_array(text).unwrap_or_default()
}

/// Like [`parse_findings`] but distinguishes "no JSON findings array at all"
/// (`None`) from a well-formed EMPTY array (`Some(vec![])`). The watch loop
/// needs the difference: a reviewer whose completed turn is `[]` is DONE with
/// zero findings, not still working — treating it like garbage left a clean
/// reviewer idling at its prompt until the stuck trip killed and respawned it,
/// three times over.
pub fn parse_findings_array(text: &str) -> Option<Vec<ReviewFinding>> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = stripped.find('[')?;
    let end = stripped.rfind(']').map(|i| i + 1)?;
    if start >= end {
        return None;
    }
    serde_json::from_str::<Vec<RawFinding>>(&stripped[start..end])
        .ok()
        .map(|raw| raw.into_iter().map(RawFinding::into_finding).collect())
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
    // Orchestrator mode: the lens slugs this reviewer delegates to sub-agents,
    // each writing its own per-lens file. Empty for fan-out (one lens, no
    // sub-agents) — which also leaves the watch guard's note on today's text.
    lens_slugs: &[String],
) -> RunOutcome {
    let path = findings_path(review_id, agent_index);
    let _ = std::fs::remove_file(&path); // clear any stale file
    // …and any per-lens file a previous attempt left behind: the orchestrator
    // merges every file it finds, so a stale one would re-import dead findings.
    remove_lens_findings_files(review_id, agent_index);
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
        model: None,
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
    submit_prompt(manager, &sid, &prompt).await;

    // Watch via the shared runner (out-file / claude transcript; exit / stuck /
    // timeout). It persists the waiting↔running transition; we never kill the
    // session here so it stays openable. The guard is armed for EVERY reviewer:
    // a findings file written while the agent still has sub-agents in flight is
    // a partial result, whatever mode produced it.
    let guard = WatchGuard {
        pending_aware: true,
        lens_files: lens_slugs
            .iter()
            .map(|slug| (slug.clone(), lens_findings_path(review_id, agent_index, slug)))
            .collect(),
    };
    watch_for_result_guarded(
        manager,
        &sid,
        provider,
        session.provider_session_id.as_deref(),
        cwd,
        &path,
        timeout,
        WAITING_IDLE,
        STUCK_IDLE,
        |t| parse_findings_array(t).is_some(),
        guard,
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
        |note: String| async move {
            // Progress only — never touches `status`, so a "waiting" row set by
            // the hook above keeps its state while the note advances.
            persist_agent(states, reviews, review_id, agent_index, move |s: &mut ReviewAgentState| {
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
        Some(FailReason::Stuck) => {
            return format!("stuck — no output for {}m", STUCK_IDLE.as_secs() / 60);
        }
        Some(FailReason::Timeout) => "timed out (grace period elapsed and the agent went quiet)",
        Some(FailReason::Exited) => "session exited before writing findings",
        Some(FailReason::SessionGone) => "session is no longer live",
        Some(FailReason::CreateFailed) => "could not start",
        Some(FailReason::Stopped) => "stopped by user",
        Some(FailReason::Superseded) => "skipped — this lens was already covered by a sibling reviewer",
        None => "unknown error",
    }
    .to_string()
}

/// A sibling row (same non-empty `lens`, different index) that has finished
/// successfully — the provider name of the first one found. Used to skip the
/// remaining retries of a failed reviewer whose lens is already covered: the
/// second provider is redundancy, not a requirement, so burning another
/// 3 × 30 min on it only delays the summarizer.
pub fn lens_covered_by(states: &[ReviewAgentState], agent_index: usize) -> Option<String> {
    let me = states.get(agent_index)?;
    if me.lens.is_empty() {
        return None;
    }
    states
        .iter()
        .enumerate()
        .find(|(i, s)| *i != agent_index && s.lens == me.lens && s.status == "done")
        .map(|(_, s)| s.provider.clone())
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
    // Orchestrator lens slugs (see `run_agent_session`); empty for fan-out.
    lens_slugs: &[String],
) -> AgentRunResult {
    let attempts = effective_max_attempts(max_attempts);
    // Shared retry loop (kills the prior session + backs off between attempts).
    // The `cancel` flag, when set by a Cancel-review request, short-circuits the
    // loop with `Stopped` and is not retried.
    // Between attempts, give up (status "skipped", not "error") once a sibling
    // reviewer — the same lens on another provider — has finished: its findings
    // already cover the lens, so another fresh session buys nothing.
    let give_up = || async {
        let g = states.lock().await;
        lens_covered_by(&g, agent_index).is_some()
    };
    let outcome = run_with_recovery_until(
        manager,
        attempts,
        &[REVIEW_RETRY_BACKOFF],
        cancel,
        give_up,
        |_attempt| {
            run_agent_session(
                manager, reviews, states, ws, user, provider, model, cwd, review_id, agent_index,
                base_prompt, timeout, skills_add_dir, lens_slugs,
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
    } else if outcome.reason == Some(FailReason::Superseded) {
        let by = {
            let g = states.lock().await;
            lens_covered_by(&g, agent_index).unwrap_or_default()
        };
        persist_agent(states, reviews, review_id, agent_index, move |s| {
            s.status = "skipped".into();
            s.note = format!("skipped — {} already covered this lens", by);
        })
        .await;
        AgentRunResult { findings: Vec::new(), errored: true }
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
pub const PROMPT_LAND_WAIT: Duration = Duration::from_secs(120);

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

/// One poll of [`wait_for_tui`]: `Some(true)` ready to paste into, `Some(false)`
/// give up, `None` keep polling. On the deadline a TUI that has drawn NOTHING
/// is NOT ready — pasting into it loses the prompt, and the caller then waits
/// out the agent's entire grace window for a turn that was never started. Pure
/// so the deadline rule is testable without a PTY.
fn tui_ready(scrollback_empty: bool, settled: bool, deadline_hit: bool) -> Option<bool> {
    if !scrollback_empty && settled {
        return Some(true);
    }
    if deadline_hit {
        return Some(!scrollback_empty);
    }
    None
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
        let verdict = tui_ready(
            handle.scrollback(1).is_empty(),
            handle.last_output_at().elapsed() >= TUI_SETTLE,
            Instant::now() >= deadline,
        );
        if let Some(ready) = verdict {
            if !ready {
                tracing::warn!("review_session: agent TUI never drew in session {sid}");
            }
            return ready;
        }
        tokio::time::sleep(TUI_POLL).await;
    }
}

/// Which teardown a finished reviewer's session gets: `suspend` when the
/// provider can resume the transcript later (so "Open session" on a finished
/// review still replays it), `kill` otherwise. Pure so the mapping is testable.
pub(crate) fn stop_action(supports_resume: bool) -> &'static str {
    if supports_resume {
        "suspend"
    } else {
        "kill"
    }
}

/// Stop every reviewer session of a finished review. Reviewers are autonomous
/// sessions nobody types into again, and each holds a PTY plus the CLI's whole
/// file-descriptor footprint — leaving them live after the summarizer ran is
/// what walks `lsof` up over a day of reviews. Best-effort: a failure is a log
/// line, never a review error.
pub(crate) async fn stop_review_sessions(manager: &Arc<SessionManager>, session_ids: &[String]) {
    for sid in session_ids {
        let Ok(session) = manager.get(sid).await else {
            continue;
        };
        // Already torn down (a Stop, a crashed CLI, or the caller's own second
        // pass on an error path) — re-suspending it would only add a duplicate
        // lifecycle row to the user's session history.
        if matches!(
            session.status,
            SessionStatus::Exited | SessionStatus::Reconnectable
        ) {
            continue;
        }
        let action = stop_action(manager.providers().supports_resume(&session.provider));
        let res = if action == "suspend" {
            manager.suspend(sid).await
        } else {
            manager.kill_session(sid).await
        };
        if let Err(e) = res {
            tracing::warn!("review teardown: could not {action} session {sid}: {e}");
        }
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

/// A short, distinctive, whitespace-normalized probe from the prompt's first
/// non-empty line — what the TUI must echo back for the paste to count.
fn paste_probe(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let norm: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    // TUIs wrap/truncate long lines; 40 chars is short enough to survive that
    // and long enough not to match boilerplate.
    norm.chars().take(40).collect()
}

/// True once the last screenful contains `probe` (whitespace-normalized) —
/// i.e. the pasted prompt is really sitting in the input box.
fn paste_echoed(manager: &Arc<SessionManager>, sid: &otto_core::Id, probe: &str) -> bool {
    if probe.is_empty() {
        return true;
    }
    let Some(h) = manager.live_handle(sid) else { return false };
    let raw = String::from_utf8_lossy(&h.scrollback(200)).into_owned();
    let norm: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    screen_shows_paste(&norm, probe)
}

/// The paste counts as echoed when the screen shows the probe text OR a
/// large-paste placeholder. Codex collapses any sizeable paste into
/// `[Pasted Content N chars]` and Claude Code into `[Pasted text #1 +N lines]`,
/// so a worker prompt never echoes verbatim; treating that as "not echoed"
/// re-pasted the prompt and the worker received it twice.
fn screen_shows_paste(norm_screen: &str, probe: &str) -> bool {
    if norm_screen.contains(probe) {
        return true;
    }
    let lower = norm_screen.to_lowercase();
    lower.contains("[pasted content ") || lower.contains("[pasted text #")
}

/// Paste `prompt` into the session and press Enter — the one submit path
/// every engine should use. Waits for the TUI to draw, pastes, then WAITS
/// FOR THE PASTE TO ECHO (re-pasting once if a late redraw wiped it) before
/// Enter, and re-sends Enter once if the submit visibly didn't dispatch.
/// Returns false when the session died before the prompt could be sent.
pub async fn submit_prompt(
    manager: &Arc<SessionManager>,
    sid: &otto_core::Id,
    prompt: &str,
) -> bool {
    if !wait_for_tui(manager, sid).await {
        return false;
    }
    let probe = paste_probe(prompt);
    for attempt in 0..2 {
        let _ = manager.input(sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let deadline = Instant::now() + PASTE_ECHO_WAIT;
        loop {
            if paste_echoed(manager, sid, &probe) {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(PASTE_ECHO_POLL).await;
        }
        if paste_echoed(manager, sid, &probe) {
            break;
        }
        if attempt == 0 {
            tracing::warn!("prompt paste did not echo in session {sid}; re-pasting once");
            // Let whatever redraw ate the paste finish before trying again.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    let before = manager.live_handle(sid).map(|h| h.last_output_at());
    let _ = manager.input(sid, b"\r").await;
    if !dispatched(manager, sid, before).await {
        let _ = manager.input(sid, b"\r").await;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_probe_is_short_normalized_and_skips_blank_lines() {
        let p = paste_probe("\n\n  You are   reviewing\tthe skill package at /tmp/x — do it carefully and thoroughly\nmore");
        assert_eq!(p.chars().count(), 40);
        assert!(p.starts_with("You are reviewing the skill package"));
        assert_eq!(paste_probe(""), "");
    }

    #[test]
    fn paste_counts_as_echoed_when_the_tui_collapses_it_to_a_placeholder() {
        let probe = "You are a Codex worker on /repo (branch";
        // Codex: any sizeable paste is shown as a placeholder, never verbatim.
        assert!(screen_shows_paste(
            "› [Pasted Content 2614 chars] gpt-5 high",
            probe
        ));
        // Claude Code's placeholder.
        assert!(screen_shows_paste("> [Pasted text #1 +38 lines]", probe));
        // Verbatim echo still counts.
        assert!(screen_shows_paste(
            "> You are a Codex worker on /repo (branch main)",
            probe
        ));
        // An empty input box does not.
        assert!(!screen_shows_paste("› Ask Codex to do anything", probe));
    }

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
    fn empty_array_is_a_complete_turn_but_garbage_is_not() {
        // A clean reviewer ends its turn with `[]` — that is DONE (zero
        // findings), and the watch loop must accept it instead of waiting for
        // the stuck trip to kill + respawn it.
        assert_eq!(parse_findings_array("[]").map(|v| v.len()), Some(0));
        assert_eq!(parse_findings_array("```json\n[]\n```").map(|v| v.len()), Some(0));
        assert_eq!(
            parse_findings_array("[{\"body\":\"n\"}]").map(|v| v.len()),
            Some(1)
        );
        // No array / broken JSON → still not a result.
        assert!(parse_findings_array("still reading the diff…").is_none());
        assert!(parse_findings_array("[not json").is_none());
        assert!(parse_findings_array("").is_none());
        // parse_findings keeps its lenient shape.
        assert!(parse_findings("[]").is_empty());
    }

    fn st(name: &str, lens: &str, provider: &str, status: &str) -> ReviewAgentState {
        ReviewAgentState {
            name: name.into(),
            provider: provider.into(),
            model: String::new(),
            status: status.into(),
            note: String::new(),
            comment_count: 0,
            session_id: None,
            findings: Vec::new(),
            fallback: false,
            lens: lens.into(),
        }
    }

    #[test]
    fn lens_covered_only_by_a_finished_sibling_of_the_same_lens() {
        let states = vec![
            st("Security · claude", "Security", "claude", "done"),
            st("Security · codex", "Security", "codex", "error"),
            st("Perf · claude", "Perf", "claude", "done"),
            st("Perf · codex", "Perf", "codex", "running"),
            st("Summarizer", "", "claude", "pending"),
        ];
        // codex Security failed; claude Security is done → covered by claude.
        assert_eq!(lens_covered_by(&states, 1).as_deref(), Some("claude"));
        // A finished agent asking is still "covered" by its sibling — callers
        // only consult this between FAILED attempts, so that's moot.
        // Perf codex is running, not done → Perf claude (index 2) has no cover.
        assert_eq!(lens_covered_by(&states, 2), None);
        // Perf codex's sibling IS done → covered.
        assert_eq!(lens_covered_by(&states, 3).as_deref(), Some("claude"));
        // Rows without a lens (summarizer, legacy rows) never match anything.
        assert_eq!(lens_covered_by(&states, 4), None);
        // A "done" row of a DIFFERENT lens doesn't count.
        let solo = vec![st("A", "A", "claude", "error"), st("B", "B", "codex", "done")];
        assert_eq!(lens_covered_by(&solo, 0), None);
        // Out of range.
        assert_eq!(lens_covered_by(&solo, 9), None);
    }

    #[test]
    fn stuck_note_reports_the_real_window() {
        assert_eq!(review_error_note(Some(FailReason::Stuck)), "stuck — no output for 15m");
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

    #[test]
    fn parse_findings_keeps_lens_and_slug_is_sanitised() {
        // Orchestrator sub-agents label their findings; the parent merges the
        // per-lens files and the label must survive into the summarizer batch.
        let raw = r#"[
          {"path":"a.rs","severity":"bug","body":"x","lens":"Correctness Review"},
          {"path":"b.rs","severity":"info","body":"y","lens":"  "},
          {"path":"c.rs","severity":"info","body":"z"}
        ]"#;
        let f = parse_findings(raw);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].lens.as_deref(), Some("correctness-review"));
        // A slug that sanitizes away carries nothing — read as absent.
        assert_eq!(f[1].lens, None);
        // Fan-out findings have no lens at all (and pre-field rows deserialize).
        assert_eq!(f[2].lens, None);

        // The sanitizer is what keeps a lens name out of the filesystem: it is
        // used both as a path component and as this label.
        assert_eq!(sanitize_lens_slug("Correctness/../review"), "correctness-review");
        assert_eq!(sanitize_lens_slug("  Go  Code   Review "), "go-code-review");
        assert_eq!(sanitize_lens_slug("--grill--"), "grill");
        assert_eq!(sanitize_lens_slug("***"), "");
        assert_eq!(sanitize_lens_slug("Ünïcödé"), "n-c-d");
        assert!(sanitize_lens_slug(&"a".repeat(80)).len() <= 40);
    }

    #[test]
    fn per_lens_files_glob_deleted_per_attempt() {
        // An explicit dir, never `set_var`: TMPDIR is process-wide and the test
        // binary runs these in parallel with everything else.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let merged = findings_path_in(dir, "R1", 0);
        let mine = ["correctness", "security"]
            .map(|s| lens_findings_path_in(dir, "R1", 0, s));
        let other_agent = lens_findings_path_in(dir, "R1", 1, "correctness");
        let other_review = lens_findings_path_in(dir, "R2", 0, "correctness");
        for p in [&merged, &mine[0], &mine[1], &other_agent, &other_review] {
            std::fs::write(p, "[]").unwrap();
        }

        remove_lens_findings_files_in(dir, "R1", 0);

        // Only THIS run's per-lens files go.
        assert!(!mine[0].exists());
        assert!(!mine[1].exists());
        // The merged file is the watch loop's signal and is removed separately —
        // the glob must not eat it (its name has no `-<slug>` segment).
        assert!(merged.exists());
        // Siblings and other reviews are untouched.
        assert!(other_agent.exists());
        assert!(other_review.exists());
        // Idempotent on a directory with nothing left to delete.
        remove_lens_findings_files_in(dir, "R1", 0);
    }

    #[test]
    fn lens_covered_by_ignores_orchestrator_rows() {
        // Orchestrator rows carry an EMPTY lens (they run every lens), so a
        // sibling provider finishing must never retire one as "covered".
        let states = vec![
            st("claude · orchestrator (3 lenses)", "", "claude", "error"),
            st("codex · orchestrator (3 lenses)", "", "codex", "done"),
            st("Summarizer", "", "claude", "pending"),
        ];
        assert_eq!(lens_covered_by(&states, 0), None);
        assert_eq!(lens_covered_by(&states, 1), None);
    }

    #[test]
    fn wait_for_tui_false_on_deadline_without_output() {
        // Drawn and settled ⇒ ready, whenever that happens.
        assert_eq!(tui_ready(false, true, false), Some(true));
        assert_eq!(tui_ready(false, true, true), Some(true));
        // Drawn but still repainting ⇒ keep polling until the deadline, then
        // accept it (there IS a TUI to paste into).
        assert_eq!(tui_ready(false, false, false), None);
        assert_eq!(tui_ready(false, false, true), Some(true));
        // NOTHING drawn by the deadline ⇒ give up. Pasting into a blank TUI
        // loses the prompt and the reviewer then idles out its whole grace.
        assert_eq!(tui_ready(true, false, true), Some(false));
        assert_eq!(tui_ready(true, false, false), None);
    }

    #[test]
    fn review_done_suspends_reviewer_sessions() {
        // Resumable providers are SUSPENDED, so "Open session" on a finished
        // review still replays the reviewer's transcript…
        assert_eq!(stop_action(true), "suspend");
        // …and everything else is killed outright.
        assert_eq!(stop_action(false), "kill");
    }
}

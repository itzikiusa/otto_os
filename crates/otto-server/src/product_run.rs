//! Analysis fan-out runner for Product Story Analysis.
//!
//! `run_analysis` is spawned as a background tokio task by the analyze handler.
//! It drives one agent per (lens × provider) concurrently, each running as a
//! REAL, openable [`otto_sessions::SessionManager`] session (exactly like a PR
//! reviewer agent — see [`crate::review_session`]), then a single summarizer
//! agent consolidates / dedupes / resolves conflicts across all lens outputs.
//!
//! Provider-honoring: every provider (claude / codex / agy / …) is spawned as a
//! real session via the SessionManager mechanism, so all three CLIs are honored
//! per-lens (the old `Orchestrator::run_agent` was claude-only).
//!
//! Concurrency approach mirrors `review_session.rs`: each agent is independent;
//! one failure never aborts the others (errors are isolated per-agent). Sessions
//! are NOT killed when done — they stay live/openable so the PO can inspect
//! each lens's terminal afterward.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Extra settle time for codex (model-loading burst can last >1s after TUI ready).
const CODEX_EXTRA_SETTLE: Duration = Duration::from_millis(1_500);
const CODEX_EXTRA_SETTLE_CAP: Duration = Duration::from_secs(5);
const CODEX_EXTRA_SETTLE_POLL: Duration = Duration::from_millis(100);
// Verify-and-repaste: how long to wait for any new output after the first paste.
const REPASTE_IDLE_THRESHOLD: Duration = Duration::from_secs(7);
// How long to poll waiting for the out file to appear (fast path before repaste).
const REPASTE_FAST_POLL: Duration = Duration::from_millis(250);
// Max repaste attempts before giving up and falling into the normal watch loop.
const REPASTE_MAX_ATTEMPTS: u32 = 3;
// Total budget for the verify-and-repaste phase.
const REPASTE_PHASE_BUDGET: Duration = Duration::from_secs(25);

use otto_core::api::CreateSessionReq;
use otto_core::domain::SessionKind;
use otto_core::Id;
use otto_state::{
    LearningPatch, NewAnalysisAgent, NewEvent, NewLearning, NewQuestion, StoryPatch,
};
use tracing::warn;

use crate::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome, WatchStatus};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Per-session cwd attribution (codex usage tracking)
// ---------------------------------------------------------------------------

/// Resolve the cwd a product agent session should run in, making it unique when
/// it would otherwise be the shared system temp-dir fallback.
///
/// WHY: the usage tailer attributes codex sessions by their cwd (1:1 `by_cwd`).
/// Product sessions (analysis fan-out, rewrite, test-gen, plan-gen) fall back to
/// `std::env::temp_dir()` when a story has no real cwd. Multiple codex sessions
/// then share `/tmp` (or `$TMPDIR`) and collide → all attributed to "external"
/// instead of the workspace. Giving each session its own temp subdir restores a
/// 1:1 cwd→session mapping. (Claude attributes by its own session id, so it's
/// unaffected — but a unique dir is harmless for it.)
///
/// A REAL story cwd (the user's repo) passes through UNCHANGED — the architecture
/// lens needs the real repo. Only the shared-temp fallback is rewritten, to a
/// freshly-created unique child of the temp dir.
fn session_cwd(requested: &str) -> String {
    let temp = std::env::temp_dir();

    // Compare against the shared temp-dir fallback. Canonicalize both so e.g.
    // /var vs /private/var (macOS) or a trailing slash don't defeat the match;
    // fall back to a raw path compare if canonicalization fails.
    let requested_path = std::path::Path::new(requested);
    let is_shared_temp = match (requested_path.canonicalize(), temp.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => requested_path == temp.as_path(),
    };

    if !is_shared_temp {
        // Real story cwd — leave it untouched.
        return requested.to_string();
    }

    // Shared-temp fallback → unique per-session subdir so codex usage attributes
    // 1:1 by cwd instead of colliding on the shared temp dir.
    let unique = temp.join(format!("otto-product-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::create_dir_all(&unique) {
        tracing::debug!("product_run: create session cwd {}: {e}", unique.display());
    }
    unique.to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One analysis agent specification (provided by the 3.3 handler).
pub struct AgentSpec {
    pub provider: String,
    pub model: Option<String>,
    pub skill: String,
    pub name: String,
}

/// The JSON schema each analysis agent must respond with.
#[derive(serde::Deserialize, Default)]
pub struct Findings {
    pub summary: String,
    pub related_repos: Vec<String>,
    pub functionalities: Vec<String>,
    pub integration_points: Vec<String>,
    pub risks: Vec<String>,
    pub open_questions: Vec<FoundQuestion>,
    pub suggested_learnings: Vec<FoundLearning>,
}

#[derive(serde::Deserialize)]
pub struct FoundQuestion {
    pub text: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub category: String,
}

#[derive(serde::Deserialize)]
pub struct FoundLearning {
    pub kind: String,
    pub title: String,
    pub body: String,
}

// ---------------------------------------------------------------------------
// Session-based, provider-honoring lens runner (mirrors review_session)
// ---------------------------------------------------------------------------

/// Outcome of one lens (or the summarizer) running as a real session. This is the
/// caller-facing shape; the run mechanics return [`crate::agent_run::RunOutcome`]
/// which `run_agent_with_recovery` flattens into this (keeping `reason` as a
/// stable `&str` for the existing notification/error-note code).
pub struct LensRunResult {
    /// Raw text the agent wrote to its out file (or the claude transcript turn).
    pub raw: Option<String>,
    /// The live SessionManager session id (so the agent stays openable).
    pub session_id: Option<Id>,
    /// True if the agent never produced output (timeout / exit / start failure).
    pub errored: bool,
    /// Short reason when `errored` ("stuck", "timeout", "exited", "session-gone",
    /// "create-failed", "stopped") — surfaced in notifications and the agent error
    /// field. `None` on success.
    pub reason: Option<&'static str>,
}

impl From<RunOutcome> for LensRunResult {
    fn from(o: RunOutcome) -> Self {
        Self {
            errored: o.errored(),
            reason: o.reason.map(|r| r.as_str()),
            raw: o.raw,
            session_id: o.session_id,
        }
    }
}

/// Spawn `provider` as a live agent session in `cwd`, inject `prompt`, and wait
/// until it writes its JSON to `out_path` (or `timeout` elapses / it exits).
///
/// Models `review_session::run_agent_session`, but product-specific and without
/// the per-agent live-state persistence (the caller owns the agent DB row). The
/// session is intentionally NOT killed so it stays openable afterward.
///
/// The `prompt` MUST already instruct the agent to write its JSON to `out_path`
/// (use [`augment_with_out_path`] / the prompt builders, which append that).
///
/// When `agent_id` is `Some`, the freshly-created session id is persisted to that
/// analysis-agent row IMMEDIATELY (before the agent does any work), mirroring
/// `review_session::run_agent_session`. That's what lets the UI show "Open" and
/// stream the live terminal *while the agent is running* — not only once it
/// finishes — and keeps the session replayable afterward as history. Callers with
/// no agent row (rewrite / generate-tests / generate-plan) pass `None`.
#[allow(clippy::too_many_arguments)]
pub async fn run_lens_session(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user_id: &Id,
    provider: &str,
    cwd: &str,
    prompt: &str,
    out_path: &Path,
    timeout: Duration,
    agent_id: Option<&Id>,
) -> RunOutcome {
    // Clear any stale output from a previous run.
    let _ = std::fs::remove_file(out_path);

    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: Some(format!("Analysis: {provider}")),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        meta: Some(serde_json::json!({ "source": "product-analysis" })),
    };

    let session = match ctx.manager.create(ws, user_id, req, None).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run: create session ({provider}): {e}");
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let sid = session.id.clone();

    // Persist the session id NOW (mirrors review_session) so the agent is
    // openable live while it runs, not only after it finishes.
    if let Some(aid) = agent_id {
        if let Err(e) = ctx.product_repo.set_agent_session(aid, &sid).await {
            warn!("product_run: early set_agent_session {aid}: {e}");
        }
    }

    // Inject the prompt once the TUI has drawn + settled.
    if wait_for_tui(&ctx.manager, &sid).await {
        // For codex, wait an extra settle period so the model-loading burst
        // finishes before we type. This avoids the "model: loading" dropped-paste
        // race. Claude's fast path is unchanged (elapsed already >= TUI_SETTLE).
        if provider == "codex" {
            let settle_deadline = Instant::now() + CODEX_EXTRA_SETTLE_CAP;
            loop {
                let elapsed = ctx
                    .manager
                    .live_handle(&sid)
                    .map(|h| h.last_output_at().elapsed())
                    .unwrap_or(CODEX_EXTRA_SETTLE);
                if elapsed >= CODEX_EXTRA_SETTLE {
                    break;
                }
                if Instant::now() >= settle_deadline {
                    break;
                }
                tokio::time::sleep(CODEX_EXTRA_SETTLE_POLL).await;
            }
        }

        // Record baseline BEFORE the first paste so we can detect a dropped paste.
        let pre_paste_time = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());

        // First paste attempt.
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            // Initial dispatch confirmation failed — try once more.
            let _ = ctx.manager.input(&sid, b"\r").await;
        }

        // Verify-and-repaste: poll for up to REPASTE_PHASE_BUDGET. If the out
        // file already appeared we short-circuit; if the session never produced
        // meaningful output since pre_paste_time we re-paste (up to
        // REPASTE_MAX_ATTEMPTS). This is the primary fix for the codex
        // dropped-paste-while-loading race.
        let repaste_deadline = Instant::now() + REPASTE_PHASE_BUDGET;
        let mut repaste_attempts: u32 = 0;
        let mut baseline = pre_paste_time;

        'repaste: loop {
            // Out file appeared — prompt was received and acted on.
            if out_path.exists() {
                break 'repaste;
            }

            // Session gone or exited — fall through to watch loop.
            let handle = match ctx.manager.live_handle(&sid) {
                Some(h) => h,
                None => break 'repaste,
            };
            if handle.on_exit().borrow().is_some() {
                break 'repaste;
            }

            // Budget exhausted — fall through to normal watch loop.
            if Instant::now() >= repaste_deadline {
                break 'repaste;
            }

            // Check if the session produced meaningful new output since baseline.
            let last_out = handle.last_output_at();
            let advanced = baseline.map(|b| last_out > b).unwrap_or(false);
            if advanced {
                // Session is responding — no repaste needed, exit early.
                break 'repaste;
            }

            // No new output since baseline for REPASTE_IDLE_THRESHOLD → repaste.
            let idle_since_baseline = baseline
                .map(|b| Instant::now().duration_since(b))
                .unwrap_or(REPASTE_IDLE_THRESHOLD);
            if idle_since_baseline >= REPASTE_IDLE_THRESHOLD {
                if repaste_attempts >= REPASTE_MAX_ATTEMPTS {
                    break 'repaste;
                }
                repaste_attempts += 1;
                warn!(
                    "product_run: session ({provider}) appears to have dropped the prompt \
                     (attempt {repaste_attempts}/{REPASTE_MAX_ATTEMPTS}); re-pasting"
                );
                let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
                tokio::time::sleep(PASTE_TO_ENTER).await;
                let before2 = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
                let _ = ctx.manager.input(&sid, b"\r").await;
                if !dispatched(&ctx.manager, &sid, before2).await {
                    let _ = ctx.manager.input(&sid, b"\r").await;
                }
                // Update baseline to reflect the repaste moment so subsequent
                // idle checks measure from here.
                baseline = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
                continue 'repaste;
            }

            tokio::time::sleep(REPASTE_FAST_POLL).await;
        }
    }

    // Watch for the result via the shared runner (out-file / claude transcript;
    // exit / stuck / timeout). Persist the waiting↔running transition on the agent
    // row (when there is one) so the UI shows it, like a review agent does.
    watch_for_result(
        &ctx.manager,
        &sid,
        provider,
        session.provider_session_id.as_deref(),
        cwd,
        out_path,
        timeout,
        WAITING_IDLE,
        STUCK_IDLE,
        |t| extract_json_block(t).is_some(),
        |st| async move {
            if let Some(aid) = agent_id {
                let status = match st {
                    WatchStatus::Waiting => "waiting",
                    WatchStatus::Resumed => "running",
                };
                let _ = ctx
                    .product_repo
                    .set_agent_status(aid, status, None, None, false)
                    .await;
            }
        },
    )
    .await
}

/// Append the "write your JSON to this file" instruction to a built prompt.
pub fn augment_with_out_path(base_prompt: &str, out_path: &str) -> String {
    format!(
        "{base_prompt}\n\n---\nWhen done, write your result as JSON to this file (overwrite it), \
         and write ONLY the JSON (no prose): {out_path}"
    )
}

// ---------------------------------------------------------------------------
// The verbatim OUTPUT CONTRACT appended to every analysis prompt.
// ---------------------------------------------------------------------------

const OUTPUT_CONTRACT: &str = r#"Investigate as needed, then respond with EXACTLY ONE ```json code block (no prose before or after) matching:
{"summary": "...", "related_repos": ["..."], "functionalities": ["..."],
 "integration_points": ["..."], "risks": ["..."],
 "open_questions": [{"text":"...","rationale":"...","category":"scope|data|ux|edge-case|dependency|other"}],
 "suggested_learnings": [{"kind":"pattern|avoid","title":"...","body":"..."}]}"#;

/// The verbatim consolidation contract for the summarizer agent. It MUST merge,
/// dedupe, and resolve conflicts across all lens outputs into ONE result.
const SUMMARIZER_CONTRACT: &str = r#"You are the SUMMARIZER. Several analysis lenses (possibly run on different AI providers) each produced findings about the SAME product story. Consolidate them into ONE result:
- Write a single cohesive summary.
- Merge related_repos, functionalities, integration_points, and risks across all lenses; remove duplicates.
- Merge AND DEDUPE the open questions (collapse near-identical questions into one; keep the clearest wording).
- RESOLVE conflicts: where lenses or providers disagree, decide the best answer and record what you reconciled in "conflict_notes".
- Merge suggested_learnings; drop duplicates.

Respond with EXACTLY ONE ```json code block (no prose before or after) matching:
{"summary": "...",
 "questions": [{"text":"...","rationale":"...","category":"scope|data|ux|edge-case|dependency|other"}],
 "related_repos": ["..."], "functionalities": ["..."], "integration_points": ["..."], "risks": ["..."],
 "suggested_learnings": [{"kind":"pattern|avoid","title":"...","body":"..."}],
 "conflict_notes": "..."}"#;

// ---------------------------------------------------------------------------
// Pure helpers (unit-testable without an agent)
// ---------------------------------------------------------------------------

/// Extract the first JSON value from `s`.
///
/// Priority:
/// 1. First ```` ```json ```` fenced block (wins even if a bare `{` appears earlier
///    in prose before the fence).
/// 2. First balanced `{…}` not preceded by a fence marker.
/// 3. `None` for prose-only input.
pub fn extract_json_block(s: &str) -> Option<serde_json::Value> {
    // --- Strategy 1: find the first ```json ... ``` fence --------------------
    if let Some(fence_start) = s.find("```json") {
        let after_marker = fence_start + "```json".len();
        // Skip an optional newline right after the marker
        let content_start = if s[after_marker..].starts_with('\n') {
            after_marker + 1
        } else {
            after_marker
        };
        if let Some(end_fence) = s[content_start..].find("```") {
            let json_str = s[content_start..content_start + end_fence].trim();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                return Some(v);
            }
        }
    }

    // --- Strategy 2: first balanced { ... } anywhere in the string ----------
    find_first_balanced_object(s)
}

/// Find and parse the first balanced `{…}` block in `s`.
fn find_first_balanced_object(s: &str) -> Option<serde_json::Value> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape_next = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s_idx) = start {
                        let candidate = &s[s_idx..=i];
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
                            return Some(v);
                        }
                    }
                    // Reset for next candidate
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

/// Build the full analysis prompt for one agent.
///
/// The story context (body + learnings + Jira details) now lives in a separate
/// CONTEXT file that agents read. This keeps the prompt compact while allowing
/// the context to be arbitrarily large.
///
/// The prompt body is: skill instructions + file-read directive + OUTPUT CONTRACT.
/// An optional `prior_summary` is still inlined (it's a short text, not a large blob).
pub fn build_analysis_prompt(
    skill_body: &str,
    context_path: &str,
    prior_summary: Option<&str>,
) -> String {
    let mut prompt = String::new();

    // 1. Skill body (instructions for this agent role)
    prompt.push_str(skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive — agents must read the file before answering.
    prompt.push_str(
        "The full story context is in this file — it may be LARGE, read it fully \
         (in chunks if needed) before answering:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. Prior analysis summary (if re-running)
    if let Some(summary) = prior_summary {
        if !summary.is_empty() {
            prompt.push_str("## Prior Analysis Summary\n\n");
            prompt.push_str(summary);
            prompt.push_str("\n\n");
        }
    }

    // 4. OUTPUT CONTRACT (verbatim, required by the brief)
    prompt.push_str("---\n\n");
    prompt.push_str(OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

/// The consolidated result the summarizer agent must produce.
#[derive(serde::Deserialize, Default)]
pub struct SummaryFindings {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub questions: Vec<FoundQuestion>,
    #[serde(default)]
    pub related_repos: Vec<String>,
    #[serde(default)]
    pub functionalities: Vec<String>,
    #[serde(default)]
    pub integration_points: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub suggested_learnings: Vec<FoundLearning>,
    #[serde(default)]
    pub conflict_notes: String,
}

/// Build the summarizer prompt, feeding every successful lens's findings JSON
/// (each labelled `Lens <name> (<provider>):`) and instructing the agent to
/// consolidate, merge, dedupe, and resolve conflicts into ONE result.
///
/// `lenses` is `(name, provider, findings_json)` per successful lens agent.
pub fn build_summarizer_prompt(
    skill_body: &str,
    story_title: &str,
    lenses: &[(String, String, String)],
) -> String {
    let mut prompt = String::new();

    if !skill_body.is_empty() {
        prompt.push_str(skill_body);
        prompt.push_str("\n\n---\n\n");
    }

    prompt.push_str("## Story: ");
    prompt.push_str(story_title);
    prompt.push_str("\n\n");

    prompt.push_str("## Lens Findings to Consolidate\n\n");
    for (name, provider, json) in lenses {
        prompt.push_str("### Lens ");
        prompt.push_str(name);
        prompt.push_str(" (");
        prompt.push_str(provider);
        prompt.push_str("):\n\n");
        prompt.push_str(json);
        prompt.push_str("\n\n");
    }

    prompt.push_str("---\n\n");
    prompt.push_str(SUMMARIZER_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// Orchestration entry point
// ---------------------------------------------------------------------------

const LENS_TIMEOUT: Duration = Duration::from_secs(600);
const SUMMARIZER_TIMEOUT: Duration = Duration::from_secs(600);

/// Quiet for this long with no result ⇒ flag the agent "waiting" (may be blocked
/// on input); < STUCK_IDLE so there's a window before auto-retry.
const WAITING_IDLE: Duration = Duration::from_secs(45);
/// No PTY output AND no result file for this long ⇒ the agent is stuck; fail fast
/// so the recovery wrapper can kill + retry instead of waiting out the timeout.
const STUCK_IDLE: Duration = Duration::from_secs(180);
/// Total attempts for an agent (initial + retries) before it's marked errored.
const MAX_AGENT_ATTEMPTS: u32 = 3;
/// How many times an orphaned agent may be auto-resumed across daemon restarts
/// before the reaper gives up and marks it errored.
const MAX_RESUME_ATTEMPTS: i64 = 2;
/// Backoff before each retry attempt (index = retry number - 1). Clamped to the
/// last entry beyond its length.
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(4)];

// ---------------------------------------------------------------------------
// Per-agent cancellation registry (mirrors skill_eval::CancelRegistry)
// ---------------------------------------------------------------------------

/// Maps analysis-agent id → a cancel flag, so a manual Stop (or shutdown) can
/// signal an in-flight `run_agent_with_recovery` to abort without it being
/// mistaken for a failure (which would auto-retry).
pub type CancelRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// Create the shared registry (called once at boot, stored in `ServerCtx`).
pub fn new_cancel_registry() -> CancelRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

fn register_cancel(reg: &CancelRegistry, agent_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    reg.lock().unwrap().insert(agent_id.to_string(), Arc::clone(&flag));
    flag
}

/// Trip the cancel flag for `agent_id` if it is registered (in-flight).
pub fn signal_cancel(reg: &CancelRegistry, agent_id: &str) {
    if let Some(flag) = reg.lock().unwrap().get(agent_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

fn unregister_cancel(reg: &CancelRegistry, agent_id: &str) {
    reg.lock().unwrap().remove(agent_id);
}

// ---------------------------------------------------------------------------
// Bounded auto-retry wrapper (delegates to the shared agent_run primitive)
// ---------------------------------------------------------------------------

/// Run an analysis agent as a real session with automatic recovery, on top of the
/// shared [`crate::agent_run::run_with_recovery`]. Each attempt is a fresh
/// `run_lens_session` (session id persisted early so Open shows the current
/// attempt). When `agent_id` is `Some`, a cancel flag is registered keyed by it so
/// a manual Stop trips it and the loop returns `stopped` WITHOUT another retry.
/// Callers with no agent row (rewrite / generate-tests / generate-plan) pass
/// `None` — they still get retry + stuck-recovery, just no Stop/Open wiring.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_with_recovery(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user_id: &Id,
    provider: &str,
    cwd: &str,
    prompt: &str,
    out_path: &Path,
    timeout: Duration,
    agent_id: Option<&Id>,
) -> LensRunResult {
    let cancel_key = agent_id.map(|a| a.to_string());
    let cancel = cancel_key
        .as_deref()
        .map(|key| register_cancel(&ctx.product_agent_cancels, key));

    let outcome = run_with_recovery(
        &ctx.manager,
        MAX_AGENT_ATTEMPTS,
        &RETRY_BACKOFF,
        cancel.as_ref(),
        |_attempt| run_lens_session(ctx, ws, user_id, provider, cwd, prompt, out_path, timeout, agent_id),
    )
    .await;

    if let Some(key) = cancel_key.as_deref() {
        unregister_cancel(&ctx.product_agent_cancels, key);
    }
    outcome.into()
}

/// One lens agent's outcome after running as a real session.
struct LensOutcome {
    name: String,
    provider: String,
    /// The findings JSON we persisted for the agent row (canonicalised).
    findings_json: Option<String>,
    findings: Option<Findings>,
    errored: bool,
}

/// Run the full analysis fan-out for a story.  Spawned as a background tokio
/// task by the analyze handler.  Returns `()` — all errors are logged and
/// isolated; no panic propagates.
///
/// Every entry in `specs` is one (lens × provider) and runs as its own real,
/// openable session (claude / codex / agy all honored). A final summarizer
/// session consolidates, dedupes, and resolves conflicts across all lenses.
#[allow(clippy::too_many_arguments)]
pub async fn run_analysis(
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    analysis_id: otto_core::Id,
    specs: Vec<AgentSpec>,
    summarizer_provider: String,
    cwd: String,
    focus: Option<String>,
) {
    // 1. Load story title (for summarizer prompt) + build the shared context file.
    let story = match ctx.product_repo.get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run: get_story {story_id}: {e}");
            let _ = ctx
                .product_repo
                .set_analysis_status(&analysis_id, "error", Some("failed to load story"), true)
                .await;
            return;
        }
    };

    let story_title = story.title.clone();

    // Build the enriched context document and write it to a temp file shared
    // by all lens agents. On error, fall back to bare story body.
    let context_path = std::env::temp_dir()
        .join(format!("otto-product-{analysis_id}-context.md"));
    {
        let context_md = match ctx.product.build_agent_context(&story_id, focus.as_deref()).await {
            Ok(md) => md,
            Err(e) => {
                warn!("product_run: build_agent_context: {e}; falling back to story body");
                // Fallback: bare story body.
                let body = match ctx.product_repo.latest_source_version(&story_id).await {
                    Ok(Some(v)) => v.body_md,
                    _ => String::new(),
                };
                format!("# {story_title}\n\n## Story\n\n{body}\n")
            }
        };
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run: write context file: {e}");
        }
    }
    let context_path_str = context_path.to_string_lossy().to_string();

    // 2. Pre-trust every distinct provider on cwd (mirror review) so no session
    //    stalls on the interactive "trust this folder?" prompt and times out.
    {
        let mut trusted = HashSet::<String>::new();
        for provider in specs
            .iter()
            .map(|s| s.provider.clone())
            .chain(std::iter::once(summarizer_provider.clone()))
        {
            if trusted.insert(provider.clone()) {
                otto_sessions::trust::ensure_trusted(&provider, &cwd);
            }
        }
    }

    // 3. Per-(lens × provider) concurrent fan-out as real sessions ------------
    // For each spec: create the agent row (status=running), build the lens
    // prompt (skill body + context-file ref + write-to-file), run a real
    // session, record session_id, persist findings. Failures are isolated.
    let mut set = tokio::task::JoinSet::new();
    for (i, spec) in specs.into_iter().enumerate() {
        let ctx = ctx.clone();
        let ws = ws.clone();
        let user_id = user_id.clone();
        let analysis_id = analysis_id.clone();
        let _story_title = story_title.clone();
        let context_path_str = context_path_str.clone();
        // Per-spec session cwd: when the story has no real cwd we fell back to the
        // shared temp dir; give EACH lens session its own unique temp subdir so the
        // codex usage tailer attributes them 1:1 by cwd instead of colliding. A real
        // story cwd passes through unchanged (the architecture lens needs the repo).
        let cwd = session_cwd(&cwd);

        set.spawn(async move {
            // Create the agent row (status = "running"). The display name
            // disambiguates the lens across providers, like review.
            let agent = match ctx
                .product_repo
                .add_analysis_agent(NewAnalysisAgent {
                    analysis_id: analysis_id.clone(),
                    name: format!("{} \u{00b7} {}", spec.name, spec.provider),
                    skill: spec.skill.clone(),
                    provider: spec.provider.clone(),
                    model: spec.model.clone().unwrap_or_default(),
                    status: "running".into(),
                    session_id: None,
                })
                .await
            {
                Ok(a) => a,
                Err(e) => {
                    warn!("product_run: add_analysis_agent '{}': {e}", spec.name);
                    return LensOutcome {
                        name: spec.name.clone(),
                        provider: spec.provider.clone(),
                        findings_json: None,
                        findings: None,
                        errored: true,
                    };
                }
            };
            let agent_id = agent.id.clone();

            // Resolve skill body: library first, then bundled, then empty.
            let skill_body = ctx
                .context_library
                .get_skill(&spec.skill)
                .map(|s| s.body)
                .or_else(|| otto_product::skill_body(&spec.skill).map(|s| s.to_string()))
                .unwrap_or_default();

            // Build the lens prompt (context lives in the context file) +
            // append the write-to-file instruction.
            let out_path = std::env::temp_dir()
                .join(format!("otto-product-{analysis_id}-{i}.json"));
            let base_prompt = build_analysis_prompt(&skill_body, &context_path_str, None);
            let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

            // Run as a real, openable session honoring this spec's provider.
            let result = run_agent_with_recovery(
                &ctx,
                &ws,
                &user_id,
                &spec.provider,
                &cwd,
                &prompt,
                &out_path,
                LENS_TIMEOUT,
                Some(&agent_id),
            )
            .await;

            // Record the session id so the UI can Open the live terminal.
            if let Some(ref sid) = result.session_id {
                if let Err(e) = ctx.product_repo.set_agent_session(&agent_id, sid).await {
                    warn!("product_run: set_agent_session {agent_id}: {e}");
                }
            }

            // Parse + persist the outcome.
            let parsed = result
                .raw
                .as_deref()
                .and_then(extract_json_block)
                .and_then(|v| serde_json::from_value::<Findings>(v).ok());

            match (result.errored, parsed) {
                (false, Some(findings)) => {
                    let findings_json = serde_json::to_string(&serde_json::json!({
                        "summary": findings.summary,
                        "related_repos": findings.related_repos,
                        "functionalities": findings.functionalities,
                        "integration_points": findings.integration_points,
                        "risks": findings.risks,
                        "open_questions": findings.open_questions.iter().map(|q| serde_json::json!({"text": q.text, "rationale": q.rationale, "category": q.category})).collect::<Vec<_>>(),
                        "suggested_learnings": findings.suggested_learnings.iter().map(|l| serde_json::json!({"kind": l.kind, "title": l.title, "body": l.body})).collect::<Vec<_>>(),
                    }))
                    .unwrap_or_default();
                    if let Err(e) = ctx
                        .product_repo
                        .set_agent_status(&agent_id, "done", Some(&findings_json), None, true)
                        .await
                    {
                        warn!("product_run: set_agent_status done {agent_id}: {e}");
                    }
                    LensOutcome {
                        name: spec.name.clone(),
                        provider: spec.provider.clone(),
                        findings_json: Some(findings_json),
                        findings: Some(findings),
                        errored: false,
                    }
                }
                _ => {
                    let stopped = result.reason == Some("stopped");
                    let err = if result.errored {
                        match result.reason {
                            Some("stopped") => "stopped by user".to_string(),
                            Some(r) => format!("agent failed after {MAX_AGENT_ATTEMPTS} attempts ({r})"),
                            None => "session produced no output (timeout/exit/start failure)".to_string(),
                        }
                    } else {
                        format!(
                            "could not parse Findings JSON from agent output (len={})",
                            result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                        )
                    };
                    warn!("product_run: lens '{}' ({}) failed: {err}", spec.name, spec.provider);
                    let _ = ctx
                        .product_repo
                        .set_agent_status(&agent_id, "error", None, Some(&err), true)
                        .await;
                    // Surface genuine failures (not user-initiated stops) so an
                    // unattended pipeline notices instead of silently degrading.
                    if !stopped {
                        let _ = ctx.events.send(otto_core::event::Event::Notice {
                            level: "warn".into(),
                            title: format!("Analysis agent failed: {} · {}", spec.name, spec.provider),
                            body: err.clone(),
                        });
                    }
                    LensOutcome {
                        name: spec.name.clone(),
                        provider: spec.provider.clone(),
                        findings_json: None,
                        findings: None,
                        errored: true,
                    }
                }
            }
        });
    }

    let mut outcomes: Vec<LensOutcome> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(o) = joined {
            outcomes.push(o);
        }
    }

    let any_errored = outcomes.iter().any(|o| o.errored);
    let lens_count = outcomes.len();

    // Successful lenses (name, provider, findings_json) feed the summarizer.
    let successful: Vec<(String, String, String)> = outcomes
        .iter()
        .filter_map(|o| {
            o.findings_json
                .as_ref()
                .map(|j| (o.name.clone(), o.provider.clone(), j.clone()))
        })
        .collect();

    // 4. Summarizer session: ONE agent consolidates / dedupes / resolves -------
    let summarizer_skill_body = ctx
        .context_library
        .get_skill("po-story-overview")
        .map(|s| s.body)
        .or_else(|| otto_product::skill_body("po-story-overview").map(|s| s.to_string()))
        .unwrap_or_default();

    let summarizer_agent = ctx
        .product_repo
        .add_analysis_agent(NewAnalysisAgent {
            analysis_id: analysis_id.clone(),
            name: format!("Summarizer \u{00b7} {summarizer_provider}"),
            skill: "po-story-overview".into(),
            provider: summarizer_provider.clone(),
            model: String::new(),
            status: "running".into(),
            session_id: None,
        })
        .await
        .ok();

    let summary: Option<SummaryFindings> = if successful.is_empty() {
        // No lens produced findings → nothing to consolidate. Don't leave the
        // summarizer row stuck in "running" (it would show as a perpetual
        // spinner with no openable session).
        if let Some(ref agent) = summarizer_agent {
            let _ = ctx
                .product_repo
                .set_agent_status(
                    &agent.id,
                    "error",
                    None,
                    Some("no successful lens outputs to consolidate"),
                    true,
                )
                .await;
        }
        None
    } else {
        let out_path =
            std::env::temp_dir().join(format!("otto-product-{analysis_id}-summary.json"));
        let base = build_summarizer_prompt(&summarizer_skill_body, &story_title, &successful);
        let prompt = augment_with_out_path(&base, &out_path.to_string_lossy());

        // Own unique session cwd for the summarizer (same temp-fallback fix as the
        // lenses); a real story cwd passes through unchanged.
        let summarizer_cwd = session_cwd(&cwd);
        let result = run_agent_with_recovery(
            &ctx,
            &ws,
            &user_id,
            &summarizer_provider,
            &summarizer_cwd,
            &prompt,
            &out_path,
            SUMMARIZER_TIMEOUT,
            summarizer_agent.as_ref().map(|a| &a.id),
        )
        .await;

        if let Some(ref agent) = summarizer_agent {
            if let Some(ref sid) = result.session_id {
                let _ = ctx.product_repo.set_agent_session(&agent.id, sid).await;
            }
        }

        let parsed = result
            .raw
            .as_deref()
            .and_then(extract_json_block)
            .and_then(|v| serde_json::from_value::<SummaryFindings>(v).ok());

        // Persist the summarizer agent row.
        if let Some(ref agent) = summarizer_agent {
            match (&parsed, result.errored) {
                (Some(_), _) => {
                    let _ = ctx
                        .product_repo
                        .set_agent_status(
                            &agent.id,
                            "done",
                            result.raw.as_deref(),
                            None,
                            true,
                        )
                        .await;
                }
                (None, _) => {
                    let _ = ctx
                        .product_repo
                        .set_agent_status(
                            &agent.id,
                            "error",
                            None,
                            Some("summarizer produced no parseable JSON"),
                            true,
                        )
                        .await;
                }
            }
        }

        parsed
    };

    // 5. Persist questions + learnings ----------------------------------------
    // The summarizer's deduped questions REPLACE the old per-lens Rust dedup.
    // Fall back to the Rust-side merge only if the summarizer failed.
    let existing_questions = ctx
        .product_repo
        .list_questions(&story_id)
        .await
        .unwrap_or_default();
    let mut seen_texts: HashSet<String> = existing_questions
        .iter()
        .map(|q| q.text.trim().to_lowercase())
        .collect();
    let mut question_count = 0usize;

    let mut create_question = |q_text: &str, rationale: &str, category: &str| {
        let norm = q_text.trim().to_lowercase();
        if norm.is_empty() || seen_texts.contains(&norm) {
            return None;
        }
        seen_texts.insert(norm);
        Some(NewQuestion {
            story_id: story_id.clone(),
            analysis_id: Some(analysis_id.clone()),
            text: q_text.to_string(),
            rationale: rationale.to_string(),
            category: category.to_string(),
            created_by: story.created_by.clone(),
        })
    };

    // Gather the questions to create (summarizer-first; else fallback).
    let mut to_create: Vec<NewQuestion> = Vec::new();
    if let Some(ref s) = summary {
        for q in &s.questions {
            if let Some(nq) = create_question(&q.text, &q.rationale, &q.category) {
                to_create.push(nq);
            }
        }
    } else {
        for o in &outcomes {
            if let Some(ref f) = o.findings {
                for q in &f.open_questions {
                    if let Some(nq) = create_question(&q.text, &q.rationale, &q.category) {
                        to_create.push(nq);
                    }
                }
            }
        }
    }
    drop(create_question);
    for nq in to_create {
        if let Err(e) = ctx.product_repo.create_question(nq).await {
            warn!("product_run: create_question: {e}");
        } else {
            question_count += 1;
        }
    }

    // Suggested learnings → inactive product_learnings (summarizer-first).
    let suggested: Vec<(&str, &str, &str)> = if let Some(ref s) = summary {
        s.suggested_learnings
            .iter()
            .map(|l| (l.kind.as_str(), l.title.as_str(), l.body.as_str()))
            .collect()
    } else {
        outcomes
            .iter()
            .filter_map(|o| o.findings.as_ref())
            .flat_map(|f| f.suggested_learnings.iter())
            .map(|l| (l.kind.as_str(), l.title.as_str(), l.body.as_str()))
            .collect()
    };
    for (kind, title, body) in suggested {
        if title.trim().is_empty() {
            continue;
        }
        let created = ctx
            .product_repo
            .create_learning(NewLearning {
                workspace_id: story.workspace_id.clone(),
                kind: kind.to_string(),
                title: title.to_string(),
                body: body.to_string(),
                tags: String::new(),
                refs_json: "[]".into(),
                source_story_id: Some(story_id.clone()),
                created_by: story.created_by.clone(),
            })
            .await;
        match created {
            Ok(learning) => {
                // create_learning hardcodes active=1; flip to inactive so
                // suggested learnings require human review before use.
                if let Err(e) = ctx
                    .product_repo
                    .update_learning(
                        &learning.id,
                        LearningPatch {
                            kind: None,
                            title: None,
                            body: None,
                            tags: None,
                            refs_json: None,
                            active: Some(false),
                        },
                    )
                    .await
                {
                    warn!("product_run: deactivate learning {}: {e}", learning.id);
                }
            }
            Err(e) => warn!("product_run: create_learning '{title}': {e}"),
        }
    }

    // 6. Finalise analysis row -------------------------------------------------
    let final_summary = match &summary {
        Some(s) if !s.summary.trim().is_empty() => s.summary.clone(),
        _ => {
            // Fallback: concat successful lens summaries.
            let joined = outcomes
                .iter()
                .filter_map(|o| o.findings.as_ref())
                .map(|f| f.summary.clone())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
            if joined.is_empty() {
                "(no agent summaries available)".to_string()
            } else {
                joined
            }
        }
    };
    let final_status = if any_errored { "partial" } else { "done" };
    if let Err(e) = ctx
        .product_repo
        .set_analysis_status(&analysis_id, final_status, Some(&final_summary), true)
        .await
    {
        warn!("product_run: set_analysis_status: {e}");
    }

    // 7. Update story stage to "analyzed" -------------------------------------
    if let Err(e) = ctx
        .product_repo
        .update_story(
            &story_id,
            StoryPatch {
                stage: Some("analyzed".into()),
                ..Default::default()
            },
        )
        .await
    {
        warn!("product_run: update_story stage: {e}");
    }

    // 8. Append event ---------------------------------------------------------
    let summary_event = format!(
        "analysis completed: {lens_count} lens agent(s), {question_count} new question(s)"
    );
    if let Err(e) = ctx
        .product_repo
        .add_event(NewEvent {
            story_id: story_id.clone(),
            section: "analysis".into(),
            kind: "analyzed".into(),
            summary: summary_event,
            actor_id: None,
            meta_json: None,
        })
        .await
    {
        warn!("product_run: add_event: {e}");
    }
}

// ---------------------------------------------------------------------------
// Per-agent retry — re-run a single failed / stuck analysis lens agent.
// ---------------------------------------------------------------------------

/// Re-run a single analysis lens agent by id.  Mirrors one iteration of the
/// `run_analysis` per-agent loop but operates on an EXISTING agent row rather
/// than creating a new one.  The summarizer is NOT re-run; only the one lens.
///
/// All errors are isolated — nothing panics.  The caller (the HTTP handler)
/// spawns this as a background task and returns 202 immediately.
pub async fn retry_analysis_agent(
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user_id: Id,
    analysis_id: Id,
    agent_id: Id,
) {
    // 1. Load the agent row.
    let agent = match ctx.product_repo.get_analysis_agent(&agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!("product_run(retry): get_analysis_agent {agent_id}: {e}");
            return;
        }
    };

    // 2. Verify the agent belongs to the requested analysis.
    if agent.analysis_id != analysis_id {
        warn!(
            "product_run(retry): agent {agent_id} does not belong to analysis {analysis_id}"
        );
        return;
    }

    // 3. Load the analysis → story (for context_path + cwd).
    let analysis = match ctx.product_repo.get_analysis(&analysis_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!("product_run(retry): get_analysis {analysis_id}: {e}");
            let _ = ctx
                .product_repo
                .set_agent_status(&agent_id, "error", None, Some("retry: analysis not found"), true)
                .await;
            return;
        }
    };

    let story = match ctx.product_repo.get_story(&analysis.story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(retry): get_story {}: {e}", analysis.story_id);
            let _ = ctx
                .product_repo
                .set_agent_status(&agent_id, "error", None, Some("retry: story not found"), true)
                .await;
            return;
        }
    };

    // 4. Mark the agent row "running" and clear prior error.
    let clear_err: Option<&str> = None;
    if let Err(e) = ctx
        .product_repo
        .set_agent_status(&agent_id, "running", None, clear_err, false)
        .await
    {
        warn!("product_run(retry): set_agent_status running {agent_id}: {e}");
    }
    // Clear the error field explicitly by re-setting to None via a raw update.
    // (set_agent_status merges existing error; we want a clean slate.)
    // We do this by passing empty-string error and relying on the merge — the
    // empty string is better than the old error text for the UI.
    // (No separate "clear_error" API; the empty string approach is idiomatic here.)

    // 5. Pre-trust provider. As in the fan-out, a missing story cwd falls back to
    // the shared temp dir; rewrite that to a unique per-session subdir so codex
    // usage attributes this retry 1:1 by cwd. A real story cwd passes unchanged.
    let cwd = session_cwd(&story.cwd.clone().unwrap_or_else(|| {
        std::env::temp_dir().to_string_lossy().to_string()
    }));
    otto_sessions::trust::ensure_trusted(&agent.provider, &cwd);

    // 6. Rebuild context file (shared with this analysis's context path, or fresh).
    let context_path = std::env::temp_dir()
        .join(format!("otto-product-{analysis_id}-context.md"));
    if !context_path.exists() {
        // Context file was cleaned up; rebuild it.
        let context_md = match ctx.product.build_agent_context(&analysis.story_id, None).await {
            Ok(md) => md,
            Err(e) => {
                warn!("product_run(retry): build_agent_context: {e}; falling back to story body");
                let body = match ctx.product_repo.latest_source_version(&analysis.story_id).await {
                    Ok(Some(v)) => v.body_md,
                    _ => String::new(),
                };
                format!("# {}\n\n## Story\n\n{body}\n", story.title)
            }
        };
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(retry): write context file: {e}");
        }
    }
    let context_path_str = context_path.to_string_lossy().to_string();

    // 7. Resolve skill body.
    let skill_body = ctx
        .context_library
        .get_skill(&agent.skill)
        .map(|s| s.body)
        .or_else(|| otto_product::skill_body(&agent.skill).map(|s| s.to_string()))
        .unwrap_or_default();

    // 8. Build prompt + unique out_path (use agent_id for uniqueness).
    let out_path = std::env::temp_dir()
        .join(format!("otto-product-{analysis_id}-retry-{agent_id}.json"));
    let base_prompt = build_analysis_prompt(&skill_body, &context_path_str, None);
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 9. Run the session.
    let result = run_agent_with_recovery(
        &ctx,
        &ws,
        &user_id,
        &agent.provider,
        &cwd,
        &prompt,
        &out_path,
        LENS_TIMEOUT,
        Some(&agent_id),
    )
    .await;

    // 10. Record the new session id.
    if let Some(ref sid) = result.session_id {
        if let Err(e) = ctx.product_repo.set_agent_session(&agent_id, sid).await {
            warn!("product_run(retry): set_agent_session {agent_id}: {e}");
        }
    }

    // 11. Parse + persist outcome — mirrors the run_analysis per-agent logic.
    let parsed = result
        .raw
        .as_deref()
        .and_then(extract_json_block)
        .and_then(|v| serde_json::from_value::<Findings>(v).ok());

    match (result.errored, parsed) {
        (false, Some(findings)) => {
            let findings_json = serde_json::to_string(&serde_json::json!({
                "summary": findings.summary,
                "related_repos": findings.related_repos,
                "functionalities": findings.functionalities,
                "integration_points": findings.integration_points,
                "risks": findings.risks,
                "open_questions": findings.open_questions.iter().map(|q| serde_json::json!({"text": q.text, "rationale": q.rationale, "category": q.category})).collect::<Vec<_>>(),
                "suggested_learnings": findings.suggested_learnings.iter().map(|l| serde_json::json!({"kind": l.kind, "title": l.title, "body": l.body})).collect::<Vec<_>>(),
            }))
            .unwrap_or_default();
            if let Err(e) = ctx
                .product_repo
                .set_agent_status(&agent_id, "done", Some(&findings_json), None, true)
                .await
            {
                warn!("product_run(retry): set_agent_status done {agent_id}: {e}");
            }
        }
        _ => {
            let err = if result.errored {
                "retry: session produced no output (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "retry: could not parse Findings JSON from agent output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(retry): agent {agent_id} failed: {err}");
            let _ = ctx
                .product_repo
                .set_agent_status(&agent_id, "error", None, Some(&err), true)
                .await;
            if result.reason != Some("stopped") {
                let _ = ctx.events.send(otto_core::event::Event::Notice {
                    level: "warn".into(),
                    title: format!("Analysis agent failed: {}", agent.name),
                    body: err.clone(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Orphan reaper — auto-resume analysis agents stranded by a daemon restart
// ---------------------------------------------------------------------------

/// Run ONCE at daemon startup. After a restart, any analysis agent still in
/// `running`/`waiting` has no surviving task driving it, so it is orphaned.
/// For each: if it hasn't exhausted its resume budget, re-run it via
/// [`retry_analysis_agent`] (which rebuilds context + prompt from the DB and runs
/// with full recovery); otherwise mark it errored and notify. Running this only at
/// startup avoids racing legitimately-in-flight agents (there are none yet).
pub async fn reap_orphaned_agents_on_startup(ctx: ServerCtx) {
    let agents = match ctx.product_repo.list_unfinished_agents().await {
        Ok(a) => a,
        Err(e) => {
            warn!("orphan reaper: list_unfinished_agents failed: {e}");
            return;
        }
    };
    if agents.is_empty() {
        return;
    }
    warn!(
        "orphan reaper: {} unfinished analysis agent(s) after restart",
        agents.len()
    );

    for agent in agents {
        let analysis = match ctx.product_repo.get_analysis(&agent.analysis_id).await {
            Ok(a) => a,
            Err(_) => {
                let _ = ctx
                    .product_repo
                    .set_agent_status(&agent.id, "error", None, Some("interrupted (restart); analysis gone"), true)
                    .await;
                continue;
            }
        };
        let story = match ctx.product_repo.get_story(&analysis.story_id).await {
            Ok(s) => s,
            Err(_) => {
                let _ = ctx
                    .product_repo
                    .set_agent_status(&agent.id, "error", None, Some("interrupted (restart); story gone"), true)
                    .await;
                continue;
            }
        };

        if agent.resume_count >= MAX_RESUME_ATTEMPTS {
            let _ = ctx
                .product_repo
                .set_agent_status(
                    &agent.id,
                    "error",
                    None,
                    Some("interrupted by daemon restart — gave up after auto-resumes"),
                    true,
                )
                .await;
            let _ = ctx.events.send(otto_core::event::Event::Notice {
                level: "warn".into(),
                title: format!("Analysis agent abandoned: {}", agent.name),
                body: "Restarted too many times to auto-resume.".into(),
            });
            continue;
        }

        let ws = match ctx.workspaces.get(&story.workspace_id).await {
            Ok(w) => w,
            Err(e) => {
                warn!("orphan reaper: workspace {} load failed: {e}", story.workspace_id);
                continue;
            }
        };

        let _ = ctx.product_repo.bump_resume_count(&agent.id).await;
        let _ = ctx.events.send(otto_core::event::Event::Notice {
            level: "info".into(),
            title: format!("Resuming analysis agent: {}", agent.name),
            body: "Re-running after a daemon restart.".into(),
        });
        tokio::spawn(retry_analysis_agent(
            ctx.clone(),
            ws,
            story.created_by.clone(),
            analysis.id.clone(),
            agent.id.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Rewrite output contract appended to the writer prompt.
// ---------------------------------------------------------------------------

const REWRITE_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"title":"...","body_markdown":"...","change_notes":"..."}"#;

// ---------------------------------------------------------------------------
// build_rewrite_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full rewrite prompt for the writer agent.
///
/// The story context (body, learnings, Jira details, answered questions) now
/// lives in a separate CONTEXT file. This keeps the prompt compact while allowing
/// arbitrarily large context.
///
/// Prompt body: writer skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_rewrite_prompt(
    writer_skill_body: &str,
    context_path: &str,
) -> String {
    let mut prompt = String::new();

    // 1. Writer skill body
    prompt.push_str(writer_skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, learnings) is in \
         this file — it may be LARGE, read it fully (in chunks if needed) before writing:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(REWRITE_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// run_rewrite — spawned as a background task by the rewrite handler
// ---------------------------------------------------------------------------

/// Parsed response from the writer agent.
#[derive(serde::Deserialize)]
struct RewriteFindings {
    title: String,
    body_markdown: String,
    change_notes: String,
}

/// Run the rewrite for a story.  Spawned as a background tokio task.
/// Returns `()` — all errors are logged; no panic propagates.
///
/// Provider-honoring: runs via `run_lens_session` (not `orchestrator.run_agent`),
/// so claude / codex / agy are all honored.  Context is written to a temp file.
#[allow(clippy::too_many_arguments)]
pub async fn run_rewrite(
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    provider: String,
    // model override reserved for future use when run_lens_session gains per-session model selection
    _model: Option<String>,
    cwd: String,
    focus: Option<String>,
) {
    // Per-invocation session cwd: when there's no real story cwd we fell back to
    // the shared temp dir; give this rewrite session its own unique temp subdir so
    // the codex usage tailer attributes it 1:1 by cwd (re-running rewrite for the
    // same story never collides). A real story cwd passes through unchanged.
    let cwd = session_cwd(&cwd);

    // 1. Load story
    let story = match ctx.product_repo.get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(rewrite): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Pick writer skill: jira → jira-story-writer, else → rfc-writer
    let writer_skill_name = if story.source_kind == "jira" {
        "jira-story-writer"
    } else {
        "rfc-writer"
    };

    let skill_body = ctx
        .context_library
        .get_skill(writer_skill_name)
        .map(|s| s.body)
        .or_else(|| {
            otto_product::skill_body(writer_skill_name).map(|s| s.to_string())
        })
        .unwrap_or_default();

    // 3. Build context file (enriched with Jira details, answered Q&A, learnings).
    //    The rewrite context also includes answered questions and analysis summary
    //    (build_agent_context provides story body + Jira + learnings; we enrich
    //    further here with the answered questions and analysis summary).
    let rewrite_id = otto_core::new_id();
    let context_path = std::env::temp_dir()
        .join(format!("otto-product-rewrite-{rewrite_id}-context.md"));

    {
        let mut context_md = match ctx.product.build_agent_context(&story_id, focus.as_deref()).await {
            Ok(md) => md,
            Err(e) => {
                warn!("product_run(rewrite): build_agent_context: {e}");
                let body = match ctx.product_repo.latest_source_version(&story_id).await {
                    Ok(Some(v)) => v.body_md,
                    _ => String::new(),
                };
                format!("# {}\n\n## Story\n\n{body}\n", story.title)
            }
        };

        // Append answered questions (important for the writer).
        let answered: Vec<otto_state::ProductQuestion> = ctx
            .product_repo
            .list_questions(&story_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|q| q.status == "answered")
            .collect();
        if !answered.is_empty() {
            context_md.push_str("\n## Answered Questions\n\n");
            for q in &answered {
                context_md.push_str(&format!("**Q:** {}\n", q.text));
                if let Some(ref ans) = q.answer {
                    if !ans.trim().is_empty() {
                        context_md.push_str(&format!("**A:** {}\n", ans));
                    }
                }
                context_md.push('\n');
            }
        }

        // Append latest analysis summary.
        let analysis_summary = {
            let analyses = ctx
                .product_repo
                .list_analyses(&story_id)
                .await
                .unwrap_or_default();
            analyses
                .into_iter()
                .max_by_key(|a| a.created_at)
                .map(|a| a.summary)
                .unwrap_or_default()
        };
        if !analysis_summary.trim().is_empty() {
            context_md.push_str("\n## Analysis Summary\n\n");
            context_md.push_str(&analysis_summary);
            context_md.push('\n');
        }

        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(rewrite): write context file: {e}");
        }
    }

    // 4. Build prompt (references the context file) + out_path for JSON output.
    let out_path = std::env::temp_dir()
        .join(format!("otto-product-rewrite-{rewrite_id}.json"));
    let base_prompt = build_rewrite_prompt(&skill_body, &context_path.to_string_lossy());
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 5. Pre-trust provider.
    otto_sessions::trust::ensure_trusted(&provider, &cwd);

    // 6. Run as a provider-honoring session.
    let result = run_agent_with_recovery(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
        None,
    )
    .await;

    // 7. Parse + persist the outcome.
    let output_opt = result.raw.as_deref()
        .and_then(extract_json_block)
        .and_then(|v| serde_json::from_value::<RewriteFindings>(v).ok());

    match output_opt {
        Some(findings) => {
            // 8. Persist suggested version
            if let Err(e) = ctx
                .product_repo
                .add_version(otto_state::NewVersion {
                    story_id: story_id.clone(),
                    kind: "suggested".into(),
                    title: findings.title,
                    body_md: findings.body_markdown,
                    raw_json: None,
                    change_notes: Some(findings.change_notes.clone()),
                    created_by: story.created_by.clone(),
                })
                .await
            {
                warn!("product_run(rewrite): add_version: {e}");
            }

            // 9. Update story stage to "refined"
            if let Err(e) = ctx
                .product_repo
                .update_story(
                    &story_id,
                    otto_state::StoryPatch {
                        stage: Some("refined".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(rewrite): update_story stage: {e}");
            }

            // 10. Add event
            if let Err(e) = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "rewrite".into(),
                    kind: "rewrite_suggested".into(),
                    summary: format!(
                        "Rewrite suggested using skill '{writer_skill_name}'; change_notes: {}",
                        findings.change_notes
                    ),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(rewrite): add_event: {e}");
            }
        }
        None => {
            let reason = if result.errored {
                "rewrite agent session failed (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "rewrite agent returned unparseable output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(rewrite): {reason}");
            let _ = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "rewrite".into(),
                    kind: "rewrite_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests output contract appended to the test-cases prompt.
// ---------------------------------------------------------------------------

const TESTS_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"testcases":[{"title":"...","category":"happy|validation|error|edge","priority":"high|medium|low","preconditions":["..."],"steps":["..."],"expected":"..."}]}"#;

// ---------------------------------------------------------------------------
// Local deserialization structs for the test-generation response.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GenTestcases {
    testcases: Vec<GenTestcase>,
}

#[derive(serde::Deserialize)]
struct GenTestcase {
    title: String,
    category: String,
    #[serde(default = "default_priority")]
    priority: String,
    #[serde(default)]
    preconditions: Vec<String>,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    expected: String,
}

fn default_priority() -> String {
    "medium".to_string()
}

// ---------------------------------------------------------------------------
// build_tests_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full test-case generation prompt.
///
/// The story context (body, answered questions, analysis summary, learnings)
/// now lives in a separate CONTEXT file read by the agent.
///
/// Prompt body: tests skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_tests_prompt(
    tests_skill_body: &str,
    context_path: &str,
) -> String {
    let mut prompt = String::new();

    // 1. Tests skill body
    prompt.push_str(tests_skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, learnings) is in \
         this file — it may be LARGE, read it fully (in chunks if needed) before generating tests:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(TESTS_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// run_generate_tests — spawned as a background task by the generate handler
// ---------------------------------------------------------------------------

/// Run the test-case generation for a story.  Spawned as a background tokio
/// task by the generate handler.  Returns `()` — all errors are logged; no
/// panic propagates.
///
/// Provider-honoring: runs via `run_lens_session` (not `orchestrator.run_agent`).
/// Context is written to a temp file.
#[allow(clippy::too_many_arguments)]
pub async fn run_generate_tests(
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    provider: String,
    // model override reserved for future use when run_lens_session gains per-session model selection
    _model: Option<String>,
    cwd: String,
    focus: Option<String>,
) {
    // Per-invocation session cwd: when there's no real story cwd we fell back to
    // the shared temp dir; give this test-gen session its own unique temp subdir so
    // the codex usage tailer attributes it 1:1 by cwd. A real story cwd passes
    // through unchanged.
    let cwd = session_cwd(&cwd);

    // 1. Load story
    let story = match ctx.product_repo.get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(generate_tests): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Resolve skill body: story-test-cases
    let skill_name = "story-test-cases";
    let skill_body = ctx
        .context_library
        .get_skill(skill_name)
        .map(|s| s.body)
        .or_else(|| otto_product::skill_body(skill_name).map(|s| s.to_string()))
        .unwrap_or_default();

    // 3. Build context file (enriched with Jira details, answered Q&A, learnings).
    let gen_id = otto_core::new_id();
    let context_path = std::env::temp_dir()
        .join(format!("otto-product-tests-{gen_id}-context.md"));

    {
        let mut context_md = match ctx.product.build_agent_context(&story_id, focus.as_deref()).await {
            Ok(md) => md,
            Err(e) => {
                warn!("product_run(generate_tests): build_agent_context: {e}");
                let body = match ctx.product_repo.latest_source_version(&story_id).await {
                    Ok(Some(v)) => v.body_md,
                    _ => String::new(),
                };
                format!("# {}\n\n## Story\n\n{body}\n", story.title)
            }
        };

        // Append answered questions.
        let answered: Vec<otto_state::ProductQuestion> = ctx
            .product_repo
            .list_questions(&story_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|q| q.status == "answered")
            .collect();
        if !answered.is_empty() {
            context_md.push_str("\n## Answered Questions\n\n");
            for q in &answered {
                context_md.push_str(&format!("**Q:** {}\n", q.text));
                if let Some(ref ans) = q.answer {
                    if !ans.trim().is_empty() {
                        context_md.push_str(&format!("**A:** {}\n", ans));
                    }
                }
                context_md.push('\n');
            }
        }

        // Append latest analysis summary.
        let analysis_summary = {
            let analyses = ctx
                .product_repo
                .list_analyses(&story_id)
                .await
                .unwrap_or_default();
            analyses
                .into_iter()
                .max_by_key(|a| a.created_at)
                .map(|a| a.summary)
                .unwrap_or_default()
        };
        if !analysis_summary.trim().is_empty() {
            context_md.push_str("\n## Analysis Summary\n\n");
            context_md.push_str(&analysis_summary);
            context_md.push('\n');
        }

        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(generate_tests): write context file: {e}");
        }
    }

    // 4. Build prompt (references the context file) + out_path for JSON output.
    let out_path = std::env::temp_dir()
        .join(format!("otto-product-tests-{gen_id}.json"));
    let base_prompt = build_tests_prompt(&skill_body, &context_path.to_string_lossy());
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 5. Pre-trust provider.
    otto_sessions::trust::ensure_trusted(&provider, &cwd);

    // 6. Run as a provider-honoring session.
    let result = run_agent_with_recovery(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
        None,
    )
    .await;

    // 7. Parse + persist the outcome.
    let parsed_opt = result.raw.as_deref()
        .and_then(extract_json_block)
        .and_then(|v| serde_json::from_value::<GenTestcases>(v).ok());

    match parsed_opt {
        Some(parsed) => {
            // 8. Create testcase run
            let run = match ctx
                .product_repo
                .create_testcase_run(&story_id, &story.created_by)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("product_run(generate_tests): create_testcase_run: {e}");
                    let _ = ctx
                        .product_repo
                        .add_event(otto_state::NewEvent {
                            story_id: story_id.clone(),
                            section: "tests".into(),
                            kind: "tests_error".into(),
                            summary: format!("test generation failed (db): {e}"),
                            actor_id: None,
                            meta_json: None,
                        })
                        .await;
                    return;
                }
            };

            // 9. Insert each test case
            let tc_count = parsed.testcases.len();
            for (i, tc) in parsed.testcases.into_iter().enumerate() {
                let steps_json = serde_json::to_string(&serde_json::json!({
                    "preconditions": tc.preconditions,
                    "steps": tc.steps,
                    "expected": tc.expected,
                }))
                .unwrap_or_else(|_| "{}".into());

                if let Err(e) = ctx
                    .product_repo
                    .add_testcase(otto_state::NewTestcase {
                        run_id: run.id.clone(),
                        story_id: story_id.clone(),
                        title: tc.title,
                        category: tc.category,
                        priority: tc.priority,
                        steps_json,
                        order_idx: i as i64,
                    })
                    .await
                {
                    warn!("product_run(generate_tests): add_testcase[{i}]: {e}");
                }
            }

            // 10. Update story stage
            if let Err(e) = ctx
                .product_repo
                .update_story(
                    &story_id,
                    otto_state::StoryPatch {
                        stage: Some("tests_drafted".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(generate_tests): update_story stage: {e}");
            }

            // 11. Add event
            if let Err(e) = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "tests".into(),
                    kind: "tests_drafted".into(),
                    summary: format!("test cases generated: {tc_count} case(s) drafted"),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(generate_tests): add_event: {e}");
            }
        }
        None => {
            let reason = if result.errored {
                "test generation session failed (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "test generation agent returned unparseable output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(generate_tests): {reason}");
            let _ = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "tests".into(),
                    kind: "tests_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Plan output contract appended to the task-breakdown prompt.
// ---------------------------------------------------------------------------

const PLAN_OUTPUT_CONTRACT: &str = r#"Respond with EXACTLY ONE ```json code block (no prose) matching:
{"plan_markdown":"..."}
where plan_markdown is the full implementation plan as Markdown using level-3 headings of the form `### Task N: <title>`, each followed by `**Goal:** ...`, a checklist of steps as `- [ ]` items, and `**Verify:** ...`. Emit every checkbox as `- [ ]` (todo)."#;

// ---------------------------------------------------------------------------
// build_plan_prompt — pure, unit-testable
// ---------------------------------------------------------------------------

/// Build the full implementation-plan prompt for the task-breakdown agent.
///
/// The story context (body, answered questions, analysis summary, approved test
/// cases, learnings) lives in a separate CONTEXT file the agent reads. The prompt
/// body is: task-breakdown skill body + file-read directive + OUTPUT CONTRACT.
pub fn build_plan_prompt(skill_body: &str, context_path: &str) -> String {
    let mut prompt = String::new();

    // 1. Task-breakdown skill body
    prompt.push_str(skill_body);
    prompt.push_str("\n\n---\n\n");

    // 2. Context file directive
    prompt.push_str(
        "The full story context (body, answered questions, analysis summary, approved test \
         cases, learnings) is in this file — it may be LARGE, read it fully (in chunks if \
         needed) before planning:\n",
    );
    prompt.push_str(context_path);
    prompt.push_str("\n\n");

    // 3. OUTPUT CONTRACT (verbatim)
    prompt.push_str("---\n\n");
    prompt.push_str(PLAN_OUTPUT_CONTRACT);
    prompt.push('\n');

    prompt
}

// ---------------------------------------------------------------------------
// run_generate_plan — spawned as a background task by the plan handler
// ---------------------------------------------------------------------------

/// Parsed response from the task-breakdown agent.
#[derive(serde::Deserialize)]
struct PlanFindings {
    plan_markdown: String,
}

/// Run the implementation-plan generation for a story. Spawned as a background
/// tokio task by the generate-plan handler. Returns `()` — all errors are
/// logged; no panic propagates.
///
/// Mirrors `run_rewrite`: provider-honoring (runs via `run_lens_session`), with a
/// temp context file enriched with answered questions, the latest analysis
/// summary, AND the latest run's approved test cases.
#[allow(clippy::too_many_arguments)]
pub async fn run_generate_plan(
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    provider: String,
    // model override reserved for future use when run_lens_session gains per-session model selection
    _model: Option<String>,
    cwd: String,
    focus: Option<String>,
) {
    // Per-invocation session cwd: when there's no real story cwd we fell back to
    // the shared temp dir; give this plan-gen session its own unique temp subdir so
    // the codex usage tailer attributes it 1:1 by cwd. A real story cwd passes
    // through unchanged.
    let cwd = session_cwd(&cwd);

    // 1. Load story
    let story = match ctx.product_repo.get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(plan): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Resolve skill body: story-task-breakdown
    let skill_name = "story-task-breakdown";
    let skill_body = ctx
        .context_library
        .get_skill(skill_name)
        .map(|s| s.body)
        .or_else(|| otto_product::skill_body(skill_name).map(|s| s.to_string()))
        .unwrap_or_default();

    // 3. Build context file (Jira details + answered Q&A + analysis summary +
    //    approved test cases + learnings).
    let plan_id = otto_core::new_id();
    let context_path = std::env::temp_dir()
        .join(format!("otto-product-plan-{plan_id}-context.md"));

    {
        let mut context_md = match ctx.product.build_agent_context(&story_id, focus.as_deref()).await {
            Ok(md) => md,
            Err(e) => {
                warn!("product_run(plan): build_agent_context: {e}");
                let body = match ctx.product_repo.latest_source_version(&story_id).await {
                    Ok(Some(v)) => v.body_md,
                    _ => String::new(),
                };
                format!("# {}\n\n## Story\n\n{body}\n", story.title)
            }
        };

        // Append answered questions.
        let answered: Vec<otto_state::ProductQuestion> = ctx
            .product_repo
            .list_questions(&story_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|q| q.status == "answered")
            .collect();
        if !answered.is_empty() {
            context_md.push_str("\n## Answered Questions\n\n");
            for q in &answered {
                context_md.push_str(&format!("**Q:** {}\n", q.text));
                if let Some(ref ans) = q.answer {
                    if !ans.trim().is_empty() {
                        context_md.push_str(&format!("**A:** {}\n", ans));
                    }
                }
                context_md.push('\n');
            }
        }

        // Append latest analysis summary.
        let analysis_summary = {
            let analyses = ctx
                .product_repo
                .list_analyses(&story_id)
                .await
                .unwrap_or_default();
            analyses
                .into_iter()
                .max_by_key(|a| a.created_at)
                .map(|a| a.summary)
                .unwrap_or_default()
        };
        if !analysis_summary.trim().is_empty() {
            context_md.push_str("\n## Analysis Summary\n\n");
            context_md.push_str(&analysis_summary);
            context_md.push('\n');
        }

        // Append approved test cases from the latest run (mirrors inject bundle
        // section 4: list_testcase_runs → first → list_testcases → approved).
        let approved: Vec<otto_state::ProductTestcase> = {
            let runs = ctx
                .product_repo
                .list_testcase_runs(&story_id)
                .await
                .unwrap_or_default();
            match runs.first() {
                Some(run) => ctx
                    .product_repo
                    .list_testcases(&run.id)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| c.status == "approved")
                    .collect(),
                None => Vec::new(),
            }
        };
        if !approved.is_empty() {
            context_md.push_str("\n## Approved Test Cases\n\n");
            for c in &approved {
                context_md.push_str(&format!(
                    "### {} ({}, {})\n",
                    c.title, c.category, c.priority
                ));
                if let Ok(steps) = serde_json::from_str::<serde_json::Value>(&c.steps_json) {
                    if let Some(pre) = steps.get("preconditions").and_then(|v| v.as_array()) {
                        if !pre.is_empty() {
                            context_md.push_str("**Preconditions:**\n");
                            for p in pre {
                                if let Some(s) = p.as_str() {
                                    context_md.push_str(&format!("- {s}\n"));
                                }
                            }
                        }
                    }
                    if let Some(st) = steps.get("steps").and_then(|v| v.as_array()) {
                        if !st.is_empty() {
                            context_md.push_str("**Steps:**\n");
                            for (i, s) in st.iter().enumerate() {
                                if let Some(s) = s.as_str() {
                                    context_md.push_str(&format!("{}. {s}\n", i + 1));
                                }
                            }
                        }
                    }
                    if let Some(exp) = steps.get("expected").and_then(|v| v.as_str()) {
                        if !exp.trim().is_empty() {
                            context_md.push_str(&format!("**Expected:** {exp}\n"));
                        }
                    }
                }
                context_md.push('\n');
            }
        }

        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(plan): write context file: {e}");
        }
    }

    // 4. Build prompt (references the context file) + out_path for JSON output.
    let out_path = std::env::temp_dir()
        .join(format!("otto-product-plan-{plan_id}.json"));
    let base_prompt = build_plan_prompt(&skill_body, &context_path.to_string_lossy());
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 5. Pre-trust provider.
    otto_sessions::trust::ensure_trusted(&provider, &cwd);

    // 6. Run as a provider-honoring session.
    let result = run_agent_with_recovery(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
        None,
    )
    .await;

    // 7. Parse + persist the outcome.
    let parsed_opt = result.raw.as_deref()
        .and_then(extract_json_block)
        .and_then(|v| serde_json::from_value::<PlanFindings>(v).ok());

    match parsed_opt {
        Some(findings) if !findings.plan_markdown.trim().is_empty() => {
            // 8. Persist the plan as a new kind="plan" version.
            if let Err(e) = ctx
                .product_repo
                .add_version(otto_state::NewVersion {
                    story_id: story_id.clone(),
                    kind: "plan".into(),
                    title: "Implementation Plan".into(),
                    body_md: findings.plan_markdown,
                    raw_json: None,
                    change_notes: None,
                    created_by: story.created_by.clone(),
                })
                .await
            {
                warn!("product_run(plan): add_version: {e}");
            }

            // 9. Update story stage to "planned".
            if let Err(e) = ctx
                .product_repo
                .update_story(
                    &story_id,
                    otto_state::StoryPatch {
                        stage: Some("planned".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(plan): update_story stage: {e}");
            }

            // 10. Add event.
            if let Err(e) = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "plan".into(),
                    kind: "plan_generated".into(),
                    summary: "Implementation plan generated".into(),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(plan): add_event: {e}");
            }
        }
        _ => {
            let reason = if result.errored {
                "plan generation session failed (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "plan generation agent returned unparseable/empty output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(plan): {reason}");
            let _ = ctx
                .product_repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "plan".into(),
                    kind: "plan_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Narrative builders for skill self-improvement (Task 4.4)
// ---------------------------------------------------------------------------

/// Build a self-improvement narrative from test-case review outcomes.
///
/// Describes the story, then each case with its title, category, and status.
/// Cases with `status == "changes_requested"` or a non-empty `review_note`
/// are highlighted so the improvement engine can learn what the PO changed or
/// rejected. Framed as a signal for improving the `story-test-cases` skill.
pub fn build_improve_narrative_from_tests(
    story: &otto_state::ProductStory,
    cases: &[otto_state::ProductTestcase],
) -> String {
    let mut s = String::new();
    s.push_str("# Test-Case Skill Improvement Signal\n\n");
    s.push_str(
        "The Product Owner has reviewed the generated test cases for the following story. \
         Reflect on the story-test-cases skill and how it can be improved based on the PO's \
         feedback (what was changed, rejected, or approved).\n\n",
    );
    s.push_str("## Story\n\n");
    s.push_str("**Title:** ");
    s.push_str(&story.title);
    s.push('\n');
    s.push_str("**Source:** ");
    s.push_str(&story.source_kind);
    s.push_str(" / ");
    s.push_str(&story.source_key);
    s.push('\n');
    s.push_str("**Stage:** ");
    s.push_str(&story.stage);
    s.push_str("\n\n");

    if cases.is_empty() {
        s.push_str("## Test Cases\n\n(none)\n");
        return s;
    }

    s.push_str("## Test Cases\n\n");
    for case in cases {
        let needs_emphasis =
            case.status == "changes_requested"
                || case.review_note.as_ref().map_or(false, |n| !n.trim().is_empty());

        if needs_emphasis {
            s.push_str("### [FEEDBACK] ");
        } else {
            s.push_str("### ");
        }
        s.push_str(&case.title);
        s.push('\n');
        s.push_str("- **Category:** ");
        s.push_str(&case.category);
        s.push('\n');
        s.push_str("- **Priority:** ");
        s.push_str(&case.priority);
        s.push('\n');
        s.push_str("- **Status:** ");
        s.push_str(&case.status);
        s.push('\n');

        if let Some(ref note) = case.review_note {
            if !note.trim().is_empty() {
                s.push_str("- **PO Review Note:** ");
                s.push_str(note.trim());
                s.push('\n');
            }
        }

        if case.status == "changes_requested" {
            s.push_str(
                "  *(PO requested changes — consider what generated this case and what \
                 should be improved in the skill to avoid this outcome.)*\n",
            );
        }
        s.push('\n');
    }

    let approved = cases.iter().filter(|c| c.status == "approved").count();
    let changed = cases.iter().filter(|c| c.status == "changes_requested").count();
    let total = cases.len();
    s.push_str(&format!(
        "## Summary\n\n{total} case(s) total: {approved} approved, {changed} with changes requested.\n"
    ));

    s
}

/// Build a self-improvement narrative from clarifying questions and notes.
///
/// Includes posted/answered questions and internal notes as signals for
/// improving clarifying-questions / po-story-overview skills.
pub fn build_improve_narrative_from_clarifications(
    story: &otto_state::ProductStory,
    questions: &[otto_state::ProductQuestion],
    notes: &[otto_state::ProductNote],
) -> String {
    let mut s = String::new();
    s.push_str("# Clarifying-Questions Skill Improvement Signal\n\n");
    s.push_str(
        "The following answered questions and internal notes were captured for this story. \
         Use them to improve the clarifying-questions and po-story-overview skills — \
         what kinds of questions were useful, what gaps they uncovered, and what notes \
         the team found important enough to record.\n\n",
    );
    s.push_str("## Story\n\n");
    s.push_str("**Title:** ");
    s.push_str(&story.title);
    s.push('\n');
    s.push_str("**Source:** ");
    s.push_str(&story.source_kind);
    s.push_str(" / ");
    s.push_str(&story.source_key);
    s.push_str("\n\n");

    let answered: Vec<&otto_state::ProductQuestion> =
        questions.iter().filter(|q| q.status == "answered").collect();

    if answered.is_empty() {
        s.push_str("## Answered Questions\n\n(none)\n\n");
    } else {
        s.push_str("## Answered Questions\n\n");
        for q in &answered {
            s.push_str("**Q:** ");
            s.push_str(&q.text);
            s.push('\n');
            if let Some(ref ans) = q.answer {
                if !ans.trim().is_empty() {
                    s.push_str("**A:** ");
                    s.push_str(ans.trim());
                    s.push('\n');
                }
            }
            if !q.category.is_empty() && q.category != "general" {
                s.push_str("**Category:** ");
                s.push_str(&q.category);
                s.push('\n');
            }
            s.push('\n');
        }
    }

    if notes.is_empty() {
        s.push_str("## Internal Notes\n\n(none)\n");
    } else {
        s.push_str("## Internal Notes\n\n");
        for note in notes {
            if let Some(ref section) = note.section {
                s.push_str("**[");
                s.push_str(section);
                s.push_str("]** ");
            }
            s.push_str(&note.body);
            s.push_str("\n\n");
        }
    }

    s
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only; run_analysis is compile-checked)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // session_cwd — per-session cwd attribution
    // -----------------------------------------------------------------------

    #[test]
    fn session_cwd_passes_real_path_through_unchanged() {
        // A real directory (use the manifest dir of this crate, which surely
        // exists and is NOT the temp dir) must pass through verbatim — the
        // architecture lens needs the actual repo cwd.
        let real = env!("CARGO_MANIFEST_DIR");
        let out = session_cwd(real);
        assert_eq!(out, real, "a real story cwd must pass through unchanged");
    }

    #[test]
    fn session_cwd_rewrites_shared_temp_fallback_to_existing_child() {
        // The fallback string used by the handlers.
        let fallback = std::env::temp_dir().to_string_lossy().to_string();
        let out = session_cwd(&fallback);

        // It must NOT be the shared temp dir itself.
        assert_ne!(out, fallback, "shared temp fallback must be rewritten");

        let out_path = std::path::Path::new(&out);
        // It must be a child of the temp dir...
        assert!(
            out_path.starts_with(std::env::temp_dir()),
            "rewritten cwd must live under the temp dir; got {out}"
        );
        // ...and it must actually exist on disk (created by the helper).
        assert!(out_path.exists(), "rewritten cwd must exist on disk; got {out}");

        // Cleanup.
        let _ = std::fs::remove_dir_all(out_path);
    }

    #[test]
    fn session_cwd_two_calls_yield_distinct_dirs() {
        let fallback = std::env::temp_dir().to_string_lossy().to_string();
        let a = session_cwd(&fallback);
        let b = session_cwd(&fallback);
        assert_ne!(a, b, "two fallback rewrites must yield distinct dirs");

        // Cleanup.
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    // -----------------------------------------------------------------------
    // extract_json_block
    // -----------------------------------------------------------------------

    #[test]
    fn extract_json_block_finds_json_in_fence() {
        let input = "Here is the result:\n```json\n{\"a\":1}\n```\n";
        let v = extract_json_block(input).expect("should find json");
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn extract_json_block_finds_bare_object() {
        let input = r#"Some prose. {"key": "value", "n": 42}"#;
        let v = extract_json_block(input).expect("should find bare object");
        assert_eq!(v["key"], "value");
        assert_eq!(v["n"], 42);
    }

    #[test]
    fn extract_json_block_returns_none_for_prose() {
        let input = "This is just some text without any JSON at all.";
        assert!(extract_json_block(input).is_none());
    }

    #[test]
    fn extract_json_block_fence_wins_over_prose_brace() {
        // There is a `{` in prose text BEFORE the fenced block; the fence should win.
        let input = "intro text { not json } then\n```json\n{\"a\":1}\n```";
        let v = extract_json_block(input).expect("should find fenced json");
        // The fenced {"a":1} wins, not the prose brace.
        assert_eq!(v["a"], 1);
        // Make sure the prose brace wasn't interpreted as the result.
        assert!(v.get("not").is_none());
    }

    #[test]
    fn extract_json_block_empty_string_returns_none() {
        assert!(extract_json_block("").is_none());
    }

    #[test]
    fn extract_json_block_nested_json_balanced() {
        let input = r#"{"outer":{"inner":"yes"},"list":[1,2,3]}"#;
        let v = extract_json_block(input).expect("should parse nested json");
        assert_eq!(v["outer"]["inner"], "yes");
    }

    // -----------------------------------------------------------------------
    // build_analysis_prompt
    // -----------------------------------------------------------------------

    #[test]
    fn build_analysis_prompt_includes_skill_body() {
        let skill = "## Skill: Analyse stories\nBe thorough.";
        let prompt = build_analysis_prompt(skill, "/tmp/ctx.md", None);
        assert!(
            prompt.contains(skill),
            "prompt must include the entire skill body; got:\n{prompt}"
        );
    }

    #[test]
    fn build_analysis_prompt_includes_context_path_reference() {
        let ctx_path = "/tmp/otto-product-TEST-context.md";
        let prompt = build_analysis_prompt("skill body", ctx_path, None);
        assert!(
            prompt.contains(ctx_path),
            "prompt must include the context file path; got:\n{prompt}"
        );
        // The prompt should tell the agent to read the file.
        assert!(
            prompt.to_lowercase().contains("read"),
            "prompt should instruct agent to read the context file; got:\n{prompt}"
        );
    }

    #[test]
    fn build_analysis_prompt_includes_output_contract_marker() {
        let prompt = build_analysis_prompt("skill body", "/tmp/ctx.md", None);
        // The key distinguishing text from the OUTPUT CONTRACT
        assert!(
            prompt.contains("EXACTLY ONE"),
            "prompt must include OUTPUT CONTRACT text 'EXACTLY ONE'; got:\n{prompt}"
        );
        assert!(
            prompt.contains("suggested_learnings"),
            "prompt must include OUTPUT CONTRACT field 'suggested_learnings'"
        );
        assert!(
            prompt.contains("open_questions"),
            "prompt must include OUTPUT CONTRACT field 'open_questions'"
        );
    }

    #[test]
    fn build_analysis_prompt_includes_prior_summary_when_given() {
        let prompt = build_analysis_prompt(
            "skill",
            "/tmp/ctx.md",
            Some("Previously we found X and Y."),
        );
        assert!(
            prompt.contains("Previously we found X and Y."),
            "prompt must include prior summary"
        );
    }

    #[test]
    fn build_analysis_prompt_omits_prior_summary_section_when_none() {
        let prompt = build_analysis_prompt("skill", "/tmp/ctx.md", None);
        assert!(
            !prompt.contains("Prior Analysis Summary"),
            "no prior summary section when None"
        );
    }

    // -----------------------------------------------------------------------
    // augment_with_out_path — the write-to-file instruction
    // -----------------------------------------------------------------------

    #[test]
    fn augment_with_out_path_appends_write_instruction_and_path() {
        let base = build_analysis_prompt("skill body", "/tmp/otto-product-X-context.md", None);
        let out = augment_with_out_path(&base, "/tmp/otto-product-X-0.json");
        // The context path is preserved.
        assert!(out.contains("/tmp/otto-product-X-context.md"));
        // The write-to-file path is present.
        assert!(
            out.contains("/tmp/otto-product-X-0.json"),
            "augmented prompt must include the out_path; got:\n{out}"
        );
        // It instructs writing JSON to the file.
        assert!(out.to_lowercase().contains("write"));
        assert!(out.contains("JSON"));
    }

    // -----------------------------------------------------------------------
    // build_summarizer_prompt — consolidate/dedupe instruction + schema marker
    // -----------------------------------------------------------------------

    #[test]
    fn build_summarizer_prompt_includes_consolidate_dedupe_and_questions_schema() {
        let lenses = vec![
            (
                "PO Overview".to_string(),
                "claude".to_string(),
                r#"{"summary":"a","open_questions":[{"text":"q1"}]}"#.to_string(),
            ),
            (
                "Architecture".to_string(),
                "codex".to_string(),
                r#"{"summary":"b","risks":["r1"]}"#.to_string(),
            ),
        ];
        let prompt = build_summarizer_prompt("synth skill", "Login Story", &lenses);

        // Consolidate / dedupe / resolve-conflict instructions are present.
        assert!(
            prompt.to_lowercase().contains("consolidate"),
            "summarizer prompt must instruct consolidation; got:\n{prompt}"
        );
        assert!(
            prompt.to_uppercase().contains("DEDUPE"),
            "summarizer prompt must instruct dedupe; got:\n{prompt}"
        );
        assert!(
            prompt.to_lowercase().contains("conflict"),
            "summarizer prompt must instruct conflict resolution; got:\n{prompt}"
        );
        // The output schema marker: a `questions` array (the summarizer schema).
        assert!(
            prompt.contains("\"questions\""),
            "summarizer prompt must include the questions schema marker; got:\n{prompt}"
        );
        assert!(
            prompt.contains("conflict_notes"),
            "summarizer prompt must include conflict_notes schema field; got:\n{prompt}"
        );
        // Each lens is labelled with name + provider, and its JSON is fed in.
        assert!(prompt.contains("Lens PO Overview (claude)"));
        assert!(prompt.contains("Lens Architecture (codex)"));
        assert!(prompt.contains(r#"{"summary":"a","open_questions":[{"text":"q1"}]}"#));
        // The story title is included.
        assert!(prompt.contains("Login Story"));
    }

    #[test]
    fn summary_findings_parses_summarizer_schema() {
        let raw = r#"```json
{"summary":"consolidated","questions":[{"text":"q","rationale":"r","category":"scope"}],
 "related_repos":["a"],"functionalities":["f"],"integration_points":["i"],"risks":["x"],
 "suggested_learnings":[{"kind":"pattern","title":"t","body":"b"}],"conflict_notes":"resolved X"}
```"#;
        let v = extract_json_block(raw).expect("should find json");
        let parsed: SummaryFindings =
            serde_json::from_value(v).expect("should parse SummaryFindings");
        assert_eq!(parsed.summary, "consolidated");
        assert_eq!(parsed.questions.len(), 1);
        assert_eq!(parsed.questions[0].text, "q");
        assert_eq!(parsed.suggested_learnings.len(), 1);
        assert_eq!(parsed.conflict_notes, "resolved X");
    }

    // -----------------------------------------------------------------------
    // build_rewrite_prompt
    // -----------------------------------------------------------------------

    #[test]
    fn build_rewrite_prompt_includes_writer_skill_body() {
        let skill = "## Jira Story Writer\nWrite excellent stories.";
        let prompt = build_rewrite_prompt(skill, "/tmp/ctx-rewrite.md");
        assert!(
            prompt.contains(skill),
            "prompt must include the entire writer skill body; got:\n{prompt}"
        );
    }

    #[test]
    fn build_rewrite_prompt_includes_context_path_reference() {
        let ctx_path = "/tmp/otto-product-rewrite-TEST-context.md";
        let prompt = build_rewrite_prompt("skill", ctx_path);
        assert!(
            prompt.contains(ctx_path),
            "prompt must reference the context file path; got:\n{prompt}"
        );
        assert!(
            prompt.to_lowercase().contains("read"),
            "prompt should instruct agent to read the context file; got:\n{prompt}"
        );
    }

    #[test]
    fn build_rewrite_prompt_includes_body_markdown_contract_marker() {
        let prompt = build_rewrite_prompt("skill", "/tmp/ctx.md");
        assert!(
            prompt.contains("body_markdown"),
            "prompt must include 'body_markdown' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("EXACTLY ONE"),
            "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("change_notes"),
            "prompt must include 'change_notes' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
    }

    // -----------------------------------------------------------------------
    // build_tests_prompt
    // -----------------------------------------------------------------------

    #[test]
    fn build_tests_prompt_includes_skill_body_verbatim() {
        let skill = "## Story Test Cases\nDraft readable test cases. Happy path plus validations and error coverage.";
        let prompt = build_tests_prompt(skill, "/tmp/ctx-tests.md");
        assert!(
            prompt.contains(skill),
            "prompt must include the entire tests skill body verbatim; got:\n{prompt}"
        );
    }

    #[test]
    fn build_tests_prompt_skill_body_mentions_happy_and_coverage_guidance() {
        // The bundled skill body mentions "happy" and "validation" / "error" coverage.
        // We embed it verbatim, so the prompt must contain those words.
        let skill = "Cover happy path plus meaningful validations and realistic errors. Not over-defensive.";
        let prompt = build_tests_prompt(skill, "/tmp/ctx.md");
        assert!(
            prompt.contains("happy"),
            "prompt must include 'happy' (from skill body); got:\n{prompt}"
        );
        assert!(
            prompt.contains("validation") || prompt.contains("error"),
            "prompt must reference 'validation' or 'error' coverage (from skill body); got:\n{prompt}"
        );
    }

    #[test]
    fn build_tests_prompt_includes_context_path_reference() {
        let ctx_path = "/tmp/otto-product-tests-TEST-context.md";
        let prompt = build_tests_prompt("skill", ctx_path);
        assert!(
            prompt.contains(ctx_path),
            "prompt must reference the context file path; got:\n{prompt}"
        );
        assert!(
            prompt.to_lowercase().contains("read"),
            "prompt should instruct agent to read the context file; got:\n{prompt}"
        );
    }

    #[test]
    fn build_plan_prompt_includes_context_path_reference() {
        let ctx_path = "/tmp/otto-product-plan-TEST-context.md";
        let prompt = build_plan_prompt("skill", ctx_path);
        assert!(
            prompt.contains(ctx_path),
            "prompt must reference the context file path; got:\n{prompt}"
        );
        assert!(
            prompt.to_lowercase().contains("read"),
            "prompt should instruct agent to read the context file; got:\n{prompt}"
        );
    }

    #[test]
    fn build_plan_prompt_includes_plan_contract_marker() {
        let prompt = build_plan_prompt("skill body", "/tmp/ctx.md");
        assert!(
            prompt.contains("plan_markdown"),
            "prompt must include 'plan_markdown' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("EXACTLY ONE"),
            "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("- [ ]"),
            "prompt must instruct emitting '- [ ]' checkboxes; got:\n{prompt}"
        );
    }

    #[test]
    fn build_tests_prompt_includes_testcases_contract_marker() {
        let prompt = build_tests_prompt("skill body", "/tmp/ctx.md");
        assert!(
            prompt.contains("testcases"),
            "prompt must include 'testcases' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("EXACTLY ONE"),
            "prompt must include 'EXACTLY ONE' from the OUTPUT CONTRACT; got:\n{prompt}"
        );
        assert!(
            prompt.contains("happy|validation|error|edge"),
            "prompt must include category enumeration from the OUTPUT CONTRACT; got:\n{prompt}"
        );
    }

    // -----------------------------------------------------------------------
    // build_improve_narrative_from_tests
    // -----------------------------------------------------------------------

    fn make_testcase(
        title: &str,
        category: &str,
        status: &str,
        review_note: Option<&str>,
    ) -> otto_state::ProductTestcase {
        use chrono::Utc;
        otto_state::ProductTestcase {
            id: otto_core::new_id(),
            run_id: otto_core::new_id(),
            story_id: otto_core::new_id(),
            title: title.to_string(),
            category: category.to_string(),
            priority: "medium".to_string(),
            steps_json: "{}".to_string(),
            status: status.to_string(),
            review_note: review_note.map(|s| s.to_string()),
            order_idx: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_story(title: &str) -> otto_state::ProductStory {
        use chrono::Utc;
        let uid = otto_core::new_id();
        otto_state::ProductStory {
            id: otto_core::new_id(),
            workspace_id: otto_core::new_id(),
            source_kind: "jira".to_string(),
            account_id: uid.clone(),
            source_key: "PROJ-42".to_string(),
            title: title.to_string(),
            url: "https://jira.example.com/PROJ-42".to_string(),
            issue_type: None,
            stage: "tests_drafted".to_string(),
            cwd: None,
            watch_enabled: false,
            watch_cadence_min: 60,
            watch_cursor: None,
            confluence_tests_page_id: None,
            confluence_tests_url: None,
            tags: String::new(),
            created_by: uid,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn narrative_from_tests_contains_story_title() {
        let story = make_story("Add Payment Gateway");
        let cases = vec![
            make_testcase("Happy path checkout", "happy", "approved", None),
            make_testcase("Invalid card number", "validation", "changes_requested", Some("Too generic, add specific error codes")),
        ];
        let narrative = super::build_improve_narrative_from_tests(&story, &cases);
        assert!(
            narrative.contains("Add Payment Gateway"),
            "narrative must contain story title; got:\n{narrative}"
        );
    }

    #[test]
    fn narrative_from_tests_highlights_changes_requested_case() {
        let story = make_story("Checkout Feature");
        let cases = vec![
            make_testcase("Invalid card number", "validation", "changes_requested", Some("Too generic, add specific error codes")),
        ];
        let narrative = super::build_improve_narrative_from_tests(&story, &cases);
        assert!(
            narrative.contains("Invalid card number"),
            "narrative must contain the changed case's title; got:\n{narrative}"
        );
        assert!(
            narrative.contains("changes_requested"),
            "narrative must mention changes_requested status; got:\n{narrative}"
        );
        assert!(
            narrative.contains("Too generic, add specific error codes"),
            "narrative must include the review_note text; got:\n{narrative}"
        );
    }

    #[test]
    fn narrative_from_tests_flags_nonempty_review_note() {
        let story = make_story("My Story");
        let cases = vec![
            make_testcase("Edge case timeout", "edge", "approved", Some("Consider shorter timeout")),
        ];
        let narrative = super::build_improve_narrative_from_tests(&story, &cases);
        assert!(
            narrative.contains("Consider shorter timeout"),
            "narrative must include non-empty review_note even for approved cases; got:\n{narrative}"
        );
    }

    // -----------------------------------------------------------------------
    // build_improve_narrative_from_clarifications
    // -----------------------------------------------------------------------

    fn make_question_full(text: &str, status: &str, answer: Option<&str>) -> otto_state::ProductQuestion {
        use chrono::Utc;
        let dummy = otto_core::new_id();
        otto_state::ProductQuestion {
            id: otto_core::new_id(),
            story_id: dummy.clone(),
            analysis_id: None,
            text: text.to_string(),
            rationale: String::new(),
            category: "scope".to_string(),
            status: status.to_string(),
            answer: answer.map(|a| a.to_string()),
            posted_ref: None,
            created_by: dummy,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_note(body: &str, section: Option<&str>) -> otto_state::ProductNote {
        use chrono::Utc;
        otto_state::ProductNote {
            id: otto_core::new_id(),
            story_id: otto_core::new_id(),
            section: section.map(|s| s.to_string()),
            body: body.to_string(),
            author_id: otto_core::new_id(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn narrative_from_clarifications_contains_answered_question() {
        let story = make_story("User Authentication Story");
        let questions = vec![
            make_question_full("What OAuth providers are supported?", "answered", Some("Google and GitHub only.")),
            make_question_full("Unanswered question", "open", None),
        ];
        let notes = vec![];
        let narrative = super::build_improve_narrative_from_clarifications(&story, &questions, &notes);
        assert!(
            narrative.contains("What OAuth providers are supported?"),
            "narrative must contain answered question text; got:\n{narrative}"
        );
        assert!(
            narrative.contains("Google and GitHub only."),
            "narrative must contain the answer text; got:\n{narrative}"
        );
        // Unanswered questions should not appear as answered
        assert!(
            !narrative.contains("Unanswered question"),
            "narrative must only show answered questions; got:\n{narrative}"
        );
    }

    #[test]
    fn narrative_from_clarifications_contains_note_body() {
        let story = make_story("Payment Story");
        let questions = vec![];
        let notes = vec![
            make_note("Remember to handle 3DS authentication flow", Some("security")),
        ];
        let narrative = super::build_improve_narrative_from_clarifications(&story, &questions, &notes);
        assert!(
            narrative.contains("Remember to handle 3DS authentication flow"),
            "narrative must contain the note body; got:\n{narrative}"
        );
    }
}

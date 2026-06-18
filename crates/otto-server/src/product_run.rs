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

use std::collections::HashSet;
use std::path::Path;
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

use crate::review_session::{
    bracketed_paste, dispatched, wait_for_tui, FINDINGS_POLL, PASTE_TO_ENTER, WAITING_IDLE,
};
use crate::state::ServerCtx;

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

/// Outcome of one lens (or the summarizer) running as a real session.
pub struct LensRunResult {
    /// Raw text the agent wrote to its out file (or the claude transcript turn).
    pub raw: Option<String>,
    /// The live SessionManager session id (so the agent stays openable).
    pub session_id: Option<Id>,
    /// True if the agent never produced output (timeout / exit / start failure).
    pub errored: bool,
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
) -> LensRunResult {
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
            return LensRunResult { raw: None, session_id: None, errored: true };
        }
    };
    let sid = session.id.clone();

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

    // Watch for the out file (or, for claude, its JSONL transcript turn),
    // detecting exit / idle / timeout. We never kill the session.
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = std::fs::read_to_string(out_path) {
            let _ = std::fs::remove_file(out_path);
            return LensRunResult { raw: Some(text), session_id: Some(sid), errored: false };
        }

        // claude-transcript fallback: codex/agy write no transcript, so the file
        // is their reliable path; claude's JSONL is a backstop if it skipped the
        // file write but produced a completed turn that parses as our Findings.
        if provider == "claude" {
            if let Some(psid) = session.provider_session_id.as_deref() {
                let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
                if let Ok(raw) = std::fs::read_to_string(&jsonl) {
                    if let Some(turn) = otto_orchestrator::claude_pty::completed_turn_text(&raw) {
                        if extract_json_block(&turn).is_some() {
                            return LensRunResult {
                                raw: Some(turn),
                                session_id: Some(sid),
                                errored: false,
                            };
                        }
                    }
                }
            }
        }

        match ctx.manager.live_handle(&sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    warn!("product_run: session ({provider}) exited before writing JSON");
                    return LensRunResult { raw: None, session_id: Some(sid), errored: true };
                }
                // Idle-with-no-output → likely blocked on input; keep waiting
                // (the PO can Open it) until the overall timeout.
                let _idle = handle.last_output_at().elapsed() >= WAITING_IDLE;
            }
            None => {
                warn!("product_run: session ({provider}) is no longer live");
                return LensRunResult { raw: None, session_id: Some(sid), errored: true };
            }
        }

        if Instant::now() >= deadline {
            warn!("product_run: session ({provider}) timed out");
            return LensRunResult { raw: None, session_id: Some(sid), errored: true };
        }
        tokio::time::sleep(FINDINGS_POLL).await;
    }
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
        let cwd = cwd.clone();

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
            let result = run_lens_session(
                &ctx,
                &ws,
                &user_id,
                &spec.provider,
                &cwd,
                &prompt,
                &out_path,
                LENS_TIMEOUT,
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
                    let err = if result.errored {
                        "session produced no output (timeout/exit/start failure)".to_string()
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

        let result = run_lens_session(
            &ctx,
            &ws,
            &user_id,
            &summarizer_provider,
            &cwd,
            &prompt,
            &out_path,
            SUMMARIZER_TIMEOUT,
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

    // 5. Pre-trust provider.
    let cwd = story.cwd.clone().unwrap_or_else(|| {
        std::env::temp_dir().to_string_lossy().to_string()
    });
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
    let result = run_lens_session(
        &ctx,
        &ws,
        &user_id,
        &agent.provider,
        &cwd,
        &prompt,
        &out_path,
        LENS_TIMEOUT,
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
        }
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
    let result = run_lens_session(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
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
    let result = run_lens_session(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
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
    let result = run_lens_session(
        &ctx,
        &ws,
        &user_id,
        &provider,
        &cwd,
        &prompt,
        &out_path,
        Duration::from_secs(300),
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

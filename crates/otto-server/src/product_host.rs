//! PTY mechanics behind the product runners (`otto_product::run`): spawn a
//! provider as a real, openable [`otto_sessions::SessionManager`] session,
//! inject the prompt (codex settle + verify-and-repaste), and watch it to a
//! result with bounded auto-recovery. Lives here, beside `agent_run` /
//! `review_session`, because it drives the daemon's shared session infra; the
//! engine reaches it through [`otto_product::ProductRunHost`]. Also implements
//! [`otto_product::ProductStudioHost`] (the story-studio handlers' host hooks).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::SessionKind;
use otto_core::Id;
use otto_product::run::{
    extract_json_block, CancelRegistry, LensRunResult, SessionAppearance, MAX_AGENT_ATTEMPTS,
};
use tracing::warn;

use crate::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome, WatchStatus};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;

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

/// Quiet for this long with no result ⇒ flag the agent "waiting" (may be blocked
/// on input); < STUCK_IDLE so there's a window before auto-retry.
const WAITING_IDLE: Duration = Duration::from_secs(45);
/// No PTY output AND no result file for this long ⇒ the agent is stuck; fail fast
/// so the recovery wrapper can kill + retry instead of waiting out the timeout.
const STUCK_IDLE: Duration = Duration::from_secs(180);
/// Backoff before each retry attempt (index = retry number - 1). Clamped to the
/// last entry beyond its length.
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(4)];

/// Flatten the run mechanics' outcome into the engine-facing shape (keeping
/// `reason` as a stable `&str` for the notification/error-note code).
fn lens_result(o: RunOutcome) -> LensRunResult {
    LensRunResult {
        errored: o.errored(),
        reason: o.reason.map(|r| r.as_str()),
        raw: o.raw,
        session_id: o.session_id,
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
/// (use [`otto_product::run::augment_with_out_path`] / the prompt builders, which append that).
///
/// When `agent_id` is `Some`, the freshly-created session id is persisted to that
/// analysis-agent row IMMEDIATELY (before the agent does any work), mirroring
/// `review_session::run_agent_session`. That's what lets the UI show "Open" and
/// stream the live terminal *while the agent is running* — not only once it
/// finishes — and keeps the session replayable afterward as history. Callers with
/// no agent row (rewrite / generate-tests / generate-plan) pass `None`.
///
/// `model` — when `Some` and the provider supports `--model`, the spawn path in
/// `SessionManager` injects `--model <name>` into the CLI args (via `model_args`
/// in manager.rs).  When the provider is `agy`/`shell` (no flag), the model is
/// stored in meta for attribution only and the provider falls back to its default.
/// A `None` leaves the provider's default model intact.
///
/// `work` — optional [`otto_core::workref::WorkRef`] serialized to a
/// `serde_json::Value`; written into `meta["work"]` at session creation so the
/// usage layer can attribute cost back to the originating story/task.
#[allow(clippy::too_many_arguments)]
async fn run_lens_session(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user_id: &Id,
    provider: &str,
    model: Option<&str>,
    work: Option<serde_json::Value>,
    cwd: &str,
    prompt: &str,
    out_path: &Path,
    timeout: Duration,
    agent_id: Option<&Id>,
    appearance: &SessionAppearance,
    on_session: Option<&(dyn Fn(&Id) + Send + Sync)>,
) -> RunOutcome {
    // Clear any stale output from a previous run.
    let _ = std::fs::remove_file(out_path);

    // Carry model into meta so SessionManager can inject `--model <name>` for
    // providers that support it (claude/codex); for others it is attribution-only.
    // The work-graph ref is stored under "work" for usage attribution.
    let mut session_meta = serde_json::json!({ "source": appearance.source });
    if let Some(obj) = session_meta.as_object_mut() {
        if let Some(m) = model.filter(|s| !s.trim().is_empty()) {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(m.trim().to_string()),
            );
        }
        if let Some(w) = work {
            obj.insert("work".to_string(), w);
        }
    }

    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: Some(appearance.title.clone()),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        model: None,
        meta: Some(session_meta),
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

    // Early session-id hook (plan flow): surface the live session id the moment
    // it exists so the caller can tile it side-by-side while it runs — analysis
    // callers (which track ids via the agent DB row) pass `None`.
    if let Some(cb) = on_session {
        cb(&sid);
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
        Some(|t| extract_json_block(t).is_some()),
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

fn register_cancel(reg: &CancelRegistry, agent_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    reg.lock()
        .unwrap()
        .insert(agent_id.to_string(), Arc::clone(&flag));
    flag
}

/// Remove `agent_id`'s cancel flag ONLY when it is still `flag` (S4-16): a
/// Stop + quick Retry registers a NEW flag under the same agent id, and the
/// old loop's exit must not unregister it (a later Stop would go unheard).
/// Returns `false` when a newer run superseded this one.
fn unregister_cancel(reg: &CancelRegistry, agent_id: &str, flag: &Arc<AtomicBool>) -> bool {
    let mut map = reg.lock().unwrap();
    match map.get(agent_id) {
        Some(cur) if Arc::ptr_eq(cur, flag) => {
            map.remove(agent_id);
            true
        }
        Some(_) => false,
        None => false,
    }
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
///
/// `model` — forwarded to [`run_lens_session`]; see that function for the
/// fall-back note for providers that have no `--model` flag.
///
/// `work` — forwarded to [`run_lens_session`] for work-graph attribution.
#[allow(clippy::too_many_arguments)]
async fn run_agent_with_recovery(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user_id: &Id,
    provider: &str,
    model: Option<&str>,
    work: Option<serde_json::Value>,
    cwd: &str,
    prompt: &str,
    out_path: &Path,
    timeout: Duration,
    agent_id: Option<&Id>,
    appearance: &SessionAppearance,
    on_session: Option<&(dyn Fn(&Id) + Send + Sync)>,
) -> LensRunResult {
    // No agent row (rewrite / tests / plan): key the flag by the story the
    // run is attributed to, so deleting the story stops it (S4-23).
    let cancel_key = match agent_id {
        Some(a) => Some(a.to_string()),
        None => work
            .as_ref()
            .and_then(|w| w.get("story_id"))
            .and_then(|v| v.as_str())
            .map(otto_product::run::story_run_cancel_key),
    };
    let cancel = cancel_key
        .as_deref()
        .map(|key| register_cancel(&ctx.product_agent_cancels, key));

    let outcome = run_with_recovery(
        &ctx.manager,
        MAX_AGENT_ATTEMPTS,
        &RETRY_BACKOFF,
        cancel.as_ref(),
        |_attempt| {
            run_lens_session(
                ctx,
                ws,
                user_id,
                provider,
                model,
                work.clone(),
                cwd,
                prompt,
                out_path,
                timeout,
                agent_id,
                appearance,
                on_session,
            )
        },
    )
    .await;

    let mut superseded = false;
    if let (Some(key), Some(flag)) = (cancel_key.as_deref(), cancel.as_ref()) {
        superseded = !unregister_cancel(&ctx.product_agent_cancels, key, flag);
    }
    if superseded {
        // A Retry took this agent row over while we ran: our result (and any
        // final "error"/"done" status write) must not clobber the new attempt.
        return LensRunResult {
            raw: None,
            // Nor re-point the row's session at this superseded attempt.
            session_id: None,
            errored: true,
            reason: Some(otto_product::run::SUPERSEDED),
        };
    }
    lens_result(outcome)
}

impl otto_product::ProductRunHost for ServerCtx {
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn workspaces(&self) -> &otto_state::WorkspacesRepo {
        &self.workspaces
    }
    fn agent_cancels(&self) -> &CancelRegistry {
        &self.product_agent_cancels
    }
    fn context_library(&self) -> &otto_context::Library {
        &self.context_library
    }
    #[allow(clippy::too_many_arguments)]
    fn run_agent_with_recovery(
        &self,
        ws: &otto_core::domain::Workspace,
        user_id: &Id,
        provider: &str,
        model: Option<&str>,
        work: Option<serde_json::Value>,
        cwd: &str,
        prompt: &str,
        out_path: &Path,
        timeout: Duration,
        agent_id: Option<&Id>,
        appearance: &SessionAppearance,
        on_session: Option<&(dyn Fn(&Id) + Send + Sync)>,
    ) -> impl std::future::Future<Output = LensRunResult> + Send {
        run_agent_with_recovery(
            self, ws, user_id, provider, model, work, cwd, prompt, out_path, timeout, agent_id,
            appearance, on_session,
        )
    }
}

impl otto_product::ProductStudioHost for ServerCtx {
    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn attachments(&self) -> &otto_state::ProductAttachmentRepo {
        &self.attachment_repo
    }
    fn discovery_chat_repo(&self) -> &otto_state::DiscoveryChatRepo {
        &self.discovery_chat_repo
    }
    fn discovery_repo(&self) -> &otto_state::ProductDiscoveryRepo {
        &self.discovery_repo
    }
    fn refinement_repo(&self) -> &otto_state::ProductRefinementRepo {
        &self.refinement_repo
    }
    fn mockup_repo(&self) -> &otto_state::ProductMockupRepo {
        &self.mockup_repo
    }
    fn canvas_repo(&self) -> &otto_state::CanvasRepo {
        &self.canvas_repo
    }
    fn swarms(&self) -> &otto_state::SwarmRepo {
        &self.swarm_repo
    }
    fn orchestrator(&self) -> &Arc<otto_orchestrator::Orchestrator> {
        &self.orchestrator
    }
    fn pool(&self) -> &otto_state::DbPool {
        &self.pool
    }
    fn improve_engine(&self) -> &Arc<otto_improve::ImprovementEngine> {
        &self.improve_engine
    }
    async fn kill_session(&self, sid: &Id) -> otto_core::Result<()> {
        self.manager.kill_session(sid).await
    }
    fn resolve_provider(
        &self,
        ws: Option<&otto_core::domain::Workspace>,
        requested: Option<&str>,
    ) -> impl std::future::Future<Output = otto_core::Result<String>> + Send {
        ServerCtx::resolve_provider(self, ws, requested)
    }
    fn session_stuck_idle(&self) -> Duration {
        crate::agent_session::STUCK_IDLE
    }
    #[allow(clippy::too_many_arguments)]
    async fn run_session_turn(
        &self,
        ws: &otto_core::domain::Workspace,
        user: &otto_core::domain::User,
        existing: Option<&Id>,
        title: &str,
        cwd: &str,
        provider: &str,
        meta: serde_json::Value,
        prompt: &str,
        stuck_after: Duration,
    ) -> otto_core::Result<(String, Id)> {
        crate::agent_session::require_owned_resume(&self.pool, &ws.id, &user.id, existing).await?;
        crate::agent_session::run_session_turn(
            self,
            ws,
            user,
            existing,
            title,
            cwd,
            provider,
            meta,
            prompt,
            stuck_after,
            |_| {},
        )
        .await
        .map_err(|e| e.0)
    }
    async fn usage_budget(&self, workspace_id: &str, provider: &str) -> otto_product::BudgetGate {
        let v = crate::routes::usage::check_budget(self, workspace_id, provider).await;
        otto_product::BudgetGate {
            blocked: v.blocked,
            reason: v.reason,
        }
    }
    fn start_swarm_coordinator(&self, swarm_id: Id) {
        otto_swarm::runtime::engine::start_coordinator(self.swarm_rt(), swarm_id);
    }
    fn emit_swarm_status(&self, workspace_id: &Id, swarm_id: &str, status: &str) {
        otto_swarm::runtime::engine::emit_status(&self.swarm_rt(), workspace_id, swarm_id, status);
    }
}

#[cfg(test)]
mod cancel_registry_tests {
    use super::*;

    #[test]
    fn old_completion_is_superseded_after_successor_already_finished() {
        let reg: CancelRegistry = Default::default();
        let old = register_cancel(&reg, "a1");
        let new = register_cancel(&reg, "a1");
        assert!(unregister_cancel(&reg, "a1", &new));
        assert!(!unregister_cancel(&reg, "a1", &old));
    }

    /// S4-16: the old loop's exit never unregisters a Retry's newer flag.
    #[test]
    fn unregister_only_removes_the_same_flag() {
        let reg: CancelRegistry = Default::default();
        let old = register_cancel(&reg, "a1");
        let new = register_cancel(&reg, "a1"); // Retry re-registers
        assert!(
            !unregister_cancel(&reg, "a1", &old),
            "old run is superseded"
        );
        assert!(Arc::ptr_eq(reg.lock().unwrap().get("a1").unwrap(), &new));
        assert!(unregister_cancel(&reg, "a1", &new));
        assert!(reg.lock().unwrap().get("a1").is_none());
    }
}

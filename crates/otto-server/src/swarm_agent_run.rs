//! Run swarm "plan" / "recruit" turns as REAL, openable [`SessionManager`]
//! sessions (like the PR-review engine) so the operator can watch the agent live
//! and Stop it server-side — instead of a headless one-shot `claude_pty` call.
//!
//! Each turn is recorded as a `SwarmRun` (kind `"plan"`/`"recruit"`) linked to
//! its session, so it shows in the Runs list with an Open button and streams
//! `SwarmRunUpdated` progress (running → waiting → done/error/stopped). There is
//! no wall-clock cap — a turn runs while it makes progress; only a stall
//! (`SWARM_STUCK_IDLE` with no output) retries it, and a 1h backstop bounds a
//! truly wedged session. A per-swarm cancel flag (set by the Stop endpoint)
//! short-circuits retries and kills the live session(s).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, User, Workspace};
use otto_core::Id;
use otto_state::{NewRun, RunPatch};

use crate::agent_run::{watch_for_result, FailReason, WatchStatus};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER, WAITING_IDLE};
use crate::state::ServerCtx;
use crate::swarm_run::emit_run;

/// No-progress (stuck) window before an attempt is retried.
const SWARM_STUCK_IDLE: Duration = Duration::from_secs(240);
/// Absolute backstop so a wedged session can't run forever (no other cap).
const SWARM_RUN_TIMEOUT: Duration = Duration::from_secs(3600);
/// Attempts before giving up (kills the stuck session + respawns between tries).
const SWARM_MAX_ATTEMPTS: u32 = 3;
const SWARM_RETRY_BACKOFF: Duration = Duration::from_secs(3);

// --- per-swarm cancel registry --------------------------------------------

/// Shared cancel state for a swarm's in-flight plan/recruit: a flag (stops
/// retries) + the live session ids (so Stop can kill them mid-turn).
#[derive(Clone)]
pub struct CancelState {
    flag: Arc<AtomicBool>,
    sessions: Arc<Mutex<Vec<Id>>>,
}

impl CancelState {
    pub fn cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
    fn track(&self, sid: &Id) {
        self.sessions.lock().unwrap().push(sid.clone());
    }
    /// A cancel handle NOT registered in the per-swarm registry — for callers
    /// (e.g. the verification controller) that own their own cancellation keyed
    /// elsewhere and must not clobber an in-flight plan/recruit handle.
    pub fn detached() -> CancelState {
        CancelState {
            flag: Arc::new(AtomicBool::new(false)),
            sessions: Arc::new(Mutex::new(Vec::new())),
        }
    }
    /// Flip the cancel flag (stops retries on the next check).
    pub fn signal(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
    /// Kill every session this handle has tracked (mid-turn abort).
    pub async fn kill_tracked(&self, ctx: &ServerCtx) {
        let sids = self.sessions.lock().unwrap().clone();
        for sid in sids {
            let _ = ctx.manager.kill_session(&sid).await;
        }
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<String, CancelState>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, CancelState>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Begin a cancellable plan/recruit for `swarm_id` (replaces any prior handle).
pub fn begin(swarm_id: &str) -> CancelState {
    let cs = CancelState {
        flag: Arc::new(AtomicBool::new(false)),
        sessions: Arc::new(Mutex::new(Vec::new())),
    };
    registry().lock().unwrap().insert(swarm_id.to_string(), cs.clone());
    cs
}

/// Drop the cancel handle once the plan/recruit finishes.
pub fn end(swarm_id: &str) {
    registry().lock().unwrap().remove(swarm_id);
}

/// Stop the in-flight plan/recruit for `swarm_id`: flag retries off + kill any
/// live session(s). No-op if nothing is running.
pub async fn stop(ctx: &ServerCtx, swarm_id: &str) {
    let cs = registry().lock().unwrap().get(swarm_id).cloned();
    if let Some(cs) = cs {
        cs.flag.store(true, Ordering::Relaxed);
        let sids = cs.sessions.lock().unwrap().clone();
        for sid in sids {
            let _ = ctx.manager.kill_session(&sid).await;
        }
    }
}

// --- the run -----------------------------------------------------------------

/// Run one agent turn as an openable session tied to a fresh `SwarmRun`. Returns
/// `(reply_text, run_id)` — `reply_text` is `None` on failure/stop. The session
/// is left OPEN on success (so the operator can inspect it) and killed between
/// retries / on stop. `transcript_ok` decides when the turn is complete.
#[allow(clippy::too_many_arguments)]
pub async fn run_swarm_agent(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    swarm_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    nominal_agent_id: &str,
    provider: &str,
    kind: &str,
    title: &str,
    cwd: &str,
    prompt: &str,
    transcript_ok: fn(&str) -> bool,
    cancel: &CancelState,
) -> (Option<String>, Id) {
    // Turn start — bounds the per-turn token/cost backfill below.
    let turn_started_at = chrono::Utc::now();
    // Canonicalize the cwd: claude resolves symlinks (e.g. macOS /var →
    // /private/var) when it computes its transcript dir, so we MUST create the
    // session in — and poll — the same resolved path, or watch_for_result never
    // finds the completed turn and the run sits "running" until it (wrongly)
    // retries. The headless `claude_pty` path applies the same fix.
    let cwd_canon = std::fs::canonicalize(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string());
    let cwd: &str = &cwd_canon;
    let run = match ctx
        .swarm_repo
        .create_run(NewRun {
            swarm_id: swarm_id.to_string(),
            workspace_id: ws.id.clone(),
            project_id: project_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            agent_id: nominal_agent_id.to_string(),
            kind: kind.to_string(),
            trigger: "manual".to_string(),
        })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("swarm_agent_run: create_run: {e}");
            return (None, String::new());
        }
    };
    let run_id = run.id.clone();
    let _ = set_run(ctx, &run_id, "running", None, false).await;

    let out_path = std::env::temp_dir().join(format!("otto-swarm-{run_id}.out"));
    let _ = std::fs::remove_file(&out_path);

    let mut last_reason: Option<FailReason> = None;
    for attempt in 0..SWARM_MAX_ATTEMPTS {
        if cancel.cancelled() {
            last_reason = Some(FailReason::Stopped);
            break;
        }
        if attempt > 0 {
            tokio::time::sleep(SWARM_RETRY_BACKOFF).await;
            if cancel.cancelled() {
                last_reason = Some(FailReason::Stopped);
                break;
            }
        }

        let meta = serde_json::json!({
            "source": "swarm",
            "swarm_id": swarm_id,
            "swarm_run_id": run_id,
            "kind": kind,
        });
        let req = CreateSessionReq {
            kind: SessionKind::Agent,
            provider: Some(provider.to_string()),
            title: Some(title.to_string()),
            cwd: Some(cwd.to_string()),
            connection_id: None,
            meta: Some(meta),
        };
        let session = match ctx.manager.create(ws, &user.id, req, None).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("swarm_agent_run: create session: {e}");
                last_reason = Some(FailReason::CreateFailed);
                continue;
            }
        };
        let sid = session.id.clone();
        cancel.track(&sid);
        // Link the session so the UI's Open button works while it runs.
        let _ = ctx
            .swarm_repo
            .update_run(
                &run_id,
                RunPatch { session_id: Some(Some(sid.clone())), status: Some("running".into()), ..Default::default() },
            )
            .await;
        emit_run(ctx, &run_id).await;

        // Inject the prompt once the TUI has settled, confirming dispatch.
        if wait_for_tui(&ctx.manager, &sid).await {
            let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
            tokio::time::sleep(PASTE_TO_ENTER).await;
            let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
            let _ = ctx.manager.input(&sid, b"\r").await;
            if !dispatched(&ctx.manager, &sid, before).await {
                let _ = ctx.manager.input(&sid, b"\r").await;
            }
        }

        let cb_run_id = run_id.clone();
        let outcome = watch_for_result(
            &ctx.manager,
            &sid,
            provider,
            session.provider_session_id.as_deref(),
            cwd,
            out_path.as_path() as &Path,
            SWARM_RUN_TIMEOUT,
            WAITING_IDLE,
            SWARM_STUCK_IDLE,
            transcript_ok,
            move |st| {
                let rid = cb_run_id.clone();
                async move {
                    let status = match st {
                        WatchStatus::Waiting => "waiting",
                        WatchStatus::Resumed => "running",
                    };
                    let _ = ctx
                        .swarm_repo
                        .update_run(&rid, RunPatch { status: Some(status.into()), ..Default::default() })
                        .await;
                    emit_run(ctx, &rid).await;
                }
            },
        )
        .await;

        if let Some(raw) = outcome.raw {
            // Best-effort per-turn token/cost backfill so verify/fix spend counts
            // against the swarm budget (review M3). Bounded to this turn.
            let (toks_in, toks_out, cost) =
                crate::swarm_run::session_usage(ctx, Some(&sid), turn_started_at).await;
            let _ = ctx
                .swarm_repo
                .update_run(
                    &run_id,
                    RunPatch {
                        tokens_input: Some(toks_in),
                        tokens_output: Some(toks_out),
                        cost_usd: Some(cost),
                        ..Default::default()
                    },
                )
                .await;
            // Success: leave the session open for inspection.
            let _ = set_run(ctx, &run_id, "done", None, true).await;
            return (Some(raw), run_id);
        }
        last_reason = outcome.reason;
        // Kill the stuck/failed session before the next attempt.
        if let Some(s) = outcome.session_id {
            let _ = ctx.manager.kill_session(&s).await;
        }
    }

    let status = if matches!(last_reason, Some(FailReason::Stopped)) || cancel.cancelled() {
        "stopped"
    } else {
        "error"
    };
    let err = last_reason.map(|r| r.as_str().to_string());
    let _ = set_run(ctx, &run_id, status, err, true).await;
    (None, run_id)
}

/// Patch a run's status (+ optional error/finished_at) and emit the update.
async fn set_run(ctx: &ServerCtx, run_id: &str, status: &str, error: Option<String>, finished: bool) {
    let patch = RunPatch {
        status: Some(status.to_string()),
        error: error.map(Some),
        started_at: if status == "running" && !finished { Some(Some(chrono::Utc::now())) } else { None },
        finished_at: if finished { Some(Some(chrono::Utc::now())) } else { None },
        ..Default::default()
    };
    let _ = ctx.swarm_repo.update_run(&run_id.to_string(), patch).await;
    emit_run(ctx, run_id).await;
}

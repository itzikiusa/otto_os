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

use crate::runtime::host::SwarmRt;
use crate::runtime::host::{FailReason, WatchStatus};
use crate::runtime::run::emit_run;

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
    pub(crate) fn same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flag, &other.flag)
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
    pub async fn kill_tracked(&self, ctx: &SwarmRt) {
        let sids = self.sessions.lock().unwrap().clone();
        for sid in sids {
            let _ = ctx.manager().kill_session(&sid).await;
        }
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<String, CancelState>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, CancelState>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The meta-agent turn kinds Stop can target separately (S17-307: "Stop
/// planning" used to kill an in-flight recruit too, and vice versa).
pub const AGENT_KINDS: [&str; 2] = ["plan", "recruit"];

fn key(swarm_id: &str, kind: &str) -> String {
    format!("{swarm_id}\u{0}{kind}")
}

/// Begin a cancellable `kind` (`plan`/`recruit`) turn for `swarm_id`
/// without replacing the Stop handle of an already admitted turn.
pub fn begin(swarm_id: &str, kind: &str) -> otto_core::Result<CancelState> {
    let mut registry = registry().lock().unwrap();
    let key = key(swarm_id, kind);
    if registry.contains_key(&key) {
        return Err(otto_core::Error::Conflict(format!(
            "{kind} is already running for this swarm"
        )));
    }
    let cs = CancelState {
        flag: Arc::new(AtomicBool::new(false)),
        sessions: Arc::new(Mutex::new(Vec::new())),
    };
    registry.insert(key, cs.clone());
    Ok(cs)
}

/// Drop the cancel handle once the plan/recruit finishes.
pub fn end(swarm_id: &str, kind: &str) {
    registry().lock().unwrap().remove(&key(swarm_id, kind));
}

/// Stop the in-flight `kind` turn for `swarm_id` (every kind when `None`):
/// flag retries off + kill any live session(s). Returns whether anything was
/// running.
pub async fn stop(ctx: &SwarmRt, swarm_id: &str, kind: Option<&str>) -> bool {
    let handles: Vec<CancelState> = {
        let reg = registry().lock().unwrap();
        AGENT_KINDS
            .iter()
            .filter(|k| kind.is_none_or(|want| want == **k))
            .filter_map(|k| reg.get(&key(swarm_id, k)).cloned())
            .collect()
    };
    for cs in &handles {
        cs.flag.store(true, Ordering::Relaxed);
        let sids = cs.sessions.lock().unwrap().clone();
        for sid in sids {
            let _ = ctx.manager().kill_session(&sid).await;
        }
    }
    !handles.is_empty()
}

// --- the run -----------------------------------------------------------------

/// Run one agent turn as an openable session tied to a fresh `SwarmRun`. Returns
/// `(reply_text, run_id)` — `reply_text` is `None` on failure/stop. The session
/// is left OPEN on success (so the operator can inspect it) and killed between
/// retries / on stop. `transcript_ok` decides when the turn is complete.
#[allow(clippy::too_many_arguments)]
pub async fn run_swarm_agent(
    ctx: &SwarmRt,
    ws: &Workspace,
    user: &User,
    swarm_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    nominal_agent_id: &str,
    provider: &str,
    model: Option<&str>,
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
    let cwd_canon = tokio::fs::canonicalize(cwd)
        .await
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string());
    let cwd: &str = &cwd_canon;
    let operation = crate::runtime::engine::operation_guard(swarm_id).await;
    if cancel.cancelled() {
        return (None, String::new());
    }
    let run = match ctx
        .swarm_repo()
        .reserve_run(
            NewRun {
                swarm_id: swarm_id.to_string(),
                workspace_id: ws.id.clone(),
                project_id: project_id.map(|s| s.to_string()),
                task_id: task_id.map(|s| s.to_string()),
                agent_id: nominal_agent_id.to_string(),
                kind: kind.to_string(),
                trigger: "manual".to_string(),
            },
            !matches!(kind, "verify" | "fix"),
        )
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
    drop(operation);

    let out_path = std::env::temp_dir().join(format!("otto-swarm-{run_id}.out"));
    let _ = tokio::fs::remove_file(&out_path).await;

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

        let operation = crate::runtime::engine::operation_guard(swarm_id).await;
        if cancel.cancelled()
            || !matches!(ctx.swarm_repo().get_run(&run_id).await,
            Ok(ref run) if matches!(run.status.as_str(), "queued" | "running" | "waiting"))
        {
            last_reason = Some(FailReason::Stopped);
            break;
        }
        let mut meta = serde_json::json!({
            "source": "swarm",
            "swarm_id": swarm_id,
            "swarm_run_id": run_id,
            "project_id": project_id,
            "kind": kind,
        });
        // A chosen model reaches the CLI only via meta["model"] (→ --model);
        // omit when empty so the provider default holds.
        if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty()) {
            meta["model"] = serde_json::json!(m);
        }
        let req = CreateSessionReq {
            kind: SessionKind::Agent,
            provider: Some(provider.to_string()),
            title: Some(title.to_string()),
            cwd: Some(cwd.to_string()),
            connection_id: None,
            model: None,
            meta: Some(meta),
        };
        let session = match ctx.manager().create(ws, &user.id, req, None).await {
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
            .swarm_repo()
            .update_run_if_status(
                &run_id,
                &["queued", "running", "waiting"],
                RunPatch {
                    session_id: Some(Some(sid.clone())),
                    status: Some("running".into()),
                    ..Default::default()
                },
            )
            .await;
        emit_run(ctx, &run_id).await;
        drop(operation);

        // Inject the prompt once the TUI has settled, confirming dispatch. A
        // dead/exited PTY means the prompt was NEVER sent — kill and retry
        // instead of watching a promptless session until the stuck window.
        if crate::runtime::engine::while_run_active(
            ctx.swarm_repo(),
            &run_id,
            ctx.wait_for_tui(&sid),
        )
        .await
            == Some(true)
        {
            if !crate::runtime::engine::send_run_input(
                ctx,
                swarm_id,
                &run_id,
                &sid,
                &ctx.bracketed_paste(prompt),
            )
            .await
            {
                last_reason = Some(FailReason::Stopped);
                break;
            }
            tokio::time::sleep(ctx.pty_timings().paste_to_enter).await;
            let before = ctx.manager().live_handle(&sid).map(|h| h.last_output_at());
            if !crate::runtime::engine::send_run_input(ctx, swarm_id, &run_id, &sid, b"\r").await {
                last_reason = Some(FailReason::Stopped);
                break;
            }
            if !ctx.dispatched(&sid, before).await
                && !crate::runtime::engine::send_run_input(ctx, swarm_id, &run_id, &sid, b"\r")
                    .await
            {
                last_reason = Some(FailReason::Stopped);
                break;
            }
        } else {
            tracing::warn!(
                "swarm_agent_run: TUI never settled for session {sid} — prompt not injected"
            );
            last_reason = Some(FailReason::Exited);
            let _ = ctx.manager().kill_session(&sid).await;
            continue;
        }
        // For claude, confirm the prompt LANDED (a "user" record in the fresh
        // session's transcript) — TUI echo can be redraw noise around a
        // swallowed paste. Re-inject once, else kill + retry this attempt.
        if provider == "claude"
            && !crate::runtime::engine::while_run_active(
                ctx.swarm_repo(),
                &run_id,
                ctx.claude_prompt_landed(&sid, cwd, 0, ctx.pty_timings().prompt_land_wait),
            )
            .await
            .unwrap_or(false)
        {
            tracing::warn!(
                "swarm_agent_run: prompt didn't land in session {sid} — re-injecting once"
            );
            if !crate::runtime::engine::send_run_input(
                ctx,
                swarm_id,
                &run_id,
                &sid,
                &ctx.bracketed_paste(prompt),
            )
            .await
            {
                last_reason = Some(FailReason::Stopped);
                break;
            }
            tokio::time::sleep(ctx.pty_timings().paste_to_enter).await;
            if !crate::runtime::engine::send_run_input(ctx, swarm_id, &run_id, &sid, b"\r").await {
                last_reason = Some(FailReason::Stopped);
                break;
            }
            if !crate::runtime::engine::while_run_active(
                ctx.swarm_repo(),
                &run_id,
                ctx.claude_prompt_landed(&sid, cwd, 0, ctx.pty_timings().prompt_land_wait),
            )
            .await
            .unwrap_or(false)
            {
                tracing::warn!(
                    "swarm_agent_run: prompt never landed in session {sid} — retrying attempt"
                );
                last_reason = Some(FailReason::Stuck);
                let _ = ctx.manager().kill_session(&sid).await;
                continue;
            }
        }

        let cb_run_id = run_id.clone();
        let on_status: &mut crate::runtime::host::StatusFn<'_> = &mut move |st| {
            let rid = cb_run_id.clone();
            Box::pin(async move {
                let status = match st {
                    WatchStatus::Waiting => "waiting",
                    WatchStatus::Resumed => "running",
                };
                let _ = ctx
                    .swarm_repo()
                    .update_run_if_status(
                        &rid,
                        &["queued", "running", "waiting"],
                        RunPatch {
                            status: Some(status.into()),
                            ..Default::default()
                        },
                    )
                    .await;
                emit_run(ctx, &rid).await;
            })
        };
        let watch = ctx.watch_for_result(
            &sid,
            provider,
            session.provider_session_id.as_deref(),
            cwd,
            out_path.as_path() as &Path,
            SWARM_RUN_TIMEOUT,
            ctx.pty_timings().waiting_idle,
            SWARM_STUCK_IDLE,
            Some(transcript_ok),
            on_status,
        );
        let outcome = tokio::select! {
            outcome = watch => outcome,
            _ = async {
                while !cancel.cancelled() { tokio::time::sleep(Duration::from_millis(100)).await; }
            } => {
                let _ = ctx.manager().kill_session(&sid).await;
                last_reason = Some(FailReason::Stopped);
                break;
            }
        };
        if cancel.cancelled() {
            let _ = ctx.manager().kill_session(&sid).await;
            last_reason = Some(FailReason::Stopped);
            break;
        }

        if let Some(raw) = outcome.raw {
            // Best-effort per-turn token/cost backfill so verify/fix spend counts
            // against the swarm budget (review M3). Bounded to this turn.
            let (toks_in, toks_out, cost) =
                crate::runtime::run::session_usage(ctx, Some(&sid), turn_started_at).await;
            let _ = ctx
                .swarm_repo()
                .update_run_if_status(
                    &run_id,
                    &["queued", "running", "waiting"],
                    RunPatch {
                        tokens_input: Some(toks_in),
                        tokens_output: Some(toks_out),
                        cost_usd: Some(cost),
                        ..Default::default()
                    },
                )
                .await;
            // Success: leave the session open for inspection.
            if set_run(ctx, &run_id, "done", None, true).await {
                return (Some(raw), run_id);
            }
            return (None, run_id);
        }
        last_reason = outcome.reason;
        // Kill the stuck/failed session before the next attempt.
        if let Some(s) = outcome.session_id {
            let _ = ctx.manager().kill_session(&s).await;
        }
    }

    let status = if matches!(last_reason, Some(FailReason::Stopped)) || cancel.cancelled() {
        "stopped"
    } else {
        "error"
    };
    cancel.kill_tracked(ctx).await;
    let err = last_reason.map(|r| r.as_str().to_string());
    let _ = set_run(ctx, &run_id, status, err, true).await;
    (None, run_id)
}

/// Patch a run's status (+ optional error/finished_at) and emit the update.
async fn set_run(
    ctx: &SwarmRt,
    run_id: &str,
    status: &str,
    error: Option<String>,
    finished: bool,
) -> bool {
    let patch = RunPatch {
        status: Some(status.to_string()),
        error: error.map(Some),
        started_at: if status == "running" && !finished {
            Some(Some(chrono::Utc::now()))
        } else {
            None
        },
        finished_at: if finished {
            Some(Some(chrono::Utc::now()))
        } else {
            None
        },
        ..Default::default()
    };
    let changed = matches!(
        ctx.swarm_repo()
            .update_run_if_status(
                &run_id.to_string(),
                &["queued", "running", "waiting"],
                patch
            )
            .await,
        Ok(Some(_))
    );
    if changed {
        emit_run(ctx, run_id).await;
    }
    changed
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn second_planner_cannot_replace_the_live_cancel_handle() {
        let first = begin("r09-same-kind", "plan").unwrap();
        assert!(begin("r09-same-kind", "plan").is_err());
        registry()
            .lock()
            .unwrap()
            .get(&key("r09-same-kind", "plan"))
            .unwrap()
            .signal();
        assert!(
            first.cancelled(),
            "Stop must still reach the admitted planner"
        );
        end("r09-same-kind", "plan");
    }

    /// S17-307: a plan and a recruit of the same swarm have separate handles
    /// — stopping one leaves the other running.
    #[test]
    fn plan_and_recruit_handles_are_independent() {
        let plan = begin("sw-reg-test", "plan").unwrap();
        let recruit = begin("sw-reg-test", "recruit").unwrap();
        {
            let reg = registry().lock().unwrap();
            reg.get(&key("sw-reg-test", "plan")).unwrap().signal();
        }
        assert!(plan.cancelled());
        assert!(!recruit.cancelled());
        end("sw-reg-test", "plan");
        assert!(registry()
            .lock()
            .unwrap()
            .contains_key(&key("sw-reg-test", "recruit")));
        end("sw-reg-test", "recruit");
    }
}

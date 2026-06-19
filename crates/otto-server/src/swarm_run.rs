//! One swarm agent turn: prepare the agent's cwd + context, spawn or RESUME its
//! session (resume = the token-efficiency win — no history re-feed), inject only
//! the new brief, watch for the result via the shared `agent_run` primitives
//! (out-file / claude transcript; 0 model tokens to read), parse the structured
//! output, and persist the run. Routing of the result (handoffs / reviews /
//! subtasks) is the Coordinator's job (`swarm_runtime`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, SessionStatus};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{RunPatch, SwarmAgent, SwarmProject, SwarmRun, SwarmTask};
use serde::Deserialize;
use serde_json::json;

use crate::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;
use serde::Serialize;

/// run_id → cancel flag. Mirrors `product_run::CancelRegistry`.
pub type CancelRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

pub fn new_cancel_registry() -> CancelRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn register_cancel(reg: &CancelRegistry, run_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    reg.lock().unwrap().insert(run_id.to_string(), Arc::clone(&flag));
    flag
}

pub fn signal_cancel(reg: &CancelRegistry, run_id: &str) {
    if let Some(flag) = reg.lock().unwrap().get(run_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

pub fn unregister_cancel(reg: &CancelRegistry, run_id: &str) {
    reg.lock().unwrap().remove(run_id);
}

const TURN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const WAITING_IDLE: Duration = Duration::from_secs(45);
const STUCK_IDLE: Duration = Duration::from_secs(180);
const MAX_ATTEMPTS: u32 = 2;
const RETRY_BACKOFF: Duration = Duration::from_secs(3);

// --- Structured turn result ------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnArtifact {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnHandoff {
    #[serde(default, alias = "to_role", alias = "to")]
    pub to_role: String,
    #[serde(default)]
    pub brief: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnReview {
    #[serde(default, alias = "of_artifact", alias = "of")]
    pub of: String,
    #[serde(default)]
    pub reviewer_role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnSubtask {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub assignee_role: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub depends_on_titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnConcern {
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwarmTurnResult {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub artifacts: Vec<TurnArtifact>,
    #[serde(default)]
    pub handoffs: Vec<TurnHandoff>,
    #[serde(default, alias = "reviews_requested")]
    pub reviews: Vec<TurnReview>,
    #[serde(default)]
    pub subtasks: Vec<TurnSubtask>,
    #[serde(default)]
    pub concerns: Vec<TurnConcern>,
}

fn out_path(run_id: &str) -> PathBuf {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir).join(format!("otto-swarm-{run_id}.json"))
}

/// Parse the agent's result file content into a `SwarmTurnResult` (tolerant of
/// fences / surrounding prose). `None` if no JSON object is present.
pub fn parse_turn_result(text: &str) -> Option<SwarmTurnResult> {
    let v = otto_swarm::recruiter::extract_json(text)?;
    serde_json::from_value(v).ok()
}

fn transcript_ok(t: &str) -> bool {
    otto_swarm::recruiter::extract_json(t).is_some()
}

// --- Prompt ----------------------------------------------------------------

/// Build the per-turn brief. The agent's full identity/skills/board-instructions
/// are already materialized into its cwd (CLAUDE.md / AGENTS.md), so this stays
/// focused on the immediate task + the output contract.
fn build_prompt(
    agent: &SwarmAgent,
    task: Option<&SwarmTask>,
    is_delegation: bool,
    directive: Option<&str>,
    board: &[String],
    out_file: &str,
) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "You are {} ({}). Continue your work for the swarm.\n\n",
        agent.name, agent.title
    ));
    if let Some(t) = task {
        p.push_str(&format!("TASK: {}\n{}\n\n", t.title, t.description));
    }
    if let Some(d) = directive {
        if !d.is_empty() {
            p.push_str(&format!("STANDING DIRECTIVE: {d}\n\n"));
        }
    }
    if !board.is_empty() {
        p.push_str("RECENT TEAM BOARD (for context):\n");
        for m in board {
            p.push_str(&format!("- {m}\n"));
        }
        p.push('\n');
    }
    if is_delegation {
        p.push_str(
            "You are a LEADER. Do NOT do the hands-on work yourself. Break this down into \
             concrete subtasks for your reports and return them in `subtasks` (give each an \
             `assignee_role` matching one of your reports' titles). Keep it lean.\n\n",
        );
    } else {
        p.push_str(
            "Do the work now. Use `./otto-post` to share progress, ideas, reviews and concerns \
             with the team as you go.\n\n",
        );
    }
    p.push_str(&format!(
        "When finished, write your result as a SINGLE JSON object to this absolute path \
         (overwrite it; write ONLY the JSON, no prose, no markdown fence):\n\n{out_file}\n\n\
         Schema:\n{}\n\nWriting that file is the last thing you do.",
        RESULT_SCHEMA
    ));
    p
}

const RESULT_SCHEMA: &str = r#"{
  "status": "done | blocked | needs_review | in_progress",
  "summary": "one or two sentences on what you did",
  "artifacts": [{"type": "file|pr|doc|url", "path": "abs path or null", "url": "url or null", "label": "short"}],
  "handoffs": [{"to_role": "a teammate's title", "brief": "what they should do"}],
  "reviews_requested": [{"of": "artifact label/path", "reviewer_role": "a teammate's title"}],
  "subtasks": [{"title": "...", "description": "...", "assignee_role": "title or null", "priority": "low|medium|high", "depends_on_titles": []}],
  "concerns": [{"severity": "low|medium|high", "text": "if the plan/timeline looks wrong"}]
}"#;

// --- The turn --------------------------------------------------------------

/// Find a reusable live/resumable session for this agent, or `None`.
async fn find_agent_session(ctx: &ServerCtx, ws: &Id, agent_id: &str) -> Option<Id> {
    let sessions = ctx.manager.list_by_workspace(ws).await.ok()?;
    sessions.into_iter().find_map(|s| {
        let is_agent = s.kind == SessionKind::Agent && !s.archived;
        let mine = s.meta.get("agent_id").and_then(|v| v.as_str()) == Some(agent_id);
        let alive = !matches!(s.status, SessionStatus::Exited);
        (is_agent && mine && alive).then_some(s.id)
    })
}

/// Run one turn for `run`. Updates the run row (running → done/error/stopped),
/// persists the parsed result, emits events. Returns the parsed result on
/// success so the Coordinator can route handoffs/subtasks/reviews.
pub async fn run_turn(ctx: ServerCtx, run: SwarmRun) -> Option<SwarmTurnResult> {
    let cancel = register_cancel(&ctx.swarm_run_cancels, &run.id);
    let res = run_turn_inner(&ctx, &run, &cancel).await;
    unregister_cancel(&ctx.swarm_run_cancels, &run.id);
    res
}

async fn run_turn_inner(
    ctx: &ServerCtx,
    run: &SwarmRun,
    cancel: &Arc<AtomicBool>,
) -> Option<SwarmTurnResult> {
    let repo = &ctx.swarm_repo;
    let swarm = repo.get_swarm(&run.swarm_id).await.ok()?;
    let agent = repo.get_agent(&run.agent_id).await.ok()?;
    let task: Option<SwarmTask> = match &run.task_id {
        Some(tid) => repo.get_task(tid).await.ok(),
        None => None,
    };
    let project: Option<SwarmProject> = match task.as_ref().map(|t| t.project_id.clone()).or_else(|| run.project_id.clone()) {
        Some(pid) => repo.get_project(&pid).await.ok(),
        None => None,
    };
    let ws = swarm.workspace_id.clone();

    // Prepare cwd + per-agent context (skills/soul/identity) + otto-post helper.
    let cwd = match crate::swarm_workspace::ensure_cwd(ctx, &swarm, &agent, project.as_ref()).await {
        Ok(c) => c,
        Err(e) => {
            mark_run_error(ctx, run, &format!("prepare cwd: {e}")).await;
            return None;
        }
    };
    let manager_title = match &agent.reports_to {
        Some(mid) => repo.get_agent(mid).await.ok().map(|m| m.title),
        None => None,
    };
    let reports: Vec<String> = repo
        .list_agents(&swarm.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.reports_to.as_deref() == Some(agent.id.as_str()))
        .map(|a| a.title)
        .collect();
    let identity = crate::swarm_workspace::render_identity(
        &swarm,
        &agent,
        manager_title.as_deref(),
        &reports,
        project.as_ref(),
        task.as_ref(),
    );
    crate::swarm_workspace::provision_agent(ctx, &agent, identity, &cwd);

    // Board context for the brief (recent messages to/about this agent).
    let board: Vec<String> = repo
        .board_for_agent(&swarm.id, &agent.id, 12)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
        .map(|m| {
            let who = m.author_agent_id.clone().unwrap_or_else(|| "user".into());
            format!("[{}] {}: {}", m.kind, who, truncate(&m.body, 200))
        })
        .collect();

    let is_delegation = run.kind == "planning" || (!reports.is_empty() && run.kind == "task");
    let directive = run
        .result
        .as_ref()
        .and_then(|v| v.get("directive").and_then(|d| d.as_str()).map(str::to_string))
        .or_else(|| {
            agent
                .schedule
                .as_ref()
                .and_then(|s| s.get("directive").and_then(|d| d.as_str()).map(str::to_string))
        });

    let out = out_path(&run.id);
    let _ = std::fs::remove_file(&out);
    let prompt = build_prompt(
        &agent,
        task.as_ref(),
        is_delegation,
        directive.as_deref(),
        &board,
        &out.to_string_lossy(),
    );

    // Mark running.
    let _ = repo
        .update_run(
            &run.id,
            RunPatch {
                status: Some("running".into()),
                started_at: Some(Some(Utc::now())),
                ..Default::default()
            },
        )
        .await;
    emit_run(ctx, &run.id).await;

    // Spawn/resume + inject + watch, with bounded recovery.
    let provider = agent.provider.clone();
    let outcome = run_with_recovery(
        &ctx.manager,
        MAX_ATTEMPTS,
        &[RETRY_BACKOFF],
        Some(cancel),
        |_attempt| {
            let ctx = ctx.clone();
            let ws = ws.clone();
            let swarm_meta = json!({
                "source": "swarm",
                "swarm_id": swarm.id,
                "agent_id": agent.id,
                "project_id": project.as_ref().map(|p| p.id.clone()),
                "task_id": task.as_ref().map(|t| t.id.clone()),
                "run_id": run.id,
            });
            let provider = provider.clone();
            let cwd = cwd.clone();
            let prompt = prompt.clone();
            let out = out.clone();
            let run_id = run.id.clone();
            let agent_id = agent.id.clone();
            let title = format!("{} · {}", agent.name, task.as_ref().map(|t| t.title.clone()).unwrap_or_else(|| agent.title.clone()));
            async move {
                run_attempt(&ctx, &ws, &agent_id, &provider, &cwd, &title, &swarm_meta, &prompt, &out, &run_id).await
            }
        },
    )
    .await;

    // Best-effort token/cost backfill for this turn, keyed on the run's session.
    // otto-usage records per-session; the run is tagged with `session_id`/`run_id`
    // (swarm_meta above), so the session totals are the turn's usage. Stays null
    // when usage tracking is off or the latest events haven't been flushed yet.
    let (toks_in, toks_out, cost) = session_usage(ctx, outcome.session_id.as_deref()).await;

    // Persist terminal state.
    if let Some(raw) = outcome.raw.as_deref() {
        let parsed = parse_turn_result(raw);
        let status = parsed.as_ref().map(|r| r.status.clone()).unwrap_or_else(|| "done".into());
        let summary = parsed.as_ref().map(|r| r.summary.clone()).unwrap_or_default();
        // Persist the parsed result plus the turn's `cwd`/`brief` so the Run
        // Inspector can show what was sent and where it ran without a new route.
        let result = enrich_result(parsed.as_ref().map(|r| serde_json::to_value(r).unwrap_or_default()), &cwd, &prompt);
        let _ = repo
            .update_run(
                &run.id,
                RunPatch {
                    status: Some("done".into()),
                    session_id: Some(outcome.session_id.clone()),
                    summary: Some(Some(if summary.is_empty() { format!("turn {status}") } else { summary })),
                    result: Some(Some(result)),
                    tokens_input: Some(toks_in),
                    tokens_output: Some(toks_out),
                    cost_usd: Some(cost),
                    finished_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await;
        emit_run(ctx, &run.id).await;
        parsed
    } else {
        let reason = outcome.reason.map(|r| r.as_str()).unwrap_or("error");
        let stopped = matches!(outcome.reason, Some(FailReason::Stopped));
        // Even on failure, keep the brief/cwd for inspection.
        let result = enrich_result(None, &cwd, &prompt);
        let _ = repo
            .update_run(
                &run.id,
                RunPatch {
                    status: Some(if stopped { "stopped".into() } else { "error".into() }),
                    session_id: Some(outcome.session_id.clone()),
                    error: Some(Some(reason.to_string())),
                    result: Some(Some(result)),
                    // The agent may have spent tokens before failing/stopping.
                    tokens_input: Some(toks_in),
                    tokens_output: Some(toks_out),
                    cost_usd: Some(cost),
                    finished_at: Some(Some(Utc::now())),
                    ..Default::default()
                },
            )
            .await;
        emit_run(ctx, &run.id).await;
        None
    }
}

/// Fold the turn's `cwd` + `brief` into the stored run `result` object so the
/// Run Inspector can surface them. The parsed turn JSON (if any) keeps its own
/// keys; `cwd`/`brief` are added only when not already present.
fn enrich_result(parsed: Option<serde_json::Value>, cwd: &str, brief: &str) -> serde_json::Value {
    let mut obj = match parsed {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    obj.entry("cwd").or_insert_with(|| json!(cwd));
    obj.entry("brief").or_insert_with(|| json!(brief));
    serde_json::Value::Object(obj)
}

/// Pull this turn's token/cost totals from otto-usage for `session_id`.
/// Returns `(input, output, cost_usd)`, each `None` when usage tracking is
/// unavailable, no session was created, or no events were recorded yet — the
/// `RunPatch` then writes nulls rather than misleading zeros.
async fn session_usage(
    ctx: &ServerCtx,
    session_id: Option<&str>,
) -> (Option<i64>, Option<i64>, Option<f64>) {
    let Some(sid) = session_id else { return (None, None, None) };
    match ctx.usage.session_totals_for(sid).await {
        Some(t) => (
            Some(t.input_tokens as i64),
            Some(t.output_tokens as i64),
            Some(t.cost_usd),
        ),
        None => (None, None, None),
    }
}

/// One attempt: find-or-create the agent session, inject the brief, watch.
#[allow(clippy::too_many_arguments)]
async fn run_attempt(
    ctx: &ServerCtx,
    ws: &Id,
    agent_id: &str,
    provider: &str,
    cwd: &str,
    title: &str,
    meta: &serde_json::Value,
    prompt: &str,
    out: &std::path::Path,
    run_id: &str,
) -> RunOutcome {
    // Reuse the agent's live/resumable session (no history re-feed) or create one.
    let sid = match find_agent_session(ctx, ws, agent_id).await {
        Some(existing) => {
            let _ = ctx.manager.ensure_live(&existing).await;
            existing
        }
        None => {
            let req = CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some(provider.to_string()),
                title: Some(title.to_string()),
                cwd: Some(cwd.to_string()),
                connection_id: None,
                meta: Some(meta.clone()),
            };
            let ws_row = match ctx.workspaces.get(ws).await {
                Ok(w) => w,
                Err(_) => return RunOutcome::failed(None, FailReason::CreateFailed),
            };
            // A system user id for swarm-created sessions: reuse the swarm's creator.
            let swarm_id = meta
                .get("swarm_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let creator = ctx
                .swarm_repo
                .get_swarm(&swarm_id)
                .await
                .map(|s| s.created_by)
                .unwrap_or_else(|_| "system".into());
            match ctx.manager.create(&ws_row, &creator, req, None).await {
                Ok(s) => s.id,
                Err(e) => {
                    tracing::warn!("swarm: create session ({provider}): {e}");
                    return RunOutcome::failed(None, FailReason::CreateFailed);
                }
            }
        }
    };

    // Persist session_id on the run immediately so the UI can Open it live.
    let _ = ctx
        .swarm_repo
        .update_run(&run_id.to_string(), RunPatch { session_id: Some(Some(sid.clone())), ..Default::default() })
        .await;
    emit_run(ctx, run_id).await;

    // Re-provision into the (possibly reused) cwd is already done by the caller.
    // Inject the brief once the TUI has drawn + settled.
    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }

    let provider_session_id = ctx
        .manager
        .get(&sid)
        .await
        .ok()
        .and_then(|s| s.provider_session_id);

    watch_for_result(
        &ctx.manager,
        &sid,
        provider,
        provider_session_id.as_deref(),
        cwd,
        out,
        TURN_TIMEOUT,
        WAITING_IDLE,
        STUCK_IDLE,
        transcript_ok,
        |st| async move {
            // Reflect waiting/resumed onto the run row.
            let _ = st;
        },
    )
    .await
}

async fn mark_run_error(ctx: &ServerCtx, run: &SwarmRun, msg: &str) {
    let _ = ctx
        .swarm_repo
        .update_run(
            &run.id,
            RunPatch {
                status: Some("error".into()),
                error: Some(Some(msg.to_string())),
                finished_at: Some(Some(Utc::now())),
                ..Default::default()
            },
        )
        .await;
    emit_run(ctx, &run.id).await;
}

/// Re-read a run and broadcast `SwarmRunUpdated`.
pub async fn emit_run(ctx: &ServerCtx, run_id: &str) {
    if let Ok(run) = ctx.swarm_repo.get_run(&run_id.to_string()).await {
        let _ = ctx.events.send(Event::SwarmRunUpdated {
            workspace_id: run.workspace_id.clone(),
            swarm_id: run.swarm_id.clone(),
            run: serde_json::to_value(&run).unwrap_or_default(),
        });
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

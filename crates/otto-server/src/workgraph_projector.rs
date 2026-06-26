//! WorkGraphProjector — materializes Mission Control's work graph from the live
//! daemon event bus plus a periodic reconcile/backfill sweep.
//!
//! Design (see `docs/design/workgraph-mission-control-design.md` §5 + the review
//! resolutions): every module already broadcasts an `Event::*`, so we do NOT
//! rewire them — a single subscriber projects those events into work items.
//!
//! Two tasks, deliberately split (review finding M5):
//!   * **event loop** — cheap, SQLite-only upserts on each source event, for a
//!     snappy live UI. NEVER calls the usage engine (a `clickhouse local`
//!     process spawn) inline, so it can't back the broadcast buffer up into
//!     `Lagged`.
//!   * **reconcile (60 s) + boot backfill** — enumerate the authoritative repos
//!     and re-derive every item (idempotent, self-healing for missed/lagged
//!     events) AND refresh per-session cost via the usage engine, off the hot
//!     path. The boot backfill is `tokio::spawn`ed so it never delays startup.

use std::time::Duration;

use otto_core::domain::{Session, SessionKind};
use otto_core::event::Event;
use otto_core::Id;
use serde::Serialize;
use serde_json::json;
use tokio::sync::broadcast;

use otto_state::{
    ArtifactKind, EdgeRelation, MissionFilter, NewArtifact, NewWorkEvent, WorkActor,
    WorkItemUpsert, WorkKind, WorkStatus, WorkflowsRepo,
};
use otto_workgraph::risk;

use crate::state::ServerCtx;

// The reconcile is a SAFETY NET for missed/lagged live events, not the primary
// path — items are materialized live by the event loop. Re-deriving every
// workspace is real work, so run it sparingly (5 min) to avoid recurring write
// load competing with the rest of the daemon.
const RECONCILE_SECS: u64 = 300;
/// Per-workflow run cap when backfilling (a workflow can have many historical
/// runs; the recent ones are what Mission Control cares about).
const RUNS_PER_WORKFLOW: usize = 5;

/// Serialize a no-data enum to its string form (lowercased) for normalization.
fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// How a live PTY session maps into the graph.
enum SessionClass {
    /// Internal/sub-agent PTY (review/swarm/loop/assistant) — represented by its
    /// parent work item; not surfaced on its own.
    Skip,
    /// A normal user-driven agent session → kind `session`.
    Normal,
    /// A channel-spawned task (Slack/Telegram/webhook) → kind `external_trigger`.
    Channel(String),
}

fn classify(session: &Session) -> SessionClass {
    if session.kind != SessionKind::Agent || session.archived {
        return SessionClass::Skip;
    }
    let src = session
        .meta
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match src {
        "channel" => {
            let ch = session
                .meta
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("channel")
                .to_string();
            SessionClass::Channel(ch)
        }
        "" | "user" => SessionClass::Normal,
        // review / swarm / goal_loop / skilleval / *_assist / product-analysis …
        _ => SessionClass::Skip,
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Start the projector: the live event loop, the reconcile interval, and a
/// one-shot boot backfill (all detached). Returns the event-loop join handle
/// (the caller drops it; the task lives for the process).
pub fn spawn(ctx: ServerCtx) -> tokio::task::JoinHandle<()> {
    // Boot backfill — off the startup path.
    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            backfill_all(&ctx).await;
            tracing::info!("workgraph: boot backfill complete");
        });
    }
    // Reconcile loop.
    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(RECONCILE_SECS));
            tick.tick().await; // consume the immediate first tick
            loop {
                tick.tick().await;
                backfill_all(&ctx).await;
            }
        });
    }
    // Live event loop.
    tokio::spawn(async move {
        let mut rx = ctx.events.subscribe();
        loop {
            match rx.recv().await {
                Ok(ev) => handle_event(&ctx, ev).await,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("workgraph projector lagged {n} events; reconcile will heal");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Live event handling (cheap; SQLite only — NO usage/ClickHouse calls)
// ---------------------------------------------------------------------------

async fn handle_event(ctx: &ServerCtx, ev: Event) {
    match ev {
        Event::SessionCreated { session } => upsert_session(ctx, &session).await,
        Event::SessionStatus {
            session_id,
            workspace_id,
            status,
        } => {
            let _ = ctx
                .workgraph
                .set_status_by_session(&workspace_id, &session_id, &enum_str(&status))
                .await;
        }
        Event::SessionRemoved {
            session_id,
            workspace_id,
        } => {
            let _ = ctx
                .workgraph
                .set_status_by_session(&workspace_id, &session_id, "exited")
                .await;
        }
        Event::TrailAppended {
            workspace_id,
            session_id,
            event,
        } => ingest_trail(ctx, &workspace_id, &session_id, &event).await,
        Event::SwarmRunUpdated { run, .. } => {
            if let Some(pid) = run.get("project_id").and_then(|v| v.as_str()) {
                upsert_swarm_project(ctx, &pid.to_string()).await;
            }
        }
        Event::SwarmTaskUpdated { project_id, .. } => {
            upsert_swarm_project(ctx, &project_id).await;
        }
        Event::SwarmStatus { swarm_id, .. } => {
            if let Ok(projects) = ctx.swarm_repo.list_projects(&swarm_id).await {
                for p in projects {
                    upsert_swarm_project(ctx, &p.id).await;
                }
            }
        }
        Event::GoalLoopUpdated { loop_id, .. } => upsert_goal_loop(ctx, &loop_id).await,
        Event::WorkflowRunUpdated { run_id, .. } => upsert_workflow_run(ctx, &run_id).await,
        Event::ReviewChanged {
            workspace_id,
            review_id,
            ..
        } => upsert_review(ctx, &workspace_id, &review_id).await,
        Event::ProductChanged { story_id, .. } | Event::PlanRun { story_id, .. } => {
            upsert_product_story(ctx, &story_id).await
        }
        // Everything else (incl. our own WorkGraphUpdated) is not a source signal.
        _ => {}
    }
}

/// Ingest a notable trail entry (a tool/command/skill call) as a work event on
/// the owning session item. Quiet (no broadcast) — high volume.
async fn ingest_trail(
    ctx: &ServerCtx,
    workspace_id: &Id,
    session_id: &Id,
    event: &otto_core::domain::TrailEvent,
) {
    let kind = enum_str(&event.kind);
    if !matches!(kind.as_str(), "tool" | "command" | "skill") {
        return;
    }
    let item = match ctx.workgraph.repo().find_session_item(workspace_id, session_id).await {
        Ok(Some(it)) => it,
        _ => return,
    };
    let _ = ctx
        .workgraph
        .ingest_event(NewWorkEvent {
            work_item_id: item.id,
            workspace_id: workspace_id.clone(),
            actor: WorkActor::Agent,
            event_type: "tool_call".into(),
            payload: json!({ "kind": kind, "summary": event.summary }),
        })
        .await;
}

// ---------------------------------------------------------------------------
// Per-kind upserts (used by both the event loop and the backfill)
// ---------------------------------------------------------------------------

async fn upsert_session(ctx: &ServerCtx, session: &Session) {
    let (kind, owner, owner_kind) = match classify(session) {
        SessionClass::Skip => return,
        SessionClass::Normal => (
            WorkKind::Session,
            Some(session.created_by.clone()),
            WorkActor::User,
        ),
        SessionClass::Channel(ch) => (WorkKind::ExternalTrigger, Some(ch), WorkActor::Integration),
    };
    let title = if session.title.trim().is_empty() {
        format!("{} session", session.provider)
    } else {
        session.title.clone()
    };
    let status = WorkStatus::from_source(kind, &enum_str(&session.status));
    let ctx_summary = format!("{} session · {}", session.provider, session.cwd);
    let up = WorkItemUpsert {
        workspace_id: session.workspace_id.clone(),
        kind,
        source_id: session.id.clone(),
        title: title.clone(),
        goal: None,
        status,
        owner,
        owner_kind,
        repo_id: Some(session.cwd.clone()),
        branch: None,
        cost_so_far: None,
        risk_level: risk(kind, &title),
        result_summary: None,
        context_summary: Some(ctx_summary),
        started_by_id: None,
    };
    if let Err(e) = ctx.workgraph.record(up).await {
        tracing::debug!("workgraph upsert_session: {e}");
    } else {
        // A session links to itself in the UI via a deep-link artifact (evidence).
        let _ = ctx
            .workgraph
            .add_artifact(NewArtifact {
                work_item_id: session.id.clone(),
                workspace_id: session.workspace_id.clone(),
                kind: ArtifactKind::Session,
                title: "Open session".into(),
                reference: Some(session.id.clone()),
                payload: json!({ "session_id": session.id }),
            })
            .await;
    }
}

async fn upsert_swarm_project(ctx: &ServerCtx, project_id: &Id) {
    let project = match ctx.swarm_repo.get_project(project_id).await {
        Ok(p) => p,
        Err(_) => return,
    };
    // Project-level cost = sum of its runs' costs.
    let cost: Option<f64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_usd), 0.0) FROM swarm_runs WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_one(&ctx.pool)
    .await
    .ok();
    let status = WorkStatus::from_source(WorkKind::Swarm, &project.status);
    let up = WorkItemUpsert {
        workspace_id: project.workspace_id.clone(),
        kind: WorkKind::Swarm,
        source_id: project.id.clone(),
        title: project.name.clone(),
        goal: project.goal_md.clone(),
        status,
        owner: Some(project.created_by.clone()),
        owner_kind: WorkActor::Agent,
        repo_id: project.repo_path.clone(),
        branch: None,
        cost_so_far: cost,
        risk_level: risk(WorkKind::Swarm, &project.name),
        result_summary: None,
        context_summary: Some(format!(
            "Swarm project · {}",
            if project.description.is_empty() {
                "no description"
            } else {
                &project.description
            }
        )),
        started_by_id: None,
    };
    if let Err(e) = ctx.workgraph.record(up).await {
        tracing::debug!("workgraph upsert_swarm_project: {e}");
    }
}

async fn upsert_goal_loop(ctx: &ServerCtx, loop_id: &Id) {
    let gl = match ctx.goal_loops_repo.get(loop_id).await {
        Ok(g) => g,
        Err(_) => return,
    };
    let status = WorkStatus::from_source(WorkKind::GoalLoop, &enum_str(&gl.status));
    let up = WorkItemUpsert {
        workspace_id: gl.workspace_id.clone(),
        kind: WorkKind::GoalLoop,
        source_id: gl.id.clone(),
        title: gl.name.clone(),
        goal: Some(gl.name.clone()),
        status,
        owner: Some(gl.created_by.clone()),
        owner_kind: WorkActor::User,
        repo_id: Some(gl.repo_path.clone()),
        branch: gl.branch.clone(),
        cost_so_far: Some(gl.cost_usd),
        risk_level: risk(WorkKind::GoalLoop, &gl.name),
        result_summary: gl.summary.clone(),
        context_summary: Some(format!(
            "Goal loop · phase {}, iteration {} ({}%)",
            enum_str(&gl.phase),
            gl.current_iteration,
            gl.progress_pct
        )),
        started_by_id: None,
    };
    if ctx.workgraph.record(up).await.is_ok() {
        if let Some(branch) = &gl.branch {
            let _ = ctx
                .workgraph
                .add_artifact(NewArtifact {
                    work_item_id: gl.id.clone(),
                    workspace_id: gl.workspace_id.clone(),
                    kind: ArtifactKind::Link,
                    title: format!("Branch {branch}"),
                    reference: gl.worktree_path.clone(),
                    payload: json!({ "branch": branch, "worktree": gl.worktree_path }),
                })
                .await;
        }
    }
}

async fn upsert_workflow_run(ctx: &ServerCtx, run_id: &Id) {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let run = match repo.get_run(run_id).await {
        Ok(r) => r,
        Err(_) => return,
    };
    let wf_name = repo
        .get(&run.workflow_id)
        .await
        .map(|w| w.name)
        .unwrap_or_else(|_| "Workflow".into());
    let status = WorkStatus::from_source(WorkKind::Workflow, run.status.as_str());
    let title = format!("{wf_name} run");
    let up = WorkItemUpsert {
        workspace_id: run.workspace_id.clone(),
        kind: WorkKind::Workflow,
        source_id: run.id.clone(),
        title: title.clone(),
        goal: Some(wf_name.clone()),
        status,
        owner: None,
        owner_kind: WorkActor::System,
        repo_id: None,
        branch: None,
        // Workflow runs carry no cost field; surfaced as 0 (documented).
        cost_so_far: Some(0.0),
        risk_level: risk(WorkKind::Workflow, &title),
        result_summary: run.error.clone(),
        context_summary: Some(format!("Workflow run · {} nodes", run.nodes.len())),
        started_by_id: None,
    };
    if let Err(e) = ctx.workgraph.record(up).await {
        tracing::debug!("workgraph upsert_workflow_run: {e}");
    }
}

async fn upsert_review(ctx: &ServerCtx, workspace_id: &Id, review_id: &Id) {
    let review = match ctx.reviews_store.get_review(review_id).await {
        Ok(r) => r,
        Err(_) => return,
    };
    let repo_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM repos WHERE id = ?")
            .bind(&review.repo_id)
            .fetch_optional(&ctx.pool)
            .await
            .ok()
            .flatten();
    let repo_label = repo_name.clone().unwrap_or_else(|| review.repo_id.clone());
    let status = WorkStatus::from_source(WorkKind::Review, &enum_str(&review.status));
    let title = format!("Review · {} PR #{}", repo_label, review.pr_number);

    // The review work item.
    let up = WorkItemUpsert {
        workspace_id: workspace_id.clone(),
        kind: WorkKind::Review,
        source_id: review.id.clone(),
        title: title.clone(),
        goal: Some(format!("Review PR #{}", review.pr_number)),
        status,
        owner: None,
        owner_kind: WorkActor::Agent,
        repo_id: Some(review.repo_id.clone()),
        branch: None,
        cost_so_far: None,
        risk_level: risk(WorkKind::Review, &title),
        result_summary: review.summary_md.clone(),
        context_summary: Some(format!(
            "Code review of PR #{} in {}",
            review.pr_number, repo_label
        )),
        started_by_id: None,
    };
    if ctx.workgraph.record(up).await.is_err() {
        return;
    }

    // A `pr` work item the review reviews (R2.7), keyed `repo:pr`.
    let pr_source = format!("{}:{}", review.repo_id, review.pr_number);
    let pr_up = WorkItemUpsert {
        workspace_id: workspace_id.clone(),
        kind: WorkKind::Pr,
        source_id: pr_source.clone(),
        title: format!("PR #{} · {}", review.pr_number, repo_label),
        goal: None,
        status: WorkStatus::Running,
        owner: None,
        owner_kind: WorkActor::System,
        repo_id: Some(review.repo_id.clone()),
        branch: None,
        cost_so_far: Some(0.0),
        risk_level: risk(WorkKind::Pr, &repo_label),
        result_summary: None,
        context_summary: Some(format!("Pull request #{} in {}", review.pr_number, repo_label)),
        started_by_id: None,
    };
    let pr_item = match ctx.workgraph.record(pr_up).await {
        Ok(it) => it,
        Err(_) => return,
    };

    // Edge: review --reviews--> pr.
    let _ = ctx
        .workgraph
        .add_edge(workspace_id, &review.id, &pr_item.id, EdgeRelation::Reviews)
        .await;

    // Evidence: a review report artifact + the PR link artifact.
    if review.summary_md.is_some() || review.verdict.is_some() {
        let _ = ctx
            .workgraph
            .add_artifact(NewArtifact {
                work_item_id: review.id.clone(),
                workspace_id: workspace_id.clone(),
                kind: ArtifactKind::Report,
                title: "Review verdict".into(),
                reference: None,
                payload: json!({
                    "verdict": review.verdict,
                    "blocker_count": review.blocker_count,
                    "summary_md": review.summary_md,
                }),
            })
            .await;
    }
    let _ = ctx
        .workgraph
        .add_artifact(NewArtifact {
            work_item_id: pr_item.id.clone(),
            workspace_id: workspace_id.clone(),
            kind: ArtifactKind::Pr,
            title: format!("PR #{}", review.pr_number),
            reference: None,
            payload: json!({ "repo_id": review.repo_id, "pr_number": review.pr_number }),
        })
        .await;
}

async fn upsert_product_story(ctx: &ServerCtx, story_id: &Id) {
    let story = match ctx.product_repo.get_story(story_id).await {
        Ok(s) => s,
        Err(_) => return,
    };
    upsert_product_story_row(ctx, &story).await;
}

async fn upsert_product_story_row(ctx: &ServerCtx, story: &otto_state::ProductStory) {
    let status = WorkStatus::from_source(WorkKind::ProductStory, &story.stage);
    let up = WorkItemUpsert {
        workspace_id: story.workspace_id.clone(),
        kind: WorkKind::ProductStory,
        source_id: story.id.clone(),
        title: story.title.clone(),
        goal: None,
        status,
        owner: Some(story.created_by.clone()),
        owner_kind: WorkActor::User,
        repo_id: story.cwd.clone(),
        branch: None,
        // Stories aggregate cost via their plan/analysis sessions (not modeled
        // for MVP); surfaced as 0.
        cost_so_far: Some(0.0),
        risk_level: risk(WorkKind::ProductStory, &story.title),
        result_summary: None,
        context_summary: Some(format!("{} story · stage {}", story.source_kind, story.stage)),
        started_by_id: None,
    };
    if let Err(e) = ctx.workgraph.record(up).await {
        tracing::debug!("workgraph upsert_product_story: {e}");
    }
}

// ---------------------------------------------------------------------------
// Backfill / reconcile — enumerate authoritative repos + refresh session cost
// ---------------------------------------------------------------------------

/// Re-derive the whole graph for every workspace. Idempotent; heals
/// missed/lagged events. SQLite-only (NO usage/ClickHouse) so it is safe to run
/// every 60 s on the reconcile loop without contending with the usage engine's
/// `clickhouse local` queries — cost is refreshed on-demand (item detail) and on
/// the user-triggered backfill, not on a background sweep.
pub async fn backfill_all(ctx: &ServerCtx) {
    let workspaces = match ctx.workspaces.list_all().await {
        Ok(w) => w,
        Err(e) => {
            tracing::debug!("workgraph backfill: list workspaces: {e}");
            return;
        }
    };
    let all_stories = ctx.product_repo.list_stories().await.unwrap_or_default();
    for ws in &workspaces {
        backfill_workspace(ctx, &ws.id, &all_stories).await;
    }
}

/// Re-derive the graph for a SINGLE workspace (the user-triggered "Refresh" path,
/// so a backfill from one workspace doesn't enumerate every other workspace's
/// sources). SQLite-only. Best-effort.
pub async fn backfill_one(ctx: &ServerCtx, workspace_id: &Id) {
    let all_stories = ctx.product_repo.list_stories().await.unwrap_or_default();
    backfill_workspace(ctx, workspace_id, &all_stories).await;
}

/// Refresh the cost snapshot of a single session/external_trigger item via the
/// usage engine. ON-DEMAND only (called from the item-detail route + the
/// user-triggered backfill) — never on a background loop, so a per-call
/// `clickhouse local` spawn can't storm the usage engine. No-op for other kinds.
pub async fn refresh_item_cost(ctx: &ServerCtx, workspace_id: &Id, item: &otto_state::WorkItem) {
    if !matches!(item.kind, WorkKind::Session | WorkKind::ExternalTrigger) {
        return;
    }
    if let Some(tot) = ctx.usage.session_totals_for(&item.source_id, None).await {
        if tot.cost_usd > 0.0 && (tot.cost_usd - item.cost_so_far).abs() > f64::EPSILON {
            let _ = ctx.workgraph.set_cost(workspace_id, &item.id, tot.cost_usd).await;
        }
    }
}

async fn backfill_workspace(
    ctx: &ServerCtx,
    ws_id: &Id,
    all_stories: &[otto_state::ProductStory],
) {
    // Sessions (+ external triggers).
    if let Ok(sessions) = ctx.manager.list_by_workspace(ws_id).await {
        for s in &sessions {
            upsert_session(ctx, s).await;
        }
    }
    // Swarm projects.
    if let Ok(swarms) = ctx.swarm_repo.list_swarms(ws_id).await {
        for sw in &swarms {
            if let Ok(projects) = ctx.swarm_repo.list_projects(&sw.id).await {
                for p in &projects {
                    upsert_swarm_project(ctx, &p.id).await;
                }
            }
        }
    }
    // Goal loops.
    if let Ok(loops) = ctx.goal_loops_repo.list_by_workspace(ws_id).await {
        for gl in &loops {
            upsert_goal_loop(ctx, &gl.id).await;
        }
    }
    // Workflow runs (recent per workflow).
    let wf_repo = WorkflowsRepo::new(ctx.pool.clone());
    if let Ok(workflows) = wf_repo.list(ws_id).await {
        for wf in &workflows {
            if let Ok(runs) = wf_repo.list_runs(&wf.id).await {
                for run in runs.iter().take(RUNS_PER_WORKFLOW) {
                    upsert_workflow_run(ctx, &run.id).await;
                }
            }
        }
    }
    // Reviews (+ derived PRs).
    let review_ids: Vec<String> = sqlx::query_scalar(
        "SELECT r.id FROM pr_reviews r JOIN repos gr ON gr.id = r.repo_id \
         WHERE gr.workspace_id = ? ORDER BY r.created_at DESC LIMIT 50",
    )
    .bind(ws_id)
    .fetch_all(&ctx.pool)
    .await
    .unwrap_or_default();
    for rid in &review_ids {
        upsert_review(ctx, ws_id, rid).await;
    }
    // Product stories belonging to this workspace.
    for story in all_stories.iter().filter(|s| &s.workspace_id == ws_id) {
        upsert_product_story_row(ctx, story).await;
    }
}

/// Refresh the cost snapshot for the session/external_trigger items of ONE
/// workspace via the usage engine. USER-TRIGGERED only (the backfill / "Refresh"
/// button) and bounded to a single workspace, so its `clickhouse local` spawns
/// stay a brief, deliberate burst — never a background or cross-workspace storm
/// (which would contend with the usage engine and stall the Usage page).
pub async fn refresh_session_costs(ctx: &ServerCtx, workspace_id: &Id) {
    let filter = MissionFilter {
        limit: Some(500),
        ..Default::default()
    };
    let items = ctx
        .workgraph
        .repo()
        .list_items(workspace_id, &filter)
        .await
        .unwrap_or_default();
    for it in &items {
        refresh_item_cost(ctx, workspace_id, it).await;
    }
}

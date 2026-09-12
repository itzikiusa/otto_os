//! Mission Control — work-queue surface (B4).
//!
//! Routes:
//!   GET  /workspaces/{id}/mission               → MissionView (6-bucket work queue)
//!   GET  /workspaces/{id}/mission/views          → Vec<SavedView>
//!   POST /workspaces/{id}/mission/views          → SavedView (create)
//!   DELETE /mission-views/{id}                   → 204 (delete)
//!
//! The view is assembled READ-ONLY from existing stores; it never mutates state.
//! Responses are cached for a few seconds to keep repeated refreshes cheap.
//!
//! ## Bucket assembly sources
//!
//! | Bucket        | Source(s)                                                   |
//! |---------------|-------------------------------------------------------------|
//! | needs_you     | `ws.needsYou` flags on active sessions (session manager)     |
//! | working       | Active sessions whose status is Running/Working              |
//! | review_ready  | PR reviews in status "running" (agents still running)        |
//! | waiting       | Active sessions whose status is Idle (could be blocked)      |
//! | failed        | Workflow runs in status "error"; swarm runs status "error"   |
//! | budget_warn   | Budget rows with `warning == true` from the usage engine     |

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::Utc;
use otto_core::domain::{SessionKind, SessionStatus, WorkspaceRole};
use otto_core::Id;
use otto_state::{NewSavedView, SavedView, SavedViewsRepo, WorkflowsRepo};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;
use tokio::sync::Mutex;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// DTOs (module-local, intentionally not promoted to otto-core::api)
// ---------------------------------------------------------------------------

/// One work-queue item in any bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionItem {
    /// "session" | "review" | "workflow_run" | "swarm_run" | "product_analysis" | "budget"
    pub kind: String,
    /// The primary entity id (session id, review id, workflow_run id, …).
    pub id: Id,
    /// Short human title for the row (session title, PR "#42", workflow name, …).
    pub title: String,
    /// Fine-grained status string native to the source (e.g. "running", "error", "exceeded").
    pub status: String,
    /// Attached session id when the work is driven by a live PTY session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Id>,
    /// Workspace-relative git repo name when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Last-known USD cost of the run/session, when the usage engine tracks it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Seconds since the item was last active / created.
    pub age_secs: i64,
}

/// The 6-bucket Mission Control view for one workspace.
///
/// All six lists are assembled in a single pass over the per-workspace data;
/// the entire response is cached for a short TTL so rapid refreshes are cheap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissionView {
    /// Sessions the user needs to act on (needs-you notices fired, not yet cleared).
    pub needs_you: Vec<MissionItem>,
    /// Sessions actively generating output (status Working/Running).
    pub working: Vec<MissionItem>,
    /// PR reviews whose agents are still running ("review_ready" = review is in
    /// progress and likely has something for the user soon).
    pub review_ready: Vec<MissionItem>,
    /// Sessions idle for a while — possibly waiting on input.
    pub waiting: Vec<MissionItem>,
    /// Workflow runs or swarm runs that ended in error.
    pub failed: Vec<MissionItem>,
    /// Budget rows whose spend has crossed the warn threshold (≥ 80 % of cap).
    pub budget_warn: Vec<MissionItem>,
}

// ---------------------------------------------------------------------------
// Short-lived per-workspace cache (3 s TTL)
// ---------------------------------------------------------------------------

struct CacheEntry {
    view: MissionView,
    born: Instant,
}

static CACHE: OnceLock<Mutex<HashMap<Id, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<Id, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const CACHE_TTL: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Assembly helpers
// ---------------------------------------------------------------------------

/// Build a `MissionItem` for a live session.
fn session_item(session: &otto_core::domain::Session, kind_override: &str) -> MissionItem {
    let age_secs = (Utc::now() - session.last_active_at).num_seconds().max(0);
    MissionItem {
        kind: "session".into(),
        id: session.id.clone(),
        title: session.title.clone(),
        status: kind_override.into(),
        session_id: Some(session.id.clone()),
        repo: None,
        cost_usd: None,
        age_secs,
    }
}

/// Assemble the full `MissionView` for one workspace from existing stores.
///
/// All reads are best-effort: a failing query yields an empty bucket for that
/// source rather than an error response, keeping the view partial but live.
async fn build_view(ctx: &ServerCtx, ws_id: &Id) -> MissionView {
    let mut view = MissionView::default();
    let now = Utc::now();

    // ------------------------------------------------------------------
    // 1. Sessions — needs_you, working, waiting
    //    Source: SessionManager live session list + workspace.needsYou flags.
    //    The "needs you" flags are stored client-side in the workspace store;
    //    on the daemon side we approximate it via `Idle` sessions that have
    //    a "waiting" activity trail entry.  We use the Idle status to fill
    //    the `waiting` bucket, and surface all non-archived agent sessions
    //    whose status is Working/Running in the `working` bucket.
    // ------------------------------------------------------------------
    let sessions = ctx
        .manager
        .list_by_workspace(ws_id)
        .await
        .unwrap_or_default();

    for session in &sessions {
        if session.archived {
            continue;
        }
        // Only consider agent sessions (not SSH/DB connections).
        if session.kind != SessionKind::Agent {
            continue;
        }

        let age = (now - session.last_active_at).num_seconds().max(0);

        match session.status {
            SessionStatus::Working | SessionStatus::Running => {
                view.working.push(session_item(session, "working"));
            }
            SessionStatus::Idle => {
                // Idle can mean "thinking" or "blocked waiting for input".
                // Surface in the `waiting` bucket; the UI can further
                // classify based on trail events or session age.
                view.waiting.push(session_item(session, "idle"));
                // Sessions idle for > 5 min with an agent (claude/codex)
                // are more likely to be waiting on the operator — promote
                // to needs_you.
                if age > 300 && !matches!(session.provider.as_str(), "shell") {
                    view.needs_you.push(MissionItem {
                        kind: "session".into(),
                        id: session.id.clone(),
                        title: session.title.clone(),
                        status: "needs_you".into(),
                        session_id: Some(session.id.clone()),
                        repo: None,
                        cost_usd: None,
                        age_secs: age,
                    });
                }
            }
            // Exited / Reconnectable sessions are not surfaced.
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // 2. PR reviews — review_ready
    //    Source: raw SQL over pr_reviews joined to git_repos on workspace.
    //    "Running" reviews have agents still in flight; "done" reviews with
    //    unposted draft comments are equally actionable.
    //
    //    ReviewsRepo has no workspace-scoped list, so we query the pool
    //    directly with a join — read-only, no mutation.
    // ------------------------------------------------------------------
    {
        // Fetch all non-error reviews for repos in this workspace, newest first.
        // We use a JOIN so we don't have to loop repo-by-repo (and load all comments).
        let rows = sqlx::query(
            "SELECT r.id, r.pr_number, r.status, r.created_at, r.agents_json,
                    gr.name AS repo_name
             FROM pr_reviews r
             JOIN repos gr ON gr.id = r.repo_id
             WHERE gr.workspace_id = ?
               AND r.status IN ('running', 'done')
             ORDER BY r.created_at DESC
             LIMIT 20",
        )
        .bind(ws_id)
        .fetch_all(&ctx.pool)
        .await
        .unwrap_or_default();

        for row in &rows {
            let review_id: String = row.get("id");
            let pr_number: i64 = row.get("pr_number");
            let status_str: String = row.get("status");
            let created_raw: String = row.get("created_at");
            let repo_name: Option<String> = row.try_get("repo_name").ok();

            let created = otto_state::convert::ts(&created_raw).unwrap_or(now);
            let age = (now - created).num_seconds().max(0);

            if status_str == "running" {
                view.review_ready.push(MissionItem {
                    kind: "review".into(),
                    id: review_id,
                    title: format!("PR #{pr_number} review"),
                    status: "running".into(),
                    session_id: None,
                    repo: repo_name,
                    cost_usd: None,
                    age_secs: age,
                });
            } else {
                // "done" — check for unposted draft comments.
                let draft_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pr_review_comments
                     WHERE review_id = ? AND state = 'draft' AND posted = 0",
                )
                .bind(&review_id)
                .fetch_one(&ctx.pool)
                .await
                .unwrap_or(0);

                if draft_count > 0 {
                    view.review_ready.push(MissionItem {
                        kind: "review".into(),
                        id: review_id,
                        title: format!(
                            "PR #{} — {} draft comment{}",
                            pr_number,
                            draft_count,
                            if draft_count == 1 { "" } else { "s" }
                        ),
                        status: "done".into(),
                        session_id: None,
                        repo: repo_name,
                        cost_usd: None,
                        age_secs: age,
                    });
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 3. Workflow runs — failed bucket
    //    Source: WorkflowsRepo SQL.  Query across all workflows in the
    //    workspace for runs in "error" status, bounded to 20 most recent.
    // ------------------------------------------------------------------
    {
        let wf_repo = WorkflowsRepo::new(ctx.pool.clone());
        let workflows = wf_repo.list(ws_id).await.unwrap_or_default();
        let mut failed_runs = 0usize;
        'wf: for wf in &workflows {
            let runs = wf_repo.list_runs(&wf.id).await.unwrap_or_default();
            for run in &runs {
                use otto_core::workflows::RunStatus;
                if matches!(run.status, RunStatus::Error) {
                    let age = (now - run.started_at).num_seconds().max(0);
                    view.failed.push(MissionItem {
                        kind: "workflow_run".into(),
                        id: run.id.clone(),
                        title: format!("{} run failed", wf.name),
                        status: "error".into(),
                        session_id: None,
                        repo: None,
                        cost_usd: None,
                        age_secs: age,
                    });
                    failed_runs += 1;
                    if failed_runs >= 20 {
                        break 'wf;
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 4. Swarm runs — failed bucket (append)
    //    Source: raw SQL on swarm_runs for this workspace + error status.
    //    SwarmRepo.list_runs has no workspace filter, so query the pool.
    // ------------------------------------------------------------------
    {
        let rows = sqlx::query(
            "SELECT id, session_id, error, cost_usd, started_at, enqueued_at
             FROM swarm_runs
             WHERE workspace_id = ? AND status = 'error'
             ORDER BY enqueued_at DESC LIMIT 20",
        )
        .bind(ws_id)
        .fetch_all(&ctx.pool)
        .await
        .unwrap_or_default();

        for row in &rows {
            let run_id: String = row.get("id");
            let session_id: Option<String> = row.try_get("session_id").ok().flatten();
            let error: Option<String> = row.try_get("error").ok().flatten();
            let cost_usd: Option<f64> = row.try_get("cost_usd").ok().flatten();
            let started_raw: Option<String> = row.try_get("started_at").ok().flatten();
            let enqueued_raw: String = row.get("enqueued_at");

            let started = started_raw
                .as_deref()
                .and_then(|s| otto_state::convert::ts(s).ok())
                .unwrap_or_else(|| {
                    otto_state::convert::ts(&enqueued_raw).unwrap_or(now)
                });
            let age = (now - started).num_seconds().max(0);
            let title = error
                .as_deref()
                .map(|e| format!("Swarm run failed: {}", &e[..e.len().min(80)]))
                .unwrap_or_else(|| "Swarm run failed".into());

            view.failed.push(MissionItem {
                kind: "swarm_run".into(),
                id: run_id,
                title,
                status: "error".into(),
                session_id,
                repo: None,
                cost_usd,
                age_secs: age,
            });
        }
    }

    // ------------------------------------------------------------------
    // 5. Budget warnings
    //    Source: usage::budget_status_pub — same helper the budgets route uses.
    // ------------------------------------------------------------------
    {
        use crate::routes::usage::budget_status_pub;
        use otto_state::SettingsRepo;

        if let Ok(Some(raw)) = SettingsRepo::new(ctx.pool.clone())
            .get("usage_budgets")
            .await
        {
            if let Ok(cfg) = serde_json::from_value::<otto_core::api::UsageBudgetConfig>(raw) {
                let status = budget_status_pub(ctx, cfg).await;
                for row in &status.rows {
                    if row.warning || row.exceeded {
                        view.budget_warn.push(MissionItem {
                            kind: "budget".into(),
                            id: row.key.clone(),
                            title: format!(
                                "{} budget: ${:.2} / ${:.2}",
                                row.label.as_deref().unwrap_or(&row.key),
                                row.spent_usd,
                                row.limit_usd,
                            ),
                            status: if row.exceeded { "exceeded" } else { "warning" }.into(),
                            session_id: None,
                            repo: None,
                            cost_usd: Some(row.spent_usd),
                            age_secs: 0,
                        });
                    }
                }
            }
        }
    }

    view
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// `GET /workspaces/{id}/mission`
///
/// Returns the six-bucket workspace work-queue view.  Results are cached for
/// 3 s so repeated refreshes from multiple UI components are not a query storm.
/// Requires Viewer-or-above workspace role.
pub async fn get_mission(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<MissionView>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;

    // Check the short-TTL cache first.
    let guard = cache().lock().await;
    if let Some(entry) = guard.get(&ws_id) {
        if entry.born.elapsed() < CACHE_TTL {
            return Ok(Json(entry.view.clone()));
        }
    }
    // Drop the lock while we do the (potentially slow) DB reads.
    drop(guard);

    let view = build_view(&ctx, &ws_id).await;

    // Write back to the cache.
    let mut guard = cache().lock().await;
    guard.insert(ws_id, CacheEntry { view: view.clone(), born: Instant::now() });
    drop(guard);

    Ok(Json(view))
}

/// `GET /workspaces/{id}/mission/views`
///
/// List the calling user's saved work-queue filter views for this workspace.
pub async fn list_views(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<SavedView>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let repo = SavedViewsRepo::new(ctx.pool.clone());
    let views = repo.list(&ws_id, &user.id).await.map_err(ApiError)?;
    Ok(Json(views))
}

/// `POST /workspaces/{id}/mission/views`
///
/// Create a new saved work-queue filter view for this workspace.
pub async fn create_view(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NewSavedView>,
) -> ApiResult<(StatusCode, Json<SavedView>)> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(otto_core::Error::Invalid(
            "view name is required".into(),
        )));
    }
    let repo = SavedViewsRepo::new(ctx.pool.clone());
    let view = repo.create(&ws_id, &user.id, req).await.map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// `DELETE /mission-views/{id}`
///
/// Delete a saved view.  Only the view's owner may delete it.
pub async fn delete_view(
    Path(view_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = SavedViewsRepo::new(ctx.pool.clone());
    // Load first to verify ownership before deleting.
    let view = repo.get(&view_id).await.map_err(ApiError)?;
    // Reject if the caller is neither the owner nor root.
    if view.user_id != user.id && !user.is_root {
        return Err(ApiError(otto_core::Error::Forbidden(
            "not your saved view".into(),
        )));
    }
    // Workspace role check: caller must at least be a Viewer in the view's workspace.
    require_ws_role(&ctx, &user, &view.workspace_id, WorkspaceRole::Viewer).await?;
    repo.delete(&view_id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Mission Control routes: mounted as an api_extra in `module_routers`.
pub fn mission_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/mission", get(get_mission))
        .route(
            "/workspaces/{id}/mission/views",
            get(list_views).post(create_view),
        )
        .route("/mission-views/{id}", delete(delete_view))
}

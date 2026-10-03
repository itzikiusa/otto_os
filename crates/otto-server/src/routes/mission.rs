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
use otto_state::{NewSavedView, SavedView, SavedViewsRepo};
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

/// Per-workspace build gates (perf §15 N3): concurrent cache misses — the
/// sidebar badge, the page and a WS-driven refresh landing together — queue
/// on one gate and the followers re-read the cache the leader filled instead
/// of each running `build_view`. Idle gates are pruned on every miss.
static BUILDING: OnceLock<std::sync::Mutex<HashMap<Id, std::sync::Arc<Mutex<()>>>>> =
    OnceLock::new();

fn build_gate(ws_id: &Id) -> std::sync::Arc<Mutex<()>> {
    let mut map = BUILDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Nobody else holds a pruned gate (strong count 1 = only the map).
    map.retain(|k, g| k == ws_id || std::sync::Arc::strong_count(g) > 1);
    map.entry(ws_id.clone()).or_default().clone()
}

async fn cached(ws_id: &Id) -> Option<MissionView> {
    let guard = cache().lock().await;
    guard
        .get(ws_id)
        .filter(|e| e.born.elapsed() < CACHE_TTL)
        .map(|e| e.view.clone())
}

/// The cached view, or one build shared by every concurrent miss of `ws_id`.
async fn single_flight<F, Fut>(ws_id: &Id, build: F) -> MissionView
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = MissionView>,
{
    if let Some(v) = cached(ws_id).await {
        return v;
    }
    let gate = build_gate(ws_id);
    let _building = gate.lock().await;
    // A leader may have filled the cache while we queued.
    if let Some(v) = cached(ws_id).await {
        return v;
    }
    let view = build().await;
    let mut guard = cache().lock().await;
    guard.retain(|_, e| e.born.elapsed() < CACHE_TTL);
    guard.insert(
        ws_id.clone(),
        CacheEntry {
            view: view.clone(),
            born: Instant::now(),
        },
    );
    view
}

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
    // Active agent sessions only, filtered in SQL (perf §15 F7) — archived
    // history and connection sessions were decoded and then skipped below.
    let sessions = ctx
        .manager
        .list_filtered(
            &[otto_state::SessionScope {
                workspace_id: ws_id.clone(),
                owner: None,
            }],
            &otto_state::SessionListFilter {
                archived: Some(false),
                kind: Some("agent".into()),
                ..Default::default()
            },
        )
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

        // Unposted draft counts for every "done" review in ONE grouped query
        // (it was one COUNT(*) per review, SA-09).
        let done_ids: Vec<String> = rows
            .iter()
            .filter(|r| r.get::<String, _>("status") != "running")
            .map(|r| r.get::<String, _>("id"))
            .collect();
        let mut drafts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        if !done_ids.is_empty() {
            let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT review_id, COUNT(*) AS n FROM pr_review_comments
                 WHERE state = 'draft' AND posted = 0 AND review_id IN (",
            );
            let mut sep = q.separated(", ");
            for id in &done_ids {
                sep.push_bind(id.clone());
            }
            q.push(") GROUP BY review_id");
            for r in q.build().fetch_all(&ctx.pool).await.unwrap_or_default() {
                drafts.insert(r.get("review_id"), r.get("n"));
            }
        }

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
                let draft_count: i64 = drafts.get(&review_id).copied().unwrap_or(0);

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
        // One query for the workspace's 20 most recent failed runs (it was
        // `list_runs` — 50 full rows with their node/state JSON — per
        // workflow, SA-09). Filtered on the RUN's workspace so it walks
        // `idx_workflow_runs_ws_status` (0145) instead of every run of every
        // workflow in the workspace (r3-07-05).
        let rows = sqlx::query(
            "SELECT r.id, r.started_at, w.name
               FROM workflow_runs r
               JOIN workflows w ON w.id = r.workflow_id
              WHERE r.workspace_id = ? AND r.status = 'error'
              ORDER BY r.started_at DESC
              LIMIT 20",
        )
        .bind(ws_id)
        .fetch_all(&ctx.pool)
        .await
        .unwrap_or_default();
        for row in &rows {
            let started_raw: String = row.get("started_at");
            let started = otto_state::convert::ts(&started_raw).unwrap_or(now);
            let name: String = row.get("name");
            view.failed.push(MissionItem {
                kind: "workflow_run".into(),
                id: row.get("id"),
                title: format!("{name} run failed"),
                status: "error".into(),
                session_id: None,
                repo: None,
                cost_usd: None,
                age_secs: (now - started).num_seconds().max(0),
            });
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
                .unwrap_or_else(|| otto_state::convert::ts(&enqueued_raw).unwrap_or(now));
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

    let view = single_flight(&ws_id, || build_view(&ctx, &ws_id)).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// perf §15 N3: eight concurrent misses on one workspace run ONE build;
    /// another workspace builds on its own; the gate map is pruned.
    #[tokio::test]
    async fn concurrent_misses_share_one_build() {
        let builds = Arc::new(AtomicUsize::new(0));
        let ws: Id = "mission-single-flight-ws".into();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let builds = builds.clone();
            let ws = ws.clone();
            tasks.push(tokio::spawn(async move {
                single_flight(&ws, || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    MissionView::default()
                })
                .await
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        let other: Id = "mission-single-flight-other".into();
        let b2 = builds.clone();
        single_flight(&other, || async move {
            b2.fetch_add(1, Ordering::SeqCst);
            MissionView::default()
        })
        .await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        // Only the last-touched gate may linger; idle ones were pruned.
        let gates = BUILDING.get().unwrap().lock().unwrap();
        assert!(!gates.contains_key(&ws), "idle gate pruned");
    }
}

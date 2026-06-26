//! `POST /webhooks/swarm/{workspace_id}/{swarm_id}` — an external trigger that
//! starts a swarm fully automatically: create a project (goal = the payload),
//! run the planner to seed tasks, set the swarm active, and start the
//! coordinator. Agents run in git **worktrees** (so several can work the same
//! repo in parallel without clobbering each other).
//!
//! Auth reuses the per-workspace webhook key (keychain `chan-bot-{ws}-webhook`,
//! the same one the channel webhook uses) via `X-Otto-Webhook-Key` or a
//! `Authorization: Bearer <key>` header. Mounted in `public_routes` (outside the
//! user-auth chokepoint), like the channel webhook + the `/ingest/*` routes.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::Id;
use serde::Deserialize;
use serde_json::json;

use crate::state::ServerCtx;
use crate::swarm_channels::{GoalSpec, LaunchOpts, Origin};

#[derive(Deserialize)]
pub struct WebhookGoalReq {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub comparator: Option<String>,
    #[serde(default)]
    pub target_value: Option<f64>,
    #[serde(default)]
    pub block_value: Option<f64>,
    #[serde(default)]
    pub verify_cmd: Option<String>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    #[serde(default)]
    pub blocking: Option<bool>,
}

#[derive(Deserialize)]
pub struct SwarmTriggerReq {
    /// What the team should do (e.g. "Feature X is ready — write acceptance
    /// tests for it"). Becomes the project's goal the planner breaks down.
    pub goal: String,
    /// Project name (defaults to the goal's first line).
    #[serde(default)]
    pub name: Option<String>,
    /// Repo the agents work in (worktree isolation branches from it; without it
    /// agents fall back to a scratch dir).
    #[serde(default)]
    pub repo_path: Option<String>,
    /// Explicit goals the leader verifies after the work is done (requirement 3).
    #[serde(default)]
    pub goals: Vec<WebhookGoalReq>,
    /// Where to POST progress/result/escalation back (optional).
    #[serde(default)]
    pub callback_url: Option<String>,
    /// Start the coordinator immediately (default true). False = plan only.
    #[serde(default)]
    pub start: Option<bool>,
}

/// Pull the webhook key from `X-Otto-Webhook-Key`, else `Authorization: Bearer`.
fn extract_key(headers: &HeaderMap) -> Option<String> {
    if let Some(k) = headers.get("x-otto-webhook-key").and_then(|v| v.to_str().ok()) {
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

/// Constant-time comparison so a bad key can't be timing-probed.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn trigger(
    Path((ws, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<SwarmTriggerReq>,
) -> Response {
    // 1. Auth against the per-workspace webhook key.
    let expected = match ctx.secrets.get(&format!("chan-bot-{ws}-webhook")) {
        Ok(Some(k)) if !k.is_empty() => k,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match extract_key(&headers) {
        Some(k) if ct_eq(k.as_bytes(), expected.as_bytes()) => {}
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    }

    // 2. Resolve + scope the target swarm to this workspace.
    let swarm = match ctx.swarm_repo.get_swarm(&sid).await {
        Ok(s) if s.workspace_id == ws => s,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if req.goal.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "goal is required").into_response();
    }

    // 3. Launch via the shared path (creates the project, attaches goals, records
    //    the channel origin, seeds tasks + starts the coordinator in the
    //    background). Agents run in worktrees by default (the cwd-mode default), so
    //    several can share the repo without clobbering each other.
    let start = req.start.unwrap_or(true);
    let goals: Vec<GoalSpec> = req
        .goals
        .into_iter()
        .map(|g| GoalSpec {
            title: g.title,
            description: g.description,
            metric: g.metric,
            comparator: g.comparator,
            target_value: g.target_value,
            block_value: g.block_value,
            verify_cmd: g.verify_cmd,
            max_retries: g.max_retries,
            blocking: g.blocking,
        })
        .collect();
    let origin = req.callback_url.clone().filter(|u| !u.trim().is_empty()).map(|url| Origin {
        channel: "webhook".into(),
        chat: url,
        thread: None,
    });
    let opts = LaunchOpts {
        goal: req.goal.clone(),
        name: req.name.clone(),
        repo_path: req.repo_path.clone(),
        goals,
        origin,
        start,
        created_by: swarm.created_by.clone(),
    };
    match crate::swarm_channels::launch(&ctx, &swarm, opts).await {
        Ok(project_id) => (
            StatusCode::ACCEPTED,
            Json(json!({ "swarm_id": sid, "project_id": project_id, "started": start })),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("swarm webhook: launch: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

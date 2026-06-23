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
use otto_state::{NewProject, SwarmPatch};
use serde::Deserialize;
use serde_json::json;

use crate::state::ServerCtx;

#[derive(Deserialize)]
pub struct SwarmTriggerReq {
    /// What the team should do (e.g. "Feature X is ready — write acceptance
    /// tests for it"). Becomes the project's goal the planner breaks down.
    pub goal: String,
    /// Project name (defaults to the goal's first line).
    #[serde(default)]
    pub name: Option<String>,
    /// Repo the agents work in (required for worktree isolation to actually
    /// branch; without it agents fall back to a scratch dir).
    #[serde(default)]
    pub repo_path: Option<String>,
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

    // 3. Enforce worktree isolation: each agent gets its own branch/worktree.
    if swarm.config.get("cwd_mode").and_then(|v| v.as_str()) != Some("worktree") {
        let mut cfg = swarm.config.clone();
        if let Some(obj) = cfg.as_object_mut() {
            obj.insert("cwd_mode".into(), json!("worktree"));
        }
        let _ = ctx
            .swarm_repo
            .update_swarm(&sid, SwarmPatch { config: Some(cfg), ..Default::default() })
            .await;
    }

    // 4. Create the project from the goal.
    let creator = swarm.created_by.clone();
    let name = req.name.clone().unwrap_or_else(|| {
        req.goal
            .lines()
            .next()
            .unwrap_or("Webhook feature")
            .chars()
            .take(80)
            .collect()
    });
    let project = match ctx
        .swarm_repo
        .create_project(NewProject {
            swarm_id: sid.clone(),
            workspace_id: ws.clone(),
            name,
            description: "Triggered via webhook".into(),
            repo_path: req.repo_path.clone(),
            goal_md: Some(req.goal.clone()),
            story_id: None,
            order_idx: 0,
            created_by: creator.clone(),
        })
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("swarm webhook: create_project: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let project_id = project.id.clone();

    // 5. Plan + start in the background; reply 202 immediately (the planner can
    //    run for a while, and external callers shouldn't block on it).
    let start = req.start.unwrap_or(true);
    let goal = req.goal.clone();
    let ctx2 = ctx.clone();
    let sid2 = sid.clone();
    let ws2 = ws.clone();
    tokio::spawn(async move {
        let _ = crate::product_swarm::seed_tasks(&ctx2, &project, &creator, &goal).await;
        if start {
            let _ = ctx2.swarm_repo.set_swarm_status(&sid2, "active").await;
            crate::swarm_runtime::set_paused(&ctx2, &sid2, false);
            crate::swarm_runtime::start_coordinator(ctx2.clone(), sid2.clone());
            crate::swarm_runtime::emit_status(&ctx2, &ws2, &sid2, "active");
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "swarm_id": sid, "project_id": project_id, "started": start })),
    )
        .into_response()
}

//! `POST /ingest/swarm/board` — a swarm agent posts to its shared board using the
//! per-session ingest token (same gate as `/ingest/claude`). The agent runs the
//! materialized `otto-post` helper, which sends `X-Otto-Session` + `X-Otto-Token`.
//! The session's `meta` (set when the swarm spawned it) carries `swarm_id` and
//! `agent_id`. Always returns 204 (fire-and-forget for the agent).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_core::event::Event;
use otto_core::Id;
use otto_state::NewMessage;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::ServerCtx;

#[derive(Deserialize)]
pub struct BoardIngestReq {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub to_agent_id: Option<Id>,
    pub body: String,
}

pub async fn board_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<BoardIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    let meta = &session.meta;
    let swarm_id = meta.get("swarm_id").and_then(Value::as_str);
    let agent_id = meta.get("agent_id").and_then(Value::as_str);
    let (Some(swarm_id), Some(agent_id)) = (swarm_id, agent_id) else {
        return StatusCode::NO_CONTENT; // not a swarm session
    };
    let project_id = meta.get("project_id").and_then(Value::as_str).map(str::to_string);
    let task_id = meta.get("task_id").and_then(Value::as_str).map(str::to_string);
    let run_id = meta.get("run_id").and_then(Value::as_str).map(str::to_string);

    let body = req.body.trim();
    if body.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    let kind = req.kind.unwrap_or_else(|| "message".into());

    match ctx
        .swarm_repo
        .create_message(NewMessage {
            swarm_id: swarm_id.to_string(),
            workspace_id: session.workspace_id.clone(),
            project_id,
            task_id,
            run_id,
            author_agent_id: Some(agent_id.to_string()),
            author_user_id: None,
            to_agent_id: req.to_agent_id,
            kind,
            body: body.to_string(),
            meta: json!({ "session_id": sid }),
        })
        .await
    {
        Ok(msg) => {
            let _ = ctx.events.send(Event::SwarmMessagePosted {
                workspace_id: session.workspace_id.clone(),
                swarm_id: swarm_id.to_string(),
                message: serde_json::to_value(&msg).unwrap_or_default(),
            });
        }
        Err(e) => tracing::warn!("swarm board ingest: {e}"),
    }
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct ProductIngestReq {
    #[serde(default)]
    pub title: Option<String>,
    pub body_md: String,
}

/// `POST /ingest/swarm/product` — a swarm (PO/feature-design) agent publishes a
/// feature DRAFT to the Product page via the materialized `otto-product` helper.
/// Same per-session auth as the board ingest. Creates a new draft story the
/// user/PO reviews. Always 204 (fire-and-forget).
pub async fn product_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<ProductIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    // Only swarm sessions may write drafts.
    if session.meta.get("swarm_id").and_then(Value::as_str).is_none() {
        return StatusCode::NO_CONTENT;
    }
    let body = req.body_md.trim();
    if body.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Feature draft");

    match ctx
        .product
        .create_draft(&session.workspace_id, &session.created_by, Some(title))
        .await
    {
        Ok(detail) => {
            let _ = ctx
                .product
                .update_draft_body(&detail.story.id, title, body, &session.created_by)
                .await;
            let _ = ctx.events.send(Event::ProductChanged {
                workspace_id: session.workspace_id.clone(),
                story_id: detail.story.id,
                section: "source".into(),
                status: "draft".into(),
            });
        }
        Err(e) => tracing::warn!("swarm product ingest: {e}"),
    }
    StatusCode::NO_CONTENT
}

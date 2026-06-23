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
use otto_state::{NewAttachment, NewMessage};
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

/// Storage sub-path under `data_dir` for story attachments (mirrors
/// `product_media::ATTACH_ROOT`).
const ATTACH_ROOT: &str = "product/attachments";

/// File extension for a generated mockup payload. Mermaid diagrams land as
/// `.mmd`; everything else (HTML) as `.html`.
fn mockup_ext(format: &str) -> &'static str {
    match format {
        "mermaid" => "mmd",
        _ => "html",
    }
}

/// MIME for a generated mockup payload, paired with `mockup_ext`.
fn mockup_mime(format: &str) -> &'static str {
    match format {
        "mermaid" => "text/vnd.mermaid",
        _ => "text/html",
    }
}

#[derive(Deserialize)]
pub struct MockupIngestReq {
    pub title: String,
    /// `"html"` | `"mermaid"`.
    pub format: String,
    pub content: String,
}

/// `POST /ingest/swarm/mockup` — a swarm discovery/design agent publishes a
/// generated mockup (HTML page or Mermaid diagram) for the story under
/// discovery. Same per-session auth as the board ingest. The target story/run is
/// derived from the session's `meta.project_id` → its discovery run; the agent
/// never supplies a story/run id. Always 204 (fire-and-forget).
pub async fn ingest_mockup(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<MockupIngestReq>,
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
    // Derive the target discovery run from the session's project — never trust
    // the agent for a story/run id.
    let project_id = match session.meta.get("project_id").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return StatusCode::NO_CONTENT, // not a discovery session
    };
    let run = match ctx.discovery_repo.get_by_project(&project_id).await {
        Ok(Some(r)) => r,
        _ => return StatusCode::NO_CONTENT, // no discovery run resolves
    };

    let title = req.title.trim();
    if title.is_empty() || req.content.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    let ext = mockup_ext(&req.format);
    let mime = mockup_mime(&req.format);

    // Mirror `product_media::upload_attachment`'s storage-path convention:
    // `data_dir/product/attachments/<story_id>/<id>.<ext>`, with `storage_path`
    // stored RELATIVE to `data_dir`.
    let id = otto_core::new_id();
    let rel = format!("{ATTACH_ROOT}/{}/{}.{}", run.story_id, id, ext);
    let dir = ctx.data_dir.join(ATTACH_ROOT).join(&run.story_id);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!("swarm mockup ingest: create dir: {e}");
        return StatusCode::NO_CONTENT;
    }
    let full = ctx.data_dir.join(&rel);
    if let Err(e) = tokio::fs::write(&full, req.content.as_bytes()).await {
        tracing::warn!("swarm mockup ingest: write: {e}");
        return StatusCode::NO_CONTENT;
    }
    let size_bytes = req.content.len() as i64;

    if let Err(e) = ctx
        .attachment_repo
        .create(NewAttachment {
            story_id: run.story_id.clone(),
            workspace_id: run.workspace_id.clone(),
            filename: format!("{title}.{ext}"),
            mime: mime.into(),
            size_bytes,
            sha256: None,
            storage_path: rel,
            kind: "mockup".into(),
            source: "agent".into(),
            meta_json: None,
            created_by: session.created_by.clone(),
        })
        .await
    {
        tracing::warn!("swarm mockup ingest: {e}");
    }
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct DiscoveryReportIngestReq {
    pub report_md: String,
}

/// `POST /ingest/swarm/discovery-report` — a swarm discovery agent publishes the
/// consolidated discovery report for the story under discovery. Same per-session
/// auth as the board ingest; the target run is derived from the session's
/// `meta.project_id`. Always 204 (fire-and-forget).
pub async fn ingest_discovery_report(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<DiscoveryReportIngestReq>,
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
    let project_id = match session.meta.get("project_id").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let run = match ctx.discovery_repo.get_by_project(&project_id).await {
        Ok(Some(r)) => r,
        _ => return StatusCode::NO_CONTENT,
    };

    let report = req.report_md.trim();
    if report.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    if let Err(e) = ctx.discovery_repo.set_report(&run.id, report).await {
        tracing::warn!("swarm discovery report ingest: {e}");
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mockup_ext_maps_format() {
        assert_eq!(mockup_ext("mermaid"), "mmd");
        assert_eq!(mockup_ext("html"), "html");
        // Unknown / anything-else falls back to html.
        assert_eq!(mockup_ext("svg"), "html");
    }

    #[test]
    fn mockup_mime_maps_format() {
        assert_eq!(mockup_mime("mermaid"), "text/vnd.mermaid");
        assert_eq!(mockup_mime("html"), "text/html");
        assert_eq!(mockup_mime("other"), "text/html");
    }
}

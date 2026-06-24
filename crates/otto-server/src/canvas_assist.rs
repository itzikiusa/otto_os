//! Agent-assisted canvas drawing.
//!
//! Turns a natural-language prompt into diagram blocks for Canvas Studio. Lives
//! here (not in `otto-canvas`) because it needs the orchestrator. A single
//! headless `run_agent` turn returns EITHER a fenced ```mermaid block (preferred
//! for sequence/flow/UML/ER/state) OR a ```json `{nodes,edges}` block; the UI
//! inserts the result into the scene. The prompt embeds an `OTTO_TASK:
//! canvas_assist` sentinel so the E2E stub can answer it deterministically.
//!
//! Routes (registered in modules.rs, gated by `Feature::Canvas`):
//!   POST /api/v1/canvas/scenes/{id}/assist   (ws editor) → AssistResult
//!   POST /api/v1/canvas/assist/preview       (canvas edit) → AssistResult

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AssistReq {
    pub prompt: String,
    /// Optional hint: `auto` (default) | `sequence` | `flow` | `uml` | `nodes`.
    #[serde(default)]
    pub mode: Option<String>,
    /// Required only by `/canvas/assist/preview` (no scene → no workspace in the
    /// path): which workspace to run the throwaway session in.
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct AssistResult {
    /// A mermaid diagram source, when the agent produced one (the common path).
    pub mermaid: Option<String>,
    /// Freeform nodes, when the agent produced tier-2 JSON instead of mermaid.
    pub nodes: Vec<Value>,
    /// Connectors for the freeform nodes.
    pub edges: Vec<Value>,
    /// A short human note (the agent's prose, or an error explanation).
    pub note: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /canvas/scenes/{id}/assist` — generate blocks for an existing scene
/// (Editor on the scene's workspace). Does not mutate the scene server-side;
/// the UI inserts the returned blocks and saves.
pub async fn assist_scene(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AssistReq>,
) -> ApiResult<Json<AssistResult>> {
    let scene = ctx
        .canvas_repo
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("canvas scene {id}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &scene.workspace_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&scene.workspace_id).await.map_err(ApiError)?;

    let prompt = build_assist_prompt(&req.prompt, req.mode.as_deref());
    let meta = serde_json::json!({ "source": "canvas_assist", "scene_id": scene.id });
    // Resume the scene's session across Ask-AI calls (visible/resumable in Agents).
    let (raw, sid) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        scene.session_id.as_ref(),
        &format!("Canvas: {}", scene.title),
        &ws.root_path,
        "claude",
        meta,
        &prompt,
    )
    .await?;
    if scene.session_id.is_none() {
        let _ = ctx.canvas_repo.set_session(&scene.id, &sid).await;
    }
    Ok(Json(parse_assist(&raw)))
}

/// `POST /canvas/assist/preview` — generate blocks with no scene (the
/// empty-canvas hero / Discovery-Chat bridge). Runs a THROWAWAY session in the
/// given workspace and archives it after (no scene to reuse it). Gated by the
/// `Feature::Canvas` edit capability (enforced upstream).
pub async fn assist_preview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AssistReq>,
) -> ApiResult<Json<AssistResult>> {
    let ws_id = req
        .workspace_id
        .clone()
        .ok_or_else(|| ApiError(Error::Invalid("workspace_id is required for preview".into())))?;
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id.clone()), WorkspaceRole::Editor)
        .await?;
    let ws = ctx.workspaces.get(&Id::from(ws_id)).await.map_err(ApiError)?;

    let prompt = build_assist_prompt(&req.prompt, req.mode.as_deref());
    let meta = serde_json::json!({ "source": "canvas_assist_preview" });
    let (raw, sid) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        None,
        "Canvas: preview",
        &ws.root_path,
        "claude",
        meta,
        &prompt,
    )
    .await?;
    // No scene to reuse the session — archive it so it doesn't clutter Agents.
    let _ = ctx.manager.kill_session(&sid).await;
    Ok(Json(parse_assist(&raw)))
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested, no DB / no agent)
// ---------------------------------------------------------------------------

/// Build the two-tier IR prompt. The `OTTO_TASK: canvas_assist` sentinel routes
/// the deterministic E2E stub; the rest instructs the real agent.
fn build_assist_prompt(user_prompt: &str, mode: Option<&str>) -> String {
    let mode_hint = match mode.unwrap_or("auto") {
        "sequence" => "Prefer a `sequenceDiagram`.",
        "flow" => "Prefer a `flowchart TD`.",
        "uml" => "Prefer a `classDiagram`.",
        "nodes" => "Prefer the tier-2 JSON `{nodes,edges}` form.",
        _ => "Pick the clearest diagram type for the request.",
    };
    format!(
        "OTTO_TASK: canvas_assist\n\
         You are a senior diagramming expert producing a POLISHED, presentation-grade \
         diagram that will be rendered onto a visual canvas as EDITABLE shapes (via \
         mermaid-to-excalidraw). Return a SINGLE fenced ```mermaid block.\n\n\
         MAKE IT TOP-NOTCH:\n\
         - Choose the RIGHT diagram type for the content. {mode_hint} (flowchart, \
         sequenceDiagram, classDiagram, stateDiagram-v2 or erDiagram).\n\
         - For flowcharts pick a sensible direction (`flowchart TD` for processes, `LR` for \
         pipelines), use rounded process boxes, diamond decisions (`id{{...}}`), and LABEL \
         the branch edges (`-->|yes|`, `-->|no|`), including error/retry paths.\n\
         - COLOR-CODE it: define a few tasteful `classDef`s and APPLY them with `class A,B name` \
         (e.g. classDef start fill:#dcfce7,stroke:#16a34a; classDef proc fill:#eef2ff,stroke:#6366f1; \
         classDef decision fill:#fef9c3,stroke:#ca8a04; classDef io fill:#f3e8ff,stroke:#9333ea; \
         classDef done fill:#fee2e2,stroke:#dc2626).\n\
         - GROUP related steps in `subgraph` blocks — use them as services / phases / swimlanes.\n\
         - Prefix node labels with a fitting EMOJI icon where it helps readability.\n\
         - Be EXHAUSTIVE and accurate: include every step and branch the request implies; keep \
         node text short; label edges with the data/event flowing.\n\
         - Output VALID mermaid ONLY inside the fence (no nested fences, no stray prose, no \
         comments that break parsing).\n\
         You may add ONE short sentence of prose before the block.\n\n\
         Request: {user_prompt}\n"
    )
}

/// Parse an assist reply: a ```mermaid fence wins; otherwise a ```json (or bare)
/// `{nodes,edges}` object; otherwise the raw text becomes the note. Never panics.
fn parse_assist(raw: &str) -> AssistResult {
    if let Some(src) = extract_fenced(raw, "mermaid") {
        return AssistResult {
            mermaid: Some(src),
            note: prose_before_fence(raw),
            ..Default::default()
        };
    }
    if let Some(v) = otto_swarm::recruiter::extract_json(raw) {
        let nodes = v
            .get("nodes")
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default();
        if !nodes.is_empty() {
            let edges = v
                .get("edges")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            return AssistResult {
                nodes,
                edges,
                note: prose_before_fence(raw),
                ..Default::default()
            };
        }
    }
    // Nothing structured — keep the agent's words so the UI can surface them.
    AssistResult {
        note: raw.trim().to_string(),
        ..Default::default()
    }
}

/// Extract the contents of the first ```<lang> ... ``` fenced block.
fn extract_fenced(raw: &str, lang: &str) -> Option<String> {
    let open = format!("```{lang}");
    let start = raw.find(&open)?;
    let after = &raw[start + open.len()..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let end = after.find("```")?;
    let body = after[..end].trim();
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

/// The prose preceding the first fenced block (a one-line note), trimmed.
fn prose_before_fence(raw: &str) -> String {
    let cut = raw.find("```").unwrap_or(raw.len());
    let prose = raw[..cut].trim();
    if prose.is_empty() {
        "Added to the canvas.".to_string()
    } else {
        prose.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_has_sentinel_and_mode() {
        let p = build_assist_prompt("service A calls B", Some("sequence"));
        assert!(p.contains("OTTO_TASK: canvas_assist"));
        assert!(p.contains("sequenceDiagram"));
        assert!(p.contains("service A calls B"));
    }

    #[test]
    fn parse_prefers_mermaid_fence() {
        let raw = "Here you go.\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n```";
        let r = parse_assist(raw);
        assert_eq!(r.mermaid.as_deref(), Some("sequenceDiagram\n  A->>B: hi"));
        assert!(r.nodes.is_empty());
        assert_eq!(r.note, "Here you go.");
    }

    #[test]
    fn parse_falls_back_to_nodes_json() {
        let raw = r#"ok ```json
{"nodes":[{"id":"n1","kind":"shape","x":0,"y":0,"w":120,"h":60,"label":"A"}],"edges":[]}
```"#;
        let r = parse_assist(raw);
        assert!(r.mermaid.is_none());
        assert_eq!(r.nodes.len(), 1);
        assert_eq!(r.nodes[0]["label"], "A");
    }

    #[test]
    fn parse_unstructured_keeps_note() {
        let raw = "I couldn't draw that, here's why...";
        let r = parse_assist(raw);
        assert!(r.mermaid.is_none());
        assert!(r.nodes.is_empty());
        assert_eq!(r.note, raw);
    }
}

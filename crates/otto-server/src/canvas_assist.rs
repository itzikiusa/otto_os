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
    /// Excalidraw element SKELETON the agent authored directly (preferred — true
    /// code blocks, icons, frames). `{ "elements": [...] }` or a bare array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excalidraw: Option<Value>,
    /// A mermaid diagram source (fallback — clean auto-layout flowcharts).
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
        "sequence" => {
            "This is a SEQUENCE — emit a ```mermaid `sequenceDiagram` instead of JSON (cleaner)."
        }
        "uml" => "This is a CLASS diagram — emit a ```mermaid `classDiagram` instead of JSON.",
        "flow" | "nodes" => "Emit the Excalidraw JSON (a flowchart / architecture diagram).",
        _ => {
            "Pick the best fit: Excalidraw JSON for flowcharts / architecture (richer — code \
             blocks, icons), or a ```mermaid `sequenceDiagram` / `classDiagram` / `erDiagram`."
        }
    };
    format!(
        "OTTO_TASK: canvas_assist\n\
         You are an expert diagrammer drawing onto an EXCALIDRAW canvas. PREFER a single fenced \
         ```json block of the form {{\"elements\": [ ...element skeletons... ]}} — the app turns \
         it into real, EDITABLE Excalidraw shapes. {mode_hint}\n\n\
         ELEMENT SKELETON (use exactly this shape):\n\
         - Shape: {{\"type\":\"rectangle\"|\"ellipse\"|\"diamond\",\"id\":\"n1\",\"x\":int,\"y\":int,\
         \"width\":int,\"height\":int,\"backgroundColor\":\"#hex\",\"strokeColor\":\"#hex\",\
         \"fillStyle\":\"solid\",\"roundness\":{{\"type\":3}},\
         \"label\":{{\"text\":\"...\",\"fontSize\":16,\"fontFamily\":2,\"strokeColor\":\"#hex\"}}}}\n\
         - Arrow (connect by node id; the app routes it): {{\"type\":\"arrow\",\"x\":int,\"y\":int,\
         \"start\":{{\"id\":\"n1\"}},\"end\":{{\"id\":\"n2\"}},\"strokeColor\":\"#94a3b8\",\
         \"label\":{{\"text\":\"yes\"}}}}\n\
         - fontFamily: 2 = normal, 3 = CODE (monospace).\n\n\
         LAYOUT — you own the coordinates, make it clean:\n\
         - Lay out left→right (pipelines) or top→down (processes); space nodes ~80px apart with \
         NO overlaps. Unique id per node; connect everything with arrows.\n\
         - SIZE EVERY BOX TO ITS TEXT so nothing clips: width >= 28 + ~9*chars-of-longest-line, \
         height >= 28 + ~22*number-of-lines.\n\n\
         STYLE — top-notch, presentation-grade:\n\
         - Colour-code by role: start fill #dcfce7 stroke #16a34a; process fill #eef2ff stroke \
         #6366f1; decision (DIAMOND) fill #fef9c3 stroke #ca8a04; io fill #f3e8ff stroke #9333ea; \
         data fill #ecfeff stroke #0891b2; done/error fill #fee2e2 stroke #dc2626. Dark readable text.\n\
         - Decisions are DIAMONDS with labeled out-arrows (yes/no), including error/retry paths.\n\
         - CODE BLOCK: a rectangle, backgroundColor #0f172a, strokeColor #334155, label fontFamily:3 \
         strokeColor #e2e8f0 with the REAL code (\\n between lines) — make it WIDE + TALL enough for \
         every line (size to the longest line).\n\
         - Prefix labels with a fitting EMOJI icon; label arrows with the data/event flowing.\n\
         - Be exhaustive + accurate; keep node text short (except code blocks). VALID JSON only \
         inside the fence (no trailing commas, no comments).\n\
         You may add ONE short sentence of prose before the block.\n\n\
         Request: {user_prompt}\n"
    )
}

/// Parse an assist reply: a ```mermaid fence wins; otherwise a ```json (or bare)
/// `{nodes,edges}` object; otherwise the raw text becomes the note. Never panics.
fn parse_assist(raw: &str) -> AssistResult {
    // Excalidraw element SKELETON (preferred): a ```json (or bare) object with a
    // non-empty `elements` array.
    if let Some(v) = otto_swarm::recruiter::extract_json(raw) {
        let has_elements = v
            .get("elements")
            .and_then(|e| e.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        if has_elements {
            return AssistResult {
                excalidraw: Some(v),
                note: prose_before_fence(raw),
                ..Default::default()
            };
        }
    }
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
    fn parse_prefers_excalidraw_elements() {
        let raw = "Here.\n\n```json\n{\"elements\":[{\"type\":\"rectangle\",\"id\":\"a\",\"x\":0,\"y\":0}]}\n```";
        let r = parse_assist(raw);
        assert!(r.excalidraw.is_some(), "an elements payload → excalidraw");
        assert!(r.mermaid.is_none());
        assert!(r.nodes.is_empty());
        assert_eq!(r.note, "Here.");
    }

    #[test]
    fn parse_prefers_mermaid_fence() {
        let raw = "Here you go.\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n```";
        let r = parse_assist(raw);
        assert_eq!(r.mermaid.as_deref(), Some("sequenceDiagram\n  A->>B: hi"));
        assert!(r.excalidraw.is_none());
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

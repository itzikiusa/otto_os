//! Agent-assisted canvas drawing — FILE-BACKED.
//!
//! Each scene has a persistent source file the agent EDITS across the
//! conversation (a `canvas.mermaid` by default, or `canvas.excalidraw.json`),
//! kept in an Otto-owned per-scene directory. An "Ask AI" turn:
//!   1. materializes the scene's current source into that file,
//!   2. runs ONE resumed agent turn whose prompt says "edit the file in place"
//!      (so follow-ups REFINE the same diagram instead of regenerating it),
//!   3. reads the file back, commits it as the scene's `doc_json`, and
//!   4. broadcasts `Event::CanvasUpdated` so the open editor re-renders.
//!
//! While the turn runs we poll the file and broadcast each change LIVE, so the
//! diagram "draws itself" as the agent writes (no `notify` dependency — mirrors
//! the JSONL-transcript poll the session runner already does).
//!
//! Mermaid is the default because the agent edits TEXT (fast, clean auto-layout)
//! instead of hand-computing coordinates for dozens of nodes. The UI renders the
//! source into real, editable Excalidraw elements.
//!
//! The reply is a FALLBACK source: if the agent printed a ```mermaid /```json
//! block instead of editing the file (or in the offline E2E stub, where no agent
//! runs), we take the source from the reply and write it into the file so the
//! next resumed turn sees it.
//!
//! Routes (registered in modules.rs, gated by `Feature::Canvas`):
//!   POST /api/v1/canvas/scenes/{id}/assist   (ws editor) → AssistResult
//!   POST /api/v1/canvas/assist/preview       (canvas edit) → AssistResult

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Live-preview file poll cadence while the agent edits.
const POLL: Duration = Duration::from_millis(900);

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
    /// Excalidraw element SKELETON (when the scene's format is `excalidraw`):
    /// `{ "elements": [...] }`. The app turns it into editable shapes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excalidraw: Option<Value>,
    /// A mermaid diagram source (the default format — clean auto-layout).
    pub mermaid: Option<String>,
    /// The scene's source format: `mermaid` | `excalidraw`. Lets the UI pick the
    /// render path without sniffing.
    pub format: String,
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

/// `POST /canvas/scenes/{id}/assist` — edit the scene's backing file, commit the
/// result to the scene, and broadcast it. Returns the new source so the UI can
/// render immediately (it also gets the live `CanvasUpdated` events).
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

    // Resolve the scene's current source + format from its opaque doc.
    let doc: Value = serde_json::from_str(&scene.doc_json).unwrap_or(Value::Null);
    let format = doc_format(&doc);
    let current = current_source(&doc, &format);

    // Materialize the source into the scene's own directory (the agent's cwd, so
    // a resumed session always finds the same file).
    let dir = ctx.data_dir.join("canvas").join(&scene.id);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Err(ApiError(Error::Internal(format!(
            "canvas scratch dir: {e}"
        ))));
    }
    let file_path = dir.join(file_name(&format));
    let _ = tokio::fs::write(&file_path, &current).await;
    let dir_str = dir.to_string_lossy().to_string();
    // The agent gets Edit/Write tools in this cwd; trust it so the PTY doesn't
    // stall on a first-run trust prompt (same as the orchestrate path).
    otto_sessions::trust::ensure_trusted("claude", &dir_str);

    // Live preview: broadcast each file change while the turn runs.
    let poll = spawn_file_poll(&ctx, &scene, &file_path, &format, &current);

    let prompt = build_assist_prompt(&req.prompt, &format, file_name(&format), &current);
    let meta = serde_json::json!({ "source": "canvas_assist", "scene_id": scene.id });
    // Surface the agent session the MOMENT it exists (turn start) so the Canvas
    // Assistant panel can attach the live shell immediately, not after the turn.
    let ready_events = ctx.events.clone();
    let ready_ws = scene.workspace_id.clone();
    let ready_scene = scene.id.clone();
    let on_ready = move |sid: &Id| {
        let _ = ready_events.send(Event::CanvasSessionStarted {
            workspace_id: ready_ws.clone(),
            scene_id: ready_scene.clone(),
            session_id: sid.clone(),
        });
    };
    let turn = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        scene.session_id.as_ref(),
        &format!("Canvas: {}", scene.title),
        &dir_str,
        &scene.provider,
        meta,
        &prompt,
        on_ready,
    )
    .await;
    poll.abort();
    let (raw, sid) = turn?;
    if scene.session_id.is_none() {
        let _ = ctx.canvas_repo.set_session(&scene.id, &sid).await;
    }

    // The committed source = the edited file, or the reply's block as a fallback.
    let parsed = parse_assist(&raw);
    let new_source = resolve_source(&file_path, &current, &format, &parsed).await;

    // Commit it as the scene's document + broadcast the final result.
    let new_doc = build_doc(&format, &new_source);
    let _ = ctx
        .canvas_repo
        .update(
            &scene.id,
            otto_state::SceneUpdate {
                title: None,
                doc_json: Some(new_doc.to_string()),
                thumbnail: None,
                provider: None,
                section: None,
                story_id: None,
            },
        )
        .await;
    let _ = ctx.events.send(Event::CanvasUpdated {
        workspace_id: scene.workspace_id.clone(),
        scene_id: scene.id.clone(),
        doc: new_doc,
    });

    Ok(Json(result_for(&format, &new_source, parsed.note)))
}

/// `POST /canvas/assist/preview` — generate blocks with no scene (the Discovery-
/// Chat bridge / legacy empty-canvas hero). Runs a THROWAWAY session and parses
/// its reply (no file to persist — there's no scene to own one). Gated upstream
/// by the `Feature::Canvas` edit capability.
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

    let prompt = build_assist_prompt(&req.prompt, "mermaid", "canvas.mermaid", "flowchart TD\n");
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
        |_| {},
    )
    .await?;
    let _ = ctx.manager.kill_session(&sid).await;
    let parsed = parse_assist(&raw);
    let src = parsed.mermaid.clone().unwrap_or_default();
    Ok(Json(result_for("mermaid", &src, parsed.note)))
}

// ---------------------------------------------------------------------------
// Source / doc helpers
// ---------------------------------------------------------------------------

/// The scene's format from its doc (`mermaid` default).
fn doc_format(doc: &Value) -> String {
    doc.get("format")
        .and_then(|f| f.as_str())
        .filter(|f| *f == "mermaid" || *f == "excalidraw")
        .unwrap_or("mermaid")
        .to_string()
}

/// The scene's current source text, or a minimal base for an empty/legacy doc.
fn current_source(doc: &Value, format: &str) -> String {
    doc.get("source")
        .and_then(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| base_source(format))
}

/// The starting content for a brand-new scene file.
fn base_source(format: &str) -> String {
    match format {
        "excalidraw" => {
            "{\n  \"type\": \"excalidraw\",\n  \"version\": 2,\n  \"source\": \"otto\",\n  \"elements\": []\n}\n"
                .to_string()
        }
        _ => "flowchart TD\n".to_string(),
    }
}

/// The agent-edited file's name in the scene directory.
fn file_name(format: &str) -> &'static str {
    match format {
        "excalidraw" => "canvas.json",
        _ => "canvas.mermaid",
    }
}

/// Build the opaque canvas document the UI + agent share.
fn build_doc(format: &str, source: &str) -> Value {
    serde_json::json!({
        "type": "otto-canvas",
        "version": 1,
        "format": format,
        "source": source,
    })
}

/// Decide the committed source: prefer the agent's in-place file edit; fall back
/// to a fenced block in the reply (E2E stub / agent that printed instead of
/// editing), writing it into the file so the next resumed turn sees it; else keep
/// the prior source.
async fn resolve_source(
    file_path: &std::path::Path,
    current: &str,
    format: &str,
    parsed: &AssistResult,
) -> String {
    let after = tokio::fs::read_to_string(file_path).await.unwrap_or_default();
    if !after.trim().is_empty() && after.trim() != current.trim() {
        return after;
    }
    let from_reply = match format {
        "excalidraw" => parsed.excalidraw.as_ref().map(|v| v.to_string()),
        _ => parsed.mermaid.clone(),
    };
    match from_reply {
        Some(s) if !s.trim().is_empty() => {
            let _ = tokio::fs::write(file_path, &s).await;
            s
        }
        _ => current.to_string(),
    }
}

/// Shape the API result from the committed source.
fn result_for(format: &str, source: &str, note: String) -> AssistResult {
    let mut r = AssistResult {
        format: format.to_string(),
        note,
        ..Default::default()
    };
    match format {
        "excalidraw" => r.excalidraw = serde_json::from_str(source).ok(),
        _ => r.mermaid = Some(source.to_string()),
    }
    r
}

/// Spawn a background task that broadcasts `CanvasUpdated` on each file change
/// while the agent edits, for a live "draws itself" preview. Aborted when the
/// turn returns.
fn spawn_file_poll(
    ctx: &ServerCtx,
    scene: &otto_state::CanvasScene,
    file_path: &std::path::Path,
    format: &str,
    base: &str,
) -> tokio::task::JoinHandle<()> {
    let events = ctx.events.clone();
    let workspace_id = scene.workspace_id.clone();
    let scene_id = scene.id.clone();
    let path = file_path.to_path_buf();
    let format = format.to_string();
    let mut last = base.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if content != last && !content.trim().is_empty() {
                    last = content.clone();
                    let _ = events.send(Event::CanvasUpdated {
                        workspace_id: workspace_id.clone(),
                        scene_id: scene_id.clone(),
                        doc: build_doc(&format, &content),
                    });
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Prompt (unit-tested, no DB / no agent)
// ---------------------------------------------------------------------------

/// Build the file-edit prompt. The `OTTO_TASK: canvas_assist` sentinel routes the
/// deterministic E2E stub; the rest instructs the real agent to edit the file.
fn build_assist_prompt(user_prompt: &str, format: &str, file: &str, current: &str) -> String {
    if format == "excalidraw" {
        return format!(
            "OTTO_TASK: canvas_assist\n\
             You are drawing on an EXCALIDRAW canvas by EDITING the file `{file}` in your working \
             directory. WRITE THE COMPLETE diagram each time as \
             `{{\"type\":\"excalidraw\",\"elements\":[ ... ]}}` using ONLY the SIMPLIFIED element \
             form below — the app expands it into a real Excalidraw scene (binds labels, ROUTES \
             arrows). Re-express any existing elements in this simplified form + apply the change.\n\n\
             CRITICAL — write EVERY element simplified. NEVER include `seed`, `versionNonce`, \
             `version`, `index`, `updated`, `boundElements`, `containerId`, or arrow `points`/\
             `x`/`y`. Put a shape's text in its own `label`; put an arrow's text in the ARROW's \
             `label`. Do NOT create separate `text` elements for shape/arrow labels (that scatters \
             them). Valid JSON only.\n\n\
             SIMPLIFIED ELEMENTS:\n\
             - Shape: {{\"type\":\"rectangle\"|\"ellipse\"|\"diamond\",\"id\":\"n1\",\"x\":int,\"y\":int,\
             \"width\":int,\"height\":int,\"backgroundColor\":\"#hex\",\"strokeColor\":\"#hex\",\
             \"fillStyle\":\"solid\",\"roundness\":{{\"type\":3}},\
             \"label\":{{\"text\":\"...\",\"fontSize\":16,\"fontFamily\":2}}}}\n\
             - Arrow (NO coordinates — routed by node id): {{\"type\":\"arrow\",\
             \"start\":{{\"id\":\"n1\"}},\"end\":{{\"id\":\"n2\"}},\"strokeColor\":\"#94a3b8\",\
             \"label\":{{\"text\":\"yes\"}}}}\n\
             - Standalone caption only (not a shape/arrow label): {{\"type\":\"text\",\"x\":int,\
             \"y\":int,\"text\":\"...\",\"fontSize\":20}}\n\
             - fontFamily 3 = code (monospace); for a CODE BLOCK use a rectangle backgroundColor \
             #0f172a + a fontFamily:3 label with the real code (\\n between lines).\n\n\
             LAYOUT: every shape needs explicit x/y; lay nodes left→right or top→down, ~80px \
             apart, NO overlaps, unique id per node, size boxes to their text (width ~= 28 + \
             9*chars, height ~= 28 + 22*lines), colour-code by role (start green, process indigo, \
             decision amber DIAMOND, error red), prefix labels with a fitting emoji.\n\n\
             The file currently contains:\n{current}\n\n\
             Reply with ONE short sentence describing what you changed.\n\n\
             Request: {user_prompt}\n"
        );
    }
    format!(
        "OTTO_TASK: canvas_assist\n\
         You are drawing a diagram by EDITING the MERMAID file `{file}` in your working directory. \
         Read it, make the requested change IN PLACE, and save it. Keep refining this SAME file \
         across the conversation. The file must always hold ONE COMPLETE, valid Mermaid diagram \
         (no ``` fences inside the file).\n\n\
         Pick the BEST diagram type for the request: `flowchart TD`/`LR` for processes & \
         architecture, `sequenceDiagram` for call/message flows, `classDiagram` for data models / \
         UML, `erDiagram` for schemas, `stateDiagram-v2` for state machines.\n\n\
         STYLE — clean + presentation-grade (flowcharts especially):\n\
         - Short labels with a leading emoji icon, e.g. `A[\"🚀 Start\"]`.\n\
         - Decisions are rhombus nodes `B{{\"❓ Valid?\"}}` with LABELLED edges `B -->|yes| C` / \
         `B -->|no| E`; include error/retry paths.\n\
         - Group related steps with `subgraph` lanes (Client / API / Data).\n\
         - Colour-code flowchart nodes with classDef + class AT THE END:\n\
         `classDef start fill:#dcfce7,stroke:#16a34a,color:#064e3b;`\n\
         `classDef process fill:#eef2ff,stroke:#6366f1,color:#1e1b4b;`\n\
         `classDef decision fill:#fef9c3,stroke:#ca8a04,color:#422006;`\n\
         `classDef data fill:#ecfeff,stroke:#0891b2,color:#083344;`\n\
         `classDef error fill:#fee2e2,stroke:#dc2626,color:#7f1d1d;`\n\
         then assign with `class A,B start;`.\n\
         - Be accurate but keep node text short. Valid Mermaid only.\n\n\
         The file currently contains:\n{current}\n\n\
         Reply with ONE short sentence describing what you changed.\n\n\
         Request: {user_prompt}\n"
    )
}

/// Parse an assist reply: a ```mermaid fence wins; otherwise an Excalidraw
/// `{elements}` (or `{nodes,edges}`) JSON object; otherwise the raw text becomes
/// the note. Used as the reply FALLBACK source. Never panics.
fn parse_assist(raw: &str) -> AssistResult {
    if let Some(src) = extract_fenced(raw, "mermaid") {
        return AssistResult {
            mermaid: Some(src),
            note: prose_before_fence(raw),
            ..Default::default()
        };
    }
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
        "Updated the canvas.".to_string()
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
    fn prompt_has_sentinel_and_file() {
        let p = build_assist_prompt("a login flow", "mermaid", "canvas.mermaid", "flowchart TD\n");
        assert!(p.contains("OTTO_TASK: canvas_assist"));
        assert!(p.contains("canvas.mermaid"));
        assert!(p.contains("MERMAID file"));
        // Mermaid mode must offer the major diagram types.
        assert!(p.contains("sequenceDiagram"));
        assert!(p.contains("classDiagram"));
        assert!(p.contains("a login flow"));
    }

    #[test]
    fn excalidraw_prompt_points_at_json_file() {
        let p = build_assist_prompt("arch", "excalidraw", "canvas.json", "{}");
        assert!(p.contains("canvas.json"));
        assert!(p.contains("SIMPLIFIED ELEMENTS"));
        assert!(p.contains("EXCALIDRAW canvas"));
    }

    #[test]
    fn base_and_format_defaults() {
        assert_eq!(doc_format(&Value::Null), "mermaid");
        assert_eq!(doc_format(&serde_json::json!({"format":"excalidraw"})), "excalidraw");
        // unknown format → default
        assert_eq!(doc_format(&serde_json::json!({"format":"weird"})), "mermaid");
        assert!(base_source("mermaid").contains("flowchart"));
        assert!(base_source("excalidraw").contains("elements"));
    }

    #[test]
    fn current_source_prefers_doc_then_base() {
        let doc = serde_json::json!({"type":"otto-canvas","format":"mermaid","source":"flowchart LR\n  A-->B"});
        assert_eq!(current_source(&doc, "mermaid"), "flowchart LR\n  A-->B");
        // empty / missing → base
        assert_eq!(current_source(&serde_json::json!({"source":"  "}), "mermaid"), base_source("mermaid"));
    }

    #[test]
    fn result_for_routes_by_format() {
        let m = result_for("mermaid", "flowchart TD\n  A-->B", "done".into());
        assert_eq!(m.mermaid.as_deref(), Some("flowchart TD\n  A-->B"));
        assert!(m.excalidraw.is_none());
        assert_eq!(m.format, "mermaid");
        assert_eq!(m.note, "done");

        let x = result_for("excalidraw", "{\"elements\":[{\"type\":\"rectangle\"}]}", "ok".into());
        assert!(x.excalidraw.is_some());
        assert!(x.mermaid.is_none());
        assert_eq!(x.format, "excalidraw");
    }

    #[test]
    fn parse_prefers_mermaid_fence() {
        let raw = "Here you go.\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n```";
        let r = parse_assist(raw);
        assert_eq!(r.mermaid.as_deref(), Some("sequenceDiagram\n  A->>B: hi"));
        assert!(r.excalidraw.is_none());
        assert_eq!(r.note, "Here you go.");
    }

    #[test]
    fn parse_excalidraw_elements() {
        let raw = "Drawn.\n\n```json\n{\"elements\":[{\"type\":\"rectangle\",\"id\":\"a\",\"x\":0,\"y\":0}]}\n```";
        let r = parse_assist(raw);
        assert!(r.excalidraw.is_some());
        assert!(r.mermaid.is_none());
        assert_eq!(r.note, "Drawn.");
    }

    #[test]
    fn parse_unstructured_keeps_note() {
        let raw = "I couldn't draw that, here's why...";
        let r = parse_assist(raw);
        assert!(r.mermaid.is_none());
        assert!(r.nodes.is_empty());
        assert_eq!(r.note, raw);
    }

    #[tokio::test]
    async fn resolve_prefers_edited_file_then_reply() {
        let dir = std::env::temp_dir().join(format!("otto-canvas-test-{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("canvas.mermaid");

        // Agent edited the file → use the file.
        tokio::fs::write(&path, "flowchart TD\n  A-->B\n  B-->C").await.unwrap();
        let parsed = AssistResult::default();
        let got = resolve_source(&path, "flowchart TD\n", "mermaid", &parsed).await;
        assert!(got.contains("B-->C"));

        // File unchanged (== base) → fall back to the reply, and write it back.
        tokio::fs::write(&path, "flowchart TD\n").await.unwrap();
        let parsed = AssistResult {
            mermaid: Some("flowchart LR\n  X-->Y".into()),
            ..Default::default()
        };
        let got = resolve_source(&path, "flowchart TD\n", "mermaid", &parsed).await;
        assert!(got.contains("X-->Y"));
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(on_disk.contains("X-->Y"), "reply source written back to file");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

//! Agent-assisted canvas drawing — FILE-BACKED.
//!
//! Each scene has a persistent source file the agent EDITS across the
//! conversation (a `canvas.mermaid` by default, or `canvas.excalidraw.json`,
//! or `canvas.d2`), kept in an Otto-owned per-scene directory. An "Ask AI" turn:
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
//! The reply is a FALLBACK source: if the agent printed a fenced `d2`/`mermaid`/
//! `json` block instead of editing the file (or in the offline E2E stub, where
//! no agent runs), we take the source from the reply and write it into the file
//! so the next resumed turn sees it.
//!
//! Routes (mounted by otto-server's modules.rs, gated by `Feature::Canvas`):
//!   POST /api/v1/canvas/scenes/{id}/assist   (ws editor) → AssistResult
//!   POST /api/v1/canvas/assist/preview       (canvas edit) → AssistResult

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::cancel_signal::{InFlightGuard, InFlightSet, RunCancelGuard, RunCancels};
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assist_ctx::{AgentTurn, ApiError, ApiResult, CanvasAssistCtx, CurrentUser};
use otto_agent_run::fence::extract_fenced;
use otto_design::format::excalidraw::{merge_excalidraw, split_excalidraw, ExcalidrawKeep};

/// Live-preview file poll cadence while the agent edits.
const POLL: Duration = Duration::from_millis(900);
/// The idle trip a canvas turn reports (the host's `STUCK_IDLE`, 10 h: an
/// interactive edit is never cut short for being quiet).
const CANVAS_STUCK_AFTER: Duration = Duration::from_secs(10 * 60 * 60);

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
    /// A D2 diagram source (when the scene's format is `d2`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d2: Option<String>,
    /// The scene's source format: `mermaid` | `excalidraw` | `d2`. Lets the UI
    /// pick the render path without sniffing.
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
pub async fn assist_scene<C: CanvasAssistCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AssistReq>,
) -> ApiResult<Json<AssistResult>> {
    let scene = ctx
        .canvas_repo()
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("canvas scene {id}"))))?;
    crate::assist_ctx::require_ws_role(&ctx, &user, &scene.workspace_id, WorkspaceRole::Editor)
        .await?;
    let ws = ctx
        .workspaces()
        .get(&scene.workspace_id)
        .await
        .map_err(ApiError)?;

    // Resolve the scene's current source + format from its opaque doc.
    let doc: Value = serde_json::from_str(&scene.doc_json).unwrap_or(Value::Null);
    let format = doc_format(&doc);
    let current = current_source(&doc, &format);

    // Materialize the source into the scene's own directory (the agent's cwd, so
    // a resumed session always finds the same file). Scene ids are daemon-minted,
    // but the id arrived as a route param — confine the join under the canvas
    // root so a hostile id can't steer the fs ops (rust/path-injection).
    let dir = otto_core::paths::confine_join(&ctx.data_dir().join("canvas"), &scene.id)
        .ok_or_else(|| {
            ApiError(Error::Invalid(format!(
                "unsafe canvas scene id {}",
                scene.id
            )))
        })?;
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Err(ApiError(Error::Internal(format!(
            "canvas scratch dir: {e}"
        ))));
    }
    let file_path = dir.join(file_name(&format));
    // On an Excalidraw board the agent only sees (and rewrites) what it can
    // express — shapes, id-routed arrows, text. Images, freehand, lines, frames
    // and the base64 `files` map are set aside and merged back into whatever
    // it writes (C2), so an Ask AI turn can no longer erase them — and the
    // prompt no longer carries megabytes of inlined image data.
    let (agent_view, keep) = if format == "excalidraw" {
        split_excalidraw(&current)
    } else {
        (current.clone(), ExcalidrawKeep::default())
    };
    let busy = prepare_scene_file(&scene.id, &file_path, &agent_view).await?;
    let dir_str = dir.to_string_lossy().to_string();
    // The agent gets Edit/Write tools in this cwd; trust it so the PTY doesn't
    // stall on a first-run trust prompt (same as the orchestrate path). Trust the
    // SCENE's provider — a non-claude provider must trust the dir it will run in.
    ctx.ensure_trusted(&scene.provider, &dir_str);

    // Live preview: broadcast each file change while the turn runs.
    // Abort-on-drop (S4-12): the turn runs inside the HTTP request, and a
    // dropped request future (client disconnect / reload) never reached the
    // `abort()` after the turn — the poller then leaked forever, re-broadcasting
    // this PRE-turn doc on every later change of the file.
    let poll = AbortOnDrop(spawn_file_poll(
        &ctx,
        &scene,
        &doc,
        &file_path,
        &format,
        &agent_view,
        &keep,
    ));

    let prompt = build_assist_prompt(&req.prompt, &format, file_name(&format), &agent_view);
    let meta = serde_json::json!({ "source": "canvas_assist", "scene_id": scene.id });
    // Surface the agent session the MOMENT it exists (turn start) so the Canvas
    // Assistant panel can attach the live shell immediately, not after the turn.
    let ready_events = ctx.events().clone();
    let ready_ws = scene.workspace_id.clone();
    let ready_scene = scene.id.clone();
    let on_ready = move |sid: &Id| {
        let _ = ready_events.send(Event::CanvasSessionStarted {
            workspace_id: ready_ws.clone(),
            scene_id: ready_scene.clone(),
            session_id: sid.clone(),
        });
    };
    let turn = ctx
        .run_agent_turn(
            AgentTurn {
                ws: &ws,
                user: &user,
                existing: scene.session_id.as_ref(),
                title: &format!("Canvas: {}", scene.title),
                cwd: &dir_str,
                provider: &scene.provider,
                meta,
                prompt: &prompt,
                // The canvas host completes on idle (`STUCK_IDLE`); the
                // shared turn type's done-marker fields stay unset.
                stuck_after: CANVAS_STUCK_AFTER,
                done_file: None,
                quiet_done: None,
            },
            on_ready,
        )
        .await;
    drop(poll);
    let (raw, sid) = turn?;
    remember_session(ctx.canvas_repo(), &scene, &sid).await;
    if busy.stopped() {
        let (current_doc, result) = stopped_document(ctx.canvas_repo(), &scene.id, &dir).await?;
        let _ = ctx.events().send(Event::CanvasUpdated {
            workspace_id: scene.workspace_id.clone(),
            scene_id: scene.id.clone(),
            doc: current_doc,
        });
        return Ok(Json(result));
    }

    // The committed source = the edited file, or the reply's block as a fallback.
    let parsed = parse_assist(&raw);
    let resolved = resolve_source(&file_path, &agent_view, &format, &parsed).await?;
    // Untouched view → keep the ORIGINAL source (full elements + files);
    // otherwise fold the set-aside elements back into the agent's scene.
    let new_source = if resolved.trim() == agent_view.trim() {
        current.clone()
    } else {
        merge_excalidraw(&resolved, &keep)
    };

    // Commit it as the scene's document + broadcast the final result.
    // Guarded by the PRE-TURN `updated_at`: an agent turn can run for minutes,
    // and a bare update would silently clobber anything the user saved while
    // it ran. On conflict, overwrite only when the user's save left the source
    // identical to the pre-turn snapshot (title/section-only edit); otherwise
    // KEEP the user's version and say so instead of losing their work.
    let mut committed_doc = build_doc(&doc, &format, &new_source);
    let mut note = parsed.note;
    // Version history (C5): keep the board as it was before this turn so a bad
    // turn is one "Restore" away. Best-effort — never blocks the commit.
    if new_source != current {
        let _ = ctx
            .canvas_repo()
            .snapshot(&scene.id, "agent", Some(&user.id), None)
            .await;
    }
    committed_doc = commit_assist_doc(
        ctx.canvas_repo(),
        &scene,
        &current,
        committed_doc,
        &mut note,
    )
    .await?;
    let final_source = current_source(&committed_doc, &doc_format(&committed_doc));
    let _ = ctx.events().send(Event::CanvasUpdated {
        workspace_id: scene.workspace_id.clone(),
        scene_id: scene.id.clone(),
        doc: committed_doc,
    });

    Ok(Json(result_for(&format, &final_source, note)))
}

/// Cancellation discards only the agent draft, never a saved human edit.
async fn stopped_document(
    repo: &otto_state::CanvasRepo,
    id: &Id,
    dir: &std::path::Path,
) -> ApiResult<(Value, AssistResult)> {
    let fresh = repo
        .portable_scene(id)
        .await?
        .ok_or_else(|| ApiError(Error::NotFound("canvas scene".into())))?;
    let doc: Value = serde_json::from_str(&fresh.doc_json).map_err(Error::from)?;
    let format = doc_format(&doc);
    let source = current_source(&doc, &format);
    tokio::fs::write(dir.join(file_name(&format)), &source)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("restore stopped canvas: {e}"))))?;
    Ok((
        doc,
        result_for(
            &format,
            &source,
            "Stopped — your saved changes were preserved.".into(),
        ),
    ))
}

/// Materialize only while owning the scene's entire turn, including preparation.
async fn prepare_scene_file(
    scene: &Id,
    path: &std::path::Path,
    source: &str,
) -> ApiResult<SceneBusy> {
    let busy = SceneBusy::claim(scene).ok_or_else(|| {
        ApiError(Error::Conflict(
            "an Ask AI turn is already running on this canvas".into(),
        ))
    })?;
    tokio::fs::write(path, source)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write canvas source: {e}"))))?;
    Ok(busy)
}

/// Publish only the document actually accepted by optimistic concurrency.
async fn commit_assist_doc(
    repo: &otto_state::CanvasRepo,
    scene: &otto_state::CanvasScene,
    current: &str,
    mut committed_doc: Value,
    note: &mut String,
) -> ApiResult<Value> {
    let commit = repo
        .update(
            &scene.id,
            otto_state::SceneUpdate {
                doc_json: Some(committed_doc.to_string()),
                expect_updated_at: Some(scene.updated_at),
                ..Default::default()
            },
        )
        .await;
    match commit {
        Ok(_) => return Ok(committed_doc),
        Err(Error::Conflict(_)) => {}
        Err(error) => return Err(ApiError(error)),
    }
    let fresh = repo
        .get(&scene.id)
        .await?
        .ok_or_else(|| ApiError(Error::NotFound("canvas scene".into())))?;
    let fresh_doc: Value = serde_json::from_str(&fresh.doc_json).map_err(Error::from)?;
    let format = doc_format(&committed_doc);
    let fresh_src = current_source(&fresh_doc, &doc_format(&fresh_doc));
    if fresh_src == current && doc_format(&fresh_doc) == format {
        // Merge onto the latest metadata; a second conflict is an error, never
        // a successful broadcast of content that was not persisted.
        committed_doc = build_doc(
            &fresh_doc,
            &format,
            &current_source(&committed_doc, &format),
        );
        repo.update(
            &scene.id,
            otto_state::SceneUpdate {
                doc_json: Some(committed_doc.to_string()),
                expect_updated_at: Some(fresh.updated_at),
                ..Default::default()
            },
        )
        .await?;
    } else {
        committed_doc = fresh_doc;
        let kept = "Kept your manual edits saved during the turn; the agent's version was NOT applied. Re-run Ask AI to regenerate on top of your changes.";
        *note = if note.is_empty() {
            kept.to_string()
        } else {
            format!("{kept}\n\n{note}")
        };
    }
    Ok(committed_doc)
}

async fn remember_session(
    repo: &otto_state::CanvasRepo,
    scene: &otto_state::CanvasScene,
    sid: &Id,
) {
    if scene.session_id.as_ref() != Some(sid) {
        let _ = repo.set_session(&scene.id, sid).await;
    }
}

/// `POST /canvas/assist/preview` — generate blocks with no scene (the Discovery-
/// Chat bridge / legacy empty-canvas hero). Runs a THROWAWAY session and parses
/// its reply (no file to persist — there's no scene to own one). Gated upstream
/// by the `Feature::Canvas` edit capability.
pub async fn assist_preview<C: CanvasAssistCtx>(
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AssistReq>,
) -> ApiResult<Json<AssistResult>> {
    let ws_id = req.workspace_id.clone().ok_or_else(|| {
        ApiError(Error::Invalid(
            "workspace_id is required for preview".into(),
        ))
    })?;
    crate::assist_ctx::require_ws_role(
        &ctx,
        &user,
        &Id::from(ws_id.clone()),
        WorkspaceRole::Editor,
    )
    .await?;
    let ws = ctx
        .workspaces()
        .get(&Id::from(ws_id))
        .await
        .map_err(ApiError)?;

    let prompt = build_assist_prompt(&req.prompt, "mermaid", "canvas.mermaid", "flowchart TD\n");
    let meta = serde_json::json!({ "source": "canvas_assist_preview" });
    // S4-11: a per-call scratch dir — never the user's repo (`ws.root_path`):
    // the prompt tells the agent to EDIT `canvas.mermaid` in its cwd, which
    // left a stray file in the repo root, and a full-tool session has no
    // business in the user's checkout for a text-only preview. The guard
    // removes the dir and kills the session even when the request is dropped.
    let preview_root = ctx.data_dir().join("canvas").join("preview");
    let scratch = preview_root.join(otto_core::new_id());
    tokio::fs::create_dir_all(&scratch)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("canvas preview dir: {e}"))))?;
    let cleanup = PreviewCleanup {
        ctx: ctx.clone(),
        dir: scratch.clone(),
        sid: Arc::new(std::sync::Mutex::new(None)),
    };
    let cwd = scratch.to_string_lossy().to_string();
    // No scene here (throwaway preview) → no scene provider to honor. Run under the
    // user's workspace/global default agent rather than hard-coding claude, and pre-trust the
    // cwd so a non-claude provider doesn't stall on the first-run trust prompt.
    let provider = ctx
        .resolve_provider(Some(&ws), None)
        .await
        .map_err(ApiError)?;
    // Trust the stable preview ROOT, not each fresh dir (S4-303): one entry
    // per dead preview dir piled up in `~/.claude.json` forever.
    ctx.ensure_trusted_scratch(&provider, &preview_root, &cwd);
    let sid_slot = Arc::clone(&cleanup.sid);
    let (raw, _sid) = ctx
        .run_agent_turn(
            AgentTurn {
                ws: &ws,
                user: &user,
                existing: None,
                title: "Canvas: preview",
                cwd: &cwd,
                provider: &provider,
                meta,
                prompt: &prompt,
                // The canvas host completes on idle (`STUCK_IDLE`); the
                // shared turn type's done-marker fields stay unset.
                stuck_after: CANVAS_STUCK_AFTER,
                done_file: None,
                quiet_done: None,
            },
            move |sid: &Id| {
                *sid_slot.lock().unwrap() = Some(sid.clone());
            },
        )
        .await?;
    drop(cleanup); // kills the throwaway session + removes the scratch dir
    let parsed = parse_assist(&raw);
    let src = parsed.mermaid.clone().unwrap_or_default();
    Ok(Json(result_for("mermaid", &src, parsed.note)))
}

// ---------------------------------------------------------------------------
// Drop guards
// ---------------------------------------------------------------------------

/// Aborts the wrapped task when dropped (a `JoinHandle` drop only detaches).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Scenes with an assist turn in flight (S4-20): the exclusive slot and the
/// turn's stop signal come from the shared run bookkeeping in
/// `otto_core::cancel_signal` (architecture-4-02), not a hand-rolled map.
fn busy_scenes() -> &'static InFlightSet {
    static BUSY: std::sync::OnceLock<InFlightSet> = std::sync::OnceLock::new();
    BUSY.get_or_init(Default::default)
}

fn scene_stops() -> &'static RunCancels {
    static STOPS: std::sync::OnceLock<RunCancels> = std::sync::OnceLock::new();
    STOPS.get_or_init(Default::default)
}

/// A claimed per-scene assist slot (with the turn's stop signal); released on
/// drop. Field order matters: the stop registration is removed BEFORE the
/// slot frees, so a later turn's registration is never the one dropped.
struct SceneBusy {
    stop: RunCancelGuard,
    _slot: InFlightGuard,
}

impl SceneBusy {
    fn claim(scene: &Id) -> Option<Self> {
        let slot = busy_scenes().claim(scene)?;
        Some(Self {
            stop: scene_stops().register(scene),
            _slot: slot,
        })
    }

    /// True once [`request_stop`] flagged this turn.
    fn stopped(&self) -> bool {
        self.stop.signal.is_cancelled()
    }
}

/// Flag the scene's in-flight assist turn as stopped. Returns false when no
/// turn is running.
fn request_stop(scene: &Id) -> bool {
    scene_stops().cancel(scene)
}

/// `POST /canvas/scenes/{id}/assist/stop` — mark the scene's running Ask AI
/// turn as stopped (S4-20). The UI interrupts the agent's PTY separately
/// (Esc keeps the session resumable); when the turn then settles, a stopped
/// turn commits NOTHING — the board, its backing file and every viewer are
/// restored to the pre-turn document instead of a half-written diagram.
/// `{ stopping: false }` when no turn is running.
pub async fn stop_assist<C: CanvasAssistCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let scene = ctx
        .canvas_repo()
        .get(&id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("canvas scene {id}"))))?;
    crate::assist_ctx::require_ws_role(&ctx, &user, &scene.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(
        serde_json::json!({ "stopping": request_stop(&scene.id) }),
    ))
}

/// Throwaway-preview teardown (S4-11): kill the session (once `on_ready` saw
/// it) and remove the scratch dir — on success, error, or a dropped request.
struct PreviewCleanup<C: CanvasAssistCtx> {
    ctx: C,
    dir: std::path::PathBuf,
    sid: Arc<std::sync::Mutex<Option<Id>>>,
}

impl<C: CanvasAssistCtx> Drop for PreviewCleanup<C> {
    // The recursive delete leaves the async workers when dropped on one (S9-305).
    #[allow(clippy::disallowed_methods)] // sync fallback outside a runtime, or inside spawn_blocking
    fn drop(&mut self) {
        let dir = std::mem::take(&mut self.dir);
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn_blocking(move || {
                    let _ = std::fs::remove_dir_all(&dir);
                });
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        if let Some(sid) = self.sid.lock().unwrap().take() {
            let ctx = self.ctx.clone();
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    let _ = ctx.kill_session(&sid).await;
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Source / doc helpers
// ---------------------------------------------------------------------------

/// The scene's format from its doc (`mermaid` default).
fn doc_format(doc: &Value) -> String {
    doc.get("format")
        .and_then(|f| f.as_str())
        .filter(|f| *f == "mermaid" || *f == "excalidraw" || *f == "d2")
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
        "d2" => "direction: right\n".to_string(),
        _ => "flowchart TD\n".to_string(),
    }
}

/// The agent-edited file's name in the scene directory.
fn file_name(format: &str) -> &'static str {
    match format {
        "excalidraw" => "canvas.json",
        "d2" => "canvas.d2",
        _ => "canvas.mermaid",
    }
}

/// Build the opaque canvas document the UI + agent share. Starts from the
/// scene's EXISTING doc and overwrites only `format`/`source` (C8), so
/// per-format extras — D2 `sketch`, Mermaid `positions`, the title — survive an
/// agent turn instead of being reset.
fn build_doc(base: &Value, format: &str, source: &str) -> Value {
    let mut doc = match base {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    doc.entry("type")
        .or_insert_with(|| Value::from("otto-canvas"));
    doc.entry("version").or_insert_with(|| Value::from(1));
    doc.insert("format".into(), Value::from(format));
    doc.insert("source".into(), Value::from(source));
    Value::Object(doc)
}

/// Above this size the prompt points at the file instead of inlining it.
const INLINE_SOURCE_MAX: usize = 12_000;

/// The file content quoted into the prompt — or, when it's large, a pointer
/// to read it (the agent has the file in its cwd either way).
fn inline_source(current: &str) -> String {
    if current.len() <= INLINE_SOURCE_MAX {
        current.to_string()
    } else {
        format!(
            "(large — {} bytes; READ the file before editing it)",
            current.len()
        )
    }
}

/// Agent files obey the same byte budget as the scene upload surface.
async fn read_agent_source(path: &std::path::Path) -> otto_core::Result<String> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::Internal(format!("open canvas source: {e}")))?;
    let mut bytes = Vec::new();
    file.take(crate::http::SCENE_BODY_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| Error::Internal(format!("read canvas source: {e}")))?;
    if bytes.len() > crate::http::SCENE_BODY_LIMIT {
        return Err(Error::PayloadTooLarge(
            "canvas source exceeds scene byte limit".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|e| Error::Invalid(format!("canvas source must be UTF-8: {e}")))
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
) -> otto_core::Result<String> {
    let after = read_agent_source(file_path).await?;
    if !after.trim().is_empty() && after.trim() != current.trim() {
        return Ok(after);
    }
    let from_reply = match format {
        "excalidraw" => parsed.excalidraw.as_ref().map(|v| v.to_string()),
        "d2" => parsed.d2.clone(),
        _ => parsed.mermaid.clone(),
    };
    match from_reply {
        Some(s) if !s.trim().is_empty() => {
            if s.len() > crate::http::SCENE_BODY_LIMIT {
                return Err(Error::PayloadTooLarge(
                    "canvas source exceeds scene byte limit".into(),
                ));
            }
            tokio::fs::write(file_path, &s)
                .await
                .map_err(|e| Error::Internal(format!("write canvas source: {e}")))?;
            Ok(s)
        }
        _ => Ok(current.to_string()),
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
        "d2" => r.d2 = Some(source.to_string()),
        _ => r.mermaid = Some(source.to_string()),
    }
    r
}

/// Spawn a background task that broadcasts `CanvasUpdated` on each file change
/// while the agent edits, for a live "draws itself" preview. Aborted when the
/// turn returns.
fn spawn_file_poll<C: CanvasAssistCtx>(
    ctx: &C,
    scene: &otto_state::CanvasScene,
    doc: &Value,
    file_path: &std::path::Path,
    format: &str,
    base: &str,
    keep: &ExcalidrawKeep,
) -> tokio::task::JoinHandle<()> {
    let doc = doc.clone();
    let keep = keep.clone();
    let events = ctx.events().clone();
    let workspace_id = scene.workspace_id.clone();
    let scene_id = scene.id.clone();
    let path = file_path.to_path_buf();
    let format = format.to_string();
    let mut last = base.to_string();
    tokio::spawn(async move {
        // Stat-gated: an unchanged (len, mtime) skips the read entirely; the
        // same byte budget as `read_agent_source` (over it: skipped).
        let mut gate = otto_agent_run::live_file::StatGate::default();
        loop {
            tokio::time::sleep(POLL).await;
            let otto_agent_run::live_file::Poll::Changed(bytes) =
                gate.poll(&path, crate::http::SCENE_BODY_LIMIT).await
            else {
                continue;
            };
            if let Ok(content) = String::from_utf8(bytes) {
                if content != last && !content.trim().is_empty() {
                    last = content.clone();
                    let _ = events.send(Event::CanvasUpdated {
                        workspace_id: workspace_id.clone(),
                        scene_id: scene_id.clone(),
                        // Merge the set-aside elements back in so the live
                        // preview (which the editor may autosave) never shows
                        // the board without its images/freehand.
                        doc: build_doc(&doc, &format, &merge_excalidraw(&content, &keep)),
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
    let shown = inline_source(current);
    if format == "excalidraw" {
        return format!(
            "OTTO_TASK: canvas_assist\n\
             You are drawing on an EXCALIDRAW canvas by EDITING the file `{file}` in your working \
             directory. WRITE THE COMPLETE diagram each time as \
             `{{\"type\":\"excalidraw\",\"elements\":[ ... ]}}` using ONLY the SIMPLIFIED element \
             form below — the app expands it into a real Excalidraw scene (binds labels, ROUTES \
             arrows). Re-express any existing elements in this simplified form + apply the change. \
             Images, freehand strokes, lines and frames on the board are NOT in the file — the app \
             keeps them automatically; never try to recreate them.\n\n\
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
             The file currently contains:\n{shown}\n\n\
             Reply with ONE short sentence describing what you changed.\n\n\
             Request: {user_prompt}\n"
        );
    }
    if format == "d2" {
        return format!(
            "OTTO_TASK: canvas_assist\n\
             You are drawing a diagram by EDITING the D2 file `{file}` in your working directory. \
             Read it, make the requested change IN PLACE, and save it. Keep refining this SAME file \
             across the conversation. The file must always hold ONE COMPLETE, valid D2 diagram (no \
             ``` fences inside the file).\n\n\
             D2 SYNTAX — the key shapes you'll need:\n\
             - Containers (nesting): `server: {{ api; db }}` — a shape becomes a container the \
             moment it has children; reference nested shapes with dotted paths (`server.api`).\n\
             - Edges + labels: `a -> b: label`; chained edges: `a -> b -> c`.\n\
             - Layout direction: `direction: right` (pipelines/architecture) or `direction: down` \
             (default, hierarchies).\n\
             - Shapes: `shape: sql_table` (schemas — with typed rows: `users: {{ shape: sql_table; \
             id: int; email: string }}`), `shape: sequence_diagram` (message flows), `shape: cylinder` \
             (data stores), `shape: queue` (message queues), `shape: person` (actors), plus \
             `circle`/`diamond`/`hexagon`/`cloud`/`page`/`step` for everything else.\n\
             - Classes (reusable styles): `classes: {{ critical: {{ style: {{ fill: \"#fee2e2\"; \
             stroke: \"#dc2626\" }} }} }}` then apply with `db.class: critical`.\n\
             - Inline styling: `style.fill`, `style.stroke`, `style.font-color` on any shape.\n\
             - Icons (optional): `icon: https://icons.terrastruct.com/...` alongside a shape.\n\
             - Layout hints: `near: top-right` pins a shape; `grid-rows`/`grid-columns` on a \
             container lay its children in a grid.\n\n\
             Be accurate but keep labels short. Valid D2 only.\n\n\
             The file currently contains:\n{shown}\n\n\
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
         The file currently contains:\n{shown}\n\n\
         Reply with ONE short sentence describing what you changed.\n\n\
         Request: {user_prompt}\n"
    )
}

/// Parse an assist reply: a ```d2 fence wins, then ```mermaid; otherwise an
/// Excalidraw `{elements}` (or `{nodes,edges}`) JSON object; otherwise the raw
/// text becomes the note. Used as the reply FALLBACK source. Never panics.
fn parse_assist(raw: &str) -> AssistResult {
    if let Some(src) = extract_fenced(raw, "d2") {
        return AssistResult {
            d2: Some(src),
            note: prose_before_fence(raw),
            ..Default::default()
        };
    }
    if let Some(src) = extract_fenced(raw, "mermaid") {
        return AssistResult {
            mermaid: Some(src),
            note: prose_before_fence(raw),
            ..Default::default()
        };
    }
    // Only a non-empty `elements` or `nodes` object is a diagram: a `{}` or a
    // quoted config snippet earlier in the reply must not shadow it
    // (backend-correctness-6-04).
    let non_empty = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    if let Some(v) = otto_core::text::extract_json_where(raw, |v| {
        non_empty(v, "elements") || non_empty(v, "nodes")
    }) {
        if non_empty(&v, "elements") {
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
    use otto_design::format::excalidraw::el_id;

    /// S4-20: one assist turn per scene; the slot frees on drop.
    #[test]
    fn scene_busy_slot_is_exclusive_and_released_on_drop() {
        let id: Id = "scene-busy-test".into();
        let a = SceneBusy::claim(&id).expect("first claim");
        assert!(
            SceneBusy::claim(&id).is_none(),
            "second concurrent claim refused"
        );
        drop(a);
        assert!(SceneBusy::claim(&id).is_some(), "released on drop");
    }

    /// S4-20: Stop flags only the scene's in-flight turn; a scene with no
    /// turn reports `false`, and a later turn starts unflagged.
    #[test]
    fn stop_flags_only_the_running_turn() {
        let id: Id = "scene-stop-test".into();
        assert!(!request_stop(&id), "nothing running");
        let turn = SceneBusy::claim(&id).expect("claim");
        assert!(!turn.stopped());
        assert!(request_stop(&id));
        assert!(turn.stopped(), "the running turn sees the stop");
        assert!(!request_stop(&"other-scene".into()));
        drop(turn);
        let next = SceneBusy::claim(&id).expect("re-claim");
        assert!(!next.stopped(), "a new turn starts unflagged");
    }

    /// S4-12: dropping the guard aborts the poller (a bare JoinHandle drop
    /// only detaches it, leaking the loop for the daemon's lifetime).
    #[tokio::test]
    async fn abort_on_drop_stops_the_task() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        let g = AbortOnDrop(tokio::spawn(async move {
            let _tx = tx; // dropped only when the task ends
            std::future::pending::<()>().await;
        }));
        drop(g);
        // The sender is dropped once the aborted task is torn down.
        let r = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert!(matches!(r, Ok(None)), "task was aborted");
    }

    #[test]
    fn prompt_has_sentinel_and_file() {
        let p = build_assist_prompt(
            "a login flow",
            "mermaid",
            "canvas.mermaid",
            "flowchart TD\n",
        );
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
        assert_eq!(
            doc_format(&serde_json::json!({"format":"excalidraw"})),
            "excalidraw"
        );
        assert_eq!(doc_format(&serde_json::json!({"format":"d2"})), "d2");
        // unknown format → default
        assert_eq!(
            doc_format(&serde_json::json!({"format":"weird"})),
            "mermaid"
        );
        assert!(base_source("mermaid").contains("flowchart"));
        assert!(base_source("excalidraw").contains("elements"));
        assert!(base_source("d2").contains("direction"));
    }

    #[test]
    fn d2_prompt_points_at_d2_file() {
        let p = build_assist_prompt("payments arch", "d2", "canvas.d2", "direction: right\n");
        assert!(p.contains("OTTO_TASK: canvas_assist"));
        assert!(p.contains("canvas.d2"));
        assert!(p.contains("D2 file"));
        assert!(p.contains("sql_table")); // prompt teaches key D2 shapes
        assert!(p.contains("payments arch"));
    }

    #[test]
    fn current_source_prefers_doc_then_base() {
        let doc = serde_json::json!({"type":"otto-canvas","format":"mermaid","source":"flowchart LR\n  A-->B"});
        assert_eq!(current_source(&doc, "mermaid"), "flowchart LR\n  A-->B");
        // empty / missing → base
        assert_eq!(
            current_source(&serde_json::json!({"source":"  "}), "mermaid"),
            base_source("mermaid")
        );
    }

    #[test]
    fn result_for_routes_by_format() {
        let m = result_for("mermaid", "flowchart TD\n  A-->B", "done".into());
        assert_eq!(m.mermaid.as_deref(), Some("flowchart TD\n  A-->B"));
        assert!(m.excalidraw.is_none());
        assert_eq!(m.format, "mermaid");
        assert_eq!(m.note, "done");

        let x = result_for(
            "excalidraw",
            "{\"elements\":[{\"type\":\"rectangle\"}]}",
            "ok".into(),
        );
        assert!(x.excalidraw.is_some());
        assert!(x.mermaid.is_none());
        assert_eq!(x.format, "excalidraw");

        let d = result_for("d2", "direction: right\na -> b", "drawn".into());
        assert_eq!(d.d2.as_deref(), Some("direction: right\na -> b"));
        assert!(d.mermaid.is_none());
        assert!(d.excalidraw.is_none());
        assert_eq!(d.format, "d2");
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
    fn parse_prefers_d2_fence() {
        let raw = "Drew it.\n\n```d2\ndirection: right\na -> b: hi\n```";
        let r = parse_assist(raw);
        assert_eq!(r.d2.as_deref(), Some("direction: right\na -> b: hi"));
        assert!(r.mermaid.is_none());
        assert!(r.excalidraw.is_none());
        assert_eq!(r.note, "Drew it.");
    }

    #[test]
    fn parse_excalidraw_elements() {
        let raw = "Drawn.\n\n```json\n{\"elements\":[{\"type\":\"rectangle\",\"id\":\"a\",\"x\":0,\"y\":0}]}\n```";
        let r = parse_assist(raw);
        assert!(r.excalidraw.is_some());
        assert!(r.mermaid.is_none());
        assert_eq!(r.note, "Drawn.");
    }

    /// backend-correctness-6-04: a decoy object earlier in the reply (a quoted
    /// config, an empty `{}`) used to shadow the diagram, so the assist fell
    /// through as if nothing came back.
    #[test]
    fn parse_skips_decoy_objects_before_the_diagram() {
        let raw = "Your config {\"name\":\"svc\",\"version\":\"1.0.0\"} and {} are fine.\n\n\
                   {\"elements\":[{\"type\":\"rectangle\",\"id\":\"a\",\"x\":0,\"y\":0}]}";
        let r = parse_assist(raw);
        assert!(
            r.excalidraw.is_some(),
            "the elements object wins over the decoys"
        );

        let raw = "Decoy {\"nodes\":[]} first, then {\"nodes\":[{\"id\":\"a\"}],\"edges\":[]}";
        let r = parse_assist(raw);
        assert_eq!(r.nodes.len(), 1, "an empty nodes list is a decoy too");
    }

    #[test]
    fn parse_unstructured_keeps_note() {
        let raw = "I couldn't draw that, here's why...";
        let r = parse_assist(raw);
        assert!(r.mermaid.is_none());
        assert!(r.nodes.is_empty());
        assert_eq!(r.note, raw);
    }

    fn board() -> String {
        serde_json::json!({
            "type": "excalidraw",
            "version": 2,
            "appState": { "viewBackgroundColor": "#fafafa" },
            "files": { "f1": { "id": "f1", "dataURL": "data:image/png;base64,AAAA" } },
            "elements": [
                { "type": "rectangle", "id": "r1", "x": 10, "y": 20, "width": 100, "height": 50,
                  "seed": 7, "versionNonce": 9, "boundElements": [{ "id": "t1", "type": "text" }],
                  "backgroundColor": "#eef2ff", "strokeColor": "#6366f1" },
                { "type": "text", "id": "t1", "containerId": "r1", "text": "API", "originalText": "API",
                  "fontSize": 16, "seed": 1 },
                { "type": "ellipse", "id": "r2", "x": 300, "y": 20, "width": 80, "height": 80, "seed": 2 },
                { "type": "arrow", "id": "a1", "startBinding": { "elementId": "r1" },
                  "endBinding": { "elementId": "r2" }, "points": [[0,0],[100,0]], "seed": 3 },
                { "type": "image", "id": "img", "fileId": "f1", "x": 0, "y": 200, "seed": 4 },
                { "type": "arrow", "id": "a2", "startBinding": { "elementId": "r1" },
                  "endBinding": { "elementId": "img" }, "seed": 5 },
                { "type": "freedraw", "id": "fd", "points": [[0,0],[1,1]], "seed": 6 },
                { "type": "line", "id": "ln", "points": [[0,0],[5,5]], "seed": 8 },
                { "type": "arrow", "id": "free", "points": [[0,0],[9,9]], "seed": 10 },
                { "type": "rectangle", "id": "gone", "isDeleted": true, "seed": 11 }
            ]
        })
        .to_string()
    }

    #[test]
    fn split_hides_inexpressible_elements_and_files() {
        let (view, keep) = split_excalidraw(&board());
        assert!(!view.contains("base64"), "no image data in the agent view");
        assert!(!view.contains("seed"), "view is the simplified form");
        let v: Value = serde_json::from_str(&view).unwrap();
        let ids: Vec<&str> = v["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(el_id)
            .collect();
        assert_eq!(ids, vec!["r1", "r2", "a1"]);
        // bound label folded into the container; arrow id-routed
        assert_eq!(v["elements"][0]["label"]["text"], "API");
        assert_eq!(v["elements"][2]["start"]["id"], "r1");
        assert_eq!(v["elements"][2]["end"]["id"], "r2");
        assert!(v["elements"][2].get("points").is_none());

        let kept: Vec<&str> = keep.elements.iter().filter_map(el_id).collect();
        // image, arrow bound to the image, freedraw, line, free arrow; the
        // deleted element is dropped entirely.
        assert_eq!(kept, vec!["img", "a2", "fd", "ln", "free"]);
        assert!(keep.extras.contains_key("files"));
        assert!(keep.extras.contains_key("appState"));
    }

    #[test]
    fn merge_restores_kept_elements_without_duplicates() {
        let (_, keep) = split_excalidraw(&board());
        // The agent rewrote the board and (wrongly) echoed the image id.
        let agent = serde_json::json!({
            "type": "excalidraw",
            "elements": [
                { "type": "rectangle", "id": "r1", "x": 0, "y": 0, "label": { "text": "API v2" } },
                { "type": "rectangle", "id": "img", "x": 1, "y": 1 },
                { "type": "rectangle", "id": "new", "x": 5, "y": 5 }
            ]
        })
        .to_string();
        let merged: Value = serde_json::from_str(&merge_excalidraw(&agent, &keep)).unwrap();
        let els = merged["elements"].as_array().unwrap();
        let ids: Vec<&str> = els.iter().filter_map(el_id).collect();
        assert_eq!(ids, vec!["r1", "new", "img", "a2", "fd", "ln", "free"]);
        let img = els.iter().find(|e| el_id(e) == Some("img")).unwrap();
        assert_eq!(img["type"], "image", "kept element wins over an echoed id");
        assert_eq!(
            merged["files"]["f1"]["dataURL"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(merged["appState"]["viewBackgroundColor"], "#fafafa");
    }

    #[test]
    fn split_merge_passthrough_for_plain_or_invalid_sources() {
        // Unparseable → untouched, nothing kept.
        let (view, keep) = split_excalidraw("not json");
        assert_eq!(view, "not json");
        assert!(keep.is_empty());
        assert_eq!(merge_excalidraw("whatever", &keep), "whatever");
        // A board with only agent-expressible elements keeps just the header keys.
        let (_, keep) = split_excalidraw(&base_source("excalidraw"));
        assert!(keep.elements.is_empty());
        // Agent output that isn't JSON can't be merged into → returned as-is.
        let (_, keep) = split_excalidraw(&board());
        assert_eq!(merge_excalidraw("oops", &keep), "oops");
    }

    #[test]
    fn build_doc_keeps_existing_extras() {
        let base = serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "d2", "source": "a",
            "sketch": true, "title": "T"
        });
        let d = build_doc(&base, "d2", "a -> b");
        assert_eq!(d["sketch"], true);
        assert_eq!(d["title"], "T");
        assert_eq!(d["source"], "a -> b");
        let fresh = build_doc(&Value::Null, "mermaid", "flowchart TD");
        assert_eq!(fresh["type"], "otto-canvas");
        assert_eq!(fresh["format"], "mermaid");
    }

    #[test]
    fn large_source_is_not_inlined_in_prompt() {
        let big = "x".repeat(INLINE_SOURCE_MAX + 1);
        let p = build_assist_prompt("go", "excalidraw", "canvas.json", &big);
        assert!(!p.contains(&big));
        assert!(p.contains("READ the file"));
        assert!(p.contains("keeps them automatically"));
    }

    #[tokio::test]
    async fn resolve_prefers_edited_file_then_reply() {
        let dir = std::env::temp_dir().join(format!("otto-canvas-test-{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("canvas.mermaid");

        // Agent edited the file → use the file.
        tokio::fs::write(&path, "flowchart TD\n  A-->B\n  B-->C")
            .await
            .unwrap();
        let parsed = AssistResult::default();
        let got = resolve_source(&path, "flowchart TD\n", "mermaid", &parsed)
            .await
            .unwrap();
        assert!(got.contains("B-->C"));

        // File unchanged (== base) → fall back to the reply, and write it back.
        tokio::fs::write(&path, "flowchart TD\n").await.unwrap();
        let parsed = AssistResult {
            mermaid: Some("flowchart LR\n  X-->Y".into()),
            ..Default::default()
        };
        let got = resolve_source(&path, "flowchart TD\n", "mermaid", &parsed)
            .await
            .unwrap();
        assert!(got.contains("X-->Y"));
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            on_disk.contains("X-->Y"),
            "reply source written back to file"
        );

        // Same reply-fallback path for a D2 scene.
        let d2_path = dir.join("canvas.d2");
        tokio::fs::write(&d2_path, "direction: right\n")
            .await
            .unwrap();
        let parsed = AssistResult {
            d2: Some("direction: right\na -> b: hi".into()),
            ..Default::default()
        };
        let got = resolve_source(&d2_path, "direction: right\n", "d2", &parsed)
            .await
            .unwrap();
        assert!(got.contains("a -> b: hi"));
        let on_disk = tokio::fs::read_to_string(&d2_path).await.unwrap();
        assert!(
            on_disk.contains("a -> b: hi"),
            "reply source written back to file"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

#[cfg(test)]
#[path = "assist_regressions.rs"]
mod regressions;

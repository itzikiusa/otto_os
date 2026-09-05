//! Agent-assisted product **mockups** — FILE-BACKED, in-place (mirrors
//! `canvas_assist`).
//!
//! A mockup is a `ProductAttachment` of `kind:"mockup"`. A specialized agent
//! generates / refines it by EDITING a backing file the daemon owns, kept in a
//! per-mockup directory so a resumed session always finds the same file. One
//! "Create with AI" / "Refine" turn:
//!   1. resolves (or creates) the mockup attachment + materializes its current
//!      source into the working file,
//!   2. runs ONE resumed agent turn whose prompt says "edit the file in place"
//!      (follow-ups REFINE the same mockup instead of regenerating it),
//!   3. reads the file back, writes it to the attachment's storage + records the
//!      new size + resumable session id, and
//!   4. broadcasts `Event::MockupUpdated` so the open Assistant panel re-renders.
//!
//! While the turn runs we poll the file and broadcast each change LIVE, so the
//! mockup "builds itself" as the agent writes (no `notify` dep — same poll the
//! session runner uses). The agent shell is surfaced at turn START via
//! `Event::MockupSessionStarted` so the panel attaches the live Terminal then.
//!
//! Four formats — the one `DesignFormat` enum (`design_format.rs`): a
//! self-contained **HTML** screen (default), a **Mermaid** diagram, an
//! **Excalidraw** board or a **scene3d** 3D document (`design_scene3d.rs`). An
//! unknown `format` is a 400, never a silent fallback. The reply is a FALLBACK
//! source: if the agent printed a ```html / ```mermaid / ```json block instead
//! of editing the file (or in the offline E2E stub, where no agent runs), we take
//! the source from the reply. A `scene3d` result must validate before commit.
//!
//! Route (registered in modules.rs):
//!   POST /api/v1/product/stories/{sid}/mockups/assist  (ws editor) → ProductAttachment

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{NewAttachment, ProductAttachment};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::design_format::DesignFormat;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Live-preview file poll cadence while the agent edits.
const POLL: Duration = Duration::from_millis(900);
/// Attachment storage root (mirrors `product_media::ATTACH_ROOT`).
const ATTACH_ROOT: &str = "product/attachments";
/// Per-artifact agent scratch dirs: `data_dir/product/mockup_assist/<aid>/`.
/// The story delete route removes them (via `ProductCtx::mockup_scratch_root`).
pub(crate) const SCRATCH_ROOT: &str = "product/mockup_assist";

#[derive(Debug, Deserialize)]
pub struct MockupAssistReq {
    /// What to draw / change.
    pub prompt: String,
    /// `html` (default) | `mermaid` | `excalidraw` | `scene3d` — anything else
    /// is a 400. Only honored when creating a NEW artifact; a refine keeps the
    /// existing artifact's stored format.
    #[serde(default)]
    pub format: Option<String>,
    /// Refine an EXISTING agent mockup (resume its session); omit to create one.
    #[serde(default)]
    pub mockup_id: Option<Id>,
    /// Agent provider to run the mockup on (built-in or custom, e.g. grok). Only
    /// used when the mockup's session is FIRST created; a refine resumes the
    /// existing session regardless. Empty/absent = default agent.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional model alias for the first turn (empty = provider default).
    #[serde(default)]
    pub model: Option<String>,
}

/// `POST /product/stories/{sid}/mockups/assist` — generate or refine a mockup with
/// the in-place agent, commit it as a `kind:"mockup"` attachment, and broadcast it.
pub async fn assist_mockup(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<MockupAssistReq>,
) -> ApiResult<Json<ProductAttachment>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&story.workspace_id).await.map_err(ApiError)?;

    // Resolve the agent provider (honored only when a NEW mockup session is
    // created; a refine resumes the existing one). Precedence mirrors Discovery
    // Chat: request → workspace default → global default → claude.
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let provider = otto_core::provider::resolve_provider(&[
        req.provider.as_deref().unwrap_or(""),
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);

    // Resolve the target attachment (+ whether THIS call minted it, for cleanup on
    // failure), its format, current source, and the resumable assist session id.
    let (att, created_now, format, current, session_id) =
        resolve_target(&ctx, &story, &user.id, &req).await?;
    let attachment_id = att.id.clone();

    // Working dir (isolated from sibling attachments) — the agent's cwd + file.
    // Attachment ids are daemon-minted, but a refine's id arrived in the request
    // body — confine the join under the mockup_assist root so a hostile id can't
    // steer the fs ops (rust/path-injection).
    let dir = otto_core::paths::confine_join(&ctx.data_dir.join(SCRATCH_ROOT), &attachment_id)
    .ok_or_else(|| ApiError(Error::Invalid(format!("unsafe mockup id {attachment_id}"))))?;
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        if created_now {
            cleanup(&ctx, &att).await;
        }
        return Err(ApiError(Error::Internal(format!("mockup scratch dir: {e}"))));
    }
    let work_file = dir.join(format.file_name());
    let _ = tokio::fs::write(&work_file, &current).await;
    let dir_str = dir.to_string_lossy().to_string();
    otto_sessions::trust::ensure_trusted(&provider, &dir_str);

    // Live preview: broadcast each file change while the turn runs.
    let poll = spawn_file_poll(&ctx, &story, &attachment_id, &work_file, format, &current);

    let prompt = build_mockup_prompt(&req.prompt, format, format.file_name(), &current, &story.title);
    let mut meta = serde_json::json!({
        "source": "mockup_assist", "story_id": story.id, "attachment_id": attachment_id,
    });
    if let Some(m) = req.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        meta["model"] = serde_json::json!(m);
    }
    // Surface the session the MOMENT it exists (turn start) so the Assistant panel
    // attaches the live shell immediately, not after the turn.
    let ready_events = ctx.events.clone();
    let ready_ws = story.workspace_id.clone();
    let ready_story = story.id.clone();
    let ready_att = attachment_id.clone();
    let on_ready = move |sid: &Id| {
        let _ = ready_events.send(Event::MockupSessionStarted {
            workspace_id: ready_ws.clone(),
            story_id: ready_story.clone(),
            attachment_id: ready_att.clone(),
            session_id: sid.clone(),
        });
    };
    let turn = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        session_id.as_ref(),
        &format!("Mockup: {}", story.title),
        &dir_str,
        &provider,
        meta,
        &prompt,
        crate::agent_session::STUCK_IDLE,
        on_ready,
    )
    .await;
    poll.abort();
    // (S2) On turn failure, don't leak the just-minted "Generating…" attachment.
    let (raw, sid) = match turn {
        Ok(v) => v,
        Err(e) => {
            if created_now {
                cleanup(&ctx, &att).await;
            }
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
    };

    // Committed source = the agent's file edit, or the reply's fenced block. A
    // 3D document that fails validation is NOT committed (the prior source
    // stays) — the viewer must never receive an unvalidated scene.
    let mut new_source = resolve_source(&work_file, &current, format, &raw).await;
    if format == DesignFormat::Scene3d {
        if let Err(e) = crate::design_scene3d::validate_bytes(new_source.as_bytes()) {
            tracing::warn!("mockup assist: agent produced an invalid scene3d, keeping prior: {e}");
            new_source = current.clone();
        }
    }
    let bytes = new_source.into_bytes();

    // Write the committed bytes to the attachment's storage + record size + the
    // resumable session id and format in meta_json.
    let full = storage_full(&ctx, &att).map_err(ApiError)?;
    if let Some(parent) = full.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&full, &bytes).await;
    let meta_json = serde_json::json!({
        "assist_session_id": sid, "format": format, "group": format.default_group(),
    })
    .to_string();
    let updated = ctx
        .attachment_repo
        .set_assist_result(&attachment_id, bytes.len() as i64, None, Some(meta_json))
        .await
        .map_err(ApiError)?;

    let _ = ctx.events.send(Event::MockupUpdated {
        workspace_id: story.workspace_id.clone(),
        story_id: story.id.clone(),
        attachment_id,
        format: format.to_string(),
        content: crate::product_media::event_content(format.mime(), &bytes),
    });

    Ok(Json(updated))
}

/// Resolve the artifact we're going to edit. Either an existing `mockup_id`
/// (resume its session) or a freshly-minted `source:"agent"` attachment seeded
/// with a stub so the row/serve are valid before the turn commits. New rows get
/// `kind` from `DesignFormat::attachment_kind` (`mockup` for html/mermaid,
/// `design` for the arena-native formats).
async fn resolve_target(
    ctx: &ServerCtx,
    story: &otto_state::ProductStory,
    user_id: &Id,
    req: &MockupAssistReq,
) -> ApiResult<(ProductAttachment, bool, DesignFormat, String, Option<Id>)> {
    if let Some(mid) = req.mockup_id.as_ref() {
        let att = ctx
            .attachment_repo
            .get(mid)
            .await
            .map_err(ApiError)?
            .filter(|a| a.story_id == story.id && (a.kind == "mockup" || a.kind == "design"))
            .ok_or_else(|| ApiError(Error::NotFound(format!("mockup {mid}"))))?;
        // Only text-backed artifacts are agent-editable — refusing a binary
        // (image / glb) avoids reading non-UTF-8 bytes as text and committing HTML
        // over a `.png` storage path (the row's mime/filename would lie). The
        // stored mime is authoritative for the format; `meta.format` is only a
        // hint that must agree with it.
        let format = DesignFormat::from_mime(&att.mime).ok_or_else(|| {
            ApiError(Error::Invalid(format!(
                "mockup {mid} is not agent-editable ({})",
                att.mime
            )))
        })?;
        let meta: Value = att
            .meta_json
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or(Value::Null);
        let session_id = meta
            .get("assist_session_id")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(Id::from);
        // Current content from storage (so the agent refines, not restarts).
        let full = storage_full(ctx, &att).map_err(ApiError)?;
        let current = read_text_capped(&full)
            .await
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format.base_stub(&story.title));
        Ok((att, false, format, current, session_id))
    } else {
        let format = crate::design_format::parse_or_default(req.format.as_deref(), DesignFormat::Html)
            .map_err(ApiError)?;
        let current = format.base_stub(&story.title);
        // Mirror upload_attachment: the storage filename id is independent of the
        // row id (storage_path is authoritative for serving).
        let file_id = otto_core::new_id();
        let rel = format!("{ATTACH_ROOT}/{}/{}{}", story.id, file_id, format.ext());
        // story.id echoes a route param — confine the join (rust/path-injection).
        let full = otto_core::paths::confine_join(&ctx.data_dir, &rel)
            .ok_or_else(|| ApiError(Error::Invalid(format!("unsafe story id {}", story.id))))?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ApiError(Error::Internal(format!("attachment dir: {e}"))))?;
        }
        let _ = tokio::fs::write(&full, current.as_bytes()).await;
        let att = ctx
            .attachment_repo
            .create(NewAttachment {
                story_id: story.id.clone(),
                workspace_id: story.workspace_id.clone(),
                filename: format.title(),
                mime: format.mime().to_string(),
                size_bytes: current.len() as i64,
                sha256: None,
                storage_path: rel,
                kind: format.attachment_kind().into(),
                source: "agent".into(),
                meta_json: Some(
                    serde_json::json!({ "format": format, "group": format.default_group() })
                        .to_string(),
                ),
                created_by: user_id.clone(),
            })
            .await
            .map_err(ApiError)?;
        Ok((att, true, format, current, None))
    }
}

/// Best-effort removal of a just-minted attachment (row + its storage file) after
/// the turn failed — so a failed "Create with AI" leaves nothing behind.
async fn cleanup(ctx: &ServerCtx, att: &ProductAttachment) {
    if let Ok(full) = storage_full(ctx, att) {
        let _ = tokio::fs::remove_file(full).await;
    }
    let _ = ctx.attachment_repo.delete(&att.id).await;
}

/// Read a text file the agent may have written, refusing anything over the raw
/// attachment cap (`product_media::MAX_RAW_BYTES`) or not UTF-8: `None` means
/// "unusable", and callers keep the prior source.
async fn read_text_capped(path: &std::path::Path) -> Option<String> {
    let len = tokio::fs::metadata(path).await.ok()?.len();
    if len > crate::product_media::MAX_RAW_BYTES as u64 {
        tracing::warn!(
            "mockup assist: {} exceeds the {} MB cap; ignored",
            path.display(),
            crate::product_media::MAX_RAW_BYTES / (1024 * 1024)
        );
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    String::from_utf8(bytes).ok()
}

/// Confine an attachment's stored `storage_path` under the data dir before any
/// fs op. Rows are daemon-written, but the join must not trust them — a
/// traversing path fails closed instead of escaping (rust/path-injection).
fn storage_full(ctx: &ServerCtx, att: &ProductAttachment) -> Result<std::path::PathBuf, Error> {
    otto_core::paths::confine_join(&ctx.data_dir, &att.storage_path).ok_or_else(|| {
        Error::Invalid(format!("attachment {} storage path escapes the data dir", att.id))
    })
}

// ---------------------------------------------------------------------------
// Source resolution (file else reply fence)
// ---------------------------------------------------------------------------

/// Decide the committed source: prefer the agent's in-place file edit; fall back
/// to a ```html / ```mermaid / ```json fence in the reply (E2E stub / agent that
/// printed instead of editing), writing it into the file so the next resumed turn
/// sees it; else keep the prior source.
async fn resolve_source(
    work_file: &std::path::Path,
    current: &str,
    format: DesignFormat,
    raw: &str,
) -> String {
    let after = read_text_capped(work_file).await.unwrap_or_default();
    if !after.trim().is_empty() && after.trim() != current.trim() {
        return after;
    }
    if let Some(src) = extract_fenced(raw, format.fence_lang()) {
        let _ = tokio::fs::write(work_file, &src).await;
        return src;
    }
    current.to_string()
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

// ---------------------------------------------------------------------------
// Live poll
// ---------------------------------------------------------------------------

fn spawn_file_poll(
    ctx: &ServerCtx,
    story: &otto_state::ProductStory,
    attachment_id: &Id,
    work_file: &std::path::Path,
    format: DesignFormat,
    base: &str,
) -> tokio::task::JoinHandle<()> {
    let events = ctx.events.clone();
    let workspace_id = story.workspace_id.clone();
    let story_id = story.id.clone();
    let attachment_id = attachment_id.clone();
    let path = work_file.to_path_buf();
    let mut last = base.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            // Capped read (a runaway agent file over MAX_RAW_BYTES is skipped,
            // never loaded into memory or broadcast).
            let Some(content) = read_text_capped(&path).await else {
                continue;
            };
            if content == last || content.trim().is_empty() {
                continue;
            }
            last = content.clone();
            // A half-written / invalid 3D document is never pushed to viewers —
            // the next poll picks up the agent's completed edit.
            if format == DesignFormat::Scene3d
                && crate::design_scene3d::validate_bytes(content.as_bytes()).is_err()
            {
                continue;
            }
            let _ = events.send(Event::MockupUpdated {
                workspace_id: workspace_id.clone(),
                story_id: story_id.clone(),
                attachment_id: attachment_id.clone(),
                format: format.to_string(),
                // `None` above the WS payload cap → clients re-fetch.
                content: crate::product_media::event_content(format.mime(), content.as_bytes()),
            });
        }
    })
}

// ---------------------------------------------------------------------------
// Prompt (unit-tested, no DB / no agent)
// ---------------------------------------------------------------------------

/// Build the file-edit prompt. The `OTTO_TASK: mockup_assist` sentinel routes the
/// deterministic E2E stub; the rest instructs the real agent to edit the file.
/// The arena-native formats inline the bundled `otto-design-2d` / `otto-design-3d`
/// skills (via `resolve_skill_inline`'s bundled-skill arm) when they exist.
fn build_mockup_prompt(
    user_prompt: &str,
    format: DesignFormat,
    file: &str,
    current: &str,
    story: &str,
) -> String {
    match format {
        DesignFormat::Mermaid => format!(
            "OTTO_TASK: mockup_assist\n\
             You are producing a Mermaid diagram MOCKUP for the product story \"{story}\" by EDITING \
             the file `{file}` in your working directory. Read it, make the requested change IN \
             PLACE, and save it. Keep refining this SAME file across the conversation. The file must \
             always hold ONE COMPLETE, valid Mermaid diagram (no ``` fences inside the file).\n\n\
             Pick the BEST diagram type (flowchart, sequenceDiagram, classDiagram, erDiagram, \
             stateDiagram-v2). Use short emoji-prefixed labels, rhombus decisions with labelled \
             edges, subgraph lanes, and colour via classDef/class at the end.\n\n\
             The file currently contains:\n{current}\n\n\
             Reply with ONE short sentence describing what you changed.\n\n\
             Request: {user_prompt}\n"
        ),
        DesignFormat::Html => format!(
            "OTTO_TASK: mockup_assist\n\
             You are producing a high-fidelity UI MOCKUP (an HTML mockup) for the product story \
             \"{story}\" by EDITING the file `{file}` in your working directory. Read it, apply the \
             requested change IN PLACE, and save it. Keep refining this SAME file across the \
             conversation.\n\n\
             RULES — the file must always hold ONE COMPLETE, SELF-CONTAINED HTML document:\n\
             - A full `<!doctype html>` page with `<meta name=viewport>` for responsiveness.\n\
             - ALL CSS inline in a single `<style>` block. NO external network requests, NO `<link>` to \
             CDNs, NO external fonts/images/scripts (use system-ui fonts, CSS shapes, inline SVG, emoji).\n\
             - Realistic, representative sample content (real-looking labels/data, not lorem ipsum).\n\
             - Clean, modern visual design: clear hierarchy, spacing, a small cohesive colour palette, \
             rounded cards, subtle borders/shadows. It should read as a polished product screen.\n\
             - It renders inside a sandboxed iframe with scripts DISABLED — make it look right with \
             pure HTML + CSS (no JS needed to convey the design).\n\n\
             The file currently contains:\n{current}\n\n\
             Reply with ONE short sentence describing what you changed.\n\n\
             Request: {user_prompt}\n"
        ),
        DesignFormat::Excalidraw => {
            let skill = bundled_skill_section("otto-design-2d");
            format!(
                "OTTO_TASK: mockup_assist\n\
                 You are producing a DESIGN BOARD (an Excalidraw scene) for the product story \
                 \"{story}\" by EDITING the file `{file}` in your working directory. Read it, apply \
                 the requested change IN PLACE, and save it. Keep refining this SAME file across the \
                 conversation.\n\n\
                 RULES — the file must always hold ONE COMPLETE, valid Excalidraw JSON document \
                 (`{{\"type\":\"excalidraw\",\"version\":2,\"elements\":[…],\"appState\":{{…}},\"files\":{{}}}}`; \
                 no ``` fences inside the file):\n\
                 - Use `frame` elements as artboards (one per screen / state), named clearly.\n\
                 - Snap to an 8-pt grid; consistent stroke widths; a small cohesive palette.\n\
                 - Build screens from rectangles, text, ellipses, arrows and lines with real-looking \
                 labels (not lorem ipsum). Give every element a unique `id`, `versionNonce`, `seed`.\n\
                 - No images / `files` entries (nothing external).\n\n\
                 {skill}\
                 The file currently contains:\n{current}\n\n\
                 Reply with ONE short sentence describing what you changed.\n\n\
                 Request: {user_prompt}\n"
            )
        }
        DesignFormat::Scene3d => {
            let skill = bundled_skill_section("otto-design-3d");
            format!(
                "OTTO_TASK: mockup_assist\n\
                 You are producing a 3D SCENE (an `otto-scene3d` JSON document) for the product story \
                 \"{story}\" by EDITING the file `{file}` in your working directory. Read it, apply \
                 the requested change IN PLACE, and save it. Keep refining this SAME file across the \
                 conversation.\n\n\
                 RULES — the file must always hold ONE COMPLETE, valid `otto-scene3d` document \
                 (`type: \"otto-scene3d\", version: 1`; no ``` fences inside the file):\n\
                 - Units are metres, y-up, origin at the floor; `rotation` is in DEGREES.\n\
                 - `objects[].type` ∈ box | sphere | cylinder | cone | torus | plane | text | gltf | group; \
                 primitives have a unit bounding box before `scale`.\n\
                 - `material`: `color` (#rrggbb), `metalness`, `roughness`, `opacity` (0..1), \
                 `emissive` (#rrggbb), `wireframe`. `lights[].type` ∈ directional | ambient | point | \
                 spot | hemisphere. Every `id` is unique, short and path-safe (no spaces or slashes).\n\
                 - `gltf` objects reference an existing attachment by `attachment_id` ONLY (never a URL \
                 or path). At most 2000 objects; all numbers finite.\n\
                 - Use `groups` to organise props; keep a floor plane, a directional key light with \
                 shadow, and an ambient fill so the blockout reads well.\n\n\
                 {skill}\
                 The file currently contains:\n{current}\n\n\
                 Reply with ONE short sentence describing what you changed.\n\n\
                 Request: {user_prompt}\n"
            )
        }
    }
}

/// The bundled skill body for the arena-native formats, wrapped as a prompt
/// section — empty when the skill isn't compiled in (the prompt's inline rules
/// carry the essentials either way).
fn bundled_skill_section(name: &str) -> String {
    match otto_skills::bundled_body(name) {
        Some(body) if !body.trim().is_empty() => {
            format!("SKILL `{name}` — follow it:\n{}\n\n", body.trim())
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_prompt_has_sentinel_file_and_rules() {
        let p = build_mockup_prompt("a settings page", DesignFormat::Html, "design.html", "<html></html>", "My Story");
        assert!(p.contains("OTTO_TASK: mockup_assist"));
        assert!(p.contains("design.html"));
        assert!(p.contains("SELF-CONTAINED HTML"));
        assert!(p.contains("My Story"));
        assert!(p.contains("a settings page"));
    }

    #[test]
    fn mermaid_prompt_points_at_mmd_file() {
        let p = build_mockup_prompt("a login flow", DesignFormat::Mermaid, "design.mmd", "flowchart TD\n", "S");
        assert!(p.contains("OTTO_TASK: mockup_assist"));
        assert!(p.contains("design.mmd"));
        assert!(p.contains("sequenceDiagram"));
        assert!(!p.contains("SELF-CONTAINED HTML"));
    }

    #[test]
    fn excalidraw_and_scene3d_prompts_carry_their_schemas() {
        let p = build_mockup_prompt("a checkout flow", DesignFormat::Excalidraw, "design.excalidraw", "{}", "S");
        assert!(p.contains("OTTO_TASK: mockup_assist"));
        assert!(p.contains("design.excalidraw"));
        assert!(p.contains("\"type\":\"excalidraw\""));
        assert!(p.contains("frame"));
        let p = build_mockup_prompt("a kiosk", DesignFormat::Scene3d, "scene.json", "{}", "S");
        assert!(p.contains("scene.json"));
        assert!(p.contains("otto-scene3d"));
        assert!(p.contains("DEGREES"));
        assert!(p.contains("attachment_id"));
    }

    #[test]
    fn unknown_format_is_rejected_not_defaulted() {
        // The old `normalize_format("weird") == "html"` fallback is gone: a bad
        // format on a NEW artifact is a 400 from `parse_or_default`.
        let err = crate::design_format::parse_or_default(Some("weird"), DesignFormat::Html).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
        assert_eq!(
            crate::design_format::parse_or_default(None, DesignFormat::Html).unwrap(),
            DesignFormat::Html
        );
        assert_eq!(DesignFormat::Mermaid.file_name(), "design.mmd");
        assert_eq!(DesignFormat::Html.file_name(), "design.html");
        assert_eq!(DesignFormat::Mermaid.mime(), "text/vnd.mermaid");
        assert_eq!(DesignFormat::Html.mime(), "text/html");
        assert!(DesignFormat::Html.base_stub("T").contains("<!doctype html>"));
        assert!(DesignFormat::Mermaid.base_stub("T").contains("flowchart"));
    }

    #[test]
    fn extract_fenced_html_and_mermaid() {
        let raw = "Built it.\n\n```html\n<!doctype html><body>hi</body>\n```";
        assert_eq!(
            extract_fenced(raw, "html").as_deref(),
            Some("<!doctype html><body>hi</body>")
        );
        let raw2 = "Done.\n\n```mermaid\nflowchart TD\n  A-->B\n```";
        assert_eq!(extract_fenced(raw2, "mermaid").as_deref(), Some("flowchart TD\n  A-->B"));
        assert!(extract_fenced("no fence", "html").is_none());
        let raw3 = "```json\n{\"type\":\"otto-scene3d\"}\n```";
        assert_eq!(extract_fenced(raw3, DesignFormat::Scene3d.fence_lang()).as_deref(), Some("{\"type\":\"otto-scene3d\"}"));
    }

    #[tokio::test]
    async fn capped_read_refuses_non_utf8_and_missing() {
        let dir = std::env::temp_dir().join(format!("otto-mockup-cap-{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let small = dir.join("small.html");
        tokio::fs::write(&small, "<p>ok</p>").await.unwrap();
        assert_eq!(read_text_capped(&small).await.as_deref(), Some("<p>ok</p>"));
        let bin = dir.join("bin.html");
        tokio::fs::write(&bin, [0xFF, 0xFE, 0x00]).await.unwrap();
        assert!(read_text_capped(&bin).await.is_none());
        assert!(read_text_capped(&dir.join("missing")).await.is_none());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn resolve_prefers_edited_file_then_reply() {
        let dir = std::env::temp_dir().join(format!("otto-mockup-test-{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("design.html");

        // Agent edited the file → use the file.
        tokio::fs::write(&path, "<html>edited</html>").await.unwrap();
        let got = resolve_source(&path, "<html>stub</html>", DesignFormat::Html, "").await;
        assert!(got.contains("edited"));

        // File unchanged (== current) → fall back to the reply fence + write it back.
        tokio::fs::write(&path, "<html>stub</html>").await.unwrap();
        let raw = "Here.\n\n```html\n<html>from-reply</html>\n```";
        let got = resolve_source(&path, "<html>stub</html>", DesignFormat::Html, raw).await;
        assert!(got.contains("from-reply"));
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(on_disk.contains("from-reply"), "reply source written back to file");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

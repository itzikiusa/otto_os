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
//! Two formats, both rendered by the existing `MockupViewer` /
//! `MockupLivePreview`: a self-contained **HTML** page (default — rich UI
//! mockups) or a **Mermaid** diagram. The reply is a FALLBACK source: if the
//! agent printed a ```html / ```mermaid block instead of editing the file (or in
//! the offline E2E stub, where no agent runs), we take the source from the reply.
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
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Live-preview file poll cadence while the agent edits.
const POLL: Duration = Duration::from_millis(900);
/// Attachment storage root (mirrors `product_media::ATTACH_ROOT`).
const ATTACH_ROOT: &str = "product/attachments";

#[derive(Debug, Deserialize)]
pub struct MockupAssistReq {
    /// What to draw / change.
    pub prompt: String,
    /// `html` (default) | `mermaid`. Only honored when creating a NEW mockup; a
    /// refine keeps the existing mockup's stored format.
    #[serde(default)]
    pub format: Option<String>,
    /// Refine an EXISTING agent mockup (resume its session); omit to create one.
    #[serde(default)]
    pub mockup_id: Option<Id>,
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

    // Resolve the target attachment (+ whether THIS call minted it, for cleanup on
    // failure), its format, current source, and the resumable assist session id.
    let (att, created_now, format, current, session_id) =
        resolve_target(&ctx, &story, &user.id, &req).await?;
    let attachment_id = att.id.clone();

    // Working dir (isolated from sibling attachments) — the agent's cwd + file.
    let dir = ctx.data_dir.join("product").join("mockup_assist").join(&attachment_id);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        if created_now {
            cleanup(&ctx, &att).await;
        }
        return Err(ApiError(Error::Internal(format!("mockup scratch dir: {e}"))));
    }
    let work_file = dir.join(file_name(&format));
    let _ = tokio::fs::write(&work_file, &current).await;
    let dir_str = dir.to_string_lossy().to_string();
    otto_sessions::trust::ensure_trusted("claude", &dir_str);

    // Live preview: broadcast each file change while the turn runs.
    let poll = spawn_file_poll(&ctx, &story, &attachment_id, &work_file, &format, &current);

    let prompt = build_mockup_prompt(&req.prompt, &format, file_name(&format), &current, &story.title);
    let meta = serde_json::json!({
        "source": "mockup_assist", "story_id": story.id, "attachment_id": attachment_id,
    });
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
        "claude",
        meta,
        &prompt,
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

    // Committed source = the agent's file edit, or the reply's fenced block.
    let new_source = resolve_source(&work_file, &current, &format, &raw).await;
    let bytes = new_source.into_bytes();

    // Write the committed bytes to the attachment's storage + record size + the
    // resumable session id and format in meta_json.
    let full = ctx.data_dir.join(&att.storage_path);
    if let Some(parent) = full.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&full, &bytes).await;
    let meta_json = serde_json::json!({ "assist_session_id": sid, "format": format }).to_string();
    let updated = ctx
        .attachment_repo
        .set_assist_result(&attachment_id, bytes.len() as i64, None, Some(meta_json))
        .await
        .map_err(ApiError)?;

    let _ = ctx.events.send(Event::MockupUpdated {
        workspace_id: story.workspace_id.clone(),
        story_id: story.id.clone(),
        attachment_id,
        format,
        content: String::from_utf8_lossy(&bytes).to_string(),
    });

    Ok(Json(updated))
}

/// Resolve the mockup we're going to edit. Either an existing `mockup_id` (resume
/// its session) or a freshly-minted `kind:"mockup", source:"agent"` attachment
/// seeded with a stub so the row/serve are valid before the turn commits.
async fn resolve_target(
    ctx: &ServerCtx,
    story: &otto_state::ProductStory,
    user_id: &Id,
    req: &MockupAssistReq,
) -> ApiResult<(ProductAttachment, bool, String, String, Option<Id>)> {
    if let Some(mid) = req.mockup_id.as_ref() {
        let att = ctx
            .attachment_repo
            .get(mid)
            .await
            .map_err(ApiError)?
            .filter(|a| a.story_id == story.id && a.kind == "mockup")
            .ok_or_else(|| ApiError(Error::NotFound(format!("mockup {mid}"))))?;
        // Only text-backed mockups are agent-editable — refusing a binary (image)
        // mockup avoids reading non-UTF-8 bytes as text and committing HTML over a
        // `.png` storage path (the row's mime/filename would lie).
        if att.mime != "text/html" && att.mime != "text/vnd.mermaid" {
            return Err(ApiError(Error::Invalid(format!(
                "mockup {mid} is not agent-editable ({})",
                att.mime
            ))));
        }
        let meta: Value = att
            .meta_json
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or(Value::Null);
        let format = meta
            .get("format")
            .and_then(|f| f.as_str())
            .map(normalize_format)
            .unwrap_or_else(|| "html".to_string());
        let session_id = meta
            .get("assist_session_id")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(Id::from);
        // Current content from storage (so the agent refines, not restarts).
        let full = ctx.data_dir.join(&att.storage_path);
        let current = tokio::fs::read_to_string(&full)
            .await
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| base_stub(&format, &story.title));
        Ok((att, false, format, current, session_id))
    } else {
        let format = req
            .format
            .as_deref()
            .map(normalize_format)
            .unwrap_or_else(|| "html".to_string());
        let current = base_stub(&format, &story.title);
        // Mirror upload_attachment: the storage filename id is independent of the
        // row id (storage_path is authoritative for serving).
        let file_id = otto_core::new_id();
        let rel = format!("{ATTACH_ROOT}/{}/{}{}", story.id, file_id, ext_for(&format));
        let full = ctx.data_dir.join(&rel);
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
                filename: title_for(&format),
                mime: mime_for(&format).to_string(),
                size_bytes: current.len() as i64,
                sha256: None,
                storage_path: rel,
                kind: "mockup".into(),
                source: "agent".into(),
                meta_json: Some(serde_json::json!({ "format": format }).to_string()),
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
    let _ = tokio::fs::remove_file(ctx.data_dir.join(&att.storage_path)).await;
    let _ = ctx.attachment_repo.delete(&att.id).await;
}

// ---------------------------------------------------------------------------
// Format helpers
// ---------------------------------------------------------------------------

fn normalize_format(f: &str) -> String {
    if f == "mermaid" {
        "mermaid".to_string()
    } else {
        "html".to_string()
    }
}
fn file_name(format: &str) -> &'static str {
    if format == "mermaid" {
        "mockup.mmd"
    } else {
        "mockup.html"
    }
}
fn ext_for(format: &str) -> &'static str {
    if format == "mermaid" {
        ".mmd"
    } else {
        ".html"
    }
}
fn mime_for(format: &str) -> &'static str {
    if format == "mermaid" {
        "text/vnd.mermaid"
    } else {
        "text/html"
    }
}
fn title_for(format: &str) -> String {
    if format == "mermaid" {
        "AI mockup.mmd".to_string()
    } else {
        "AI mockup.html".to_string()
    }
}

/// A minimal valid placeholder so a brand-new mockup renders before the agent
/// commits real content.
fn base_stub(format: &str, story_title: &str) -> String {
    if format == "mermaid" {
        "flowchart TD\n  A([\"Generating…\"])\n".to_string()
    } else {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title></head>\
<body style=\"font:15px/1.5 system-ui;padding:40px;color:#334155\">\
<p>Generating a mockup for <strong>{title}</strong>…</p></body></html>\n",
            title = html_escape(story_title)
        )
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Source resolution (file else reply fence)
// ---------------------------------------------------------------------------

/// Decide the committed source: prefer the agent's in-place file edit; fall back
/// to a ```html / ```mermaid fence in the reply (E2E stub / agent that printed
/// instead of editing), writing it into the file so the next resumed turn sees it;
/// else keep the prior source.
async fn resolve_source(work_file: &std::path::Path, current: &str, format: &str, raw: &str) -> String {
    let after = tokio::fs::read_to_string(work_file).await.unwrap_or_default();
    if !after.trim().is_empty() && after.trim() != current.trim() {
        return after;
    }
    let lang = if format == "mermaid" { "mermaid" } else { "html" };
    if let Some(src) = extract_fenced(raw, lang) {
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
    format: &str,
    base: &str,
) -> tokio::task::JoinHandle<()> {
    let events = ctx.events.clone();
    let workspace_id = story.workspace_id.clone();
    let story_id = story.id.clone();
    let attachment_id = attachment_id.clone();
    let path = work_file.to_path_buf();
    let format = format.to_string();
    let mut last = base.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if content != last && !content.trim().is_empty() {
                    last = content.clone();
                    let _ = events.send(Event::MockupUpdated {
                        workspace_id: workspace_id.clone(),
                        story_id: story_id.clone(),
                        attachment_id: attachment_id.clone(),
                        format: format.clone(),
                        content,
                    });
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Prompt (unit-tested, no DB / no agent)
// ---------------------------------------------------------------------------

/// Build the file-edit prompt. The `OTTO_TASK: mockup_assist` sentinel routes the
/// deterministic E2E stub; the rest instructs the real agent to edit the file.
fn build_mockup_prompt(user_prompt: &str, format: &str, file: &str, current: &str, story: &str) -> String {
    if format == "mermaid" {
        return format!(
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
        );
    }
    format!(
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
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_prompt_has_sentinel_file_and_rules() {
        let p = build_mockup_prompt("a settings page", "html", "mockup.html", "<html></html>", "My Story");
        assert!(p.contains("OTTO_TASK: mockup_assist"));
        assert!(p.contains("mockup.html"));
        assert!(p.contains("SELF-CONTAINED HTML"));
        assert!(p.contains("My Story"));
        assert!(p.contains("a settings page"));
    }

    #[test]
    fn mermaid_prompt_points_at_mmd_file() {
        let p = build_mockup_prompt("a login flow", "mermaid", "mockup.mmd", "flowchart TD\n", "S");
        assert!(p.contains("OTTO_TASK: mockup_assist"));
        assert!(p.contains("mockup.mmd"));
        assert!(p.contains("sequenceDiagram"));
        assert!(!p.contains("SELF-CONTAINED HTML"));
    }

    #[test]
    fn format_helpers() {
        assert_eq!(normalize_format("mermaid"), "mermaid");
        assert_eq!(normalize_format("html"), "html");
        assert_eq!(normalize_format("weird"), "html");
        assert_eq!(file_name("mermaid"), "mockup.mmd");
        assert_eq!(file_name("html"), "mockup.html");
        assert_eq!(mime_for("mermaid"), "text/vnd.mermaid");
        assert_eq!(mime_for("html"), "text/html");
        assert!(base_stub("html", "T").contains("<!doctype html>"));
        assert!(base_stub("mermaid", "T").contains("flowchart"));
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
    }

    #[tokio::test]
    async fn resolve_prefers_edited_file_then_reply() {
        let dir = std::env::temp_dir().join(format!("otto-mockup-test-{}", std::process::id()));
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("mockup.html");

        // Agent edited the file → use the file.
        tokio::fs::write(&path, "<html>edited</html>").await.unwrap();
        let got = resolve_source(&path, "<html>stub</html>", "html", "").await;
        assert!(got.contains("edited"));

        // File unchanged (== current) → fall back to the reply fence + write it back.
        tokio::fs::write(&path, "<html>stub</html>").await.unwrap();
        let raw = "Here.\n\n```html\n<html>from-reply</html>\n```";
        let got = resolve_source(&path, "<html>stub</html>", "html", raw).await;
        assert!(got.contains("from-reply"));
        let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(on_disk.contains("from-reply"), "reply source written back to file");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

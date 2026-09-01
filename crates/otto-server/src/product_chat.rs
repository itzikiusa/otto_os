//! Discovery Chat — talk to an agent on a Product story from an empty/Untitled
//! draft to help with EARLY discovery and research (before anything is written).
//!
//! Distinct from refinement (which edits an existing version) and from the
//! swarm discovery run (a heavyweight multi-agent report). Each turn assembles a
//! relevance-bounded context bundle — the latest relevant version, the story's
//! mockups/attachments (text ones inlined), the most recent discovery report,
//! open questions and notes — and replays the chat history into a single
//! `run_agent` turn. The agent answers in prose AND may emit a fenced ```json
//! `{actions:[...]}` block; actions are NEVER auto-applied — the UI renders them
//! as cards the user explicitly applies via `/apply`.
//!
//! Routes (registered in modules.rs under the `/product/` policy prefix):
//!   POST /api/v1/product/stories/{sid}/discovery-chats   (ws editor) → DiscoveryChat
//!   GET  /api/v1/product/stories/{sid}/discovery-chats   (ws viewer) → DiscoveryChat[]
//!   GET  /api/v1/product/discovery-chats/{cid}           (ws viewer) → ChatDetail
//!   POST /api/v1/product/discovery-chats/{cid}/messages  (ws editor) → ChatTurn
//!   POST /api/v1/product/discovery-chats/{cid}/archive   (ws editor) → DiscoveryChat
//!   POST /api/v1/product/discovery-chats/{cid}/apply     (ws editor) → ApplyResult


use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Error, Id};
use otto_state::{
    DiscoveryChat, DiscoveryChatMessage, NewDiscoveryChat, NewDiscoveryChatMessage, NewNote,
    NewQuestion, NewScene,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Total context-bundle char budget (oldest/least-relevant trimmed first).
const CTX_BUDGET: usize = 24_000;
/// Per-attachment inline cap for text mockups.
const ATTACH_INLINE_CAP: usize = 4_000;
/// Discovery report cap.
const DISCOVERY_BUDGET: usize = 4_000;

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct CreateChatReq {
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatDetail {
    pub chat: DiscoveryChat,
    pub messages: Vec<DiscoveryChatMessage>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageReq {
    pub body: String,
    /// Agent provider to run the chat on (built-in or custom, e.g. grok). Only
    /// used when the chat's persistent session is FIRST created; later messages
    /// resume the existing session regardless. Empty/absent = default agent.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional model alias for the first turn (empty = provider default).
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatTurn {
    pub user_message: DiscoveryChatMessage,
    pub agent_message: DiscoveryChatMessage,
}

#[derive(Debug, Deserialize)]
pub struct ApplyReq {
    /// The action object the agent emitted (tolerant — dispatched by `type`).
    pub action: Value,
}

#[derive(Debug, Default, Serialize)]
pub struct ApplyResult {
    pub story_updated: bool,
    pub created_question_ids: Vec<Id>,
    pub created_note_ids: Vec<Id>,
    pub canvas_id: Option<Id>,
}

// ---------------------------------------------------------------------------
// Workspace resolution helper
// ---------------------------------------------------------------------------

async fn chat_with_role(
    ctx: &ServerCtx,
    user: &User,
    cid: &Id,
    role: WorkspaceRole,
) -> ApiResult<DiscoveryChat> {
    let chat = ctx
        .discovery_chat_repo
        .get_chat(cid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("discovery chat {cid}"))))?;
    crate::auth::require_ws_role(ctx, user, &chat.workspace_id, role).await?;
    Ok(chat)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /product/stories/{sid}/discovery-chats` — start a discovery chat on a
/// story (Editor). Allocates a fresh scratch working dir for the agent.
pub async fn create_chat(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<CreateChatReq>>,
) -> ApiResult<Json<DiscoveryChat>> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;

    // Prefer the story's cwd (a real repo) so the agent can research code; else a
    // fresh scratch dir under data_dir.
    let cwd = match &story.cwd {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => ctx
            .data_dir
            .join("product/discovery-chat")
            .join(new_id())
            .to_string_lossy()
            .to_string(),
    };
    let title = req
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Discovery".to_string());

    let chat = ctx
        .discovery_chat_repo
        .create_chat(NewDiscoveryChat {
            story_id: story.id.clone(),
            workspace_id: story.workspace_id.clone(),
            cwd,
            title,
            model: None,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(chat))
}

/// `GET /product/stories/{sid}/discovery-chats` — list a story's chats (Viewer).
pub async fn list_chats(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DiscoveryChat>>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Viewer).await?;
    let chats = ctx
        .discovery_chat_repo
        .list_for_story(&sid)
        .await
        .map_err(ApiError)?;
    Ok(Json(chats))
}

/// `GET /product/discovery-chats/{cid}` — a chat + its transcript (Viewer).
pub async fn get_chat(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ChatDetail>> {
    let chat = chat_with_role(&ctx, &user, &cid, WorkspaceRole::Viewer).await?;
    let messages = ctx
        .discovery_chat_repo
        .get_messages(&cid)
        .await
        .map_err(ApiError)?;
    Ok(Json(ChatDetail { chat, messages }))
}

/// `POST /product/discovery-chats/{cid}/archive` — archive a chat (Editor).
pub async fn archive_chat(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<DiscoveryChat>> {
    chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;
    let chat = ctx
        .discovery_chat_repo
        .set_status(&cid, "archived")
        .await
        .map_err(ApiError)?;
    Ok(Json(chat))
}

/// `POST /product/discovery-chats/{cid}/messages` — one conversational turn
/// (Editor). Assembles context, runs the agent, splits prose from proposed
/// actions, persists both messages.
pub async fn send_message(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SendMessageReq>,
) -> ApiResult<Json<ChatTurn>> {
    let chat = chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;

    // Assemble the context bundle (also stored on the user message for audit).
    let context = assemble_context(&ctx, &chat.story_id).await;

    // Persist the user message with the bundle in meta.
    let user_message = ctx
        .discovery_chat_repo
        .add_message(NewDiscoveryChatMessage {
            chat_id: cid.clone(),
            role: "user".into(),
            body: req.body.clone(),
            actions_json: None,
            meta_json: Some(json!({ "context": context }).to_string()),
        })
        .await
        .map_err(ApiError)?;

    // The chat runs as ONE persistent, resumable Otto session (visible in
    // Agents, MCP-wired), so the agent RETAINS the conversation itself — we send
    // framing + the refreshed context + the new message, NOT a replayed history.
    let prompt = build_chat_prompt(&context, &[], &req.body);
    let ws = ctx
        .workspaces
        .get(&chat.workspace_id)
        .await
        .map_err(ApiError)?;
    // Provider is honored only on FIRST message (session create); resumes ignore
    // it. Default to the workspace default, else the global default, else claude.
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        req.provider.as_deref().unwrap_or(""),
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    let mut meta = json!({ "source": "discovery_chat", "story_id": chat.story_id, "chat_id": cid });
    if let Some(m) = req.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        meta["model"] = json!(m);
    }
    let (raw, sid) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        chat.session_id.as_ref(),
        &format!("Discovery: {}", chat.title),
        &chat.cwd,
        &default_provider,
        meta,
        &prompt,
        crate::agent_session::STUCK_IDLE,
        |_| {},
    )
    .await?;
    // Link the session on the first turn so later turns resume it.
    if chat.session_id.is_none() {
        let _ = ctx.discovery_chat_repo.set_session(&cid, &sid).await;
    }

    let (markdown, actions_json) = split_actions(&raw);

    let agent_message = ctx
        .discovery_chat_repo
        .add_message(NewDiscoveryChatMessage {
            chat_id: cid.clone(),
            role: "agent".into(),
            body: markdown,
            actions_json,
            meta_json: None,
        })
        .await
        .map_err(ApiError)?;

    Ok(Json(ChatTurn {
        user_message,
        agent_message,
    }))
}

/// `POST /product/discovery-chats/{cid}/apply` — apply ONE proposed action
/// (Editor). Dispatched by the action's `type` field.
pub async fn apply_action(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApplyReq>,
) -> ApiResult<Json<ApplyResult>> {
    let chat = chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;
    let action = &req.action;
    let kind = action.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let mut result = ApplyResult::default();

    match kind {
        "apply_draft" => {
            let body_md = action.get("body_md").and_then(|b| b.as_str()).unwrap_or("");
            // Don't blank the story title when the agent omits it — keep the
            // current one.
            let title = match action.get("title").and_then(|t| t.as_str()) {
                Some(t) if !t.trim().is_empty() => t.to_string(),
                _ => ctx
                    .product_repo
                    .get_story(&chat.story_id)
                    .await
                    .map_err(ApiError)?
                    .title,
            };
            ctx.product
                .update_draft_body(&chat.story_id, &title, body_md, &user.id)
                .await
                .map_err(ApiError)?;
            result.story_updated = true;
        }
        "add_questions" => {
            if let Some(qs) = action.get("questions").and_then(|q| q.as_array()) {
                for q in qs {
                    let text = q.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    let created = ctx
                        .product_repo
                        .create_question(NewQuestion {
                            story_id: chat.story_id.clone(),
                            analysis_id: None,
                            text: text.to_string(),
                            rationale: q
                                .get("rationale")
                                .and_then(|r| r.as_str())
                                .unwrap_or("")
                                .to_string(),
                            category: q
                                .get("category")
                                .and_then(|c| c.as_str())
                                .unwrap_or("other")
                                .to_string(),
                            created_by: user.id.clone(),
                        })
                        .await
                        .map_err(ApiError)?;
                    result.created_question_ids.push(created.id);
                }
            }
        }
        "add_notes" => {
            if let Some(ns) = action.get("notes").and_then(|n| n.as_array()) {
                for n in ns {
                    let body = n.get("body").and_then(|b| b.as_str()).unwrap_or("");
                    if body.trim().is_empty() {
                        continue;
                    }
                    let created = ctx
                        .product_repo
                        .create_note(NewNote {
                            story_id: chat.story_id.clone(),
                            section: Some("discovery".into()),
                            body: body.to_string(),
                            author_id: user.id.clone(),
                        })
                        .await
                        .map_err(ApiError)?;
                    result.created_note_ids.push(created.id);
                }
            }
        }
        "create_canvas" => {
            // Note: this route is gated on Product (the `/product/` policy prefix),
            // so a Product-Editor creates the scene as a byproduct of discovery —
            // the Canvas capability axis is intentionally NOT additionally required
            // here (the scene lands in the user's own workspace; Canvas:View gates
            // whether they can then open the Canvas module to see it).
            let title = action
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("Discovery diagram")
                .to_string();
            let doc = canvas_doc_from_action(action, &title);
            let scene = ctx
                .canvas_repo
                .create(NewScene {
                    workspace_id: chat.workspace_id.clone(),
                    story_id: Some(chat.story_id.clone()),
                    title,
                    doc_json: doc.to_string(),
                    provider: "claude".into(),
                    section: None,
                    created_by: user.id.clone(),
                })
                .await
                .map_err(ApiError)?;
            result.canvas_id = Some(scene.id);
        }
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "unknown discovery action type: {other}"
            ))));
        }
    }
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Context assembly (DB + fs)
// ---------------------------------------------------------------------------

/// Build the relevance-bounded context string for a story. Pulls the latest
/// relevant version, attachments (text mockups inlined), the most recent
/// discovery report, open questions and notes. Bounded by `CTX_BUDGET`.
async fn assemble_context(ctx: &ServerCtx, story_id: &Id) -> String {
    let story = match ctx.product_repo.get_story(story_id).await {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    // Latest relevant version: suggested > draft > source > the title.
    let mut body = String::new();
    for kind in ["suggested", "draft", "source"] {
        if let Ok(Some(v)) = ctx.product_repo.latest_version_of_kind(story_id, kind).await {
            if !v.body_md.trim().is_empty() {
                body = v.body_md;
                break;
            }
        }
    }
    if body.trim().is_empty() {
        body = story.title.clone();
    }

    // Attachments — inline text mockups; list raster images by absolute path.
    let mut attachments: Vec<(String, String, String, Option<String>)> = Vec::new();
    if let Ok(atts) = ctx.attachment_repo.list_for_story(story_id).await {
        for a in atts {
            // Confine the stored path's join under the data dir — a traversing
            // storage_path drops the attachment instead of reading outside it
            // (rust/path-injection).
            let Some(path) = otto_core::paths::confine_join(&ctx.data_dir, &a.storage_path) else {
                continue;
            };
            let inlined = if is_text_mockup(&a.mime, &a.filename) {
                // Async read — don't block the tokio worker on disk I/O.
                tokio::fs::read_to_string(&path)
                    .await
                    .ok()
                    .map(|t| truncate(&t, ATTACH_INLINE_CAP))
            } else {
                None
            };
            attachments.push((
                a.filename,
                a.mime,
                path.display().to_string(),
                inlined,
            ));
        }
    }

    // Most recent discovery report.
    let discovery = ctx
        .discovery_repo
        .list_for_story(story_id)
        .await
        .ok()
        .and_then(|runs| runs.into_iter().next())
        .and_then(|r| r.report_md)
        .filter(|r| !r.trim().is_empty());

    // Open questions + notes.
    let questions: Vec<String> = ctx
        .product_repo
        .list_questions(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|q| q.status == "open")
        .map(|q| q.text)
        .collect();
    let notes: Vec<String> = ctx
        .product_repo
        .list_notes(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.body)
        .collect();

    format_context(
        &story.title,
        &body,
        &attachments,
        discovery.as_deref(),
        &questions,
        &notes,
    )
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

fn is_text_mockup(mime: &str, filename: &str) -> bool {
    let m = mime.to_lowercase();
    m == "text/vnd.mermaid"
        || m == "text/html"
        || m == "text/markdown"
        || m == "text/plain"
        || filename.to_lowercase().ends_with(".mmd")
        || filename.to_lowercase().ends_with(".md")
}

/// Assemble the bounded context string. Text mockups are inlined; the whole
/// bundle is capped at `CTX_BUDGET` with a clear truncation marker.
fn format_context(
    title: &str,
    body: &str,
    attachments: &[(String, String, String, Option<String>)],
    discovery: Option<&str>,
    questions: &[String],
    notes: &[String],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("## Latest version\n# {title}\n\n{body}\n\n"));

    if !attachments.is_empty() {
        s.push_str("## Mockups & attachments\n");
        for (filename, mime, path, inlined) in attachments {
            s.push_str(&format!("- {filename} ({mime}) at {path}\n"));
            if let Some(text) = inlined {
                s.push_str("```\n");
                s.push_str(text);
                s.push_str("\n```\n");
            }
        }
        s.push('\n');
    }

    if let Some(report) = discovery {
        s.push_str("## Discovery findings\n");
        s.push_str(&truncate(report, DISCOVERY_BUDGET));
        s.push_str("\n\n");
    }

    if !questions.is_empty() {
        s.push_str("## Open questions\n");
        for q in questions {
            s.push_str(&format!("- {q}\n"));
        }
        s.push('\n');
    }

    if !notes.is_empty() {
        s.push_str("## Notes\n");
        for n in notes {
            s.push_str(&format!("- {n}\n"));
        }
        s.push('\n');
    }

    truncate(&s, CTX_BUDGET)
}

/// Build the one-shot discovery-chat prompt. The `OTTO_TASK: discovery_chat`
/// sentinel routes the deterministic E2E stub; the actions contract is
/// load-bearing (the turn engine parses the reply against it).
fn build_chat_prompt(context: &str, history: &[DiscoveryChatMessage], new_message: &str) -> String {
    let mut s = String::new();
    s.push_str(
        "OTTO_TASK: discovery_chat\n\
         You are a product discovery partner. The user may have NOTHING written yet — \
         help them research, ask the right questions, surface edge cases, and shape a \
         story. Be concrete and use the context below (their draft, mockups, discovery \
         notes). Reply conversationally in markdown. When useful, ALSO emit a SINGLE \
         fenced ```json block with an `actions` array to propose concrete next steps. \
         Supported actions (never auto-applied — the user approves each):\n\
         {\"actions\":[\n\
           {\"type\":\"apply_draft\",\"title\":\"…\",\"body_md\":\"full story markdown\"},\n\
           {\"type\":\"add_questions\",\"questions\":[{\"text\":\"…\",\"rationale\":\"…\",\"category\":\"…\"}]},\n\
           {\"type\":\"add_notes\",\"notes\":[{\"body\":\"…\"}]},\n\
           {\"type\":\"create_canvas\",\"title\":\"…\",\"mermaid\":\"sequenceDiagram…\"}\n\
         ]}\n\
         Omit the json block entirely when you have no concrete proposal.\n\n",
    );

    if !context.trim().is_empty() {
        s.push_str("# Context\n");
        s.push_str(context);
        s.push('\n');
    }

    if history.len() > 1 {
        s.push_str("## Conversation so far\n");
        // Skip the just-added user message (it's appended last below).
        for m in &history[..history.len().saturating_sub(1)] {
            let who = if m.role == "agent" { "Assistant" } else { "User" };
            s.push_str(&format!("{who}: {}\n", m.body));
        }
        s.push('\n');
    }

    s.push_str(&format!("## New message\nUser: {new_message}\n"));
    s
}

/// Split an agent reply into `(markdown, actions_json)`. The `actions` array
/// (when present and valid) is returned as a JSON string; the markdown is the
/// prose with the json block removed. Tolerant — never panics.
fn split_actions(raw: &str) -> (String, Option<String>) {
    // Find an actions array via the shared extractor.
    let actions_json = otto_swarm::recruiter::extract_json(raw)
        .and_then(|v| v.get("actions").cloned())
        .filter(|a| a.is_array() && !a.as_array().map(|x| x.is_empty()).unwrap_or(true))
        .map(|a| a.to_string());

    // Markdown = prose before the json fence / object.
    let cut = raw
        .find("```json")
        .or_else(|| {
            // No fence — cut at the first '{' that starts the object (best effort).
            actions_json.as_ref().and_then(|_| raw.find('{'))
        })
        .unwrap_or(raw.len());
    let mut markdown = raw[..cut].trim().to_string();
    if markdown.is_empty() {
        markdown = if actions_json.is_some() {
            "Here are some suggestions.".to_string()
        } else {
            raw.trim().to_string()
        };
    }
    (markdown, actions_json)
}

/// Build a Scene doc from a `create_canvas` action (mermaid or nodes) in the
/// FILE-BACKED canvas shape (`{type:"otto-canvas", version:1, format, source}`)
/// — the only shape Canvas Studio renders. The legacy Svelte-Flow doc
/// (`{schema:1, nodes:[{kind:"mermaid",…}]}`) this used to emit has no
/// top-level `source`, so every "Open in Canvas" from Discovery Chat landed on
/// a permanently blank board.
fn canvas_doc_from_action(action: &Value, title: &str) -> Value {
    let source = match action.get("mermaid").and_then(|m| m.as_str()) {
        Some(src) => src.to_string(),
        // Structured nodes/edges → synthesize a mermaid flowchart so the
        // proposed diagram still lands editable instead of being dropped.
        None => {
            let nodes = action
                .get("nodes")
                .and_then(|n| n.as_array())
                .cloned()
                .unwrap_or_default();
            let edges = action
                .get("edges")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            mermaid_from_nodes(&nodes, &edges)
        }
    };
    json!({
        "type": "otto-canvas",
        "version": 1,
        "format": "mermaid",
        "title": title,
        "source": source,
    })
}

/// Best-effort mermaid flowchart from a `create_canvas` action's node/edge
/// lists. Node ids are sanitized to mermaid-safe tokens; labels are quoted.
fn mermaid_from_nodes(nodes: &[Value], edges: &[Value]) -> String {
    let sid = |raw: &str| -> String {
        let s: String = raw
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if s.is_empty() { "n".to_string() } else { s }
    };
    let mut out = String::from("flowchart TD\n");
    for (i, n) in nodes.iter().enumerate() {
        let id = n
            .get("id")
            .and_then(Value::as_str)
            .map(sid)
            .unwrap_or_else(|| format!("n{i}"));
        let label = n
            .get("label")
            .or_else(|| n.get("title"))
            .or_else(|| n.get("text"))
            .and_then(Value::as_str)
            .unwrap_or(&id);
        out.push_str(&format!("  {id}[\"{}\"]\n", label.replace('"', "'")));
    }
    for e in edges {
        let from = e.get("from").or_else(|| e.get("source")).and_then(Value::as_str);
        let to = e.get("to").or_else(|| e.get("target")).and_then(Value::as_str);
        if let (Some(f), Some(t)) = (from, to) {
            out.push_str(&format!("  {} --> {}\n", sid(f), sid(t)));
        }
    }
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n…[truncated]")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn msg(role: &str, body: &str) -> DiscoveryChatMessage {
        DiscoveryChatMessage {
            id: "m".into(),
            chat_id: "c".into(),
            role: role.into(),
            body: body.into(),
            actions_json: None,
            meta_json: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn context_includes_version_mockup_and_discovery() {
        let atts = vec![(
            "flow.mmd".to_string(),
            "text/vnd.mermaid".to_string(),
            "/data/a/flow.mmd".to_string(),
            Some("sequenceDiagram\n A->>B: x".to_string()),
        )];
        let s = format_context(
            "Login",
            "As a user I log in.",
            &atts,
            Some("Affected: auth-svc."),
            &["What is the lockout policy?".to_string()],
            &["saw a similar flow in billing".to_string()],
        );
        assert!(s.contains("As a user I log in."));
        assert!(s.contains("flow.mmd"));
        assert!(s.contains("A->>B: x")); // inlined mockup text
        assert!(s.contains("Affected: auth-svc"));
        assert!(s.contains("lockout policy"));
        assert!(s.contains("billing"));
    }

    #[test]
    fn context_respects_budget() {
        let big = "x".repeat(CTX_BUDGET * 2);
        let s = format_context("T", &big, &[], None, &[], &[]);
        assert!(s.chars().count() <= CTX_BUDGET + 32); // bounded + marker
        assert!(s.contains("[truncated]"));
    }

    #[test]
    fn prompt_has_sentinel_and_actions_contract() {
        let p = build_chat_prompt("# Context\nbody", &[msg("user", "hi")], "scope X");
        assert!(p.contains("OTTO_TASK: discovery_chat"));
        assert!(p.contains("apply_draft"));
        assert!(p.contains("create_canvas"));
        assert!(p.contains("scope X"));
    }

    #[test]
    fn split_actions_extracts_array_and_prose() {
        let raw = "Here's my take.\n\n```json\n{\"actions\":[{\"type\":\"add_questions\",\"questions\":[{\"text\":\"q1\"}]}]}\n```";
        let (md, actions) = split_actions(raw);
        assert_eq!(md, "Here's my take.");
        let actions = actions.expect("actions");
        assert!(actions.contains("add_questions"));
    }

    #[test]
    fn split_actions_no_block_keeps_prose() {
        let raw = "Just some thoughts, no proposal yet.";
        let (md, actions) = split_actions(raw);
        assert_eq!(md, raw);
        assert!(actions.is_none());
    }

    #[test]
    fn canvas_doc_from_mermaid_action() {
        // Regression: must be the FILE-BACKED shape (format+source) — the
        // legacy nodes[] shape rendered a permanently blank canvas.
        let action = json!({"type":"create_canvas","title":"Seq","mermaid":"sequenceDiagram\n A->>B: x"});
        let doc = canvas_doc_from_action(&action, "Seq");
        assert_eq!(doc["type"], "otto-canvas");
        assert_eq!(doc["format"], "mermaid");
        assert!(doc["source"].as_str().unwrap().contains("A->>B"));
    }

    #[test]
    fn canvas_doc_from_nodes_action_synthesizes_mermaid() {
        let action = json!({
            "type": "create_canvas",
            "nodes": [
                {"id": "a", "label": "Start"},
                {"id": "b-2", "label": "End \"quoted\""},
            ],
            "edges": [{"from": "a", "to": "b-2"}],
        });
        let doc = canvas_doc_from_action(&action, "Flow");
        assert_eq!(doc["format"], "mermaid");
        let src = doc["source"].as_str().unwrap();
        assert!(src.starts_with("flowchart TD"));
        assert!(src.contains("a[\"Start\"]"));
        assert!(src.contains("b_2[\"End 'quoted'\"]"), "sanitized id + quotes: {src}");
        assert!(src.contains("a --> b_2"));
    }
}

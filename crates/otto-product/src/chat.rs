//! Discovery Chat — the story Chat's earlier engine, now READ-ONLY history.
//!
//! The story Chat runs on the refinement engine (`refine`): one conversation
//! that writes a proposed body as a `suggested` version and carries this
//! chat's action cards (`chat_actions`). What stays here:
//! - the old chats and their transcripts, readable as "Earlier chats" (nothing
//!   is deleted or migrated);
//! - the relevance-bounded context bundle (`assemble_context`) and the reply
//!   splitter (`split_actions`), which the merged Chat reuses.
//!
//! Routes (registered in modules.rs under the `/product/` policy prefix):
//!   POST /api/v1/product/stories/{sid}/discovery-chats   (ws editor) → DiscoveryChat
//!   GET  /api/v1/product/stories/{sid}/discovery-chats   (ws viewer) → DiscoveryChat[]
//!   GET  /api/v1/product/discovery-chats/{cid}           (ws viewer) → ChatDetail
//!   POST /api/v1/product/discovery-chats/{cid}/messages  (ws editor) → 409, read-only
//!   POST /api/v1/product/discovery-chats/{cid}/archive   (ws editor) → DiscoveryChat
//!   POST /api/v1/product/discovery-chats/{cid}/apply     (ws editor) → ApplyResult

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Error, Id};
use otto_state::{DiscoveryChat, DiscoveryChatMessage, NewDiscoveryChat};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat_actions::{apply_story_action, ActionTarget, ApplyReq, ApplyResult};
use crate::host::ProductStudioHost;
use crate::http::{ApiError, ApiResult, CurrentUser};

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

// ---------------------------------------------------------------------------
// Workspace resolution helper
// ---------------------------------------------------------------------------

async fn chat_with_role<C: ProductStudioHost>(
    ctx: &C,
    user: &User,
    cid: &Id,
    role: WorkspaceRole,
) -> ApiResult<DiscoveryChat> {
    let chat = ctx
        .discovery_chat_repo()
        .get_chat(cid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("discovery chat {cid}"))))?;
    ctx.roles().check(user, &chat.workspace_id, role).await?;
    Ok(chat)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /product/stories/{sid}/discovery-chats` — start a discovery chat on a
/// story (Editor). Allocates a fresh scratch working dir for the agent.
pub async fn create_chat<C: ProductStudioHost>(
    Path(sid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<CreateChatReq>>,
) -> ApiResult<Json<DiscoveryChat>> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Prefer the story's cwd (a real repo) so the agent can research code; else a
    // fresh scratch dir under data_dir.
    let cwd = match &story.cwd {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => ctx
            .data_dir()
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
        .discovery_chat_repo()
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
pub async fn list_chats<C: ProductStudioHost>(
    Path(sid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DiscoveryChat>>> {
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let chats = ctx
        .discovery_chat_repo()
        .list_for_story(&sid)
        .await
        .map_err(ApiError)?;
    Ok(Json(chats))
}

/// `GET /product/discovery-chats/{cid}` — a chat + its transcript (Viewer).
pub async fn get_chat<C: ProductStudioHost>(
    Path(cid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ChatDetail>> {
    let chat = chat_with_role(&ctx, &user, &cid, WorkspaceRole::Viewer).await?;
    let messages = ctx
        .discovery_chat_repo()
        .get_messages(&cid)
        .await
        .map_err(ApiError)?;
    Ok(Json(ChatDetail { chat, messages }))
}

/// `POST /product/discovery-chats/{cid}/archive` — archive a chat (Editor).
pub async fn archive_chat<C: ProductStudioHost>(
    Path(cid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<DiscoveryChat>> {
    chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;
    let chat = ctx
        .discovery_chat_repo()
        .set_status(&cid, "archived")
        .await
        .map_err(ApiError)?;
    Ok(Json(chat))
}

/// `POST /product/discovery-chats/{cid}/messages` — refused (409): a
/// discovery chat is read-only history. The conversation continues in the
/// story Chat (`refine`), which writes suggestions instead of replacing text.
/// Kept as a route so an old client gets a clear answer, not a 404.
pub async fn send_message<C: ProductStudioHost>(
    Path(cid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;
    Err(ApiError(Error::Conflict(
        "this discovery chat is read-only; continue in the story's Chat tab".into(),
    )))
}

/// `POST /product/discovery-chats/{cid}/apply` — apply ONE action proposed
/// in an earlier chat (Editor), on the same rules as the story Chat
/// (`chat_actions`): a proposed body becomes a `suggested` version.
pub async fn apply_action<C: ProductStudioHost>(
    Path(cid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApplyReq>,
) -> ApiResult<Json<ApplyResult>> {
    let chat = chat_with_role(&ctx, &user, &cid, WorkspaceRole::Editor).await?;
    let target = ActionTarget {
        story_id: &chat.story_id,
        workspace_id: &chat.workspace_id,
        user_id: &user.id,
    };
    let result = apply_story_action(ctx.product(), ctx.canvas_repo(), &target, &req.action)
        .await
        .map_err(ApiError)?;
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Context assembly (DB + fs)
// ---------------------------------------------------------------------------

/// Build the relevance-bounded context string for a story. Pulls the latest
/// relevant version, attachments (text mockups inlined), the most recent
/// discovery report, open questions and notes. Bounded by `CTX_BUDGET`.
pub(crate) async fn assemble_context<C: ProductStudioHost>(ctx: &C, story_id: &Id) -> String {
    let story = match ctx.product_repo().get_story(story_id).await {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    // Latest relevant version: suggested > draft > source > the title.
    let mut body = String::new();
    for kind in ["suggested", "draft", "source"] {
        if let Ok(Some(v)) = ctx
            .product_repo()
            .latest_version_of_kind(story_id, kind)
            .await
        {
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
    if let Ok(atts) = ctx.attachments().list_for_story(story_id).await {
        for a in atts {
            // Confine the stored path's join under the data dir — a traversing
            // storage_path drops the attachment instead of reading outside it
            // (rust/path-injection).
            let Some(path) = otto_core::paths::confine_join(ctx.data_dir(), &a.storage_path) else {
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
            attachments.push((a.filename, a.mime, path.display().to_string(), inlined));
        }
    }

    // Open questions + notes.
    let questions: Vec<String> = ctx
        .product_repo()
        .list_questions(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|q| q.status == "open")
        .map(|q| q.text)
        .collect();
    let notes: Vec<String> = ctx
        .product_repo()
        .list_notes(story_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.body)
        .collect();

    format_context(&story.title, &body, &attachments, None, &questions, &notes)
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

/// Split an agent reply into `(markdown, actions_json)`. The `actions` array
/// (when present, non-empty and valid) is returned as a JSON string; the
/// markdown is the reply with ONLY the block that carried it removed — an
/// example payload fence, a `{orderId}` placeholder and every word after them
/// stay. (It used to cut at the first ```` ```json ```` or `{`, so a reply that
/// showed an example payload lost it, everything after it, and — since the
/// example parsed first — the actions too.) Tolerant — never panics.
pub(crate) fn split_actions(raw: &str) -> (String, Option<String>) {
    let has_actions = |v: &Value| {
        v.get("actions")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    let Some(found) = otto_core::text::extract_json_where(raw, has_actions) else {
        return (raw.trim().to_string(), None);
    };
    let actions_json = found.get("actions").map(Value::to_string);

    let mut markdown = match actions_span(raw, &found) {
        Some(span) => {
            let before = raw[..span.start].trim_end();
            let after = raw[span.end..].trim_start();
            match (before.is_empty(), after.is_empty()) {
                (false, false) => format!("{before}\n\n{after}"),
                (false, true) => before.to_string(),
                _ => after.to_string(),
            }
        }
        None => raw.trim().to_string(),
    };
    if markdown.is_empty() {
        markdown = "Here are some suggestions.".to_string();
    }
    (markdown, actions_json)
}

/// The byte range in `raw` of the block that parsed to `found`: a whole
/// ```` ```json … ``` ```` fence (markers included) when one holds it, else
/// the bare balanced object.
fn actions_span(raw: &str, found: &Value) -> Option<std::ops::Range<usize>> {
    const FENCE: &str = "```json";
    let mut from = 0;
    while let Some(rel) = raw[from..].find(FENCE) {
        let start = from + rel;
        let body_start = start + FENCE.len();
        let Some(end_rel) = raw[body_start..].find("```") else {
            break;
        };
        let body_end = body_start + end_rel;
        if serde_json::from_str::<Value>(raw[body_start..body_end].trim())
            .ok()
            .as_ref()
            == Some(found)
        {
            return Some(start..body_end + 3);
        }
        from = body_end + 3;
    }
    // No fence holds it: try each `{` in turn, skipping past objects that
    // parse but are not the one we want (their inner `{`s are not
    // candidates — same order as the shared balanced-span scan).
    let mut i = 0;
    while let Some(rel) = raw[i..].find('{') {
        let start = i + rel;
        let mut stream = serde_json::Deserializer::from_str(&raw[start..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(v)) => {
                let end = start + stream.byte_offset();
                if &v == found {
                    return Some(start..end);
                }
                i = end;
            }
            _ => i = start + 1,
        }
    }
    None
}

fn truncate(text: &str, max: usize) -> String {
    otto_core::text::clip_chars_with(text, max, "\n…[truncated]")
}

/// Stop exactly the caller's turn and wait for managed-session cleanup.
pub async fn stop_turn<C: ProductStudioHost>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StopTurnReq>,
) -> ApiResult<Json<StopTurnResp>> {
    let conversation = chat_with_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let stopped =
        crate::studio_turn::stop("discovery", &conversation.workspace_id, &id, &req.turn_id).await;
    Ok(Json(StopTurnResp { stopped }))
}

#[derive(Debug, Deserialize)]
pub struct StopTurnReq {
    pub turn_id: String,
}
#[derive(Debug, Serialize)]
pub struct StopTurnResp {
    pub stopped: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::{DiscoveryChatRepo, NewDiscoveryChatMessage};

    #[tokio::test]
    async fn earlier_chats_stay_listable_and_readable_with_their_proposals() {
        // Nothing migrates or deletes the old Discovery Chat rows: the story's
        // list and each transcript (with the agent's `actions_json`) are what
        // the merged tab's "Earlier chats" renders.
        let (_svc, pool, user, ws, sid) = crate::chat_actions::tests::fixture_with_pool().await;
        let repo = DiscoveryChatRepo::new(pool);
        let chat = repo
            .create_chat(NewDiscoveryChat {
                story_id: sid.clone(),
                workspace_id: ws,
                cwd: "/tmp/discovery".into(),
                title: "Discovery".into(),
                model: None,
                created_by: user,
            })
            .await
            .unwrap();
        for (role, body, actions) in [
            ("user", "help me scope login", None),
            (
                "agent",
                "Two questions first.",
                Some(r#"[{"type":"add_questions","questions":[{"text":"SLA?"}]}]"#),
            ),
        ] {
            repo.add_message(NewDiscoveryChatMessage {
                chat_id: chat.id.clone(),
                role: role.into(),
                body: body.into(),
                actions_json: actions.map(str::to_string),
                meta_json: None,
            })
            .await
            .unwrap();
        }

        let listed = repo.list_for_story(&sid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, chat.id);
        let messages = repo.get_messages(&chat.id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert!(messages[1]
            .actions_json
            .as_deref()
            .is_some_and(|a| a.contains("add_questions")));
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
    fn split_actions_extracts_array_and_prose() {
        let raw = "Here's my take.\n\n```json\n{\"actions\":[{\"type\":\"add_questions\",\"questions\":[{\"text\":\"q1\"}]}]}\n```";
        let (md, actions) = split_actions(raw);
        assert_eq!(md, "Here's my take.");
        let actions = actions.expect("actions");
        assert!(actions.contains("add_questions"));
    }

    #[test]
    fn split_actions_keeps_example_fence_and_trailing_prose() {
        // An example payload fence comes first and parses; the actions fence
        // is the second one, and prose follows both. The example and the
        // prose survive, and the actions are still found.
        let raw = "The webhook body looks like\n\n```json\n{\"event\":\"paid\"}\n```\n\nEdge cases: refunds, retries.\n\n```json\n{\"actions\":[{\"type\":\"add_questions\",\"questions\":[{\"text\":\"q1\"}]}]}\n```\n\nTell me which fits.";
        let (md, actions) = split_actions(raw);
        let actions = actions.expect("actions found past the example fence");
        assert!(actions.contains("add_questions"));
        assert!(md.contains("{\"event\":\"paid\"}"), "example kept: {md}");
        assert!(md.contains("Edge cases: refunds, retries."), "{md}");
        assert!(md.contains("Tell me which fits."), "{md}");
        assert!(!md.contains("add_questions"), "actions block removed: {md}");
    }

    #[test]
    fn split_actions_bare_object_keeps_placeholders_and_prose() {
        // No fence: a `{orderId}` placeholder before the bare actions object
        // used to be the cut point, dropping everything after it.
        let raw = "Use the `{orderId}` path parameter. {\"actions\":[{\"type\":\"add_questions\",\"questions\":[]}]} And log it.";
        let (md, actions) = split_actions(raw);
        assert!(actions.is_some());
        assert_eq!(md, "Use the `{orderId}` path parameter.\n\nAnd log it.");
    }

    #[test]
    fn split_actions_no_block_keeps_prose() {
        let raw = "Just some thoughts, no proposal yet.";
        let (md, actions) = split_actions(raw);
        assert_eq!(md, raw);
        assert!(actions.is_none());
    }
}

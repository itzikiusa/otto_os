//! The story Chat: one conversation on a Product story (the merged Chat and
//! Refine tabs).
//!
//! The PO talks to an agent that sees a relevance-bounded context bundle (the
//! story's best body, its mockups and attachments with text ones inlined, open
//! questions and notes — `chat::assemble_context`) plus the replayed thread
//! history. Two kinds of output, both explicit:
//! - a proposed story body ALWAYS becomes a new `suggested` version
//!   (`ProductService::propose_story_body`), never an in-place edit; Rewrite
//!   reviews and publishes it;
//! - `actions` (questions, notes, a design diagram) come back as cards the PO
//!   applies one at a time (`chat_actions`, `POST …/apply`).
//!
//! Each turn is one managed `run_session_turn` (provider-selectable), driven
//! with the **last `HISTORY_TURN_CAP` messages replayed in the prompt**. The
//! agent returns `{reply, updated_story_md?, summary?, actions?}`; a reply in
//! the earlier Discovery Chat shape (prose + a fenced `{actions:[…]}` block)
//! is read too, and its `apply_draft` folds into the suggested body.
//!
//! Routes (registered in `orchestrator_routes()` because the turn engine needs the
//! product repo, the refinement repo, and the orchestrator together):
//!   POST /api/v1/product/stories/{sid}/refinement-threads   (ws editor) → RefinementThread
//!   GET  /api/v1/product/stories/{sid}/refinement-threads   (ws viewer) → RefinementThread[]
//!   GET  /api/v1/product/refinement-threads/{tid}           (ws viewer) → {thread, messages}
//!   POST /api/v1/product/refinement-threads/{tid}/messages  (ws editor) → TurnResp
//!   POST /api/v1/product/refinement-threads/{tid}/apply     (ws editor) → ApplyResult
//!   POST /api/v1/product/refinement-threads/{tid}/archive   (ws editor) → RefinementThread

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Error, Id};
use otto_state::{NewRefinementMessage, NewRefinementThread, RefinementMessage, RefinementThread};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

use crate::chat_actions::{apply_story_action, card_actions, ActionTarget, ApplyReq, ApplyResult};
use crate::host::ProductStudioHost;
use crate::http::{ApiError, ApiResult, CurrentUser};

/// No-progress window for a refinement turn — a *stuck* window, NOT a wall-clock
/// cap (the hard PTY cap lives in `claude_pty.rs`). Matches the discovery planner.
const REFINE_NO_PROGRESS: Duration = Duration::from_secs(150);
/// Replay at most the last N messages into the prompt (bound prompt growth).
const HISTORY_TURN_CAP: usize = 40;

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

/// Request body for `POST /product/stories/{sid}/refinement-threads`.
#[derive(Debug, Default, Deserialize)]
pub struct CreateThreadReq {
    /// Optionally link a discovery run so its findings seed the agent's context.
    #[serde(default)]
    pub discovery_run_id: Option<String>,
    /// Override the thread title (defaults to `"Refinement"`).
    #[serde(default)]
    pub title: Option<String>,
}

/// A thread plus its full message transcript.
#[derive(Debug, Serialize)]
pub struct ThreadDetail {
    pub thread: RefinementThread,
    pub messages: Vec<RefinementMessage>,
}

/// Request body for `POST /product/refinement-threads/{tid}/messages`.
#[derive(Debug, Deserialize)]
pub struct SendMessageReq {
    pub body: String,
    /// Unique per-request identity used by Stop; omit for legacy callers.
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Agent provider to run this turn on (built-in or custom, e.g. grok).
    /// Empty/absent = default agent (resolved request → workspace → global →
    /// claude). Each turn replays the whole thread history in the prompt, so a
    /// per-turn provider is self-contained.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional model alias for this turn (empty = the thread's model, else the
    /// provider default).
    #[serde(default)]
    pub model: Option<String>,
}

/// Result of one conversational turn: the persisted user + agent messages,
/// whether a `suggested` version was written (with its number), and the
/// action cards the agent proposed (also stored in the agent message's
/// `meta_json.actions`; empty when none).
#[derive(Debug, Serialize)]
pub struct TurnResp {
    pub user_message: RefinementMessage,
    pub agent_message: RefinementMessage,
    pub story_updated: bool,
    pub version_no: Option<i64>,
    pub actions: Vec<Value>,
}

// ---------------------------------------------------------------------------
// Workspace resolution helper
// ---------------------------------------------------------------------------

/// Load a thread, resolve its story → workspace, and role-check the caller.
/// Returns the loaded thread on success (404 when the thread is absent).
async fn thread_with_role<C: ProductStudioHost>(
    ctx: &C,
    user: &User,
    tid: &Id,
    role: WorkspaceRole,
) -> ApiResult<RefinementThread> {
    let thread = ctx
        .refinement_repo()
        .get_thread(tid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("refinement thread {tid}"))))?;
    let story = ctx
        .product_repo()
        .get_story(&thread.story_id)
        .await
        .map_err(ApiError)?;
    ctx.roles().check(user, &story.workspace_id, role).await?;
    Ok(thread)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /product/stories/{sid}/refinement-threads` — create a refinement thread
/// on a story (Editor). Allocates an isolated working dir for the thread (the
/// linked discovery run's repo path when present + resolvable, else a fresh
/// scratch dir under `data_dir/product/refine/`). The dir is created lazily in
/// the turn engine, not here.
pub async fn create_thread<C: ProductStudioHost>(
    Path(sid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<CreateThreadReq>>,
) -> ApiResult<Json<RefinementThread>> {
    let req = body.map(|b| b.0).unwrap_or_default();

    // Resolve the story + Editor role-check via its workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Pick the working dir: fresh per-thread scratch dir (fresh ULID so it's
    // unique regardless of the row id the repo generates).
    let discovery_run_id = req.discovery_run_id.filter(|r| !r.trim().is_empty());
    let cwd = ctx
        .data_dir()
        .join("product/refine")
        .join(new_id())
        .to_string_lossy()
        .to_string();

    let title = req
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Refinement".to_string());

    let thread = ctx
        .refinement_repo()
        .create_thread(NewRefinementThread {
            story_id: story.id.clone(),
            workspace_id: story.workspace_id.clone(),
            discovery_run_id,
            cwd,
            title,
            model: None,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(thread))
}

/// `GET /product/stories/{sid}/refinement-threads` — list a story's refinement
/// threads, newest first (Viewer).
pub async fn list_threads<C: ProductStudioHost>(
    Path(sid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<RefinementThread>>> {
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let threads = ctx
        .refinement_repo()
        .list_threads_for_story(&sid)
        .await
        .map_err(ApiError)?;
    Ok(Json(threads))
}

/// `GET /product/refinement-threads/{tid}` — a thread + its full transcript
/// (Viewer).
pub async fn get_thread<C: ProductStudioHost>(
    Path(tid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ThreadDetail>> {
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Viewer).await?;
    let messages = ctx
        .refinement_repo()
        .list_messages(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(ThreadDetail { thread, messages }))
}

/// `POST /product/refinement-threads/{tid}/archive` — archive a thread (Editor),
/// returning the reloaded (archived) thread.
pub async fn archive_thread<C: ProductStudioHost>(
    Path(tid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<RefinementThread>> {
    thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;
    ctx.refinement_repo()
        .archive_thread(&tid)
        .await
        .map_err(ApiError)?;
    let thread = ctx
        .refinement_repo()
        .get_thread(&tid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("refinement thread {tid}"))))?;
    Ok(Json(thread))
}

/// `POST /product/refinement-threads/{tid}/messages` — one conversational turn
/// (Editor). The synchronous turn engine:
///   1. Resolve thread + Editor.
///   2. Persist the user message.
///   3. Gather context: the story bundle (`chat::assemble_context`) and the
///      history (last `HISTORY_TURN_CAP`, without the new message).
///   4. Build the one-shot prompt.
///   5. `create_dir_all(&thread.cwd)` (best-effort) BEFORE the agent run.
///   6. `run_session_turn` (history replayed in the prompt).
///   7. Parse the reply (tolerant; malformed → reply=raw, no update, no cards).
///   8. On a proposed body: write a `suggested` version + emit `ProductChanged`.
///   9. Persist the agent message (meta `{story_updated, version_no, actions?}`)
///      + record a Product event.
///  10. Return the user + agent messages, the update result and the cards.
pub async fn send_message<C: ProductStudioHost>(
    Path(tid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SendMessageReq>,
) -> ApiResult<Json<TurnResp>> {
    // Dropping the HTTP response must not release the turn while its managed
    // session still owns cleanup. Stop continues to address the same task.
    crate::studio_turn::complete(async move {
        send_message_inner(Path(tid), State(ctx), CurrentUser(user), Json(req)).await
    })
    .await
}

async fn send_message_inner<C: ProductStudioHost>(
    Path(tid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SendMessageReq>,
) -> ApiResult<Json<TurnResp>> {
    // 1. Resolve thread + Editor role-check.
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;
    let _story_turn = crate::story_lifecycle::claim(&thread.story_id).await?;
    let turn = crate::studio_turn::claim_with_id(
        "refinement",
        &thread.workspace_id,
        &tid,
        req.turn_id.as_deref(),
    )?;
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;

    let story = ctx
        .product_repo()
        .get_story(&thread.story_id)
        .await
        .map_err(ApiError)?;

    // 2. Persist the user message.
    let user_message = ctx
        .refinement_repo()
        .create_message(NewRefinementMessage {
            thread_id: tid.clone(),
            role: "user".into(),
            body: req.body.clone(),
            meta_json: None,
        })
        .await
        .map_err(ApiError)?;

    // 3. The story bundle — best body (suggested → draft → source → title),
    //    mockups/attachments (text inlined, paths confined to the data dir),
    //    open questions and notes — bounded like the earlier Discovery Chat.
    let context = crate::chat::assemble_context(&ctx, &thread.story_id).await;

    // 3b. History — the last `HISTORY_TURN_CAP` messages (chronological),
    //     without the message just persisted: the prompt appends it last.
    let all_history = ctx
        .refinement_repo()
        .list_messages(&tid)
        .await
        .unwrap_or_default();
    let mut history: Vec<RefinementMessage> = all_history
        .iter()
        .filter(|m| m.id != user_message.id)
        .cloned()
        .collect();
    let older = history.len().saturating_sub(HISTORY_TURN_CAP);
    history.drain(..older);

    // 4. Build the one-shot prompt.
    let prompt = build_turn_prompt(&context, &history, &req.body);

    // 5. Ensure the thread cwd exists (run_prompt canonicalizes it and degrades
    //    if it's missing). Best-effort — logged on error.
    if let Err(e) = std::fs::create_dir_all(&thread.cwd) {
        warn!("product_refine: create_dir_all({}) failed: {e}", thread.cwd);
    }

    // 6. Run the agent as a managed, provider-aware Otto session (history
    //    replayed in the prompt, so each turn is self-contained): `existing:
    //    None` gives a fresh session per turn, visible in Agents and drivable by
    //    any configured provider. Provider precedence: request → workspace
    //    default → global default → claude.
    let ws = ctx
        .workspaces()
        .get(&story.workspace_id)
        .await
        .map_err(ApiError)?;
    let provider = ctx
        .resolve_provider(Some(&ws), req.provider.as_deref())
        .await
        .map_err(ApiError)?;
    let mut meta =
        json!({ "source": "product_refine", "story_id": thread.story_id, "thread_id": tid });
    // Model: this turn's override, else the thread's stored model.
    if let Some(m) = req
        .model
        .as_deref()
        .or(thread.model.as_deref())
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        meta["model"] = json!(m);
    }
    turn.check_running()?;
    let (raw, _sid) = ctx
        .run_session_turn(
            &ws,
            &user,
            None,
            &format!("Chat: {}", story.title),
            &thread.cwd,
            &provider,
            meta,
            &prompt,
            REFINE_NO_PROGRESS,
            turn.cancel(),
        )
        .await?;

    turn.check_running()?;
    // 7. Parse — tolerant; malformed/missing → reply=raw, no update, no cards.
    let parsed = parse_turn(&raw);

    // 8. A proposed body becomes a `suggested` version (never an in-place edit).
    let mut version_no: Option<i64> = None;
    if let Some(updated) = parsed.updated_story_md.as_deref() {
        let v = ctx
            .product()
            .propose_story_body(
                &thread.story_id,
                parsed.title.as_deref(),
                updated,
                parsed.summary.clone(),
                &user.id,
            )
            .await
            .map_err(ApiError)?;
        version_no = Some(v.version_no);
        let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
            workspace_id: story.workspace_id.clone(),
            story_id: thread.story_id.clone(),
            section: "refine".into(),
            status: "suggested".into(),
        });
    }
    let story_updated = version_no.is_some();

    // 9. Persist the agent message (with the turn-outcome meta) + a Product event.
    let agent_message = ctx
        .refinement_repo()
        .create_message(NewRefinementMessage {
            thread_id: tid.clone(),
            role: "agent".into(),
            body: parsed.reply,
            meta_json: Some(turn_meta(version_no, &parsed.actions).to_string()),
        })
        .await
        .map_err(ApiError)?;
    let _ = ctx
        .product_repo()
        .add_event(otto_state::NewEvent {
            story_id: thread.story_id.clone(),
            section: "refine".into(),
            kind: "refined".into(),
            summary: parsed.summary.unwrap_or_else(|| "Chat turn".into()),
            actor_id: Some(user.id.clone()),
            meta_json: Some(json!({ "thread_id": tid, "version_no": version_no }).to_string()),
        })
        .await;

    // 10. Return both messages, the update result and the cards.
    Ok(Json(TurnResp {
        user_message,
        agent_message,
        story_updated,
        version_no,
        actions: parsed.actions,
    }))
}

/// `POST /product/refinement-threads/{tid}/apply` — apply ONE card the agent
/// proposed in this conversation (Editor): questions, notes or a diagram, on
/// the shared rules in `chat_actions`.
pub async fn apply_action<C: ProductStudioHost>(
    Path(tid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApplyReq>,
) -> ApiResult<Json<ApplyResult>> {
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;
    let target = ActionTarget {
        story_id: &thread.story_id,
        workspace_id: &thread.workspace_id,
        user_id: &user.id,
    };
    let result = apply_story_action(ctx.product(), ctx.canvas_repo(), &target, &req.action)
        .await
        .map_err(ApiError)?;
    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested, no DB)
// ---------------------------------------------------------------------------

/// The agent message's meta: the turn outcome, plus the cards when there are
/// any (the UI renders them from here when it reloads the thread).
fn turn_meta(version_no: Option<i64>, actions: &[Value]) -> Value {
    let mut meta = json!({ "story_updated": version_no.is_some(), "version_no": version_no });
    if !actions.is_empty() {
        meta["actions"] = Value::Array(actions.to_vec());
    }
    meta
}

/// Build the one-shot turn prompt: framing + the JSON contract, the story
/// bundle, the replayed history and the new PO message last. The contract is
/// load-bearing (`parse_turn` reads the reply against it). The first line is
/// the `OTTO_TASK: discovery_chat` sentinel, which routes the deterministic
/// E2E stub (whose prose + `actions` reply `parse_turn` also reads).
fn build_turn_prompt(context: &str, history: &[RefinementMessage], new_message: &str) -> String {
    let mut s = String::new();

    // 1. Framing + the JSON contract (verbatim — load-bearing).
    s.push_str(
        "OTTO_TASK: discovery_chat\n\
         You are the Product Owner's partner on this story. The story may be empty — \
         help research, ask the right questions, surface edge cases, and improve \
         clarity, completeness and acceptance criteria. When the PO asks for changes, \
         produce an improved FULL story body; it is saved as a suggestion the PO reviews, \
         never applied directly. Respond ONLY as a single JSON object: \
         {\"reply\": \"conversational reply to the PO, in markdown\", \"updated_story_md\": \
         \"full new story markdown, or null if no change this turn\", \"summary\": \"≤1 line \
         of what changed, or null\", \"actions\": [optional concrete proposals the PO \
         approves one by one]}.\n\
         Supported actions:\n\
           {\"type\":\"add_questions\",\"questions\":[{\"text\":\"…\",\"rationale\":\"…\",\"category\":\"…\"}]}\n\
           {\"type\":\"add_notes\",\"notes\":[{\"body\":\"…\"}]}\n\
           {\"type\":\"create_canvas\",\"title\":\"…\",\"mermaid\":\"sequenceDiagram…\"}\n\
         Omit `actions` when you have no concrete proposal. A story body goes in \
         `updated_story_md`, never in an action.\n\n",
    );

    // 2. The story bundle.
    if !context.trim().is_empty() {
        s.push_str("# Context\n");
        s.push_str(context);
        s.push('\n');
    }

    // 3. Replayed history (chronological).
    if !history.is_empty() {
        s.push_str("## Conversation so far\n");
        for m in history {
            let who = if m.role == "agent" { "Agent" } else { "PO" };
            s.push_str(&format!("{who}: {}\n", m.body));
        }
        s.push('\n');
    }

    // 4. The new PO message, last.
    s.push_str(&format!("## New message from the PO\nPO: {new_message}\n"));

    s
}

/// One parsed turn reply.
#[derive(Debug, Default, PartialEq)]
struct ParsedTurn {
    reply: String,
    /// A proposed full story body (→ a `suggested` version).
    updated_story_md: Option<String>,
    /// The proposal's title, when the agent named one (an `apply_draft`).
    title: Option<String>,
    summary: Option<String>,
    /// Card actions only (see `chat_actions::CARD_ACTIONS`).
    actions: Vec<Value>,
}

/// Parse a turn reply. Tolerant, never panics, never loses the agent's words:
/// - a `{reply, updated_story_md?, summary?, actions?}` object is the contract;
/// - otherwise prose + a fenced/bare `{actions:[…]}` block (the earlier
///   Discovery Chat shape) is split by `chat::split_actions`;
/// - anything else is `reply = raw` with no update and no cards.
///
/// An `apply_draft` action never becomes a card: its `body_md` is the
/// proposed body when `updated_story_md` is absent (Refine's rule: a body is
/// always a suggestion). JSON `null` / empty strings are `None`.
fn parse_turn(raw: &str) -> ParsedTurn {
    let text = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    // Shape predicate: a quoted `{…}` before the real object (a config
    // snippet, an example) must not shadow it — taking the first span fell
    // back to the raw text and DROPPED `updated_story_md`.
    let mut turn = ParsedTurn::default();
    let raw_actions: Vec<Value> = match otto_core::text::extract_json_where(raw, |v| {
        v.get("reply").is_some_and(Value::is_string)
    }) {
        Some(v) => {
            turn.reply = v["reply"].as_str().unwrap_or_default().to_string();
            turn.updated_story_md = text(&v, "updated_story_md");
            turn.summary = text(&v, "summary");
            v.get("actions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        }
        None => {
            let (markdown, actions_json) = crate::chat::split_actions(raw);
            turn.reply = if actions_json.is_some() {
                markdown
            } else {
                raw.to_string()
            };
            actions_json
                .and_then(|j| serde_json::from_str::<Vec<Value>>(&j).ok())
                .unwrap_or_default()
        }
    };
    if turn.updated_story_md.is_none() {
        if let Some(draft) = raw_actions
            .iter()
            .find(|a| a.get("type").and_then(Value::as_str) == Some("apply_draft"))
        {
            turn.updated_story_md = text(draft, "body_md");
            if turn.updated_story_md.is_some() {
                turn.title = text(draft, "title");
            }
        }
    }
    turn.actions = card_actions(&raw_actions);
    turn
}

/// Stop exactly the caller's turn and wait for managed-session cleanup.
pub async fn stop_turn<C: ProductStudioHost>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<crate::chat::StopTurnReq>,
) -> ApiResult<Json<crate::chat::StopTurnResp>> {
    let conversation = thread_with_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let stopped =
        crate::studio_turn::stop("refinement", &conversation.workspace_id, &id, &req.turn_id).await;
    Ok(Json(crate::chat::StopTurnResp { stopped }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_state::RefinementMessage;

    fn msg(role: &str, body: &str) -> RefinementMessage {
        RefinementMessage {
            id: "m".into(),
            thread_id: "t".into(),
            role: role.into(),
            body: body.into(),
            meta_json: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn prompt_includes_context_history_and_the_actions_contract() {
        let history = vec![msg("user", "add edge cases"), msg("agent", "sure, which?")];
        let context = "## Latest version\n# Login flow\n\nAs a user I log in.\n\n## Open questions\n- lockout?\n";
        let p = build_turn_prompt(context, &history, "tighten the AC for lockout");
        assert!(p.starts_with("OTTO_TASK: discovery_chat\n")); // E2E stub sentinel
        assert!(p.contains("single JSON object")); // framing + JSON contract
        for kind in ["add_questions", "add_notes", "create_canvas"] {
            assert!(p.contains(kind), "contract names {kind}");
        }
        assert!(!p.contains("apply_draft"), "a body is never an action");
        assert!(p.contains("As a user I log in.")); // story bundle
        assert!(p.contains("- lockout?"));
        assert!(p.contains("PO: add edge cases")); // replayed history
        assert!(p.contains("Agent: sure, which?"));
        assert!(p.ends_with("PO: tighten the AC for lockout\n")); // new message last
    }

    #[test]
    fn parse_extracts_reply_and_updated_body() {
        let raw = r##"Here you go: {"reply":"updated the ACs","updated_story_md":"# New body","summary":"added lockout AC"}"##;
        let t = parse_turn(raw);
        assert_eq!(t.reply, "updated the ACs");
        assert_eq!(t.updated_story_md.as_deref(), Some("# New body"));
        assert_eq!(t.summary.as_deref(), Some("added lockout AC"));
        assert!(t.actions.is_empty());
    }

    #[test]
    fn parse_malformed_preserves_reply_no_update() {
        let raw = "I couldn't format JSON but here are my thoughts...";
        let t = parse_turn(raw);
        assert_eq!(t.reply, raw); // whole raw reply preserved
        assert!(t.updated_story_md.is_none()); // no story update
        assert!(t.summary.is_none());
        assert!(t.actions.is_empty());
    }

    #[test]
    fn parse_null_updated_is_none() {
        let raw = r#"{"reply":"no change needed","updated_story_md":null,"summary":null}"#;
        let t = parse_turn(raw);
        assert_eq!(t.reply, "no change needed");
        assert!(t.updated_story_md.is_none()); // JSON null → None (no version written)
    }

    #[test]
    fn parse_skips_a_decoy_object_before_the_reply() {
        // A quoted config object comes first; the turn object follows. The
        // first-span parse fell back to raw text and dropped the update.
        let raw = r##"The flag lives in `{"feature":"lockout","enabled":true}`. {"reply":"added the lockout AC","updated_story_md":"# Body v2"}"##;
        let t = parse_turn(raw);
        assert_eq!(t.reply, "added the lockout AC");
        assert_eq!(t.updated_story_md.as_deref(), Some("# Body v2"));
    }

    #[test]
    fn a_turn_returns_card_actions_beside_the_reply() {
        let raw = r##"{"reply":"Two things to pin down.","updated_story_md":null,"actions":[
            {"type":"add_questions","questions":[{"text":"Who can unlock?","category":"security"}]},
            {"type":"add_notes","notes":[{"body":"billing has a lockout too"}]},
            {"type":"create_canvas","title":"Lockout","mermaid":"flowchart TD\n A-->B"},
            {"type":"delete_everything"}
        ]}"##;
        let t = parse_turn(raw);
        assert_eq!(t.reply, "Two things to pin down.");
        assert!(t.updated_story_md.is_none());
        let kinds: Vec<_> = t
            .actions
            .iter()
            .map(|a| a["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["add_questions", "add_notes", "create_canvas"]);
        // The cards ride on the agent message so a reload renders them.
        let meta = turn_meta(None, &t.actions);
        assert_eq!(meta["story_updated"], false);
        assert_eq!(meta["actions"].as_array().unwrap().len(), 3);
        assert!(turn_meta(Some(4), &[]).get("actions").is_none());
    }

    #[test]
    fn a_discovery_style_apply_draft_becomes_the_suggested_body_not_a_card() {
        // The earlier Discovery Chat shape (and the E2E stub's reply): prose +
        // a fenced actions block whose `apply_draft` carries the body.
        let raw = "Here's a first cut.\n\n```json\n{\"actions\":[\
            {\"type\":\"apply_draft\",\"title\":\"Lockout\",\"body_md\":\"## Summary\\nLock after 5 tries\"},\
            {\"type\":\"add_questions\",\"questions\":[{\"text\":\"SLA?\"}]}]}\n```";
        let t = parse_turn(raw);
        assert_eq!(t.reply, "Here's a first cut.");
        assert_eq!(
            t.updated_story_md.as_deref(),
            Some("## Summary\nLock after 5 tries")
        );
        assert_eq!(t.title.as_deref(), Some("Lockout"));
        assert_eq!(t.actions.len(), 1, "only the question card remains");
        assert_eq!(t.actions[0]["type"], "add_questions");
    }

    #[test]
    fn an_explicit_body_wins_over_an_apply_draft_action() {
        let raw = r##"{"reply":"ok","updated_story_md":"# From the contract","actions":[{"type":"apply_draft","title":"X","body_md":"# From the action"}]}"##;
        let t = parse_turn(raw);
        assert_eq!(t.updated_story_md.as_deref(), Some("# From the contract"));
        assert!(t.title.is_none());
        assert!(t.actions.is_empty());
    }
}

//! Talk-to-agent story refinement: a conversational thread on a Product story.
//!
//! The PO chats with an agent that holds the full thread history + the story +
//! (optionally) a linked discovery run's findings + the story's attachments, and
//! can edit the story by emitting a new `suggested` version that the existing
//! Publish-as-Jira/RFC buttons pick up.
//!
//! Each turn is one-shot `orchestrator.run_agent(prompt, cwd, model, no_progress)`
//! with the **whole history replayed in the prompt**. The agent returns a single
//! JSON object `{reply, updated_story_md?, summary?}`; when `updated_story_md` is
//! present the backend writes a new `suggested` story version (non-destructive —
//! the same kind `to-swarm`/discovery already treat as the refined body) and emits
//! a `ProductChanged` refresh event. No PTY/session coupling, no ingest plumbing.
//!
//! Routes (registered in `orchestrator_routes()` because the turn engine needs the
//! product repo, the refinement repo, and the orchestrator together):
//!   POST /api/v1/product/stories/{sid}/refinement-threads   (ws editor) → RefinementThread
//!   GET  /api/v1/product/stories/{sid}/refinement-threads   (ws viewer) → RefinementThread[]
//!   GET  /api/v1/product/refinement-threads/{tid}           (ws viewer) → {thread, messages}
//!   POST /api/v1/product/refinement-threads/{tid}/messages  (ws editor) → TurnResp
//!   POST /api/v1/product/refinement-threads/{tid}/archive   (ws editor) → RefinementThread

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Error, Id};
use otto_state::{
    NewRefinementMessage, NewRefinementThread, NewVersion, RefinementMessage, RefinementThread,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// No-progress window for a refinement turn — a *stuck* window, NOT a wall-clock
/// cap (the hard PTY cap lives in `claude_pty.rs`). Matches the discovery planner.
const REFINE_NO_PROGRESS: Duration = Duration::from_secs(150);
/// Replay at most the last N messages into the prompt (bound prompt growth).
const HISTORY_TURN_CAP: usize = 40;
/// Char budget for the discovery report + each task summary in the prompt.
const DISCOVERY_BUDGET: usize = 1600;

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
}

/// Result of one conversational turn: the persisted user + agent messages and
/// whether the story was edited this turn (with the new version number).
#[derive(Debug, Serialize)]
pub struct TurnResp {
    pub user_message: RefinementMessage,
    pub agent_message: RefinementMessage,
    pub story_updated: bool,
    pub version_no: Option<i64>,
}

// ---------------------------------------------------------------------------
// Workspace resolution helper
// ---------------------------------------------------------------------------

/// Load a thread, resolve its story → workspace, and role-check the caller.
/// Returns the loaded thread on success (404 when the thread is absent).
async fn thread_with_role(
    ctx: &ServerCtx,
    user: &User,
    tid: &Id,
    role: WorkspaceRole,
) -> ApiResult<RefinementThread> {
    let thread = ctx
        .refinement_repo
        .get_thread(tid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("refinement thread {tid}"))))?;
    let story = ctx
        .product_repo
        .get_story(&thread.story_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(ctx, user, &story.workspace_id, role).await?;
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
pub async fn create_thread(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<CreateThreadReq>>,
) -> ApiResult<Json<RefinementThread>> {
    let req = body.map(|b| b.0).unwrap_or_default();

    // Resolve the story + Editor role-check via its workspace.
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;

    // Pick the working dir: prefer the repo path the discovery used (when the
    // request links a resolvable run AND the story has a cwd); else a fresh,
    // per-thread scratch dir. The scratch dir name is a fresh ULID so it's unique
    // regardless of the row id the repo generates.
    let discovery_run_id = req.discovery_run_id.filter(|r| !r.trim().is_empty());
    let mut cwd: Option<String> = None;
    if let Some(run_id) = &discovery_run_id {
        if let Ok(Some(_run)) = ctx.discovery_repo.get(run_id).await {
            if let Some(story_cwd) = &story.cwd {
                if !story_cwd.trim().is_empty() {
                    cwd = Some(story_cwd.clone());
                }
            }
        }
    }
    let cwd = cwd.unwrap_or_else(|| {
        ctx.data_dir
            .join("product/refine")
            .join(new_id())
            .to_string_lossy()
            .to_string()
    });

    let title = req
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Refinement".to_string());

    let thread = ctx
        .refinement_repo
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
pub async fn list_threads(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<RefinementThread>>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Viewer).await?;
    let threads = ctx
        .refinement_repo
        .list_threads_for_story(&sid)
        .await
        .map_err(ApiError)?;
    Ok(Json(threads))
}

/// `GET /product/refinement-threads/{tid}` — a thread + its full transcript
/// (Viewer).
pub async fn get_thread(
    Path(tid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ThreadDetail>> {
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Viewer).await?;
    let messages = ctx
        .refinement_repo
        .list_messages(&tid)
        .await
        .map_err(ApiError)?;
    Ok(Json(ThreadDetail { thread, messages }))
}

/// `POST /product/refinement-threads/{tid}/archive` — archive a thread (Editor),
/// returning the reloaded (archived) thread.
pub async fn archive_thread(
    Path(tid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<RefinementThread>> {
    thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;
    ctx.refinement_repo
        .archive_thread(&tid)
        .await
        .map_err(ApiError)?;
    let thread = ctx
        .refinement_repo
        .get_thread(&tid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("refinement thread {tid}"))))?;
    Ok(Json(thread))
}

/// `POST /product/refinement-threads/{tid}/messages` — one conversational turn
/// (Editor). The synchronous turn engine (see module docs / spec §6):
///   1. Resolve thread + Editor.
///   2. Persist the user message.
///   3. Gather context: current body, attachments-by-path, discovery (optional),
///      history (last `HISTORY_TURN_CAP`).
///   4. Build the one-shot prompt.
///   5. `create_dir_all(&thread.cwd)` (best-effort) BEFORE the agent run.
///   6. `run_agent` (whole history replayed in the prompt).
///   7. Parse `{reply, updated_story_md?, summary?}` (tolerant; malformed → reply=raw).
///   8. On a non-empty `updated_story_md`: write a `suggested` version + emit
///      `ProductChanged`.
///   9. Persist the agent message (with `{story_updated, version_no}` meta) +
///      record a Product event.
///  10. Return the user + agent messages and the update result.
pub async fn send_message(
    Path(tid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SendMessageReq>,
) -> ApiResult<Json<TurnResp>> {
    // 1. Resolve thread + Editor role-check.
    let thread = thread_with_role(&ctx, &user, &tid, WorkspaceRole::Editor).await?;
    let story = ctx
        .product_repo
        .get_story(&thread.story_id)
        .await
        .map_err(ApiError)?;

    // 2. Persist the user message.
    let user_message = ctx
        .refinement_repo
        .create_message(NewRefinementMessage {
            thread_id: tid.clone(),
            role: "user".into(),
            body: req.body.clone(),
            meta_json: None,
        })
        .await
        .map_err(ApiError)?;

    // 3a. Current story body: latest `suggested` → `source` → the title.
    let story_body = match ctx
        .product_repo
        .latest_version_of_kind(&thread.story_id, "suggested")
        .await
        .ok()
        .flatten()
        .map(|v| v.body_md)
    {
        Some(b) if !b.trim().is_empty() => b,
        _ => match ctx
            .product_repo
            .latest_version_of_kind(&thread.story_id, "source")
            .await
            .ok()
            .flatten()
            .map(|v| v.body_md)
        {
            Some(b) if !b.trim().is_empty() => b,
            _ => story.title.clone(),
        },
    };

    // 3b. Attachments by absolute path (never copied — the agent opens them).
    let attachment_lines: Vec<String> = ctx
        .attachment_repo
        .list_for_story(&thread.story_id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|a| {
            format!(
                "- {} ({}) at {}",
                a.filename,
                a.mime,
                ctx.data_dir.join(&a.storage_path).display()
            )
        })
        .collect();

    // 3c. Discovery context (optional): the run's report + its task titles.
    let mut discovery_report: Option<String> = None;
    let mut discovery_task_summaries: Vec<String> = Vec::new();
    if let Some(run_id) = &thread.discovery_run_id {
        if let Ok(Some(run)) = ctx.discovery_repo.get(run_id).await {
            discovery_report = run.report_md.filter(|r| !r.trim().is_empty());
            if let Ok(tasks) = ctx.swarm_repo.list_tasks(&run.project_id).await {
                discovery_task_summaries = tasks
                    .into_iter()
                    .map(|t| t.title)
                    .filter(|t| !t.trim().is_empty())
                    .collect();
            }
        }
    }

    // 3d. History — replay the last `HISTORY_TURN_CAP` messages (chronological).
    let all_history = ctx
        .refinement_repo
        .list_messages(&tid)
        .await
        .unwrap_or_default();
    let history: Vec<RefinementMessage> = all_history
        .iter()
        .rev()
        .take(HISTORY_TURN_CAP)
        .rev()
        .cloned()
        .collect();

    // 4. Build the one-shot prompt.
    let prompt = build_refinement_prompt(
        &story.title,
        &story_body,
        discovery_report.as_deref(),
        &discovery_task_summaries,
        &attachment_lines,
        &history,
        &req.body,
    );

    // 5. Ensure the thread cwd exists (run_prompt canonicalizes it and degrades
    //    if it's missing). Best-effort — logged on error.
    if let Err(e) = std::fs::create_dir_all(&thread.cwd) {
        warn!("product_refine: create_dir_all({}) failed: {e}", thread.cwd);
    }

    // 6. Run the agent (whole history replayed in the prompt).
    let raw = ctx
        .orchestrator
        .run_agent(
            &prompt,
            &thread.cwd,
            thread.model.as_deref(),
            REFINE_NO_PROGRESS,
        )
        .await
        .map_err(ApiError)?;

    // 7. Parse — tolerant; malformed/missing → reply=raw, no update.
    let (reply, updated_story_md, summary) = parse_turn(&raw);

    // 8. On a non-empty updated body: write a `suggested` version + emit refresh.
    let mut story_updated = false;
    let mut version_no: Option<i64> = None;
    if let Some(updated) = updated_story_md.as_ref().filter(|u| !u.trim().is_empty()) {
        let v = ctx
            .product_repo
            .add_version(NewVersion {
                story_id: thread.story_id.clone(),
                kind: "suggested".into(),
                title: story.title.clone(),
                body_md: updated.clone(),
                raw_json: None,
                change_notes: summary.clone(),
                created_by: user.id.clone(),
            })
            .await
            .map_err(ApiError)?;
        version_no = Some(v.version_no);
        story_updated = true;
        let _ = ctx.events.send(otto_core::event::Event::ProductChanged {
            workspace_id: story.workspace_id.clone(),
            story_id: thread.story_id.clone(),
            section: "refine".into(),
            status: "suggested".into(),
        });
    }

    // 9. Persist the agent message (with the turn-outcome meta) + a Product event.
    let agent_message = ctx
        .refinement_repo
        .create_message(NewRefinementMessage {
            thread_id: tid.clone(),
            role: "agent".into(),
            body: reply,
            meta_json: Some(
                json!({ "story_updated": story_updated, "version_no": version_no }).to_string(),
            ),
        })
        .await
        .map_err(ApiError)?;
    let _ = ctx
        .product_repo
        .add_event(otto_state::NewEvent {
            story_id: thread.story_id.clone(),
            section: "refine".into(),
            kind: "refined".into(),
            summary: summary.unwrap_or_else(|| "Refinement turn".into()),
            actor_id: Some(user.id.clone()),
            meta_json: Some(json!({ "thread_id": tid, "version_no": version_no }).to_string()),
        })
        .await;

    // 10. Return both messages + the update result.
    Ok(Json(TurnResp {
        user_message,
        agent_message,
        story_updated,
        version_no,
    }))
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested, no DB)
// ---------------------------------------------------------------------------

/// Build the one-shot refinement prompt: system framing + the current story +
/// (optional) discovery context + attachments-by-path + the replayed history +
/// the new PO message last. The JSON contract in the framing is load-bearing —
/// the turn engine parses the agent's reply against it.
#[allow(clippy::too_many_arguments)]
fn build_refinement_prompt(
    story_title: &str,
    story_body: &str,
    discovery_report: Option<&str>,
    discovery_task_summaries: &[String],
    attachment_lines: &[String],
    history: &[RefinementMessage],
    new_message: &str,
) -> String {
    let mut s = String::new();

    // 1. System framing + the JSON contract (verbatim — load-bearing).
    s.push_str(
        "You are refining a product story with the Product Owner. Improve clarity, \
         completeness, acceptance criteria, and edge cases. When the PO asks for changes, \
         produce an improved FULL story body. Respond ONLY as a single JSON object: \
         {\"reply\": \"conversational reply to the PO\", \"updated_story_md\": \"full new \
         story markdown, or null if no change this turn\", \"summary\": \"≤1 line of what \
         changed, or null\"}.\n\n",
    );

    // 2. Current story.
    s.push_str(&format!(
        "## Current story\n# {story_title}\n\n{story_body}\n\n"
    ));

    // 3. Discovery context (only when a report is present).
    if let Some(report) = discovery_report {
        s.push_str("## Discovery findings\n");
        s.push_str(&truncate(report, DISCOVERY_BUDGET));
        s.push('\n');
        if !discovery_task_summaries.is_empty() {
            for summary in discovery_task_summaries {
                s.push_str(&format!("- {}\n", truncate(summary, DISCOVERY_BUDGET)));
            }
        }
        s.push('\n');
    }

    // 4. Attachments (absolute paths; never copied).
    if !attachment_lines.is_empty() {
        s.push_str("## Attachments (open with your file tools)\n");
        s.push_str(&attachment_lines.join("\n"));
        s.push_str("\n\n");
    }

    // 5. Replayed history (chronological).
    if !history.is_empty() {
        s.push_str("## Conversation so far\n");
        for m in history {
            let who = if m.role == "agent" { "Agent" } else { "PO" };
            s.push_str(&format!("{who}: {}\n", m.body));
        }
        s.push('\n');
    }

    // 6. The new PO message, last.
    s.push_str(&format!("## New message from the PO\nPO: {new_message}\n"));

    s
}

/// Parse a refinement turn reply into `(reply, updated_story_md, summary)`.
/// Tolerant: a parseable `{reply, updated_story_md?, summary?}` object is
/// extracted; anything malformed/missing falls back to `reply = raw` with no
/// update (never panics, never loses the agent's words). JSON `null` (or empty)
/// for `updated_story_md`/`summary` → `None`.
fn parse_turn(raw: &str) -> (String, Option<String>, Option<String>) {
    let Some(v) = otto_swarm::recruiter::extract_json(raw) else {
        return (raw.to_string(), None, None);
    };
    let reply = v
        .get("reply")
        .and_then(|r| r.as_str())
        .map(|r| r.to_string());
    let Some(reply) = reply else {
        // Parseable JSON but no usable `reply` field — preserve the raw words.
        return (raw.to_string(), None, None);
    };
    let updated = v
        .get("updated_story_md")
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .filter(|u| !u.trim().is_empty());
    let summary = v
        .get("summary")
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    (reply, updated, summary)
}

/// Truncate `text` to at most `max` chars, appending an ellipsis when cut. Keeps
/// the prompt bounded so a sprawling report/summary doesn't blow the budget.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
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
    fn prompt_includes_history_story_and_discovery_context() {
        let history = vec![msg("user", "add edge cases"), msg("agent", "sure, which?")];
        let p = build_refinement_prompt(
            "Login flow",
            "As a user I log in.",
            Some("Affected: auth-svc, gateway."),
            &["Map auth flow".to_string()],
            &["- mock.png (image/png) at /data/product/attachments/s1/a1.png".to_string()],
            &history,
            "tighten the AC for lockout",
        );
        assert!(p.contains("single JSON object")); // framing + JSON contract
        assert!(p.contains("As a user I log in.")); // story body
        assert!(p.contains("Affected: auth-svc")); // discovery report
        assert!(p.contains("Map auth flow")); // task summary
        assert!(p.contains("/data/product/attachments/s1/a1.png")); // attachment by abs path
        assert!(p.contains("PO: add edge cases")); // replayed history
        assert!(p.contains("Agent: sure, which?"));
        assert!(p.contains("tighten the AC for lockout")); // new message last
    }

    #[test]
    fn parse_extracts_reply_and_updated_body() {
        let raw = r##"Here you go: {"reply":"updated the ACs","updated_story_md":"# New body","summary":"added lockout AC"}"##;
        let (reply, updated, summary) = parse_turn(raw);
        assert_eq!(reply, "updated the ACs");
        assert_eq!(updated.as_deref(), Some("# New body"));
        assert_eq!(summary.as_deref(), Some("added lockout AC"));
    }

    #[test]
    fn parse_malformed_preserves_reply_no_update() {
        let raw = "I couldn't format JSON but here are my thoughts...";
        let (reply, updated, summary) = parse_turn(raw);
        assert_eq!(reply, raw); // whole raw reply preserved
        assert!(updated.is_none()); // no story update
        assert!(summary.is_none());
    }

    #[test]
    fn parse_null_updated_is_none() {
        let raw = r#"{"reply":"no change needed","updated_story_md":null,"summary":null}"#;
        let (reply, updated, _summary) = parse_turn(raw);
        assert_eq!(reply, "no change needed");
        assert!(updated.is_none()); // JSON null → None (no version written)
    }
}

//! Agent activity endpoints: per-session live trail + normalized task tracker,
//! plus the provider ingest endpoint the injected agent hooks post to.
//!
//! Authed routes (`/workspaces/{wid}/sessions/{sid}/trail|tasks`,
//! `/workspaces/{wid}/activity/summary`) serve the UI. The ingest route
//! (`/ingest/claude`) is unauthenticated but gated by the per-session token Otto
//! sets on the agent's PTY; it normalizes a provider's native hook payload into
//! Otto's unified trail/task model, raises task-transition trail lines, and
//! surfaces milestone/blocked notifications — the one place that knows each
//! provider's quirks ("the wrapper").

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_core::api::{AppendTrailReq, PutTasksReq};
use otto_core::domain::{
    AgentTask, NoticeAction, NoticeKind, NoticeSeverity, Session, SessionActivitySummary,
    TaskStatus, TrailEvent, TrailKind, TrailLevel, TrailSource, WorkspaceRole,
};
use otto_core::Id;
use otto_state::{NewNotice, NewTask, NewTrail};
use serde_json::Value;

use crate::auth::{require_session_owner_or_admin, require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Cap on trail entries returned (newest kept, then ordered oldest→newest).
const TRAIL_LIMIT: i64 = 500;

/// Ensure `sid` exists and lives in `wid`, returning the loaded session.
async fn session_in_ws(ctx: &ServerCtx, wid: &Id, sid: &Id) -> Result<Session, ApiError> {
    let session = ctx.manager.get(sid).await.map_err(ApiError)?;
    if &session.workspace_id != wid {
        return Err(ApiError(otto_core::Error::NotFound(
            "session is not in this workspace".into(),
        )));
    }
    Ok(session)
}

/// `GET /workspaces/{wid}/sessions/{sid}/trail` — session owner or ws-admin.
///
/// A workspace Viewer/Editor who is **not** the session owner receives 403.
/// The owner, a workspace Admin, and root all have access (same gate as every
/// other per-session endpoint: `require_session_owner_or_admin`).
pub async fn list_trail(
    Path((wid, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<TrailEvent>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let session = session_in_ws(&ctx, &wid, &sid).await?;
    require_session_owner_or_admin(&ctx, &user, &session).await?;
    let trail = ctx
        .activity()
        .repo()
        .list_trail(&sid, TRAIL_LIMIT)
        .await
        .map_err(ApiError)?;
    Ok(Json(trail))
}

/// `POST /workspaces/{wid}/sessions/{sid}/trail` — editor. Appends one entry
/// (used by the UI for human "notes"; defaults to source=user, kind=note).
pub async fn append_trail(
    Path((wid, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AppendTrailReq>,
) -> ApiResult<Json<TrailEvent>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    session_in_ws(&ctx, &wid, &sid).await?;
    let source = req
        .source
        .as_deref()
        .and_then(TrailSource::parse)
        .unwrap_or(TrailSource::User);
    let kind = req
        .kind
        .as_deref()
        .and_then(TrailKind::parse)
        .unwrap_or(TrailKind::Note);
    let level = req
        .level
        .as_deref()
        .and_then(TrailLevel::parse)
        .unwrap_or(TrailLevel::Info);
    let event = ctx
        .activity()
        .append_trail(NewTrail {
            session_id: sid,
            workspace_id: wid,
            source,
            kind,
            level,
            summary: req.summary,
            detail: req.detail,
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(event))
}

/// `GET /workspaces/{wid}/sessions/{sid}/tasks` — session owner or ws-admin.
///
/// A workspace Viewer/Editor who is **not** the session owner receives 403.
/// The owner, a workspace Admin, and root all have access.
pub async fn list_tasks(
    Path((wid, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AgentTask>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let session = session_in_ws(&ctx, &wid, &sid).await?;
    require_session_owner_or_admin(&ctx, &user, &session).await?;
    let tasks = ctx
        .activity()
        .repo()
        .list_tasks(&sid)
        .await
        .map_err(ApiError)?;
    Ok(Json(tasks))
}

/// `PUT /workspaces/{wid}/sessions/{sid}/tasks` — editor. Replaces the whole
/// task list (manual override; providers sync via the ingest path).
pub async fn put_tasks(
    Path((wid, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PutTasksReq>,
) -> ApiResult<Json<Vec<AgentTask>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    session_in_ws(&ctx, &wid, &sid).await?;
    let tasks: Vec<NewTask> = req
        .tasks
        .into_iter()
        .map(|t| NewTask {
            ext_id: t.ext_id,
            title: t.title,
            status: TaskStatus::parse(&t.status).unwrap_or(TaskStatus::Pending),
        })
        .collect();
    let tasks = ctx
        .activity()
        .put_tasks(&sid, &wid, &tasks)
        .await
        .map_err(ApiError)?;
    Ok(Json(tasks))
}

/// `GET /workspaces/{wid}/activity/summary` — viewer. Per-session task roll-up
/// for the multi-agent overview (sidebar chips).
///
/// **Admin / root:** full workspace aggregate (all users' sessions).
/// **Non-admin (editor/viewer/owner):** restricted to the caller's own sessions
/// (`created_by = caller`), preventing cross-user data leakage (#L18).
pub async fn workspace_summary(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<SessionActivitySummary>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    // Root and workspace-Admins see the full cross-user roll-up; everyone else
    // sees only their own sessions' activity to prevent data leakage (#L18).
    let is_admin = user.is_root
        || ctx
            .roles
            .check(&user, &wid, WorkspaceRole::Admin)
            .await
            .is_ok();
    let summary = if is_admin {
        ctx.activity()
            .repo()
            .workspace_summary(&wid)
            .await
            .map_err(ApiError)?
    } else {
        ctx.activity()
            .repo()
            .workspace_summary_for_user(&wid, &user.id)
            .await
            .map_err(ApiError)?
    };
    Ok(Json(summary))
}

// ---------------------------------------------------------------------------
// Provider ingest (unauthenticated, per-session token gated)
// ---------------------------------------------------------------------------

/// `POST /ingest/claude` — receives a Claude Code hook payload (raw JSON on the
/// body) and normalizes it into the session's trail + task tracker, raising
/// task-transition lines and milestone/blocked notifications. Identity is the
/// `X-Otto-Session` + `X-Otto-Token` headers, verified against the
/// SessionManager's per-session token. Always returns 204 (fire-and-forget for
/// the agent's hook), even on unknown payloads.
pub async fn claude_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        // Wrong/absent token — silently ignore (never surface to the agent).
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    let wid = session.workspace_id.clone();
    let event_name = payload
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let norm = normalize_claude(&payload);
    let activity = ctx.activity();

    // Task sync + transition trail + "all done" milestone.
    if let Some(new_tasks) = norm.tasks {
        let prior = activity.repo().list_tasks(&sid).await.unwrap_or_default();
        let prior_all_done = all_done(&prior);
        if let Ok(updated) = activity.put_tasks(&sid, &wid, &new_tasks).await {
            if let Some(summary) = task_transition_summary(&prior, &updated) {
                let _ = activity
                    .append_trail(NewTrail {
                        session_id: sid.clone(),
                        workspace_id: wid.clone(),
                        source: TrailSource::Agent,
                        kind: TrailKind::Task,
                        level: TrailLevel::Info,
                        summary,
                        detail: None,
                    })
                    .await;
            }
            // Notify only on the transition into "all complete".
            if !prior_all_done && all_done(&updated) {
                let done = updated.len();
                notify(
                    &ctx,
                    &session,
                    NoticeSeverity::Info,
                    "Agent finished its tasks",
                    format!(
                        "{} · {done} task{} complete",
                        session.title,
                        if done == 1 { "" } else { "s" }
                    ),
                    "tasks_done",
                )
                .await;
            }
        }
    }

    // Non-task trail entry.
    if let Some(d) = norm.trail {
        let _ = activity
            .append_trail(NewTrail {
                session_id: sid.clone(),
                workspace_id: wid.clone(),
                source: d.source,
                kind: d.kind,
                level: d.level,
                summary: d.summary,
                detail: d.detail,
            })
            .await;
    }

    // Blocked / needs-attention: Claude fires a Notification hook when it is
    // waiting on the user (input or a permission it couldn't auto-accept).
    if event_name == "Notification" {
        let msg = payload
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Agent is waiting");
        notify(
            &ctx,
            &session,
            NoticeSeverity::Warn,
            "Agent needs attention",
            format!("{} · {}", session.title, clip(msg, 160)),
            "waiting",
        )
        .await;
    }

    StatusCode::NO_CONTENT
}

/// `POST /ingest/codex` — receives a Codex `notify` payload (raw JSON body) and
/// records it. Codex's notify surface is coarse (turn completion / approval
/// needed), so the trail this produces is sparse compared to Claude's; richer
/// codex activity comes from Otto-side lifecycle + user capture. Same per-session
/// token gate as the claude ingest.
pub async fn codex_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> StatusCode {
    let Some(session) = ingest_session(&ctx, &headers).await else {
        return StatusCode::NO_CONTENT;
    };
    if let Some(d) = normalize_codex(&payload) {
        let _ = ctx
            .activity()
            .append_trail(NewTrail {
                session_id: session.id.clone(),
                workspace_id: session.workspace_id.clone(),
                source: d.source,
                kind: d.kind,
                level: d.level,
                summary: d.summary,
                detail: d.detail,
            })
            .await;
    }
    StatusCode::NO_CONTENT
}

/// Verify the `X-Otto-Session`/`X-Otto-Token` headers against the per-session
/// ingest token and return the loaded session, or `None` to silently ignore.
async fn ingest_session(ctx: &ServerCtx, headers: &HeaderMap) -> Option<Session> {
    let sid: Id = headers.get("x-otto-session").and_then(|v| v.to_str().ok())?.to_string();
    let token = headers.get("x-otto-token").and_then(|v| v.to_str().ok()).unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return None;
    }
    ctx.manager.get(&sid).await.ok()
}

/// Map a Codex `notify` payload to a trail entry. `agent-turn-complete` is the
/// common event; anything else is surfaced generically.
fn normalize_codex(p: &Value) -> Option<TrailDraft> {
    let kind = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "agent-turn-complete" => Some(TrailDraft {
            source: TrailSource::Agent,
            kind: TrailKind::Session,
            level: TrailLevel::Info,
            summary: "Agent finished responding".to_string(),
            detail: None,
        }),
        "" => None,
        other => Some(TrailDraft {
            source: TrailSource::Agent,
            kind: TrailKind::Other,
            level: TrailLevel::Info,
            summary: clip(other, 80),
            detail: None,
        }),
    }
}

/// Persist + broadcast a per-session notice, de-duped on a stable source key.
async fn notify(
    ctx: &ServerCtx,
    session: &Session,
    severity: NoticeSeverity,
    title: &str,
    body: String,
    key_suffix: &str,
) {
    let _ = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::Session,
            severity,
            title: title.to_string(),
            body,
            source_key: Some(format!("session:{}:{key_suffix}", session.id)),
            action: Some(NoticeAction::OpenSession {
                session_id: session.id.clone(),
            }),
            user_id: None, // global session notice
        })
        .await;
}

/// True when there is at least one task and every task is completed.
fn all_done(tasks: &[AgentTask]) -> bool {
    !tasks.is_empty() && tasks.iter().all(|t| t.status == TaskStatus::Completed)
}

/// Summarize what changed between two task snapshots (matched by title) into one
/// trail line: newly-completed wins, then newly-started, then a first-plan line.
/// Returns `None` when nothing notable changed (e.g. only pending tasks added).
fn task_transition_summary(prior: &[AgentTask], now: &[AgentTask]) -> Option<String> {
    let prior_done: HashSet<&str> = prior
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .map(|t| t.title.as_str())
        .collect();
    let prior_active: HashSet<&str> = prior
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress)
        .map(|t| t.title.as_str())
        .collect();

    let newly_done: Vec<&str> = now
        .iter()
        .filter(|t| t.status == TaskStatus::Completed && !prior_done.contains(t.title.as_str()))
        .map(|t| t.title.as_str())
        .collect();
    if !newly_done.is_empty() {
        return Some(if newly_done.len() == 1 {
            format!("✓ Completed: {}", clip(newly_done[0], 120))
        } else {
            format!("✓ Completed {} tasks", newly_done.len())
        });
    }

    let newly_started: Vec<&str> = now
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress && !prior_active.contains(t.title.as_str()))
        .map(|t| t.title.as_str())
        .collect();
    if !newly_started.is_empty() {
        return Some(if newly_started.len() == 1 {
            format!("▶ Started: {}", clip(newly_started[0], 120))
        } else {
            format!("▶ Started {} tasks", newly_started.len())
        });
    }

    // First plan: prior was empty, now has tasks.
    if prior.is_empty() && !now.is_empty() {
        return Some(format!(
            "Planned {} task{}",
            now.len(),
            if now.len() == 1 { "" } else { "s" }
        ));
    }
    None
}

/// A trail entry the normalizer wants written.
struct TrailDraft {
    source: TrailSource,
    kind: TrailKind,
    level: TrailLevel,
    summary: String,
    detail: Option<Value>,
}

/// What a Claude hook payload maps to: an optional trail entry and/or a task
/// list replacement (TodoWrite).
#[derive(Default)]
struct Normalized {
    trail: Option<TrailDraft>,
    tasks: Option<Vec<NewTask>>,
}

/// Truncate `s` to at most `max` chars (char-boundary safe), appending `…`.
fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Last path segment of `p` (for terser file summaries).
fn basename(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// True when a PostToolUse payload signals the tool failed. Conservative — only
/// explicit flags, so a healthy run never shows red.
fn tool_failed(p: &Value) -> bool {
    fn flagged(v: &Value) -> bool {
        v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
            || v.get("success").and_then(|b| b.as_bool()).map(|s| !s).unwrap_or(false)
    }
    flagged(p) || p.get("tool_response").is_some_and(flagged)
}

/// Map a Claude Code hook payload to Otto's model. Knows the quirks of each
/// `hook_event_name` / `tool_name` — the single place that translates a
/// provider's shape into the unified trail/task model.
fn normalize_claude(p: &Value) -> Normalized {
    let event = p.get("hook_event_name").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = Normalized::default();

    match event {
        "UserPromptSubmit" => {
            let prompt = p.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            if !prompt.trim().is_empty() {
                out.trail = Some(TrailDraft {
                    source: TrailSource::User,
                    kind: TrailKind::Prompt,
                    level: TrailLevel::Info,
                    summary: clip(prompt, 240),
                    detail: None,
                });
            }
        }
        "SessionStart" => {
            let src = p.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let msg = match src {
                "resume" => "Session resumed",
                "compact" => "Context compacted",
                "clear" => "Conversation cleared",
                _ => "Session started",
            };
            out.trail = Some(TrailDraft {
                source: TrailSource::Agent,
                kind: TrailKind::Session,
                level: TrailLevel::Info,
                summary: msg.to_string(),
                detail: None,
            });
        }
        "Stop" => {
            out.trail = Some(TrailDraft {
                source: TrailSource::Agent,
                kind: TrailKind::Session,
                level: TrailLevel::Info,
                summary: "Agent finished responding".to_string(),
                detail: None,
            });
        }
        "Notification" => {
            let msg = p.get("message").and_then(|v| v.as_str()).unwrap_or("Notification");
            out.trail = Some(TrailDraft {
                source: TrailSource::Agent,
                kind: TrailKind::Other,
                level: TrailLevel::Warn,
                summary: clip(msg, 200),
                detail: None,
            });
        }
        "PostToolUse" => {
            let tool = p.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
            let input = p.get("tool_input").cloned();
            let failed = tool_failed(p);
            normalize_tool(tool, input.as_ref(), failed, &mut out);
        }
        _ => {}
    }
    out
}

/// Normalize one PostToolUse event by tool name. Read-only/navigation tools
/// (Read/Glob/Grep/LS) are intentionally dropped to keep the trail signal-rich.
/// `failed` raises the entry's level to error.
fn normalize_tool(tool: &str, input: Option<&Value>, failed: bool, out: &mut Normalized) {
    let s = |k: &str| -> String {
        input
            .and_then(|i| i.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let lvl = |base: TrailLevel| if failed { TrailLevel::Error } else { base };
    let mut draft = |source, kind, level, summary, detail| {
        out.trail = Some(TrailDraft { source, kind, level, summary, detail });
    };

    match tool {
        "TodoWrite" => {
            // Tasks only — the ingest path emits the transition trail line.
            let todos = input.and_then(|i| i.get("todos")).and_then(|v| v.as_array());
            if let Some(todos) = todos {
                out.tasks = Some(
                    todos
                        .iter()
                        .map(|t| {
                            let title = t
                                .get("content")
                                .and_then(|v| v.as_str())
                                .or_else(|| t.get("activeForm").and_then(|v| v.as_str()))
                                .unwrap_or("(task)")
                                .to_string();
                            let status = t
                                .get("status")
                                .and_then(|v| v.as_str())
                                .and_then(TaskStatus::parse)
                                .unwrap_or(TaskStatus::Pending);
                            NewTask { ext_id: None, title, status }
                        })
                        .collect(),
                );
            }
        }
        "Bash" => {
            let cmd = s("command");
            if !cmd.trim().is_empty() {
                draft(
                    TrailSource::Agent,
                    TrailKind::Command,
                    lvl(TrailLevel::Info),
                    format!("$ {}", clip(&cmd, 200)),
                    input.cloned(),
                );
            }
        }
        "Skill" => {
            let name = {
                let c = s("command");
                if c.is_empty() { s("skill") } else { c }
            };
            draft(
                TrailSource::Agent,
                TrailKind::Skill,
                lvl(TrailLevel::Info),
                format!("Loaded skill: {}", clip(&name, 80)),
                None,
            );
        }
        "Task" => {
            let desc = {
                let d = s("description");
                if d.is_empty() { s("subagent_type") } else { d }
            };
            draft(
                TrailSource::Agent,
                TrailKind::Tool,
                lvl(TrailLevel::Info),
                format!("Task: {}", clip(&desc, 120)),
                None,
            );
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let path = s("file_path");
            let label = if path.is_empty() {
                tool.to_string()
            } else {
                format!("{tool}: {}", basename(&path))
            };
            draft(
                TrailSource::Agent,
                TrailKind::File,
                lvl(TrailLevel::Info),
                label,
                input.cloned(),
            );
        }
        "WebFetch" | "WebSearch" => {
            let what = {
                let u = s("url");
                if u.is_empty() { s("query") } else { u }
            };
            draft(
                TrailSource::Agent,
                TrailKind::Web,
                lvl(TrailLevel::Info),
                format!("{tool}: {}", clip(&what, 120)),
                None,
            );
        }
        // Skip pure read/navigation tools (Read, Glob, Grep, LS, …); surface MCP
        // tools and anything else as a generic tool entry.
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" | "" => {}
        other => {
            draft(
                TrailSource::Agent,
                TrailKind::Tool,
                lvl(TrailLevel::Info),
                clip(other, 80),
                input.cloned(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(title: &str, status: TaskStatus) -> AgentTask {
        AgentTask {
            id: "t".into(),
            session_id: "s".into(),
            workspace_id: "w".into(),
            ext_id: None,
            title: title.into(),
            status,
            position: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn user_prompt_becomes_user_trail() {
        let p = json!({ "hook_event_name": "UserPromptSubmit", "prompt": "build the thing" });
        let n = normalize_claude(&p);
        let d = n.trail.expect("trail");
        assert_eq!(d.source, TrailSource::User);
        assert_eq!(d.kind, TrailKind::Prompt);
        assert_eq!(d.summary, "build the thing");
        assert!(n.tasks.is_none());
    }

    #[test]
    fn bash_tool_becomes_command_trail() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "cargo build" }
        });
        let d = normalize_claude(&p).trail.expect("trail");
        assert_eq!(d.kind, TrailKind::Command);
        assert_eq!(d.level, TrailLevel::Info);
        assert_eq!(d.summary, "$ cargo build");
        assert!(d.detail.is_some());
    }

    #[test]
    fn failed_tool_is_error_level() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "exit 1" },
            "tool_response": { "is_error": true }
        });
        let d = normalize_claude(&p).trail.expect("trail");
        assert_eq!(d.level, TrailLevel::Error);
    }

    #[test]
    fn skill_tool_becomes_skill_trail() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Skill",
            "tool_input": { "command": "brainstorming" }
        });
        let d = normalize_claude(&p).trail.expect("trail");
        assert_eq!(d.kind, TrailKind::Skill);
        assert_eq!(d.summary, "Loaded skill: brainstorming");
    }

    #[test]
    fn todowrite_syncs_tasks_without_trail() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "TodoWrite",
            "tool_input": { "todos": [
                { "content": "design", "status": "completed", "activeForm": "designing" },
                { "content": "build",  "status": "in_progress", "activeForm": "building" }
            ]}
        });
        let n = normalize_claude(&p);
        let tasks = n.tasks.expect("tasks");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert!(n.trail.is_none(), "transition line comes from the ingest path, not the normalizer");
    }

    #[test]
    fn read_tool_is_filtered_out() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Read",
            "tool_input": { "file_path": "/tmp/x" }
        });
        let n = normalize_claude(&p);
        assert!(n.trail.is_none());
        assert!(n.tasks.is_none());
    }

    #[test]
    fn mcp_tool_falls_through_to_generic() {
        let p = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "mcp__slack__send",
            "tool_input": {}
        });
        let d = normalize_claude(&p).trail.expect("trail");
        assert_eq!(d.kind, TrailKind::Tool);
        assert_eq!(d.summary, "mcp__slack__send");
    }

    #[test]
    fn transition_detects_completion_start_and_plan() {
        let now = vec![task("a", TaskStatus::Pending), task("b", TaskStatus::Pending)];
        assert_eq!(task_transition_summary(&[], &now).as_deref(), Some("Planned 2 tasks"));

        let prior = vec![task("a", TaskStatus::Pending)];
        let now = vec![task("a", TaskStatus::InProgress)];
        assert_eq!(task_transition_summary(&prior, &now).as_deref(), Some("▶ Started: a"));

        let prior = vec![task("a", TaskStatus::InProgress)];
        let now = vec![task("a", TaskStatus::Completed)];
        assert_eq!(task_transition_summary(&prior, &now).as_deref(), Some("✓ Completed: a"));

        let prior = vec![task("a", TaskStatus::Completed)];
        let now = vec![task("a", TaskStatus::Completed), task("b", TaskStatus::Pending)];
        assert_eq!(task_transition_summary(&prior, &now), None);
    }

    #[test]
    fn all_done_requires_nonempty_and_all_completed() {
        assert!(!all_done(&[]));
        assert!(!all_done(&[task("a", TaskStatus::Pending)]));
        assert!(all_done(&[task("a", TaskStatus::Completed)]));
    }

    #[test]
    fn codex_turn_complete_maps_to_session_trail() {
        let d = normalize_codex(&json!({ "type": "agent-turn-complete" })).expect("trail");
        assert_eq!(d.kind, TrailKind::Session);
        assert_eq!(d.summary, "Agent finished responding");
        assert!(normalize_codex(&json!({})).is_none());
    }
}

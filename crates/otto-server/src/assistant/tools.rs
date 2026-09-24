//! `POST /assistant/agent/{tool}` — the back-end of the assistant's MCP tools
//! (`assistant_remember` / `_forget` / `_recall` / `_create_reminder` /
//! `_create_task` / `_update_task` / `_delegate` / `_request_approval`), served
//! natively by the per-session stdio server (`ottod mcp-tools`) and, for the
//! memory tools, by the governed `otto.assistant_*` catalog.
//!
//! Identity: the calling SESSION decides the thread. An Otto-issued
//! per-session token carries an immutable `managed_session_id` that OVERRIDES
//! any `session_id` in the body, so a session can only ever act for its own
//! thread; a plain user token may name one of its OWN sessions. The session
//! must belong to the caller and carry `meta.assistant_thread` (else 403).
//! Without a session (an outward MCP client) only user-level effects happen:
//! memories are saved without a chip, tasks have no thread.

use otto_core::auth::AuthContext;
use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_state::AssistantThread;
use serde_json::{json, Value};

use super::memory::{self, to_wire};
use super::repo;
use super::tasks;
use super::types::CreateTaskReq;
use crate::state::ServerCtx;

/// Every `{tool}` segment, and the MCP tool it backs.
pub const TOOLS: [(&str, &str); 8] = [
    ("remember", "assistant_remember"),
    ("forget", "assistant_forget"),
    ("recall", "assistant_recall"),
    ("reminder", "assistant_create_reminder"),
    ("task", "assistant_create_task"),
    ("task_update", "assistant_update_task"),
    ("delegate", "assistant_delegate"),
    ("approval", "assistant_request_approval"),
];

/// The tools that read or write memory (refused on incognito threads).
const MEMORY_TOOLS: [&str; 3] = ["remember", "forget", "recall"];

fn arg<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn req_arg<'a>(body: &'a Value, key: &str) -> Result<&'a str> {
    arg(body, key).ok_or_else(|| Error::Invalid(format!("`{key}` is required")))
}

/// The session a call speaks for: the token's immutable binding wins over the
/// body (an agent can never name another session).
pub fn calling_session(auth: &AuthContext, body: &Value) -> Option<String> {
    auth.managed_session_id
        .clone()
        .or_else(|| arg(body, "session_id").map(str::to_string))
}

/// Resolve the calling session to the caller's thread (see module docs).
async fn resolve_thread(
    ctx: &ServerCtx,
    user: &User,
    session_id: Option<&str>,
) -> Result<Option<AssistantThread>> {
    let Some(sid) = session_id else {
        return Ok(None);
    };
    let session = ctx
        .manager
        .get(&sid.to_string())
        .await
        .map_err(|_| Error::Invalid("unknown session_id".into()))?;
    if session.created_by != user.id {
        return Err(Error::Forbidden("not your session".into()));
    }
    let thread_id = session
        .meta
        .get("assistant_thread")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Forbidden("not an Otto Assistant session".into()))?;
    Ok(Some(repo(ctx).get_thread(&user.id, thread_id).await?))
}

pub async fn run(ctx: &ServerCtx, auth: &AuthContext, tool: &str, body: Value) -> Result<Value> {
    if !TOOLS.iter().any(|(t, _)| *t == tool) {
        return Err(Error::NotFound(format!("assistant tool '{tool}'")));
    }
    let user = &auth.effective_user;
    let owner = user.id.as_str();
    let sid = calling_session(auth, &body);
    let thread = resolve_thread(ctx, user, sid.as_deref()).await?;
    if MEMORY_TOOLS.contains(&tool) && thread.as_ref().is_some_and(|t| t.incognito) {
        return Err(Error::Invalid(
            "this is an incognito thread: no memory reads or writes".into(),
        ));
    }
    let thread_id = thread.as_ref().map(|t| t.id.clone());
    let tid = thread_id.as_deref();

    match tool {
        "remember" => {
            let text = req_arg(&body, "text")?;
            let tags: Vec<String> = body
                .get("tags")
                .and_then(|t| serde_json::from_value(t.clone()).ok())
                .unwrap_or_default();
            let (m, pending) =
                memory::remember_from_agent(ctx, owner, tid, text, arg(&body, "kind"), tags)
                    .await?;
            Ok(json!({ "memory": to_wire(&m), "pending": pending }))
        }
        "forget" => {
            let query = req_arg(&body, "query")?;
            let (gone, tokens) = memory::forget_matching(ctx, owner, query).await?;
            if let Some(t) = tid {
                memory::forgot_chip(ctx, owner, t, &gone, &tokens).await;
            }
            Ok(json!({
                "forgotten": gone.iter().map(to_wire).collect::<Vec<_>>(),
                "undo_tokens": tokens,
            }))
        }
        "recall" => {
            let k = body
                .get("k")
                .and_then(Value::as_i64)
                .unwrap_or(10)
                .clamp(1, 20);
            let (accepted, _) = memory::list(ctx, owner, arg(&body, "query"), k).await?;
            let profile = memory::read_profile(ctx, owner)
                .await
                .map(|p| p.content)
                .unwrap_or_default();
            Ok(json!({
                "profile": profile,
                "memories": accepted.iter().take(k as usize).map(to_wire).collect::<Vec<_>>(),
            }))
        }
        "reminder" => {
            let task = tasks::create(
                ctx,
                owner,
                CreateTaskReq {
                    kind: "reminder".into(),
                    title: req_arg(&body, "text")?.to_string(),
                    detail: arg(&body, "detail").map(str::to_string),
                    thread_id: thread_id.clone(),
                    run_at: Some(req_arg(&body, "run_at")?.to_string()),
                    timezone: arg(&body, "timezone").map(str::to_string),
                    origin: Some("thread".into()),
                },
                true,
            )
            .await?;
            Ok(json!(task))
        }
        "task" => {
            let task = tasks::create(
                ctx,
                owner,
                CreateTaskReq {
                    kind: "task".into(),
                    title: req_arg(&body, "title")?.to_string(),
                    detail: arg(&body, "detail").map(str::to_string),
                    thread_id: thread_id.clone(),
                    run_at: None,
                    timezone: None,
                    origin: Some("thread".into()),
                },
                true,
            )
            .await?;
            Ok(json!(task))
        }
        "task_update" => {
            let task = tasks::agent_update(
                ctx,
                owner,
                req_arg(&body, "task_id")?,
                req_arg(&body, "state")?,
                body.get("result").cloned().filter(|v| !v.is_null()),
                arg(&body, "question"),
                body.get("options").cloned(),
            )
            .await?;
            Ok(json!(task))
        }
        "delegate" => {
            let task = tasks::delegate(
                ctx,
                user,
                tid,
                req_arg(&body, "agent")?,
                req_arg(&body, "directive")?,
            )
            .await?;
            Ok(json!(task))
        }
        "approval" => tasks::request_approval(ctx, owner, tid, &body).await,
        _ => Err(Error::NotFound(format!("assistant tool '{tool}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(managed: Option<&str>) -> AuthContext {
        let user: User = serde_json::from_value(json!({
            "id": "u1", "username": "u", "display_name": "U", "is_root": false,
            "disabled": false, "created_at": "2026-09-24T00:00:00Z"
        }))
        .expect("user fixture");
        AuthContext {
            real_user: user.clone(),
            effective_user: user,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: managed.map(str::to_string),
        }
    }

    #[test]
    fn the_token_binding_overrides_the_body_session() {
        let body = json!({"session_id": "other-session"});
        assert_eq!(
            calling_session(&auth(Some("mine")), &body).as_deref(),
            Some("mine")
        );
        assert_eq!(
            calling_session(&auth(None), &body).as_deref(),
            Some("other-session")
        );
        assert_eq!(
            calling_session(&auth(None), &json!({"session_id": " "})),
            None
        );
        assert_eq!(calling_session(&auth(None), &json!({})), None);
    }

    #[test]
    fn every_tool_segment_backs_one_mcp_tool() {
        let names: std::collections::HashSet<&str> = TOOLS.iter().map(|(_, m)| *m).collect();
        assert_eq!(names.len(), TOOLS.len());
        for (seg, mcp) in TOOLS {
            assert!(mcp.starts_with("assistant_"), "{mcp}");
            assert!(!seg.contains('/'), "{seg}");
        }
        for m in MEMORY_TOOLS {
            assert!(TOOLS.iter().any(|(t, _)| *t == m));
        }
    }
}

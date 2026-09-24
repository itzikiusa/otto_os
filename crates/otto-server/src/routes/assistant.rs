//! Otto Assistant REST endpoints (`docs/contracts/api.md` "Otto Assistant").
//!
//! RBAC: `feature_guard` maps every `/assistant/*` path to `Feature::Agents`
//! (View for GET, Edit for writes) via `policy.rs`. There is no workspace
//! axis — every row is the CALLER's own (`owner_user_id`), and the storage
//! layer answers 404 for anyone else's id (root included: the assistant is
//! personal). Delegation additionally checks `scheduled_tasks:Edit` + Editor
//! on the Personal Agent's workspace (in `assistant::tasks::resolve_agent`).

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use serde_json::{json, Value};

use otto_core::{new_id, Error};
use otto_state::{
    AssistantAttachment, AssistantTask, AssistantThread, AssistantTurn, NewAssistantThread,
};

use crate::assistant::limits::LimitState;
use crate::assistant::router::{self as arouter, RouteDecision, RoutingSettings};
use crate::assistant::types::*;
use crate::assistant::{
    self, hermes, memory, owned_thread, repo, system_turn, tasks, threads, tools,
};
use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Decoded attachment cap.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/assistant/threads", get(list_threads).post(create_thread))
        .route(
            "/assistant/threads/{id}",
            get(get_thread).patch(update_thread).delete(delete_thread),
        )
        .route(
            "/assistant/threads/{id}/turns",
            get(list_turns).post(send_turn),
        )
        .route(
            "/assistant/threads/{id}/attachments",
            post(add_attachment).layer(DefaultBodyLimit::max(30 * 1024 * 1024)),
        )
        .route("/assistant/threads/{id}/route", post(set_route))
        .route("/assistant/threads/{id}/delegate", post(delegate))
        .route("/assistant/route/preview", post(preview))
        .route("/assistant/needs-you", get(needs_you))
        .route("/assistant/tasks", get(list_tasks).post(create_task))
        .route("/assistant/tasks/{id}", get(get_task))
        .route("/assistant/tasks/{id}/{action}", post(task_action))
        .route(
            "/assistant/memory",
            get(get_memory).put(put_memory).post(add_memory),
        )
        .route("/assistant/memory/undo", post(undo_memory))
        .route(
            "/assistant/memory/import/hermes",
            get(hermes_preview).post(hermes_import),
        )
        .route("/assistant/memory/{id}", delete(delete_memory))
        .route("/assistant/memory/{id}/accept", post(accept_memory))
        .route("/assistant/forget", post(forget))
        .route("/assistant/routing", get(get_routing).put(put_routing))
        .route("/assistant/limits", get(get_limits))
        .route("/assistant/agent/{tool}", post(agent_tool))
}

// --- Threads ---------------------------------------------------------------------

async fn list_threads(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AssistantThread>>> {
    let mut out = Vec::new();
    for t in repo(&ctx).list_threads(&user.id).await? {
        out.push(threads::with_status(&ctx, t).await);
    }
    Ok(Json(out))
}

async fn create_thread(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateThreadReq>,
) -> ApiResult<Json<AssistantThread>> {
    let pinned = req
        .provider
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty());
    let (settings, _) = threads::load_settings(&ctx, &user.id).await;
    let target = if pinned {
        let p = req.provider.clone().unwrap_or_default();
        if !arouter::valid_provider(p.trim()) {
            return Err(ApiError(Error::Invalid(format!(
                "provider '{p}' is not a valid provider name"
            ))));
        }
        arouter::RouteTarget {
            provider: p.trim().to_string(),
            model: req.model.clone().filter(|m| !m.trim().is_empty()),
            account_id: req.account_id.clone().filter(|a| !a.trim().is_empty()),
        }
    } else {
        settings.targets.chat.clone()
    };
    let title = req.title.clone().unwrap_or_default();
    if title.len() > 200 {
        return Err(ApiError(Error::Invalid("title is too long".into())));
    }
    let t = repo(&ctx)
        .create_thread(NewAssistantThread {
            owner_user_id: user.id.clone(),
            title: title.trim().to_string(),
            space_slot: req.space_slot,
            provider: target.provider,
            model: target.model,
            account_id: target.account_id,
            route_pinned: pinned,
            incognito: req.incognito.unwrap_or(false),
        })
        .await?;
    Ok(Json(threads::with_status(&ctx, t).await))
}

async fn get_thread(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<AssistantThread>> {
    Ok(Json(owned_thread(&ctx, &user.id, &id).await?))
}

async fn update_thread(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateThreadReq>,
) -> ApiResult<Json<AssistantThread>> {
    if req.title.as_deref().is_some_and(|t| t.len() > 200) {
        return Err(ApiError(Error::Invalid("title is too long".into())));
    }
    let t = repo(&ctx)
        .update_thread(
            &user.id,
            &id,
            req.title.map(|t| t.trim().to_string()),
            req.space_slot,
        )
        .await?;
    Ok(Json(threads::with_status(&ctx, t).await))
}

async fn delete_thread(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let t = repo(&ctx).delete_thread(&user.id, &id).await?;
    // The Otto session row goes; the provider's own transcript on disk stays.
    if let Some(sid) = t.session_id.as_ref() {
        let _ = ctx.manager.remove(sid).await;
    }
    Ok(Json(json!({"ok": true})))
}

async fn list_turns(
    Path(id): Path<String>,
    Query(q): Query<TurnsQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AssistantTurn>>> {
    repo(&ctx).get_thread(&user.id, &id).await?;
    Ok(Json(
        repo(&ctx)
            .list_turns(&id, q.before.as_deref(), q.limit.unwrap_or(100))
            .await?,
    ))
}

async fn send_turn(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SendReq>,
) -> ApiResult<Json<SendResp>> {
    Ok(Json(threads::send(&ctx, &user.id, &id, req, None).await?))
}

async fn add_attachment(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AttachmentReq>,
) -> ApiResult<Json<AssistantAttachment>> {
    repo(&ctx).get_thread(&user.id, &id).await?;
    if req.content_base64.len() > (MAX_ATTACHMENT_BYTES / 3 + 1) * 4 + 16 {
        return Err(ApiError(Error::PayloadTooLarge(
            "attachment exceeds 20 MiB".into(),
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.content_base64.trim())
        .map_err(|_| ApiError(Error::Invalid("content_base64 is not valid base64".into())))?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError(Error::PayloadTooLarge(
            "attachment exceeds 20 MiB".into(),
        )));
    }
    let name = safe_name(&req.name)
        .ok_or_else(|| ApiError(Error::Invalid("name must be a plain file name".into())))?;
    let att_id = new_id();
    let inbox = assistant::assistant_dir(&ctx, &user.id).join("inbox");
    tokio::fs::create_dir_all(&inbox)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("inbox: {e}"))))?;
    // The id prefix keeps two uploads of `report.pdf` apart.
    let path = inbox.join(format!("{att_id}-{name}"));
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("inbox: {e}"))))?;
    let a = AssistantAttachment {
        id: att_id,
        name,
        path: path.to_string_lossy().to_string(),
        mime: req
            .mime
            .filter(|m| !m.trim().is_empty() && m.len() <= 200)
            .unwrap_or_else(|| "application/octet-stream".into()),
        size: bytes.len() as i64,
    };
    repo(&ctx).add_attachment(&user.id, &id, &a).await?;
    Ok(Json(a))
}

/// One path segment: no separators, no `..`, no leading dot, no NUL/control
/// characters; capped at 150 bytes (on a char boundary) so the stored
/// `<id>-<name>` stays under the 255-byte file-name limit.
pub fn safe_name(raw: &str) -> Option<String> {
    let n = raw.trim();
    let base = n.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty()
        || base.starts_with('.')
        || base.chars().any(|c| c.is_control())
        || otto_core::paths::safe_component(base).is_none()
    {
        return None;
    }
    let mut end = base.len().min(150);
    while !base.is_char_boundary(end) {
        end -= 1;
    }
    Some(base[..end].to_string())
}

async fn set_route(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RouteReq>,
) -> ApiResult<Json<AssistantThread>> {
    let before = repo(&ctx).get_thread(&user.id, &id).await?;
    match req
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(p) => {
            if !arouter::valid_provider(p) {
                return Err(ApiError(Error::Invalid(format!("provider '{p}'"))));
            }
            let model = req.model.as_deref().filter(|m| !m.trim().is_empty());
            let account = req.account_id.as_deref().filter(|a| !a.trim().is_empty());
            repo(&ctx)
                .set_thread_pin(&id, true, Some(p), model, account)
                .await?;
            let text = format!(
                "Pinned to {}{}. It takes over from the next message.",
                threads::display_provider(p),
                model.map(|m| format!(" · {m}")).unwrap_or_default()
            );
            system_turn(
                &ctx,
                &user.id,
                &id,
                "route",
                &text,
                Some(json!({"pinned": true, "provider": p, "model": model})),
            )
            .await;
        }
        None => {
            repo(&ctx)
                .set_thread_pin(&id, false, None, None, None)
                .await?;
            if before.route_pinned {
                system_turn(
                    &ctx,
                    &user.id,
                    &id,
                    "route",
                    "Unpinned — Otto picks the model per message again.",
                    Some(json!({"pinned": false})),
                )
                .await;
            }
        }
    }
    Ok(Json(owned_thread(&ctx, &user.id, &id).await?))
}

async fn delegate(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DelegateReq>,
) -> ApiResult<Json<AssistantTask>> {
    Ok(Json(
        tasks::delegate(
            &ctx,
            &user,
            Some(id.as_str()),
            &req.agent_id,
            &req.directive,
        )
        .await?,
    ))
}

async fn preview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PreviewReq>,
) -> ApiResult<Json<RouteDecision>> {
    assistant::check_text("text", &req.text, threads::MAX_TURN_BYTES)?;
    Ok(Json(
        threads::preview(
            &ctx,
            &user.id,
            &req.text,
            req.thread_id.as_deref(),
            req.voice,
        )
        .await?,
    ))
}

// --- Needs-you + tasks -------------------------------------------------------------

async fn needs_you(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AssistantTask>>> {
    Ok(Json(repo(&ctx).needs_you(&user.id).await?))
}

async fn list_tasks(
    Query(q): Query<TasksQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AssistantTask>>> {
    if let Some(s) = q.state.as_deref() {
        if !otto_state::assistant::TASK_STATES.contains(&s) {
            return Err(ApiError(Error::Invalid(format!("unknown state '{s}'"))));
        }
    }
    Ok(Json(
        repo(&ctx)
            .list_tasks(
                &user.id,
                q.state.as_deref(),
                q.thread_id.as_deref(),
                q.limit.unwrap_or(100),
            )
            .await?,
    ))
}

async fn create_task(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateTaskReq>,
) -> ApiResult<Json<AssistantTask>> {
    Ok(Json(tasks::create(&ctx, &user.id, req, false).await?))
}

async fn get_task(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<AssistantTask>> {
    Ok(Json(repo(&ctx).get_task(&user.id, &id).await?))
}

async fn task_action(
    Path((id, action)): Path<(String, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<DecisionReq>>,
) -> ApiResult<Json<AssistantTask>> {
    if !tasks::ACTIONS.contains(&action.as_str()) {
        return Err(ApiError(Error::NotFound(format!("action '{action}'"))));
    }
    let req = body.map(|Json(b)| b).unwrap_or_default();
    Ok(Json(tasks::act(&ctx, &user, &id, &action, req).await?))
}

// --- Memory ------------------------------------------------------------------------

async fn get_memory(
    Query(q): Query<MemoryQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<MemoryView>> {
    let (accepted, pending) =
        memory::list(&ctx, &user.id, q.q.as_deref(), q.limit.unwrap_or(200)).await?;
    let (settings, _) = threads::load_settings(&ctx, &user.id).await;
    Ok(Json(MemoryView {
        profile: memory::read_profile(&ctx, &user.id).await?,
        memories: accepted.iter().map(memory::to_wire).collect(),
        pending: pending.iter().map(memory::to_wire).collect(),
        memory_approval: settings.memory_approval,
    }))
}

async fn put_memory(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PutMemoryReq>,
) -> ApiResult<Json<ProfileDoc>> {
    Ok(Json(
        memory::save_profile(&ctx, &user.id, &req.profile.version, &req.profile.content).await?,
    ))
}

async fn add_memory(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AddMemoryReq>,
) -> ApiResult<Json<AssistantMemory>> {
    let m = memory::save(
        &ctx,
        &user.id,
        &req.text,
        req.kind.as_deref(),
        req.tags,
        "user",
        None,
        false,
    )
    .await?;
    Ok(Json(memory::to_wire(&m)))
}

async fn delete_memory(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let token = memory::forget_one(&ctx, &user.id, &id).await?;
    memory::settle_review(&ctx, &user.id, &id, "rejected").await;
    Ok(Json(json!({"ok": true, "undo_token": token})))
}

async fn accept_memory(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<AssistantMemory>> {
    Ok(Json(memory::to_wire(
        &memory::accept(&ctx, &user.id, &id).await?,
    )))
}

async fn undo_memory(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UndoReq>,
) -> ApiResult<Json<AssistantMemory>> {
    Ok(Json(memory::to_wire(
        &memory::undo(&ctx, &user.id, req.undo_token.trim()).await?,
    )))
}

async fn forget(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ForgetReq>,
) -> ApiResult<Json<ForgetResp>> {
    if let Some(tid) = req.thread_id.as_deref() {
        let t = repo(&ctx).get_thread(&user.id, tid).await?;
        if t.incognito {
            return Err(ApiError(Error::Invalid(
                "this is an incognito thread: no memory reads or writes".into(),
            )));
        }
    }
    let (gone, tokens) = memory::forget_matching(&ctx, &user.id, &req.query).await?;
    if let Some(tid) = req.thread_id.as_deref() {
        memory::forgot_chip(&ctx, &user.id, tid, &gone, &tokens).await;
    }
    Ok(Json(ForgetResp {
        forgotten: gone.iter().map(memory::to_wire).collect(),
        undo_tokens: tokens,
    }))
}

/// READ-ONLY scan of `~/.hermes/memories` (nothing is written anywhere).
async fn hermes_preview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<HermesPreview>> {
    let Some(dir) = hermes::default_dir() else {
        return Ok(Json(HermesPreview {
            available: false,
            files: vec![],
            entries: vec![],
        }));
    };
    let available = dir.is_dir();
    let files = tokio::task::spawn_blocking(move || hermes::scan_dir(&dir))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("hermes scan: {e}"))))?;
    let mut entries = Vec::new();
    for f in &files {
        for e in &f.entries {
            entries.push(HermesEntry {
                file: f.name.clone(),
                text: e.clone(),
                duplicate: memory::is_duplicate(&ctx, &user.id, e).await,
            });
        }
    }
    Ok(Json(HermesPreview {
        available,
        files: files
            .iter()
            .map(|f| HermesFileSummary {
                name: f.name.clone(),
                entries: f.entries.len(),
            })
            .collect(),
        entries,
    }))
}

/// One-time, user-triggered import: every entry not already remembered lands
/// as a PENDING memory for review. Reads `~/.hermes`, never writes it.
async fn hermes_import(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<HermesImportResp>> {
    let Some(dir) = hermes::default_dir() else {
        return Ok(Json(HermesImportResp {
            queued: 0,
            duplicates: 0,
            files: vec![],
        }));
    };
    let files = tokio::task::spawn_blocking(move || hermes::scan_dir(&dir))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("hermes scan: {e}"))))?;
    let (mut queued, mut duplicates) = (0usize, 0usize);
    for f in &files {
        for e in &f.entries {
            if memory::is_duplicate(&ctx, &user.id, e).await {
                duplicates += 1;
                continue;
            }
            match memory::save(
                &ctx,
                &user.id,
                e,
                Some("fact"),
                hermes::tags_for(&f.name),
                "hermes",
                Some(f.name.clone()),
                true,
            )
            .await
            {
                Ok(_) => queued += 1,
                Err(err) => tracing::warn!("assistant: hermes import entry skipped: {err}"),
            }
        }
    }
    Ok(Json(HermesImportResp {
        queued,
        duplicates,
        files: files.iter().map(|f| f.name.clone()).collect(),
    }))
}

// --- Routing + limits --------------------------------------------------------------

async fn get_routing(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<RoutingSettings>> {
    Ok(Json(threads::load_settings(&ctx, &user.id).await.0))
}

async fn put_routing(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(patch): Json<Value>,
) -> ApiResult<Json<RoutingSettings>> {
    let (current, _) = threads::load_settings(&ctx, &user.id).await;
    let merged =
        arouter::merge_settings(&current, &patch).map_err(|e| ApiError(Error::Invalid(e)))?;
    threads::save_settings(&ctx, &user.id, &merged).await?;
    Ok(Json(threads::load_settings(&ctx, &user.id).await.0))
}

async fn get_limits(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<LimitState>>> {
    Ok(Json(threads::load_settings(&ctx, &user.id).await.1))
}

// --- Agent tools -------------------------------------------------------------------

async fn agent_tool(
    Path(tool): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Ok(Json(tools::run(&ctx, &auth, &tool, body).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_names_are_one_plain_segment() {
        assert_eq!(safe_name("report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(
            safe_name("/tmp/x/../report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            safe_name("C:\\Users\\me\\scan.png").as_deref(),
            Some("scan.png")
        );
        assert!(safe_name("..").is_none());
        assert!(safe_name(".env").is_none());
        assert!(safe_name("").is_none());
        assert!(safe_name("a\u{0}b").is_none());
        assert_eq!(safe_name(&"n".repeat(300)).unwrap().len(), 150);
        // Multi-byte names are cut on a char boundary.
        assert!(safe_name(&"é".repeat(100)).unwrap().len() <= 150);
    }
}

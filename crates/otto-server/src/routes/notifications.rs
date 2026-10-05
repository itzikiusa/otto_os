//! Notification center endpoints: list, mark-read, dismiss, clear, settings.
//!
//! Notices are daemon-wide (not workspace-scoped) but per-user scoped: a notice
//! is either global (`user_id IS NULL`, e.g. credential/system notices) or owned
//! by one user. A non-root user sees global notices plus their own and may only
//! mark-read / dismiss / clear their OWN — global / shared notices are read-only
//! to them so one user can't alter another's (or the system's) state. Root sees
//! and manages everything (`NoticeAccess::All`). Mutations that change nothing
//! still return 204. Settings remain a single daemon-wide row.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::NotificationSettings;
use otto_core::domain::{Notice, User};
use otto_core::Id;
use otto_state::{NoticeAccess, NotificationsRepo};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Cap on how many notices `GET /notifications` returns (newest first).
const LIST_LIMIT: i64 = 200;

/// Map an authenticated user to the notice access scope they should operate
/// under: the root operator manages every notice; everyone else is confined to
/// global notices (read-only) plus their own.
fn access_for(user: &User) -> NoticeAccess {
    if user.is_root {
        NoticeAccess::All
    } else {
        NoticeAccess::User(user.id.clone())
    }
}

/// Tell the caller's other windows (the menu-bar tray's glyph) that their
/// notice list changed — reads/dismissals have no `notification` event.
fn changed(ctx: &ServerCtx, user: &User) {
    let _ = ctx
        .events
        .send(otto_core::event::Event::NotificationsChanged {
            user_id: user.id.clone(),
        });
}

/// `GET /api/v1/notifications`
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Notice>>> {
    let repo = NotificationsRepo::new(ctx.pool.clone());
    Ok(Json(repo.list(LIST_LIMIT, &access_for(&user)).await?))
}

/// `POST /api/v1/notifications/{id}/read`
pub async fn mark_read(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .mark_read(&id, &access_for(&user))
        .await?;
    changed(&ctx, &user);
    Ok(StatusCode::NO_CONTENT)
}

/// Most ids one bulk call may carry (the UI sends ≤ 25; the cap keeps the
/// statement well under SQLite's bound-parameter limit).
pub const BULK_MAX: usize = 500;

/// Body of the bulk read / dismiss calls.
#[derive(Debug, serde::Deserialize)]
pub struct BulkIds {
    pub ids: Vec<Id>,
}

/// Reply of the bulk read / dismiss calls: rows actually changed (foreign,
/// global-for-non-root, unknown and already-read ids are skipped).
#[derive(Debug, serde::Serialize)]
pub struct BulkResult {
    pub changed: u64,
}

fn check_bulk(ids: &[Id]) -> ApiResult<()> {
    if ids.len() > BULK_MAX {
        return Err(crate::error::ApiError(otto_core::Error::Invalid(format!(
            "at most {BULK_MAX} ids per call"
        ))));
    }
    Ok(())
}

/// `POST /api/v1/notifications/read` `{ids}` — mark a batch read in one
/// statement with ONE `notifications_changed` broadcast (perf §15 N4).
pub async fn mark_many_read(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<BulkIds>,
) -> ApiResult<Json<BulkResult>> {
    check_bulk(&body.ids)?;
    let changed_n = NotificationsRepo::new(ctx.pool.clone())
        .mark_many_read(&body.ids, &access_for(&user))
        .await?;
    if changed_n > 0 {
        changed(&ctx, &user);
    }
    Ok(Json(BulkResult { changed: changed_n }))
}

/// `POST /api/v1/notifications/dismiss` `{ids}` — dismiss a batch in one
/// statement with ONE `notifications_changed` broadcast.
pub async fn dismiss_many(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<BulkIds>,
) -> ApiResult<Json<BulkResult>> {
    check_bulk(&body.ids)?;
    let changed_n = NotificationsRepo::new(ctx.pool.clone())
        .dismiss_many(&body.ids, &access_for(&user))
        .await?;
    if changed_n > 0 {
        changed(&ctx, &user);
    }
    Ok(Json(BulkResult { changed: changed_n }))
}

/// `POST /api/v1/notifications/read-all`
pub async fn mark_all_read(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .mark_all_read(&access_for(&user))
        .await?;
    changed(&ctx, &user);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/notifications/{id}`
pub async fn dismiss(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .dismiss(&id, &access_for(&user))
        .await?;
    changed(&ctx, &user);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/notifications` — clear the caller's notices.
pub async fn clear(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .clear(&access_for(&user))
        .await?;
    changed(&ctx, &user);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/notifications/settings`
pub async fn get_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<NotificationSettings>> {
    let repo = NotificationsRepo::new(ctx.pool.clone());
    Ok(Json(repo.get_settings().await?))
}

/// `PUT /api/v1/notifications/settings` — replace and return settings.
///
/// Root only (S8-05): the settings are ONE daemon-wide row (native toasts,
/// session events, credential-expiry threshold), so letting any member — a
/// Viewer included — write it let them silence root's notices.
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<NotificationSettings>,
) -> ApiResult<Json<NotificationSettings>> {
    crate::auth::require_root(&user)?;
    let repo = NotificationsRepo::new(ctx.pool.clone());
    Ok(Json(repo.put_settings(&body).await?))
}

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
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/notifications/read-all`
pub async fn mark_all_read(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .mark_all_read(&access_for(&user))
        .await?;
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
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    Json(body): Json<NotificationSettings>,
) -> ApiResult<Json<NotificationSettings>> {
    let repo = NotificationsRepo::new(ctx.pool.clone());
    Ok(Json(repo.put_settings(&body).await?))
}

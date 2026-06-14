//! Notification center endpoints: list, mark-read, dismiss, clear, settings.
//!
//! Notices are daemon-wide (not workspace-scoped) and mirror the `/ws/events`
//! `Event::Notification` stream, so any authenticated user may read and manage
//! them. Mutations that change nothing still return 204.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::NotificationSettings;
use otto_core::domain::Notice;
use otto_core::Id;
use otto_state::NotificationsRepo;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Cap on how many notices `GET /notifications` returns (newest first).
const LIST_LIMIT: i64 = 200;

/// `GET /api/v1/notifications`
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Vec<Notice>>> {
    let repo = NotificationsRepo::new(ctx.pool.clone());
    Ok(Json(repo.list(LIST_LIMIT).await?))
}

/// `POST /api/v1/notifications/{id}/read`
pub async fn mark_read(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .mark_read(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/notifications/read-all`
pub async fn mark_all_read(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone())
        .mark_all_read()
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/notifications/{id}`
pub async fn dismiss(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone()).dismiss(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/notifications` — clear all.
pub async fn clear(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<StatusCode> {
    NotificationsRepo::new(ctx.pool.clone()).clear().await?;
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

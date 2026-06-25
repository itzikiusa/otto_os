//! Session **name theme** endpoints.
//!
//! New agent sessions are auto-named from the creating user's active theme
//! (e.g. "Ronaldo", "Messi") instead of "claude #3". Built-in themes are
//! compiled into the daemon (`otto_sessions::names`); users can also define
//! custom name lists (family names, …). Every endpoint is per-user and needs
//! only an authenticated caller (no root).
//!
//! - `GET    /api/v1/name-themes`         — built-ins + the caller's custom themes + active id
//! - `PUT    /api/v1/name-themes/active`  — set the caller's active theme
//! - `POST   /api/v1/name-themes`         — create a custom theme
//! - `PUT    /api/v1/name-themes/{id}`    — replace a custom theme
//! - `DELETE /api/v1/name-themes/{id}`    — delete a custom theme

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{
    CreateNameThemeReq, CustomThemeResp, NameThemeInfo, NameThemesResp, SetActiveThemeReq,
    UpdateNameThemeReq,
};
use otto_core::{Error, Id};
use otto_sessions::names;
use otto_state::NameThemesRepo;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Build the combined theme list (built-ins first, then the caller's custom
/// themes) plus the caller's active selection.
async fn themes_response(repo: &NameThemesRepo, user_id: &Id) -> ApiResult<NameThemesResp> {
    let mut themes: Vec<NameThemeInfo> = names::builtin_theme_infos()
        .into_iter()
        .map(|t| NameThemeInfo {
            id: t.id,
            label: t.label,
            kind: "builtin".into(),
            capacity: t.capacity,
            sample: t.sample,
        })
        .collect();
    for c in repo.list_for_owner(user_id).await? {
        let sample: Vec<String> = c
            .names
            .iter()
            .filter(|n| !n.trim().is_empty())
            .take(6)
            .cloned()
            .collect();
        let capacity = c.names.iter().filter(|n| !n.trim().is_empty()).count();
        themes.push(NameThemeInfo {
            id: c.id,
            label: c.label,
            kind: "custom".into(),
            capacity,
            sample,
        });
    }
    let active = repo
        .active(user_id)
        .await?
        .unwrap_or_else(|| names::DEFAULT_THEME.to_string());
    Ok(NameThemesResp { themes, active })
}

/// `GET /api/v1/name-themes`
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<NameThemesResp>> {
    let repo = NameThemesRepo::new(ctx.pool.clone());
    Ok(Json(themes_response(&repo, &user.id).await?))
}

/// `PUT /api/v1/name-themes/active` — validate then persist the active theme.
pub async fn set_active(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SetActiveThemeReq>,
) -> ApiResult<Json<NameThemesResp>> {
    let repo = NameThemesRepo::new(ctx.pool.clone());
    let id = req.theme_id.trim();
    // Accept the "none" sentinel, any built-in id, or one of the caller's own
    // custom themes. Reject anything else so a stale id can't silently stick.
    let valid = id == names::THEME_NONE
        || names::is_builtin(id)
        || repo
            .list_for_owner(&user.id)
            .await?
            .iter()
            .any(|c| c.id == id);
    if !valid {
        return Err(ApiError(Error::Invalid(format!("unknown theme '{id}'"))));
    }
    repo.set_active(&user.id, id).await?;
    Ok(Json(themes_response(&repo, &user.id).await?))
}

/// `POST /api/v1/name-themes` — create a custom theme.
pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateNameThemeReq>,
) -> ApiResult<Json<CustomThemeResp>> {
    let label = req.label.trim();
    if label.is_empty() {
        return Err(ApiError(Error::Invalid("theme label is empty".into())));
    }
    let repo = NameThemesRepo::new(ctx.pool.clone());
    let t = repo.create(&user.id, label, &req.names).await?;
    Ok(Json(CustomThemeResp {
        id: t.id,
        label: t.label,
        names: t.names,
    }))
}

/// `PUT /api/v1/name-themes/{id}` — replace a custom theme (owner-scoped).
pub async fn update(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Json(req): Json<UpdateNameThemeReq>,
) -> ApiResult<Json<CustomThemeResp>> {
    let label = req.label.trim();
    if label.is_empty() {
        return Err(ApiError(Error::Invalid("theme label is empty".into())));
    }
    let repo = NameThemesRepo::new(ctx.pool.clone());
    let t = repo.update(&id, &user.id, label, &req.names).await?;
    Ok(Json(CustomThemeResp {
        id: t.id,
        label: t.label,
        names: t.names,
    }))
}

/// `DELETE /api/v1/name-themes/{id}` — delete a custom theme (owner-scoped). If
/// it was the caller's active theme, reset them to the "none" sentinel so new
/// sessions don't silently jump to the default built-in.
pub async fn delete(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let repo = NameThemesRepo::new(ctx.pool.clone());
    repo.delete(&id, &user.id).await?;
    if repo.active(&user.id).await?.as_deref() == Some(id.as_str()) {
        repo.set_active(&user.id, names::THEME_NONE).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

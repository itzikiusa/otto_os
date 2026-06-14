//! Endpoints #57-58: daemon settings (root only). Shape: flat JSON object
//! `{ "<key>": <value_json>, ... }`.

use axum::extract::State;
use axum::Json;
use otto_state::SettingsRepo;
use serde_json::{Map, Value};

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/settings`
pub async fn get_all(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;
    Ok(Json(SettingsRepo::new(ctx.pool.clone()).all().await?))
}

/// `PUT /api/v1/settings` — upserts every key in the body, returns the full
/// settings object.
pub async fn put_all(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<Map<String, Value>>,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;
    let repo = SettingsRepo::new(ctx.pool.clone());
    for (key, value) in &body {
        repo.put(key, value).await?;
    }
    // Custom providers apply immediately — no daemon restart needed.
    if let Some(value) = body.get("providers") {
        ctx.manager.providers().reload(Some(value));
    }
    Ok(Json(repo.all().await?))
}

//! Endpoints #57-58: daemon settings (root only). Shape: flat JSON object
//! `{ "<key>": <value_json>, ... }`.

use axum::extract::State;
use axum::Json;
use otto_state::{NewAuditEntry, SettingsRepo};
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

    // Audit the change. The list of changed keys is the durable record; secret
    // values are deliberately NOT captured. The network listener is a setting,
    // so its toggle flows through here — give it a dedicated, easily-filtered
    // entry (with the new enabled/port) on top of the generic settings change.
    if let Some(listener) = body.get("network_listener") {
        let enabled = listener
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ctx.audit(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "network_listener.toggle".into(),
            target: Some(if enabled { "on" } else { "off" }.into()),
            detail: Some(listener.clone()),
            ip: None,
        })
        .await;
    }
    let mut keys: Vec<&String> = body.keys().collect();
    keys.sort();
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "settings.change".into(),
        target: Some(keys.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(",")),
        detail: Some(serde_json::json!({ "keys": keys })),
        ip: None,
    })
    .await;

    Ok(Json(repo.all().await?))
}

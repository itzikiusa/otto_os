//! Trust & Safety Center: the security audit log + a derived posture snapshot
//! (both root only). The audit log is written best-effort by `ServerCtx::audit`
//! at sensitive sites; this module only reads it. The posture summary derives
//! from the live listener, saved settings and the auth store — no new tables.

use axum::extract::{Query, State};
use axum::Json;
use otto_core::api::{AuditLogQuery, AuditLogResp, SecurityPostureResp};
use otto_rbac::AuthRepo;
use otto_state::{AuditRepo, SettingsRepo};
use serde_json::Value;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/audit-log?from=&to=&action=&user_id=&limit=&offset=` (root only)
/// — a filtered, paged page of audit entries (newest first) plus the total
/// matching the filters.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<AuditLogQuery>,
) -> ApiResult<Json<AuditLogResp>> {
    require_root(&user)?;
    let repo = AuditRepo::new(ctx.pool.clone());
    let entries = repo.list(&q).await?;
    let total = repo.count(&q).await?;
    Ok(Json(AuditLogResp { entries, total }))
}

/// `GET /api/v1/security-posture` (root only) — a snapshot the Trust & Safety
/// Center renders: network-listener state + bind port, loopback-only status,
/// and the count of active API tokens. Saved settings identify pending restarts.
pub async fn posture(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SecurityPostureResp>> {
    require_root(&user)?;

    // The network listener is stored as `{ "enabled": bool, "port": u16 }` (see
    // ottod's boot). These are desired settings, not proof of a live socket.
    let listener = SettingsRepo::new(ctx.pool.clone())
        .get("network_listener")
        .await?;
    let configured_enabled = listener
        .as_ref()
        .and_then(|v| v.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false);
    let configured_port = listener
        .as_ref()
        .and_then(|v| v.get("port").and_then(Value::as_u64))
        .and_then(|p| u16::try_from(p).ok());
    let network_listener_port = crate::transport::network_listener_port();
    let network_listener = network_listener_port.is_some();
    let desired_port = configured_port.or_else(|| ctx.base_url.rsplit(':').next()?.parse().ok());
    let network_listener_restart_required = configured_enabled != network_listener
        || (configured_enabled && desired_port != network_listener_port);

    let active_api_tokens = AuthRepo::new(ctx.pool.clone())
        .count_active_api_tokens()
        .await?;

    Ok(Json(SecurityPostureResp {
        network_listener,
        network_listener_port,
        loopback_only: !network_listener,
        network_listener_restart_required,
        active_api_tokens,
    }))
}

//! MCP Control Plane extras owned by the control-plane UI: per-token rotation
//! and the per-workspace session-attach switch. Kept out of `mcp_outward.rs` on
//! purpose.

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::{Capability, Feature, User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::{GrantsRepo, NewAuditEntry, SettingsRepo, WorkspacesRepo};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

async fn require_mcp_admin(ctx: &ServerCtx, user: &User) -> ApiResult<()> {
    if user.is_root {
        return Ok(());
    }
    let cap = GrantsRepo::new(ctx.pool.clone())
        .capability_of(user, Feature::Mcp)
        .await?;
    if cap >= Capability::Admin {
        Ok(())
    } else {
        Err(Error::Forbidden("requires mcp:admin".into()).into())
    }
}

/// `POST /api/v1/mcp/tokens/{id}/rotate` — replace one MCP token, preserving
/// its owner, label, and scope while leaving every other token untouched.
pub async fn rotate_mcp_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_mcp_admin(&ctx, &user).await?;
    let Some((token, info)) = AuthRepo::with_cache(ctx.pool.clone(), ctx.auth_cache.clone())
        .rotate_mcp_token(&id)
        .await
        .map_err(ApiError)?
    else {
        return Err(Error::NotFound("mcp token not found".into()).into());
    };
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.token.rotate".into(),
        target: Some(id.clone()),
        detail: Some(json!({ "new_id": info.id })),
        ip: None,
    })
    .await;
    Ok(Json(
        json!({ "token": token, "info": info, "revoked_id": id }),
    ))
}

/// `GET /api/v1/workspaces/{wid}/mcp/session-attach` — report whether newly
/// spawned sessions in this workspace receive Otto's built-in MCP server.
pub async fn get_session_attach(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(wid): Path<Id>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let value = SettingsRepo::new(ctx.pool.clone())
        .get(otto_state::OTTO_MCP_ENABLED_KEY)
        .await?;
    Ok(Json(json!({
        "workspace_id": wid,
        "attached": otto_state::otto_mcp_enabled_for(value.as_ref(), &wid),
    })))
}

#[derive(Deserialize)]
pub struct SessionAttachReq {
    pub enabled: bool,
}

/// `PATCH /api/v1/workspaces/{wid}/mcp/session-attach` — persist a workspace
/// override in map form without changing any other workspace's effective value.
pub async fn set_session_attach(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(wid): Path<Id>,
    Json(req): Json<SessionAttachReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    require_mcp_admin(&ctx, &user).await?;

    let settings = SettingsRepo::new(ctx.pool.clone());
    let current = settings.get(otto_state::OTTO_MCP_ENABLED_KEY).await?;
    let all_ids: Vec<String> = WorkspacesRepo::new(ctx.pool.clone())
        .list_all()
        .await?
        .into_iter()
        .map(|workspace| workspace.id)
        .collect();
    let next = next_otto_mcp_enabled(current.as_ref(), &wid, req.enabled, &all_ids);
    settings
        .put(otto_state::OTTO_MCP_ENABLED_KEY, &next)
        .await?;
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.session_attach.toggle".into(),
        target: Some(wid.clone()),
        detail: Some(json!({ "enabled": req.enabled })),
        ip: None,
    })
    .await;
    Ok(Json(json!({
        "workspace_id": wid,
        "attached": otto_state::otto_mcp_enabled_for(Some(&next), &wid),
    })))
}

/// Convert the session-attach setting to its per-workspace map form while
/// preserving every known workspace's effective value.
pub fn next_otto_mcp_enabled(
    current: Option<&Value>,
    wid: &str,
    enabled: bool,
    all_ws_ids: &[String],
) -> Value {
    let mut next = match current {
        Some(Value::Bool(false)) => all_ws_ids
            .iter()
            .map(|id| (id.clone(), Value::Bool(false)))
            .collect::<Map<String, Value>>(),
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    next.insert(wid.to_owned(), Value::Bool(enabled));
    Value::Object(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_setting_from_absent_is_a_single_override() {
        assert_eq!(
            next_otto_mcp_enabled(None, "a", false, &["a".into(), "b".into()]),
            json!({ "a": false })
        );
    }

    #[test]
    fn next_setting_from_true_is_a_single_override() {
        assert_eq!(
            next_otto_mcp_enabled(Some(&json!(true)), "a", false, &["a".into(), "b".into()],),
            json!({ "a": false })
        );
    }

    #[test]
    fn next_setting_from_false_preserves_known_workspaces_as_false() {
        assert_eq!(
            next_otto_mcp_enabled(Some(&json!(false)), "a", true, &["a".into(), "b".into()],),
            json!({ "a": true, "b": false })
        );
    }

    #[test]
    fn next_setting_from_object_preserves_other_overrides() {
        assert_eq!(
            next_otto_mcp_enabled(
                Some(&json!({ "a": false, "b": false })),
                "a",
                true,
                &["a".into(), "b".into()],
            ),
            json!({ "a": true, "b": false })
        );
    }
}

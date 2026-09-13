//! Explicit, freshly authorized connection export. Prepared credentials exist
//! only in the response; no export files or credentials are written to disk/logs.
use crate::{
    auth::{require_root, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue},
    routing::{get, post},
    Json, Router,
};
use otto_connections::conn_export::{self, ExportFormat, ExportResult};
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/state/connections/export/formats", get(export_formats))
        .route("/state/connections/export", post(export_connections))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub format: String,
    pub scope: String,
    #[serde(default)]
    pub workspace_ids: Vec<Id>,
    #[serde(default)]
    pub include_passwords: bool,
}
#[derive(Serialize)]
pub struct FormatsResponse {
    formats: Vec<ExportFormat>,
}
async fn fresh_root(ctx: &ServerCtx, id: &Id) -> ApiResult<()> {
    let actor = otto_state::UsersRepo::new(ctx.pool.clone()).get(id).await?;
    if actor.disabled {
        return Err(ApiError(Error::Forbidden(
            "connection export requires an active root user".into(),
        )));
    }
    require_root(&actor)
}
fn invalid(message: &str) -> ApiError {
    Error::Invalid(message.into()).into()
}

pub async fn export_formats(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FormatsResponse>> {
    fresh_root(&ctx, &user.id).await?;
    Ok(Json(FormatsResponse {
        formats: conn_export::formats(),
    }))
}

fn validate_scope(req: &ExportRequest) -> ApiResult<()> {
    match req.scope.as_str() {
        "all" if req.workspace_ids.is_empty() => Ok(()),
        "workspaces" if !req.workspace_ids.is_empty() && req.workspace_ids.len() <= 1000 => Ok(()),
        _ => Err(invalid(
            "Choose scope all without workspace_ids, or workspaces with 1–1000 workspace_ids",
        )),
    }
}
fn in_scope(req: &ExportRequest, workspace: Option<&str>) -> bool {
    req.scope == "all"
        || workspace.is_none()
        || workspace.is_some_and(|id| req.workspace_ids.iter().any(|w| w == id))
}

pub async fn export_connections(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ExportRequest>,
) -> ApiResult<(HeaderMap, Json<ExportResult>)> {
    fresh_root(&ctx, &user.id).await?;
    validate_scope(&req)?;
    let format = conn_export::formats()
        .into_iter()
        .find(|f| f.id == req.format)
        .ok_or_else(|| invalid("Unknown connection export format"))?;
    for wid in &req.workspace_ids {
        otto_state::WorkspacesRepo::new(ctx.pool.clone())
            .get(wid)
            .await?;
    }
    let rows: Vec<(Id, Option<Id>)> =
        sqlx::query_as("SELECT id,workspace_id FROM connections ORDER BY name,id LIMIT 10001")
            .fetch_all(&ctx.pool)
            .await
            .map_err(|_| {
                ApiError(Error::Internal(
                    "Could not enumerate connection profiles".into(),
                ))
            })?;
    if rows.len() > 10000 {
        return Err(invalid("Connection export exceeds 10,000 profiles"));
    }
    let mut profiles = Vec::new();
    let mut bytes = 0;
    for (id, workspace) in rows {
        if !in_scope(&req, workspace.as_deref()) {
            continue;
        }
        let mut profile = ctx.connections.export_profile(&id, &user.id, false).await?;
        let kind = profile.record["kind"].as_str().unwrap_or("");
        if req.include_passwords && (format.kinds.contains(&"*") || format.kinds.contains(&kind)) {
            profile = ctx.connections.export_profile(&id, &user.id, true).await?;
        }
        if !in_scope(&req, profile.record["workspace_id"].as_str()) {
            continue;
        }
        bytes +=
            profile.record.to_string().len() + profile.password.as_ref().map_or(0, String::len);
        if bytes > 32 * 1024 * 1024 {
            return Err(invalid(
                "Connection export exceeds 32 MiB; select fewer workspaces",
            ));
        }
        profiles.push(profile);
    }
    let result = conn_export::render(&req.format, profiles, req.include_passwords)?;
    fresh_root(&ctx, &user.id).await?;
    // Only counts and explicit options enter audit; never profile parameters,
    // generated content, credential references, or passwords.
    super::backup::record_action(&ctx,&user.id,"connections.export",None,Some(serde_json::json!({
        "format":req.format,"scope":req.scope,"include_passwords":req.include_passwords,
        "exported_connections":result.exported_connections,"skipped_connections":result.skipped.len()
    }))).await;
    fresh_root(&ctx, &user.id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    Ok((headers, Json(result)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_workspaces_include_globals_but_never_other_workspaces() {
        let req = ExportRequest {
            format: "json".into(),
            scope: "workspaces".into(),
            workspace_ids: vec!["a".into()],
            include_passwords: false,
        };
        assert!(validate_scope(&req).is_ok());
        assert!(in_scope(&req, None));
        assert!(in_scope(&req, Some("a")));
        assert!(!in_scope(&req, Some("b")));
        assert!(validate_scope(&ExportRequest {
            scope: "all".into(),
            ..req
        })
        .is_err());
    }
    #[test]
    fn password_export_is_explicit_and_unknown_options_rejected() {
        let req: ExportRequest =
            serde_json::from_value(serde_json::json!({"format":"json","scope":"all"})).unwrap();
        assert!(!req.include_passwords);
        assert!(validate_scope(&req).is_ok());
        assert!(serde_json::from_value::<ExportRequest>(
            serde_json::json!({"format":"json","scope":"all","passwords":true})
        )
        .is_err());
    }
}

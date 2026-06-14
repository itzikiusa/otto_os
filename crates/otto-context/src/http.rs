//! Axum router for the Otto library + per-workspace context endpoints.
//! All paths are relative to the `/api/v1` mount point.
//!
//! The library is instance-global: reads are allowed to any authenticated
//! member, writes are root-only. Per-workspace context follows the standard
//! workspace-role gate (Viewer read / Admin write / Editor materialize).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{
    GlobalSoulReq, GlobalSoulResp, LibraryContext, LibrarySkill, LibrarySoul,
    MaterializeProviderResult, MaterializeResp, Problem, UpdateWorkspaceContextReq,
    UpsertLibraryEntryReq, WorkspaceContextConfig,
};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;
use serde::Deserialize;

use crate::library::Library;
use crate::{config, materialize};

/// Dependencies the context router needs from the host application state.
pub trait ContextCtx: Clone + Send + Sync + 'static {
    fn library(&self) -> &Library;
    fn workspaces(&self) -> &WorkspacesRepo;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// Library writes are root-only.
fn require_root(user: &AuthUser) -> Result<(), ApiErr> {
    if user.0.is_root {
        Ok(())
    } else {
        Err(Error::Forbidden("library writes require root".into()).into())
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the context router. Paths are relative to the `/api/v1` mount point.
pub fn router<C: ContextCtx>() -> Router<C> {
    Router::new()
        // Library: skills
        .route("/library/skills", get(list_skills::<C>))
        .route(
            "/library/skills/{name}",
            get(get_skill::<C>).put(put_skill::<C>).delete(delete_skill::<C>),
        )
        // Library: souls
        .route("/library/souls", get(list_souls::<C>))
        .route(
            "/library/souls/{name}",
            get(get_soul::<C>).put(put_soul::<C>).delete(delete_soul::<C>),
        )
        // Library: context
        .route("/library/context", get(list_context::<C>))
        .route(
            "/library/context/{name}",
            get(get_context::<C>).put(put_context::<C>).delete(delete_context::<C>),
        )
        // Library: global default soul
        .route(
            "/library/default-soul",
            get(get_default_soul::<C>).put(set_default_soul::<C>),
        )
        // Per-workspace context config
        .route(
            "/workspaces/{id}/context",
            get(get_ws_context::<C>).put(update_ws_context::<C>),
        )
        .route(
            "/workspaces/{id}/context/materialize",
            post(materialize_ws::<C>),
        )
}

// ---------------------------------------------------------------------------
// Library: skills
// ---------------------------------------------------------------------------

async fn list_skills<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<LibrarySkill>>> {
    Ok(Json(s.library().list_skills()))
}

async fn get_skill<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<Json<LibrarySkill>> {
    s.library()
        .get_skill(&name)
        .map(Json)
        .ok_or_else(|| Error::NotFound(format!("skill '{name}'")).into())
}

async fn put_skill<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
    Json(req): Json<UpsertLibraryEntryReq>,
) -> ApiResult<Json<LibrarySkill>> {
    require_root(&user)?;
    s.library()
        .put_skill(&name, &req.body)
        .map_err(|e| Error::Invalid(format!("put skill: {e}")))?;
    s.library()
        .get_skill(&name)
        .map(Json)
        .ok_or_else(|| Error::Internal("skill not found after put".into()).into())
}

async fn delete_skill<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    s.library()
        .delete_skill(&name)
        .map_err(|e| Error::Invalid(format!("delete skill: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Library: souls
// ---------------------------------------------------------------------------

async fn list_souls<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<LibrarySoul>>> {
    Ok(Json(s.library().list_souls()))
}

async fn get_soul<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<Json<LibrarySoul>> {
    s.library()
        .get_soul(&name)
        .map(Json)
        .ok_or_else(|| Error::NotFound(format!("soul '{name}'")).into())
}

async fn put_soul<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
    Json(req): Json<UpsertLibraryEntryReq>,
) -> ApiResult<Json<LibrarySoul>> {
    require_root(&user)?;
    s.library()
        .put_soul(&name, &req.body)
        .map_err(|e| Error::Invalid(format!("put soul: {e}")))?;
    s.library()
        .get_soul(&name)
        .map(Json)
        .ok_or_else(|| Error::Internal("soul not found after put".into()).into())
}

async fn delete_soul<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    s.library()
        .delete_soul(&name)
        .map_err(|e| Error::Invalid(format!("delete soul: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Library: context
// ---------------------------------------------------------------------------

async fn list_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<LibraryContext>>> {
    Ok(Json(s.library().list_context()))
}

async fn get_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<Json<LibraryContext>> {
    s.library()
        .get_context(&name)
        .map(Json)
        .ok_or_else(|| Error::NotFound(format!("context '{name}'")).into())
}

async fn put_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
    Json(req): Json<UpsertLibraryEntryReq>,
) -> ApiResult<Json<LibraryContext>> {
    require_root(&user)?;
    s.library()
        .put_context(&name, &req.body)
        .map_err(|e| Error::Invalid(format!("put context: {e}")))?;
    s.library()
        .get_context(&name)
        .map(Json)
        .ok_or_else(|| Error::Internal("context not found after put".into()).into())
}

async fn delete_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    s.library()
        .delete_context(&name)
        .map_err(|e| Error::Invalid(format!("delete context: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Library: global default soul
// ---------------------------------------------------------------------------

async fn get_default_soul<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<GlobalSoulResp>> {
    Ok(Json(GlobalSoulResp {
        name: s.library().default_soul(),
    }))
}

async fn set_default_soul<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<GlobalSoulReq>,
) -> ApiResult<Json<GlobalSoulResp>> {
    require_root(&user)?;
    s.library()
        .set_default_soul(&req.name)
        .map_err(|e| Error::Invalid(format!("set default soul: {e}")))?;
    Ok(Json(GlobalSoulResp {
        name: s.library().default_soul(),
    }))
}

// ---------------------------------------------------------------------------
// Per-workspace context config
// ---------------------------------------------------------------------------

async fn get_ws_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<WorkspaceContextConfig>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Viewer).await?;
    let ws = s.workspaces().get(&ws_id).await?;
    Ok(Json(config::from_settings(&ws.settings)))
}

async fn update_ws_context<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpdateWorkspaceContextReq>,
) -> ApiResult<Json<WorkspaceContextConfig>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Admin).await?;
    let ws = s.workspaces().get(&ws_id).await?;
    let cfg = WorkspaceContextConfig {
        skills: req.skills,
        soul: req.soul,
        extra_context_md: req.extra_context_md,
        include_memory: req.include_memory,
    };
    let merged = config::write_into_settings(&ws.settings, &cfg);
    let updated = s
        .workspaces()
        .update(&ws_id, None, None, Some(&merged), None)
        .await?;
    Ok(Json(config::from_settings(&updated.settings)))
}

#[derive(Deserialize)]
struct MaterializeQuery {
    provider: Option<String>,
}

async fn materialize_ws<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(q): Query<MaterializeQuery>,
) -> ApiResult<Json<MaterializeResp>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Editor).await?;
    let ws = s.workspaces().get(&ws_id).await?;
    let cfg = config::from_settings(&ws.settings);

    let providers: Vec<String> = match q.provider {
        Some(p) => vec![p],
        None => vec!["claude".to_string(), "codex".to_string()],
    };

    let provider_results: Vec<MaterializeProviderResult> = providers
        .iter()
        .map(|p| materialize::provision(s.library(), &cfg, &ws.root_path, p))
        .collect();

    Ok(Json(MaterializeResp { provider_results }))
}

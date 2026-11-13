//! Review-comment routes (merged into `crate::http::router`; `/design/*`
//! RBAC — GET = design View, writes = design Edit — comes from the server's
//! policy table, the workspace role from the artifact's row):
//!
//!   GET    /design/artifacts/{id}/comments[?version=&resolved=&limit=]  (ws viewer)
//!   POST   /design/artifacts/{id}/comments                  (ws editor) → 201
//!   PATCH  /design/artifacts/{id}/comments/{comment_id}     (ws editor; body = author only)
//!   DELETE /design/artifacts/{id}/comments/{comment_id}     (ws editor + author, or ws admin)
//!
//! A comment never changes the design, so a mirrored (read-only) or archived
//! artifact takes comments too. Every write emits the usual
//! `design_artifact_updated {change: "comments"}` so other tabs refresh.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Extension, Json, Router};
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::Deserialize;

use super::{clean_body, clean_pin, CommentFilter, CommentPin, DesignComment, NewComment};
use crate::http::DesignCtx;
use crate::service::DesignService;
use crate::types::DesignArtifact;

struct CommentErr(Error);

impl From<Error> for CommentErr {
    fn from(e: Error) -> Self {
        CommentErr(e)
    }
}

impl IntoResponse for CommentErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(crate::mirror::problem(&self.0))).into_response()
    }
}

type CommentResult<T> = std::result::Result<T, CommentErr>;

#[derive(Deserialize)]
struct IdPath {
    id: Id,
}

#[derive(Deserialize)]
struct CommentPath {
    id: Id,
    comment_id: Id,
}

#[derive(Deserialize, Default)]
struct ListQuery {
    /// Only threads made on this version id.
    #[serde(default)]
    version: Option<Id>,
    /// `true` = resolved threads only, `false` = open ones only.
    #[serde(default)]
    resolved: Option<bool>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct CreateReq {
    body: String,
    /// The version the comment is about (default: the head).
    #[serde(default)]
    version_id: Option<Id>,
    /// A reply to this top-level comment (inherits its version; no pin).
    #[serde(default)]
    parent_id: Option<Id>,
    #[serde(default)]
    pin: Option<CommentPin>,
}

#[derive(Deserialize)]
struct UpdateReq {
    /// New text (the author only).
    #[serde(default)]
    body: Option<String>,
    /// Resolve (`true`) / reopen (`false`) a thread (any editor).
    #[serde(default)]
    resolved: Option<bool>,
}

/// The comment routes, relative to the `/api/v1` mount point.
pub fn routes<S: DesignCtx>() -> Router<S> {
    Router::new()
        .route(
            "/design/artifacts/{id}/comments",
            get(list::<S>).post(create::<S>),
        )
        .route(
            "/design/artifacts/{id}/comments/{comment_id}",
            patch(update::<S>).delete(remove::<S>),
        )
}

async fn load_artifact<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> CommentResult<DesignArtifact> {
    let a = svc.store().require_artifact(id).await?;
    ctx.roles().check(user, &a.workspace_id, role).await?;
    Ok(a)
}

/// The comment, which must belong to `a` (another artifact's id is a 404).
async fn load_comment(
    svc: &DesignService,
    a: &DesignArtifact,
    comment_id: &str,
) -> CommentResult<DesignComment> {
    let c = svc.store().require_comment(comment_id).await?;
    if c.artifact_id != a.id {
        return Err(CommentErr(Error::NotFound(format!(
            "design comment {comment_id}"
        ))));
    }
    Ok(c)
}

fn display_name(user: &User) -> String {
    if user.display_name.trim().is_empty() {
        user.username.clone()
    } else {
        user.display_name.clone()
    }
}

fn is_author(c: &DesignComment, user: &User) -> bool {
    c.author_kind == "user" && c.author_id == user.id
}

/// Tell other tabs (and the artifact view's Comments tab) to refetch.
fn notify(svc: &DesignService, a: &DesignArtifact) {
    svc.emit(Event::DesignArtifactUpdated {
        workspace_id: a.workspace_id.clone(),
        artifact_id: a.id.clone(),
        format: a.format.clone(),
        change: "comments".into(),
        version_id: None,
        content: None,
    });
}

async fn list<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<ListQuery>,
) -> CommentResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Viewer).await?;
    let f = CommentFilter {
        version_id: q.version.filter(|v| !v.trim().is_empty()),
        resolved: q.resolved,
        limit: q.limit,
    };
    Ok(Json(svc.store().list_comments(&a.id, &f).await?).into_response())
}

async fn create<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<CreateReq>,
) -> CommentResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let CreateReq {
        body,
        version_id,
        parent_id,
        pin,
    } = req;
    let body = clean_body(&body)?;
    let (version_id, version_seq, pin) = match &parent_id {
        Some(pid) => {
            let parent = load_comment(&svc, &a, pid).await?;
            if parent.parent_id.is_some() {
                return Err(CommentErr(Error::Invalid(
                    "replies are one level deep — reply to the thread instead".into(),
                )));
            }
            if pin.is_some() {
                return Err(CommentErr(Error::Invalid(
                    "a reply has no pin; the thread's pin marks the spot".into(),
                )));
            }
            (parent.version_id, parent.version_seq, None)
        }
        None => {
            let pin = clean_pin(pin)?;
            let wanted = version_id
                .filter(|v| !v.trim().is_empty())
                .or_else(|| a.head_version_id.clone());
            let v = match wanted {
                Some(vid) => Some(
                    svc.store()
                        .get_version(&vid)
                        .await?
                        .filter(|v| v.artifact_id == a.id)
                        .ok_or_else(|| {
                            Error::NotFound(format!("version {vid} of design artifact {}", a.id))
                        })?,
                ),
                None => None,
            };
            (
                v.as_ref().map(|v| v.id.clone()),
                v.as_ref().map(|v| v.seq),
                pin,
            )
        }
    };
    let c = svc
        .store()
        .insert_comment(NewComment {
            artifact_id: a.id.clone(),
            workspace_id: a.workspace_id.clone(),
            version_id,
            version_seq,
            parent_id,
            author_kind: "user".into(),
            author_id: user.id.clone(),
            author_name: display_name(&user),
            session_id: None,
            body,
            pin,
        })
        .await?;
    notify(&svc, &a);
    Ok((StatusCode::CREATED, Json(c)).into_response())
}

async fn update<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(CommentPath { id, comment_id }): Path<CommentPath>,
    Json(req): Json<UpdateReq>,
) -> CommentResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let c = load_comment(&svc, &a, &comment_id).await?;
    if req.body.is_none() && req.resolved.is_none() {
        return Err(CommentErr(Error::Invalid(
            "send `body` and/or `resolved`".into(),
        )));
    }
    // Every check before any write: a refused half never lands.
    let body = match &req.body {
        Some(_) if !is_author(&c, &user) => {
            return Err(CommentErr(Error::Forbidden(
                "only its author can edit a comment".into(),
            )));
        }
        Some(b) => Some(clean_body(b)?),
        None => None,
    };
    if req.resolved.is_some() && c.parent_id.is_some() {
        return Err(CommentErr(Error::Invalid(
            "resolve the thread, not a reply".into(),
        )));
    }
    let mut out = c.clone();
    if let Some(b) = body {
        out = svc.store().update_comment_body(&c.id, &b).await?;
    }
    if let Some(r) = req.resolved.filter(|r| *r != c.resolved) {
        let name = display_name(&user);
        out = svc
            .store()
            .set_comment_resolved(&c.id, r.then_some((user.id.as_str(), name.as_str())))
            .await?;
    }
    notify(&svc, &a);
    Ok(Json(out).into_response())
}

async fn remove<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(CommentPath { id, comment_id }): Path<CommentPath>,
) -> CommentResult<Response> {
    let svc = ctx.design();
    let a = load_artifact(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let c = load_comment(&svc, &a, &comment_id).await?;
    if !is_author(&c, &user)
        && ctx
            .roles()
            .check(&user, &a.workspace_id, WorkspaceRole::Admin)
            .await
            .is_err()
    {
        return Err(CommentErr(Error::Forbidden(
            "only its author or a workspace admin can delete a comment".into(),
        )));
    }
    svc.store().delete_comment(&c.id).await?;
    notify(&svc, &a);
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests;

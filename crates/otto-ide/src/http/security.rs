use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_security::{ManageScan, ProjectScans, ScanDetail, ScanSettings, ScanSummary};
use serde_json::{json, Value};

use super::ApiResult;
use crate::types::IdeTargetSpec;
use crate::IdeCtx;

pub(super) async fn project_roots<S: IdeCtx>(
    ctx: &S,
    user: &User,
    id: &str,
) -> Result<Vec<PathBuf>> {
    let project = ctx
        .ide()
        .projects
        .list_for_user_with_ctx(ctx, user)
        .await?
        .into_iter()
        .find(|project| project.id == id)
        .ok_or_else(|| Error::NotFound("IDE project".into()))?;
    let data_dir = tokio::fs::canonicalize(ctx.data_dir())
        .await
        .unwrap_or_else(|_| ctx.data_dir().to_path_buf());
    let mut roots = Vec::new();
    for root in std::iter::once(project.primary_root).chain(project.folders) {
        let spec = IdeTargetSpec::local(root.clone());
        ctx.authorize_local_target(user, &spec, false).await?;
        let root = tokio::fs::canonicalize(&root)
            .await
            .map_err(|_| Error::Invalid("A project folder is unavailable.".into()))?;
        if !otto_core::auth::root_authority(user) {
            crate::target::enforce_non_root_path(&root)?;
        }
        if root.parent().is_none() || !tokio::fs::metadata(&root).await.is_ok_and(|m| m.is_dir()) {
            return Err(Error::Invalid("Choose a local project directory.".into()));
        }
        if data_dir.starts_with(&root) || root.starts_with(&data_dir) {
            return Err(Error::Invalid(
                "Select a project folder outside Otto's data directory.".into(),
            ));
        }
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
}

pub(super) async fn project<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<ProjectScans>> {
    project_roots(&ctx, &user, &id).await?;
    Ok(Json(ctx.ide().security.project(&ctx.pool(), &id).await?))
}

pub(super) async fn configure<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(settings): Json<ScanSettings>,
) -> ApiResult<Json<ScanSettings>> {
    project_roots(&ctx, &user, &id).await?;
    Ok(Json(
        ctx.ide()
            .security
            .configure(&ctx.pool(), &id, settings)
            .await?,
    ))
}

pub(super) async fn start<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<ScanSummary>)> {
    let scan = enqueue(&ctx, user, id).await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}

async fn enqueue<S: IdeCtx>(ctx: &S, user: User, id: String) -> Result<ScanSummary> {
    project_roots(ctx, &user, &id).await?;
    let service = ctx.ide().security.clone();
    let pool = ctx.pool();
    let job_project = id.clone();
    let host = ctx.clone();
    let load_host = ctx.clone();
    service
        .enqueue(
            pool,
            id,
            move || async move { project_roots(&host, &user, &job_project).await },
            move || {
                let ctx = load_host.clone();
                async move { ctx.security_scan_busy().await }
            },
        )
        .await
}

/// Called by the existing scheduled-task supervisor. The scheduled user is
/// re-authorized at each occurrence and again when the job leaves the queue.
pub async fn scheduled_tick<S: IdeCtx>(ctx: &S) -> Result<()> {
    otto_security::schedule::tick(&ctx.pool(), chrono::Utc::now(), |user, project| {
        super::security_dast::enqueue_scheduled(ctx, user, project)
    })
    .await
}

pub(super) async fn detail<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, scan_id)): Path<(String, String)>,
) -> ApiResult<Json<ScanDetail>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let mut scan = ctx
        .ide()
        .security
        .detail(&ctx.pool(), &id, &scan_id)
        .await?;
    scan.findings.retain(|item| {
        roots
            .iter()
            .any(|root| root == std::path::Path::new(&item.root))
    });
    Ok(Json(scan))
}

pub(super) async fn cancel<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, scan_id)): Path<(String, String)>,
) -> ApiResult<Json<ScanSummary>> {
    project_roots(&ctx, &user, &id).await?;
    Ok(Json(
        ctx.ide()
            .security
            .cancel(&ctx.pool(), &id, &scan_id)
            .await?,
    ))
}

pub(super) async fn manage<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(command): Json<ManageScan>,
) -> ApiResult<Json<Value>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let pool = ctx.pool();
    let reply = match command {
        ManageScan::PreviewSchedule { schedule } => json!(otto_security::schedule::preview(
            schedule,
            chrono::Utc::now()
        )?),
        ManageScan::SetSchedule { schedule } => {
            json!(otto_security::schedule::save(&pool, &id, &user.id, schedule).await?)
        }
        ManageScan::SetSuppression {
            fingerprint,
            reason,
        } => {
            otto_security::preferences::set_suppression(
                &pool,
                &id,
                &fingerprint,
                reason.as_deref(),
            )
            .await?;
            Value::Null
        }
        ManageScan::SetBaseline { scan_id } => {
            otto_security::preferences::set_baseline(&pool, &id, scan_id.as_deref()).await?;
            Value::Null
        }
        ManageScan::ImportSarif { sarif } => {
            let scan_id = otto_security::sarif::import(&pool, &id, &roots, sarif).await?;
            json!({"scan_id": scan_id})
        }
    };
    Ok(Json(reply))
}

pub(super) async fn export<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, scan_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let scan = ctx
        .ide()
        .security
        .detail(&ctx.pool(), &id, &scan_id)
        .await?;
    Ok(Json(otto_security::sarif::export(&scan, &roots)))
}

use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Result};
use otto_security::passive::{
    self, Candidate, ConfirmTarget, ProjectPassive, RunDastRequest, Target,
};
use otto_security::ScanSummary;

use super::{security::project_roots, ApiResult};
use crate::types::IdeTargetSpec;
use crate::IdeCtx;

struct ProjectCandidate {
    root: PathBuf,
    raw_url: String,
    candidate: Candidate,
}

async fn candidates<S: IdeCtx>(ctx: &S, roots: &[PathBuf]) -> Result<Vec<ProjectCandidate>> {
    let mut candidates = Vec::new();
    for root in roots {
        let spec = IdeTargetSpec::local(root.to_string_lossy());
        for config in ctx
            .ide()
            .environments
            .list_run_configs(&ctx.pool(), &spec)
            .await?
        {
            if let Some(raw) = config.env_overrides.get(passive::CONFIG_KEY) {
                if let Ok((url, public)) = passive::parse_url(raw) {
                    candidates.push(ProjectCandidate {
                        root: root.clone(),
                        raw_url: raw.clone(),
                        candidate: Candidate {
                            run_config_id: config.id,
                            run_config_name: config.name,
                            url: url.to_string(),
                            public,
                        },
                    });
                }
            }
        }
    }
    Ok(candidates)
}

async fn eligible<S: IdeCtx>(
    ctx: &S,
    user: &User,
    project: &str,
    roots: &[PathBuf],
    scheduled: bool,
) -> Result<Option<(Target, PathBuf)>> {
    let Some(target) = passive::get(&ctx.pool(), project).await? else {
        return Ok(None);
    };
    if scheduled && !target.scheduled {
        return Ok(None);
    }
    let current = otto_state::UsersRepo::new(ctx.pool()).get(&user.id).await?;
    if current.disabled
        || otto_state::GrantsRepo::new(ctx.pool())
            .capability_of(&current, Feature::Agents)
            .await?
            < Capability::Edit
    {
        return Ok(None);
    }
    let match_run = candidates(ctx, roots).await?.into_iter().find(|candidate| {
        candidate.candidate.run_config_id == target.run_config_id
            && candidate.candidate.url == target.url
            && candidate.candidate.public == target.allow_public
    });
    let Some(matched) = match_run else {
        return Ok(None);
    };
    let spec = IdeTargetSpec::local(matched.root.to_string_lossy());
    ctx.authorize_local_target(&current, &spec, false).await?;
    if !ctx.ide().environments.has_live_security_run(
        &current,
        &spec,
        &target.run_config_id,
        &matched.raw_url,
    ) {
        return Ok(None);
    }
    Ok(Some((target, matched.root)))
}

pub(super) async fn config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<ProjectPassive>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let confirmed = passive::get(&ctx.pool(), &id).await?;
    let candidates = candidates(&ctx, &roots)
        .await?
        .into_iter()
        .map(|entry| entry.candidate)
        .collect();
    Ok(Json(ProjectPassive {
        confirmed,
        candidates,
    }))
}

pub(super) async fn confirm<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ConfirmTarget>,
) -> ApiResult<Json<Target>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let candidate = candidates(&ctx, &roots)
        .await?
        .into_iter()
        .find(|entry| entry.candidate.run_config_id == request.run_config_id)
        .ok_or_else(|| Error::NotFound("project run configuration with an audit URL".into()))?;
    Ok(Json(
        passive::confirm(&ctx.pool(), &id, candidate.candidate, request).await?,
    ))
}

pub(super) async fn revoke<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    project_roots(&ctx, &user, &id).await?;
    passive::revoke(&ctx.pool(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn start<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<ScanSummary>)> {
    let request: RunDastRequest = if body.is_empty() {
        RunDastRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| Error::Invalid("Invalid DAST run options.".into()))?
    };
    let active = request.active;
    let roots = project_roots(&ctx, &user, &id).await?;
    let (expected, _) = eligible(&ctx, &user, &id, &roots, false)
        .await?
        .ok_or_else(|| {
            Error::Conflict(
                "Confirm this URL and start its saved project run configuration.".into(),
            )
        })?;
    let service = ctx.ide().security.clone();
    let runner = service.clone();
    let job_ctx = ctx.clone();
    let project = id.clone();
    let scan = service
        .enqueue_job(
            ctx.pool(),
            id,
            move || {
                let ctx = ctx.clone();
                async move { ctx.security_scan_busy().await }
            },
            move |settings, cancel, _| async move {
                let roots = project_roots(&job_ctx, &user, &project).await?;
                let (target, root) = eligible(&job_ctx, &user, &project, &roots, false)
                    .await?
                    .filter(|(target, _)| target == &expected)
                    .ok_or_else(|| {
                        Error::Conflict(
                            "The confirmed target or its running configuration changed.".into(),
                        )
                    })?;
                let (run, findings) = runner
                    .run_dast(&target, &root, &settings, active, &cancel)
                    .await?;
                Ok((vec![run], findings))
            },
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}

pub(super) async fn enqueue_scheduled<S: IdeCtx>(
    ctx: &S,
    user: User,
    id: String,
) -> Result<ScanSummary> {
    let roots = project_roots(ctx, &user, &id).await?;
    let expected = eligible(ctx, &user, &id, &roots, true).await?;
    let service = ctx.ide().security.clone();
    let worker = service.clone();
    let load_ctx = ctx.clone();
    let job_ctx = ctx.clone();
    let project = id.clone();
    service
        .enqueue_job(
            ctx.pool(),
            id,
            move || {
                let ctx = load_ctx.clone();
                async move { ctx.security_scan_busy().await }
            },
            move |settings, cancel, busy| async move {
                let roots = project_roots(&job_ctx, &user, &project).await?;
                let (mut runs, mut findings) = worker
                    .scan_sources(&roots, &settings, &cancel, &busy)
                    .await?;
                if let Some((expected, _)) = expected {
                    otto_security::wait_for_idle(&busy, &cancel).await?;
                    if let Some((target, root)) = eligible(&job_ctx, &user, &project, &roots, true)
                        .await?
                        .filter(|(target, _)| {
                            target.confirmed_at == expected.confirmed_at
                                && target.url == expected.url
                                && target.scheduled_active == expected.scheduled_active
                        })
                    {
                        match worker
                            .run_dast(&target, &root, &settings, false, &cancel)
                            .await
                        {
                            Ok((run, mut extra)) => {
                                runs.push(run);
                                findings.append(&mut extra);
                            }
                            Err(error) => {
                                if cancel.is_cancelled() {
                                    return Err(error);
                                }
                                tracing::warn!(%error, "scheduled passive audit did not complete")
                            }
                        }
                        if target.scheduled_active {
                            otto_security::wait_for_idle(&busy, &cancel).await?;
                            if let Some((target, root)) =
                                eligible(&job_ctx, &user, &project, &roots, true)
                                    .await?
                                    .filter(|(target, _)| {
                                        target.confirmed_at == expected.confirmed_at
                                            && target.url == expected.url
                                            && target.scheduled_active
                                    })
                            {
                                match worker
                                    .run_dast(&target, &root, &settings, true, &cancel)
                                    .await
                                {
                                    Ok((run, mut extra)) => {
                                        runs.push(run);
                                        findings.append(&mut extra);
                                    }
                                    Err(error) => {
                                        if cancel.is_cancelled() {
                                            return Err(error);
                                        }
                                        tracing::warn!(%error, "scheduled active DAST did not complete")
                                    }
                                }
                            }
                        }
                    }
                }
                Ok((runs, findings))
            },
        )
        .await
}

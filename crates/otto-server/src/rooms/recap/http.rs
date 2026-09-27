use super::super::{auth::owner_auth, recap_engines};
use super::{archive, store, Capture};
use crate::{
    auth::{require_root, CurrentAuthContext},
    ApiResult, ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use otto_core::{api::*, Error, Id, Result};
use serde::Deserialize;
pub async fn settings(ctx: &ServerCtx) -> Result<RecapEngineSettings> {
    Ok(otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("room_recap_engine")
        .await?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| Error::Internal(e.to_string()))?
        .unwrap_or_default())
}
pub fn config(s: RecapEngineSettings) -> recap_engines::EngineConfig {
    recap_engines::EngineConfig {
        whisper_executable: s.whisper_executable,
        whisper_model: s.whisper_model,
        language: s.language,
        threads: s.threads,
    }
}
pub async fn get_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<RecapEngineSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    Ok(Json(settings(&ctx).await?))
}
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RecapEngineSettings>,
) -> ApiResult<Json<RecapEngineSettings>> {
    owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    if !(1..=4).contains(&req.threads)
        || !matches!(req.language.as_str(), "auto" | "en" | "he")
        || [&req.whisper_executable, &req.whisper_model]
            .iter()
            .any(|p| p.len() > 4096 || (!p.is_empty() && !std::path::Path::new(p).is_absolute()))
    {
        return Err(Error::Invalid(
            "Use absolute executable/model paths, auto/en/he language and 1–4 threads".into(),
        )
        .into());
    }
    otto_state::SettingsRepo::new(ctx.pool.clone())
        .put("room_recap_engine", &serde_json::to_value(&req).unwrap())
        .await?;
    Ok(Json(req))
}
pub async fn capabilities(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<serde_json::Value>> {
    owner_auth(&auth)?;
    Ok(Json(
        recap_engines::capabilities(&config(settings(&ctx).await?)).await?,
    ))
}
pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<CreateRecapReq>,
) -> ApiResult<Json<RecapMetadata>> {
    owner_auth(&auth)?;
    let shared = ctx.rooms.get(&id).await?;
    {
        let r = shared.lock().await;
        if r.owner != auth.effective_user.id.as_str() {
            return Err(Error::Forbidden("Only the room owner can create recaps".into()).into());
        }
        r.live()?;
    }
    let capabilities = recap_engines::capabilities(&config(settings(&ctx).await?)).await?;
    let available = capabilities["speech_ready"].as_bool().unwrap_or(false);
    if !available && !req.allow_unavailable_speech {
        return Err(Error::Conflict("Local speech recognition is unavailable. Configure it or explicitly allow a recap with missing speech.".into()).into());
    }
    let storage = store(&ctx).await?;
    let _creation = storage.creation.lock().await;
    let r = shared.lock().await;
    r.live()?;
    if r.recap
        .as_ref()
        .is_some_and(|c| c.view.state != RecapState::Stopped)
    {
        return Err(Error::Conflict("This room already has an active recap".into()).into());
    }
    let handle = ctx
        .manager
        .live_handle(&Id::from(r.session.clone()))
        .filter(|h| h.spawn_seq() == r.spawn_seq && !h.has_exited())
        .ok_or_else(|| Error::Conflict("The shared process is no longer live".into()))?;
    let now = chrono::Utc::now().to_rfc3339();
    let metadata = RecapMetadata {
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: r.owner.clone(),
        room_id: r.id.clone(),
        session_id: r.session.clone(),
        session_title: r.title.clone(),
        created_at: now.clone(),
        updated_at: now,
        status: RecapState::AwaitingConsent,
        capture_epoch: 1,
        bytes_used: 0,
        quota_bytes: archive::QUOTA,
        last_seq: 0,
        speech_available: available,
        speech_error: (!available).then(|| {
            capabilities["configuration_error"]
                .as_str()
                .unwrap_or("Local speech recognition unavailable")
                .into()
        }),
        summary_status: RecapSummaryStatus::Idle,
        summary_error: None,
        summary_through_seq: None,
    };
    drop(r);
    let disk = storage.clone();
    let archive = tokio::task::spawn_blocking(move || disk.create(metadata))
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
    let mut r = shared.lock().await;
    if r.live().is_err() || handle.has_exited() {
        drop(r);
        let target = archive.clone();
        tokio::task::spawn_blocking(move || {
            target.transition(&RecapCapture {
                pending_jobs: 0,
                id: target.metadata().id,
                state: RecapState::Stopped,
                epoch: 2,
                consented_member_ids: vec![],
                reason: Some("Room ended during recap creation".into()),
                started_at: None,
            })
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
        return Err(Error::Conflict("Room ended during recap creation".into()).into());
    }
    let mut capture = Capture::new(archive.clone(), &shared, handle, r.recap_required());
    capture.publish(None);
    r.recap = Some(capture);
    r.changed();
    drop(r);
    Ok(Json(archive.metadata()))
}
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<RecapMetadata>>> {
    owner_auth(&auth)?;
    let store = store(&ctx).await?;
    let owner = auth.effective_user.id.to_string();
    Ok(Json(
        tokio::task::spawn_blocking(move || store.list(&owner))
            .await
            .map_err(|e| Error::Internal(e.to_string()))?,
    ))
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    after: u64,
    #[serde(default = "limit")]
    limit: usize,
}
fn limit() -> usize {
    200
}
pub async fn detail(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<RecapDetail>> {
    owner_auth(&auth)?;
    let a = super::get_archive(&ctx, &id, auth.effective_user.id.as_str()).await?;
    Ok(Json(
        tokio::task::spawn_blocking(move || a.detail(page.after, page.limit))
            .await
            .map_err(|e| Error::Internal(e.to_string()))??,
    ))
}
pub async fn image(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, image)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    owner_auth(&auth)?;
    let a = super::get_archive(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let bytes = tokio::task::spawn_blocking(move || a.image(&image))
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    ))
}
pub async fn export(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    owner_auth(&auth)?;
    let archive = super::get_archive(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let file = tokio::fs::File::open(archive.events_path()?)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    use futures_util::StreamExt;
    use tokio::io::AsyncReadExt;
    let (extent, tail) = archive.export_extent()?;
    let stream =
        tokio_util::io::ReaderStream::new(file.take(extent)).chain(futures_util::stream::once(
            async move { Ok::<_, std::io::Error>(axum::body::Bytes::from(tail)) },
        ));
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=room-recap.jsonl",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(stream),
    ))
}
pub async fn summary(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let store = store(&ctx).await?;
    let lookup = store.clone();
    let owner = auth.effective_user.id.to_string();
    let archive = tokio::task::spawn_blocking(move || lookup.get_summary(&id, &owner))
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
    let permit = store
        .summary
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Conflict("Local processing is busy; retry shortly".into()))?;
    let input_archive = archive.clone();
    let (events, images) = tokio::task::spawn_blocking(move || -> Result<_> {
        let events = input_archive.summary_events()?;
        let images = input_archive.representative_images(&events)?;
        Ok((events, images))
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))??;
    let through = events.last().and_then(|e| e["seq"].as_u64()).unwrap_or(0);
    let generation = {
        let mut s = archive.state.lock().unwrap();
        s.summary_generation += 1;
        s.metadata.summary_status = RecapSummaryStatus::Running;
        s.metadata.summary_error = None;
        archive.save(&s)?;
        s.summary_generation
    };
    let task_archive = archive.clone();
    let task = tokio::spawn(async move {
        let archive = task_archive;
        let _permit = permit;
        let result = recap_engines::summarize(&events, &images).await;
        let mut s = archive.state.lock().unwrap();
        if s.summary_generation != generation {
            return;
        }
        match result {
            Ok(d) => {
                let draft = RecapDraft {
                    overview: d.overview,
                    decisions: d.decisions,
                    actions: d.actions,
                    open_questions: d.open_questions,
                    coverage: d.coverage,
                    source_event_ids: d.source_event_ids,
                };
                match archive.draft(&draft, &mut s) {
                    Ok(()) => {
                        s.metadata.summary_status = RecapSummaryStatus::Ready;
                        s.metadata.summary_through_seq = Some(through);
                    }
                    Err(e) => {
                        s.metadata.summary_status = RecapSummaryStatus::Error;
                        s.metadata.summary_error = Some(e.to_string());
                    }
                }
            }
            Err(e) => {
                s.metadata.summary_status = RecapSummaryStatus::Error;
                s.metadata.summary_error = Some(e.to_string());
            }
        }
        let _ = archive.save(&s);
    });
    archive.state.lock().unwrap().summary_task = Some(task.abort_handle());
    Ok(Json(RecapQueued { queued: true }))
}
pub async fn cancel_summary(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    owner_auth(&auth)?;
    let a = super::get_archive(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let mut s = a.state.lock().unwrap();
    s.summary_generation += 1;
    if let Some(task) = s.summary_task.take() {
        task.abort();
    }
    s.metadata.summary_status = RecapSummaryStatus::Cancelled;
    a.save(&s)?;
    Ok(StatusCode::NO_CONTENT)
}
pub use super::media::{audio, gap, screen};

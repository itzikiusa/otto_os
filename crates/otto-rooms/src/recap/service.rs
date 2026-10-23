//! Recap operations behind the owner routes. Each handler authenticates the
//! owner (`auth::owner_auth`, plus root for settings) and passes the effective
//! user id as `owner`; every archive lookup is scoped to that owner.
use super::{archive, store, Capture};
use crate::{recap_engines, RoomsCtx};
use otto_core::{api::*, Error, Result};
use serde::Deserialize;
pub async fn settings<C: RoomsCtx>(ctx: &C) -> Result<RecapEngineSettings> {
    Ok(ctx
        .setting("room_recap_engine")
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
pub async fn put_settings<C: RoomsCtx>(
    ctx: &C,
    req: RecapEngineSettings,
) -> Result<RecapEngineSettings> {
    if !(1..=4).contains(&req.threads)
        || !matches!(req.language.as_str(), "auto" | "en" | "he")
        || [&req.whisper_executable, &req.whisper_model]
            .iter()
            .any(|p| p.len() > 4096 || (!p.is_empty() && !std::path::Path::new(p).is_absolute()))
    {
        return Err(Error::Invalid(
            "Use absolute executable/model paths, auto/en/he language and 1–4 threads".into(),
        ));
    }
    ctx.put_setting("room_recap_engine", &serde_json::to_value(&req).unwrap())
        .await?;
    Ok(req)
}
pub async fn capabilities<C: RoomsCtx>(ctx: &C) -> Result<serde_json::Value> {
    recap_engines::capabilities(&config(settings(ctx).await?)).await
}
/// Open a recap archive for the owner's live room (consent comes later).
pub async fn create<C: RoomsCtx>(
    ctx: &C,
    owner: &str,
    id: &str,
    req: CreateRecapReq,
) -> Result<RecapMetadata> {
    let shared = ctx.rooms().get(id).await?;
    {
        let r = shared.lock().await;
        if r.owner != owner {
            return Err(Error::Forbidden(
                "Only the room owner can create recaps".into(),
            ));
        }
        r.live()?;
    }
    let capabilities = recap_engines::capabilities(&config(settings(ctx).await?)).await?;
    let available = capabilities["speech_ready"].as_bool().unwrap_or(false);
    if !available && !req.allow_unavailable_speech {
        return Err(Error::Conflict("Local speech recognition is unavailable. Configure it or explicitly allow a recap with missing speech.".into()));
    }
    let storage = store(ctx).await?;
    let _creation = storage.creation.lock().await;
    let r = shared.lock().await;
    r.live()?;
    if r.recap
        .as_ref()
        .is_some_and(|c| c.view.state != RecapState::Stopped)
    {
        return Err(Error::Conflict(
            "This room already has an active recap".into(),
        ));
    }
    let handle = ctx
        .live_handle(&r.session)
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
        return Err(Error::Conflict("Room ended during recap creation".into()));
    }
    let mut capture = Capture::new(archive.clone(), &shared, handle, r.recap_required());
    capture.publish(None);
    r.recap = Some(capture);
    r.changed();
    drop(r);
    Ok(archive.metadata())
}
pub async fn list<C: RoomsCtx>(ctx: &C, owner: &str) -> Result<Vec<RecapMetadata>> {
    let store = store(ctx).await?;
    let owner = owner.to_owned();
    tokio::task::spawn_blocking(move || store.list(&owner))
        .await
        .map_err(|e| Error::Internal(e.to_string()))
}
/// `?after=&limit=` for the detail route.
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    pub after: u64,
    #[serde(default = "limit")]
    pub limit: usize,
}
fn limit() -> usize {
    200
}
pub async fn revision<C: RoomsCtx>(ctx: &C, owner: &str, id: String) -> Result<RecapRevision> {
    let store = store(ctx).await?;
    let owner = owner.to_owned();
    tokio::task::spawn_blocking(move || store.revision(&id, &owner))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}
pub async fn detail<C: RoomsCtx>(
    ctx: &C,
    owner: &str,
    id: &str,
    page: Page,
) -> Result<RecapDetail> {
    let a = super::get_archive(ctx, id, owner).await?;
    tokio::task::spawn_blocking(move || a.detail(page.after, page.limit))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}
/// One stored JPEG of the owner's archive.
pub async fn image<C: RoomsCtx>(ctx: &C, owner: &str, id: &str, image: String) -> Result<Vec<u8>> {
    let a = super::get_archive(ctx, id, owner).await?;
    tokio::task::spawn_blocking(move || a.image(&image))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}
/// The NDJSON export: the first `extent` bytes of `file` (opened before the
/// extent is read, so they are already on disk), then `tail`.
pub struct Export {
    pub file: tokio::fs::File,
    pub extent: u64,
    pub tail: Vec<u8>,
}
pub async fn export<C: RoomsCtx>(ctx: &C, owner: &str, id: &str) -> Result<Export> {
    let archive = super::get_archive(ctx, id, owner).await?;
    let file = tokio::fs::File::open(archive.events_path()?)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    let (extent, tail) = archive.export_extent()?;
    Ok(Export { file, extent, tail })
}
/// Queue a draft summary; a newer request or a cancel supersedes it.
pub async fn summary<C: RoomsCtx>(ctx: &C, owner: &str, id: String) -> Result<RecapQueued> {
    let store = store(ctx).await?;
    let lookup = store.clone();
    let owner = owner.to_owned();
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
    Ok(RecapQueued { queued: true })
}
pub async fn cancel_summary<C: RoomsCtx>(ctx: &C, owner: &str, id: &str) -> Result<()> {
    let a = super::get_archive(ctx, id, owner).await?;
    let mut s = a.state.lock().unwrap();
    s.summary_generation += 1;
    if let Some(task) = s.summary_task.take() {
        task.abort();
    }
    s.metadata.summary_status = RecapSummaryStatus::Cancelled;
    a.save(&s)
}

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
    let (generation, snapshot) = {
        let mut s = archive.state.lock().unwrap();
        s.summary_generation += 1;
        s.metadata.summary_status = RecapSummaryStatus::Running;
        s.metadata.summary_error = None;
        (s.summary_generation, s.metadata.clone())
    };
    persist_metadata_snapshot(archive.clone(), generation, snapshot).await?;
    let task_archive = archive.clone();
    let task = tokio::spawn(async move {
        let archive = task_archive;
        let _permit = permit;
        let result = recap_engines::summarize(&events, &images).await;
        let write_archive = archive.clone();
        let _ = tokio::task::spawn_blocking(move || {
            persist_summary_outcome(&write_archive, generation, through, result)
        })
        .await;
    });
    {
        let mut s = archive.state.lock().unwrap();
        if s.summary_generation == generation {
            s.summary_task = Some(task.abort_handle());
        } else {
            task.abort();
        }
    }
    Ok(RecapQueued { queued: true })
}
pub async fn cancel_summary<C: RoomsCtx>(ctx: &C, owner: &str, id: &str) -> Result<()> {
    let a = super::get_archive(ctx, id, owner).await?;
    let (generation, snapshot) = {
        let mut s = a.state.lock().unwrap();
        s.summary_generation += 1;
        if let Some(task) = s.summary_task.take() {
            task.abort();
        }
        s.metadata.summary_status = RecapSummaryStatus::Cancelled;
        (s.summary_generation, s.metadata.clone())
    };
    persist_metadata_snapshot(a, generation, snapshot).await
}

/// Atomically persist a cloned `RecapMetadata` snapshot on the blocking pool
/// without holding `archive.state` across disk I/O / `fsync`. If a newer
/// generation superseded `generation` before or during the write, the latest
/// metadata snapshot wins on disk so stale writes never clobber a cancel.
async fn persist_metadata_snapshot(
    archive: std::sync::Arc<archive::Archive>,
    generation: u64,
    snapshot: RecapMetadata,
) -> Result<()> {
    tokio::task::spawn_blocking(move || persist_metadata_checked(&archive, generation, snapshot))
        .await
        .map_err(|e| Error::Internal(format!("Recap archive: {e}")))?
}

fn persist_metadata_checked(
    archive: &archive::Archive,
    generation: u64,
    snapshot: RecapMetadata,
) -> Result<()> {
    // Skip stale writes if a newer generation already took over before disk I/O.
    if archive.state.lock().unwrap().summary_generation != generation {
        return Ok(());
    }
    write_metadata_atomic(&archive.path, &snapshot)?;
    // If a newer generation raced with our rename, re-persist the newest
    // metadata so on-disk state always reflects the winning generation.
    let newer = {
        let s = archive.state.lock().unwrap();
        (s.summary_generation != generation).then(|| s.metadata.clone())
    };
    if let Some(latest) = newer {
        let _ = write_metadata_atomic(&archive.path, &latest);
    }
    Ok(())
}

/// Blocking-pool helper that writes `draft.json` (if any) and `metadata.json`
/// without holding `archive.state` across `fsync`/rename, preserving
/// `summary_generation` checks on cancel.
fn persist_summary_outcome(
    archive: &archive::Archive,
    generation: u64,
    through: u64,
    result: Result<recap_engines::SummaryDraft>,
) {
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
            // Serialize and stage draft.json off-lock so archive.state is never
            // held across blocking disk I/O / fsync.
            let staged = stage_draft_off_lock(archive, generation, &draft);
            let snapshot = {
                let mut s = archive.state.lock().unwrap();
                if s.summary_generation != generation {
                    if let Ok((temp, _)) = staged {
                        let _ = std::fs::remove_file(temp);
                    }
                    return;
                }
                match staged {
                    Ok((temp, new_bytes)) => {
                        let dest = archive.path.join("draft.json");
                        match std::fs::rename(&temp, &dest) {
                            Ok(()) => {
                                s.metadata.bytes_used = new_bytes;
                                s.metadata.summary_status = RecapSummaryStatus::Ready;
                                s.metadata.summary_through_seq = Some(through);
                            }
                            Err(e) => {
                                let _ = std::fs::remove_file(&temp);
                                s.metadata.summary_status = RecapSummaryStatus::Error;
                                s.metadata.summary_error = Some(format!("Recap archive: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        s.metadata.summary_status = RecapSummaryStatus::Error;
                        s.metadata.summary_error = Some(e.to_string());
                    }
                }
                s.metadata.clone()
            };
            let _ = persist_metadata_checked(archive, generation, snapshot);
        }
        Err(e) => {
            let snapshot = {
                let mut s = archive.state.lock().unwrap();
                if s.summary_generation != generation {
                    return;
                }
                s.metadata.summary_status = RecapSummaryStatus::Error;
                s.metadata.summary_error = Some(e.to_string());
                s.metadata.clone()
            };
            let _ = persist_metadata_checked(archive, generation, snapshot);
        }
    }
}

fn stage_draft_off_lock(
    archive: &archive::Archive,
    generation: u64,
    draft: &RecapDraft,
) -> Result<(std::path::PathBuf, u64)> {
    let (bytes_used, quota_bytes) = {
        let s = archive.state.lock().unwrap();
        if s.summary_generation != generation {
            return Err(Error::Conflict("Summary was superseded".into()));
        }
        (s.metadata.bytes_used, s.metadata.quota_bytes)
    };
    let bytes =
        serde_json::to_vec(draft).map_err(|e| Error::Internal(format!("Recap archive: {e}")))?;
    let dest = archive.path.join("draft.json");
    let previous = if dest.exists() {
        let meta = std::fs::symlink_metadata(&dest)
            .map_err(|e| Error::Internal(format!("Recap archive: {e}")))?;
        if meta.file_type().is_symlink() {
            return Err(Error::Forbidden(
                "Archive symlinks are not supported".into(),
            ));
        }
        meta.len()
    } else {
        0
    };
    let total = bytes_used.saturating_sub(previous) + bytes.len() as u64;
    if total + 8192 > quota_bytes {
        return Err(Error::Conflict(
            "Recap quota cannot fit the draft; export the transcript".into(),
        ));
    }
    let temp = dest.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    write_private_synced(&temp, &bytes)?;
    Ok((temp, total))
}

fn write_metadata_atomic(archive_dir: &std::path::Path, metadata: &RecapMetadata) -> Result<()> {
    let path = archive_dir.join("metadata.json");
    let bytes =
        serde_json::to_vec(metadata).map_err(|e| Error::Internal(format!("Recap archive: {e}")))?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    write_private_synced(&temp, &bytes)?;
    std::fs::rename(&temp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        Error::Internal(format!("Recap archive: {e}"))
    })
}

fn write_private_synced(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| Error::Internal(format!("Recap archive: {e}")))?;
    if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_data()) {
        let _ = std::fs::remove_file(path);
        return Err(Error::Internal(format!("Recap archive: {e}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata(owner: &str) -> RecapMetadata {
        let now = chrono::Utc::now().to_rfc3339();
        RecapMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: owner.into(),
            room_id: uuid::Uuid::new_v4().to_string(),
            session_id: "s1".into(),
            session_title: "Session".into(),
            created_at: now.clone(),
            updated_at: now,
            status: RecapState::Stopped,
            capture_epoch: 1,
            bytes_used: 0,
            quota_bytes: archive::QUOTA,
            last_seq: 0,
            speech_available: false,
            speech_error: None,
            summary_status: RecapSummaryStatus::Idle,
            summary_error: None,
            summary_through_seq: None,
        }
    }

    #[tokio::test]
    async fn offloaded_metadata_save_preserves_generation_check_on_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let store = archive::Store::open(dir.path().join("archives")).unwrap();
        let arch = store.create(sample_metadata("alice")).unwrap();

        // Generation 1 starts running.
        let (gen1, snap1) = {
            let mut s = arch.state.lock().unwrap();
            s.summary_generation += 1;
            s.metadata.summary_status = RecapSummaryStatus::Running;
            (s.summary_generation, s.metadata.clone())
        };
        persist_metadata_snapshot(arch.clone(), gen1, snap1)
            .await
            .unwrap();

        // Generation 2 cancels before generation 1 finishes.
        let (gen2, snap2) = {
            let mut s = arch.state.lock().unwrap();
            s.summary_generation += 1;
            s.metadata.summary_status = RecapSummaryStatus::Cancelled;
            (s.summary_generation, s.metadata.clone())
        };
        persist_metadata_snapshot(arch.clone(), gen2, snap2)
            .await
            .unwrap();

        // Stale generation 1 finishing must NOT overwrite Cancelled status.
        persist_summary_outcome(
            &arch,
            gen1,
            10,
            Ok(recap_engines::SummaryDraft {
                overview: "stale draft".into(),
                decisions: vec![],
                actions: vec![],
                open_questions: vec![],
                coverage: vec![],
                source_event_ids: vec![],
            }),
        );
        assert_eq!(
            arch.metadata().summary_status,
            RecapSummaryStatus::Cancelled
        );
        assert!(!arch.path.join("draft.json").exists());
        let on_disk: RecapMetadata =
            serde_json::from_slice(&std::fs::read(arch.path.join("metadata.json")).unwrap())
                .unwrap();
        assert_eq!(on_disk.summary_status, RecapSummaryStatus::Cancelled);
    }
}

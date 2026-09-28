//! Media accepts owner uploads only, with consent and source identity checked twice.
use super::super::{auth::owner_auth, recap_engines, registry::SharedRoom};
use super::{
    http::{config, settings},
    store, Capture,
};
use crate::{auth::CurrentAuthContext, ApiResult, ServerCtx};
use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use otto_core::{api::*, Error, Result};
async fn room(ctx: &ServerCtx, id: &str, owner: &str) -> Result<SharedRoom> {
    let a = super::get_archive(ctx, id, owner).await?;
    let shared = ctx.rooms.get(&a.metadata().room_id).await?;
    {
        let r = shared.lock().await;
        r.live()?;
        if !r.members.get(&r.host).is_some_and(|m| m.view.connected)
            || r.recap.as_ref().is_none_or(|c| c.view.id != id)
        {
            return Err(Error::Forbidden(
                "Recap capture is no longer connected".into(),
            ));
        }
    }
    Ok(shared)
}
async fn while_consented<F: std::future::Future>(
    future: F,
    mut control: tokio::sync::watch::Receiver<Option<RecapCapture>>,
    epoch: u64,
) -> Option<F::Output> {
    if !control.borrow().as_ref().is_some_and(|c| {
        matches!(c.state, RecapState::Capturing | RecapState::Finalizing) && c.epoch == epoch
    }) {
        return None;
    }
    tokio::pin!(future);
    loop {
        tokio::select! {biased; changed=control.changed()=>{if changed.is_err()||!control.borrow().as_ref().is_some_and(|c|matches!(c.state,RecapState::Capturing|RecapState::Finalizing)&&c.epoch==epoch){return None;}}, result=&mut future=>return Some(result)}
    }
}
fn active(c: &Capture, epoch: u64, offset: u64) -> Result<()> {
    if !c.accepts(epoch) {
        return Err(Error::Forbidden("Capture consent epoch expired".into()));
    }
    if c.started
        .is_none_or(|t| offset > t.elapsed().as_millis() as u64 + 5000)
    {
        return Err(Error::Invalid(
            "Media offset is outside the current capture interval".into(),
        ));
    }
    Ok(())
}
fn sequence(c: &mut Capture, key: String, seq: u64, offset: u64, screen: bool) -> Result<()> {
    if seq == 0
        || c.sequences.get(&key).is_some_and(|(last, at)| {
            seq <= *last || offset < *at || (screen && offset.saturating_sub(*at) < 15000)
        })
    {
        return Err(Error::Conflict(
            "Duplicate, reordered or too frequent capture chunk".into(),
        ));
    }
    c.sequences.insert(key, (seq, offset));
    Ok(())
}
fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
fn audio_interval(windows: &[(u64, Option<u64>)], offset: u64, end: u64) -> bool {
    windows.iter().any(|(start, stop)| {
        offset.saturating_add(500) >= *start
            && stop.is_none_or(|stop| end <= stop.saturating_add(500))
    })
}
fn wav(bytes: &[u8], duration: u64) -> Result<()> {
    let actual = recap_engines::validate_wav(bytes)?;
    if duration == 0 || duration > 30000 || actual.abs_diff(duration) > 1 {
        return Err(invalid("WAV duration does not match metadata"));
    }
    Ok(())
}
fn jpeg(bytes: &[u8]) -> Result<()> {
    recap_engines::jpeg_dimensions(bytes).map(|_| ())
}
pub async fn audio(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapAudioReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let shared = room(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let bytes = STANDARD
        .decode(&req.wav_base64)
        .map_err(|_| invalid("Invalid base64 WAV"))?;
    wav(&bytes, req.duration_ms)?;
    let store = store(&ctx).await?;
    let permit = store
        .queued
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Conflict("Local processing is busy; retry shortly".into()))?;
    let cfg = config(settings(&ctx).await?);
    let (name, control, job) = {
        let mut r = shared.lock().await;
        let m = r.admitted(&req.member_id)?;
        if !m.view.connected || m.view.generation != req.member_generation {
            return Err(Error::Forbidden("Participant audio is not shared".into()).into());
        }
        let name = m.view.name.clone();
        let c = r.recap.as_mut().ok_or_else(|| invalid("Recap ended"))?;
        if c.view.id != id {
            return Err(Error::Forbidden("Recap was replaced".into()).into());
        }
        active(
            c,
            req.capture_epoch,
            req.offset_ms
                .checked_add(req.duration_ms)
                .ok_or_else(|| invalid("Audio offset overflow"))?,
        )?;
        let end = req
            .offset_ms
            .checked_add(req.duration_ms)
            .ok_or_else(|| invalid("Audio offset overflow"))?;
        if !c
            .audio_windows
            .get(&req.member_id)
            .is_some_and(|windows| audio_interval(windows, req.offset_ms, end))
        {
            return Err(Error::Forbidden(
                "Audio chunk falls outside a shared audible interval".into(),
            )
            .into());
        }
        sequence(
            c,
            format!("audio:{}", req.member_id),
            req.sequence,
            req.offset_ms,
            false,
        )?;
        c.enqueue(
            req.capture_epoch,
            RecapEventData::AudioPending {
                member_id: req.member_id.clone(),
                member_name: name.clone(),
                sequence: req.sequence,
                offset_ms: req.offset_ms,
                duration_ms: req.duration_ms,
            },
            None,
        )?;
        {
            let job = super::JobGuard::new(c, &shared);
            let control = c.control.subscribe();
            r.changed();
            (name, control, job)
        }
    };
    tokio::spawn(async move {
        let _permit = permit;
        let _job = job;
        let Some(result) = while_consented(
            async {
                let _processing = store
                    .inference
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| invalid("Processor stopped"))?;
                recap_engines::transcribe(&cfg, &bytes).await
            },
            control,
            req.capture_epoch,
        )
        .await
        else {
            return;
        };
        let mut r = shared.lock().await;
        if !r
            .recap
            .as_ref()
            .is_some_and(|c| c.processing(req.capture_epoch) && c.view.id == id)
            || !r
                .members
                .get(&req.member_id)
                .is_some_and(|m| m.view.connected && m.view.generation == req.member_generation)
        {
            return;
        }
        let event = match result {
            Ok(segments) => RecapEventData::Speech {
                member_id: req.member_id,
                member_name: name,
                sequence: req.sequence,
                offset_ms: req.offset_ms,
                segments: segments
                    .into_iter()
                    .map(|s| RecapSpeechSegment {
                        start_ms: s.start_ms,
                        end_ms: s.end_ms,
                        text: s.text,
                    })
                    .collect(),
            },
            Err(e) => RecapEventData::Gap {
                kind: "speech".into(),
                member_id: Some(req.member_id),
                source_id: None,
                reason: e.to_string(),
            },
        };
        r.record_processed(req.capture_epoch, event);
    });
    Ok(Json(RecapQueued { queued: true }))
}
pub async fn screen(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapScreenReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    let shared = room(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let bytes = STANDARD
        .decode(&req.jpeg_base64)
        .map_err(|_| invalid("Invalid base64 JPEG"))?;
    jpeg(&bytes)?;
    let storage = store(&ctx).await?;
    let permit = storage
        .queued
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Conflict("Local processing is busy; retry shortly".into()))?;
    let (title, control, job) = {
        let mut r = shared.lock().await;
        let source = r
            .presentations
            .iter()
            .find(|p| p.id == req.source_id && p.generation == req.source_generation)
            .ok_or_else(|| Error::Forbidden("Source generation expired".into()))?;
        let title = source.title.clone();
        let c = r.recap.as_mut().ok_or_else(|| invalid("Recap ended"))?;
        if c.view.id != id {
            return Err(Error::Forbidden("Recap was replaced".into()).into());
        }
        active(c, req.capture_epoch, req.offset_ms)?;
        sequence(
            c,
            format!("screen:{}:{}", req.source_id, req.source_generation),
            req.sequence,
            req.offset_ms,
            true,
        )?;
        {
            let job = super::JobGuard::new(c, &shared);
            let control = c.control.subscribe();
            r.changed();
            (title, control, job)
        }
    };
    tokio::spawn(async move {
        let _permit = permit;
        let _job = job;
        let Some(result) = while_consented(
            async {
                let _processing = storage
                    .inference
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| invalid("Processor stopped"))?;
                {
                    let r = shared.lock().await;
                    if !r
                        .presentations
                        .iter()
                        .any(|p| p.id == req.source_id && p.generation == req.source_generation)
                    {
                        return Err(Error::Forbidden(
                            "Screen source changed before recognition".into(),
                        ));
                    }
                }
                recap_engines::recognize_screen(&bytes).await
            },
            control,
            req.capture_epoch,
        )
        .await
        else {
            return;
        };
        let mut r = shared.lock().await;
        if !r
            .recap
            .as_ref()
            .is_some_and(|c| c.processing(req.capture_epoch) && c.view.id == id)
        {
            return;
        }
        if !r
            .presentations
            .iter()
            .any(|p| p.id == req.source_id && p.generation == req.source_generation)
        {
            r.record_processed(
                req.capture_epoch,
                RecapEventData::Gap {
                    kind: "screen".into(),
                    member_id: None,
                    source_id: Some(req.source_id),
                    reason: "Screen sample processing was invalidated by a source change".into(),
                },
            );
            return;
        }
        let (text, error) = match result {
            Ok(t) => (t, None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        let queued = r.recap.as_ref().unwrap().enqueue(
            req.capture_epoch,
            RecapEventData::Screen {
                source_id: req.source_id.clone(),
                source_generation: req.source_generation,
                title,
                image_id: String::new(),
                offset_ms: req.offset_ms,
                text,
            },
            Some(bytes),
        );
        if queued.is_err() {
            r.recap_interrupt("Archive capture queue is full", false);
            r.changed();
        }
        if let Some(reason) = error {
            r.record_processed(
                req.capture_epoch,
                RecapEventData::Gap {
                    kind: "screen_text".into(),
                    member_id: None,
                    source_id: Some(req.source_id),
                    reason,
                },
            );
        }
    });
    Ok(Json(RecapQueued { queued: true }))
}
pub async fn gap(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<RecapGapReq>,
) -> ApiResult<Json<RecapQueued>> {
    owner_auth(&auth)?;
    if req.kind.is_empty()
        || req.kind.len() > 64
        || req.reason.is_empty()
        || req.reason.len() > 1024
    {
        return Err(invalid("Gap kind/reason is empty or too long").into());
    }
    let shared = room(&ctx, &id, auth.effective_user.id.as_str()).await?;
    let mut r = shared.lock().await;
    let c = r.recap.as_ref().ok_or_else(|| invalid("Recap ended"))?;
    if c.view.id != id {
        return Err(Error::Forbidden("Recap was replaced".into()).into());
    }
    active(c, req.capture_epoch, 0)?;
    if let Some(member) = &req.member_id {
        r.admitted(member)?;
    }
    if req
        .source_id
        .as_ref()
        .is_some_and(|id| !r.presentations.iter().any(|p| &p.id == id))
    {
        return Err(invalid("Source no longer exists").into());
    }
    r.record_processed(
        req.capture_epoch,
        RecapEventData::Gap {
            kind: req.kind,
            member_id: req.member_id,
            source_id: req.source_id,
            reason: req.reason,
        },
    );
    Ok(Json(RecapQueued { queued: true }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_finalization_keeps_already_accepted_recognition() {
        let view = RecapCapture {
            pending_jobs: 1,
            id: "archive".into(),
            state: RecapState::Capturing,
            epoch: 1,
            consented_member_ids: vec![],
            reason: None,
            started_at: None,
        };
        let (tx, rx) = tokio::sync::watch::channel(Some(view.clone()));
        let (complete, done) = tokio::sync::oneshot::channel();
        let job = tokio::spawn(while_consented(async move { done.await.unwrap() }, rx, 1));
        tx.send_replace(Some(RecapCapture {
            state: RecapState::Finalizing,
            ..view
        }));
        complete.send("final words").unwrap();
        assert_eq!(job.await.unwrap(), Some("final words"));
    }
    #[tokio::test]
    async fn withdrawal_cancels_pending_recognition_and_releases_processor() {
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let view = RecapCapture {
            pending_jobs: 0,
            id: "archive".into(),
            state: RecapState::Finalizing,
            epoch: 1,
            consented_member_ids: vec![],
            reason: None,
            started_at: None,
        };
        let (tx, rx) = tokio::sync::watch::channel(Some(view.clone()));
        let sem = semaphore.clone();
        let job = tokio::spawn(while_consented(
            async move {
                let _permit = sem.acquire_owned().await.unwrap();
                std::future::pending::<()>().await
            },
            rx,
            1,
        ));
        tokio::task::yield_now().await;
        tx.send_replace(Some(RecapCapture {
            state: RecapState::Paused,
            epoch: 2,
            ..view
        }));
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), job)
            .await
            .unwrap()
            .unwrap()
            .is_none());
        assert!(semaphore.try_acquire().is_ok());
    }
    #[test]
    fn final_pre_mute_chunk_is_kept_but_post_mute_speech_is_rejected() {
        let windows = [(1000, Some(6000))];
        assert!(audio_interval(&windows, 1000, 6000));
        assert!(audio_interval(&windows, 1000, 6400));
        assert!(!audio_interval(&windows, 1000, 7000));
        assert!(!audio_interval(&windows, 0, 6000));
        assert!(!audio_interval(&[], 1000, 6000));
    }
    #[test]
    fn rejects_non_wav_and_oversized_images() {
        assert!(wav(b"not audio", 1).is_err());
        assert!(jpeg(&vec![0; 256 * 1024 + 1]).is_err());
        assert!(jpeg(&[255, 216, 255, 217]).is_err());
    }
}

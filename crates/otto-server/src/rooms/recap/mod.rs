//! Explicit consent gates capture; archive writes run away from PTY readers.
pub(super) mod archive;
mod consent;
pub(super) mod http;
mod media;
use super::registry::{Room, SharedRoom};
use archive::{Archive, Store};
use base64::{engine::general_purpose::STANDARD, Engine};
use consent::Consent;
use otto_core::{api::*, Error, Result};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
    time::Instant,
};
use tokio::sync::{mpsc, Mutex};
struct ImageJob {
    bytes: Vec<u8>,
    source: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
}
pub(super) struct Capture {
    queued: Arc<std::sync::atomic::AtomicU64>,
    sources: HashMap<String, Arc<std::sync::atomic::AtomicU64>>,
    pub view: RecapCapture,
    consent: Consent,
    pub archive: Arc<Archive>,
    control: tokio::sync::watch::Sender<Option<RecapCapture>>,
    queue: mpsc::Sender<(u64, RecapEventData, Option<ImageJob>)>,
    room: Weak<Mutex<Room>>,
    handle: Arc<otto_pty::PtyHandle>,
    terminal: Option<tokio::task::JoinHandle<()>>,
    pub started: Option<Instant>,
    pub sequences: HashMap<String, (u64, u64)>,
    pub audio_windows: HashMap<String, Vec<(u64, Option<u64>)>>,
}
struct PendingWrite(Arc<std::sync::atomic::AtomicU64>);
impl Drop for PendingWrite {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
pub(super) struct JobGuard {
    room: Weak<Mutex<Room>>,
    id: String,
    epoch: u64,
}
impl JobGuard {
    fn new(c: &mut Capture, room: &SharedRoom) -> Self {
        c.view.pending_jobs += 1;
        Self {
            room: Arc::downgrade(room),
            id: c.view.id.clone(),
            epoch: c.view.epoch,
        }
    }
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        let room = self.room.clone();
        let id = self.id.clone();
        let epoch = self.epoch;
        tokio::spawn(async move {
            if let Some(room) = room.upgrade() {
                let mut r = room.lock().await;
                if let Some(c) = &mut r.recap {
                    if c.view.id == id && c.view.epoch == epoch {
                        c.view.pending_jobs = c.view.pending_jobs.saturating_sub(1);
                        r.changed();
                    }
                }
            }
        });
    }
}
async fn finalize(room: Weak<Mutex<Room>>, id: String, epoch: u64) {
    let deadline = Instant::now() + std::time::Duration::from_secs(180);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    loop {
        tick.tick().await;
        let Some(room) = room.upgrade() else {
            return;
        };
        let mut r = room.lock().await;
        let Some(c) = r.recap.as_ref().filter(|c| {
            c.view.id == id && c.view.epoch == epoch && c.view.state == RecapState::Finalizing
        }) else {
            return;
        };
        let drained =
            c.view.pending_jobs == 0 && c.queued.load(std::sync::atomic::Ordering::Acquire) == 0;
        if drained || Instant::now() >= deadline {
            r.recap_interrupt(
                if drained {
                    "The host finished the recap"
                } else {
                    "Finalization timed out; unfinished chunks were discarded"
                },
                true,
            );
            r.changed();
            return;
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(t) = self.terminal.take() {
            t.abort();
        }
    }
}
pub(super) async fn store(ctx: &crate::ServerCtx) -> Result<Arc<Store>> {
    let root = ctx.data_dir.join("room-recaps");
    ctx.rooms
        .recaps
        .get_or_try_init(|| async move {
            tokio::task::spawn_blocking(move || Store::open(root).map(Arc::new))
                .await
                .map_err(|e| Error::Internal(e.to_string()))?
        })
        .await
        .cloned()
}
pub(super) async fn get_archive(
    ctx: &crate::ServerCtx,
    id: &str,
    owner: &str,
) -> Result<Arc<Archive>> {
    let store = store(ctx).await?;
    let id = id.to_owned();
    let owner = owner.to_owned();
    tokio::task::spawn_blocking(move || store.get(&id, &owner))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}
impl Capture {
    fn new(
        archive: Arc<Archive>,
        room: &SharedRoom,
        handle: Arc<otto_pty::PtyHandle>,
        required: BTreeSet<String>,
    ) -> Self {
        let (queue, mut rx) = mpsc::channel::<(u64, RecapEventData, Option<ImageJob>)>(128);
        let (control, mut control_rx) = tokio::sync::watch::channel::<Option<RecapCapture>>(None);
        let queued = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let writer_queued = queued.clone();
        let target = archive.clone();
        let weak = Arc::downgrade(room);
        let archive_id = archive.metadata().id;
        tokio::spawn(async move {
            loop {
                let item = tokio::select! {biased;
                    changed=control_rx.changed()=>{if changed.is_err(){break;}let view=control_rx.borrow_and_update().clone();if let Some(view)=view{let a=target.clone();let result=tokio::task::spawn_blocking(move||a.transition(&view)).await;if !matches!(result,Ok(Ok(()))){if let Some(shared)=weak.upgrade(){let mut r=shared.lock().await;if r.recap.as_ref().is_some_and(|c|c.view.id==archive_id){r.recap_interrupt("Archive state could not be saved",true);r.changed();}}break;}}continue;},
                    item=rx.recv()=>item,
                };
                let Some((epoch, mut event, image)) = item else {
                    break;
                };
                let _pending = PendingWrite(writer_queued.clone());
                if let RecapEventData::Screen {
                    source_id,
                    source_generation,
                    ..
                } = &event
                {
                    let Some(shared) = weak.upgrade() else {
                        continue;
                    };
                    let mut r = shared.lock().await;
                    if !r
                        .recap
                        .as_ref()
                        .is_some_and(|c| c.view.id == archive_id && c.processing(epoch))
                    {
                        continue;
                    }
                    if !r
                        .presentations
                        .iter()
                        .any(|p| p.id == *source_id && p.generation == *source_generation)
                    {
                        r.record_processed(
                            epoch,
                            RecapEventData::Gap {
                                kind: "screen".into(),
                                member_id: None,
                                source_id: Some(source_id.clone()),
                                reason: "Queued screen sample was invalidated by a source change"
                                    .into(),
                            },
                        );
                        continue;
                    }
                }
                let a = target.clone();
                let result = tokio::task::spawn_blocking(move || {
                    if let Some(image) = image {
                        let valid = || {
                            a.accepts(epoch)
                                && image.source.load(std::sync::atomic::Ordering::Acquire)
                                    == image.generation
                        };
                        let source_id = if let RecapEventData::Screen { source_id, .. } = &event {
                            Some(source_id.clone())
                        } else {
                            None
                        };
                        let Some(id) = a.put_image(&image.bytes, valid)? else {
                            return a.append(
                                epoch,
                                RecapEventData::Gap {
                                    kind: "screen".into(),
                                    member_id: None,
                                    source_id,
                                    reason: "Screen sample invalidated before storage".into(),
                                },
                            );
                        };
                        if let RecapEventData::Screen { image_id, .. } = &mut event {
                            *image_id = id.clone();
                        }
                        let result = a.append_if(epoch, event, valid);
                        if !matches!(result, Ok(true)) {
                            let _ = a.discard_image(&id, image.bytes.len() as u64);
                        }
                        return result.map(|_| ());
                    }
                    a.append(epoch, event)
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    if let Some(shared) = weak.upgrade() {
                        let mut r = shared.lock().await;
                        if r.recap.as_ref().is_none_or(|c| c.view.id != archive_id) {
                            break;
                        }
                        r.recap_interrupt("Archive storage failed or quota reached", true);
                        r.changed();
                        let view = r.recap.as_ref().map(|c| c.view.clone());
                        drop(r);
                        if let Some(view) = view {
                            let a = target.clone();
                            let _ = tokio::task::spawn_blocking(move || a.transition(&view)).await;
                        }
                    }
                    break;
                }
            }
        });
        Self {
            queued,
            view: RecapCapture {
                pending_jobs: 0,
                id: archive.metadata().id,
                state: RecapState::AwaitingConsent,
                epoch: 1,
                consented_member_ids: vec![],
                reason: None,
                started_at: None,
            },
            consent: Consent::new(required),
            archive,
            queue,
            control,
            room: Arc::downgrade(room),
            handle,
            terminal: None,
            started: None,
            sequences: HashMap::new(),
            audio_windows: HashMap::new(),
            sources: HashMap::new(),
        }
    }
    pub fn processing(&self, epoch: u64) -> bool {
        self.view.epoch == epoch
            && matches!(
                self.view.state,
                RecapState::Capturing | RecapState::Finalizing
            )
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.consent.accepts(epoch)
    }
    fn publish(&mut self, reason: Option<String>) {
        self.view.state = self.consent.state;
        self.view.epoch = self.consent.epoch;
        self.view.consented_member_ids = self.consent.consented.iter().cloned().collect();
        self.view.reason = reason;
        self.archive.set_capture(
            self.view.epoch,
            matches!(
                self.view.state,
                RecapState::Capturing | RecapState::Finalizing
            ),
        );
        self.control.send_replace(Some(self.view.clone()));
        if self.view.state != RecapState::Capturing {
            if let Some(task) = self.terminal.take() {
                task.abort();
            }
            self.started = None;
            self.view.started_at = None;
            self.sequences.clear();
            self.audio_windows.clear();
            if self.view.state != RecapState::Finalizing {
                self.view.pending_jobs = 0;
                for token in self.sources.values() {
                    token.store(0, std::sync::atomic::Ordering::Release);
                }
                self.sources.clear();
            }
        }
    }
    pub fn enqueue(&self, epoch: u64, event: RecapEventData, image: Option<Vec<u8>>) -> Result<()> {
        let image = if let Some(bytes) = image {
            let RecapEventData::Screen {
                source_id,
                source_generation,
                ..
            } = &event
            else {
                return Err(Error::Invalid("Image requires source attribution".into()));
            };
            let source = self
                .sources
                .get(source_id)
                .ok_or_else(|| Error::Forbidden("Source expired".into()))?
                .clone();
            Some(ImageJob {
                bytes,
                source,
                generation: *source_generation,
            })
        } else {
            None
        };
        self.queued
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.queue.try_send((epoch, event, image)).map_err(|_| {
            self.queued
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            Error::Conflict("Archive capture queue is full".into())
        })
    }
    fn start_terminal(&mut self) {
        let mut output = self.handle.subscribe();
        let room = self.room.clone();
        let epoch = self.view.epoch;
        let archive_id = self.view.id.clone();
        self.terminal = Some(tokio::spawn(async move {
            loop {
                match output.recv().await {
                    Ok(bytes) => {
                        let Some(shared) = room.upgrade() else { break };
                        let mut r = shared.lock().await;
                        if !r
                            .recap
                            .as_ref()
                            .is_some_and(|c| c.accepts(epoch) && c.view.id == archive_id)
                        {
                            break;
                        }
                        r.record(RecapEventData::Terminal {
                            data_base64: STANDARD.encode(bytes),
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(shared) = room.upgrade() {
                            let mut r = shared.lock().await;
                            if r.recap.as_ref().is_none_or(|c| c.view.id != archive_id) {
                                break;
                            }
                            r.recap_interrupt(
                                "Terminal capture fell behind; capture paused",
                                false,
                            );
                            r.changed();
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
        }));
    }
}
impl Room {
    fn recap_required(&self) -> BTreeSet<String> {
        self.members
            .values()
            .filter(|m| m.view.connected && m.view.admission == RoomAdmission::Admitted)
            .map(|m| m.view.id.clone())
            .collect()
    }
    pub fn recap_audio_changed(&mut self) {
        let Some(c) = &mut self.recap else {
            return;
        };
        let Some(started) = c.started else {
            return;
        };
        let at = started.elapsed().as_millis() as u64;
        for m in self.members.values() {
            let windows = c.audio_windows.entry(m.view.id.clone()).or_default();
            let audible =
                m.view.connected && m.view.audio_joined && !m.view.muted && !m.view.room_muted;
            let open = windows.last().is_some_and(|(_, end)| end.is_none());
            if audible && !open {
                windows.push((at, None));
            } else if !audible && open {
                windows.last_mut().unwrap().1 = Some(at);
            }
            windows.retain(|(_, end)| end.is_none_or(|end| at.saturating_sub(end) < 60000));
        }
    }
    pub fn recap_interrupt(&mut self, reason: &str, stop: bool) {
        let required = self.recap_required();
        if let Some(c) = &mut self.recap {
            if c.view.state == RecapState::Stopped {
                return;
            }
            c.consent.interrupt(required, stop);
            c.publish(Some(reason.into()));
        }
    }
    pub fn recap_action(&mut self, actor: &str, action: RoomAction) -> Result<()> {
        if !matches!(action, RoomAction::RecapConsent { .. }) {
            self.host(actor)?;
        }
        let required = self.recap_required();
        let c = self
            .recap
            .as_mut()
            .ok_or_else(|| Error::Conflict("Create a recap through the owner API first".into()))?;
        match action {
            RoomAction::RecapConsent { epoch, allow } => {
                c.consent.consent(actor, epoch, allow)?;
                c.publish((!allow).then(|| "A participant withdrew consent".into()));
            }
            RoomAction::RecapStart { epoch } => {
                c.consent.start(epoch)?;
                c.started = Some(Instant::now());
                c.view.started_at = Some(chrono::Utc::now().to_rfc3339());
                c.publish(None);
                c.start_terminal();
            }
            RoomAction::RecapPause => {
                c.consent.interrupt(required, false);
                c.publish(Some(
                    "The host paused capture; unfinished processing was discarded".into(),
                ));
            }
            RoomAction::RecapStop => {
                if c.view.state == RecapState::Stopped || c.view.state == RecapState::Finalizing {
                    return Ok(());
                }
                c.consent.state = RecapState::Finalizing;
                c.publish(Some(
                    "Finishing accepted transcript and screen samples".into(),
                ));
                let room = c.room.clone();
                let id = c.view.id.clone();
                let epoch = c.view.epoch;
                tokio::spawn(finalize(room, id, epoch));
            }
            _ => unreachable!(),
        }
        if matches!(action, RoomAction::RecapStart { .. }) {
            self.recap_audio_changed();
            let members = self
                .members
                .values()
                .filter(|m| m.view.admission == RoomAdmission::Admitted)
                .map(|m| m.view.clone())
                .collect();
            self.record(RecapEventData::Participants { members });
            for presentation in self.presentations.clone() {
                self.record(RecapEventData::Presentation {
                    operation: "current".into(),
                    presentation,
                });
            }
        }
        Ok(())
    }
    pub fn record_processed(&mut self, epoch: u64, payload: RecapEventData) {
        if let Some(c) = &self.recap {
            if c.processing(epoch) && c.enqueue(epoch, payload, None).is_err() {
                self.recap_interrupt("Archive capture queue is full", false);
                self.changed();
            }
        }
    }
    pub fn record(&mut self, payload: RecapEventData) {
        if let Some(c) = &mut self.recap {
            if let RecapEventData::Presentation {
                operation,
                presentation,
            } = &payload
            {
                let prefix = format!("screen:{}:", presentation.id);
                c.sequences.retain(|key, _| !key.starts_with(&prefix));
                if operation == "stop" {
                    if let Some(token) = c.sources.remove(&presentation.id) {
                        token.store(0, std::sync::atomic::Ordering::Release);
                    }
                } else {
                    let token = c
                        .sources
                        .entry(presentation.id.clone())
                        .or_insert_with(|| Arc::new(std::sync::atomic::AtomicU64::new(0)));
                    token.store(
                        presentation.generation,
                        std::sync::atomic::Ordering::Release,
                    );
                }
            }
        }
        if let Some(c) = &self.recap {
            if c.accepts(c.view.epoch) && c.enqueue(c.view.epoch, payload, None).is_err() {
                self.recap_interrupt("Archive capture queue filled; capture paused", false);
                self.changed();
            }
        }
    }
    pub fn record_event(&mut self, event: &RoomEvent) {
        if let Some(c) = &self.recap {
            if !c.accepts(c.view.epoch) {
                return;
            }
            let payload = match event {
                RoomEvent::Annotation { annotation } => RecapEventData::Annotation {
                    annotation: annotation.clone(),
                },
                RoomEvent::ClearAnnotations {
                    source_id,
                    clear_epoch,
                } => RecapEventData::AnnotationsCleared {
                    source_id: source_id.clone(),
                    clear_epoch: *clear_epoch,
                },
                RoomEvent::AnnotationRemoved {
                    source_id,
                    annotation_id,
                } => RecapEventData::AnnotationRemoved {
                    source_id: source_id.clone(),
                    annotation_id: annotation_id.clone(),
                },
                _ => return,
            };
            self.record(payload);
        }
    }
}
#[cfg(test)]
mod finalization_tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finish_waits_for_actual_archive_write_after_recognition_finishes() {
        let registry = super::super::RoomRegistry::new();
        let host = registry
            .create("session", "owner", "Host", "Session", "shell")
            .await
            .unwrap();
        let shared = registry.get(&host.room_id).await.unwrap();
        shared.lock().await.connect(&host.member_id).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("archives")).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let archive = store
            .create(RecapMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: "owner".into(),
                room_id: host.room_id.clone(),
                session_id: "session".into(),
                session_title: "Session".into(),
                created_at: now.clone(),
                updated_at: now,
                status: RecapState::AwaitingConsent,
                capture_epoch: 1,
                bytes_used: 0,
                quota_bytes: archive::QUOTA,
                last_seq: 0,
                speech_available: true,
                speech_error: None,
                summary_status: RecapSummaryStatus::Idle,
                summary_error: None,
                summary_through_seq: None,
            })
            .unwrap();
        let handle = Arc::new(
            otto_pty::PtyHandle::spawn(&otto_pty::CommandSpec {
                program: "/bin/sleep".into(),
                args: vec!["30".into()],
                cwd: None,
                env: vec![],
            })
            .unwrap(),
        );
        {
            let mut r = shared.lock().await;
            r.recap = Some(Capture::new(
                archive.clone(),
                &shared,
                handle,
                r.recap_required(),
            ));
        }
        {
            let mut r = shared.lock().await;
            for n in 0..100 {
                let presentation = RoomPresentation {
                    id: format!("source-{n}"),
                    member_id: host.member_id.clone(),
                    generation: 1,
                    title: "Screen".into(),
                    width: 100,
                    height: 100,
                    clear_epoch: 0,
                };
                r.record(RecapEventData::Presentation {
                    operation: "start".into(),
                    presentation: presentation.clone(),
                });
                let c = r.recap.as_mut().unwrap();
                let token = c.sources[&presentation.id].clone();
                c.sequences
                    .insert(format!("screen:{}:1", presentation.id), (1, 0));
                r.record(RecapEventData::Presentation {
                    operation: "stop".into(),
                    presentation,
                });
                assert_eq!(token.load(std::sync::atomic::Ordering::Acquire), 0);
                assert!(r.recap.as_ref().unwrap().sources.is_empty());
                assert!(r.recap.as_ref().unwrap().sequences.is_empty());
            }
        }
        let (locked, ready) = std::sync::mpsc::channel();
        let (release, unblock) = std::sync::mpsc::channel();
        let blocked = archive.clone();
        let writer_block = std::thread::spawn(move || {
            let _guard = blocked.state.lock().unwrap();
            locked.send(()).unwrap();
            unblock.recv().unwrap();
        });
        ready.recv().unwrap();
        {
            let mut r = shared.lock().await;
            r.recap_action(
                &host.member_id,
                RoomAction::RecapConsent {
                    epoch: 1,
                    allow: true,
                },
            )
            .unwrap();
            r.recap_action(&host.member_id, RoomAction::RecapStart { epoch: 1 })
                .unwrap();
            r.record_processed(
                1,
                RecapEventData::Speech {
                    member_id: host.member_id.clone(),
                    member_name: "Host".into(),
                    sequence: 1,
                    offset_ms: 0,
                    segments: vec![RecapSpeechSegment {
                        start_ms: 0,
                        end_ms: 100,
                        text: "final words".into(),
                    }],
                },
            );
            r.recap_action(&host.member_id, RoomAction::RecapStop)
                .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        {
            let r = shared.lock().await;
            let c = r.recap.as_ref().unwrap();
            assert_eq!(c.view.state, RecapState::Finalizing);
            assert_eq!(c.view.pending_jobs, 0);
            assert!(c.queued.load(std::sync::atomic::Ordering::Acquire) > 0);
        }
        release.send(()).unwrap();
        writer_block.join().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if shared.lock().await.recap.as_ref().unwrap().view.state == RecapState::Stopped {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(archive.detail(0,100).unwrap().events.iter().any(|e|matches!(&e.payload,RecapEventData::Speech{segments,..} if segments[0].text=="final words")));
    }
}

//! Live legacy-source sync: a Canvas scene or a Product story attachment
//! re-mirrors into the graph moments after it changes, instead of at the next
//! daemon start.
//!
//! The loop listens on the event bus for the events that name a source —
//! `CanvasUpdated` (a scene, agent turns), `MockupUpdated` (an attachment:
//! UI saves, agent turns, Blender outputs) and `DesignSourceChanged` (the
//! paths that emit neither: a person's Canvas save, attachment create /
//! delete) — and runs [`import::sync_one`](crate::import::sync_one) per
//! source once it has been quiet for [`DEBOUNCE`], so an agent's per-poll
//! mid-turn writes merge into one sync. A lagged receiver falls back to the
//! full catch-up import (`import::run`), which also runs once at boot.

use std::collections::HashMap;
use std::time::Duration;

use otto_core::event::Event;
use otto_core::Id;
use tokio::sync::broadcast;
use tokio::time::Instant;

use crate::import::{self, SOURCE_ATTACHMENT, SOURCE_SCENE};
use crate::service::DesignService;

/// How long a source must stay quiet before it is synced.
pub const DEBOUNCE: Duration = Duration::from_millis(1_500);

/// One legacy design source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceRef {
    /// A `product_attachments` row.
    Attachment(Id),
    /// A `canvas_scenes` row.
    Scene(Id),
}

impl SourceRef {
    /// From a `source_kind` (`product_attachment` | `canvas_scene`) + id.
    pub fn from_kind(kind: &str, id: &str) -> Option<Self> {
        match kind {
            SOURCE_ATTACHMENT => Some(Self::Attachment(id.to_string())),
            SOURCE_SCENE => Some(Self::Scene(id.to_string())),
            _ => None,
        }
    }

    /// The graph's `source_kind` for this source.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Attachment(_) => SOURCE_ATTACHMENT,
            Self::Scene(_) => SOURCE_SCENE,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Attachment(id) | Self::Scene(id) => id,
        }
    }
}

/// The source an event says changed, if any. The graph's own events
/// (`design_*`) are never sources, so a sync can't feed itself.
pub fn source_of(ev: &Event) -> Option<SourceRef> {
    match ev {
        Event::CanvasUpdated { scene_id, .. } => Some(SourceRef::Scene(scene_id.clone())),
        Event::MockupUpdated { attachment_id, .. } => {
            Some(SourceRef::Attachment(attachment_id.clone()))
        }
        Event::DesignSourceChanged { kind, id, .. } => SourceRef::from_kind(kind, id),
        _ => None,
    }
}

/// Per-source trailing-edge debounce: every touch pushes that source's
/// deadline to `now + window`; a source is due once its deadline passed.
#[derive(Debug)]
pub struct Debouncer {
    window: Duration,
    pending: HashMap<SourceRef, Instant>,
}

impl Debouncer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: HashMap::new(),
        }
    }

    pub fn touch(&mut self, src: SourceRef, now: Instant) {
        self.pending.insert(src, now + self.window);
    }

    /// The earliest pending deadline (what the loop sleeps until).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// Remove and return every source whose deadline is at or before `now`.
    pub fn take_due(&mut self, now: Instant) -> Vec<SourceRef> {
        let due: Vec<SourceRef> = self
            .pending
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(src, _)| src.clone())
            .collect();
        for src in &due {
            self.pending.remove(src);
        }
        due
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// The live sync loop (runs until the bus closes). `import::startup` calls it
/// after the boot catch-up pass, with a receiver taken BEFORE that pass so
/// nothing saved meanwhile is missed.
pub async fn run_live(svc: DesignService, mut rx: broadcast::Receiver<Event>, window: Duration) {
    let mut debounce = Debouncer::new(window);
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    if let Some(src) = source_of(&ev) {
                        debounce.touch(src, Instant::now());
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "design live sync lagged; running the catch-up import");
                    if let Err(e) = import::run(&svc).await {
                        tracing::warn!("design: catch-up import failed: {e}");
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = sleep_until(debounce.next_deadline()) => {
                for src in debounce.take_due(Instant::now()) {
                    match import::sync_one(&svc, &src).await {
                        Ok(r) => tracing::debug!(
                            kind = src.kind(),
                            id = src.id(),
                            created = r.created,
                            synced = r.synced,
                            side_versions = r.side_versions,
                            archived = r.archived,
                            "design: live sync"
                        ),
                        Err(e) => tracing::warn!(
                            kind = src.kind(),
                            id = src.id(),
                            "design: live sync failed: {e}"
                        ),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_name_their_source() {
        let scene = Event::CanvasUpdated {
            workspace_id: "w1".into(),
            scene_id: "sc1".into(),
            doc: json!({}),
        };
        assert_eq!(source_of(&scene), Some(SourceRef::Scene("sc1".into())));
        let mockup = Event::MockupUpdated {
            workspace_id: "w1".into(),
            story_id: "st1".into(),
            attachment_id: "att1".into(),
            format: "html".into(),
            content: None,
        };
        assert_eq!(
            source_of(&mockup),
            Some(SourceRef::Attachment("att1".into()))
        );
        let changed = Event::DesignSourceChanged {
            workspace_id: "w1".into(),
            kind: SOURCE_ATTACHMENT.into(),
            id: "att2".into(),
        };
        assert_eq!(
            source_of(&changed),
            Some(SourceRef::Attachment("att2".into()))
        );
        let unknown = Event::DesignSourceChanged {
            workspace_id: "w1".into(),
            kind: "nope".into(),
            id: "x".into(),
        };
        assert_eq!(source_of(&unknown), None);
        // The graph's own events never loop back into a sync.
        let own = Event::DesignArtifactUpdated {
            workspace_id: "w1".into(),
            artifact_id: "A".into(),
            format: "html".into(),
            change: "content".into(),
            version_id: None,
            content: None,
        };
        assert_eq!(source_of(&own), None);
    }

    #[test]
    fn debounce_coalesces_a_burst_into_one_sync() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut d = Debouncer::new(DEBOUNCE);
        let scene = SourceRef::Scene("sc1".into());
        let att = SourceRef::Attachment("att1".into());
        // An agent's mid-turn writes: three touches 400 ms apart.
        d.touch(scene.clone(), t0);
        d.touch(scene.clone(), t0 + ms(400));
        d.touch(att.clone(), t0 + ms(500));
        d.touch(scene.clone(), t0 + ms(800));
        assert_eq!(d.next_deadline(), Some(t0 + ms(500) + DEBOUNCE));
        // 1.5 s after the FIRST touch nothing is due yet (the burst goes on).
        assert!(d.take_due(t0 + DEBOUNCE).is_empty());
        // The attachment is quiet first…
        assert_eq!(d.take_due(t0 + ms(2_000)), vec![att]);
        // …then the scene, exactly once for the whole burst.
        assert_eq!(d.take_due(t0 + ms(2_300)), vec![scene]);
        assert!(d.is_empty());
        assert!(d.take_due(t0 + ms(10_000)).is_empty());
        assert_eq!(d.next_deadline(), None);
    }

    #[tokio::test]
    async fn live_loop_mirrors_a_scene_after_its_events() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        let (tx, _keep) = broadcast::channel(64);
        let svc = DesignService::new(pool.clone(), dir.path(), Some(tx.clone()));
        assert!(svc.store().ensure_fts().await);
        let repo = otto_state::CanvasRepo::new(pool);
        let scene = repo
            .create(otto_state::NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Live".into(),
                doc_json: json!({"type":"otto-canvas","version":1,"format":"mermaid",
                    "source":"flowchart TD\n A --> B\n"})
                .to_string(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        let rx = svc.subscribe().expect("a bus");
        let task = tokio::spawn(run_live(svc.clone(), rx, Duration::from_millis(50)));
        for _ in 0..3 {
            tx.send(Event::DesignSourceChanged {
                workspace_id: "w1".into(),
                kind: SOURCE_SCENE.into(),
                id: scene.id.clone(),
            })
            .unwrap();
        }
        // `create_artifact` inserts the row, THEN commits v1: wait for the
        // committed head, not just the row (polling the row raced the commit).
        let mut mirror = None;
        for _ in 0..200 {
            mirror = svc
                .store()
                .find_by_source(SOURCE_SCENE, &scene.id)
                .await
                .unwrap()
                .filter(|a| a.head_version_id.is_some());
            if mirror.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        // Well past the window: the burst must not have queued more syncs
        // (a re-sync of an unchanged scene would write nothing anyway).
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mirror = mirror.expect("the scene is mirrored live");
        assert_eq!(mirror.title, "Live");
        let versions = svc
            .store()
            .list_versions(&mirror.id, None, 0, 0)
            .await
            .unwrap();
        assert_eq!(versions.len(), 1, "one burst, one mirror version");
        task.abort();
    }
}

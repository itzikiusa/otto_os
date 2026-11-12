//! LiveEvolver — the in-loop skill evolver.
//!
//! Subscribes to the daemon event bus. When a *watched* agent session goes
//! `Idle` (the existing idle detector) it arms a debounce; if the session stays
//! idle (the interaction concluded) it runs a single-session `evolve_session`
//! pass. Re-armed when the session next goes `Working`. A session is watched
//! only while its workspace has self-improvement ENABLED, and then when the
//! workspace has `self_improvement.live_evolve == true` or the session itself
//! carries `meta.evolve == true`. Engine-owned background sessions (channel
//! tickets, workflow steps, reviewers, scheduled tasks, personal agents…) are
//! never watched: nobody interacts with them, and on 2026-11-12 they kept
//! starting Claude turns (31 runs in 5 hours) after the owner had switched
//! self-improvement off — `enabled` was never checked here.
//!
//! A fire with no new evidence since the last successful analysis (the
//! common case: an idle blip with no new user turn) writes no run row at all
//! ([`has_new_evidence`]); before, each one left a "no new evidence" row (108
//! in one day).
//!
//! Best-effort throughout: all errors are logged, never propagated.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::api::SelfImprovementConfig;
use otto_core::domain::{Session, SessionKind, SessionStatus};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::improvements::is_run_in_progress;
use otto_state::{ImprovementsRepo, SessionsRepo, WorkspacesRepo};
use tokio::sync::{broadcast, Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::config::effective_config;
use crate::engine::ImprovementEngine;

/// How long a watched session must stay idle before we evolve.
const IDLE_DEBOUNCE: Duration = Duration::from_secs(30);

/// Handle; dropping it stops the evolver (mirrors ChannelHandle/SchedulerHandle).
pub struct LiveEvolverHandle {
    cancel: Arc<AtomicBool>,
    /// Wakes the event loop on shutdown (it otherwise parks on the bus with no
    /// timer — the old 1 s poll existed only to notice `cancel`).
    wake: Arc<Notify>,
    _task: JoinHandle<()>,
}
impl LiveEvolverHandle {
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }
}
impl Drop for LiveEvolverHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
struct EpisodeState {
    /// Bumped on every Working/Idle transition; a debounced fire only runs if
    /// the generation it captured is still current (no transition since).
    generation: u64,
    in_flight: bool,
}

type Episodes = Arc<Mutex<HashMap<Id, EpisodeState>>>;

pub struct LiveEvolver {
    engine: Arc<ImprovementEngine>,
    workspaces: WorkspacesRepo,
    sessions: SessionsRepo,
}

impl LiveEvolver {
    pub fn new(
        engine: Arc<ImprovementEngine>,
        workspaces: WorkspacesRepo,
        sessions: SessionsRepo,
    ) -> Self {
        Self {
            engine,
            workspaces,
            sessions,
        }
    }

    pub fn start(self, events: broadcast::Receiver<Event>) -> LiveEvolverHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(self.run(events, Arc::clone(&cancel), Arc::clone(&wake)));
        LiveEvolverHandle {
            cancel,
            wake,
            _task: task,
        }
    }

    async fn run(
        self,
        mut events: broadcast::Receiver<Event>,
        cancel: Arc<AtomicBool>,
        wake: Arc<Notify>,
    ) {
        let episodes: Episodes = Arc::new(Mutex::new(HashMap::new()));
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let received = tokio::select! {
                event = events.recv() => event,
                _ = wake.notified() => continue, // re-checks `cancel` above
            };
            let evt = match received {
                Ok(e) => e,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let Event::SessionStatus {
                session_id,
                workspace_id,
                status,
            } = evt
            else {
                continue;
            };

            match status {
                // Activity (re)started — invalidate any pending fire.
                SessionStatus::Working | SessionStatus::Running => {
                    let mut map = episodes.lock().await;
                    map.entry(session_id).or_default().generation += 1;
                }
                // Interaction paused — arm a debounced evolve if watched.
                SessionStatus::Idle => {
                    if !self.is_watched(&workspace_id, &session_id).await {
                        continue;
                    }
                    let gen = {
                        let mut map = episodes.lock().await;
                        let e = map.entry(session_id.clone()).or_default();
                        e.generation += 1;
                        e.generation
                    };
                    self.arm_fire(session_id, gen, Arc::clone(&episodes), Arc::clone(&cancel));
                }
                // Exited / Reconnectable — drop tracking.
                _ => {
                    episodes.lock().await.remove(&session_id);
                }
            }
        }
    }

    /// Spawn a debounced task that evolves `session_id` if it is still idle
    /// (same `gen`) and the transcript has grown since the last evolve.
    fn arm_fire(&self, session_id: Id, mut gen: u64, episodes: Episodes, cancel: Arc<AtomicBool>) {
        let engine = Arc::clone(&self.engine);
        let sessions = self.sessions.clone();
        let workspaces = self.workspaces.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(IDLE_DEBOUNCE).await;
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                // Recheck lifecycle and opt-in after the debounce, since either
                // may have changed while the timer was armed.
                let Ok(session) = sessions.get(&session_id).await else {
                    return;
                };
                if session.status != SessionStatus::Idle || !watched(&workspaces, &session).await {
                    return;
                }
                {
                    let mut map = episodes.lock().await;
                    let Some(e) = map.get_mut(&session_id) else {
                        return;
                    };
                    if e.generation != gen || e.in_flight {
                        return;
                    }
                    e.in_flight = true;
                }
                // Peek for new evidence BEFORE `evolve_session` writes a run
                // row: an idle blip with nothing new costs no row and no turn.
                let fresh = has_new_evidence(
                    &engine.sessions,
                    &engine.improvements,
                    engine.library_root.parent().unwrap_or(Path::new(".")),
                    &session,
                    lookback_hours(&workspaces, &session).await,
                )
                .await;
                let outcome = match fresh {
                    Ok(true) => Some(engine.evolve_session(&session_id).await),
                    Ok(false) => None,
                    Err(e) => Some(Err(e)),
                };
                match outcome {
                    None => debug!(session = %session_id, "live evolve skipped: no new evidence"),
                    Some(Ok(_)) => {}
                    // Another run (scheduled, manual, another session) holds
                    // the workspace: the normal outcome, answered from the
                    // read pool without touching the write lock.
                    Some(Err(e)) if is_run_in_progress(&e) => {
                        debug!(session = %session_id, "live evolve skipped: {e}");
                    }
                    Some(Err(e)) => warn!(session = %session_id, "live evolve failed: {e}"),
                }
                let mut map = episodes.lock().await;
                let Some(e) = map.get_mut(&session_id) else {
                    return;
                };
                e.in_flight = false;
                // If another interaction completed during analysis, give its
                // newest delta its own debounce instead of losing the wakeup.
                if e.generation == gen {
                    return;
                }
                gen = e.generation;
            }
        });
    }

    async fn is_watched(&self, _workspace_id: &Id, session_id: &Id) -> bool {
        match self.sessions.get(session_id).await {
            Ok(session) => watched(&self.workspaces, &session).await,
            Err(_) => false,
        }
    }
}

async fn watched(workspaces: &WorkspacesRepo, session: &Session) -> bool {
    // Cheap, settings-free refusals first: most idle events are background.
    if !watchable(session) {
        return false;
    }
    match workspaces.get(&session.workspace_id).await {
        Ok(ws) => watch_decision(session, &effective_config(&ws.settings)),
        Err(_) => false,
    }
}

/// A session the live evolver may ever look at: a live, foreground agent the
/// user drives. Background sessions (`BACKGROUND_SESSION_SOURCES`, e.g.
/// `channel`, `workflow`, `review`, `scheduled_task`) and the gated
/// unattended ones (`personal_agent`) are out — no person interacts with them.
fn watchable(session: &Session) -> bool {
    !session.archived
        && session.kind == SessionKind::Agent
        && session.provider != "shell"
        && session.is_foreground_agent()
        && !otto_sessions::admission::is_gated_session(session)
}

/// Whether `session` is watched under its workspace's `cfg`. `enabled` is the
/// master switch: neither `live_evolve` nor a session's own `meta.evolve`
/// opt-in starts a turn while self-improvement is off.
fn watch_decision(session: &Session, cfg: &SelfImprovementConfig) -> bool {
    if !watchable(session) || !cfg.enabled {
        return false;
    }
    cfg.live_evolve || session.meta.get("evolve").and_then(|v| v.as_bool()) == Some(true)
}

/// The workspace's evidence look-back (the engine reads the same setting).
async fn lookback_hours(workspaces: &WorkspacesRepo, session: &Session) -> u32 {
    workspaces
        .get(&session.workspace_id)
        .await
        .map(|ws| effective_config(&ws.settings).lookback_hours)
        .unwrap_or_else(|_| SelfImprovementConfig::default().lookback_hours)
}

/// Whether `session` has evidence the last successful analysis has not seen.
/// Leases the evidence stream, reads the same delta the engine would, and
/// releases the lease WITHOUT advancing the cursor, so the engine's own pass
/// re-reads exactly this delta. A stream another pass holds counts as nothing
/// new (that pass consumes it).
async fn has_new_evidence(
    sessions: &SessionsRepo,
    improvements: &ImprovementsRepo,
    data_dir: &Path,
    session: &Session,
    lookback_hours: u32,
) -> otto_core::Result<bool> {
    let source = format!("session:{}", session.id);
    let Some(claim) = improvements.claim_evidence(&source).await? else {
        return Ok(false);
    };
    let batch = crate::evidence::collect(
        session,
        sessions,
        improvements,
        data_dir,
        &claim.checkpoint,
        lookback_hours,
    )
    .await;
    improvements
        .finish_evidence(&source, &claim.token, None)
        .await?;
    Ok(batch?.digest.is_some())
}

#[cfg(test)]
mod tests {
    use otto_core::domain::ImprovementTrigger;

    /// The busy answer the live trigger logs quietly is exactly the one
    /// `create_run` returns; any other conflict still warns.
    #[test]
    fn busy_workspace_is_recognised_as_the_quiet_outcome() {
        use otto_core::Error;
        use otto_state::improvements::is_run_in_progress;
        assert!(is_run_in_progress(&Error::Conflict(
            "an improvement run is already in progress".into()
        )));
        assert!(!is_run_in_progress(&Error::Conflict("edit changed".into())));
        assert!(!is_run_in_progress(&Error::Internal(
            "an improvement run is already in progress".into()
        )));
    }

    fn session(meta: serde_json::Value) -> super::Session {
        super::Session {
            id: "s1".into(),
            workspace_id: "w1".into(),
            kind: super::SessionKind::Agent,
            provider: "claude".into(),
            title: "t".into(),
            status: super::SessionStatus::Idle,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta,
        }
    }

    fn cfg(enabled: bool, live_evolve: bool) -> super::SelfImprovementConfig {
        super::SelfImprovementConfig {
            enabled,
            live_evolve,
            ..Default::default()
        }
    }

    /// `enabled` is the master switch: with it off neither the workspace's
    /// `live_evolve` nor a session's own `meta.evolve` starts a turn.
    #[test]
    fn live_evolve_requires_self_improvement_enabled() {
        use super::watch_decision;
        let plain = session(serde_json::json!({}));
        let opted = session(serde_json::json!({ "evolve": true }));
        assert!(!watch_decision(&plain, &cfg(false, true)));
        assert!(!watch_decision(&opted, &cfg(false, false)));
        assert!(!watch_decision(&opted, &cfg(false, true)));
        assert!(watch_decision(&plain, &cfg(true, true)));
        assert!(!watch_decision(&plain, &cfg(true, false)));
        assert!(watch_decision(&opted, &cfg(true, false)));
    }

    /// Engine-owned background sessions are never analysed, whatever the
    /// workspace or the session meta says.
    #[test]
    fn background_sessions_are_never_watched() {
        use super::watch_decision;
        for src in [
            "channel",
            "workflow",
            "review",
            "scheduled_task",
            "personal_agent",
        ] {
            let s = session(serde_json::json!({ "source": src, "evolve": true }));
            assert!(!watch_decision(&s, &cfg(true, true)), "{src}");
        }
        let mut archived = session(serde_json::json!({}));
        archived.archived = true;
        assert!(!watch_decision(&archived, &cfg(true, true)));
        let mut shell = session(serde_json::json!({}));
        shell.provider = "shell".into();
        assert!(!watch_decision(&shell, &cfg(true, true)));
    }

    /// The pre-check reports nothing new for a session with no transcript and
    /// no trail, sees a new user prompt, and never advances or keeps the lease
    /// (the engine's own pass must re-read the same delta).
    #[tokio::test]
    async fn new_evidence_is_peeked_without_consuming_it() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        let uid = otto_state::UsersRepo::new(pool.clone())
            .create("root", "pw", "root", true)
            .await
            .unwrap()
            .id;
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .create("t", dir.path().to_str().unwrap(), &uid)
            .await
            .unwrap();
        let sessions = otto_state::SessionsRepo::new(pool.clone());
        let improvements = otto_state::ImprovementsRepo::new(pool.clone());
        let s = sessions
            .create(otto_state::NewSession {
                workspace_id: ws.id.clone(),
                kind: super::SessionKind::Agent,
                // agy: trail-only evidence, never a transcript lookup.
                provider: "agy".into(),
                title: "live fixture".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
                provider_session_id: None,
                connection_id: None,
                created_by: uid,
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        let peek = || super::has_new_evidence(&sessions, &improvements, dir.path(), &s, 24);
        assert!(!peek().await.unwrap(), "nothing new yet");
        otto_state::ActivityRepo::new(pool.clone())
            .append_trail(otto_state::NewTrail {
                session_id: s.id.clone(),
                workspace_id: ws.id.clone(),
                source: otto_core::domain::TrailSource::User,
                kind: otto_core::domain::TrailKind::Prompt,
                level: otto_core::domain::TrailLevel::Info,
                summary: "Correction: keep the tests next to the code".into(),
                detail: None,
            })
            .await
            .unwrap();
        assert!(peek().await.unwrap());
        // Not consumed: a second peek still sees it, and the lease is free.
        assert!(peek().await.unwrap());
        let claim = improvements
            .claim_evidence(&format!("session:{}", s.id))
            .await
            .unwrap();
        assert!(claim.is_some(), "the peek released its lease");
    }

    #[test]
    fn live_trigger_round_trips() {
        assert_eq!(
            ImprovementTrigger::parse("live"),
            Some(ImprovementTrigger::Live)
        );
        assert_eq!(ImprovementTrigger::Live.as_str(), "live");
    }
}

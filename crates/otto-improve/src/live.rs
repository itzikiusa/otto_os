//! LiveEvolver — the in-loop skill evolver.
//!
//! Subscribes to the daemon event bus. When a *watched* agent session goes
//! `Idle` (the existing idle detector) it arms a debounce; if the session stays
//! idle (the interaction concluded) it runs a single-session `evolve_session`
//! pass. Re-armed when the session next goes `Working`. A session is watched
//! when its workspace has `self_improvement.live_evolve == true`, or the session
//! itself carries `meta.evolve == true`.
//!
//! Best-effort throughout: all errors are logged, never propagated.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::{SessionKind, SessionStatus};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{SessionsRepo, WorkspacesRepo};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::config::effective_config;
use crate::engine::ImprovementEngine;

/// How long a watched session must stay idle before we evolve.
const IDLE_DEBOUNCE: Duration = Duration::from_secs(30);

/// Handle; dropping it stops the evolver (mirrors ChannelHandle/SchedulerHandle).
pub struct LiveEvolverHandle {
    cancel: Arc<AtomicBool>,
    _task: JoinHandle<()>,
}
impl LiveEvolverHandle {
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for LiveEvolverHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        let task = tokio::spawn(self.run(events, Arc::clone(&cancel)));
        LiveEvolverHandle {
            cancel,
            _task: task,
        }
    }

    async fn run(self, mut events: broadcast::Receiver<Event>, cancel: Arc<AtomicBool>) {
        let episodes: Episodes = Arc::new(Mutex::new(HashMap::new()));
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let received = tokio::select! {
                event = events.recv() => event,
                _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
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
                if let Err(e) = engine.evolve_session(&session_id).await {
                    warn!(session = %session_id, "live evolve failed: {e}");
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

async fn watched(workspaces: &WorkspacesRepo, session: &otto_core::domain::Session) -> bool {
    if session.archived || session.kind != SessionKind::Agent || session.provider == "shell" {
        return false;
    }
    if session.meta.get("evolve").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    workspaces
        .get(&session.workspace_id)
        .await
        .is_ok_and(|ws| effective_config(&ws.settings).live_evolve)
}

#[cfg(test)]
mod tests {
    use otto_core::domain::ImprovementTrigger;

    #[test]
    fn live_trigger_round_trips() {
        assert_eq!(
            ImprovementTrigger::parse("live"),
            Some(ImprovementTrigger::Live)
        );
        assert_eq!(ImprovementTrigger::Live.as_str(), "live");
    }
}

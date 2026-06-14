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

use otto_core::domain::SessionStatus;
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{SessionsRepo, WorkspacesRepo};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::config::effective_config;
use crate::digest::build_digest;
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
    /// Turn count at the last evolve — skip if the transcript hasn't grown.
    last_turns: usize,
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
            let evt = match events.recv().await {
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
                    self.arm_fire(session_id, gen, Arc::clone(&episodes));
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
    fn arm_fire(&self, session_id: Id, gen: u64, episodes: Episodes) {
        let engine = Arc::clone(&self.engine);
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_DEBOUNCE).await;

            // Claim the fire: still current generation, not already running.
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

            // Skip if the conversation hasn't grown since the last evolve.
            let turns = match sessions.get(&session_id).await {
                Ok(s) => build_digest(&s).map(|d| d.turns).unwrap_or(0),
                Err(_) => 0,
            };
            let grown = {
                let map = episodes.lock().await;
                map.get(&session_id).map(|e| turns > e.last_turns).unwrap_or(false)
            };

            if grown {
                if let Err(e) = engine.evolve_session(&session_id).await {
                    warn!(session = %session_id, "live evolve failed: {e}");
                }
            }

            // Release; record the turn count we acted on.
            let mut map = episodes.lock().await;
            if let Some(e) = map.get_mut(&session_id) {
                e.in_flight = false;
                if grown {
                    e.last_turns = turns;
                }
            }
        });
    }

    /// Watched iff the workspace opted in (`live_evolve`) or the session did
    /// (`meta.evolve == true`), and it is a non-archived agent session.
    async fn is_watched(&self, workspace_id: &Id, session_id: &Id) -> bool {
        if let Ok(ws) = self.workspaces.get(workspace_id).await {
            if effective_config(&ws.settings).live_evolve {
                return true;
            }
        }
        if let Ok(s) = self.sessions.get(session_id).await {
            if s.archived {
                return false;
            }
            if s.meta.get("evolve").and_then(|v| v.as_bool()).unwrap_or(false) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use otto_core::domain::ImprovementTrigger;

    #[test]
    fn live_trigger_round_trips() {
        assert_eq!(ImprovementTrigger::parse("live"), Some(ImprovementTrigger::Live));
        assert_eq!(ImprovementTrigger::Live.as_str(), "live");
    }
}

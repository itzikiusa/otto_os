//! Scheduler — a background supervisor that fires due self-reflection runs.
//!
//! Mirrors otto-channels' ChannelManager: `start()` spawns a supervisor task
//! that every ~60s lists workspaces, reads each one's config, and spawns a run
//! for any that are due and not already running. An in-memory in-flight set
//! plus a DB `has_running` check guarantee one run per workspace at a time.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_core::domain::ImprovementTrigger;
use otto_state::WorkspacesRepo;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{effective_config, is_due};
use crate::engine::ImprovementEngine;

const SCAN_INTERVAL: Duration = Duration::from_secs(60);

pub struct SchedulerHandle {
    cancel: Arc<AtomicBool>,
    /// Wakes the sleeping supervisor on shutdown (`notify_one` keeps a permit,
    /// so a shutdown racing the flag check is never lost).
    wake: Arc<Notify>,
    _supervisor: JoinHandle<()>,
}
impl SchedulerHandle {
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }
}
impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct Scheduler {
    engine: Arc<ImprovementEngine>,
    workspaces: WorkspacesRepo,
}

impl Scheduler {
    pub fn new(engine: Arc<ImprovementEngine>, workspaces: WorkspacesRepo) -> Self {
        Self { engine, workspaces }
    }

    pub async fn start(self) -> SchedulerHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let supervisor = tokio::spawn(self.supervise(Arc::clone(&cancel), Arc::clone(&wake)));
        SchedulerHandle {
            cancel,
            wake,
            _supervisor: supervisor,
        }
    }

    async fn supervise(self, cancel: Arc<AtomicBool>, wake: Arc<Notify>) {
        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            // User-facing list: a scheduled improvement run must never scan the
            // scratch workspace (its root is the daemon `$HOME`).
            match self.workspaces.list_user_all().await {
                Ok(list) => {
                    let now = Utc::now();
                    for ws in list {
                        if ws.archived {
                            continue;
                        }
                        let cfg = effective_config(&ws.settings);
                        if !is_due(&cfg, now) {
                            continue;
                        }
                        // Skip if already running (in-memory or DB).
                        {
                            let guard = in_flight.lock().await;
                            if guard.contains(&ws.id) {
                                continue;
                            }
                        }
                        if self
                            .engine
                            .improvements
                            .has_running(&ws.id)
                            .await
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        in_flight.lock().await.insert(ws.id.clone());
                        let engine = Arc::clone(&self.engine);
                        let flight = Arc::clone(&in_flight);
                        let ws_id = ws.id.clone();
                        info!(workspace = %ws_id, "self-improvement: starting scheduled run");
                        tokio::spawn(async move {
                            if let Err(e) = engine
                                .run_for_workspace(&ws_id, ImprovementTrigger::Scheduled)
                                .await
                            {
                                warn!(workspace = %ws_id, "self-improvement run failed: {e}");
                            }
                            flight.lock().await.remove(&ws_id);
                        });
                    }
                }
                Err(e) => warn!("self-improvement scheduler: list workspaces: {e}"),
            }

            // One timer per scan; shutdown wakes it (was 500 ms slices).
            tokio::select! {
                _ = tokio::time::sleep(SCAN_INTERVAL) => {}
                _ = wake.notified() => {}
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
        }
    }
}

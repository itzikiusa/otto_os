//! Machine resources for agent sessions — the manager's side of
//! [`crate::admission`] (admission gate, live ceiling, low priority) and
//! [`crate::build_budget`] (parallelism hints in the agent's env).
//!
//! Where it hooks in:
//! - `create` waits in [`SessionManager::admit_create`] BEFORE it takes any
//!   lock or writes the row, so a queued agent holds nothing and a caller
//!   that drops the future just leaves the queue;
//! - every PTY spawn (create, restart / resume) goes through
//!   `spawn_session_pty`, which adds the build budget
//!   ([`SessionManager::with_machine_budget`]) and, for local spawns,
//!   [`SessionManager::spawn_local_pty`] lowers a gated agent's priority and
//!   registers the process with the gate until it exits;
//! - the structured runtime (`prepare_runtime_spawn`) gets the build budget.

use super::*;

use crate::admission::{self, AdmissionPermit, AdmissionPolicy, Poll};
use crate::build_budget;
use crate::machine_load::{logical_cores, MachineMonitor, Sampler};

/// Settings are re-read on every gate check; this bounds the DB reads a
/// queue of waiters causes (a write through the repo is seen at once).
const SETTINGS_TTL: Duration = Duration::from_secs(5);

impl SessionManager {
    /// Replace the OS machine sampler (tests). Builder-style.
    pub fn with_machine_sampler(mut self, sampler: Arc<dyn Sampler>) -> Self {
        self.admission = Arc::new(admission::AdmissionGate::new(MachineMonitor::new(sampler)));
        self
    }

    async fn resource_setting(&self, key: &str) -> Option<serde_json::Value> {
        self.settings
            .as_ref()?
            .get_cached(key, SETTINGS_TTL)
            .await
            .ok()
            .flatten()
    }

    /// The gate's current thresholds. Disabled without a settings store:
    /// the gate is armed only in the daemon, never in a bare harness.
    pub async fn admission_policy(&self) -> AdmissionPolicy {
        let cores = logical_cores();
        if self.settings.is_none() {
            return AdmissionPolicy::disabled(cores);
        }
        AdmissionPolicy::from_settings(
            cores,
            self.resource_setting(admission::ADMISSION_ENABLED_SETTING)
                .await
                .as_ref(),
            self.resource_setting(admission::BACKGROUND_MAX_LIVE_SETTING)
                .await
                .as_ref(),
            self.resource_setting(admission::MAX_LOAD_PER_CORE_SETTING)
                .await
                .as_ref(),
            self.resource_setting(admission::MAX_WAIT_SETTING)
                .await
                .as_ref(),
        )
    }

    /// Machine load and the admission queue, for the UI.
    pub async fn machine_status(&self) -> admission::MachineStatus {
        let policy = self.admission_policy().await;
        let sample = self.admission.monitor().current().await;
        self.admission
            .status(&sample, &policy, std::time::Instant::now())
    }

    /// Wait for a place for a background agent of `source` BEFORE starting a
    /// clock of your own (a summarizer's hard budget, an assist turn cap),
    /// then [`AdmissionPermit::attach`] / [`AdmissionPermit::stamp`] it onto
    /// the create request's meta: `create` takes the place over, so the
    /// permit may then be kept or dropped freely. Returns at once (an empty permit) for a source
    /// the gate does not hold back or while the gate is off. Cancellable:
    /// dropping the future leaves the queue.
    pub async fn admit_background(&self, source: &str) -> AdmissionPermit {
        if !admission::is_gated_source(source) {
            return AdmissionPermit::none();
        }
        self.wait_for_admission(source).await
    }

    /// `create`'s gate: strips a pre-acquired permit's key from `req.meta`
    /// (never persisted) and takes its place over; without one, a gated
    /// background agent waits for its turn. The returned permit must live
    /// until the process has spawned.
    pub(super) async fn admit_create(
        &self,
        mut req: CreateSessionReq,
    ) -> (CreateSessionReq, AdmissionPermit) {
        let key = req
            .meta
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|m| m.remove(admission::PERMIT_META_KEY));
        let claimed = key
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .and_then(|k| self.admission.claim(k));
        if let Some((ticket, key)) = claimed {
            let gate = Arc::clone(&self.admission);
            return (req, AdmissionPermit::reserved(gate, ticket, key));
        }
        let source = admission::gated_source(req.kind, req.meta.as_ref()).map(str::to_string);
        let Some(source) = source else {
            return (req, AdmissionPermit::none());
        };
        let permit = self.wait_for_admission(&source).await;
        (req, permit)
    }

    /// Queue, then poll until admitted (see [`crate::admission`]).
    async fn wait_for_admission(&self, source: &str) -> AdmissionPermit {
        let mut policy = self.admission_policy().await;
        if !policy.enabled {
            return AdmissionPermit::none();
        }
        let gate = Arc::clone(&self.admission);
        let ticket = gate.enqueue(source, std::time::Instant::now());
        let mut queued = admission::QueuedTicket::new(&gate, ticket);
        let mut announced = false;
        loop {
            let sample = gate.monitor().current().await;
            // Before the poll: a release in between still wakes us.
            let changed = gate.changed();
            let now = std::time::Instant::now();
            match gate.poll(ticket, &sample, &policy, now) {
                Poll::Admitted(a) => {
                    queued.disarm();
                    if let Some(reason) = a.overdue {
                        tracing::warn!(
                            source,
                            waited_secs = a.waited.as_secs(),
                            reason = %reason.describe(),
                            "admission: background agent waited the maximum — starting it anyway"
                        );
                    } else if a.queued {
                        tracing::info!(
                            source,
                            waited_secs = a.waited.as_secs(),
                            "admission: background agent admitted"
                        );
                    }
                    return AdmissionPermit::reserved(Arc::clone(&gate), ticket, a.key);
                }
                Poll::Gone => return AdmissionPermit::none(),
                Poll::Wait(reason) => {
                    if !announced && !matches!(reason, admission::WaitReason::Spacing) {
                        announced = true;
                        self.announce_queued(source, &reason, now);
                    }
                }
            }
            tokio::select! {
                _ = changed => {}
                _ = tokio::time::sleep(admission::POLL_INTERVAL) => {}
            }
            policy = self.admission_policy().await;
            if !policy.enabled {
                // Switched off while queued: start now (`queued` leaves).
                return AdmissionPermit::none();
            }
        }
    }

    /// A background agent just queued: log it, and raise one "machine busy"
    /// notice per [`admission::BUSY_NOTICE_EVERY`].
    fn announce_queued(
        &self,
        source: &str,
        reason: &admission::WaitReason,
        now: std::time::Instant,
    ) {
        tracing::info!(
            source,
            reason = %reason.describe(),
            "admission: background agent queued — machine busy"
        );
        if matches!(reason, admission::WaitReason::Queue) || !self.admission.take_busy_notice(now) {
            return;
        }
        let _ = self.events.send(Event::Notice {
            level: "info".into(),
            title: "Background agents queued".into(),
            body: format!(
                "The machine is busy ({}). Otto starts reviews, workflow steps, scheduled \
                 tasks and other background agents as it frees up. Your own sessions are \
                 not delayed.",
                reason.describe()
            ),
        });
    }

    /// `spec` with the build-budget env added (agent sessions only; see
    /// [`crate::build_budget`]).
    pub(super) async fn with_machine_budget(
        &self,
        session: &Session,
        mut spec: CommandSpec,
    ) -> CommandSpec {
        self.apply_machine_budget(session, &mut spec).await;
        spec
    }

    /// [`Self::with_machine_budget`] in place. A variable already in
    /// `spec.env` or in the daemon's own environment is left alone. Agent
    /// CLIs only ([`counts_toward_cap`]): a plain shell tab is the user typing
    /// their own commands, and a connection terminal runs no builds.
    pub(super) async fn apply_machine_budget(&self, session: &Session, spec: &mut CommandSpec) {
        if !counts_toward_cap(session) {
            return;
        }
        let mode = build_budget::budget_mode(
            self.resource_setting(build_budget::BUILD_PARALLELISM_SETTING)
                .await
                .as_ref(),
        );
        let live = self.admission.live_agents_except(&session.id) + 1;
        let Some(n) = build_budget::budget_for(mode, logical_cores(), live) else {
            return;
        };
        let add = build_budget::budget_env(n, |key| {
            spec.env
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .or_else(|| std::env::var(key).ok())
        });
        spec.env.extend(add);
    }

    /// The in-process PTY spawn (fork/exec on the blocking pool). A gated
    /// background agent then drops to [`admission::BACKGROUND_NICE`] — before
    /// its CLI has had time to start any tool, so its builds inherit it —
    /// unless `agent_background_low_priority` is off.
    pub(super) async fn spawn_local_pty(
        &self,
        session: &Session,
        spec: CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<PtyHandle> {
        let handle = tokio::task::spawn_blocking(move || PtyHandle::spawn_sized(&spec, cols, rows))
            .await
            .unwrap_or_else(|e| Err(Error::Internal(format!("pty spawn task: {e}"))))?;
        if admission::is_gated_session(session) {
            let low = admission::low_priority_enabled(
                self.resource_setting(admission::LOW_PRIORITY_SETTING)
                    .await
                    .as_ref(),
            );
            if let (true, Some(pid)) = (low, handle.pid()) {
                if let Err(e) = admission::lower_priority(pid) {
                    tracing::debug!(session = %session.id, pid, "lower priority: {e}");
                }
            }
        }
        Ok(self.track_spawn(session, handle))
    }

    /// Count an agent's fresh process with the gate until it exits (its exit
    /// watch, so no teardown path has to remember to unregister it).
    pub(super) fn track_spawn(&self, session: &Session, handle: PtyHandle) -> PtyHandle {
        if counts_toward_cap(session) {
            self.admission.register(
                &session.id,
                admission::is_gated_session(session),
                handle.on_exit(),
            );
        }
        handle
    }

    /// The idle sweep left the live-session cap exceeded: every session over
    /// it is in use, so nothing was suspended (the cap only reclaims quiet
    /// sessions). Logged each sweep; once it has lasted
    /// [`admission::CAP_NOTICE_AFTER`], one warning notice.
    pub(super) fn note_live_cap(&self, cap: usize, over: usize) {
        if over == 0 {
            return;
        }
        tracing::info!(
            cap,
            over,
            "live-session cap: {over} agent session(s) over the cap of {cap} are all in use \
             (watched, working, mid-turn or not resumable), so none was suspended — the cap \
             only reclaims quiet sessions; new background agents are held by the admission gate"
        );
        if !self
            .admission
            .observe_cap_exceeded(std::time::Instant::now())
        {
            return;
        }
        tracing::warn!(
            cap,
            over,
            "live-session cap exceeded for over 5 minutes by busy sessions"
        );
        let _ = self.events.send(Event::Notice {
            level: "warn".into(),
            title: "Many agent sessions running".into(),
            body: format!(
                "{over} agent session(s) above the limit of {cap} have stayed busy for over \
                 5 minutes, so Otto could not free their memory. They keep running; close the \
                 ones you no longer need, or raise the limit in Settings."
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_load::{MachineSample, MemoryPressure};
    use otto_core::new_id;

    struct Fixed(f64);
    impl Sampler for Fixed {
        fn sample(&self) -> MachineSample {
            MachineSample {
                load1: Some(self.0),
                cores: 8,
                pressure: MemoryPressure::Normal,
                swap_used_bytes: None,
                taken: std::time::Instant::now(),
            }
        }
    }

    async fn manager(settings: bool, load: f64) -> SessionManager {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        std::mem::forget(dir);
        let (events, _rx) = broadcast::channel(16);
        let mut m = SessionManager::new(
            SessionsRepo::new(pool.clone()),
            events,
            ProviderRegistry::new(None),
        )
        .with_machine_sampler(Arc::new(Fixed(load)));
        if settings {
            m = m.with_settings_repo(otto_state::SettingsRepo::new(pool));
        }
        m
    }

    fn req(meta: serde_json::Value) -> CreateSessionReq {
        CreateSessionReq {
            kind: SessionKind::Agent,
            provider: Some("claude".into()),
            title: None,
            cwd: None,
            connection_id: None,
            meta: Some(meta),
            model: None,
        }
    }

    fn agent(id: &str, meta: serde_json::Value) -> Session {
        let now = chrono::Utc::now();
        Session {
            id: id.into(),
            workspace_id: "w".into(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "t".into(),
            status: SessionStatus::Running,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "u".into(),
            created_at: now,
            last_active_at: now,
            archived: false,
            meta,
        }
    }

    #[tokio::test]
    async fn a_bare_harness_never_queues() {
        let m = manager(false, 500.0).await;
        assert!(!m.admission_policy().await.enabled);
        let (_, permit) = m
            .admit_create(req(serde_json::json!({ "source": "review" })))
            .await;
        assert!(!permit.is_reserved());
    }

    #[tokio::test]
    async fn a_gated_agent_on_a_calm_machine_is_admitted_at_once() {
        let m = manager(true, 0.5).await;
        let (_, permit) = m
            .admit_create(req(serde_json::json!({ "source": "review" })))
            .await;
        assert!(permit.is_reserved());
        assert_eq!(m.machine_status().await.background_live, 1);
        drop(permit);
        assert_eq!(m.machine_status().await.background_live, 0);
        // The user's own session never takes a place.
        let (_, permit) = m.admit_create(req(serde_json::json!({}))).await;
        assert!(!permit.is_reserved());
    }

    #[tokio::test]
    async fn a_pre_acquired_permit_is_honoured_and_never_persisted() {
        let m = manager(true, 0.5).await;
        let permit = m.admit_background("review_summarizer").await;
        assert!(permit.is_reserved());
        let mut r = req(serde_json::json!({ "source": "review_summarizer" }));
        permit.attach(&mut r);
        let (r, second) = m.admit_create(r).await;
        assert!(second.is_reserved(), "create took the place over");
        assert_eq!(m.machine_status().await.background_live, 1, "counted once");
        let meta = r.meta.unwrap();
        assert!(meta.get(admission::PERMIT_META_KEY).is_none(), "stripped");
        assert_eq!(meta["source"], "review_summarizer");
        // The caller may keep its permit; only create's own one holds the place.
        drop(permit);
        assert_eq!(m.machine_status().await.background_live, 1);
        drop(second);
        assert_eq!(m.machine_status().await.background_live, 0);
        // A forged / stale key is stripped and the agent queues normally.
        let mut r = req(serde_json::json!({ "source": "review" }));
        r.meta.as_mut().unwrap()[admission::PERMIT_META_KEY] = "bogus".into();
        let (r, third) = m.admit_create(r).await;
        assert!(third.is_reserved());
        assert!(r.meta.unwrap().get(admission::PERMIT_META_KEY).is_none());
        // Ungated sources get an empty permit straight away.
        assert!(!m.admit_background("canvas_assist").await.is_reserved());
    }

    #[tokio::test]
    async fn switching_the_gate_off_admits_without_a_place() {
        let m = manager(true, 500.0).await;
        m.settings
            .as_ref()
            .unwrap()
            .put(
                admission::ADMISSION_ENABLED_SETTING,
                &serde_json::json!(false),
            )
            .await
            .unwrap();
        // Load 500 on 8 cores would queue; off, it starts at once.
        let (_, permit) = m
            .admit_create(req(serde_json::json!({ "source": "workflow" })))
            .await;
        assert!(!permit.is_reserved());
    }

    #[tokio::test]
    async fn the_build_budget_lands_in_agent_env_without_clobbering() {
        let m = manager(true, 0.5).await;
        let session = agent(&new_id(), serde_json::json!({}));
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: None,
            env: vec![("CARGO_BUILD_JOBS".into(), "1".into())],
        };
        let spec = m.with_machine_budget(&session, spec).await;
        let get = |k: &str| {
            spec.env
                .iter()
                .rev()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("CARGO_BUILD_JOBS").as_deref(), Some("1"), "kept");
        assert_eq!(
            spec.env
                .iter()
                .filter(|(k, _)| k == "CARGO_BUILD_JOBS")
                .count(),
            1
        );
        if std::env::var_os("RUST_TEST_THREADS").is_none() {
            let n: usize = get("RUST_TEST_THREADS").unwrap().parse().unwrap();
            assert!((2..=logical_cores().max(2)).contains(&n));
        }

        // "off" sets nothing; a connection terminal or a plain shell tab
        // never gets a budget.
        m.settings
            .as_ref()
            .unwrap()
            .put(
                build_budget::BUILD_PARALLELISM_SETTING,
                &serde_json::json!("off"),
            )
            .await
            .unwrap();
        let bare = CommandSpec {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: None,
            env: vec![],
        };
        let off = m.with_machine_budget(&session, bare.clone()).await;
        assert!(off.env.is_empty());
        m.settings
            .as_ref()
            .unwrap()
            .put(
                build_budget::BUILD_PARALLELISM_SETTING,
                &serde_json::json!(3),
            )
            .await
            .unwrap();
        let mut conn = agent(&new_id(), serde_json::json!({}));
        conn.kind = SessionKind::Connection;
        assert!(m
            .with_machine_budget(&conn, bare.clone())
            .await
            .env
            .is_empty());
        let mut shell = agent(&new_id(), serde_json::json!({}));
        shell.provider = "shell".into();
        assert!(m
            .with_machine_budget(&shell, bare.clone())
            .await
            .env
            .is_empty());
        let fixed = m.with_machine_budget(&session, bare).await;
        if std::env::var_os("GOMAXPROCS").is_none() {
            assert!(fixed.env.contains(&("GOMAXPROCS".into(), "3".into())));
        }
    }

    #[tokio::test]
    async fn a_spawned_agent_is_counted_until_it_exits() {
        let m = manager(true, 0.5).await;
        let review = agent("r1", serde_json::json!({ "source": "review" }));
        let mine = agent("m1", serde_json::json!({}));
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            cwd: None,
            env: vec![],
        };
        let h1 = m.track_spawn(&review, PtyHandle::spawn(&spec).unwrap());
        let h2 = m.track_spawn(&mine, PtyHandle::spawn(&spec).unwrap());
        assert_eq!(m.admission.live_counts(), (2, 1));
        assert_eq!(m.admission.live_agents_except(&"r1".to_string()), 1);
        let mut exit = h1.on_exit();
        let _ = h1.kill();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            exit.wait_for(|code| code.is_some()),
        )
        .await;
        assert_eq!(m.admission.live_counts(), (1, 0));
        let _ = h2.kill();
    }
}

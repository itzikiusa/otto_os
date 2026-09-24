//! [`LiveRuntime`] — the daemon-wide owner of remote live sessions: settings,
//! the one-time engine install, the Chromium process pool (shared ephemeral
//! process + one per persistent profile, capped), the SSRF guard proxy, the
//! per-tab session map, and the janitor (idle sessions, empty processes,
//! crashed leftovers). Everything is lazy: nothing starts until a user opens
//! a remote live tab or asks for the engine download.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::cdp::CdpError;

use super::chrome::LaunchSpec;
use super::hooks::{LiveAudit, LiveHooks};
use super::install::{
    self, current_platform, effective_sha256, is_installed, pin_for, resolve_binary, BinarySource,
    DittoExtractor, Extractor, Fetcher, InstallJob, InstallState, ReqwestFetcher,
};
use super::process::{ChromeProcess, ProcessKey};
use super::proxy::GuardProxy;
use super::session::{LiveSession, OpenParams};
use super::types::{
    valid_profile_name, ChromeBuild, LiveSettings, Viewport, MAX_PROCESSES, PROCESS_IDLE_EXIT_SECS,
};

/// How often the janitor sweeps.
const SWEEP_EVERY: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("remote live view is not supported on this platform (mac-arm64 only)")]
    Unsupported,
    #[error("no Chromium build is installed — enable remote live view first")]
    NotInstalled,
    #[error("{0}")]
    Invalid(String),
    #[error("no live session")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("too many live sessions (limit {0})")]
    TooMany(u32),
    #[error("browser engine unavailable: {0}")]
    Engine(String),
    #[error("navigation to {0} blocked (SSRF guard)")]
    Blocked(String),
    #[error("the agent is paused while a person drives this tab")]
    AgentPaused,
    #[error("timed out waiting for {0}")]
    Timeout(String),
}

pub fn cdp_live(e: CdpError) -> LiveError {
    match e {
        CdpError::Timeout(what) => LiveError::Timeout(what),
        CdpError::Blocked(host) => LiveError::Blocked(host),
        other => LiveError::Engine(other.to_string()),
    }
}

/// `BrowserEngineBuildStatus`.
#[derive(Debug, Clone, Serialize)]
pub struct BuildStatus {
    pub build: ChromeBuild,
    pub version: String,
    pub platform: String,
    pub installed: bool,
    pub download_bytes: u64,
    pub label: String,
    pub sha256_pinned: bool,
    pub path: Option<String>,
    pub source: Option<BinarySource>,
}

/// `BrowserLiveStatus`.
#[derive(Debug, Clone, Serialize)]
pub struct LiveStatus {
    pub platform_supported: bool,
    pub builds: Vec<BuildStatus>,
    pub settings: LiveSettings,
    pub install: Option<InstallJob>,
    pub processes: usize,
    pub sessions: usize,
}

pub struct LiveRuntime {
    data_dir: PathBuf,
    hooks: Arc<dyn LiveHooks>,
    settings: RwLock<LiveSettings>,
    procs: AsyncMutex<HashMap<ProcessKey, Arc<ChromeProcess>>>,
    sessions: StdMutex<HashMap<String, Arc<LiveSession>>>,
    open_lock: AsyncMutex<()>,
    proxy: AsyncMutex<Option<Arc<GuardProxy>>>,
    install: StdMutex<Option<InstallJob>>,
    installing: AtomicBool,
    fetcher: Arc<dyn Fetcher>,
    extractor: Arc<dyn Extractor>,
    janitor: StdMutex<Option<JoinHandle<()>>>,
}

impl LiveRuntime {
    pub fn new(data_dir: PathBuf, hooks: Arc<dyn LiveHooks>, settings: LiveSettings) -> Arc<Self> {
        Self::with_io(
            data_dir,
            hooks,
            settings,
            Arc::new(ReqwestFetcher::new()),
            Arc::new(DittoExtractor),
        )
    }

    /// Injectable download/extract (tests).
    pub fn with_io(
        data_dir: PathBuf,
        hooks: Arc<dyn LiveHooks>,
        settings: LiveSettings,
        fetcher: Arc<dyn Fetcher>,
        extractor: Arc<dyn Extractor>,
    ) -> Arc<Self> {
        Arc::new(Self {
            data_dir,
            hooks,
            settings: RwLock::new(settings),
            procs: AsyncMutex::new(HashMap::new()),
            sessions: StdMutex::new(HashMap::new()),
            open_lock: AsyncMutex::new(()),
            proxy: AsyncMutex::new(None),
            install: StdMutex::new(None),
            installing: AtomicBool::new(false),
            fetcher,
            extractor,
            janitor: StdMutex::new(None),
        })
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn settings(&self) -> LiveSettings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Replace the settings (validated by the caller). Running processes keep
    /// the build/headed mode they were launched with.
    pub fn set_settings(&self, s: LiveSettings) {
        if let Ok(mut g) = self.settings.write() {
            *g = s;
        }
    }

    pub async fn status(&self) -> LiveStatus {
        let platform = current_platform();
        let builds = ChromeBuild::ALL
            .iter()
            .filter_map(|b| {
                let pin = pin_for(*b, platform.unwrap_or("mac-arm64"))?;
                let resolved = platform.and(resolve_binary(&self.data_dir, *b));
                Some(BuildStatus {
                    build: *b,
                    version: pin.version.to_string(),
                    platform: pin.platform.to_string(),
                    installed: resolved.is_some(),
                    download_bytes: pin.size,
                    label: pin.label.to_string(),
                    sha256_pinned: effective_sha256(pin).is_some(),
                    path: resolved.as_ref().map(|(p, _)| p.display().to_string()),
                    source: resolved.map(|(_, s)| s),
                })
            })
            .collect();
        let processes = self
            .procs
            .lock()
            .await
            .values()
            .filter(|p| !p.is_dead())
            .count();
        LiveStatus {
            platform_supported: platform.is_some(),
            builds,
            settings: self.settings(),
            install: self.install.lock().ok().and_then(|j| j.clone()),
            processes,
            sessions: self.live_session_count(),
        }
    }

    // --------------------------------------------------------------- install

    /// Start the one-time download of `build` (explicit user action only).
    /// Returns the job as it starts — or an `installed` job when there is
    /// nothing to do.
    pub fn start_install(self: &Arc<Self>, build: ChromeBuild) -> Result<InstallJob, LiveError> {
        let platform = current_platform().ok_or(LiveError::Unsupported)?;
        let pin = *pin_for(build, platform).ok_or(LiveError::Unsupported)?;
        if is_installed(&self.data_dir, &pin) {
            let mut job = InstallJob::new(&pin);
            job.state = InstallState::Installed;
            job.received_bytes = pin.size;
            job.finished_at = Some(job.started_at.clone());
            return Ok(job);
        }
        let sha = effective_sha256(&pin).ok_or_else(|| {
            LiveError::Invalid(format!(
                "the {} download's sha256 is not pinned in this Otto build — refusing to install it",
                build.as_str()
            ))
        })?;
        if self.installing.swap(true, Ordering::SeqCst) {
            return Err(LiveError::Conflict(
                "an engine download is already running".into(),
            ));
        }
        let job = InstallJob::new(&pin);
        if let Ok(mut j) = self.install.lock() {
            *j = Some(job.clone());
        }
        let me = self.clone();
        tokio::spawn(async move {
            let weak = Arc::downgrade(&me);
            let progress = move |j: &InstallJob| {
                if let Some(rt) = weak.upgrade() {
                    if let Ok(mut slot) = rt.install.lock() {
                        *slot = Some(j.clone());
                    }
                    rt.hooks.install_progress(j);
                }
            };
            let result = install::install(
                &me.data_dir,
                &pin,
                &sha,
                me.fetcher.as_ref(),
                me.extractor.clone(),
                &progress,
            )
            .await;
            me.installing.store(false, Ordering::SeqCst);
            match &result {
                Ok(path) => tracing::info!(
                    "browser live: installed {} {} at {}",
                    pin.build.as_str(),
                    pin.version,
                    path.display()
                ),
                Err(e) => tracing::warn!(
                    "browser live: installing {} failed: {e}",
                    pin.build.as_str()
                ),
            }
        });
        Ok(job)
    }

    // -------------------------------------------------------------- sessions

    pub fn session(&self, tab_id: &str) -> Option<Arc<LiveSession>> {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(tab_id).cloned())
    }

    pub fn sessions_in(&self, workspace_id: &str) -> Vec<Arc<LiveSession>> {
        self.sessions
            .lock()
            .map(|s| {
                s.values()
                    .filter(|x| x.workspace_id == workspace_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn live_session_count(&self) -> usize {
        self.sessions
            .lock()
            .map(|s| s.values().filter(|x| x.is_live()).count())
            .unwrap_or(0)
    }

    /// Open (or re-attach to the owner's existing) session for a tab.
    /// `Ok((session, created))`.
    pub async fn open(
        self: &Arc<Self>,
        p: OpenParams,
    ) -> Result<(Arc<LiveSession>, bool), LiveError> {
        self.ensure_janitor();
        if !valid_profile_name(&p.profile) {
            return Err(LiveError::Invalid(
                "profile must be \"ephemeral\" or match [a-z0-9_-]{1,40}".into(),
            ));
        }
        let _serial = self.open_lock.lock().await;
        if let Some(existing) = self.session(&p.tab_id) {
            if existing.is_live() {
                if existing.owner_id != p.owner_id {
                    return Err(LiveError::Conflict(
                        "another user owns this tab's live session".into(),
                    ));
                }
                if let Some(url) = &p.url {
                    existing
                        .navigate(super::protocol::NavAction::Goto, Some(url.clone()))
                        .await?;
                }
                return Ok((existing, false));
            }
            // Crashed / closed leftovers are replaced.
            self.remove(&p.tab_id);
            existing.close("replaced").await;
        }

        let settings = self.settings();
        if self.live_session_count() >= settings.max_sessions as usize {
            return Err(LiveError::TooMany(settings.max_sessions));
        }
        let (binary, _source) = match resolve_binary(&self.data_dir, settings.build) {
            Some(b) => b,
            None if current_platform().is_none() => return Err(LiveError::Unsupported),
            None => return Err(LiveError::NotInstalled),
        };
        let key = ProcessKey::for_profile(&p.workspace_id, &p.owner_id, &p.profile);
        let proc = self.process_for(key, binary, &settings).await?;
        let session =
            LiveSession::create(proc, p, self.data_dir.clone(), self.hooks.clone()).await?;
        if let Ok(mut s) = self.sessions.lock() {
            s.insert(session.tab_id.clone(), session.clone());
        }
        self.audit(
            "browser.live.open",
            &session,
            json!({"build": settings.build.as_str(), "headed": settings.headed}),
        )
        .await;
        Ok((session, true))
    }

    fn remove(&self, tab_id: &str) -> Option<Arc<LiveSession>> {
        self.sessions.lock().ok().and_then(|mut s| s.remove(tab_id))
    }

    /// Close a tab's session (no-op when none).
    pub async fn close(&self, tab_id: &str, reason: &'static str) -> bool {
        let Some(s) = self.remove(tab_id) else {
            return false;
        };
        s.close(reason).await;
        self.audit("browser.live.close", &s, json!({"reason": reason}))
            .await;
        true
    }

    async fn audit(&self, action: &'static str, s: &LiveSession, mut detail: serde_json::Value) {
        detail["workspace_id"] = json!(s.workspace_id);
        detail["profile"] = json!(s.profile);
        self.hooks
            .audit(LiveAudit {
                action,
                user_id: Some(s.owner_id.clone()),
                target: s.tab_id.clone(),
                detail,
            })
            .await;
    }

    async fn proxy(&self) -> Result<Arc<GuardProxy>, LiveError> {
        let mut g = self.proxy.lock().await;
        if let Some(p) = g.as_ref() {
            return Ok(p.clone());
        }
        let p = Arc::new(
            GuardProxy::start()
                .await
                .map_err(|e| LiveError::Engine(format!("start the guard proxy: {e}")))?,
        );
        *g = Some(p.clone());
        Ok(p)
    }

    async fn process_for(
        &self,
        key: ProcessKey,
        binary: PathBuf,
        settings: &LiveSettings,
    ) -> Result<Arc<ChromeProcess>, LiveError> {
        let mut procs = self.procs.lock().await;
        if let Some(p) = procs.get(&key) {
            if !p.is_dead() {
                return Ok(p.clone());
            }
            procs.remove(&key);
        }
        if procs.len() >= MAX_PROCESSES {
            let idle = procs
                .iter()
                .find(|(_, p)| p.session_count() == 0 || p.is_dead())
                .map(|(k, _)| k.clone());
            match idle.and_then(|k| procs.remove(&k)) {
                Some(p) => p.shutdown().await,
                None => {
                    return Err(LiveError::Conflict(format!(
                        "at most {MAX_PROCESSES} browser profiles can run at once — close a live tab first"
                    )))
                }
            }
        }
        let proxy = self.proxy().await?;
        let spec = LaunchSpec {
            binary,
            build: settings.build,
            headed: settings.headed,
            user_data_dir: key.user_data_dir(&self.data_dir),
            viewport: Viewport::default(),
            extra_args: proxy.chrome_args(),
            log_path: self
                .data_dir
                .join("browser")
                .join("logs")
                .join(key.log_name()),
        };
        let downloads = key.downloads_dir(&self.data_dir);
        let proc = ChromeProcess::start(key.clone(), spec, settings.downloads, downloads).await?;
        procs.insert(key, proc.clone());
        Ok(proc)
    }

    // ---------------------------------------------------------------- janitor

    fn ensure_janitor(self: &Arc<Self>) {
        let Ok(mut j) = self.janitor.lock() else {
            return;
        };
        if j.is_some() {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        *j = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(SWEEP_EVERY).await;
                let Some(rt) = weak.upgrade() else { return };
                rt.sweep().await;
            }
        }));
    }

    /// Close idle sessions, drop crashed/closed ones nobody watches, and stop
    /// processes that have been empty for [`PROCESS_IDLE_EXIT_SECS`].
    pub async fn sweep(&self) {
        let now = Instant::now();
        let idle_limit = Duration::from_secs(self.settings().idle_timeout_secs);
        let snapshot: Vec<(String, Arc<LiveSession>)> = self
            .sessions
            .lock()
            .map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (tab, s) in snapshot {
            if !s.is_live() {
                if s.viewer_count() == 0 {
                    self.close(&tab, "closed").await;
                }
                continue;
            }
            if s.idle_for(now).is_some_and(|d| d >= idle_limit) {
                tracing::info!("browser live: closing idle session for tab {tab}");
                self.close(&tab, "idle").await;
            }
        }
        let stale: Vec<Arc<ChromeProcess>> = {
            let mut procs = self.procs.lock().await;
            let keys: Vec<ProcessKey> = procs
                .iter()
                .filter(|(_, p)| {
                    p.is_dead()
                        || (p.session_count() == 0
                            && p.empty_for(now)
                                .is_some_and(|d| d >= Duration::from_secs(PROCESS_IDLE_EXIT_SECS)))
                })
                .map(|(k, _)| k.clone())
                .collect();
            keys.iter().filter_map(|k| procs.remove(k)).collect()
        };
        for p in stale {
            p.shutdown().await;
        }
    }

    /// Daemon shutdown: close every session, stop every process.
    pub async fn shutdown(&self) {
        let tabs: Vec<String> = self
            .sessions
            .lock()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default();
        for t in tabs {
            self.close(&t, "closed").await;
        }
        let procs: Vec<Arc<ChromeProcess>> =
            self.procs.lock().await.drain().map(|(_, p)| p).collect();
        for p in procs {
            p.shutdown().await;
        }
        if let Ok(mut j) = self.janitor.lock() {
            if let Some(h) = j.take() {
                h.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::hooks::NoopHooks;

    fn rt(dir: &std::path::Path) -> Arc<LiveRuntime> {
        LiveRuntime::new(
            dir.to_path_buf(),
            Arc::new(NoopHooks),
            LiveSettings::default(),
        )
    }

    fn params(profile: &str) -> OpenParams {
        OpenParams {
            tab_id: "tab1".into(),
            workspace_id: "w1".into(),
            owner_id: "u1".into(),
            profile: profile.into(),
            viewport: Viewport::default(),
            url: None,
        }
    }

    #[tokio::test]
    async fn status_reports_both_builds_uninstalled_on_a_fresh_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let st = rt(tmp.path()).status().await;
        assert_eq!(st.builds.len(), 2);
        if std::env::var(install::ENV_BIN).is_err() {
            assert!(st.builds.iter().all(|b| !b.installed));
        }
        assert_eq!(st.sessions, 0);
        assert_eq!(st.processes, 0);
        assert_eq!(st.settings, LiveSettings::default());
    }

    #[tokio::test]
    async fn opening_without_an_engine_is_refused_before_any_launch() {
        if std::env::var(install::ENV_BIN).is_ok() {
            return; // a developer machine pointing at a real Chrome
        }
        let tmp = tempfile::tempdir().unwrap();
        let r = rt(tmp.path()).open(params("ephemeral")).await;
        assert!(matches!(
            r,
            Err(LiveError::NotInstalled) | Err(LiveError::Unsupported)
        ));
    }

    #[tokio::test]
    async fn bad_profile_names_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let r = rt(tmp.path()).open(params("../escape")).await;
        assert!(matches!(r, Err(LiveError::Invalid(_))));
    }

    #[tokio::test]
    async fn install_is_refused_when_the_checksum_is_not_pinned() {
        if current_platform().is_none() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let runtime = rt(tmp.path());
        for b in ChromeBuild::ALL {
            let pin = pin_for(b, current_platform().unwrap()).unwrap();
            if effective_sha256(pin).is_none() {
                assert!(matches!(
                    runtime.start_install(b),
                    Err(LiveError::Invalid(_))
                ));
            }
        }
    }

    #[test]
    fn cdp_errors_map_to_live_errors() {
        assert!(matches!(
            cdp_live(CdpError::Timeout("x".into())),
            LiveError::Timeout(_)
        ));
        assert!(matches!(cdp_live(CdpError::Closed), LiveError::Engine(_)));
        assert!(matches!(
            cdp_live(CdpError::Blocked("h".into())),
            LiveError::Blocked(_)
        ));
    }
}

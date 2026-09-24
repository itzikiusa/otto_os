//! One daemon-owned Chromium process (the shared ephemeral one, or one per
//! persistent profile): launch + CDP setup, the browser-level event loop
//! (session-long `Fetch` guard pump, target/popup/crash tracking, downloads),
//! and shutdown. Page sessions ([`super::session::LiveSession`]) register
//! their target id here so browser-level events reach them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::process::Child;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tokio::task::JoinHandle;

use super::chrome::{launch, restrict_dir, LaunchSpec, Launched};
use super::conn::{CdpConn, CdpEvent};
use super::guard::{self, Decision, Paused, VerdictCache};
use super::runtime::LiveError;
use super::session::LiveSession;
use super::types::{safe_segment, ChromeBuild, DownloadPolicy, EPHEMERAL_PROFILE};

/// A page-initiated download larger than this is cancelled.
pub const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Concurrent in-flight guard verdicts (DNS lookups) per process.
const GUARD_SLOTS: usize = 64;

/// Which cookie jar a process serves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProcessKey {
    /// Throwaway: one browser context per session inside it.
    Ephemeral,
    /// Persistent jar scoped to (workspace, owner, name).
    Profile {
        workspace: String,
        owner: String,
        name: String,
    },
}

impl ProcessKey {
    pub fn for_profile(workspace: &str, owner: &str, profile: &str) -> Self {
        if profile == EPHEMERAL_PROFILE {
            ProcessKey::Ephemeral
        } else {
            ProcessKey::Profile {
                workspace: workspace.to_string(),
                owner: owner.to_string(),
                name: profile.to_string(),
            }
        }
    }

    pub fn user_data_dir(&self, data_dir: &std::path::Path) -> PathBuf {
        let base = data_dir.join("browser");
        match self {
            ProcessKey::Ephemeral => base.join("ephemeral-udd"),
            ProcessKey::Profile {
                workspace,
                owner,
                name,
            } => base
                .join("profiles")
                .join(safe_segment(workspace))
                .join(safe_segment(owner))
                .join(safe_segment(name)),
        }
    }

    /// Quarantine folder for a profile's downloads (ephemeral sessions get a
    /// per-(workspace, owner) folder via [`ephemeral_downloads_dir`]).
    pub fn downloads_dir(&self, data_dir: &std::path::Path) -> PathBuf {
        let base = data_dir.join("browser").join("downloads");
        match self {
            ProcessKey::Ephemeral => base.join("ephemeral").join("_default"),
            ProcessKey::Profile {
                workspace,
                owner,
                name,
            } => base
                .join(safe_segment(workspace))
                .join(safe_segment(owner))
                .join(safe_segment(name)),
        }
    }

    pub fn log_name(&self) -> String {
        match self {
            ProcessKey::Ephemeral => "chrome-ephemeral.log".into(),
            ProcessKey::Profile {
                workspace,
                owner,
                name,
            } => format!(
                "chrome-{}-{}-{}.log",
                safe_segment(workspace),
                safe_segment(owner),
                safe_segment(name)
            ),
        }
    }
}

pub fn ephemeral_downloads_dir(data_dir: &std::path::Path, workspace: &str, owner: &str) -> PathBuf {
    data_dir
        .join("browser")
        .join("downloads")
        .join("ephemeral")
        .join(safe_segment(workspace))
        .join(safe_segment(owner))
}

struct DownloadInfo {
    session: Weak<LiveSession>,
    filename: String,
    dir: PathBuf,
    context_id: Option<String>,
}

pub struct ChromeProcess {
    pub key: ProcessKey,
    pub build: ChromeBuild,
    pub version: String,
    pub headed: bool,
    pub conn: Arc<CdpConn>,
    pub download_policy: DownloadPolicy,
    pub downloads_dir: PathBuf,
    /// Browser-level `Fetch.enable` took (covers every target, incl. OOPIFs,
    /// workers and popups). When `false`, each session enables it on its own
    /// target instead (the guard proxy still vets every connection).
    pub browser_fetch: bool,
    child: AsyncMutex<Option<Child>>,
    targets: StdMutex<HashMap<String, Weak<LiveSession>>>,
    popups: StdMutex<HashMap<String, String>>,
    downloads: StdMutex<HashMap<String, DownloadInfo>>,
    verdicts: StdMutex<VerdictCache>,
    guard_slots: Arc<Semaphore>,
    empty_since: StdMutex<Option<Instant>>,
    dead: AtomicBool,
    loop_task: StdMutex<Option<JoinHandle<()>>>,
}

impl ChromeProcess {
    /// Launch + set up. `spec.user_data_dir` must come from `key`.
    pub async fn start(
        key: ProcessKey,
        spec: LaunchSpec,
        download_policy: DownloadPolicy,
        downloads_dir: PathBuf,
    ) -> Result<Arc<Self>, LiveError> {
        if key == ProcessKey::Ephemeral {
            // Nothing from an earlier ephemeral run survives a relaunch.
            let _ = std::fs::remove_dir_all(&spec.user_data_dir);
        }
        let Launched {
            child,
            conn,
            browser_events,
        } = launch(&spec).map_err(|e| LiveError::Engine(format!("launch chromium: {e}")))?;

        let setup = Self::setup(&conn, download_policy, &downloads_dir).await;
        let (version, browser_fetch) = match setup {
            Ok(v) => v,
            Err(e) => {
                let mut child = child;
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                conn.shutdown();
                return Err(e);
            }
        };

        let proc = Arc::new(Self {
            key,
            build: spec.build,
            version,
            headed: spec.headed,
            conn,
            download_policy,
            downloads_dir,
            browser_fetch,
            child: AsyncMutex::new(Some(child)),
            targets: StdMutex::new(HashMap::new()),
            popups: StdMutex::new(HashMap::new()),
            downloads: StdMutex::new(HashMap::new()),
            verdicts: StdMutex::new(VerdictCache::default()),
            guard_slots: Arc::new(Semaphore::new(GUARD_SLOTS)),
            empty_since: StdMutex::new(Some(Instant::now())),
            dead: AtomicBool::new(false),
            loop_task: StdMutex::new(None),
        });
        let weak = Arc::downgrade(&proc);
        let task = tokio::spawn(event_loop(weak, browser_events));
        if let Ok(mut t) = proc.loop_task.lock() {
            *t = Some(task);
        }
        tracing::info!(
            "browser live: chromium {} ready ({:?}, browser-level fetch guard: {})",
            proc.version,
            proc.key,
            proc.browser_fetch
        );
        Ok(proc)
    }

    async fn setup(
        conn: &Arc<CdpConn>,
        policy: DownloadPolicy,
        downloads_dir: &std::path::Path,
    ) -> Result<(String, bool), LiveError> {
        let v = conn
            .call_with_timeout("Browser.getVersion", json!({}), None, Duration::from_secs(20))
            .await
            .map_err(|e| LiveError::Engine(format!("chromium did not answer on the CDP pipe: {e}")))?;
        let product = v.get("product").and_then(Value::as_str).unwrap_or("");
        let version = product.split('/').nth(1).unwrap_or(product).to_string();
        conn.call("Target.setDiscoverTargets", json!({"discover": true}), None)
            .await
            .map_err(|e| LiveError::Engine(format!("Target.setDiscoverTargets: {e}")))?;
        let browser_fetch = conn
            .call(
                "Fetch.enable",
                json!({"patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
                None,
            )
            .await
            .is_ok();
        if !browser_fetch {
            tracing::warn!(
                "browser live: browser-level Fetch.enable refused; guarding per target instead"
            );
        }
        conn.call("Browser.setDownloadBehavior", download_behavior(policy, downloads_dir, None), None)
            .await
            .map_err(|e| LiveError::Engine(format!("Browser.setDownloadBehavior: {e}")))?;
        Ok((version, browser_fetch))
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst) || self.conn.is_closed()
    }

    pub fn register(&self, target_id: &str, session: &Arc<LiveSession>) {
        if let Ok(mut t) = self.targets.lock() {
            t.insert(target_id.to_string(), Arc::downgrade(session));
        }
        if let Ok(mut e) = self.empty_since.lock() {
            *e = None;
        }
    }

    pub fn unregister(&self, target_id: &str) {
        let empty = match self.targets.lock() {
            Ok(mut t) => {
                t.remove(target_id);
                t.retain(|_, w| w.strong_count() > 0);
                t.is_empty()
            }
            Err(_) => false,
        };
        if empty {
            if let Ok(mut e) = self.empty_since.lock() {
                *e = Some(Instant::now());
            }
        }
    }

    /// How long this process has had no sessions (`None` while it has some).
    pub fn empty_for(&self, now: Instant) -> Option<Duration> {
        self.empty_since
            .lock()
            .ok()
            .and_then(|e| *e)
            .map(|at| now.saturating_duration_since(at))
    }

    pub fn session_count(&self) -> usize {
        self.targets
            .lock()
            .map(|t| t.values().filter(|w| w.strong_count() > 0).count())
            .unwrap_or(0)
    }

    fn session(&self, target_id: &str) -> Option<Arc<LiveSession>> {
        self.targets
            .lock()
            .ok()
            .and_then(|t| t.get(target_id).and_then(Weak::upgrade))
    }

    fn only_session(&self) -> Option<Arc<LiveSession>> {
        let t = self.targets.lock().ok()?;
        let live: Vec<_> = t.values().filter_map(Weak::upgrade).collect();
        if live.len() == 1 {
            live.into_iter().next()
        } else {
            None
        }
    }

    /// Answer one paused request (browser-level, or `via` a page session when
    /// the per-target fallback is in use).
    pub async fn on_paused(self: Arc<Self>, params: Value, via: Option<String>) {
        let Some(req) = Paused::parse(&params) else {
            return;
        };
        let permit = self.guard_slots.clone().acquire_owned().await.ok();
        let allowed = guard::vet(&req.url, &self.verdicts).await;
        drop(permit);
        let session = req.frame_id.as_deref().and_then(|f| self.session(f));
        let agent_drives = session.as_ref().is_some_and(|s| s.agent_drives());
        match guard::decide(allowed, &req, session.is_some(), agent_drives) {
            Decision::Continue => {
                self.conn.send(
                    "Fetch.continueRequest",
                    json!({"requestId": req.request_id}),
                    via.as_deref(),
                );
            }
            Decision::Fail { document } => {
                self.conn.send(
                    "Fetch.failRequest",
                    json!({"requestId": req.request_id, "errorReason": "BlockedByClient"}),
                    via.as_deref(),
                );
                if document {
                    if let Some(s) = &session {
                        s.notify_blocked(&guard::host_of(&req.url));
                    }
                    tracing::info!(
                        host = %guard::host_of(&req.url),
                        "browser live: navigation blocked by the SSRF guard"
                    );
                }
            }
            Decision::HoldOutward => match session {
                Some(s) => s.hold_outward(req, via).await,
                None => self.conn.send(
                    "Fetch.continueRequest",
                    json!({"requestId": req.request_id}),
                    via.as_deref(),
                ),
            },
        }
    }

    fn on_browser_event(self: &Arc<Self>, ev: CdpEvent) {
        let p = &ev.params;
        match ev.method.as_str() {
            "Fetch.requestPaused" => {
                let me = self.clone();
                let params = ev.params.clone();
                let via = ev.session_id.clone();
                tokio::spawn(async move { me.on_paused(params, via).await });
            }
            "Target.targetInfoChanged" => {
                let info = &p["targetInfo"];
                let tid = info["targetId"].as_str().unwrap_or("");
                let url = info["url"].as_str().unwrap_or("");
                if self.is_popup(tid) {
                    if is_web_url(url) {
                        self.close_popup(tid, Some(url.to_string()));
                    }
                    return;
                }
                if let Some(s) = self.session(tid) {
                    s.on_target_info(info["title"].as_str().unwrap_or(""), url);
                }
            }
            "Target.targetCreated" => {
                let info = &p["targetInfo"];
                let tid = info["targetId"].as_str().unwrap_or("").to_string();
                let opener = info["openerId"].as_str().unwrap_or("");
                let is_page = info["type"].as_str() == Some("page");
                if is_page && !opener.is_empty() && self.session(opener).is_some() {
                    if let Ok(mut pp) = self.popups.lock() {
                        pp.insert(tid.clone(), opener.to_string());
                    }
                    let url = info["url"].as_str().unwrap_or("");
                    if is_web_url(url) {
                        self.close_popup(&tid, Some(url.to_string()));
                    } else {
                        // A popup that never navigates (script-written) is
                        // closed without a url after a short grace period.
                        let me = Arc::downgrade(self);
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            if let Some(me) = me.upgrade() {
                                if me.is_popup(&tid) {
                                    me.close_popup(&tid, None);
                                }
                            }
                        });
                    }
                }
            }
            "Target.targetDestroyed" => {
                let tid = p["targetId"].as_str().unwrap_or("");
                if let Ok(mut pp) = self.popups.lock() {
                    pp.remove(tid);
                }
                if let Some(s) = self.session(tid) {
                    s.on_target_gone(false);
                }
            }
            "Target.targetCrashed" => {
                let tid = p["targetId"].as_str().unwrap_or("");
                if let Some(s) = self.session(tid) {
                    s.on_target_gone(true);
                }
            }
            "Browser.downloadWillBegin" => self.on_download_begin(p),
            "Browser.downloadProgress" => self.on_download_progress(p),
            _ => {}
        }
    }

    fn is_popup(&self, tid: &str) -> bool {
        self.popups
            .lock()
            .map(|p| p.contains_key(tid))
            .unwrap_or(false)
    }

    fn close_popup(&self, tid: &str, url: Option<String>) {
        let opener = self.popups.lock().ok().and_then(|mut p| p.remove(tid));
        self.conn
            .send("Target.closeTarget", json!({"targetId": tid}), None);
        if let (Some(opener), Some(url)) = (opener, url) {
            if let Some(s) = self.session(&opener) {
                s.notify_popup(&url);
            }
        }
    }

    fn on_download_begin(&self, p: &Value) {
        let guid = p["guid"].as_str().unwrap_or("").to_string();
        if guid.is_empty() {
            return;
        }
        let filename = sanitize_filename(p["suggestedFilename"].as_str().unwrap_or("download"));
        let session = p["frameId"]
            .as_str()
            .and_then(|f| self.session(f))
            .or_else(|| self.only_session());
        let (dir, context_id) = match &session {
            Some(s) => (s.downloads_dir(), s.context_id()),
            None => (self.downloads_dir.clone(), None),
        };
        if self.download_policy == DownloadPolicy::Block {
            if let Some(s) = &session {
                s.notify_download("blocked", &filename, None);
            }
            return;
        }
        if let Ok(mut d) = self.downloads.lock() {
            d.insert(
                guid,
                DownloadInfo {
                    session: session.as_ref().map(Arc::downgrade).unwrap_or_default(),
                    filename,
                    dir,
                    context_id,
                },
            );
        }
    }

    fn on_download_progress(&self, p: &Value) {
        let guid = p["guid"].as_str().unwrap_or("").to_string();
        let state = p["state"].as_str().unwrap_or("");
        let received = p["receivedBytes"].as_f64().unwrap_or(0.0).max(0.0) as u64;
        match state {
            "inProgress" if received > MAX_DOWNLOAD_BYTES => {
                let info = self.downloads.lock().ok().and_then(|mut d| d.remove(&guid));
                if let Some(info) = info {
                    let mut params = json!({"guid": guid});
                    if let Some(ctx) = &info.context_id {
                        params["browserContextId"] = json!(ctx);
                    }
                    self.conn.send("Browser.cancelDownload", params, None);
                    if let Some(s) = info.session.upgrade() {
                        s.notify_download("blocked", &info.filename, Some(received));
                    }
                }
            }
            "completed" => {
                let info = self.downloads.lock().ok().and_then(|mut d| d.remove(&guid));
                if let Some(info) = info {
                    let path = info.dir.join(&guid);
                    let meta = info.dir.join(format!("{guid}.json"));
                    let filename = info.filename.clone();
                    tokio::spawn(async move {
                        quarantine(&path).await;
                        let _ = tokio::fs::write(
                            &meta,
                            serde_json::to_vec(&json!({
                                "filename": filename,
                                "bytes": received,
                                "saved_at": chrono::Utc::now().to_rfc3339(),
                            }))
                            .unwrap_or_default(),
                        )
                        .await;
                    });
                    if let Some(s) = info.session.upgrade() {
                        s.notify_download("quarantined", &info.filename, Some(received));
                    }
                }
            }
            "canceled" => {
                if let Ok(mut d) = self.downloads.lock() {
                    d.remove(&guid);
                }
            }
            _ => {}
        }
    }

    /// Every session on this process lost its browser.
    fn on_exit(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let sessions: Vec<Arc<LiveSession>> = self
            .targets
            .lock()
            .map(|t| t.values().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for s in sessions {
            s.on_target_gone(true);
        }
        tracing::warn!("browser live: chromium ({:?}) exited", self.key);
    }

    /// `Browser.close`, then SIGKILL after a grace period.
    pub async fn shutdown(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let _ = self
            .conn
            .call_with_timeout("Browser.close", json!({}), None, Duration::from_secs(3))
            .await;
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                }
            }
        }
        self.conn.shutdown();
        if let Ok(mut t) = self.loop_task.lock() {
            if let Some(h) = t.take() {
                h.abort();
            }
        }
    }

    /// Reap the child after an unexpected exit (no zombie).
    async fn reap(&self) {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        }
    }
}

async fn event_loop(proc: Weak<ChromeProcess>, mut events: tokio::sync::mpsc::UnboundedReceiver<CdpEvent>) {
    while let Some(ev) = events.recv().await {
        let Some(p) = proc.upgrade() else {
            return;
        };
        p.on_browser_event(ev);
    }
    // The pipe closed: the browser is gone.
    if let Some(p) = proc.upgrade() {
        p.on_exit();
        p.reap().await;
    }
}

/// `Browser.setDownloadBehavior` params for a policy (optionally for one
/// browser context).
pub fn download_behavior(policy: DownloadPolicy, dir: &std::path::Path, context_id: Option<&str>) -> Value {
    let mut v = match policy {
        DownloadPolicy::Block => json!({"behavior": "deny", "eventsEnabled": true}),
        DownloadPolicy::Quarantine => {
            let _ = std::fs::create_dir_all(dir);
            restrict_dir(dir);
            // `allowAndName` saves under the download GUID — the page can't
            // choose the on-disk name (no path tricks, no overwrite).
            json!({
                "behavior": "allowAndName",
                "downloadPath": dir.display().to_string(),
                "eventsEnabled": true,
            })
        }
    };
    if let Some(ctx) = context_id {
        v["browserContextId"] = json!(ctx);
    }
    v
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// A display-only filename: no path separators or control characters.
pub fn sanitize_filename(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c })
        .take(200)
        .collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "download".into()
    } else {
        s
    }
}

/// Mark a saved download as quarantined for Gatekeeper and drop any exec bit
/// — Otto never opens or runs it.
async fn quarantine(path: &std::path::Path) {
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    let value = format!("0081;{:x};Otto;", chrono::Utc::now().timestamp());
    let _ = tokio::process::Command::new("/usr/bin/xattr")
        .arg("-w")
        .arg("com.apple.quarantine")
        .arg(value)
        .arg(path)
        .output()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn profile_keys_scope_jars_by_workspace_owner_and_name() {
        let d = Path::new("/data");
        assert_eq!(
            ProcessKey::for_profile("w1", "u1", EPHEMERAL_PROFILE),
            ProcessKey::Ephemeral
        );
        let a = ProcessKey::for_profile("w1", "u1", "work");
        let b = ProcessKey::for_profile("w1", "u2", "work");
        let c = ProcessKey::for_profile("w2", "u1", "work");
        assert_ne!(a.user_data_dir(d), b.user_data_dir(d));
        assert_ne!(a.user_data_dir(d), c.user_data_dir(d));
        assert_eq!(
            a.user_data_dir(d),
            Path::new("/data/browser/profiles/w1/u1/work")
        );
        // Hostile ids can't escape the profiles root.
        let evil = ProcessKey::for_profile("../..", "..", "x");
        assert!(evil
            .user_data_dir(d)
            .starts_with("/data/browser/profiles/"));
        assert!(!evil.user_data_dir(d).to_string_lossy().contains(".."));
        assert_ne!(a.downloads_dir(d), b.downloads_dir(d));
    }

    #[test]
    fn download_behavior_per_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let v = download_behavior(DownloadPolicy::Block, tmp.path(), None);
        assert_eq!(v["behavior"], "deny");
        let dir = tmp.path().join("q");
        let v = download_behavior(DownloadPolicy::Quarantine, &dir, Some("CTX"));
        assert_eq!(v["behavior"], "allowAndName");
        assert_eq!(v["browserContextId"], "CTX");
        assert!(dir.is_dir());
    }

    #[test]
    fn filenames_are_display_safe() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a\u{0}b\nc.pdf"), "abc.pdf");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename(""), "download");
    }
}

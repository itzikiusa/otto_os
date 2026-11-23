//! Built-in one-click installer for Android Emulator / Platform-Tools and
//! iOS Simulator Runtime (`POST /api/v1/simulators/install`).
//!
//! - Never runs at daemon boot — only triggered by an explicit Admin action.
//! - Single-flight with monotonic `run_id` (`409 Conflict` if busy; cancelled
//!   runs cannot overwrite a newer run's status or staging directory).
//! - Synchronous discovery-cache invalidation before publishing `Done`.
//! - HTTPS-only downloads through `otto_netguard::guarded_client_builder()`,
//!   bounded size cap, real writability probe, zero shell interpolation.

pub mod android;
pub mod ios;

pub use crate::android_env::discover_java_home;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use otto_core::auth::BoxFuture;
use tokio::sync::Mutex;

use crate::exec::{CommandRunner, HostProbe};
use crate::types::{SimInstallReq, SimInstallStage, SimInstallStatus, SimInstallTarget};

/// Official Google Android Command-Line Tools archive for macOS (verified HTTP 200).
pub const ANDROID_CMDLINETOOLS_MAC_URL: &str =
    "https://dl.google.com/android/repository/commandlinetools-mac-13114758_latest.zip";

/// Hard upper bound on downloaded cmdline-tools zip (~143 MB official; cap at 350 MB).
pub const MAX_CMDLINETOOLS_BYTES: u64 = 350 * 1024 * 1024;

/// Maximum log lines kept in `SimInstallStatus::log_tail`.
const MAX_LOG_TAIL: usize = 80;

/// Async hook called synchronously before publishing terminal install status.
pub type BeforePublishFn = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// Byte stream fetcher abstraction (real = netguarded reqwest; tests = fake).
#[async_trait]
pub trait ArchiveFetcher: Send + Sync {
    async fn open(
        &self,
        url: &str,
    ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String>;
}

/// Safe archive extractor abstraction (real = macOS `ditto -x -k`; tests = fake).
#[async_trait]
pub trait ArchiveExtractor: Send + Sync {
    async fn extract_zip(&self, archive: &Path, dest: &Path) -> Result<(), String>;
}

/// Production HTTPS fetcher backed by `otto_netguard::guarded_client_builder()`.
pub struct NetguardFetcher {
    client: reqwest::Client,
}

impl Default for NetguardFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl NetguardFetcher {
    pub fn new() -> Self {
        let client = otto_netguard::guarded_client_builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent("Otto-Simulators/1.0")
            .build()
            .expect("build netguarded http client");
        Self { client }
    }
}

#[async_trait]
impl ArchiveFetcher for NetguardFetcher {
    async fn open(
        &self,
        url: &str,
    ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String> {
        validate_https_download_url(url)?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download returned HTTP {}", resp.status()));
        }
        let len = resp.content_length();
        let stream = resp
            .bytes_stream()
            .map(|chunk| chunk.map(|b| b.to_vec()).map_err(|e| e.to_string()));
        Ok((len, Box::pin(stream)))
    }
}

/// Production zip extractor using macOS `ditto -x -k` (or `unzip -q -o` fallback).
#[derive(Debug, Default)]
pub struct SystemZipExtractor;

#[async_trait]
impl ArchiveExtractor for SystemZipExtractor {
    async fn extract_zip(&self, archive: &Path, dest: &Path) -> Result<(), String> {
        tokio::fs::create_dir_all(dest)
            .await
            .map_err(|e| format!("create extract dir: {e}"))?;
        let (prog, args): (&str, Vec<String>) = if Path::new("/usr/bin/ditto").is_file() {
            (
                "/usr/bin/ditto",
                vec![
                    "-x".into(),
                    "-k".into(),
                    archive.to_string_lossy().into_owned(),
                    dest.to_string_lossy().into_owned(),
                ],
            )
        } else {
            (
                "unzip",
                vec![
                    "-q".into(),
                    "-o".into(),
                    archive.to_string_lossy().into_owned(),
                    "-d".into(),
                    dest.to_string_lossy().into_owned(),
                ],
            )
        };
        let out = tokio::process::Command::new(prog)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| format!("failed to run {prog}: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("zip extraction failed: {}", err.trim()));
        }
        Ok(())
    }
}

/// Reject any non-HTTPS download URL (prevents plaintext MITM or file:// abuse).
pub fn validate_https_download_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") {
        return Err("installer download URL must use https://".into());
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("installer download URL contains invalid characters".into());
    }
    Ok(())
}

/// Default Android system image for the current host CPU architecture.
pub fn default_android_system_image() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "system-images;android-35;google_apis;arm64-v8a"
    } else {
        "system-images;android-35;google_apis;x86_64"
    }
}

/// Otto's managed Android SDK location under `<data_dir>/android-sdk`.
pub fn managed_android_sdk_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("android-sdk")
}

/// Verify actual directory writability by creating and removing a temporary
/// probe file (Unix `Permissions::readonly()` returns false for
/// root-owned `0755` directories).
pub(crate) async fn is_writable_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let probe_path = path.join(format!(
        ".otto-write-probe-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .await
    {
        Ok(f) => {
            drop(f);
            let _ = tokio::fs::remove_file(&probe_path).await;
            true
        }
        Err(_) => false,
    }
}

/// Single-flight install manager owned by [`crate::SimulatorsService`].
pub struct InstallManager {
    data_dir: PathBuf,
    fetcher: Arc<dyn ArchiveFetcher>,
    extractor: Arc<dyn ArchiveExtractor>,
    state: Arc<Mutex<InstallInner>>,
}

pub(crate) struct InstallInner {
    pub(crate) run_id: u64,
    pub(crate) status: SimInstallStatus,
    pub(crate) abort: Option<tokio::task::AbortHandle>,
}

impl InstallManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_deps(
            data_dir,
            Arc::new(NetguardFetcher::new()),
            Arc::new(SystemZipExtractor),
        )
    }

    pub fn with_deps(
        data_dir: PathBuf,
        fetcher: Arc<dyn ArchiveFetcher>,
        extractor: Arc<dyn ArchiveExtractor>,
    ) -> Self {
        Self {
            data_dir,
            fetcher,
            extractor,
            state: Arc::new(Mutex::new(InstallInner {
                run_id: 0,
                status: SimInstallStatus::default(),
                abort: None,
            })),
        }
    }

    pub fn managed_sdk_dir(&self) -> PathBuf {
        managed_android_sdk_dir(&self.data_dir)
    }

    pub async fn status(&self) -> SimInstallStatus {
        self.state.lock().await.status.clone()
    }

    pub async fn cancel(&self) -> SimInstallStatus {
        let mut guard = self.state.lock().await;
        if let Some(abort) = guard.abort.take() {
            abort.abort();
        }
        if guard.status.running {
            // Bump run_id so any racing finalizer from the aborted run is a no-op.
            guard.run_id = guard.run_id.wrapping_add(1);
            guard.status.running = false;
            guard.status.stage = SimInstallStage::Cancelled;
            guard.status.message = "Installation cancelled.".into();
            guard.status.updated_at = chrono::Utc::now().to_rfc3339();
            push_log(&mut guard.status, "Installation cancelled by user.");
        }
        guard.status.clone()
    }

    pub async fn start(
        &self,
        req: SimInstallReq,
        runner: Arc<dyn CommandRunner>,
        probe: Arc<dyn HostProbe>,
        adb_lock: Arc<Mutex<()>>,
        before_publish: BeforePublishFn,
    ) -> otto_core::Result<SimInstallStatus> {
        let mut guard = self.state.lock().await;
        if guard.status.running {
            return Err(otto_core::Error::Conflict(format!(
                "another installation is already running ({})",
                guard
                    .status
                    .target
                    .map(SimInstallTarget::label)
                    .unwrap_or("unknown")
            )));
        }
        guard.run_id = guard.run_id.wrapping_add(1);
        let run_id = guard.run_id;
        let initial = SimInstallStatus {
            running: true,
            target: Some(req.target),
            stage: SimInstallStage::Downloading,
            progress_pct: Some(0),
            message: format!("Starting {} installation…", req.target.label()),
            log_tail: vec![format!("Starting {} installation…", req.target.label())],
            error: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        guard.status = initial.clone();

        let state = self.state.clone();
        let data_dir = self.data_dir.clone();
        let fetcher = self.fetcher.clone();
        let extractor = self.extractor.clone();
        let create_default_avds = req.create_default_avds.unwrap_or(true);

        let handle = tokio::spawn(async move {
            let outcome = match req.target {
                SimInstallTarget::IosRuntime => {
                    ios::run_ios_runtime_install(&state, run_id, runner.as_ref()).await
                }
                SimInstallTarget::AndroidPlatformTools | SimInstallTarget::AndroidEmulator => {
                    let ctx = android::AndroidInstallCtx {
                        state: &state,
                        run_id,
                        data_dir: &data_dir,
                        runner: runner.as_ref(),
                        probe: probe.as_ref(),
                        adb_lock: adb_lock.as_ref(),
                        fetcher: fetcher.as_ref(),
                        extractor: extractor.as_ref(),
                        full_emulator: req.target == SimInstallTarget::AndroidEmulator,
                        create_default_avds: req.target == SimInstallTarget::AndroidEmulator
                            && create_default_avds,
                    };
                    android::run_android_install(&ctx).await
                }
            };
            // Invalidate discovery cache synchronously BEFORE publishing terminal Done/Failed.
            before_publish().await;
            let mut g = state.lock().await;
            if g.run_id == run_id && g.status.stage != SimInstallStage::Cancelled {
                g.status.running = false;
                g.abort = None;
                g.status.updated_at = chrono::Utc::now().to_rfc3339();
                match outcome {
                    Ok(msg) => {
                        g.status.stage = SimInstallStage::Done;
                        g.status.progress_pct = Some(100);
                        g.status.message = msg.clone();
                        g.status.error = None;
                        push_log(&mut g.status, &msg);
                    }
                    Err(err) => {
                        g.status.stage = SimInstallStage::Failed;
                        g.status.message = format!("Installation failed: {err}");
                        g.status.error = Some(err.clone());
                        push_log(&mut g.status, &format!("ERROR: {err}"));
                    }
                }
            }
        });
        guard.abort = Some(handle.abort_handle());
        Ok(initial)
    }
}

pub(crate) fn push_log(status: &mut SimInstallStatus, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    status.log_tail.push(trimmed.to_string());
    if status.log_tail.len() > MAX_LOG_TAIL {
        let excess = status.log_tail.len() - MAX_LOG_TAIL;
        status.log_tail.drain(0..excess);
    }
}

pub(crate) async fn update_stage(
    state: &Arc<Mutex<InstallInner>>,
    run_id: u64,
    stage: SimInstallStage,
    pct: Option<u8>,
    msg: impl Into<String>,
) {
    let m = msg.into();
    let mut g = state.lock().await;
    if g.run_id != run_id {
        return;
    }
    g.status.stage = stage;
    if pct.is_some() {
        g.status.progress_pct = pct;
    }
    g.status.message = m.clone();
    g.status.updated_at = chrono::Utc::now().to_rfc3339();
    push_log(&mut g.status, &m);
}

pub(crate) async fn append_log(
    state: &Arc<Mutex<InstallInner>>,
    run_id: u64,
    line: &str,
    pct: Option<u8>,
) {
    let mut g = state.lock().await;
    if g.run_id != run_id {
        return;
    }
    if let Some(p) = pct {
        g.status.progress_pct = Some(p);
    }
    g.status.updated_at = chrono::Utc::now().to_rfc3339();
    push_log(&mut g.status, line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{err_out, ok_out, FakeCommandRunner, FakeHostProbe};
    use tokio::sync::Notify;

    fn noop_hook() -> BeforePublishFn {
        Arc::new(|| Box::pin(async {}))
    }

    struct FakeFetcher {
        declared_len: Option<u64>,
        chunks: Vec<Vec<u8>>,
        block: Option<Arc<Notify>>,
    }

    impl FakeFetcher {
        fn simple(payload: Vec<u8>) -> Self {
            Self {
                declared_len: Some(payload.len() as u64),
                chunks: vec![payload],
                block: None,
            }
        }
    }

    #[async_trait]
    impl ArchiveFetcher for FakeFetcher {
        async fn open(
            &self,
            url: &str,
        ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String> {
            validate_https_download_url(url)?;
            let chunks = self.chunks.clone();
            let block = self.block.clone();
            let s = futures_util::stream::unfold(
                (0usize, chunks, block),
                |(idx, chunks, block)| async move {
                    if let Some(ref notify) = block {
                        notify.notified().await;
                    }
                    if idx < chunks.len() {
                        let c = chunks[idx].clone();
                        Some((Ok(c), (idx + 1, chunks, None)))
                    } else {
                        None
                    }
                },
            );
            Ok((self.declared_len, Box::pin(s)))
        }
    }

    struct FakeExtractor;

    #[async_trait]
    impl ArchiveExtractor for FakeExtractor {
        async fn extract_zip(&self, _archive: &Path, dest: &Path) -> Result<(), String> {
            let bin = dest.join("cmdline-tools/bin");
            tokio::fs::create_dir_all(&bin)
                .await
                .map_err(|e| e.to_string())?;
            tokio::fs::write(bin.join("sdkmanager"), b"#!/bin/sh\n")
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    async fn wait_terminal(mgr: &InstallManager) -> SimInstallStatus {
        for _ in 0..100 {
            let st = mgr.status().await;
            if !st.running {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        panic!("install did not complete in time");
    }

    #[test]
    fn validates_https_download_urls() {
        assert!(validate_https_download_url(ANDROID_CMDLINETOOLS_MAC_URL).is_ok());
        assert!(validate_https_download_url("http://dl.google.com/a.zip").is_err());
        assert!(validate_https_download_url("file:///tmp/a.zip").is_err());
    }

    #[tokio::test]
    async fn discovers_temurin_jdk_when_java_home_unset() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        probe.add_dir(
            "/Library/Java/JavaVirtualMachines",
            vec!["temurin-21.jdk".into()],
        );
        probe.add_executable(
            "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java",
        );
        let found = discover_java_home(&runner, &probe).await;
        assert_eq!(
            found.as_deref(),
            Some("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home")
        );
    }

    #[tokio::test]
    async fn is_writable_dir_probes_actual_write() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(is_writable_dir(tmp.path()).await);
        assert!(!is_writable_dir(&tmp.path().join("does-not-exist")).await);
    }

    #[tokio::test]
    async fn installs_android_emulator_into_managed_data_dir_and_creates_avds() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        let adb_lock = Arc::new(Mutex::new(()));
        probe.set_env(
            "JAVA_HOME",
            "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home",
        );
        probe.add_executable(
            "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home/bin/java",
        );

        let sdk_root = managed_android_sdk_dir(tmp.path());
        let sdk_flag = format!("--sdk_root={}", sdk_root.display());
        let sys_img = default_android_system_image();

        runner.on_exact(
            "sdkmanager",
            &[&sdk_flag, "--licenses"],
            ok_out(b"All SDK package licenses accepted.\n"),
        );
        runner.on_exact(
            "sdkmanager",
            &[&sdk_flag, "platform-tools", "emulator", sys_img],
            ok_out(b"done\n"),
        );
        // WARNING banner only → still creates default AVDs.
        runner.on_exact("emulator", &["-list-avds"], ok_out(b"WARNING | no avds\n"));
        runner.on_exact(
            "avdmanager",
            &[
                "create",
                "avd",
                "--force",
                "-n",
                "Otto_Pixel_Phone",
                "-k",
                sys_img,
                "-d",
                "pixel_8",
            ],
            ok_out(b"Created\n"),
        );
        runner.on_exact(
            "avdmanager",
            &[
                "create",
                "avd",
                "--force",
                "-n",
                "Otto_Pixel_Tablet",
                "-k",
                sys_img,
                "-d",
                "pixel_tablet",
            ],
            ok_out(b"Created\n"),
        );

        let mgr = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(b"PK\x03\x04fakezip".to_vec())),
            Arc::new(FakeExtractor),
        );

        let started = mgr
            .start(
                SimInstallReq {
                    target: SimInstallTarget::AndroidEmulator,
                    create_default_avds: Some(true),
                },
                runner,
                probe,
                adb_lock,
                noop_hook(),
            )
            .await
            .unwrap();
        assert!(started.running);
        let st = wait_terminal(&mgr).await;
        assert_eq!(st.stage, SimInstallStage::Done, "status: {st:?}");
        assert_eq!(st.progress_pct, Some(100));
        assert!(sdk_root
            .join("cmdline-tools/latest/bin/sdkmanager")
            .is_file());
    }

    #[tokio::test]
    async fn single_flight_conflict_and_real_cancellation_preserve_cancelled_state() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        let adb_lock = Arc::new(Mutex::new(()));
        let blocker = Arc::new(Notify::new());

        let mgr = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher {
                declared_len: Some(100),
                chunks: vec![b"abc".to_vec()],
                block: Some(blocker.clone()),
            }),
            Arc::new(FakeExtractor),
        );

        let first = mgr
            .start(
                SimInstallReq {
                    target: SimInstallTarget::AndroidPlatformTools,
                    create_default_avds: Some(false),
                },
                runner.clone(),
                probe.clone(),
                adb_lock.clone(),
                noop_hook(),
            )
            .await
            .unwrap();
        assert!(first.running);

        // Concurrent start returns 409 Conflict.
        let conflict = mgr
            .start(
                SimInstallReq {
                    target: SimInstallTarget::IosRuntime,
                    create_default_avds: None,
                },
                runner,
                probe,
                adb_lock,
                noop_hook(),
            )
            .await;
        assert!(matches!(conflict, Err(otto_core::Error::Conflict(_))));

        // Cancelling the blocked install transitions to Cancelled and stays Cancelled.
        let cancelled = mgr.cancel().await;
        assert!(!cancelled.running);
        assert_eq!(cancelled.stage, SimInstallStage::Cancelled);
        blocker.notify_waiters();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(mgr.status().await.stage, SimInstallStage::Cancelled);
    }

    #[tokio::test]
    async fn enforces_download_byte_caps_and_sdkmanager_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        let adb_lock = Arc::new(Mutex::new(()));

        // 1. Declared Content-Length over MAX_CMDLINETOOLS_BYTES fails.
        let mgr_hdr = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher {
                declared_len: Some(MAX_CMDLINETOOLS_BYTES + 1),
                chunks: vec![],
                block: None,
            }),
            Arc::new(FakeExtractor),
        );
        mgr_hdr
            .start(
                SimInstallReq {
                    target: SimInstallTarget::AndroidPlatformTools,
                    create_default_avds: Some(false),
                },
                runner.clone(),
                probe.clone(),
                adb_lock.clone(),
                noop_hook(),
            )
            .await
            .unwrap();
        let st = wait_terminal(&mgr_hdr).await;
        assert_eq!(st.stage, SimInstallStage::Failed);
        assert!(st.error.as_deref().unwrap_or("").contains("size cap"));

        // 2. sdkmanager failure marks stage Failed.
        let sdk_root = managed_android_sdk_dir(tmp.path());
        let sdk_flag = format!("--sdk_root={}", sdk_root.display());
        runner.on_exact(
            "sdkmanager",
            &[&sdk_flag, "--licenses"],
            err_out(b"Unable to locate a Java Runtime"),
        );
        let mgr_fail = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(b"PK\x03\x04ok".to_vec())),
            Arc::new(FakeExtractor),
        );
        mgr_fail
            .start(
                SimInstallReq {
                    target: SimInstallTarget::AndroidPlatformTools,
                    create_default_avds: Some(false),
                },
                runner,
                probe,
                adb_lock,
                noop_hook(),
            )
            .await
            .unwrap();
        let st = wait_terminal(&mgr_fail).await;
        assert_eq!(st.stage, SimInstallStage::Failed);
        assert!(st.error.as_deref().unwrap_or("").contains("Java 17+"));
    }

    #[tokio::test]
    async fn ios_runtime_install_verifies_exit_code_and_available_runtimes() {
        let tmp = tempfile::tempdir().unwrap();
        let adb_lock = Arc::new(Mutex::new(()));
        let probe = Arc::new(FakeHostProbe::new());

        // 1. Happy path: exit 0 + available iOS runtime in JSON.
        let runner_ok = Arc::new(FakeCommandRunner::new());
        runner_ok.set_log_lines(vec![
            "Downloading iOS 18.2 Simulator: 50%".into(),
            "Installed iOS 18.2 Simulator".into(),
        ]);
        runner_ok.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            ok_out(
                br#"{"runtimes":[{"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-18-2","name":"iOS 18.2","isAvailable":true}]}"#,
            ),
        );
        let mgr_ok = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(vec![])),
            Arc::new(FakeExtractor),
        );
        mgr_ok
            .start(
                SimInstallReq {
                    target: SimInstallTarget::IosRuntime,
                    create_default_avds: None,
                },
                runner_ok,
                probe.clone(),
                adb_lock.clone(),
                noop_hook(),
            )
            .await
            .unwrap();
        let st = wait_terminal(&mgr_ok).await;
        assert_eq!(st.stage, SimInstallStage::Done);
        assert!(st.log_tail.iter().any(|l| l.contains("50%")));

        // 2. Non-zero exit from xcodebuild must fail.
        let runner_exit = Arc::new(FakeCommandRunner::new());
        runner_exit.set_log_lines(vec!["network timeout".into()]);
        runner_exit.set_log_exit_code(70);
        let mgr_exit = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(vec![])),
            Arc::new(FakeExtractor),
        );
        mgr_exit
            .start(
                SimInstallReq {
                    target: SimInstallTarget::IosRuntime,
                    create_default_avds: None,
                },
                runner_exit,
                probe.clone(),
                adb_lock.clone(),
                noop_hook(),
            )
            .await
            .unwrap();
        let st = wait_terminal(&mgr_exit).await;
        assert_eq!(st.stage, SimInstallStage::Failed);
        assert!(st.error.as_deref().unwrap_or("").contains("70"));

        // 3. Exit 0 but `{"runtimes":[]}` must fail.
        let runner_empty = Arc::new(FakeCommandRunner::new());
        runner_empty.set_log_lines(vec!["Done".into()]);
        runner_empty.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            ok_out(br#"{"runtimes":[]}"#),
        );
        let mgr_empty = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(vec![])),
            Arc::new(FakeExtractor),
        );
        mgr_empty
            .start(
                SimInstallReq {
                    target: SimInstallTarget::IosRuntime,
                    create_default_avds: None,
                },
                runner_empty,
                probe,
                adb_lock,
                noop_hook(),
            )
            .await
            .unwrap();
        let st = wait_terminal(&mgr_empty).await;
        assert_eq!(st.stage, SimInstallStage::Failed);
        assert!(st
            .error
            .as_deref()
            .unwrap_or("")
            .contains("no available iOS Simulator runtime"));
    }

    #[tokio::test]
    async fn android_install_fails_when_requested_default_avds_all_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = Arc::new(FakeCommandRunner::new());
        let probe = Arc::new(FakeHostProbe::new());
        let adb_lock = Arc::new(Mutex::new(()));
        probe.set_env(
            "JAVA_HOME",
            "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home",
        );
        probe.add_executable(
            "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home/bin/java",
        );

        let sdk_root = managed_android_sdk_dir(tmp.path());
        let sdk_flag = format!("--sdk_root={}", sdk_root.display());
        let sys_img = default_android_system_image();

        runner.on_exact(
            "sdkmanager",
            &[&sdk_flag, "--licenses"],
            ok_out(b"All SDK package licenses accepted.\n"),
        );
        runner.on_exact(
            "sdkmanager",
            &[&sdk_flag, "platform-tools", "emulator", sys_img],
            ok_out(b"done\n"),
        );
        runner.on_exact("emulator", &["-list-avds"], ok_out(b""));
        runner.on(|spec| {
            if spec.program.ends_with("avdmanager") {
                Some(err_out(b"Error: Package path is not valid"))
            } else {
                None
            }
        });

        let mgr = InstallManager::with_deps(
            tmp.path().to_path_buf(),
            Arc::new(FakeFetcher::simple(b"PK\x03\x04ok".to_vec())),
            Arc::new(FakeExtractor),
        );
        mgr.start(
            SimInstallReq {
                target: SimInstallTarget::AndroidEmulator,
                create_default_avds: Some(true),
            },
            runner,
            probe,
            adb_lock,
            noop_hook(),
        )
        .await
        .unwrap();

        let st = wait_terminal(&mgr).await;
        assert_eq!(st.stage, SimInstallStage::Failed);
        assert!(
            st.error
                .as_deref()
                .unwrap_or("")
                .contains("creating default AVDs failed"),
            "expected AVD failure surfaced, got {:?}",
            st.error
        );
        // Verify per-run staging directory under cmdline-tools was cleaned up on drop.
        let ct_dir = sdk_root.join("cmdline-tools");
        let mut rd = tokio::fs::read_dir(&ct_dir).await.unwrap();
        let mut leftovers = Vec::new();
        while let Some(entry) = rd.next_entry().await.unwrap() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".otto-staging-")
            {
                leftovers.push(entry.file_name());
            }
        }
        assert!(leftovers.is_empty(), "staging dirs must be removed on drop");
    }
}

//! Chrome for Testing: pinned builds, install detection, and the one-time
//! download (only ever started by an explicit user action —
//! `POST /browser/live/install`). Nothing here runs at daemon boot.
//!
//! Integrity: every pin carries the exact archive size and a sha256; the
//! archive is hashed while it streams and verified BEFORE it is extracted.
//! A build whose sha256 is not pinned refuses to install (fail closed) unless
//! the operator supplies the pin via `OTTO_CHROME_SHA256_CHROME` /
//! `OTTO_CHROME_SHA256_HEADLESS_SHELL` (64 hex chars).
//!
//! Layout: `<data>/browser/chromium/<version>/<build>/…` plus a
//! `.otto-installed` marker holding the verified sha256.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::types::ChromeBuild;

/// The Chrome for Testing version Playwright 1.61 pins (`browsers.json`:
/// chromium + chromium-headless-shell revision 1228 → 149.0.7827.55).
pub const CFT_VERSION: &str = "149.0.7827.55";

/// Refuse any archive larger than this, whatever the server claims.
pub const MAX_DOWNLOAD_BYTES: u64 = 400 * 1024 * 1024;

/// Marker written into a build dir once it is verified + extracted.
pub const INSTALLED_MARKER: &str = ".otto-installed";

/// Explicit binary override (dev/test escape hatch, like `OTTO_LIGHTPANDA_BIN`).
pub const ENV_BIN: &str = "OTTO_CHROME_BIN";

/// One pinned downloadable build.
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub build: ChromeBuild,
    pub platform: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    /// Lowercase hex sha256 of the archive; empty = not pinned in this build.
    pub sha256: &'static str,
    /// Exact archive size in bytes (from the CfT bucket's object metadata).
    pub size: u64,
    /// The executable, relative to the extracted build dir.
    pub exe: &'static str,
    pub label: &'static str,
}

/// Pinned builds. Sizes (and, for cross-checking, the bucket's MD5s:
/// chrome `buJBBrGIX2ireeELGVumkA==`, headless shell
/// `nqCm0W5G3MaF1GLSENeBFg==`) come from the object metadata of
/// `storage.googleapis.com/chrome-for-testing-public/149.0.7827.55/mac-arm64/`.
/// The CfT JSON publishes no sha256 and this build was prepared without
/// downloading the archives, so the sha256 pins are EMPTY: fill them from a
/// one-time `curl -sL <url> | shasum -a 256` on a trusted machine (or set the
/// env override) — until then install is refused.
pub const PINS: &[Pin] = &[
    Pin {
        build: ChromeBuild::Chrome,
        platform: "mac-arm64",
        version: CFT_VERSION,
        url: "https://storage.googleapis.com/chrome-for-testing-public/149.0.7827.55/mac-arm64/chrome-mac-arm64.zip",
        sha256: "",
        size: 179_277_110,
        exe: "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        label: "Chrome for Testing — full browser (~180 MB)",
    },
    Pin {
        build: ChromeBuild::ChromeHeadlessShell,
        platform: "mac-arm64",
        version: CFT_VERSION,
        url: "https://storage.googleapis.com/chrome-for-testing-public/149.0.7827.55/mac-arm64/chrome-headless-shell-mac-arm64.zip",
        sha256: "",
        size: 98_043_456,
        exe: "chrome-headless-shell-mac-arm64/chrome-headless-shell",
        label: "Chrome Headless Shell — lighter, headless only (~98 MB)",
    },
];

/// The platform key of this daemon host, when a pin exists for it.
pub fn current_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("mac-arm64")
    } else {
        None
    }
}

pub fn pin_for(build: ChromeBuild, platform: &str) -> Option<&'static Pin> {
    PINS.iter()
        .find(|p| p.build == build && p.platform == platform)
}

fn sha_env_var(build: ChromeBuild) -> &'static str {
    match build {
        ChromeBuild::Chrome => "OTTO_CHROME_SHA256_CHROME",
        ChromeBuild::ChromeHeadlessShell => "OTTO_CHROME_SHA256_HEADLESS_SHELL",
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The sha256 the archive must match: the compiled-in pin, else a
/// well-formed env override, else `None` (install refused).
pub fn effective_sha256(pin: &Pin) -> Option<String> {
    effective_sha256_with(pin, std::env::var(sha_env_var(pin.build)).ok().as_deref())
}

fn effective_sha256_with(pin: &Pin, env: Option<&str>) -> Option<String> {
    if is_sha256_hex(pin.sha256) {
        return Some(pin.sha256.to_ascii_lowercase());
    }
    env.map(str::trim)
        .filter(|s| is_sha256_hex(s))
        .map(str::to_ascii_lowercase)
}

pub fn chromium_root(data_dir: &Path) -> PathBuf {
    data_dir.join("browser").join("chromium")
}

pub fn build_dir(data_dir: &Path, pin: &Pin) -> PathBuf {
    chromium_root(data_dir)
        .join(pin.version)
        .join(pin.build.as_str())
}

pub fn managed_exe(data_dir: &Path, pin: &Pin) -> PathBuf {
    build_dir(data_dir, pin).join(pin.exe)
}

/// Installed = the executable exists AND the verified-install marker is
/// present (a half-extracted dir from a crash doesn't count).
pub fn is_installed(data_dir: &Path, pin: &Pin) -> bool {
    managed_exe(data_dir, pin).is_file() && build_dir(data_dir, pin).join(INSTALLED_MARKER).is_file()
}

/// Where a binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinarySource {
    Managed,
    Env,
}

/// The executable to launch for `build`: `OTTO_CHROME_BIN` first (when it
/// names a real file), else the managed install.
pub fn resolve_binary(data_dir: &Path, build: ChromeBuild) -> Option<(PathBuf, BinarySource)> {
    if let Some(p) = std::env::var(ENV_BIN)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let pb = PathBuf::from(p);
        // An explicit path is authoritative — a wrong one is "not installed",
        // never a silent fallback (same rule as `Lightpanda::locate`).
        return pb.is_file().then_some((pb, BinarySource::Env));
    }
    let pin = pin_for(build, current_platform()?)?;
    is_installed(data_dir, pin).then(|| (managed_exe(data_dir, pin), BinarySource::Managed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Downloading,
    Verifying,
    Extracting,
    Installed,
    Failed,
}

impl InstallState {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallState::Downloading => "downloading",
            InstallState::Verifying => "verifying",
            InstallState::Extracting => "extracting",
            InstallState::Installed => "installed",
            InstallState::Failed => "failed",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, InstallState::Installed | InstallState::Failed)
    }
}

/// `BrowserEngineInstallJob` on the wire.
#[derive(Debug, Clone, Serialize)]
pub struct InstallJob {
    pub build: ChromeBuild,
    pub version: String,
    pub state: InstallState,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

impl InstallJob {
    pub fn new(pin: &Pin) -> Self {
        Self {
            build: pin.build,
            version: pin.version.to_string(),
            state: InstallState::Downloading,
            received_bytes: 0,
            total_bytes: Some(pin.size),
            error: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
        }
    }
}

/// Byte source for the archive (the real one is a netguarded `reqwest`
/// stream; tests use a fake).
#[async_trait::async_trait]
pub trait Fetcher: Send + Sync {
    /// `(content length if known, chunk stream)`.
    async fn open(
        &self,
        url: &str,
    ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String>;
}

/// Unpacks a verified archive into a directory.
pub trait Extractor: Send + Sync {
    fn extract(&self, archive: &Path, dest: &Path) -> Result<(), String>;
}

/// The real fetcher: the SSRF-guarded client (the CfT bucket is public, so the
/// guard never gets in the way; it just keeps redirects honest).
pub struct ReqwestFetcher {
    client: reqwest::Client,
}

impl ReqwestFetcher {
    pub fn new() -> Self {
        let client = otto_netguard::guarded_client_builder()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

impl Default for ReqwestFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Fetcher for ReqwestFetcher {
    async fn open(
        &self,
        url: &str,
    ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: HTTP {}", resp.status()));
        }
        let len = resp.content_length();
        let stream = resp
            .bytes_stream()
            .map(|r| r.map(|b| b.to_vec()).map_err(|e| e.to_string()))
            .boxed();
        Ok((len, stream))
    }
}

/// `ditto -x -k` — preserves the app bundle's symlinks, permissions and
/// code signature (a generic unzip mangles `Versions/Current` links).
pub struct DittoExtractor;

impl Extractor for DittoExtractor {
    fn extract(&self, archive: &Path, dest: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
        let out = std::process::Command::new("/usr/bin/ditto")
            .arg("-x")
            .arg("-k")
            .arg(archive)
            .arg(dest)
            .output()
            .map_err(|e| format!("run ditto: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "ditto failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }
}

/// How often a `downloading` progress tick may be emitted.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Download → verify → extract `pin` into its build dir. `progress` is called
/// on every state change and at most every [`PROGRESS_EVERY`] while bytes
/// flow. Returns the executable path. On any failure the partial archive and
/// staging dir are removed and the build dir is left untouched.
pub async fn install(
    data_dir: &Path,
    pin: &Pin,
    sha256: &str,
    fetcher: &dyn Fetcher,
    extractor: Arc<dyn Extractor>,
    progress: &(dyn Fn(&InstallJob) + Send + Sync),
) -> Result<PathBuf, String> {
    let mut job = InstallJob::new(pin);
    progress(&job);
    let version_dir = chromium_root(data_dir).join(pin.version);
    tokio::fs::create_dir_all(&version_dir)
        .await
        .map_err(|e| format!("create {}: {e}", version_dir.display()))?;
    let part = version_dir.join(format!("{}.zip.part", pin.build.as_str()));
    let staging = version_dir.join(format!("{}.staging", pin.build.as_str()));

    let result = install_inner(
        data_dir, pin, sha256, fetcher, extractor, progress, &mut job, &part, &staging,
    )
    .await;
    let _ = tokio::fs::remove_file(&part).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    match result {
        Ok(exe) => {
            job.state = InstallState::Installed;
            job.finished_at = Some(chrono::Utc::now().to_rfc3339());
            progress(&job);
            Ok(exe)
        }
        Err(e) => {
            job.state = InstallState::Failed;
            job.error = Some(e.clone());
            job.finished_at = Some(chrono::Utc::now().to_rfc3339());
            progress(&job);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn install_inner(
    data_dir: &Path,
    pin: &Pin,
    sha256: &str,
    fetcher: &dyn Fetcher,
    extractor: Arc<dyn Extractor>,
    progress: &(dyn Fn(&InstallJob) + Send + Sync),
    job: &mut InstallJob,
    part: &Path,
    staging: &Path,
) -> Result<PathBuf, String> {
    let (len, mut stream) = fetcher.open(pin.url).await?;
    if let Some(n) = len {
        if n != pin.size || n > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "unexpected archive size {n} (pinned {})",
                pin.size
            ));
        }
    }
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|e| format!("create {}: {e}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut last_tick = Instant::now();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(60), stream.next())
            .await
            .map_err(|_| "download stalled (no data for 60s)".to_string())?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| format!("download interrupted: {e}"))?;
        job.received_bytes += chunk.len() as u64;
        if job.received_bytes > pin.size || job.received_bytes > MAX_DOWNLOAD_BYTES {
            return Err("archive is larger than the pinned size".into());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write archive: {e}"))?;
        if last_tick.elapsed() >= PROGRESS_EVERY {
            last_tick = Instant::now();
            progress(job);
        }
    }
    file.flush().await.map_err(|e| format!("flush archive: {e}"))?;
    drop(file);
    progress(job);

    job.state = InstallState::Verifying;
    progress(job);
    if job.received_bytes != pin.size {
        return Err(format!(
            "archive is truncated ({} of {} bytes)",
            job.received_bytes, pin.size
        ));
    }
    let digest = hex::encode(hasher.finalize());
    if !digest.eq_ignore_ascii_case(sha256) {
        return Err("checksum mismatch — the download does not match the pinned sha256".into());
    }

    job.state = InstallState::Extracting;
    progress(job);
    let _ = tokio::fs::remove_dir_all(staging).await;
    let dest = staging.to_path_buf();
    // Blocking (ditto): off the async workers.
    {
        let archive = part.to_path_buf();
        let dest = dest.clone();
        tokio::task::spawn_blocking(move || extractor.extract(&archive, &dest))
            .await
            .map_err(|e| format!("extract task failed: {e}"))??;
    }
    if !dest.join(pin.exe).is_file() {
        return Err("archive does not contain the expected executable".into());
    }
    std::fs::write(dest.join(INSTALLED_MARKER), &digest)
        .map_err(|e| format!("write install marker: {e}"))?;
    let final_dir = build_dir(data_dir, pin);
    let _ = tokio::fs::remove_dir_all(&final_dir).await;
    tokio::fs::rename(&dest, &final_dir)
        .await
        .map_err(|e| format!("move build into place: {e}"))?;
    Ok(managed_exe(data_dir, pin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeFetcher {
        body: Vec<u8>,
        claimed_len: Option<u64>,
    }

    #[async_trait::async_trait]
    impl Fetcher for FakeFetcher {
        async fn open(
            &self,
            _url: &str,
        ) -> Result<(Option<u64>, BoxStream<'static, Result<Vec<u8>, String>>), String> {
            let chunks: Vec<Result<Vec<u8>, String>> =
                self.body.chunks(7).map(|c| Ok(c.to_vec())).collect();
            Ok((self.claimed_len, futures_util::stream::iter(chunks).boxed()))
        }
    }

    /// Writes the pinned executable path into the staging dir.
    struct FakeExtractor;

    impl Extractor for FakeExtractor {
        fn extract(&self, _archive: &Path, dest: &Path) -> Result<(), String> {
            let exe = dest.join("bin/chrome");
            std::fs::create_dir_all(exe.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&exe, b"#!/bin/sh\n").map_err(|e| e.to_string())
        }
    }

    fn fake_pin(size: u64) -> Pin {
        Pin {
            build: ChromeBuild::ChromeHeadlessShell,
            platform: "test",
            version: "1.2.3",
            url: "https://example.invalid/chrome.zip",
            sha256: "",
            size,
            exe: "bin/chrome",
            label: "test",
        }
    }

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn installs_when_size_and_sha256_match() {
        let tmp = tempfile::tempdir().unwrap();
        let body = b"pretend this is a zip archive".to_vec();
        let pin = fake_pin(body.len() as u64);
        let states = Mutex::new(Vec::new());
        let exe = install(
            tmp.path(),
            &pin,
            &sha(&body),
            &FakeFetcher {
                body: body.clone(),
                claimed_len: Some(body.len() as u64),
            },
            Arc::new(FakeExtractor),
            &|j: &InstallJob| states.lock().unwrap().push(j.state),
        )
        .await
        .unwrap();
        assert!(exe.is_file());
        assert!(is_installed(tmp.path(), &pin));
        let s = states.lock().unwrap().clone();
        assert_eq!(s.first(), Some(&InstallState::Downloading));
        assert!(s.contains(&InstallState::Verifying));
        assert!(s.contains(&InstallState::Extracting));
        assert_eq!(s.last(), Some(&InstallState::Installed));
        // No partial archive left behind.
        assert!(!tmp
            .path()
            .join("browser/chromium/1.2.3/chrome-headless-shell.zip.part")
            .exists());
    }

    #[tokio::test]
    async fn a_checksum_mismatch_installs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let body = b"tampered archive bytes".to_vec();
        let pin = fake_pin(body.len() as u64);
        let last = Mutex::new(None);
        let err = install(
            tmp.path(),
            &pin,
            &sha(b"the real archive"),
            &FakeFetcher {
                body,
                claimed_len: None,
            },
            Arc::new(FakeExtractor),
            &|j: &InstallJob| *last.lock().unwrap() = Some(j.clone()),
        )
        .await
        .unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(!is_installed(tmp.path(), &pin));
        assert!(!build_dir(tmp.path(), &pin).exists());
        let last = last.lock().unwrap().clone().unwrap();
        assert_eq!(last.state, InstallState::Failed);
        assert!(last.error.is_some());
    }

    #[tokio::test]
    async fn wrong_sizes_are_refused_before_and_during_the_download() {
        let tmp = tempfile::tempdir().unwrap();
        let body = vec![7u8; 64];
        // Server claims a different size than pinned → refused up front.
        let pin = fake_pin(64);
        let err = install(
            tmp.path(),
            &pin,
            &sha(&body),
            &FakeFetcher {
                body: body.clone(),
                claimed_len: Some(65),
            },
            Arc::new(FakeExtractor),
            &|_: &InstallJob| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("unexpected archive size"), "{err}");
        // More bytes than pinned (no length header) → aborted mid-stream.
        let small = fake_pin(10);
        let err = install(
            tmp.path(),
            &small,
            &sha(&body),
            &FakeFetcher {
                body: body.clone(),
                claimed_len: None,
            },
            Arc::new(FakeExtractor),
            &|_: &InstallJob| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("larger than the pinned size"), "{err}");
        // Fewer bytes → truncated.
        let big = fake_pin(100);
        let err = install(
            tmp.path(),
            &big,
            &sha(&body),
            &FakeFetcher {
                body,
                claimed_len: None,
            },
            Arc::new(FakeExtractor),
            &|_: &InstallJob| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("truncated"), "{err}");
    }

    #[test]
    fn pins_cover_both_builds_with_exact_sizes() {
        for b in ChromeBuild::ALL {
            let p = pin_for(b, "mac-arm64").expect("pinned");
            assert_eq!(p.version, CFT_VERSION);
            assert!(p.url.starts_with("https://storage.googleapis.com/chrome-for-testing-public/"));
            assert!(p.url.contains(CFT_VERSION));
            assert!(p.size > 50_000_000 && p.size < MAX_DOWNLOAD_BYTES);
            // Either a real sha256 or empty (install refused) — never junk.
            assert!(p.sha256.is_empty() || is_sha256_hex(p.sha256));
        }
        assert!(pin_for(ChromeBuild::Chrome, "linux64").is_none());
    }

    #[test]
    fn sha_pin_falls_back_to_a_well_formed_env_override_only() {
        let pin = fake_pin(1);
        assert_eq!(effective_sha256_with(&pin, None), None);
        assert_eq!(effective_sha256_with(&pin, Some("abc")), None);
        let good = "A".repeat(64);
        assert_eq!(
            effective_sha256_with(&pin, Some(&good)),
            Some("a".repeat(64))
        );
        let pinned = Pin {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..pin
        };
        // The compiled-in pin wins over the env.
        assert_eq!(
            effective_sha256_with(&pinned, Some(&good)),
            Some("0".repeat(64))
        );
    }

    #[test]
    fn a_half_extracted_dir_is_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let pin = fake_pin(1);
        let exe = managed_exe(tmp.path(), &pin);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"x").unwrap();
        assert!(!is_installed(tmp.path(), &pin), "no marker yet");
        std::fs::write(build_dir(tmp.path(), &pin).join(INSTALLED_MARKER), b"sha").unwrap();
        assert!(is_installed(tmp.path(), &pin));
    }
}

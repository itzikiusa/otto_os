//! In-app provisioning of the embedded headless Kiri runtime into
//! `<data_dir>/bin/kiri` and `<data_dir>/runtime/guest/` (ZERO separate `.app`).
//!
//! Sources supported in order:
//! 1. Local directory (`req.source_dir` or `OTTO_KIRI_SOURCE_DIR`) containing
//!    a built/staged `kiri` binary and optional `guest/` kernel+initramfs files.
//! 2. Explicit `.tar.gz` URL (`req.download_url` or `OTTO_KIRI_RELEASE_URL`)
//!    with optional SHA-256 verification.
//! 3. Upstream Kiri release feed (`OTTO_KIRI_FEED_URL` or
//!    `https://download.runkiri.com/latest-mac-{arm64,x86_64}.json`), which
//!    supplies `{ version, url, sha256 }` for the signed tarball; Otto extracts
//!    ONLY the headless `kiri` binary + `guest/` assets and discards any `.app`
//!    wrapper.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use flate2::read::GzDecoder;
use otto_core::{Error, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::fs;

const DEFAULT_FEED_ARM64: &str = "https://download.runkiri.com/latest-mac-arm64.json";
const DEFAULT_FEED_X86_64: &str = "https://download.runkiri.com/latest-mac-x86_64.json";

const FEED_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_FEED_BYTES: usize = 64 * 1024;
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
const MAX_EXTRACTED_TOTAL_BYTES: usize = 192 * 1024 * 1024;
const MAX_TAR_ENTRIES: usize = 4096;

const GUEST_ASSET_NAMES: &[&str] = &[
    "Image",
    "bzImage",
    "initramfs.cpio",
    "data.img.tgz",
    "data.img",
];

const KIRI_ENTITLEMENTS_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.virtualization</key>
    <true/>
</dict>
</plist>
"#;

/// Per-`data_dir` install progress / last-error snapshot.
#[derive(Debug, Clone, Default)]
pub struct InstallState {
    pub installing: bool,
    pub progress: Option<String>,
    pub error: Option<String>,
}

static INSTALL_STATES: OnceLock<Mutex<HashMap<PathBuf, InstallState>>> = OnceLock::new();

fn state_map() -> &'static Mutex<HashMap<PathBuf, InstallState>> {
    INSTALL_STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read the current install state for `data_dir`.
pub fn current_install_state(data_dir: &Path) -> InstallState {
    state_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(data_dir)
        .cloned()
        .unwrap_or_default()
}

struct InstallGuard {
    data_dir: PathBuf,
    finished: bool,
}

impl InstallGuard {
    fn acquire(data_dir: &Path) -> Result<Self> {
        let mut map = state_map().lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(data_dir.to_path_buf()).or_default();
        if entry.installing {
            return Err(Error::Conflict(
                "an embedded Kiri runtime install is already in progress".into(),
            ));
        }
        entry.installing = true;
        entry.progress = Some("Preparing embedded Kiri install…".into());
        entry.error = None;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            finished: false,
        })
    }

    fn set_progress(&self, msg: impl Into<String>) {
        let mut map = state_map().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = map.get_mut(&self.data_dir) {
            entry.progress = Some(msg.into());
        }
    }

    fn finish_ok(mut self) {
        self.finished = true;
        let mut map = state_map().lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(self.data_dir.clone()).or_default();
        entry.installing = false;
        entry.progress = None;
        entry.error = None;
    }

    fn finish_err(mut self, err: &Error) {
        self.finished = true;
        let mut map = state_map().lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(self.data_dir.clone()).or_default();
        entry.installing = false;
        entry.progress = None;
        entry.error = Some(err.public_message());
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if !self.finished {
            let mut map = state_map().lock().unwrap_or_else(|p| p.into_inner());
            if let Some(entry) = map.get_mut(&self.data_dir) {
                entry.installing = false;
                entry.progress = None;
            }
        }
    }
}

/// Extracted embedded Kiri payload from a tarball.
#[derive(Debug)]
pub struct ExtractedKiri {
    pub binary: Vec<u8>,
    pub guest_files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Deserialize)]
struct ReleaseFeed {
    url: String,
    #[serde(default)]
    sha256: Option<String>,
}

/// Provision the embedded `kiri` binary into `<data_dir>/bin/kiri` and guest
/// kernel/initramfs assets into `<data_dir>/runtime/guest/`.
pub async fn provision_kiri(
    data_dir: &Path,
    source_dir: Option<&str>,
    download_url: Option<&str>,
    expected_sha256: Option<&str>,
    allow_env_defaults: bool,
) -> Result<PathBuf> {
    let guard = InstallGuard::acquire(data_dir)?;
    let res = provision_inner(
        data_dir,
        source_dir,
        download_url,
        expected_sha256,
        allow_env_defaults,
        &guard,
    )
    .await;
    match res {
        Ok(bin_path) => {
            guard.finish_ok();
            Ok(bin_path)
        }
        Err(err) => {
            guard.finish_err(&err);
            Err(err)
        }
    }
}

async fn provision_inner(
    data_dir: &Path,
    source_dir: Option<&str>,
    download_url: Option<&str>,
    expected_sha256: Option<&str>,
    allow_env_defaults: bool,
    guard: &InstallGuard,
) -> Result<PathBuf> {
    // 1. Local staged/build directory?
    let local_dir = source_dir
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            if allow_env_defaults {
                std::env::var("OTTO_KIRI_SOURCE_DIR")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            } else {
                None
            }
        });

    if let Some(dir) = local_dir {
        guard.set_progress("Copying embedded Kiri from local staging directory…");
        return install_from_local_dir(data_dir, &dir).await;
    }

    // 2. Explicit download URL or upstream release feed.
    let explicit_url = download_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if allow_env_defaults {
                std::env::var("OTTO_KIRI_RELEASE_URL")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        });

    let (archive_url, checksum) = match explicit_url {
        Some(url) => (
            validate_download_url(&url)?,
            normalize_sha256(expected_sha256)?,
        ),
        None => {
            guard.set_progress("Fetching Kiri release metadata…");
            let feed_url = if allow_env_defaults {
                std::env::var("OTTO_KIRI_FEED_URL")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| default_feed_url().to_string())
            } else {
                default_feed_url().to_string()
            };
            let feed = fetch_release_feed(&feed_url).await?;
            let feed_sha = normalize_sha256(expected_sha256.or(feed.sha256.as_deref()))?;
            (validate_download_url(&feed.url)?, feed_sha)
        }
    };

    guard.set_progress("Downloading embedded Kiri runtime…");
    let archive_bytes = download_capped(&archive_url, DOWNLOAD_TIMEOUT, MAX_ARCHIVE_BYTES).await?;

    if let Some(expected) = checksum {
        guard.set_progress("Verifying SHA-256 checksum…");
        let actual = hex::encode(Sha256::digest(&archive_bytes));
        if actual != expected {
            return Err(Error::Upstream(format!(
                "Kiri archive SHA-256 mismatch: expected {expected}, got {actual}"
            )));
        }
    }

    guard.set_progress("Extracting headless Kiri runtime and guest assets…");
    let extracted = tokio::task::spawn_blocking(move || extract_kiri_tar_gz(&archive_bytes))
        .await
        .map_err(|e| Error::Internal(format!("tar extraction task failed: {e}")))??;

    guard.set_progress("Installing embedded Kiri into Otto data directory…");
    let installed_bin = install_extracted_files(data_dir, &extracted).await?;

    guard.set_progress("Verifying virtualization code signature…");
    ensure_macos_virtualization_signature(&installed_bin).await?;

    Ok(installed_bin)
}

fn default_feed_url() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        DEFAULT_FEED_X86_64
    } else {
        DEFAULT_FEED_ARM64
    }
}

fn validate_download_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("download_url cannot be empty".into()));
    }
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| Error::Invalid(format!("invalid download_url: {e}")))?;
    match parsed.scheme() {
        "https" => Ok(trimmed.to_string()),
        // Allow http:// only for loopback (used by local staging servers & unit tests).
        "http"
            if matches!(
                parsed.host_str(),
                Some("127.0.0.1") | Some("localhost") | Some("::1")
            ) =>
        {
            Ok(trimmed.to_string())
        }
        other => Err(Error::Invalid(format!(
            "download_url must use https:// (got {other}://)"
        ))),
    }
}

fn normalize_sha256(raw: Option<&str>) -> Result<Option<String>> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let lower = s.to_ascii_lowercase();
    if lower.len() != 64 || !lower.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid(
            "sha256 must be a 64-character hex string".into(),
        ));
    }
    Ok(Some(lower))
}

async fn fetch_release_feed(url: &str) -> Result<ReleaseFeed> {
    let valid = validate_download_url(url)?;
    let bytes = download_capped(&valid, FEED_TIMEOUT, MAX_FEED_BYTES).await?;
    serde_json::from_slice::<ReleaseFeed>(&bytes)
        .map_err(|e| Error::Upstream(format!("invalid Kiri release feed JSON: {e}")))
}

async fn download_capped(url: &str, timeout: Duration, max_bytes: usize) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Otto-Containers/1.0")
        .build()
        .map_err(|e| Error::Internal(format!("build HTTP client: {e}")))?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("download failed ({url}): {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Upstream(format!(
            "download returned HTTP {} ({url})",
            resp.status().as_u16()
        )));
    }
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(Error::PayloadTooLarge(format!(
                "download size {len} exceeds cap ({max_bytes} bytes)"
            )));
        }
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Upstream(format!("read download chunk: {e}")))?
    {
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err(Error::PayloadTooLarge(format!(
                "download exceeded cap ({max_bytes} bytes)"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

async fn install_from_local_dir(data_dir: &Path, source_dir: &Path) -> Result<PathBuf> {
    if !source_dir.is_absolute()
        || source_dir
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::Invalid(format!(
            "source_dir must be a clean absolute path: {}",
            source_dir.display()
        )));
    }
    let bin_candidates = [
        source_dir.join("kiri"),
        source_dir.join("bin/kiri"),
        source_dir.join("Contents/MacOS/kiri"),
        source_dir.join("Kiri.app/Contents/MacOS/kiri"),
        source_dir.join("target/release/kiri"),
    ];
    let mut found_bin = None;
    for cand in bin_candidates {
        if fs::metadata(&cand)
            .await
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            found_bin = Some(cand);
            break;
        }
    }
    let src_bin = found_bin.ok_or_else(|| {
        Error::NotFound(format!(
            "no `kiri` binary found under {}",
            source_dir.display()
        ))
    })?;

    let binary = fs::read(&src_bin)
        .await
        .map_err(|e| Error::Internal(format!("read {}: {e}", src_bin.display())))?;

    let guest_candidates = [
        source_dir.join("guest"),
        source_dir.join("runtime/guest"),
        source_dir.join("Contents/Resources/guest"),
        source_dir.join("Kiri.app/Contents/Resources/guest"),
        source_dir.join("guest/out"),
    ];
    let mut guest_files = Vec::new();
    for gdir in guest_candidates {
        if fs::metadata(&gdir)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            for &name in GUEST_ASSET_NAMES {
                let p = gdir.join(name);
                if fs::metadata(&p).await.map(|m| m.is_file()).unwrap_or(false) {
                    let bytes = fs::read(&p)
                        .await
                        .map_err(|e| Error::Internal(format!("read {}: {e}", p.display())))?;
                    guest_files.push((name.to_string(), bytes));
                }
            }
            if !guest_files.is_empty() {
                break;
            }
        }
    }

    let extracted = ExtractedKiri {
        binary,
        guest_files,
    };
    let installed_bin = install_extracted_files(data_dir, &extracted).await?;
    ensure_macos_virtualization_signature(&installed_bin).await?;
    Ok(installed_bin)
}

async fn install_extracted_files(data_dir: &Path, extracted: &ExtractedKiri) -> Result<PathBuf> {
    let bin_dir = data_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| Error::Internal(format!("create {}: {e}", bin_dir.display())))?;
    let bin_target = bin_dir.join("kiri");
    atomic_write(&bin_target, &extracted.binary, true).await?;

    if !extracted.guest_files.is_empty() {
        let guest_dir = data_dir.join("runtime").join("guest");
        fs::create_dir_all(&guest_dir)
            .await
            .map_err(|e| Error::Internal(format!("create {}: {e}", guest_dir.display())))?;
        for (name, bytes) in &extracted.guest_files {
            atomic_write(&guest_dir.join(name), bytes, false).await?;
        }
    }
    Ok(bin_target)
}

async fn atomic_write(target: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::Internal("atomic_write target missing parent".into()))?;
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("asset");
    let tmp = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));
    fs::write(&tmp, bytes)
        .await
        .map_err(|e| Error::Internal(format!("write {}: {e}", tmp.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
            .await
            .map_err(|e| Error::Internal(format!("chmod {}: {e}", tmp.display())))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    fs::rename(&tmp, target)
        .await
        .map_err(|e| Error::Internal(format!("rename {}: {e}", target.display())))?;
    Ok(())
}

/// On macOS, keep an existing `com.apple.security.virtualization` signature
/// only when it also verifies (a notarized upstream release); otherwise
/// apply an ad-hoc signature with the virtualization entitlement, and fail
/// when even that does not verify. The upstream binary is signed inside
/// `Kiri.app`: taken out of it, its signature names an Info.plist it no
/// longer has, still lists the entitlement, and macOS kills it at exec.
pub(crate) async fn ensure_macos_virtualization_signature(bin_path: &Path) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bin_path;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        sign_for_virtualization(Path::new("/usr/bin/codesign"), bin_path).await
    }
}

/// Keep a signature only when it carries the entitlement AND verifies.
fn keep_signature(entitled: bool, verifies: bool) -> bool {
    entitled && verifies
}

#[cfg(target_os = "macos")]
async fn signature_ok(codesign: &Path, bin_path: &Path) -> bool {
    let run = |args: &[&str]| {
        let mut c = tokio::process::Command::new(codesign);
        c.args(args).arg(bin_path);
        async move { c.output().await.ok() }
    };
    let entitled = run(&["-d", "--entitlements", "-"])
        .await
        .is_some_and(|out| {
            let all = [out.stdout, out.stderr].concat();
            out.status.success()
                && String::from_utf8_lossy(&all).contains("com.apple.security.virtualization")
        });
    let verifies = run(&["--verify"])
        .await
        .is_some_and(|out| out.status.success());
    keep_signature(entitled, verifies)
}

#[cfg(target_os = "macos")]
async fn sign_for_virtualization(codesign: &Path, bin_path: &Path) -> Result<()> {
    // Only codesign Mach-O binaries; scripts (e.g. unit test fixtures) skip cleanly.
    let header = fs::read(bin_path).await.unwrap_or_default();
    if !is_macho_binary(&header) || signature_ok(codesign, bin_path).await {
        return Ok(());
    }
    // Write temporary entitlements plist beside the binary and ad-hoc sign.
    let parent = bin_path.parent().unwrap_or_else(|| Path::new("/tmp"));
    let plist = parent.join(format!(".kiri-entitlements-{}.plist", std::process::id()));
    fs::write(&plist, KIRI_ENTITLEMENTS_PLIST)
        .await
        .map_err(|e| Error::Internal(format!("write entitlements plist: {e}")))?;
    let status = tokio::process::Command::new(codesign)
        .args(["--force", "--sign", "-", "--entitlements"])
        .arg(&plist)
        .arg(bin_path)
        .output()
        .await;
    let _ = fs::remove_file(&plist).await;
    if let Ok(out) = status {
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Upstream(format!(
                "codesign virtualization entitlement failed: {}",
                stderr.trim()
            )));
        }
    }
    if !signature_ok(codesign, bin_path).await {
        return Err(Error::Upstream(format!(
            "the Kiri runtime at {} does not pass `codesign --verify` even after re-signing it, so macOS would stop it at start",
            bin_path.display()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_macho_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    matches!(
        [bytes[0], bytes[1], bytes[2], bytes[3]],
        [0xCF, 0xFA, 0xED, 0xFE]
            | [0xFE, 0xED, 0xFA, 0xCF]
            | [0xCA, 0xFE, 0xBA, 0xBE]
            | [0xBE, 0xBA, 0xFE, 0xCA]
    )
}

/// Pure-Rust, traversal-safe `.tar.gz` extractor for embedded Kiri artifacts.
///
/// Rejects absolute paths, `..` traversal, NUL bytes, symlinks/hardlinks, and
/// archives exceeding entry or uncompressed size caps. Extracts ONLY the
/// headless `kiri` binary and allowlisted guest files (`Image`, `bzImage`,
/// `initramfs.cpio`, `data.img.tgz`, `data.img`).
pub fn extract_kiri_tar_gz(gz_bytes: &[u8]) -> Result<ExtractedKiri> {
    let mut decoder = GzDecoder::new(gz_bytes);
    let mut header = [0u8; 512];
    let mut binary: Option<Vec<u8>> = None;
    let mut guest_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_extracted: usize = 0;
    let mut entries_seen: usize = 0;
    let mut pax_path: Option<String> = None;

    loop {
        if !read_exact_or_eof(&mut decoder, &mut header)? {
            break;
        }
        if header.iter().all(|&b| b == 0) {
            break;
        }
        entries_seen += 1;
        if entries_seen > MAX_TAR_ENTRIES {
            return Err(Error::Invalid(format!(
                "tar archive exceeds max entry count ({MAX_TAR_ENTRIES})"
            )));
        }

        let size = parse_tar_octal(&header[124..136])? as usize;
        let typeflag = header[156];
        let raw_name = match pax_path.take() {
            Some(p) => p,
            None => parse_tar_name(&header),
        };

        // PAX extended header (`x` / `g`) or GNU LongLink (`L`) may supply the next entry's path.
        if typeflag == b'x' || typeflag == b'g' {
            let data = read_tar_payload(&mut decoder, size, 64 * 1024)?;
            if typeflag == b'x' {
                pax_path = parse_pax_path(&data);
            }
            continue;
        }
        if typeflag == b'L' {
            let data = read_tar_payload(&mut decoder, size, 64 * 1024)?;
            let text = String::from_utf8_lossy(&data);
            pax_path = Some(text.trim_end_matches('\0').to_string());
            continue;
        }

        // Validate path on EVERY archive entry before touching or skipping it.
        let clean = validate_tar_entry_path(&raw_name)?;

        // Reject symlinks (`2`) and hardlinks (`1`) on target files to prevent link attacks.
        if matches!(typeflag, b'1' | b'2') && classify_tar_target(&clean).is_some() {
            return Err(Error::Invalid(format!(
                "tar archive contains disallowed link entry: `{clean}`"
            )));
        }

        let is_regular = typeflag == 0 || typeflag == b'0';
        let target = if is_regular {
            classify_tar_target(&clean)
        } else {
            None
        };

        if let Some(kind) = target {
            if total_extracted.saturating_add(size) > MAX_EXTRACTED_TOTAL_BYTES {
                return Err(Error::PayloadTooLarge(format!(
                    "extracted Kiri assets exceed cap ({MAX_EXTRACTED_TOTAL_BYTES} bytes)"
                )));
            }
            let data = read_tar_payload(&mut decoder, size, MAX_EXTRACTED_TOTAL_BYTES)?;
            total_extracted = total_extracted.saturating_add(data.len());
            match kind {
                TarTarget::Binary => binary = Some(data),
                TarTarget::GuestAsset(name) => {
                    if !guest_files.iter().any(|(n, _)| n == &name) {
                        guest_files.push((name, data));
                    }
                }
            }
        } else {
            skip_tar_payload(&mut decoder, size)?;
        }
    }

    let binary = binary.ok_or_else(|| {
        Error::Invalid(
            "Kiri archive did not contain a `kiri` binary (`kiri`, `bin/kiri`, or `Contents/MacOS/kiri`)"
                .into(),
        )
    })?;
    Ok(ExtractedKiri {
        binary,
        guest_files,
    })
}

enum TarTarget {
    Binary,
    GuestAsset(String),
}

fn classify_tar_target(clean_path: &str) -> Option<TarTarget> {
    if clean_path == "kiri"
        || clean_path == "bin/kiri"
        || clean_path.ends_with("/Contents/MacOS/kiri")
        || clean_path == "Contents/MacOS/kiri"
    {
        return Some(TarTarget::Binary);
    }
    for &asset in GUEST_ASSET_NAMES {
        let suffix_res = format!("/Contents/Resources/guest/{asset}");
        let suffix_direct = format!("guest/{asset}");
        let suffix_nested = format!("/guest/{asset}");
        if clean_path == suffix_direct
            || clean_path.ends_with(&suffix_res)
            || clean_path == format!("Contents/Resources/guest/{asset}")
            || clean_path.ends_with(&suffix_nested)
        {
            return Some(TarTarget::GuestAsset(asset.to_string()));
        }
    }
    None
}

/// Reject absolute paths, NUL bytes, backslashes, and `..` traversal in tar entries.
pub fn validate_tar_entry_path(raw: &str) -> Result<String> {
    if raw.contains('\0') {
        return Err(Error::Invalid("tar entry path contains NUL byte".into()));
    }
    if raw.starts_with('/') || raw.contains('\\') {
        return Err(Error::Invalid(format!(
            "tar entry path must be relative POSIX path: `{raw}`"
        )));
    }
    let mut parts = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(Error::Invalid(format!(
                "tar entry path contains traversal `..`: `{raw}`"
            )));
        }
        parts.push(seg);
    }
    Ok(parts.join("/"))
}

fn parse_tar_name(header: &[u8; 512]) -> String {
    let name = cstr_slice(&header[0..100]);
    let prefix = cstr_slice(&header[345..500]);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn cstr_slice(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn parse_tar_octal(field: &[u8]) -> Result<u64> {
    let trimmed = cstr_slice(field);
    let digits = trimmed.trim();
    if digits.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(digits, 8)
        .map_err(|_| Error::Invalid(format!("invalid octal tar size `{digits}`")))
}

fn parse_pax_path(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    for line in text.lines() {
        if let Some((_, kv)) = line.split_once(' ') {
            if let Some(val) = kv.strip_prefix("path=") {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut read = 0;
    while read < buf.len() {
        match r.read(&mut buf[read..]) {
            Ok(0) if read == 0 => return Ok(false),
            Ok(0) => {
                return Err(Error::Invalid(
                    "unexpected EOF inside tar archive header".into(),
                ))
            }
            Ok(n) => read += n,
            Err(e) => return Err(Error::Invalid(format!("corrupt gzip/tar archive: {e}"))),
        }
    }
    Ok(true)
}

fn read_tar_payload<R: Read>(r: &mut R, size: usize, cap: usize) -> Result<Vec<u8>> {
    if size > cap {
        return Err(Error::PayloadTooLarge(format!(
            "tar entry size {size} exceeds cap ({cap} bytes)"
        )));
    }
    let mut data = vec![0u8; size];
    r.read_exact(&mut data)
        .map_err(|e| Error::Invalid(format!("unexpected EOF reading tar entry: {e}")))?;
    let pad = (512 - (size % 512)) % 512;
    if pad > 0 {
        let mut sink = [0u8; 512];
        r.read_exact(&mut sink[..pad])
            .map_err(|e| Error::Invalid(format!("unexpected EOF reading tar padding: {e}")))?;
    }
    Ok(data)
}

fn skip_tar_payload<R: Read>(r: &mut R, size: usize) -> Result<()> {
    let padded = size.div_ceil(512).saturating_mul(512);
    let mut remaining = padded;
    let mut scratch = [0u8; 8192];
    while remaining > 0 {
        let step = remaining.min(scratch.len());
        r.read_exact(&mut scratch[..step])
            .map_err(|e| Error::Invalid(format!("unexpected EOF skipping tar entry: {e}")))?;
        remaining -= step;
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod signature_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake `codesign`: `-d` lists the entitlement, `--verify` fails until
    /// `--force` re-signed (a flag file), every call is logged.
    async fn fake(dir: &Path, entitled: bool, verify_after_sign: bool) -> std::path::PathBuf {
        let bin = dir.join("codesign");
        let ent = if entitled {
            "echo com.apple.security.virtualization"
        } else {
            "echo none"
        };
        let after = if verify_after_sign {
            "exit 0"
        } else {
            "exit 1"
        };
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> {dir}/log\ncase \"$1\" in -d) {ent}; exit 0;; --verify) if [ -f {dir}/signed ]; then {after}; fi; echo 'invalid Info.plist (plist or signature have been modified)' >&2; exit 1;; --force) touch {dir}/signed; exit 0;; esac\n",
            dir = dir.display()
        );
        fs::write(&bin, script).await.unwrap();
        fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        bin
    }

    async fn macho(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("kiri");
        fs::write(&p, [0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0])
            .await
            .unwrap();
        p
    }

    #[test]
    fn a_signature_is_kept_only_when_entitled_and_verified() {
        assert!(keep_signature(true, true));
        assert!(
            !keep_signature(true, false),
            "the entitlement alone is not enough"
        );
        assert!(!keep_signature(false, true));
    }

    /// The upstream binary taken out of Kiri.app: entitled, but its signature
    /// does not verify: it is re-signed ad hoc, and then verifies.
    #[tokio::test]
    async fn an_entitled_binary_that_does_not_verify_is_re_signed() {
        let dir = tempfile::tempdir().unwrap();
        let codesign = fake(dir.path(), true, true).await;
        sign_for_virtualization(&codesign, &macho(dir.path()).await)
            .await
            .unwrap();
        let log = fs::read_to_string(dir.path().join("log")).await.unwrap();
        assert!(log.contains("--force --sign - --entitlements"), "{log}");
    }

    #[tokio::test]
    async fn a_re_sign_that_still_does_not_verify_fails_the_install() {
        let dir = tempfile::tempdir().unwrap();
        let codesign = fake(dir.path(), true, false).await;
        let err = sign_for_virtualization(&codesign, &macho(dir.path()).await)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not pass"), "{err}");
    }

    /// Scripts (test fixtures) are never signed.
    #[tokio::test]
    async fn a_script_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let codesign = fake(dir.path(), false, false).await;
        let script = dir.path().join("kiri.sh");
        fs::write(&script, "#!/bin/sh\n").await.unwrap();
        sign_for_virtualization(&codesign, &script).await.unwrap();
        assert!(!dir.path().join("log").exists());
    }
}

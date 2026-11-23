//! Local HTTPS streaming downloader (with SHA-256 verification), archive
//! extractor, and local SDK installer routines for [`super::ToolchainManager`].

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::toolchains_catalog::{CatalogSdkEntry, EcosystemKind};

use super::discovery::discover_local_sdks_with_shell;
use super::{
    find_executable_in, is_executable_file, read_subdirs, resolve_java_home_dir,
    system_search_dirs_with_shell, ToolchainManager,
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const LOCAL_CMD_TIMEOUT: Duration = Duration::from_secs(600);

pub(super) struct InstallerProcessGroupGuard {
    pub pid: Option<u32>,
    pub disarmed: bool,
}

impl Drop for InstallerProcessGroupGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                crate::lsp_host::pool_spawn::kill_process_group_sync(pid);
            }
        }
    }
}

pub(crate) const MAX_TOOLCHAIN_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_COMPANION_SHA_BYTES: usize = 4096;

pub(super) async fn download_https_to_file(
    mgr: &ToolchainManager,
    job_id: &str,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    base_pct: u8,
) -> std::result::Result<(), String> {
    download_https_to_file_capped(
        mgr,
        job_id,
        url,
        dest,
        expected_sha256,
        base_pct,
        MAX_TOOLCHAIN_DOWNLOAD_BYTES,
    )
    .await
}

pub(super) async fn download_https_to_file_capped(
    mgr: &ToolchainManager,
    job_id: &str,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    base_pct: u8,
    max_bytes: u64,
) -> std::result::Result<(), String> {
    let client = if mgr.allows_test_loopback() && url.starts_with("http://127.0.0.1:") {
        reqwest::Client::builder()
            .user_agent("otto-ide/0.2")
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .map_err(|e| format!("http client: {e}"))?
    } else {
        if !url.starts_with("https://") {
            return Err(format!("refusing non-HTTPS toolchain URL: {url}"));
        }
        otto_netguard::check_url(url)
            .await
            .map_err(|e| format!("blocked toolchain URL ({url}): {e}"))?;
        otto_netguard::guarded_client_builder()
            .https_only(true)
            .user_agent("otto-ide/0.2")
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .map_err(|e| format!("http client: {e}"))?
    };
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed ({url}): {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download HTTP {} from {url}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    if total > max_bytes {
        return Err(format!(
            "download Content-Length ({total} bytes) exceeds {max_bytes} byte limit ({url})"
        ));
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let (mut received, mut last_pct) = (0u64, base_pct);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("stream error: {e}"))?;
        let next_received = received.saturating_add(bytes.len() as u64);
        if next_received > max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(format!("download exceeded {max_bytes} byte limit ({url})"));
        }
        hasher.update(&bytes);
        file.write_all(&bytes).await.map_err(|e| e.to_string())?;
        received = next_received;
        if total > 0 {
            let pct = (base_pct + ((received as f64 / total as f64) * 35.0) as u8).min(90);
            if pct >= last_pct + 10 {
                last_pct = pct;
                mgr.log_progress(
                    job_id,
                    pct,
                    format!(
                        "Downloaded {:.1}/{:.1} MB...",
                        received as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    ),
                );
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    let actual_hex = hex::encode(hasher.finalize());
    if let Err(mismatch) = verify_sha256_digest(&actual_hex, expected_sha256, url) {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(mismatch);
    }
    if expected_sha256.is_some_and(|s| !s.trim().is_empty()) {
        mgr.log_progress(
            job_id,
            base_pct.saturating_add(36).min(92),
            format!("Verified SHA-256 checksum ({}…)", &actual_hex[..12]),
        );
    }
    Ok(())
}

pub(crate) fn verify_sha256_digest(
    actual_hex: &str,
    expected_sha256: Option<&str>,
    url: &str,
) -> std::result::Result<(), String> {
    if let Some(exp) = expected_sha256.map(str::trim).filter(|s| !s.is_empty()) {
        if !actual_hex.eq_ignore_ascii_case(exp) {
            return Err(format!(
                "SHA-256 checksum mismatch for {url}: expected {exp}, got {actual_hex}"
            ));
        }
    }
    Ok(())
}

pub(super) async fn run_local_cmd(
    mgr: &ToolchainManager,
    job_id: &str,
    pct: u8,
    program: &Path,
    args: &[&str],
    envs: &[(&str, &Path)],
) -> std::result::Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(mgr.shell_env_snapshot())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", program.display()))?;
    let pid = child.id();
    let mut pg_guard = InstallerProcessGroupGuard {
        pid,
        disarmed: false,
    };

    let out = match tokio::time::timeout(LOCAL_CMD_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("wait {}: {e}", program.display())),
        Err(_) => {
            return Err(format!(
                "{} timed out after {}s",
                program.display(),
                LOCAL_CMD_TIMEOUT.as_secs()
            ));
        }
    };
    pg_guard.disarmed = true;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in combined.lines().take(15) {
        mgr.log_progress(job_id, pct, line.to_string());
    }
    if !out.status.success() {
        return Err(format!("{} failed ({})", program.display(), out.status));
    }
    Ok(())
}

pub(crate) fn make_executable(path: &Path) -> std::result::Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod +x {}: {e}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

pub(super) fn relink_current(link: &Path, target: &Path) {
    #[cfg(unix)]
    {
        if let Some(p) = link.parent() {
            let _ = std::fs::create_dir_all(p);
            let tmp_link = p.join(format!(
                ".{}.{}.tmp",
                link.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("current"),
                otto_core::new_id()
            ));
            let _ = std::fs::remove_file(&tmp_link);
            if std::os::unix::fs::symlink(target, &tmp_link).is_ok() {
                if std::fs::rename(&tmp_link, link).is_ok() {
                    return;
                }
                let _ = std::fs::remove_file(&tmp_link);
            }
        }
        let _ = std::fs::remove_file(link);
        let _ = std::os::unix::fs::symlink(target, link);
    }
    let _ = (link, target);
}

pub(super) async fn install_local_sdk(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    entry: &CatalogSdkEntry,
) -> std::result::Result<String, String> {
    let tmp = root.join(".downloads");
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    let exp_sha = entry.sha256.as_deref();
    match eco {
        EcosystemKind::Java => {
            let archive = tmp.join(format!("temurin-{}.tar.gz", entry.version));
            download_https_to_file(mgr, job_id, &entry.url, &archive, exp_sha, 15).await?;
            let dest = root
                .join("java/jdks")
                .join(format!("jdk-{}", entry.version));
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                50,
                Path::new("tar"),
                &[
                    "-xzf",
                    &archive.to_string_lossy(),
                    "-C",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&archive).await;
            let jh = resolve_java_home_dir(&dest).ok_or("JDK archive missing bin/java")?;
            make_executable(&jh.join("bin/java"))?;
            relink_current(&root.join("java/current"), &jh);
            run_local_cmd(mgr, job_id, 58, &jh.join("bin/java"), &["-version"], &[]).await?;
            Ok(jh.to_string_lossy().into_owned())
        }
        EcosystemKind::Go => {
            let archive = tmp.join(format!("go-{}.tar.gz", entry.version));
            download_https_to_file(mgr, job_id, &entry.url, &archive, exp_sha, 15).await?;
            let dest = root.join("go/sdks").join(&entry.version);
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                50,
                Path::new("tar"),
                &[
                    "-xzf",
                    &archive.to_string_lossy(),
                    "-C",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&archive).await;
            let goroot = dest.join("go");
            make_executable(&goroot.join("bin/go"))?;
            relink_current(&root.join("go/current"), &goroot);
            run_local_cmd(mgr, job_id, 58, &goroot.join("bin/go"), &["version"], &[]).await?;
            Ok(goroot.to_string_lossy().into_owned())
        }
        EcosystemKind::Rust => {
            let init_bin = tmp.join("rustup-init");
            download_https_to_file(mgr, job_id, &entry.url, &init_bin, exp_sha, 15).await?;
            make_executable(&init_bin)?;
            let (rh, ch) = (root.join("rust/rustup"), root.join("rust/cargo"));
            tokio::fs::create_dir_all(&rh)
                .await
                .map_err(|e| e.to_string())?;
            tokio::fs::create_dir_all(&ch)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                52,
                &init_bin,
                &[
                    "-y",
                    "--no-modify-path",
                    "--default-toolchain",
                    &entry.version,
                ],
                &[("RUSTUP_HOME", &rh), ("CARGO_HOME", &ch)],
            )
            .await?;
            let _ = tokio::fs::remove_file(&init_bin).await;
            run_local_cmd(
                mgr,
                job_id,
                58,
                &ch.join("bin/rustc"),
                &["--version"],
                &[("RUSTUP_HOME", &rh), ("CARGO_HOME", &ch)],
            )
            .await?;
            Ok(ch.to_string_lossy().into_owned())
        }
        EcosystemKind::Node => {
            let archive = tmp.join(format!("node-{}.tar.gz", entry.version));
            download_https_to_file(mgr, job_id, &entry.url, &archive, exp_sha, 15).await?;
            let dest = root.join("node/sdks").join(&entry.version);
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                50,
                Path::new("tar"),
                &[
                    "-xzf",
                    &archive.to_string_lossy(),
                    "--strip-components=1",
                    "-C",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&archive).await;
            make_executable(&dest.join("bin/node"))?;
            relink_current(&root.join("node/current"), &dest);
            run_local_cmd(mgr, job_id, 58, &dest.join("bin/node"), &["--version"], &[]).await?;
            Ok(dest.to_string_lossy().into_owned())
        }
        EcosystemKind::Python => {
            let uv_bin = ensure_local_uv(mgr, job_id, root, &entry.url, exp_sha).await?;
            let interp_dir = root.join("python/interpreters");
            tokio::fs::create_dir_all(&interp_dir)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                54,
                &uv_bin,
                &["python", "install", &entry.version],
                &[("UV_PYTHON_INSTALL_DIR", &interp_dir)],
            )
            .await?;
            if let Some(latest) = read_subdirs(&interp_dir).last() {
                relink_current(&root.join("python/current"), latest);
            }
            Ok(interp_dir.to_string_lossy().into_owned())
        }
        EcosystemKind::Shell => {
            let bin_dir = root.join("shell/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let shfmt = bin_dir.join("shfmt");
            download_https_to_file(mgr, job_id, &entry.url, &shfmt, exp_sha, 15).await?;
            make_executable(&shfmt)?;
            run_local_cmd(mgr, job_id, 58, &shfmt, &["--version"], &[]).await?;
            Ok(shfmt.to_string_lossy().into_owned())
        }
        EcosystemKind::Json => {
            let bin_dir = root.join("json/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let jq = bin_dir.join("jq");
            download_https_to_file(mgr, job_id, &entry.url, &jq, exp_sha, 15).await?;
            make_executable(&jq)?;
            run_local_cmd(mgr, job_id, 58, &jq, &["--version"], &[]).await?;
            Ok(jq.to_string_lossy().into_owned())
        }
        EcosystemKind::Ios => {
            Err("Install Xcode from the Mac App Store, then run `xcode-select --install` and `xcodebuild -downloadPlatform iOS`.".into())
        }
        EcosystemKind::Android => {
            Err("Install Android SDK via Android Studio or `brew install --cask android-commandlinetools`, then register its path here.".into())
        }
        EcosystemKind::Flutter => {
            Err("Install Flutter SDK from https://docs.flutter.dev or `brew install --cask flutter`, then register its path here.".into())
        }
    }
}

pub(super) async fn fetch_companion_sha256(
    mgr: &ToolchainManager,
    archive_url: &str,
) -> Option<String> {
    let sha_url = format!("{archive_url}.sha256");
    let client = if mgr.allows_test_loopback() && sha_url.starts_with("http://127.0.0.1:") {
        reqwest::Client::builder()
            .user_agent("otto-ide/0.2")
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?
    } else if sha_url.starts_with("https://github.com/astral-sh/uv/") {
        otto_netguard::check_url(&sha_url).await.ok()?;
        otto_netguard::guarded_client_builder()
            .https_only(true)
            .user_agent("otto-ide/0.2")
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?
    } else {
        return None;
    };
    let mut resp = client.get(&sha_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len().saturating_add(chunk.len()) > MAX_COMPANION_SHA_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    let body = String::from_utf8(buf).ok()?;
    let first_tok = body.split_whitespace().next()?.trim();
    (first_tok.len() == 64 && first_tok.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| first_tok.to_ascii_lowercase())
}

pub(super) async fn ensure_local_uv(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    uv_url: &str,
    expected_sha256: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    let uv_bin = root.join("python/bin/uv");
    if is_executable_file(&uv_bin) {
        return Ok(uv_bin);
    }
    let sp = mgr.shell_path();
    if let Some(sys_uv) = find_executable_in(&system_search_dirs_with_shell(sp.as_deref()), &["uv"])
    {
        return Ok(sys_uv);
    }
    tokio::fs::create_dir_all(uv_bin.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let companion_sha = match expected_sha256.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(s.to_string()),
        None => fetch_companion_sha256(mgr, uv_url).await,
    };
    let archive = root.join(".downloads/uv.tar.gz");
    download_https_to_file(mgr, job_id, uv_url, &archive, companion_sha.as_deref(), 20).await?;
    let extract = root.join(".downloads/uv-extract");
    let _ = tokio::fs::remove_dir_all(&extract).await;
    tokio::fs::create_dir_all(&extract)
        .await
        .map_err(|e| e.to_string())?;
    run_local_cmd(
        mgr,
        job_id,
        35,
        Path::new("tar"),
        &[
            "-xzf",
            &archive.to_string_lossy(),
            "-C",
            &extract.to_string_lossy(),
        ],
        &[],
    )
    .await?;
    let extract_clone = extract.clone();
    let found = tokio::task::spawn_blocking(move || find_file_recursive(&extract_clone, "uv"))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("uv binary missing in archive")?;
    tokio::fs::copy(&found, &uv_bin)
        .await
        .map_err(|e| e.to_string())?;
    make_executable(&uv_bin)?;
    let _ = tokio::fs::remove_file(&archive).await;
    let _ = tokio::fs::remove_dir_all(&extract).await;
    Ok(uv_bin)
}

pub(super) fn resolve_best_sdk_bin(
    mgr: &ToolchainManager,
    root: &Path,
    eco: EcosystemKind,
    bin_name: &str,
) -> Option<PathBuf> {
    let sp = mgr.shell_path();
    discover_local_sdks_with_shell(&[root.to_path_buf()], eco, sp.as_deref())
        .into_iter()
        .map(|sdk| PathBuf::from(sdk.path).join("bin").join(bin_name))
        .find(|cand| is_executable_file(cand))
        .or_else(|| find_executable_in(&system_search_dirs_with_shell(sp.as_deref()), &[bin_name]))
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking on newly extracted archive dir
fn find_file_recursive(dir: &Path, target_name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_file() && p.file_name().and_then(|s| s.to_str()) == Some(target_name) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(hit) = find_file_recursive(&p, target_name) {
                return Some(hit);
            }
        }
    }
    None
}

//! Local HTTPS streaming downloader (with SHA-256 verification), archive
//! extractor, process-group-isolated installer commands, and remote SSH
//! installer routines for [`super::ToolchainManager`].

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::toolchains_catalog::{
    codelldb_vsix_url, jq_binary_url, rust_analyzer_gz_url, shfmt_binary_url, uv_tarball_url,
    CatalogSdkEntry, EcosystemKind, TargetPlatform, BASH_LSP_NPM_PACKAGES, DEBUGPY_PKG, DLV_PKG,
    GOPLS_PKG, JAVA_DEBUG_JAR_NAME, JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL, JSON_LSP_NPM_PACKAGES,
    JS_DEBUG_TARBALL_URL, PYRIGHT_PKG, TS_LSP_NPM_PACKAGES,
};
use crate::types::{IdeInstallComponent, IdeSshSpec};

use super::{
    discover_local_sdks, find_executable_in, is_executable_file, read_subdirs,
    resolve_java_home_dir, system_search_dirs, ToolchainManager,
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const LOCAL_CMD_TIMEOUT: Duration = Duration::from_secs(600);

struct InstallerProcessGroupGuard {
    pid: Option<u32>,
    disarmed: bool,
}

impl Drop for InstallerProcessGroupGuard {
    #[allow(clippy::disallowed_methods)] // synchronous Drop safeguard against orphan installer children
    fn drop(&mut self) {
        if !self.disarmed {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

pub(super) async fn download_https_to_file(
    mgr: &ToolchainManager,
    job_id: &str,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    base_pct: u8,
) -> std::result::Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("refusing non-HTTPS toolchain URL: {url}"));
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .user_agent("otto-ide/0.2")
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed ({url}): {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download HTTP {} from {url}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
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
        hasher.update(&bytes);
        file.write_all(&bytes).await.map_err(|e| e.to_string())?;
        received += bytes.len() as u64;
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

async fn run_local_cmd(
    mgr: &ToolchainManager,
    job_id: &str,
    pct: u8,
    program: &Path,
    args: &[&str],
    envs: &[(&str, &Path)],
) -> std::result::Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(crate::environments::cached_shell_env_snapshot())
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

fn relink_current(link: &Path, target: &Path) {
    #[cfg(unix)]
    {
        if let Some(p) = link.parent() {
            let _ = std::fs::create_dir_all(p);
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
            let uv_bin = ensure_local_uv(mgr, job_id, root, &entry.url).await?;
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
    }
}

async fn ensure_local_uv(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    uv_url: &str,
) -> std::result::Result<PathBuf, String> {
    let uv_bin = root.join("python/bin/uv");
    if is_executable_file(&uv_bin) {
        return Ok(uv_bin);
    }
    if let Some(sys_uv) = find_executable_in(&system_search_dirs(), &["uv"]) {
        return Ok(sys_uv);
    }
    tokio::fs::create_dir_all(uv_bin.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let archive = root.join(".downloads/uv.tar.gz");
    download_https_to_file(mgr, job_id, uv_url, &archive, None, 20).await?;
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

pub(super) async fn install_local_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
) -> std::result::Result<String, String> {
    let tmp = root.join(".downloads");
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    match eco {
        EcosystemKind::Java => {
            let archive = tmp.join("jdtls.tar.gz");
            download_https_to_file(mgr, job_id, JDTLS_TARBALL_URL, &archive, None, 65).await?;
            let dest = root.join("java/lsp");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
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
            let bin = dest.join("bin/jdtls");
            make_executable(&bin)?;
            Ok(bin.to_string_lossy().into_owned())
        }
        EcosystemKind::Go => {
            let go_bin = resolve_best_sdk_bin(root, EcosystemKind::Go, "go")
                .ok_or("Go SDK required first")?;
            let (gobin, gopath) = (root.join("go/bin"), root.join("go/gopath"));
            tokio::fs::create_dir_all(&gobin)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
                &go_bin,
                &["install", GOPLS_PKG],
                &[("GOBIN", &gobin), ("GOPATH", &gopath)],
            )
            .await?;
            Ok(gobin.join("gopls").to_string_lossy().into_owned())
        }
        EcosystemKind::Rust => {
            let gz = tmp.join("rust-analyzer.gz");
            download_https_to_file(mgr, job_id, &rust_analyzer_gz_url(platform), &gz, None, 65)
                .await?;
            let bin_dir = root.join("rust/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let dest = bin_dir.join("rust-analyzer");
            let _ = tokio::fs::remove_file(&dest).await;
            run_local_cmd(
                mgr,
                job_id,
                75,
                Path::new("gzip"),
                &["-d", "-f", &gz.to_string_lossy()],
                &[],
            )
            .await?;
            tokio::fs::rename(tmp.join("rust-analyzer"), &dest)
                .await
                .map_err(|e| e.to_string())?;
            make_executable(&dest)?;
            Ok(dest.to_string_lossy().into_owned())
        }
        EcosystemKind::Node => {
            let npm = resolve_best_sdk_bin(root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required first")?;
            let prefix = root.join("node/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(TS_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/typescript-language-server")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Python => {
            let uv_bin = ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform)).await?;
            let (tool_dir, bin_dir) = (root.join("python/tools"), root.join("python/bin"));
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
                &uv_bin,
                &["tool", "install", "--force", PYRIGHT_PKG],
                &[("UV_TOOL_DIR", &tool_dir), ("UV_TOOL_BIN_DIR", &bin_dir)],
            )
            .await?;
            Ok(bin_dir
                .join("pyright-langserver")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Shell => {
            let npm = resolve_best_sdk_bin(root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required to install bash-language-server")?;
            let prefix = root.join("shell/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(BASH_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/bash-language-server")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Json => {
            let npm = resolve_best_sdk_bin(root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required to install vscode-json-language-server")?;
            let prefix = root.join("json/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(JSON_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/vscode-json-language-server")
                .to_string_lossy()
                .into_owned())
        }
    }
}

pub(super) async fn install_local_dap(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
) -> std::result::Result<String, String> {
    let tmp = root.join(".downloads");
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    match eco {
        EcosystemKind::Java => {
            let jar = root.join("java/dap").join(JAVA_DEBUG_JAR_NAME);
            download_https_to_file(mgr, job_id, JAVA_DEBUG_JAR_URL, &jar, None, 85).await?;
            Ok(jar.to_string_lossy().into_owned())
        }
        EcosystemKind::Go => {
            let go_bin = resolve_best_sdk_bin(root, EcosystemKind::Go, "go")
                .ok_or("Go SDK required first")?;
            let (gobin, gopath) = (root.join("go/bin"), root.join("go/gopath"));
            tokio::fs::create_dir_all(&gobin)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                92,
                &go_bin,
                &["install", DLV_PKG],
                &[("GOBIN", &gobin), ("GOPATH", &gopath)],
            )
            .await?;
            Ok(gobin.join("dlv").to_string_lossy().into_owned())
        }
        EcosystemKind::Rust => {
            let vsix = tmp.join("codelldb.vsix");
            download_https_to_file(mgr, job_id, &codelldb_vsix_url(platform), &vsix, None, 85)
                .await?;
            let dest = root.join("rust/dap");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                94,
                Path::new("unzip"),
                &[
                    "-q",
                    "-o",
                    &vsix.to_string_lossy(),
                    "-d",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&vsix).await;
            let adapter = dest.join("extension/adapter/codelldb");
            make_executable(&adapter)?;
            relink_current(&root.join("rust/codelldb"), &dest.join("extension"));
            Ok(adapter.to_string_lossy().into_owned())
        }
        EcosystemKind::Node => {
            let archive = tmp.join("js-debug.tar.gz");
            download_https_to_file(mgr, job_id, JS_DEBUG_TARBALL_URL, &archive, None, 85).await?;
            let dest = root.join("node/dap");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                94,
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
            relink_current(&root.join("node/js-debug"), &dest.join("js-debug"));
            Ok(dest
                .join("js-debug/src/dapDebugServer.js")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Python => {
            let uv_bin = ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform)).await?;
            let dap_venv = root.join("python/dap");
            run_local_cmd(
                mgr,
                job_id,
                90,
                &uv_bin,
                &["venv", "--clear", &dap_venv.to_string_lossy()],
                &[],
            )
            .await?;
            let venv_py = dap_venv.join("bin/python");
            run_local_cmd(
                mgr,
                job_id,
                95,
                &uv_bin,
                &[
                    "pip",
                    "install",
                    "--python",
                    &venv_py.to_string_lossy(),
                    DEBUGPY_PKG,
                ],
                &[],
            )
            .await?;
            Ok(venv_py.to_string_lossy().into_owned())
        }
        EcosystemKind::Shell => {
            if let Some(sys_bashdb) = find_executable_in(&system_search_dirs(), &["bashdb"]) {
                return Ok(sys_bashdb.to_string_lossy().into_owned());
            }
            let bin_dir = root.join("shell/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let wrapper = bin_dir.join("bashdb");
            tokio::fs::write(
                &wrapper,
                "#!/usr/bin/env bash\nexport PS4='+ ${BASH_SOURCE}:${LINENO}: '\nexec bash -x \"$@\"\n",
            )
            .await
            .map_err(|e| e.to_string())?;
            make_executable(&wrapper)?;
            Ok(wrapper.to_string_lossy().into_owned())
        }
        EcosystemKind::Json => {
            let bin_dir = root.join("json/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let jq = bin_dir.join("jq");
            if !is_executable_file(&jq) {
                download_https_to_file(
                    mgr,
                    job_id,
                    &jq_binary_url("1.7.1", platform),
                    &jq,
                    None,
                    85,
                )
                .await?;
                make_executable(&jq)?;
            }
            Ok(jq.to_string_lossy().into_owned())
        }
    }
}

fn resolve_best_sdk_bin(root: &Path, eco: EcosystemKind, bin_name: &str) -> Option<PathBuf> {
    discover_local_sdks(&[root.to_path_buf()], eco)
        .into_iter()
        .map(|sdk| PathBuf::from(sdk.path).join("bin").join(bin_name))
        .find(|cand| is_executable_file(cand))
        .or_else(|| find_executable_in(&system_search_dirs(), &[bin_name]))
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

pub(super) async fn run_ssh_streaming<F: FnMut(String)>(
    ssh: &IdeSshSpec,
    script: &str,
    mut on_line: F,
) -> std::result::Result<String, String> {
    if !otto_ssh::valid_ssh_host(&ssh.host) || !otto_ssh::valid_ssh_user(&ssh.user) {
        return Err("invalid SSH host or user".into());
    }
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ConnectTimeout=12",
    ]);
    if ssh.port != 0 && ssh.port != 22 {
        cmd.arg("-p").arg(ssh.port.to_string());
    }
    if let Some(id_file) = ssh
        .identity_file
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        cmd.arg("-i").arg(id_file);
    }
    if let Some(jump) = ssh.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
        if !otto_ssh::valid_proxy_jump(jump) {
            return Err("invalid SSH ProxyJump".into());
        }
        cmd.arg("-J").arg(jump);
    }
    let dest = if ssh.user.trim().is_empty() {
        ssh.host.clone()
    } else {
        format!("{}@{}", ssh.user.trim(), ssh.host)
    };
    cmd.arg("--")
        .arg(dest)
        .args(["sh", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))?;
    let mut pg_guard = InstallerProcessGroupGuard {
        pid: child.id(),
        disarmed: false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    let stdout = child.stdout.take().ok_or("missing ssh stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut installed = String::from("~/.otto/toolchains");
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(p) = line.strip_prefix("INSTALLED_PATH:") {
            installed = p.trim().to_string();
        } else {
            on_line(line);
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    pg_guard.disarmed = true;
    if !status.success() {
        return Err(format!("remote installer exited with {status}"));
    }
    Ok(installed)
}

pub(super) fn build_remote_install_script(
    eco: EcosystemKind,
    component: IdeInstallComponent,
    platform: TargetPlatform,
    sdk: Option<&CatalogSdkEntry>,
) -> std::result::Result<String, String> {
    let mut s =
        String::from("set -eu\nTC=\"$HOME/.otto/toolchains\"\nmkdir -p \"$TC/.downloads\"\n");
    if matches!(
        component,
        IdeInstallComponent::Sdk | IdeInstallComponent::All
    ) {
        let entry = sdk.ok_or("missing SDK entry")?;
        let (u, v) = (
            shell_words::quote(&entry.url),
            shell_words::quote(&entry.version),
        );
        match eco {
            EcosystemKind::Java => s.push_str(&format!("DEST=\"$TC/java/jdks/jdk-{0}\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} | tar -xz -C \"$DEST\"\nln -sfn \"$DEST\" \"$TC/java/current\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Go => s.push_str(&format!("DEST=\"$TC/go/sdks/{0}\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} | tar -xz -C \"$DEST\"\n\"$DEST/go/bin/go\" version\nln -sfn \"$DEST/go\" \"$TC/go/current\"\necho \"INSTALLED_PATH:$DEST/go\"\n", entry.version)),
            EcosystemKind::Rust => s.push_str(&format!("INIT=\"$TC/.downloads/rustup-init\"\ncurl -fsSL {u} -o \"$INIT\" && chmod +x \"$INIT\"\nRUSTUP_HOME=\"$TC/rust/rustup\" CARGO_HOME=\"$TC/rust/cargo\" \"$INIT\" -y --no-modify-path --default-toolchain {v}\necho \"INSTALLED_PATH:$TC/rust/cargo\"\n")),
            EcosystemKind::Node => s.push_str(&format!("DEST=\"$TC/node/sdks/{0}\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} | tar -xz --strip-components=1 -C \"$DEST\"\n\"$DEST/bin/node\" --version\nln -sfn \"$DEST\" \"$TC/node/current\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Python => s.push_str(&format!("mkdir -p \"$TC/python/bin\" \"$TC/python/interpreters\"\ncurl -fsSL {u} | tar -xz -C \"$TC/.downloads\"\nUV_BIN=$(find \"$TC/.downloads\" -name uv -type f | head -n 1)\ncp \"$UV_BIN\" \"$TC/python/bin/uv\" && chmod +x \"$TC/python/bin/uv\"\nUV_PYTHON_INSTALL_DIR=\"$TC/python/interpreters\" \"$TC/python/bin/uv\" python install {v}\necho \"INSTALLED_PATH:$TC/python/interpreters\"\n")),
            EcosystemKind::Shell => s.push_str(&format!("mkdir -p \"$TC/shell/bin\"\ncurl -fsSL '{}' -o \"$TC/shell/bin/shfmt\" && chmod +x \"$TC/shell/bin/shfmt\"\necho \"INSTALLED_PATH:$TC/shell/bin/shfmt\"\n", shfmt_binary_url(&entry.version, platform))),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\ncurl -fsSL '{}' -o \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url(&entry.version, platform))),
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Lsp | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java => s.push_str(&format!("mkdir -p \"$TC/java/lsp\"\ncurl -fsSL '{JDTLS_TARBALL_URL}' | tar -xz -C \"$TC/java/lsp\" && chmod +x \"$TC/java/lsp/bin/jdtls\"\necho \"INSTALLED_PATH:$TC/java/lsp/bin/jdtls\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/sdks/{}/go/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{GOPLS_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/gopls\"\n", eco.default_sdk_version())),
            EcosystemKind::Rust => s.push_str(&format!("mkdir -p \"$TC/rust/bin\"\ncurl -fsSL '{}' | gzip -d > \"$TC/rust/bin/rust-analyzer\" && chmod +x \"$TC/rust/bin/rust-analyzer\"\necho \"INSTALLED_PATH:$TC/rust/bin/rust-analyzer\"\n", rust_analyzer_gz_url(platform))),
            EcosystemKind::Node => s.push_str(&format!("export PATH=\"$TC/node/sdks/{}/bin:$PATH\"\nmkdir -p \"$TC/node/lsp\"\nnpm install --prefix \"$TC/node/lsp\" typescript typescript-language-server\necho \"INSTALLED_PATH:$TC/node/lsp/node_modules/.bin/typescript-language-server\"\n", eco.default_sdk_version())),
            EcosystemKind::Python => s.push_str(&format!("UV_TOOL_DIR=\"$TC/python/tools\" UV_TOOL_BIN_DIR=\"$TC/python/bin\" \"$TC/python/bin/uv\" tool install --force '{PYRIGHT_PKG}'\necho \"INSTALLED_PATH:$TC/python/bin/pyright-langserver\"\n")),
            EcosystemKind::Shell => s.push_str("mkdir -p \"$TC/shell/lsp\"\nnpm install --prefix \"$TC/shell/lsp\" bash-language-server\necho \"INSTALLED_PATH:$TC/shell/lsp/node_modules/.bin/bash-language-server\"\n"),
            EcosystemKind::Json => s.push_str("mkdir -p \"$TC/json/lsp\"\nnpm install --prefix \"$TC/json/lsp\" vscode-langservers-extracted\necho \"INSTALLED_PATH:$TC/json/lsp/node_modules/.bin/vscode-json-language-server\"\n"),
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Dap | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java => s.push_str(&format!("mkdir -p \"$TC/java/dap\"\ncurl -fsSL '{JAVA_DEBUG_JAR_URL}' -o \"$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\necho \"INSTALLED_PATH:$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/sdks/{}/go/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{DLV_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/dlv\"\n", eco.default_sdk_version())),
            EcosystemKind::Rust => s.push_str(&format!("rm -rf \"$TC/rust/dap\" && mkdir -p \"$TC/rust/dap\"\ncurl -fsSL '{}' -o \"$TC/.downloads/codelldb.vsix\" && unzip -q -o \"$TC/.downloads/codelldb.vsix\" -d \"$TC/rust/dap\" && chmod +x \"$TC/rust/dap/extension/adapter/codelldb\"\nln -sfn \"$TC/rust/dap/extension\" \"$TC/rust/codelldb\"\necho \"INSTALLED_PATH:$TC/rust/dap/extension/adapter/codelldb\"\n", codelldb_vsix_url(platform))),
            EcosystemKind::Node => s.push_str(&format!("rm -rf \"$TC/node/dap\" && mkdir -p \"$TC/node/dap\"\ncurl -fsSL '{JS_DEBUG_TARBALL_URL}' | tar -xz -C \"$TC/node/dap\"\nln -sfn \"$TC/node/dap/js-debug\" \"$TC/node/js-debug\"\necho \"INSTALLED_PATH:$TC/node/dap/js-debug/src/dapDebugServer.js\"\n")),
            EcosystemKind::Python => s.push_str(&format!("\"$TC/python/bin/uv\" venv --clear \"$TC/python/dap\" && \"$TC/python/bin/uv\" pip install --python \"$TC/python/dap/bin/python\" '{DEBUGPY_PKG}'\necho \"INSTALLED_PATH:$TC/python/dap/bin/python\"\n")),
            EcosystemKind::Shell => s.push_str("mkdir -p \"$TC/shell/bin\"\nprintf '#!/usr/bin/env bash\\nexec bash -x \"$@\"\\n' > \"$TC/shell/bin/bashdb\" && chmod +x \"$TC/shell/bin/bashdb\"\necho \"INSTALLED_PATH:$TC/shell/bin/bashdb\"\n"),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\ncurl -fsSL '{}' -o \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url("1.7.1", platform))),
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_verifier_accepts_case_insensitive_match_and_rejects_corruption() {
        let actual = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256_digest(actual, None, "https://go.dev/dl/go.tar.gz").is_ok());
        assert!(verify_sha256_digest(
            actual,
            Some("E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855"),
            "https://go.dev/dl/go.tar.gz"
        )
        .is_ok());
        let err = verify_sha256_digest(
            actual,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            "https://go.dev/dl/go.tar.gz",
        )
        .unwrap_err();
        assert!(err.contains("SHA-256 checksum mismatch"));
    }
}

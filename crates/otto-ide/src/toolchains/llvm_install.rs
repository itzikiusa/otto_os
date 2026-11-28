//! Managed LLVM / Clang install (rc.6 design B §1): preflight, a download
//! capped at and hash-checked against the pinned manifest, an allowlisted
//! unpack into staging, a self-check that runs every tool and compiles and
//! runs a C++20 program, then a rename into `toolchains/cpp/sdks/llvm-<ver>`.
//! Any failure removes staging and the archive and leaves the previous
//! install untouched.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::toolchains_catalog::{CatalogSdkEntry, TargetOs, TargetPlatform};

use super::installer::download_https_to_file_capped;
use super::llvm_manifest::{llvm_for, Compression, LlvmAsset};
use super::ToolchainManager;

const CHECK_TIMEOUT: Duration = Duration::from_secs(120);

const SELF_TEST_CPP: &str = "#include <cstdio>\n#include <span>\n#include <vector>\n\
int main() { std::vector<int> v{1, 2, 3}; std::span<int> s{v}; \
std::printf(\"ok %d\\n\", s.front() + s.back()); }\n";

pub(super) async fn install_llvm_local(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    entry: &CatalogSdkEntry,
) -> std::result::Result<String, String> {
    let platform = TargetPlatform::host();
    let asset = llvm_for(platform)
        .filter(|a| a.version == entry.version)
        .ok_or_else(|| {
            format!(
                "No managed LLVM build is published for {}/{}. Install clang with your package manager and add it with Add local toolchain.",
                platform.os.as_str(),
                platform.arch.as_str()
            )
        })?;
    let sdkroot = preflight(asset, root).await?;
    let archive = root.join(".downloads").join(asset.asset);
    let staging = root.join("cpp").join(format!(".staging-{job_id}"));
    let res = install_steps(
        mgr, job_id, root, asset, &entry.url, &archive, &staging, sdkroot,
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_file(&archive).await;
    res
}

#[allow(clippy::too_many_arguments)]
async fn install_steps(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    asset: &LlvmAsset,
    url: &str,
    archive: &Path,
    staging: &Path,
    sdkroot: Option<String>,
) -> std::result::Result<String, String> {
    download_https_to_file_capped(
        mgr,
        job_id,
        url,
        archive,
        Some(asset.sha256),
        15,
        asset.bytes,
    )
    .await
    .map_err(|e| {
        if e.contains("checksum mismatch") {
            "The downloaded LLVM archive does not match the expected file (SHA-256 mismatch). Nothing was installed.".to_string()
        } else if e.contains("byte limit") {
            "The downloaded LLVM archive does not match the expected file (size mismatch). Nothing was installed.".to_string()
        } else {
            format!("Download stopped. Nothing was installed. ({e})")
        }
    })?;
    mgr.log_progress(
        job_id,
        72,
        "Extracting clang, clangd, clang-format, clang-tidy…",
    );
    let _ = tokio::fs::remove_dir_all(staging).await;
    tokio::fs::create_dir_all(staging)
        .await
        .map_err(|e| format!("create {}: {e}", staging.display()))?;
    let (arc, stg, a) = (archive.to_path_buf(), staging.to_path_buf(), *asset);
    match asset.compression {
        Compression::Zst => {
            tokio::task::spawn_blocking(move || unpack_zst(&arc, a.top_dir, &a.allowlist(), &stg))
                .await
                .map_err(|e| e.to_string())??
        }
        Compression::Xz => unpack_xz_with_tar(archive, asset, staging).await?,
    }
    let home = staging.join(asset.top_dir);
    let (h, a) = (home.clone(), *asset);
    tokio::task::spawn_blocking(move || check_members(&h, &a))
        .await
        .map_err(|e| e.to_string())??;
    let _ = tokio::fs::remove_file(archive).await;

    mgr.log_progress(job_id, 95, "Checking tools…");
    self_check(mgr, job_id, asset, &home, staging, sdkroot.as_deref()).await?;

    let dest = root.join("cpp/sdks").join(asset.dir_name());
    tokio::fs::create_dir_all(root.join("cpp/sdks"))
        .await
        .map_err(|e| e.to_string())?;
    let _ = tokio::fs::remove_dir_all(&dest).await;
    tokio::fs::rename(&home, &dest)
        .await
        .map_err(|e| format!("move LLVM into {}: {e}", dest.display()))?;
    mgr.log_progress(job_id, 99, format!("LLVM {} installed", asset.version));
    Ok(dest.to_string_lossy().into_owned())
}

/// Prerequisites, checked before anything is downloaded. Returns the macOS SDK
/// path (`SDKROOT` for the self-check).
async fn preflight(asset: &LlvmAsset, root: &Path) -> std::result::Result<Option<String>, String> {
    let sdkroot = if asset.os == TargetOs::Macos {
        let sdk = capture("xcrun", &["--show-sdk-path"], &[]).await.ok();
        let sdk = sdk.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if sdk.is_none() {
            return Err("LLVM needs Apple's Command Line Tools (SDK and linker). Install them (xcode-select --install), then try again.".into());
        }
        sdk
    } else {
        let gxx = capture("g++", &["--version"], &[]).await.is_ok()
            || capture("gcc", &["-print-libgcc-file-name"], &[])
                .await
                .is_ok();
        if !gxx {
            return Err("Clang on Linux uses the system C++ runtime. Install build-essential (apt) or gcc-c++ (dnf), then try again.".into());
        }
        None
    };
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|e| e.to_string())?;
    let need = (asset.bytes + asset.installed_bytes) / 10 * 11;
    let root_s = root.to_string_lossy().into_owned();
    if let Some(free) = capture("df", &["-Pk", &root_s], &[])
        .await
        .ok()
        .and_then(|out| parse_df_free_bytes(&out))
    {
        if free < need {
            return Err(format!(
                "LLVM needs {} free while installing; {} has {}.",
                gb(need),
                root.display(),
                gb(free)
            ));
        }
    }
    Ok(sdkroot)
}

/// Free bytes from `df -Pk <dir>` (POSIX: the 4th column of the 2nd line, in KiB).
fn parse_df_free_bytes(out: &str) -> Option<u64> {
    let kib: u64 = out
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(kib * 1024)
}

fn gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        super::llvm_manifest::mb(bytes)
    }
}

fn is_allowed(allow: &[String], rel: &str) -> bool {
    allow.iter().any(|a| {
        if let Some(dir) = a.strip_suffix('/') {
            rel == dir || rel.starts_with(a.as_str())
        } else if let Some(prefix) = a.strip_suffix('*') {
            rel.starts_with(prefix)
        } else {
            rel == a
        }
    })
}

/// Stream the `.tar.zst` and write only allowlisted members under `staging`.
/// Regular files, directories and symlinks that stay inside their directory;
/// `unpack_in` itself refuses absolute paths and `..`.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
fn unpack_zst(
    archive: &Path,
    top_dir: &str,
    allow: &[String],
    staging: &Path,
) -> std::result::Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("open archive: {e}"))?;
    let mut dec = zstd::stream::read::Decoder::new(file).map_err(|e| format!("zstd: {e}"))?;
    // Upstream compresses with a 1 GiB window (`zstd --long=30`); decoding
    // holds that window in memory until the stream ends.
    dec.window_log_max(30).map_err(|e| format!("zstd: {e}"))?;
    let mut tar = tar::Archive::new(dec);
    for entry in tar.entries().map_err(|e| format!("read archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("read archive: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("archive path: {e}"))?
            .into_owned();
        let Ok(rel) = path.strip_prefix(top_dir) else {
            continue;
        };
        if !is_allowed(allow, &rel.to_string_lossy()) {
            continue;
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| format!("archive link: {e}"))?
                .unwrap_or_default();
            if target.is_absolute()
                || target
                    .components()
                    .any(|c| matches!(c, Component::ParentDir))
            {
                return Err(format!(
                    "archive symlink {} points outside its folder",
                    path.display()
                ));
            }
        } else if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        if !entry
            .unpack_in(staging)
            .map_err(|e| format!("unpack {}: {e}", path.display()))?
        {
            return Err(format!("archive path {} is unsafe", path.display()));
        }
    }
    Ok(())
}

/// Intel Macs: LLVM 20.x ships `.tar.xz` only; macOS bsdtar has liblzma.
async fn unpack_xz_with_tar(
    archive: &Path,
    asset: &LlvmAsset,
    staging: &Path,
) -> std::result::Result<(), String> {
    let mut args = vec![
        "-xJf".to_string(),
        archive.to_string_lossy().into_owned(),
        "-C".to_string(),
        staging.to_string_lossy().into_owned(),
    ];
    for m in asset.allowlist() {
        args.push(format!("{}/{}", asset.top_dir, m.trim_end_matches('/')));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    capture("tar", &refs, &[])
        .await
        .map(|_| ())
        .map_err(|e| format!("unpack LLVM archive: {e}"))
}

/// Every allowlisted member must have been unpacked.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
fn check_members(home: &Path, asset: &LlvmAsset) -> std::result::Result<(), String> {
    for m in asset.allowlist() {
        let present = if let Some(prefix) = m.strip_suffix('*') {
            let p = home.join(prefix);
            let (dir, name) = (
                p.parent().unwrap_or(home),
                p.file_name().unwrap_or_default(),
            );
            std::fs::read_dir(dir).is_ok_and(|rd| {
                rd.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&*name.to_string_lossy())
                })
            })
        } else {
            std::fs::symlink_metadata(home.join(m.trim_end_matches('/'))).is_ok()
        };
        if !present {
            return Err(format!(
                "The LLVM {} archive has no {m}. Nothing was installed.",
                asset.version
            ));
        }
    }
    Ok(())
}

/// Run each tool and compile + run a C++20 program in staging before the
/// install is accepted.
async fn self_check(
    mgr: &ToolchainManager,
    job_id: &str,
    asset: &LlvmAsset,
    home: &Path,
    staging: &Path,
    sdkroot: Option<&str>,
) -> std::result::Result<(), String> {
    let bin = home.join("bin");
    let env: Vec<(&str, &str)> = sdkroot.map(|s| ("SDKROOT", s)).into_iter().collect();
    let fail = |tool: &str, err: String| {
        format!(
            "LLVM {} was downloaded but {tool} does not run on this machine: {}. Nothing was installed.",
            asset.version,
            err.lines().next().unwrap_or("no output")
        )
    };
    let clang = bin.join("clang");
    let v = capture_path(&clang, &["--version"], &env)
        .await
        .map_err(|e| fail("clang", e))?;
    if !v.contains(asset.version) {
        return Err(fail(
            "clang",
            format!("it reports {}", v.lines().next().unwrap_or("")),
        ));
    }
    let first = |out: &str| out.lines().next().unwrap_or("").to_string();
    mgr.log_progress(job_id, 95, first(&v));
    for tool in ["clangd", "clang-format", "clang-tidy"] {
        let out = capture_path(&bin.join(tool), &["--version"], &env)
            .await
            .map_err(|e| fail(tool, e))?;
        mgr.log_progress(job_id, 95, first(&out));
    }
    if asset.os == TargetOs::Linux {
        capture_path(&bin.join("lldb-dap"), &["--help"], &env)
            .await
            .map_err(|e| fail("lldb-dap", e))?;
    }
    let src = staging.join("selftest.cpp");
    let exe = staging.join("selftest");
    tokio::fs::write(&src, SELF_TEST_CPP)
        .await
        .map_err(|e| e.to_string())?;
    let (src_s, exe_s) = (src.to_string_lossy(), exe.to_string_lossy());
    capture_path(
        &bin.join("clang++"),
        &["-std=c++20", &src_s, "-o", &exe_s],
        &env,
    )
    .await
    .map_err(|e| fail("clang++", e))?;
    let out = capture_path(&exe, &[], &env)
        .await
        .map_err(|e| fail("a program built by clang++", e))?;
    if out.trim() != "ok 4" {
        return Err(fail(
            "a program built by clang++",
            format!("it printed {out:?}"),
        ));
    }
    mgr.log_progress(
        job_id,
        98,
        "clang++ -std=c++20 compiled and ran a test program",
    );
    Ok(())
}

async fn capture(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::result::Result<String, String> {
    capture_path(&PathBuf::from(program), args, env).await
}

/// Run `program` and return stdout; on failure the error is its stderr (or the
/// spawn / exit reason).
async fn capture_path(
    program: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::result::Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = tokio::time::timeout(CHECK_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("{} timed out", program.display()))?
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() {
        format!("exited with {}", out.status)
    } else {
        err
    })
}

#[cfg(test)]
#[path = "llvm_install_tests.rs"]
mod tests;

//! Phased Android SDK / Emulator installation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::android::create_avd;
use crate::android_env::{discover_java_home, parse_avd_list, with_android_sdk_env};
use crate::detect::{detect_android, ResolvedAndroidTools};
use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::SimInstallStage;

use super::{
    append_log, default_android_system_image, is_writable_dir, managed_android_sdk_dir,
    update_stage, validate_https_download_url, ArchiveExtractor, ArchiveFetcher, InstallInner,
    ANDROID_CMDLINETOOLS_MAC_URL, MAX_CMDLINETOOLS_BYTES,
};

pub(crate) struct AndroidInstallCtx<'a> {
    pub state: &'a Arc<Mutex<InstallInner>>,
    pub run_id: u64,
    pub data_dir: &'a Path,
    pub runner: &'a dyn CommandRunner,
    pub probe: &'a dyn HostProbe,
    pub adb_lock: &'a Mutex<()>,
    pub fetcher: &'a dyn ArchiveFetcher,
    pub extractor: &'a dyn ArchiveExtractor,
    pub full_emulator: bool,
    pub create_default_avds: bool,
}

/// Remove a per-run staging directory on drop (including error / cancel paths).
struct StagingDirGuard(PathBuf);

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        #[allow(clippy::disallowed_methods)] // sync Drop cleanup of small per-run staging dir
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) async fn run_android_install(ctx: &AndroidInstallCtx<'_>) -> Result<String, String> {
    let managed_sdk = managed_android_sdk_dir(ctx.data_dir);
    let (detected, _) =
        detect_android(ctx.runner, ctx.probe, ctx.adb_lock, Some(&managed_sdk)).await;

    let sdk_root = match &detected.sdk_root {
        Some(p) if is_writable_dir(p).await => p.clone(),
        _ => managed_sdk,
    };
    let java_home = discover_java_home(ctx.runner, ctx.probe).await;

    let sdkmanager_path = ensure_sdkmanager(ctx, &detected, &sdk_root).await?;
    accept_licenses(ctx, &sdkmanager_path, &sdk_root, java_home.as_deref()).await?;
    install_sdk_packages(ctx, &sdkmanager_path, &sdk_root, java_home.as_deref()).await?;

    let avd_note = if ctx.full_emulator && ctx.create_default_avds {
        let summary = create_default_avds(ctx, &sdkmanager_path, &sdk_root).await;
        if summary.created == 0 && summary.existing == 0 && !summary.failed.is_empty() {
            return Err(format!(
                "Android SDK installed at {}, but creating default AVDs failed: {}",
                sdk_root.display(),
                summary.failed.join("; ")
            ));
        }
        if !summary.failed.is_empty() {
            Some(format!(
                " ({} AVD(s) ready; {} failed: {})",
                summary.created + summary.existing,
                summary.failed.len(),
                summary.failed.join("; ")
            ))
        } else {
            None
        }
    } else {
        None
    };

    Ok(format!(
        "{} installed at {}.{note}",
        if ctx.full_emulator {
            "Android Emulator & SDK"
        } else {
            "Android Platform Tools (adb)"
        },
        sdk_root.display(),
        note = avd_note.unwrap_or_default()
    ))
}

async fn ensure_sdkmanager(
    ctx: &AndroidInstallCtx<'_>,
    detected: &ResolvedAndroidTools,
    sdk_root: &Path,
) -> Result<PathBuf, String> {
    if let Some(existing) = &detected.sdkmanager {
        return Ok(existing.clone());
    }
    let candidate = sdk_root.join("cmdline-tools/latest/bin/sdkmanager");
    if ctx.probe.is_executable(&candidate) {
        return Ok(candidate);
    }

    let url = ctx
        .probe
        .env_var("OTTO_ANDROID_CMDLINETOOLS_URL")
        .unwrap_or_else(|| ANDROID_CMDLINETOOLS_MAC_URL.to_string());
    validate_https_download_url(&url)?;
    update_stage(
        ctx.state,
        ctx.run_id,
        SimInstallStage::Downloading,
        Some(5),
        format!(
            "Downloading Android Command-Line Tools into {}…",
            sdk_root.display()
        ),
    )
    .await;

    // Stage inside `<sdk_root>/cmdline-tools/.otto-staging-<run_id>` so the
    // final rename into `cmdline-tools/latest` always stays on the same
    // filesystem (avoiding cross-volume EXDEV errors) with RAII cleanup.
    let target_parent = sdk_root.join("cmdline-tools");
    tokio::fs::create_dir_all(&target_parent)
        .await
        .map_err(|e| format!("create cmdline-tools dir: {e}"))?;
    let staging_dir = target_parent.join(format!(".otto-staging-{}", ctx.run_id));
    let _staging_guard = StagingDirGuard(staging_dir.clone());
    let zip_path = staging_dir.join("commandlinetools.zip");
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .map_err(|e| format!("create staging dir: {e}"))?;

    download_archive(ctx, &url, &zip_path).await?;

    update_stage(
        ctx.state,
        ctx.run_id,
        SimInstallStage::Extracting,
        Some(38),
        "Extracting Android Command-Line Tools…",
    )
    .await;

    let unpack_dir = staging_dir.join("unpacked");
    ctx.extractor.extract_zip(&zip_path, &unpack_dir).await?;

    // Non-destructive atomic swap into `<sdk_root>/cmdline-tools/latest`:
    // keep any previous `latest` as a backup until the new dir is in place.
    let src_ct = unpack_dir.join("cmdline-tools");
    let target_latest = target_parent.join("latest");
    let backup = target_parent.join(format!("latest.bak-{}", ctx.run_id));
    if src_ct.is_dir() {
        let had_existing = target_latest.exists();
        if had_existing {
            let _ = tokio::fs::remove_dir_all(&backup).await;
            tokio::fs::rename(&target_latest, &backup)
                .await
                .map_err(|e| format!("backup existing cmdline-tools/latest: {e}"))?;
        }
        if let Err(e) = tokio::fs::rename(&src_ct, &target_latest).await {
            if had_existing {
                let _ = tokio::fs::rename(&backup, &target_latest).await;
            }
            return Err(format!("install cmdline-tools/latest: {e}"));
        }
        if had_existing {
            let _ = tokio::fs::remove_dir_all(&backup).await;
        }
    }
    Ok(target_latest.join("bin/sdkmanager"))
}

async fn download_archive(
    ctx: &AndroidInstallCtx<'_>,
    url: &str,
    zip_path: &Path,
) -> Result<(), String> {
    let (total_len, mut stream) = ctx.fetcher.open(url).await?;
    if let Some(len) = total_len {
        if len > MAX_CMDLINETOOLS_BYTES {
            return Err(format!(
                "command-line tools archive exceeds size cap ({len} bytes)"
            ));
        }
    }
    let mut file = tokio::fs::File::create(zip_path)
        .await
        .map_err(|e| format!("create zip file: {e}"))?;
    let mut received: u64 = 0;
    let mut last_tick = Instant::now();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .map_err(|_| "download stalled (no data for 60s)".to_string())?
    {
        let bytes = chunk.map_err(|e| format!("download interrupted: {e}"))?;
        received = received.saturating_add(bytes.len() as u64);
        if received > MAX_CMDLINETOOLS_BYTES {
            return Err("download exceeded size cap".into());
        }
        file.write_all(&bytes)
            .await
            .map_err(|e| format!("write archive: {e}"))?;
        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
            let pct = total_len
                .filter(|&t| t > 0)
                .map(|t| (5 + ((received * 30) / t).min(30)) as u8);
            let mut g = ctx.state.lock().await;
            if g.run_id == ctx.run_id {
                if let Some(p) = pct {
                    g.status.progress_pct = Some(p);
                }
                g.status.message = format!(
                    "Downloading Android Command-Line Tools ({:.1} MB)…",
                    (received as f64) / (1024.0 * 1024.0)
                );
                g.status.updated_at = chrono::Utc::now().to_rfc3339();
            }
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("flush archive: {e}"))?;
    Ok(())
}

async fn accept_licenses(
    ctx: &AndroidInstallCtx<'_>,
    sdkmanager_path: &Path,
    sdk_root: &Path,
    java_home: Option<&str>,
) -> Result<(), String> {
    update_stage(
        ctx.state,
        ctx.run_id,
        SimInstallStage::Licensing,
        Some(45),
        "Accepting Android SDK licenses…",
    )
    .await;
    let sdk_flag = format!("--sdk_root={}", sdk_root.display());
    let spec = with_android_sdk_env(
        CmdSpec::new(sdkmanager_path.to_string_lossy(), [&sdk_flag, "--licenses"])
            .with_stdin("y\n".repeat(32).into_bytes())
            .with_timeout(Duration::from_secs(60)),
        Some(sdk_root),
        java_home,
    );
    let out = ctx
        .runner
        .run(spec)
        .await
        .map_err(|e| format_sdkmanager_error(&e.to_string(), java_home.is_some()))?;
    if !out.success() {
        return Err(format_sdkmanager_error(
            &out.combined_err(),
            java_home.is_some(),
        ));
    }
    Ok(())
}

async fn install_sdk_packages(
    ctx: &AndroidInstallCtx<'_>,
    sdkmanager_path: &Path,
    sdk_root: &Path,
    java_home: Option<&str>,
) -> Result<(), String> {
    let sys_img = default_android_system_image();
    let sdk_flag = format!("--sdk_root={}", sdk_root.display());
    let mut pkg_args = vec![sdk_flag, "platform-tools".to_string()];
    if ctx.full_emulator {
        pkg_args.push("emulator".into());
        pkg_args.push(sys_img.into());
    }
    update_stage(
        ctx.state,
        ctx.run_id,
        SimInstallStage::InstallingPackages,
        Some(55),
        if ctx.full_emulator {
            format!("Installing platform-tools, emulator, and {sys_img}…")
        } else {
            "Installing Android platform-tools (adb)…".into()
        },
    )
    .await;

    let spec = with_android_sdk_env(
        CmdSpec::new(sdkmanager_path.to_string_lossy(), pkg_args)
            .with_stdin("y\n".repeat(16).into_bytes())
            .with_timeout(Duration::from_secs(900)),
        Some(sdk_root),
        java_home,
    );
    let out = ctx
        .runner
        .run(spec)
        .await
        .map_err(|e| format_sdkmanager_error(&e.to_string(), java_home.is_some()))?;
    if !out.success() {
        return Err(format_sdkmanager_error(
            &out.combined_err(),
            java_home.is_some(),
        ));
    }
    let stdout = out.stdout_str();
    let tail: Vec<&str> = stdout.lines().rev().take(6).collect();
    for line in tail.into_iter().rev() {
        append_log(ctx.state, ctx.run_id, line, Some(80)).await;
    }
    Ok(())
}

struct AvdCreationSummary {
    created: usize,
    existing: usize,
    failed: Vec<String>,
}

async fn create_default_avds(
    ctx: &AndroidInstallCtx<'_>,
    sdkmanager_path: &Path,
    sdk_root: &Path,
) -> AvdCreationSummary {
    let emulator_bin = sdk_root.join("emulator/emulator");
    let existing_avds = ctx
        .runner
        .run(
            CmdSpec::new(emulator_bin.to_string_lossy(), ["-list-avds"])
                .with_timeout(Duration::from_secs(10)),
        )
        .await
        .ok()
        .filter(|o| o.success())
        .map(|o| parse_avd_list(&o.stdout_str()).len())
        .unwrap_or(0);
    if existing_avds > 0 {
        return AvdCreationSummary {
            created: 0,
            existing: existing_avds,
            failed: Vec::new(),
        };
    }

    let avdmanager_path = sdkmanager_path
        .parent()
        .map(|p| p.join("avdmanager"))
        .unwrap_or_else(|| sdk_root.join("cmdline-tools/latest/bin/avdmanager"));
    let tools = ResolvedAndroidTools {
        sdk_root: Some(sdk_root.to_path_buf()),
        studio_app: None,
        adb: Some(sdk_root.join("platform-tools/adb")),
        emulator: Some(emulator_bin),
        avdmanager: Some(avdmanager_path),
        sdkmanager: Some(sdkmanager_path.to_path_buf()),
    };
    let sys_img = default_android_system_image();

    update_stage(
        ctx.state,
        ctx.run_id,
        SimInstallStage::CreatingAvds,
        Some(88),
        "Creating default Otto_Pixel_Phone and Otto_Pixel_Tablet AVDs…",
    )
    .await;

    let mut summary = AvdCreationSummary {
        created: 0,
        existing: 0,
        failed: Vec::new(),
    };

    for (avd_name, profile) in [
        ("Otto_Pixel_Phone", "pixel_8"),
        ("Otto_Pixel_Tablet", "pixel_tablet"),
    ] {
        match create_avd(
            ctx.runner,
            ctx.probe,
            &tools,
            avd_name,
            sys_img,
            Some(profile),
        )
        .await
        {
            Ok(()) => {
                summary.created += 1;
                append_log(
                    ctx.state,
                    ctx.run_id,
                    &format!("Created AVD {avd_name} ({profile})."),
                    Some(95),
                )
                .await;
            }
            Err(first_err) => {
                // Retry without `-d <profile>` in case profile ID changed across cmdline-tools releases.
                match create_avd(ctx.runner, ctx.probe, &tools, avd_name, sys_img, None).await {
                    Ok(()) => {
                        summary.created += 1;
                        append_log(
                            ctx.state,
                            ctx.run_id,
                            &format!("Created AVD {avd_name} (default profile)."),
                            Some(95),
                        )
                        .await;
                    }
                    Err(retry_err) => {
                        let detail = format!("{avd_name}: {retry_err} (initial: {first_err})");
                        summary.failed.push(detail.clone());
                        append_log(
                            ctx.state,
                            ctx.run_id,
                            &format!("Error creating {detail}"),
                            None,
                        )
                        .await;
                    }
                }
            }
        }
    }

    summary
}

fn format_sdkmanager_error(raw: &str, has_java: bool) -> String {
    let lc = raw.to_ascii_lowercase();
    if !has_java
        || lc.contains("unable to locate a java runtime")
        || lc.contains("java_home")
        || lc.contains("unsupportedclassversionerror")
    {
        format!(
            "Android sdkmanager requires Java 17+ (install via `brew install openjdk@17`). Detail: {raw}"
        )
    } else {
        raw.to_string()
    }
}

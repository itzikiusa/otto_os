//! The Unity debugger: DotRush's Mono debug adapter (`monodbg`, MIT,
//! https://github.com/JaneySprings/DotRush). Its release bundle also carries
//! Microsoft's `Microsoft.VisualStudio.Shared.VSCodeDebugProtocol.dll` under
//! Microsoft's own licence terms, so Otto never bundles it: this job downloads the
//! pinned upstream release on the person's request and checks its SHA-256.
//! `monodbg` is framework-dependent `net10.0` (`rollForward: LatestMajor`), so
//! the job also installs the .NET 10 SDK when no .NET 10+ runtime is found.
//! This Mac only (the Unity Editor is a desktop app).
//!
//! Updating the pin is a deliberate change that reruns the live suite
//! (`tests/unity_attach_live.rs`).

use std::path::{Path, PathBuf};

use super::installer::{download_https_to_file, install_local_sdk, relink_current, run_local_cmd};
use super::ToolchainManager;
use crate::toolchains_catalog::{resolve_sdk_download, EcosystemKind, TargetPlatform};

/// Pinned DotRush release (tag) of `DotRush.Bundle.DebuggerMono.zip`.
pub const DOTRUSH_MONODBG_RELEASE: &str = "2026.11";
/// SHA-256 of the 2026.11 bundle (8,756,344 bytes; GitHub's published asset digest).
pub const DOTRUSH_MONODBG_SHA256: &str =
    "66675e46198705d0b6d25fdf938f33c9ebc472353b7bb9a085d1e044e1e84f71";

pub fn monodbg_url() -> String {
    format!(
        "https://github.com/JaneySprings/DotRush/releases/download/{DOTRUSH_MONODBG_RELEASE}/DotRush.Bundle.DebuggerMono.zip"
    )
}

/// `<toolchains>/dotnet/monodbg/<release>/monodbg.dll`.
pub(crate) fn monodbg_dll(toolchains: &Path) -> PathBuf {
    toolchains
        .join("dotnet/monodbg")
        .join(DOTRUSH_MONODBG_RELEASE)
        .join("monodbg.dll")
}

/// The Unity debugger row of the .NET toolchain (status only; this Mac).
pub(crate) fn unity_dap_status(toolchains: &Path) -> crate::types::IdeToolBinaryStatus {
    let dll = monodbg_dll(toolchains);
    let available = dll.is_file();
    crate::types::IdeToolBinaryStatus {
        name: "DotRush Mono debugger".into(),
        available,
        path: available.then(|| dll.to_string_lossy().into_owned()),
        version: Some(DOTRUSH_MONODBG_RELEASE.into()),
        outdated: None,
    }
}

/// A `dotnet` muxer that has a `Microsoft.NETCore.App` runtime 10 or newer:
/// Otto's managed .NET first, then `DOTNET_ROOT`, `PATH` and the usual installs.
pub(crate) async fn find_dotnet_10(
    toolchains: &Path,
    dotnet_root: Option<&str>,
    path_var: &str,
) -> Option<String> {
    let mut cands: Vec<PathBuf> = vec![toolchains.join("dotnet/current/dotnet")];
    // Otto's other managed SDKs: the Unity job adds .NET 10 there without making
    // it the current SDK. Newest version first.
    if let Ok(mut rd) = tokio::fs::read_dir(toolchains.join("dotnet/sdks")).await {
        let mut sdks = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            sdks.push(e.path().join("dotnet"));
        }
        sdks.sort_by(|a, b| b.cmp(a));
        cands.extend(sdks);
    }
    if let Some(dr) = dotnet_root.map(str::trim).filter(|s| !s.is_empty()) {
        cands.push(Path::new(dr).join("dotnet"));
    }
    cands.extend(
        path_var
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join("dotnet")),
    );
    cands.push("/usr/local/share/dotnet/dotnet".into());
    cands.push("/opt/homebrew/opt/dotnet/libexec/dotnet".into());
    for cand in cands {
        if !cand.is_file() {
            continue;
        }
        let Ok(out) = tokio::process::Command::new(&cand)
            .arg("--list-runtimes")
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .kill_on_drop(true)
            .output()
            .await
        else {
            continue;
        };
        if has_runtime_10(&String::from_utf8_lossy(&out.stdout)) {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

/// `dotnet --list-runtimes` lists a `Microsoft.NETCore.App` 10 or newer.
pub(crate) fn has_runtime_10(list: &str) -> bool {
    list.lines().any(|l| {
        l.strip_prefix("Microsoft.NETCore.App ")
            .and_then(|v| v.split('.').next())
            .and_then(|m| m.parse::<u32>().ok())
            .is_some_and(|major| major >= 10)
    })
}

/// Install the pinned `monodbg` (and the .NET 10 SDK when needed); returns the
/// path of `monodbg.dll`.
pub(super) async fn install_local_unity_dap(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
) -> std::result::Result<String, String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    if find_dotnet_10(root, None, &path_var).await.is_none() {
        let entry = resolve_sdk_download(
            EcosystemKind::Dotnet,
            // The catalog default, .NET 10 LTS.
            Some(EcosystemKind::Dotnet.default_sdk_version()),
            TargetPlatform::host(),
            &[],
        )?;
        mgr.log_progress(
            job_id,
            10,
            format!(
                "The Unity debugger needs .NET 10: installing {}...",
                entry.label
            ),
        );
        // The person's current .NET stays current: only the debugger uses this one.
        let current = root.join("dotnet/current");
        let before = tokio::fs::read_link(&current).await.ok();
        let installed = install_local_sdk(mgr, job_id, root, EcosystemKind::Dotnet, &entry).await;
        restore_current_link(&current, before).await;
        installed?;
    }
    mgr.log_progress(
        job_id,
        70,
        format!("Downloading DotRush Mono debugger {DOTRUSH_MONODBG_RELEASE} (MIT; includes Microsoft components under their own licence terms)..."),
    );
    let stage = root
        .join(".downloads")
        .join(format!("monodbg-stage-{job_id}"));
    let _ = tokio::fs::remove_dir_all(&stage).await;
    let extract = stage.join("extract");
    tokio::fs::create_dir_all(&extract)
        .await
        .map_err(|e| e.to_string())?;
    let zip = stage.join("monodbg.zip");
    let res = async {
        download_https_to_file(
            mgr,
            job_id,
            &monodbg_url(),
            &zip,
            Some(DOTRUSH_MONODBG_SHA256),
            75,
        )
        .await?;
        run_local_cmd(
            mgr,
            job_id,
            92,
            Path::new("unzip"),
            &[
                "-q",
                "-o",
                &zip.to_string_lossy(),
                "-d",
                &extract.to_string_lossy(),
            ],
            &[],
        )
        .await?;
        if !extract.join("monodbg.dll").is_file() {
            return Err("DotRush bundle has no monodbg.dll".to_string());
        }
        let dest = monodbg_dll(root);
        let dest_dir = dest.parent().ok_or("bad monodbg path")?;
        let _ = tokio::fs::remove_dir_all(dest_dir).await;
        if let Some(parent) = dest_dir.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::rename(&extract, dest_dir)
            .await
            .map_err(|e| e.to_string())?;
        Ok(dest.to_string_lossy().into_owned())
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&stage).await;
    res
}

/// Put `dotnet/current` back as it was (`None`: there was no link).
async fn restore_current_link(link: &Path, before: Option<PathBuf>) {
    match before {
        Some(prev) => relink_current(link, &prev),
        None => {
            let _ = tokio::fs::remove_file(link).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_a_runtime_of_major_10_or_newer() {
        let old = "Microsoft.AspNetCore.App 10.0.15 [/x]\nMicrosoft.NETCore.App 8.0.34 [/x]\nMicrosoft.NETCore.App 9.0.23 [/x]\n";
        assert!(!has_runtime_10(old));
        assert!(has_runtime_10("Microsoft.NETCore.App 10.0.15 [/usr/local/share/dotnet/shared/Microsoft.NETCore.App]\n"));
        assert!(has_runtime_10("Microsoft.NETCore.App 11.0.0 [/x]\n"));
        assert!(!has_runtime_10(""));
    }

    #[test]
    fn pins_the_dotrush_release_and_installs_under_its_tag() {
        assert_eq!(
            monodbg_url(),
            "https://github.com/JaneySprings/DotRush/releases/download/2026.11/DotRush.Bundle.DebuggerMono.zip"
        );
        assert_eq!(DOTRUSH_MONODBG_SHA256.len(), 64);
        assert!(monodbg_dll(Path::new("/tc")).ends_with("dotnet/monodbg/2026.11/monodbg.dll"));
        // monodbg is net10.0: the SDK the job adds must carry a .NET 10+ runtime.
        let major: u32 = EcosystemKind::Dotnet
            .default_sdk_version()
            .split('.')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(major >= 10);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finds_dotnet_10_installed_beside_the_current_sdk() {
        // SEC6-4: the Unity job installs .NET 10 without making it the current SDK.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dotnet = tmp.path().join("dotnet/sdks/10.0.404/dotnet");
        std::fs::create_dir_all(dotnet.parent().unwrap()).unwrap();
        std::fs::write(
            &dotnet,
            "#!/bin/sh\necho 'Microsoft.NETCore.App 10.0.15 [/x/shared/Microsoft.NETCore.App]'\n",
        )
        .unwrap();
        std::fs::set_permissions(&dotnet, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_dotnet_10(tmp.path(), None, "").await.as_deref(),
            Some(dotnet.to_string_lossy().as_ref())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_current_sdk_link_is_put_back() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("dotnet/current");
        let (old, new) = (tmp.path().join("8.0"), tmp.path().join("10.0"));
        relink_current(&link, &old);
        let before = tokio::fs::read_link(&link).await.ok();
        relink_current(&link, &new);
        restore_current_link(&link, before).await;
        assert_eq!(std::fs::read_link(&link).unwrap(), old);
        restore_current_link(&link, None).await;
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "no link before means none after"
        );
    }
}

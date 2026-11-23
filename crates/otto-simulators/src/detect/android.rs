//! Android SDK / Emulator / ADB host detection and readiness classification.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::Mutex;

use crate::android_env::{ensure_adb_server, parse_avd_list};
use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::{
    IssueSeverity, Platform, PlatformReadiness, ReadinessCode, ReadinessIssue, ReadinessState,
    SetupStep, ToolStatus,
};

use super::ResolvedAndroidTools;

/// Probe Android SDK / Emulator / ADB state on the host.
pub async fn detect_android(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    adb_lock: &Mutex<()>,
    managed_android_sdk: Option<&Path>,
) -> (ResolvedAndroidTools, PlatformReadiness) {
    let studio_app = {
        let p = PathBuf::from("/Applications/Android Studio.app");
        probe.path_exists(&p).then_some(p)
    };
    let sdk_root = find_android_sdk_root(probe, managed_android_sdk);
    let adb = resolve_android_tool(probe, sdk_root.as_deref(), &["platform-tools/adb"], "adb");
    let emulator = resolve_android_tool(
        probe,
        sdk_root.as_deref(),
        &["emulator/emulator"],
        "emulator",
    );
    let avdmanager = resolve_android_tool(
        probe,
        sdk_root.as_deref(),
        &[
            "cmdline-tools/latest/bin/avdmanager",
            "cmdline-tools/bin/avdmanager",
            "tools/bin/avdmanager",
        ],
        "avdmanager",
    );
    let sdkmanager = resolve_android_tool(
        probe,
        sdk_root.as_deref(),
        &[
            "cmdline-tools/latest/bin/sdkmanager",
            "cmdline-tools/bin/sdkmanager",
            "tools/bin/sdkmanager",
        ],
        "sdkmanager",
    );

    let adb_version = probe_adb_version(runner, adb.as_deref()).await;
    let avd_count = probe_avd_count(runner, emulator.as_deref()).await;
    let connected_count = probe_connected_count(runner, adb_lock, adb.as_deref()).await;

    let tools = build_android_tools(
        &studio_app,
        &adb,
        &adb_version,
        &emulator,
        &avdmanager,
        &sdkmanager,
    );
    let (state, code, summary, issues, setup_steps) = classify_android_readiness(
        &sdk_root,
        &studio_app,
        &adb,
        &emulator,
        avd_count,
        connected_count,
    );

    let resolved = ResolvedAndroidTools {
        sdk_root: sdk_root.clone(),
        studio_app,
        adb,
        emulator,
        avdmanager,
        sdkmanager,
    };

    let readiness = PlatformReadiness {
        platform: Platform::Android,
        state,
        code,
        summary,
        sdk_path: sdk_root.map(|p| p.to_string_lossy().into_owned()),
        tools,
        issues,
        setup_steps,
    };

    (resolved, readiness)
}

async fn probe_adb_version(runner: &dyn CommandRunner, adb: Option<&Path>) -> Option<String> {
    let adb_path = adb?;
    let out = runner
        .run(
            CmdSpec::new(adb_path.to_string_lossy(), ["version"])
                .with_timeout(Duration::from_secs(5)),
        )
        .await
        .ok()?;
    if !out.success() {
        return None;
    }
    out.stdout_str()
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

async fn probe_avd_count(runner: &dyn CommandRunner, emulator: Option<&Path>) -> usize {
    let Some(emu_path) = emulator else {
        return 0;
    };
    let Ok(out) = runner
        .run(
            CmdSpec::new(emu_path.to_string_lossy(), ["-list-avds"])
                .with_timeout(Duration::from_secs(8)),
        )
        .await
    else {
        return 0;
    };
    if !out.success() {
        return 0;
    }
    parse_avd_list(&out.stdout_str()).len()
}

async fn probe_connected_count(
    runner: &dyn CommandRunner,
    adb_lock: &Mutex<()>,
    adb: Option<&Path>,
) -> usize {
    let Some(adb_path) = adb else {
        return 0;
    };
    let adb_str = adb_path.to_string_lossy().into_owned();
    ensure_adb_server(runner, adb_lock, &adb_str).await;
    let Ok(out) = runner
        .run(CmdSpec::new(&adb_str, ["devices"]).with_timeout(Duration::from_secs(5)))
        .await
    else {
        return 0;
    };
    if !out.success() {
        return 0;
    }
    out.stdout_str()
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with('*')
                && !l.starts_with("List of devices")
                && !l.starts_with("adb server")
                && l.contains('\t')
        })
        .count()
}

fn build_android_tools(
    studio_app: &Option<PathBuf>,
    adb: &Option<PathBuf>,
    adb_version: &Option<String>,
    emulator: &Option<PathBuf>,
    avdmanager: &Option<PathBuf>,
    sdkmanager: &Option<PathBuf>,
) -> Vec<ToolStatus> {
    vec![
        ToolStatus {
            name: "Android Studio".into(),
            available: studio_app.is_some(),
            path: studio_app
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: None,
        },
        ToolStatus {
            name: "adb".into(),
            available: adb.is_some(),
            path: adb.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: adb_version.clone(),
            detail: None,
        },
        ToolStatus {
            name: "emulator".into(),
            available: emulator.is_some(),
            path: emulator.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: None,
        },
        ToolStatus {
            name: "avdmanager".into(),
            available: avdmanager.is_some(),
            path: avdmanager
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: None,
        },
        ToolStatus {
            name: "sdkmanager".into(),
            available: sdkmanager.is_some(),
            path: sdkmanager
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: None,
        },
    ]
}

fn classify_android_readiness(
    sdk_root: &Option<PathBuf>,
    studio_app: &Option<PathBuf>,
    adb: &Option<PathBuf>,
    emulator: &Option<PathBuf>,
    avd_count: usize,
    connected_count: usize,
) -> (
    ReadinessState,
    ReadinessCode,
    String,
    Vec<ReadinessIssue>,
    Vec<SetupStep>,
) {
    let mut issues = Vec::new();
    let mut steps = Vec::new();

    if sdk_root.is_none() && adb.is_none() && emulator.is_none() {
        if studio_app.is_some() {
            issues.push(ReadinessIssue {
                code: "studio_installed_sdk_missing".into(),
                severity: IssueSeverity::Warning,
                message:
                    "Android Studio is installed, but the Android SDK has not been initialized yet."
                        .into(),
            });
            steps.push(SetupStep {
                title: "Complete Android Studio first-run setup".into(),
                command: Some("open -a \"Android Studio\"".into()),
                url: None,
                detail: "Open Android Studio and complete the SDK Setup Wizard (installs platform-tools, emulator, and a Pixel AVD under ~/Library/Android/sdk).".into(),
            });
            return (
                ReadinessState::Unavailable,
                ReadinessCode::StudioInstalledSdkMissing,
                "Android Studio is installed, but the Android SDK is missing.".into(),
                issues,
                steps,
            );
        }
        issues.push(ReadinessIssue {
            code: "sdk_not_found".into(),
            severity: IssueSeverity::Error,
            message: "Android SDK not found (checked ANDROID_HOME, ANDROID_SDK_ROOT, ~/Library/Android/sdk, Otto managed SDK, and Homebrew).".into(),
        });
        steps.push(SetupStep {
            title: "Install Android Emulator directly in Otto (1-click, no Android Studio required)"
                .into(),
            command: None,
            url: None,
            detail: "Click \"Install Android Emulator\" in Otto to download Google's official Android Command-Line Tools, platform-tools (adb), emulator, Android 15 system image, and default Phone + Tablet AVDs into Otto's managed SDK directory.".into(),
        });
        steps.push(SetupStep {
            title: "Or install CLI tools manually via Homebrew".into(),
            command: Some(
                "brew install --cask android-commandlinetools && sdkmanager \"platform-tools\" \"emulator\" \"system-images;android-35;google_apis;arm64-v8a\""
                    .into(),
            ),
            url: None,
            detail: "Standalone CLI setup without Android Studio.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::SdkNotFound,
            "Android SDK and emulator tools are not installed.".into(),
            issues,
            steps,
        );
    }

    if adb.is_none() {
        issues.push(ReadinessIssue {
            code: "adb_missing".into(),
            severity: IssueSeverity::Error,
            message: "Android SDK found, but platform-tools (adb) is missing.".into(),
        });
        steps.push(SetupStep {
            title: "Install Android platform-tools (adb)".into(),
            command: Some("sdkmanager \"platform-tools\"".into()),
            url: None,
            detail: "Required to inspect devices, open URLs, forward localhost ports, capture screens, and send input.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::AdbMissing,
            "Android platform-tools (adb) is missing.".into(),
            issues,
            steps,
        );
    }

    if emulator.is_none() && connected_count == 0 {
        issues.push(ReadinessIssue {
            code: "emulator_missing".into(),
            severity: IssueSeverity::Warning,
            message: "adb is installed, but the Android Emulator binary is missing and no devices are connected.".into(),
        });
        steps.push(SetupStep {
            title: "Install the Android Emulator package".into(),
            command: Some("sdkmanager \"emulator\"".into()),
            url: None,
            detail: "Installs the Android Emulator so AVDs can be launched locally.".into(),
        });
        return (
            ReadinessState::Degraded,
            ReadinessCode::EmulatorMissing,
            "adb is ready, but Android Emulator is not installed.".into(),
            issues,
            steps,
        );
    }

    if avd_count == 0 && connected_count == 0 {
        issues.push(ReadinessIssue {
            code: "no_avds".into(),
            severity: IssueSeverity::Info,
            message: "Android SDK and emulator are installed, but no AVDs exist yet.".into(),
        });
        steps.push(SetupStep {
            title: "Create an Android Virtual Device (AVD)".into(),
            command: Some(
                "avdmanager create avd -n Pixel_9_API_35 -k \"system-images;android-35;google_apis;arm64-v8a\" -d pixel_9"
                    .into(),
            ),
            url: None,
            detail: "Or use \"+ New Device / AVD\" inside Otto.".into(),
        });
        return (
            ReadinessState::Degraded,
            ReadinessCode::NoAvds,
            "Android SDK is ready, but no AVDs are created yet.".into(),
            issues,
            steps,
        );
    }

    (
        ReadinessState::Ready,
        ReadinessCode::Ready,
        format!("Android ready ({avd_count} AVD(s), {connected_count} connected device(s))."),
        issues,
        steps,
    )
}

fn find_android_sdk_root(
    probe: &dyn HostProbe,
    managed_android_sdk: Option<&Path>,
) -> Option<PathBuf> {
    for env_key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(val) = probe.env_var(env_key) {
            let p = PathBuf::from(val);
            if probe.path_exists(&p) {
                return Some(p);
            }
        }
    }
    if let Some(home) = probe.home_dir() {
        for rel in ["Library/Android/sdk", "Android/Sdk"] {
            let p = home.join(rel);
            if probe.path_exists(&p) {
                return Some(p);
            }
        }
    }
    if let Some(managed) = managed_android_sdk {
        if probe.path_exists(managed) {
            return Some(managed.to_path_buf());
        }
    }
    for sys in [
        "/opt/homebrew/share/android-commandlinetools",
        "/usr/local/share/android-commandlinetools",
    ] {
        let p = PathBuf::from(sys);
        if probe.path_exists(&p) {
            return Some(p);
        }
    }
    None
}

fn resolve_android_tool(
    probe: &dyn HostProbe,
    sdk_root: Option<&Path>,
    relative_candidates: &[&str],
    bin_name: &str,
) -> Option<PathBuf> {
    if let Some(root) = sdk_root {
        for rel in relative_candidates {
            let p = root.join(rel);
            if probe.is_executable(&p) {
                return Some(p);
            }
        }
        if matches!(bin_name, "avdmanager" | "sdkmanager") {
            let ct_dir = root.join("cmdline-tools");
            for entry in probe.list_dir(&ct_dir) {
                let p = ct_dir.join(&entry).join("bin").join(bin_name);
                if probe.is_executable(&p) {
                    return Some(p);
                }
            }
        }
    }
    probe.which(bin_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};

    #[tokio::test]
    async fn detects_android_studio_without_sdk_and_ready_android() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let adb_lock = Mutex::new(());
        probe.add_path("/Applications/Android Studio.app");

        let (_, android) = detect_android(&runner, &probe, &adb_lock, None).await;
        assert_eq!(android.state, ReadinessState::Unavailable);
        assert_eq!(android.code, ReadinessCode::StudioInstalledSdkMissing);

        let sdk = PathBuf::from("/Users/tester/Library/Android/sdk");
        probe.add_path(&sdk);
        probe.add_executable(sdk.join("platform-tools/adb"));
        probe.add_executable(sdk.join("emulator/emulator"));
        probe.add_executable(sdk.join("cmdline-tools/latest/bin/avdmanager"));
        runner.on_exact(
            "adb",
            &["version"],
            ok_out(b"Android Debug Bridge version 1.0.41\n"),
        );
        // WARNING banner with 0 real AVDs must report NoAvds, NOT Ready.
        runner.on_exact(
            "emulator",
            &["-list-avds"],
            ok_out(b"WARNING | Please update the emulator to one that supports the feature(s): Vulkan\n"),
        );
        runner.on_exact("adb", &["devices"], ok_out(b"List of devices attached\n"));
        let (_, no_avds) = detect_android(&runner, &probe, &adb_lock, None).await;
        assert_eq!(no_avds.state, ReadinessState::Degraded);
        assert_eq!(no_avds.code, ReadinessCode::NoAvds);

        // Now with a real AVD present.
        runner.on_exact("emulator", &["-list-avds"], ok_out(b"Pixel_9_API_35\n"));
        let (_, android_ready) = detect_android(&runner, &probe, &adb_lock, None).await;
        assert_eq!(android_ready.state, ReadinessState::Ready);
        assert_eq!(android_ready.code, ReadinessCode::Ready);
        assert!(android_ready.summary.contains("1 AVD"));
    }

    #[tokio::test]
    async fn detects_otto_managed_android_sdk_dir() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let adb_lock = Mutex::new(());
        let managed = PathBuf::from("/Users/tester/Library/Application Support/Otto/android-sdk");
        probe.add_path(&managed);
        probe.add_executable(managed.join("platform-tools/adb"));
        probe.add_executable(managed.join("emulator/emulator"));
        runner.on_exact(
            "adb",
            &["version"],
            ok_out(b"Android Debug Bridge 1.0.41\n"),
        );
        runner.on_exact("emulator", &["-list-avds"], ok_out(b"Otto_Pixel_Phone\n"));
        runner.on_exact("adb", &["devices"], ok_out(b"List of devices attached\n"));

        let (tools, ready) = detect_android(&runner, &probe, &adb_lock, Some(&managed)).await;
        assert_eq!(tools.sdk_root.as_deref(), Some(managed.as_path()));
        assert_eq!(ready.state, ReadinessState::Ready);
    }
}

//! Xcode / iOS Simulator host detection and readiness classification.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::{
    IssueSeverity, Platform, PlatformReadiness, ReadinessCode, ReadinessIssue, ReadinessState,
    SetupStep, ToolStatus,
};

use super::ResolvedIosTools;

struct SimctlProbeResult {
    simctl_ok: bool,
    license_required: bool,
    detail: Option<String>,
    available_ios_runtimes: usize,
    total_devices: usize,
}

/// Count available iOS runtimes in `xcrun simctl list runtimes -j` JSON output.
/// Shared with `install::ios` so runtime installs verify an actual iOS runtime.
pub fn count_available_ios_runtimes(stdout: &[u8]) -> usize {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return 0;
    };
    let Some(arr) = v.get("runtimes").and_then(|r| r.as_array()) else {
        return 0;
    };
    arr.iter()
        .filter(|rt| {
            let avail = rt
                .get("isAvailable")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let id = rt.get("identifier").and_then(|s| s.as_str()).unwrap_or("");
            let name = rt.get("name").and_then(|s| s.as_str()).unwrap_or("");
            avail && (id.contains("iOS") || name.to_ascii_lowercase().starts_with("ios"))
        })
        .count()
}

/// Probe iOS Simulator / Xcode state on the host.
pub async fn detect_ios(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
) -> (ResolvedIosTools, PlatformReadiness) {
    let apps_entries = probe.list_dir(Path::new("/Applications"));
    let xcode_app = find_xcode_app(probe, &apps_entries);
    let xcode_downloading = find_xcode_downloading(probe, &apps_entries);
    let xcrun = probe.which("xcrun").or_else(|| {
        let p = PathBuf::from("/usr/bin/xcrun");
        probe.is_executable(&p).then_some(p)
    });
    let idb = find_idb(probe);

    let developer_dir = probe_developer_dir(runner).await;
    let xcode_version = probe_xcode_version(runner).await;
    let sim = probe_simctl(runner, xcrun.as_deref()).await;

    let tools = build_ios_tools(
        &xcode_app,
        &xcode_downloading,
        &developer_dir,
        &xcode_version,
        &xcrun,
        &idb,
        &sim,
    );
    let (state, code, summary, issues, setup_steps) =
        classify_ios_readiness(&xcode_app, &xcode_downloading, &sim);

    let resolved = ResolvedIosTools {
        xcode_app: xcode_app.clone(),
        xcode_downloading,
        developer_dir: developer_dir.clone(),
        xcrun,
        idb,
        simctl_ok: sim.simctl_ok,
        license_required: sim.license_required,
    };

    let readiness = PlatformReadiness {
        platform: Platform::Ios,
        state,
        code,
        summary,
        sdk_path: developer_dir.or_else(|| xcode_app.map(|p| p.to_string_lossy().into_owned())),
        tools,
        issues,
        setup_steps,
    };

    (resolved, readiness)
}

/// Locate `idb` on PATH or in standard macOS Homebrew / pipx / user bin directories
/// so `launchd`-spawned daemons (minimal PATH) still detect it automatically.
fn find_idb(probe: &dyn HostProbe) -> Option<PathBuf> {
    if let Some(p) = probe.which("idb") {
        return Some(p);
    }
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/idb"),
        PathBuf::from("/usr/local/bin/idb"),
    ];
    if let Some(home) = probe.home_dir() {
        candidates.push(home.join(".local/bin/idb"));
    }
    candidates.into_iter().find(|p| probe.is_executable(p))
}

async fn probe_developer_dir(runner: &dyn CommandRunner) -> Option<String> {
    let out = runner
        .run(CmdSpec::new("xcode-select", ["-p"]).with_timeout(Duration::from_secs(5)))
        .await
        .ok()?;
    if !out.success() {
        return None;
    }
    let p = out.stdout_str().trim().to_string();
    (!p.is_empty()).then_some(p)
}

async fn probe_xcode_version(runner: &dyn CommandRunner) -> Option<String> {
    let out = runner
        .run(CmdSpec::new("xcodebuild", ["-version"]).with_timeout(Duration::from_secs(5)))
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

async fn probe_simctl(runner: &dyn CommandRunner, xcrun: Option<&Path>) -> SimctlProbeResult {
    let mut res = SimctlProbeResult {
        simctl_ok: false,
        license_required: false,
        detail: None,
        available_ios_runtimes: 0,
        total_devices: 0,
    };
    let Some(xcrun_path) = xcrun else {
        return res;
    };
    let xcrun_str = xcrun_path.to_string_lossy().into_owned();
    match runner
        .run(
            CmdSpec::new(&xcrun_str, ["simctl", "list", "runtimes", "-j"])
                .with_timeout(Duration::from_secs(10)),
        )
        .await
    {
        Ok(out) if out.success() => {
            res.simctl_ok = true;
            res.available_ios_runtimes = count_available_ios_runtimes(&out.stdout);
            if let Ok(dev_out) = runner
                .run(
                    CmdSpec::new(&xcrun_str, ["simctl", "list", "devices", "-j"])
                        .with_timeout(Duration::from_secs(10)),
                )
                .await
            {
                if dev_out.success() {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&dev_out.stdout) {
                        if let Some(map) = v.get("devices").and_then(|d| d.as_object()) {
                            for arr in map.values().filter_map(|v| v.as_array()) {
                                res.total_devices += arr
                                    .iter()
                                    .filter(|d| {
                                        d.get("isAvailable")
                                            .and_then(|b| b.as_bool())
                                            .unwrap_or(false)
                                    })
                                    .count();
                            }
                        }
                    }
                }
            }
        }
        Ok(out) => {
            let err = out.combined_err();
            let lc = err.to_ascii_lowercase();
            if lc.contains("license") || lc.contains("xcodebuild -license") {
                res.license_required = true;
            }
            res.detail = Some(err);
        }
        Err(e) => {
            res.detail = Some(e.to_string());
        }
    }
    res
}

fn build_ios_tools(
    xcode_app: &Option<PathBuf>,
    xcode_downloading: &Option<PathBuf>,
    developer_dir: &Option<String>,
    xcode_version: &Option<String>,
    xcrun: &Option<PathBuf>,
    idb: &Option<PathBuf>,
    sim: &SimctlProbeResult,
) -> Vec<ToolStatus> {
    vec![
        ToolStatus {
            name: "Xcode".into(),
            available: xcode_app.is_some(),
            path: xcode_app.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: xcode_version.clone(),
            detail: xcode_downloading
                .as_ref()
                .map(|p| format!("Download in progress: {}", p.display())),
        },
        ToolStatus {
            name: "xcode-select".into(),
            available: developer_dir
                .as_deref()
                .is_some_and(|d| d.contains("Xcode")),
            path: developer_dir.clone(),
            version: None,
            detail: developer_dir.as_deref().and_then(|d| {
                d.contains("CommandLineTools")
                    .then(|| "Points to CommandLineTools instead of Xcode.app".into())
            }),
        },
        ToolStatus {
            name: "simctl".into(),
            available: sim.simctl_ok,
            path: xcrun.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: sim.detail.clone(),
        },
        ToolStatus {
            name: "idb".into(),
            available: idb.is_some(),
            path: idb.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: None,
            detail: idb
                .is_none()
                .then(|| "Optional: enables direct tap/swipe/text input on iOS simulators".into()),
        },
    ]
}

fn classify_ios_readiness(
    xcode_app: &Option<PathBuf>,
    xcode_downloading: &Option<PathBuf>,
    sim: &SimctlProbeResult,
) -> (
    ReadinessState,
    ReadinessCode,
    String,
    Vec<ReadinessIssue>,
    Vec<SetupStep>,
) {
    let mut issues = Vec::new();
    let mut steps = Vec::new();

    if let (None, Some(dl)) = (xcode_app, xcode_downloading) {
        issues.push(ReadinessIssue {
            code: "xcode_downloading".into(),
            severity: IssueSeverity::Warning,
            message: format!("Xcode is currently downloading ({})", dl.display()),
        });
        steps.push(SetupStep {
            title: "Wait for Xcode download to complete".into(),
            command: None,
            url: None,
            detail: "Once /Applications/Xcode.app finishes installing, click Refresh.".into(),
        });
        steps.push(SetupStep {
            title: "Point xcode-select to Xcode and download iOS Simulator runtime".into(),
            command: Some(
                "sudo xcode-select -s /Applications/Xcode.app/Contents/Developer && xcodebuild -downloadPlatform iOS"
                    .into(),
            ),
            url: None,
            detail: "Run after Xcode installation completes.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::XcodeDownloading,
            "Xcode is mid-download; iOS Simulator will be ready once installation finishes.".into(),
            issues,
            steps,
        );
    }

    if xcode_app.is_none() && !sim.simctl_ok {
        issues.push(ReadinessIssue {
            code: "xcode_not_installed".into(),
            severity: IssueSeverity::Error,
            message: "Xcode is not installed in /Applications.".into(),
        });
        steps.push(SetupStep {
            title: "Install Xcode from the Mac App Store or Apple Developer".into(),
            command: None,
            url: Some("https://developer.apple.com/xcode/".into()),
            detail: "Xcode provides xcrun simctl and iOS Simulator.".into(),
        });
        steps.push(SetupStep {
            title: "Install the iOS Simulator runtime".into(),
            command: Some("xcodebuild -downloadPlatform iOS".into()),
            url: None,
            detail: "Downloads the latest iOS Simulator disk image.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::XcodeNotInstalled,
            "Xcode is not installed.".into(),
            issues,
            steps,
        );
    }

    if sim.license_required {
        issues.push(ReadinessIssue {
            code: "xcode_license_required".into(),
            severity: IssueSeverity::Error,
            message: "Xcode license agreement must be accepted before simctl can run.".into(),
        });
        steps.push(SetupStep {
            title: "Accept the Xcode license".into(),
            command: Some("sudo xcodebuild -license accept".into()),
            url: None,
            detail: "Accepts the Xcode and Apple SDKs license agreement.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::XcodeLicenseRequired,
            "Xcode license agreement must be accepted.".into(),
            issues,
            steps,
        );
    }

    if !sim.simctl_ok {
        let target_dev = xcode_app
            .as_ref()
            .map(|p| format!("{}/Contents/Developer", p.display()))
            .unwrap_or_else(|| "/Applications/Xcode.app/Contents/Developer".into());
        issues.push(ReadinessIssue {
            code: "xcode_select_required".into(),
            severity: IssueSeverity::Error,
            message: sim.detail.clone().unwrap_or_else(|| {
                "xcrun simctl is unavailable; xcode-select may point to CommandLineTools.".into()
            }),
        });
        steps.push(SetupStep {
            title: "Select Xcode as the active developer directory".into(),
            command: Some(format!("sudo xcode-select -s {target_dev}")),
            url: None,
            detail: "Switches developer tools from CommandLineTools to full Xcode.".into(),
        });
        return (
            ReadinessState::Unavailable,
            ReadinessCode::XcodeSelectRequired,
            "Xcode is installed, but xcode-select does not point to it.".into(),
            issues,
            steps,
        );
    }

    if sim.available_ios_runtimes == 0 {
        issues.push(ReadinessIssue {
            code: "ios_runtime_missing".into(),
            severity: IssueSeverity::Warning,
            message: "Xcode and simctl are available, but no iOS Simulator runtime is installed."
                .into(),
        });
        steps.push(SetupStep {
            title: "Download the iOS Simulator runtime".into(),
            command: Some("xcodebuild -downloadPlatform iOS".into()),
            url: None,
            detail: "Or open Xcode → Settings → Components / Platforms and click Get next to iOS."
                .into(),
        });
        return (
            ReadinessState::Degraded,
            ReadinessCode::IosRuntimeMissing,
            "Xcode is installed, but no iOS Simulator runtime is downloaded yet.".into(),
            issues,
            steps,
        );
    }

    if sim.total_devices == 0 {
        issues.push(ReadinessIssue {
            code: "no_devices".into(),
            severity: IssueSeverity::Info,
            message: "iOS runtime is installed, but no simulator devices exist yet.".into(),
        });
        steps.push(SetupStep {
            title: "Create an iOS Simulator device".into(),
            command: Some(
                "xcrun simctl create \"iPhone 16 Pro\" com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"
                    .into(),
            ),
            url: None,
            detail: "You can also create a simulator directly from Otto.".into(),
        });
        return (
            ReadinessState::Degraded,
            ReadinessCode::NoDevices,
            "iOS runtime is ready, but no simulator devices are created yet.".into(),
            issues,
            steps,
        );
    }

    (
        ReadinessState::Ready,
        ReadinessCode::Ready,
        format!(
            "iOS Simulator ready ({} runtime(s), {} device(s)).",
            sim.available_ios_runtimes, sim.total_devices
        ),
        issues,
        steps,
    )
}

fn find_xcode_app(probe: &dyn HostProbe, apps_entries: &[String]) -> Option<PathBuf> {
    let default = PathBuf::from("/Applications/Xcode.app");
    if probe.path_exists(&default) {
        return Some(default);
    }
    apps_entries
        .iter()
        .find(|name| name.starts_with("Xcode") && name.ends_with(".app"))
        .map(|name| Path::new("/Applications").join(name))
}

fn find_xcode_downloading(probe: &dyn HostProbe, apps_entries: &[String]) -> Option<PathBuf> {
    let default = PathBuf::from("/Applications/Xcode.appdownload");
    if probe.path_exists(&default) {
        return Some(default);
    }
    apps_entries
        .iter()
        .find(|name| name.starts_with("Xcode") && name.ends_with(".appdownload"))
        .map(|name| Path::new("/Applications").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{err_out, ok_out, FakeCommandRunner, FakeHostProbe};

    #[tokio::test]
    async fn detects_xcode_installed_but_ios_runtime_missing() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        probe.add_path("/Applications/Xcode.app");
        probe.add_which("xcrun", "/usr/bin/xcrun");
        runner.on_exact(
            "xcode-select",
            &["-p"],
            ok_out(b"/Applications/Xcode.app/Contents/Developer\n"),
        );
        runner.on_exact(
            "xcodebuild",
            &["-version"],
            ok_out(b"Xcode 16.2\nBuild version 16C5032a\n"),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            ok_out(br#"{"runtimes":[]}"#),
        );
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "devices", "-j"],
            ok_out(br#"{"devices":{}}"#),
        );

        let (_, ios) = detect_ios(&runner, &probe).await;
        assert_eq!(ios.state, ReadinessState::Degraded);
        assert_eq!(ios.code, ReadinessCode::IosRuntimeMissing);
        assert!(ios
            .setup_steps
            .iter()
            .any(|s| s.command.as_deref() == Some("xcodebuild -downloadPlatform iOS")));
    }

    #[tokio::test]
    async fn detects_xcode_license_required() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        probe.add_path("/Applications/Xcode.app");
        probe.add_which("xcrun", "/usr/bin/xcrun");
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
            err_out(b"You have not agreed to the Xcode license agreements. Run sudo xcodebuild -license"),
        );

        let (_, ios) = detect_ios(&runner, &probe).await;
        assert_eq!(ios.state, ReadinessState::Unavailable);
        assert_eq!(ios.code, ReadinessCode::XcodeLicenseRequired);
    }
}

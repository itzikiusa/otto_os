//! iOS Simulator driver (`xcrun simctl` discovery, catalog, lifecycle).

pub mod controls;

pub use controls::{
    get_clipboard, install_app, launch_app, open_url, recent_logs, screenshot, send_input,
    set_appearance, set_clipboard, set_orientation, set_status_bar, spawn_logs, terminate_app,
};

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::detect::ResolvedIosTools;
use crate::exec::{CmdSpec, CommandRunner};
use crate::types::{
    is_safe_token, BootSimDeviceReq, DeviceCapabilities, DeviceState, FormFactor, Orientation,
    Platform, ScreenMetrics, SimDevice, SimDeviceType, SimRuntime,
};

#[derive(Debug, Deserialize)]
struct SimctlListPayload {
    #[serde(default)]
    runtimes: Vec<SimctlRuntime>,
    #[serde(default)]
    devicetypes: Vec<SimctlDeviceType>,
    #[serde(default)]
    devices: HashMap<String, Vec<SimctlDevice>>,
}

#[derive(Debug, Deserialize)]
struct SimctlRuntime {
    identifier: String,
    name: String,
    #[serde(default)]
    version: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
}

#[derive(Debug, Deserialize)]
struct SimctlDeviceType {
    identifier: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct SimctlDevice {
    udid: String,
    name: String,
    #[serde(default)]
    state: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
    #[serde(rename = "deviceTypeIdentifier", default)]
    device_type_identifier: Option<String>,
}

pub(crate) fn require_xcrun(tools: &ResolvedIosTools) -> otto_core::Result<String> {
    if !tools.simctl_ok {
        return Err(otto_core::Error::Upstream(
            "iOS Simulator (xcrun simctl) is not ready on this host; check /api/v1/simulators/status"
                .into(),
        ));
    }
    tools
        .xcrun
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| otto_core::Error::Upstream("xcrun binary not found".into()))
}

/// Map `simctl create`/`delete` CLI errors to accurate HTTP error classes:
/// missing device → 404 NotFound, invalid runtime/type/name → 400 Invalid, else 502 Upstream.
pub(crate) fn classify_simctl_err(action: &str, detail: String) -> otto_core::Error {
    let lc = detail.to_ascii_lowercase();
    if lc.contains("invalid device")
        || lc.contains("unable to find device")
        || lc.contains("does not exist")
        || lc.contains("no such device")
    {
        otto_core::Error::NotFound(format!("{action} failed: {detail}"))
    } else if lc.contains("invalid runtime")
        || lc.contains("invalid device type")
        || lc.contains("incompatible device")
        || lc.contains("invalid name")
    {
        otto_core::Error::Invalid(format!("{action} failed: {detail}"))
    } else {
        otto_core::Error::Upstream(format!("{action} failed: {detail}"))
    }
}

async fn simctl_list(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
) -> otto_core::Result<SimctlListPayload> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(CmdSpec::new(xcrun, ["simctl", "list", "-j"]).with_timeout(Duration::from_secs(15)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "xcrun simctl list failed: {}",
            out.combined_err()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| otto_core::Error::Internal(format!("failed to parse simctl list JSON: {e}")))
}

pub async fn list_devices(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    tracked_orientations: &HashMap<String, Orientation>,
) -> otto_core::Result<Vec<SimDevice>> {
    if !tools.simctl_ok {
        return Ok(Vec::new());
    }
    let payload = simctl_list(runner, tools).await?;
    let runtime_map: HashMap<&str, &SimctlRuntime> = payload
        .runtimes
        .iter()
        .map(|r| (r.identifier.as_str(), r))
        .collect();
    let has_idb = tools.idb.is_some();

    let mut out = Vec::new();
    for (runtime_id, devs) in &payload.devices {
        let os_version = runtime_map
            .get(runtime_id.as_str())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| humanize_runtime_id(runtime_id));
        for d in devs {
            if !d.is_available {
                continue;
            }
            let form_factor =
                infer_ios_form_factor(&d.name, d.device_type_identifier.as_deref().unwrap_or(""));
            let screen = default_ios_screen(&d.name, form_factor);
            let orientation = tracked_orientations
                .get(&d.udid)
                .copied()
                .unwrap_or(Orientation::Portrait);
            out.push(SimDevice {
                id: format!("ios:{}", d.udid),
                platform: Platform::Ios,
                name: d.name.clone(),
                udid: Some(d.udid.clone()),
                avd_name: None,
                serial: None,
                state: parse_ios_state(&d.state),
                os_version: os_version.clone(),
                api_level: None,
                runtime_id: Some(runtime_id.clone()),
                device_type: d.device_type_identifier.clone(),
                form_factor,
                screen,
                orientation,
                appearance: None,
                is_physical: false,
                capabilities: ios_capabilities(has_idb),
            });
        }
    }
    out.sort_by(|a, b| {
        let a_rank = if a.state.is_live() { 0 } else { 1 };
        let b_rank = if b.state.is_live() { 0 } else { 1 };
        a_rank
            .cmp(&b_rank)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.os_version.cmp(&b.os_version))
    });
    Ok(out)
}

pub async fn list_catalog(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
) -> otto_core::Result<(Vec<SimRuntime>, Vec<SimDeviceType>)> {
    if !tools.simctl_ok {
        return Ok((Vec::new(), Vec::new()));
    }
    let payload = simctl_list(runner, tools).await?;
    let runtimes = payload
        .runtimes
        .into_iter()
        .map(|r| SimRuntime {
            id: r.identifier,
            name: r.name,
            version: r.version,
            platform: Platform::Ios,
            available: r.is_available,
        })
        .collect();
    let device_types = payload
        .devicetypes
        .into_iter()
        .map(|dt| {
            let ff = infer_ios_form_factor(&dt.name, &dt.identifier);
            SimDeviceType {
                id: dt.identifier,
                name: dt.name,
                form_factor: ff,
            }
        })
        .collect();
    Ok((runtimes, device_types))
}

pub async fn create_device(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    name: &str,
    device_type_id: &str,
    runtime_id: &str,
) -> otto_core::Result<String> {
    let xcrun = require_xcrun(tools)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(|c| c.is_control()) {
        return Err(otto_core::Error::Invalid("invalid simulator name".into()));
    }
    if !is_safe_token(device_type_id) || !is_safe_token(runtime_id) {
        return Err(otto_core::Error::Invalid(
            "invalid device_type_id or runtime_id".into(),
        ));
    }
    let out = runner
        .run(
            CmdSpec::new(
                xcrun,
                ["simctl", "create", name, device_type_id, runtime_id],
            )
            .with_timeout(Duration::from_secs(30)),
        )
        .await?;
    if !out.success() {
        return Err(classify_simctl_err("simctl create", out.combined_err()));
    }
    let udid = out.stdout_str().trim().to_string();
    if udid.is_empty() {
        return Err(otto_core::Error::Internal(
            "simctl create returned empty UDID".into(),
        ));
    }
    Ok(udid)
}

pub async fn delete_device(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
) -> otto_core::Result<()> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(CmdSpec::new(xcrun, ["simctl", "delete", udid]).with_timeout(Duration::from_secs(30)))
        .await?;
    if !out.success() {
        return Err(classify_simctl_err("simctl delete", out.combined_err()));
    }
    Ok(())
}

pub async fn boot_device(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    req: &BootSimDeviceReq,
) -> otto_core::Result<()> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(CmdSpec::new(&xcrun, ["simctl", "boot", udid]).with_timeout(Duration::from_secs(30)))
        .await?;
    if !out.success() {
        let err = out.combined_err();
        if !err.to_ascii_lowercase().contains("current state: booted") {
            return Err(otto_core::Error::Upstream(format!(
                "simctl boot failed: {err}"
            )));
        }
    }
    if req.open_gui.unwrap_or(false) {
        let _ = runner
            .run(
                CmdSpec::new(
                    "open",
                    ["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid],
                )
                .with_timeout(Duration::from_secs(10)),
            )
            .await;
    }
    if req.wait.unwrap_or(false) {
        let secs = req.timeout_secs.unwrap_or(90).clamp(5, 300);
        let wait_out = runner
            .run(
                CmdSpec::new(&xcrun, ["simctl", "bootstatus", udid, "-b"])
                    .with_timeout(Duration::from_secs(secs)),
            )
            .await?;
        if !wait_out.success() {
            return Err(otto_core::Error::Upstream(format!(
                "simctl bootstatus failed: {}",
                wait_out.combined_err()
            )));
        }
    }
    Ok(())
}

pub async fn shutdown_device(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
) -> otto_core::Result<()> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(
            CmdSpec::new(xcrun, ["simctl", "shutdown", udid]).with_timeout(Duration::from_secs(30)),
        )
        .await?;
    if !out.success() {
        let err = out.combined_err();
        if !err.to_ascii_lowercase().contains("current state: shutdown") {
            return Err(otto_core::Error::Upstream(format!(
                "simctl shutdown failed: {err}"
            )));
        }
    }
    Ok(())
}

fn ios_capabilities(has_idb: bool) -> DeviceCapabilities {
    DeviceCapabilities {
        boot: true,
        shutdown: true,
        delete: true,
        open_url: true,
        screenshot: true,
        stream: true,
        tap: has_idb,
        swipe: has_idb,
        text: has_idb,
        key_event: has_idb,
        rotate: true,
        appearance: true,
        status_bar: true,
        clipboard: true,
        port_forward: false,
        app_install: true,
        logs: true,
    }
}

fn parse_ios_state(raw: &str) -> DeviceState {
    match raw.trim().to_ascii_lowercase().as_str() {
        "booted" => DeviceState::Booted,
        "booting" => DeviceState::Booting,
        "shutdown" => DeviceState::Shutdown,
        "shutting down" => DeviceState::ShuttingDown,
        _ => DeviceState::Unknown,
    }
}

fn humanize_runtime_id(id: &str) -> String {
    let tail = id.rsplit('.').next().unwrap_or(id);
    if let Some((os, ver)) = tail.split_once('-') {
        format!("{os} {}", ver.replace('-', "."))
    } else {
        tail.to_string()
    }
}

fn infer_ios_form_factor(name: &str, type_id: &str) -> FormFactor {
    let combined = format!("{name} {type_id}").to_ascii_lowercase();
    if combined.contains("ipad") {
        FormFactor::Tablet
    } else if combined.contains("iphone") {
        FormFactor::Phone
    } else if combined.contains("watch") {
        FormFactor::Watch
    } else if combined.contains("tv") {
        FormFactor::Tv
    } else {
        FormFactor::Other
    }
}

fn default_ios_screen(name: &str, form_factor: FormFactor) -> Option<ScreenMetrics> {
    let lc = name.to_ascii_lowercase();
    match form_factor {
        FormFactor::Phone => {
            if lc.contains("pro max") || lc.contains("plus") {
                Some(ScreenMetrics {
                    width: 430,
                    height: 932,
                    scale: Some(3.0),
                    dpi: None,
                })
            } else if lc.contains("se") {
                Some(ScreenMetrics {
                    width: 375,
                    height: 667,
                    scale: Some(2.0),
                    dpi: None,
                })
            } else {
                Some(ScreenMetrics {
                    width: 393,
                    height: 852,
                    scale: Some(3.0),
                    dpi: None,
                })
            }
        }
        FormFactor::Tablet => {
            if lc.contains("13") || lc.contains("12.9") {
                Some(ScreenMetrics {
                    width: 1024,
                    height: 1366,
                    scale: Some(2.0),
                    dpi: None,
                })
            } else if lc.contains("mini") {
                Some(ScreenMetrics {
                    width: 744,
                    height: 1133,
                    scale: Some(2.0),
                    dpi: None,
                })
            } else {
                Some(ScreenMetrics {
                    width: 834,
                    height: 1194,
                    scale: Some(2.0),
                    dpi: None,
                })
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{err_out, ok_out, FakeCommandRunner};
    use std::path::PathBuf;

    pub(crate) fn ready_ios_tools(with_idb: bool) -> ResolvedIosTools {
        ResolvedIosTools {
            xcode_app: Some(PathBuf::from("/Applications/Xcode.app")),
            xcode_downloading: None,
            developer_dir: Some("/Applications/Xcode.app/Contents/Developer".into()),
            xcrun: Some(PathBuf::from("/usr/bin/xcrun")),
            idb: with_idb.then(|| PathBuf::from("/opt/homebrew/bin/idb")),
            simctl_ok: true,
            license_required: false,
        }
    }

    const SAMPLE_SIMCTL_JSON: &str = r#"{
        "runtimes": [
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-18-2", "name": "iOS 18.2", "version": "18.2", "isAvailable": true}
        ],
        "devicetypes": [
            {"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro", "name": "iPhone 16 Pro"},
            {"identifier": "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-11-inch-M4", "name": "iPad Pro 11-inch (M4)"}
        ],
        "devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-18-2": [
                {"udid": "AAA-111", "name": "iPhone 16 Pro", "state": "Booted", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro"},
                {"udid": "BBB-222", "name": "iPad Pro 11-inch (M4)", "state": "Shutdown", "isAvailable": true, "deviceTypeIdentifier": "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-11-inch-M4"},
                {"udid": "CCC-333", "name": "Unavailable", "state": "Shutdown", "isAvailable": false}
            ]
        }
    }"#;

    #[tokio::test]
    async fn parses_ios_devices_and_capabilities() {
        let runner = FakeCommandRunner::new();
        runner.on_exact(
            "xcrun",
            &["simctl", "list", "-j"],
            ok_out(SAMPLE_SIMCTL_JSON.as_bytes()),
        );
        let mut orientations = HashMap::new();
        orientations.insert("AAA-111".to_string(), Orientation::Landscape);
        let devices = list_devices(&runner, &ready_ios_tools(false), &orientations)
            .await
            .unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "ios:AAA-111");
        assert_eq!(devices[0].state, DeviceState::Booted);
        assert_eq!(devices[0].orientation, Orientation::Landscape);
        assert_eq!(devices[1].orientation, Orientation::Portrait);
        assert_eq!(devices[0].form_factor, FormFactor::Phone);
        assert_eq!(devices[0].os_version, "iOS 18.2");
        assert!(!devices[0].capabilities.tap);
        assert!(devices[0].capabilities.status_bar);

        assert_eq!(devices[1].id, "ios:BBB-222");
        assert_eq!(devices[1].form_factor, FormFactor::Tablet);
        assert_eq!(devices[1].screen.as_ref().unwrap().width, 834);
    }

    #[tokio::test]
    async fn delete_device_maps_missing_to_not_found_and_crash_to_upstream() {
        let runner = FakeCommandRunner::new();
        let tools = ready_ios_tools(false);
        runner.on_exact(
            "xcrun",
            &["simctl", "delete", "MISSING-1"],
            err_out(b"Invalid device: MISSING-1"),
        );
        assert!(matches!(
            delete_device(&runner, &tools, "MISSING-1").await,
            Err(otto_core::Error::NotFound(_))
        ));

        runner.on_exact(
            "xcrun",
            &["simctl", "delete", "CRASH-1"],
            err_out(b"CoreSimulatorService connection interrupted"),
        );
        assert!(matches!(
            delete_device(&runner, &tools, "CRASH-1").await,
            Err(otto_core::Error::Upstream(_))
        ));
    }
}

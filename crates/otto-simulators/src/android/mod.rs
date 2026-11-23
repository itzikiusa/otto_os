//! Android Emulator & ADB driver: discovery, batched device probing, catalog,
//! shared AVD-list parsing, and SDK environment wiring.

pub mod controls;
pub mod lifecycle;

pub use controls::{
    add_port_forward, escape_adb_input_text, install_app, launch_app, list_port_forwards, open_url,
    quote_device_shell_arg, recent_logs, remove_port_forward, screenshot, send_input,
    set_appearance, set_orientation, spawn_logs, terminate_app, validate_android_keycode,
};
pub use lifecycle::{boot_avd, create_avd, delete_avd, shutdown_serial};

use std::collections::HashMap;
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::Mutex;

use crate::android_env::{
    discover_java_home, ensure_adb_server, parse_avd_list, with_android_sdk_env,
};
use crate::detect::ResolvedAndroidTools;
use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::{
    AndroidDeviceProfile, AndroidSystemImage, DeviceCapabilities, DeviceState, FormFactor,
    Orientation, Platform, ScreenMetrics, SimDevice,
};

/// Batched single-invocation `adb shell` probe script replacing 5 sequential
/// subprocesses per booted Android device with 1.
pub const BATCHED_ADB_PROBE_CMD: &str = "getprop ro.build.version.release; echo __OTTO__; \
     getprop ro.build.version.sdk; echo __OTTO__; \
     wm size; echo __OTTO__; \
     wm density; echo __OTTO__; \
     settings get system user_rotation";

pub(crate) fn require_adb(tools: &ResolvedAndroidTools) -> otto_core::Result<String> {
    tools
        .adb
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| {
            otto_core::Error::Upstream(
                "Android platform-tools (adb) not found; check /api/v1/simulators/status".into(),
            )
        })
}

pub(crate) fn require_emulator(tools: &ResolvedAndroidTools) -> otto_core::Result<String> {
    tools
        .emulator
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| {
            otto_core::Error::Upstream(
                "Android emulator binary not found; check /api/v1/simulators/status".into(),
            )
        })
}

pub(crate) fn require_avdmanager(tools: &ResolvedAndroidTools) -> otto_core::Result<String> {
    tools
        .avdmanager
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| {
            otto_core::Error::Upstream(
                "Android avdmanager not found; install Android cmdline-tools".into(),
            )
        })
}

#[derive(Debug, Clone)]
struct ConnectedAdbDevice {
    serial: String,
    state: DeviceState,
    model: Option<String>,
    is_emulator: bool,
    avd_name: Option<String>,
    os_version: String,
    api_level: Option<u32>,
    screen: Option<ScreenMetrics>,
    orientation: Orientation,
}

#[derive(Debug, Clone, Default)]
struct AvdIniMeta {
    display_name: Option<String>,
    device_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    dpi: Option<u32>,
    api_level: Option<u32>,
    tag_id: Option<String>,
}

/// Discover all Android AVDs and connected ADB devices, merging running
/// emulator serials into their corresponding AVD entries so IDs remain stable
/// across boot/shutdown transitions.
pub async fn list_devices(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    adb_lock: &Mutex<()>,
    tools: &ResolvedAndroidTools,
) -> otto_core::Result<Vec<SimDevice>> {
    let mut avd_names = Vec::new();
    if let Some(emu) = &tools.emulator {
        if let Ok(out) = runner
            .run(
                CmdSpec::new(emu.to_string_lossy(), ["-list-avds"])
                    .with_timeout(Duration::from_secs(10)),
            )
            .await
        {
            if out.success() {
                avd_names = parse_avd_list(&out.stdout_str());
            }
        }
    }

    let connected = list_connected_adb(runner, adb_lock, tools).await;
    let mut running_by_avd: HashMap<String, ConnectedAdbDevice> = HashMap::new();
    let mut standalone_connected: Vec<ConnectedAdbDevice> = Vec::new();

    for c in connected {
        if let Some(avd) = &c.avd_name {
            if avd_names.iter().any(|n| n == avd) {
                running_by_avd.insert(avd.clone(), c);
                continue;
            }
        }
        standalone_connected.push(c);
    }

    let mut out = Vec::new();
    for avd in avd_names {
        let meta = read_avd_config(probe, &avd);
        let running = running_by_avd.remove(&avd);

        let (state, serial, os_version, api_level, screen, orientation) = if let Some(r) = running {
            let screen = r.screen.or_else(|| meta_to_screen(&meta));
            let api = r.api_level.or(meta.api_level);
            let os = if !r.os_version.is_empty() {
                r.os_version
            } else {
                api.map(|a| format!("Android (API {a})"))
                    .unwrap_or_else(|| "Android".into())
            };
            (r.state, Some(r.serial), os, api, screen, r.orientation)
        } else {
            let screen = meta_to_screen(&meta);
            let os = meta
                .api_level
                .map(|a| format!("Android (API {a})"))
                .unwrap_or_else(|| "Android".into());
            (
                DeviceState::Shutdown,
                None,
                os,
                meta.api_level,
                screen,
                Orientation::Portrait,
            )
        };

        let display_name = meta
            .display_name
            .clone()
            .unwrap_or_else(|| avd.replace('_', " "));
        let form_factor = infer_android_form_factor(
            &display_name,
            meta.device_name.as_deref().unwrap_or(""),
            meta.tag_id.as_deref().unwrap_or(""),
            screen.as_ref(),
        );

        out.push(SimDevice {
            id: format!("android:avd:{avd}"),
            platform: Platform::Android,
            name: display_name,
            udid: None,
            avd_name: Some(avd),
            serial,
            state,
            os_version,
            api_level,
            runtime_id: api_level.map(|a| format!("android-{a}")),
            device_type: meta.device_name,
            form_factor,
            screen,
            orientation,
            appearance: None,
            is_physical: false,
            capabilities: android_capabilities(true, true),
        });
    }

    for c in standalone_connected {
        let name = c
            .avd_name
            .clone()
            .or_else(|| c.model.clone())
            .unwrap_or_else(|| c.serial.clone());
        let form_factor = infer_android_form_factor(&name, "", "", c.screen.as_ref());
        let is_physical = !c.is_emulator;
        out.push(SimDevice {
            id: format!("android:serial:{}", c.serial),
            platform: Platform::Android,
            name,
            udid: None,
            avd_name: c.avd_name,
            serial: Some(c.serial),
            state: c.state,
            os_version: if c.os_version.is_empty() {
                "Android".into()
            } else {
                c.os_version
            },
            api_level: c.api_level,
            runtime_id: c.api_level.map(|a| format!("android-{a}")),
            device_type: c.model,
            form_factor,
            screen: c.screen,
            orientation: c.orientation,
            appearance: None,
            is_physical,
            capabilities: android_capabilities(false, c.is_emulator),
        });
    }

    out.sort_by(|a, b| {
        let a_rank = if a.state.is_live() { 0 } else { 1 };
        let b_rank = if b.state.is_live() { 0 } else { 1 };
        a_rank.cmp(&b_rank).then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

async fn list_connected_adb(
    runner: &dyn CommandRunner,
    adb_lock: &Mutex<()>,
    tools: &ResolvedAndroidTools,
) -> Vec<ConnectedAdbDevice> {
    let Some(adb) = &tools.adb else {
        return Vec::new();
    };
    let adb_str = adb.to_string_lossy().into_owned();
    ensure_adb_server(runner, adb_lock, &adb_str).await;
    let Ok(out) = runner
        .run(CmdSpec::new(&adb_str, ["devices", "-l"]).with_timeout(Duration::from_secs(8)))
        .await
    else {
        return Vec::new();
    };
    if !out.success() {
        return Vec::new();
    }

    let mut raw_entries = Vec::new();
    for line in out.stdout_str().lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('*')
            || line.starts_with("List of devices")
            || line.starts_with("adb server")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let Some(raw_state) = parts.next() else {
            continue;
        };
        let state = match raw_state {
            "device" => DeviceState::Booted,
            "offline" => DeviceState::Offline,
            "unauthorized" | "authorizing" => DeviceState::Booting,
            _ => DeviceState::Unknown,
        };
        let mut model = None;
        for kv in parts {
            if let Some(m) = kv.strip_prefix("model:") {
                model = Some(m.replace('_', " "));
            }
        }
        raw_entries.push((serial.to_string(), state, model));
    }

    // Probe all connected devices concurrently, using 1 batched shell
    // call per booted device instead of 5 sequential subprocesses.
    join_all(raw_entries.into_iter().map(|(serial, state, model)| {
        let adb_ref = adb_str.as_str();
        async move { probe_connected_device(runner, adb_ref, serial, state, model).await }
    }))
    .await
}

async fn probe_connected_device(
    runner: &dyn CommandRunner,
    adb_str: &str,
    serial: String,
    state: DeviceState,
    model: Option<String>,
) -> ConnectedAdbDevice {
    let is_emulator = serial.starts_with("emulator-");
    let mut avd_name = None;
    let mut os_version = String::new();
    let mut api_level = None;
    let mut screen = None;
    let mut orientation = Orientation::Portrait;

    if state == DeviceState::Booted {
        let avd_fut = async {
            if !is_emulator {
                return None;
            }
            let out = runner
                .run(
                    CmdSpec::new(adb_str, ["-s", &serial, "emu", "avd", "name"])
                        .with_timeout(Duration::from_secs(4)),
                )
                .await
                .ok()?;
            if !out.success() {
                return None;
            }
            out.stdout_str()
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && *l != "OK")
                .map(ToOwned::to_owned)
        };
        let props_fut = runner.run(
            CmdSpec::new(adb_str, ["-s", &serial, "shell", BATCHED_ADB_PROBE_CMD])
                .with_timeout(Duration::from_secs(5)),
        );
        let (avd_res, props_res) = tokio::join!(avd_fut, props_fut);
        avd_name = avd_res;

        if let Ok(out) = props_res {
            if out.success() {
                let raw = out.stdout_str();
                let mut sections = raw.split("__OTTO__");
                let rel = sections.next().unwrap_or("").trim();
                if !rel.is_empty() {
                    os_version = format!("Android {rel}");
                }
                api_level = sections.next().unwrap_or("").trim().parse::<u32>().ok();
                let sz_txt = sections.next().unwrap_or("");
                let dpi_txt = sections.next().unwrap_or("");
                if let Some((w, h)) = parse_wm_size(sz_txt) {
                    let dpi = parse_wm_density(dpi_txt);
                    screen = Some(ScreenMetrics {
                        width: w,
                        height: h,
                        scale: dpi.map(|d| (d as f32) / 160.0),
                        dpi,
                    });
                }
                orientation = match sections.next().unwrap_or("").trim() {
                    "0" => Orientation::Portrait,
                    "1" => Orientation::Landscape,
                    "2" => Orientation::PortraitUpsideDown,
                    "3" => Orientation::LandscapeRight,
                    _ => Orientation::Portrait,
                };
            }
        }
    }

    ConnectedAdbDevice {
        serial,
        state,
        model,
        is_emulator,
        avd_name,
        os_version,
        api_level,
        screen,
        orientation,
    }
}

pub async fn list_catalog(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    tools: &ResolvedAndroidTools,
) -> otto_core::Result<(Vec<AndroidSystemImage>, Vec<AndroidDeviceProfile>)> {
    let mut images = Vec::new();
    if let Some(root) = &tools.sdk_root {
        let base = root.join("system-images");
        for api_dir in probe.list_dir(&base) {
            let api_level = api_dir
                .strip_prefix("android-")
                .and_then(|s| s.parse::<u32>().ok());
            let api_path = base.join(&api_dir);
            for tag in probe.list_dir(&api_path) {
                let tag_path = api_path.join(&tag);
                for abi in probe.list_dir(&tag_path) {
                    images.push(AndroidSystemImage {
                        id: format!("system-images;{api_dir};{tag};{abi}"),
                        api_level,
                        abi: Some(abi),
                        tag: Some(tag.clone()),
                        installed: true,
                    });
                }
            }
        }
    }

    let mut profiles = Vec::new();
    if let Some(avdm) = &tools.avdmanager {
        let java_home = discover_java_home(runner, probe).await;
        let spec = with_android_sdk_env(
            CmdSpec::new(avdm.to_string_lossy(), ["list", "device", "-c"])
                .with_timeout(Duration::from_secs(10)),
            tools.sdk_root.as_deref(),
            java_home.as_deref(),
        );
        if let Ok(out) = runner.run(spec).await {
            if out.success() {
                for line in out.stdout_str().lines() {
                    let id = line.trim();
                    if id.is_empty() {
                        continue;
                    }
                    let name = id.replace('_', " ");
                    let ff = infer_android_form_factor(&name, id, "", None);
                    profiles.push(AndroidDeviceProfile {
                        id: id.to_string(),
                        name,
                        form_factor: ff,
                    });
                }
            }
        }
    }
    Ok((images, profiles))
}

fn android_capabilities(is_avd: bool, can_shutdown: bool) -> DeviceCapabilities {
    DeviceCapabilities {
        boot: is_avd,
        shutdown: can_shutdown,
        delete: is_avd,
        open_url: true,
        screenshot: true,
        stream: true,
        tap: true,
        swipe: true,
        text: true,
        key_event: true,
        rotate: true,
        appearance: true,
        status_bar: false,
        clipboard: false,
        port_forward: true,
        app_install: true,
        logs: true,
    }
}

fn read_avd_config(probe: &dyn HostProbe, avd_name: &str) -> AvdIniMeta {
    let Some(home) = probe.home_dir() else {
        return AvdIniMeta::default();
    };
    let config_path = home
        .join(".android")
        .join("avd")
        .join(format!("{avd_name}.avd"))
        .join("config.ini");
    let Some(content) = probe.read_to_string(&config_path) else {
        return AvdIniMeta::default();
    };
    parse_avd_config_ini(&content)
}

fn parse_avd_config_ini(content: &str) -> AvdIniMeta {
    let mut meta = AvdIniMeta::default();
    for line in content.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "avd.ini.displayname" => meta.display_name = Some(val.to_string()),
            "hw.device.name" => meta.device_name = Some(val.to_string()),
            "hw.lcd.width" => meta.width = val.parse().ok(),
            "hw.lcd.height" => meta.height = val.parse().ok(),
            "hw.lcd.density" => meta.dpi = val.parse().ok(),
            "tag.id" => meta.tag_id = Some(val.to_string()),
            "image.sysdir.1" => {
                for seg in val.split(['/', '\\']) {
                    if let Some(num) = seg.strip_prefix("android-").and_then(|s| s.parse().ok()) {
                        meta.api_level = Some(num);
                    }
                }
            }
            _ => {}
        }
    }
    meta
}

fn meta_to_screen(meta: &AvdIniMeta) -> Option<ScreenMetrics> {
    let (w, h) = (meta.width?, meta.height?);
    Some(ScreenMetrics {
        width: w,
        height: h,
        scale: meta.dpi.map(|d| (d as f32) / 160.0),
        dpi: meta.dpi,
    })
}

fn parse_wm_size(stdout: &str) -> Option<(u32, u32)> {
    let mut physical = None;
    let mut override_sz = None;
    for line in stdout.lines() {
        if let Some((_, rhs)) = line.split_once(':') {
            let rhs = rhs.trim();
            if let Some((w, h)) = rhs.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    if line.to_ascii_lowercase().contains("override") {
                        override_sz = Some((w, h));
                    } else {
                        physical = Some((w, h));
                    }
                }
            }
        }
    }
    override_sz.or(physical)
}

fn parse_wm_density(stdout: &str) -> Option<u32> {
    let mut physical = None;
    let mut override_d = None;
    for line in stdout.lines() {
        if let Some((_, rhs)) = line.split_once(':') {
            if let Ok(d) = rhs.trim().parse::<u32>() {
                if line.to_ascii_lowercase().contains("override") {
                    override_d = Some(d);
                } else {
                    physical = Some(d);
                }
            }
        }
    }
    override_d.or(physical)
}

fn infer_android_form_factor(
    name: &str,
    device_name: &str,
    tag_id: &str,
    screen: Option<&ScreenMetrics>,
) -> FormFactor {
    let combined = format!("{name} {device_name} {tag_id}").to_ascii_lowercase();
    if combined.contains("wear") || combined.contains("watch") {
        return FormFactor::Watch;
    }
    if combined.contains("tv") {
        return FormFactor::Tv;
    }
    if combined.contains("tablet") || combined.contains("tab_") || combined.contains("tab ") {
        return FormFactor::Tablet;
    }
    if let Some(s) = screen {
        let dpi = s.dpi.unwrap_or(420).max(120);
        let min_px = s.width.min(s.height);
        let sw_dp = (min_px * 160) / dpi;
        if sw_dp >= 600 {
            return FormFactor::Tablet;
        }
    }
    FormFactor::Phone
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};
    use std::path::PathBuf;

    pub(crate) fn ready_android_tools() -> ResolvedAndroidTools {
        ResolvedAndroidTools {
            sdk_root: Some(PathBuf::from("/Users/tester/Library/Android/sdk")),
            studio_app: None,
            adb: Some(PathBuf::from(
                "/Users/tester/Library/Android/sdk/platform-tools/adb",
            )),
            emulator: Some(PathBuf::from(
                "/Users/tester/Library/Android/sdk/emulator/emulator",
            )),
            avdmanager: Some(PathBuf::from(
                "/Users/tester/Library/Android/sdk/cmdline-tools/latest/bin/avdmanager",
            )),
            sdkmanager: None,
        }
    }

    #[tokio::test]
    async fn merges_running_emulator_with_avd_and_detects_tablet() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let adb_lock = Mutex::new(());
        let tools = ready_android_tools();

        probe.add_file(
            "/Users/tester/.android/avd/Pixel_9_API_35.avd/config.ini",
            "avd.ini.displayname=Pixel 9\nhw.device.name=pixel_9\nhw.lcd.width=1080\nhw.lcd.height=2424\nhw.lcd.density=420\nimage.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n",
        );
        probe.add_file(
            "/Users/tester/.android/avd/Pixel_Tablet_API_35.avd/config.ini",
            "avd.ini.displayname=Pixel Tablet\nhw.device.name=pixel_tablet\nhw.lcd.width=2560\nhw.lcd.height=1600\nhw.lcd.density=320\nimage.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n",
        );

        runner.on_exact(
            "emulator",
            &["-list-avds"],
            ok_out(b"WARNING | some banner\nPixel_9_API_35\nPixel_Tablet_API_35\n"),
        );
        runner.on_exact(
            "adb",
            &["devices", "-l"],
            ok_out(
                b"* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\nemulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64\n",
            ),
        );
        runner.on_exact(
            "adb",
            &["-s", "emulator-5554", "emu", "avd", "name"],
            ok_out(b"Pixel_9_API_35\nOK\n"),
        );
        runner.on_exact(
            "adb",
            &["-s", "emulator-5554", "shell", BATCHED_ADB_PROBE_CMD],
            ok_out(
                b"15\n__OTTO__\n35\n__OTTO__\nPhysical size: 1080x2424\n__OTTO__\nPhysical density: 420\n__OTTO__\n1\n",
            ),
        );

        let devices = list_devices(&runner, &probe, &adb_lock, &tools)
            .await
            .unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "android:avd:Pixel_9_API_35");
        assert_eq!(devices[0].state, DeviceState::Booted);
        assert_eq!(devices[0].serial.as_deref(), Some("emulator-5554"));
        assert_eq!(devices[0].os_version, "Android 15");
        assert_eq!(devices[0].orientation, Orientation::Landscape);
        assert_eq!(devices[0].form_factor, FormFactor::Phone);
        assert!(devices[0].capabilities.port_forward);
        assert!(devices[0].capabilities.tap);

        assert_eq!(devices[1].id, "android:avd:Pixel_Tablet_API_35");
        assert_eq!(devices[1].state, DeviceState::Shutdown);
        assert_eq!(devices[1].form_factor, FormFactor::Tablet);
    }
}

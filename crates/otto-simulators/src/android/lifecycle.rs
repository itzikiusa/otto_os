//! Android AVD lifecycle: create, delete, boot (idempotent), wait-for-boot,
//! and shutdown (`emu kill`).

use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::android_env::{discover_java_home, ensure_adb_server, with_android_sdk_env};
use crate::detect::ResolvedAndroidTools;
use crate::exec::{CmdSpec, CommandRunner, HostProbe};
use crate::types::{is_safe_avd_name, is_safe_token, BootSimDeviceReq};

use super::{list_devices, require_adb, require_avdmanager, require_emulator};

const MAX_SYSTEM_IMAGE_ID_LEN: usize = 256;

fn classify_avdmanager_err(action: &str, detail: String) -> otto_core::Error {
    let lc = detail.to_ascii_lowercase();
    if lc.contains("there is no android virtual device")
        || lc.contains("no avd")
        || lc.contains("not found")
    {
        otto_core::Error::NotFound(format!("{action} failed: {detail}"))
    } else if lc.contains("package path is not valid")
        || lc.contains("invalid")
        || lc.contains("unknown device")
    {
        otto_core::Error::Invalid(format!("{action} failed: {detail}"))
    } else {
        otto_core::Error::Upstream(format!("{action} failed: {detail}"))
    }
}

pub async fn create_avd(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    tools: &ResolvedAndroidTools,
    name: &str,
    system_image_id: &str,
    device_profile: Option<&str>,
) -> otto_core::Result<()> {
    let avdm = require_avdmanager(tools)?;
    if !is_safe_avd_name(name) {
        return Err(otto_core::Error::Invalid(
            "AVD name must match [A-Za-z0-9._-] and not start with '-'".into(),
        ));
    }
    let img = system_image_id.trim();
    if img.is_empty()
        || img.len() > MAX_SYSTEM_IMAGE_ID_LEN
        || img.starts_with('-')
        || !img
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b';' | b'-' | b'_' | b'.'))
    {
        return Err(otto_core::Error::Invalid("invalid system_image_id".into()));
    }
    let mut args = vec![
        "create".to_string(),
        "avd".to_string(),
        "--force".to_string(),
        "-n".to_string(),
        name.to_string(),
        "-k".to_string(),
        img.to_string(),
    ];
    if let Some(dp) = device_profile.map(str::trim).filter(|s| !s.is_empty()) {
        if !is_safe_token(dp) {
            return Err(otto_core::Error::Invalid("invalid device_profile".into()));
        }
        args.push("-d".into());
        args.push(dp.to_string());
    }
    let java_home = discover_java_home(runner, probe).await;
    let spec = with_android_sdk_env(
        CmdSpec::new(avdm, args)
            .with_stdin(b"no\n".to_vec())
            .with_timeout(Duration::from_secs(45)),
        tools.sdk_root.as_deref(),
        java_home.as_deref(),
    );
    let out = runner.run(spec).await?;
    if !out.success() {
        return Err(classify_avdmanager_err(
            "avdmanager create avd",
            out.combined_err(),
        ));
    }
    Ok(())
}

pub async fn delete_avd(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    tools: &ResolvedAndroidTools,
    name: &str,
) -> otto_core::Result<()> {
    let avdm = require_avdmanager(tools)?;
    if !is_safe_avd_name(name) {
        return Err(otto_core::Error::Invalid("invalid AVD name".into()));
    }
    let java_home = discover_java_home(runner, probe).await;
    let spec = with_android_sdk_env(
        CmdSpec::new(avdm, ["delete", "avd", "-n", name]).with_timeout(Duration::from_secs(30)),
        tools.sdk_root.as_deref(),
        java_home.as_deref(),
    );
    let out = runner.run(spec).await?;
    if !out.success() {
        return Err(classify_avdmanager_err(
            "avdmanager delete avd",
            out.combined_err(),
        ));
    }
    Ok(())
}

pub async fn boot_avd(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    adb_lock: &Mutex<()>,
    tools: &ResolvedAndroidTools,
    avd_name: &str,
    req: &BootSimDeviceReq,
) -> otto_core::Result<()> {
    let emu = require_emulator(tools)?;
    if !is_safe_avd_name(avd_name) {
        return Err(otto_core::Error::Invalid("invalid AVD name".into()));
    }
    // Idempotency: if this AVD already has an active serial, do not
    // spawn a colliding duplicate QEMU instance.
    let existing = list_devices(runner, probe, adb_lock, tools).await?;
    let already_running = existing
        .iter()
        .find(|d| d.avd_name.as_deref() == Some(avd_name))
        .and_then(|d| d.serial.as_deref())
        .is_some();

    if !already_running {
        let mut args = vec![
            "-avd".to_string(),
            avd_name.to_string(),
            "-no-boot-anim".to_string(),
        ];
        // Embedded-first: headless (`-no-window`, mirrored inside Otto) unless
        // the caller asks for a window (`headless: false` or `open_gui: true`).
        if req.headless.unwrap_or(!req.open_gui.unwrap_or(false)) {
            args.push("-no-window".into());
        }
        if req.wipe_data.unwrap_or(false) {
            args.push("-wipe-data".into());
        }
        let spec = with_android_sdk_env(CmdSpec::new(emu, args), tools.sdk_root.as_deref(), None);
        runner.spawn_detached(spec).await?;
    }

    if req.wait.unwrap_or(false) {
        let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(120).clamp(5, 300));
        wait_for_avd_boot(runner, probe, adb_lock, tools, avd_name, timeout).await?;
    }
    Ok(())
}

async fn wait_for_avd_boot(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    adb_lock: &Mutex<()>,
    tools: &ResolvedAndroidTools,
    avd_name: &str,
    timeout: Duration,
) -> otto_core::Result<()> {
    let adb = require_adb(tools)?;
    ensure_adb_server(runner, adb_lock, &adb).await;
    let start = Instant::now();
    while start.elapsed() < timeout {
        let devs = list_devices(runner, probe, adb_lock, tools).await?;
        if let Some(d) = devs
            .into_iter()
            .find(|d| d.avd_name.as_deref() == Some(avd_name))
        {
            if let Some(serial) = d.serial {
                if let Ok(out) = runner
                    .run(
                        CmdSpec::new(
                            &adb,
                            ["-s", &serial, "shell", "getprop", "sys.boot_completed"],
                        )
                        .with_timeout(Duration::from_secs(5)),
                    )
                    .await
                {
                    if out.success() && out.stdout_str().trim() == "1" {
                        return Ok(());
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(otto_core::Error::Upstream(format!(
        "timed out waiting for Android AVD {avd_name} to finish booting"
    )))
}

pub async fn shutdown_serial(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
) -> otto_core::Result<()> {
    let adb = require_adb(tools)?;
    let out = runner
        .run(
            CmdSpec::new(&adb, ["-s", serial, "emu", "kill"]).with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb emu kill failed: {}",
            out.combined_err()
        )));
    }
    // `emu kill` returns before QEMU exits. Wait (bounded) until adb drops the
    // serial, so a restart's boot does not find the dying instance and skip
    // the spawn (leaving the device down) or collide with its AVD lock.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        let out = runner
            .run(CmdSpec::new(&adb, ["devices"]).with_timeout(Duration::from_secs(5)))
            .await?;
        let listed = |l: &str| l.split_whitespace().eq([serial, "device"]);
        if !out.stdout_str().lines().any(listed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::tests::ready_android_tools;
    use crate::exec::fake::{err_out, FakeCommandRunner, FakeHostProbe};

    #[tokio::test]
    async fn create_avd_rejects_oversized_or_leading_dash_image_and_maps_delete_errors() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let tools = ready_android_tools();

        // Leading dash and oversized system_image_id are rejected with 400.
        assert!(matches!(
            create_avd(&runner, &probe, &tools, "Pixel_A", "--bad-flag", None).await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            create_avd(&runner, &probe, &tools, "Pixel_A", &"a".repeat(260), None).await,
            Err(otto_core::Error::Invalid(_))
        ));

        // Missing AVD → 404 NotFound, Java crash → 502 Upstream.
        runner.on_exact(
            "avdmanager",
            &["delete", "avd", "-n", "Missing_AVD"],
            err_out(b"Error: There is no Android Virtual Device with name 'Missing_AVD'."),
        );
        assert!(matches!(
            delete_avd(&runner, &probe, &tools, "Missing_AVD").await,
            Err(otto_core::Error::NotFound(_))
        ));

        runner.on_exact(
            "avdmanager",
            &["delete", "avd", "-n", "Java_Fail"],
            err_out(b"Unable to locate a Java Runtime."),
        );
        assert!(matches!(
            delete_avd(&runner, &probe, &tools, "Java_Fail").await,
            Err(otto_core::Error::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn boot_avd_defaults_to_headless_no_window_and_allows_native_opt_out() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let tools = ready_android_tools();
        let lock = Mutex::new(());
        runner.on_exact(
            "emulator",
            &["-list-avds"],
            crate::exec::fake::ok_out(b"Pixel_Phone\n"),
        );
        runner.on_exact(
            "adb",
            &["devices", "-l"],
            crate::exec::fake::ok_out(b"List of devices attached\n"),
        );

        // Default boot passes `-no-window` (embedded-first).
        boot_avd(
            &runner,
            &probe,
            &lock,
            &tools,
            "Pixel_Phone",
            &BootSimDeviceReq::default(),
        )
        .await
        .unwrap();
        let detached = runner.detached();
        let (_, emu_default_args) = detached
            .iter()
            .rev()
            .find(|(prog, args)| prog.ends_with("emulator") && args.iter().any(|a| a == "-avd"))
            .expect("emulator spawn recorded");
        assert!(
            emu_default_args.iter().any(|a| a == "-no-window"),
            "default boot must pass -no-window, got: {emu_default_args:?}"
        );

        // `headless: false` (the UI's pop-out) or `open_gui: true` (what the
        // MCP approval card calls "native window") opens a window.
        for req in [
            BootSimDeviceReq {
                headless: Some(false),
                ..Default::default()
            },
            BootSimDeviceReq {
                open_gui: Some(true),
                ..Default::default()
            },
        ] {
            boot_avd(&runner, &probe, &lock, &tools, "Pixel_Phone", &req)
                .await
                .unwrap();
            let detached = runner.detached();
            let (_, emu_gui_args) = detached.last().expect("emulator spawn recorded");
            assert!(
                !emu_gui_args.iter().any(|a| a == "-no-window"),
                "{req:?} must omit -no-window, got: {emu_gui_args:?}"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_waits_until_adb_drops_the_dying_emulator() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let runner = FakeCommandRunner::new();
        runner.on_exact(
            "adb",
            &["-s", "emulator-5554", "emu", "kill"],
            crate::exec::fake::ok_out(b"OK: killing emulator, bye bye\n"),
        );
        let polls = Arc::new(AtomicUsize::new(0));
        let p = polls.clone();
        runner.on(move |spec| {
            (spec.args == ["devices"]).then(|| {
                // Still listed right after `emu kill`, gone on the next poll.
                let out: &[u8] = if p.fetch_add(1, Ordering::SeqCst) == 0 {
                    b"List of devices attached\nemulator-5554\tdevice\n"
                } else {
                    b"List of devices attached\n"
                };
                crate::exec::fake::ok_out(out)
            })
        });
        shutdown_serial(&runner, &ready_android_tools(), "emulator-5554")
            .await
            .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }
}

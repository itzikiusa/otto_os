//! Android device controls: open URL with auto `adb reverse`, screencap,
//! input (tap/swipe/text/keyevent), rotation, dark mode, port forwards, apps, logs.

use std::path::Path;
use std::time::Duration;

use crate::detect::ResolvedAndroidTools;
use crate::exec::{CmdSpec, CommandRunner, HostProbe, LogStreamHandle};
use crate::types::{
    extract_localhost_port, image_dimensions, is_safe_token, resolve_point, validate_open_url,
    ActionOkResp, Appearance, InputAction, OpenSimUrlResp, Orientation, PortForward, ScreenMetrics,
    ScreenshotData, MAX_INPUT_TEXT_CHARS,
};

use super::require_adb;

pub async fn open_url(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    raw_url: &str,
    auto_forward_localhost: bool,
) -> otto_core::Result<OpenSimUrlResp> {
    let adb = require_adb(tools)?;
    let url = validate_open_url(raw_url)?;
    let (effective_url, local_port) = extract_localhost_port(&url);
    let mut forwarded_ports = Vec::new();

    if auto_forward_localhost {
        if let Some(port) = local_port {
            add_port_forward(runner, tools, serial, port, port).await?;
            forwarded_ports.push(port);
        }
    }

    // `adb shell` joins its args with spaces and runs them through `/system/bin/sh`:
    // single-quoting keeps `&`, `;`, `$()`, etc. literal on the device.
    let quoted = quote_device_shell_arg(&effective_url);
    let out = runner
        .run(
            CmdSpec::new(
                adb,
                [
                    "-s",
                    serial,
                    "shell",
                    "am",
                    "start",
                    "-W",
                    "-a",
                    "android.intent.action.VIEW",
                    "-d",
                    &quoted,
                ],
            )
            .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    let stdout = out.stdout_str();
    if !out.success() || stdout.lines().any(|l| l.trim_start().starts_with("Error")) {
        return Err(otto_core::Error::Upstream(format!(
            "adb open-url failed: {}",
            out.combined_err()
        )));
    }
    Ok(OpenSimUrlResp {
        ok: true,
        url,
        effective_url,
        forwarded_ports,
    })
}

pub async fn screenshot(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
) -> otto_core::Result<ScreenshotData> {
    let adb = require_adb(tools)?;
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "exec-out", "screencap", "-p"])
                .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb screencap failed: {}",
            out.combined_err()
        )));
    }
    if out.stdout.is_empty() {
        return Err(otto_core::Error::Upstream(
            "adb screencap returned empty image".into(),
        ));
    }
    let (width, height) = image_dimensions(&out.stdout).unwrap_or((0, 0));
    Ok(ScreenshotData {
        mime: "image/png".to_string(),
        width,
        height,
        bytes: out.stdout,
        captured_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn send_input(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    screen: Option<&ScreenMetrics>,
    orientation: Orientation,
    action: &InputAction,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    let args: Vec<String> = match action {
        InputAction::Tap { x, y, normalized } => {
            let (px, py) =
                resolve_point(*x, *y, *normalized, screen, orientation, (1080.0, 2400.0))?;
            vec![
                "-s".into(),
                serial.into(),
                "shell".into(),
                "input".into(),
                "tap".into(),
                px.to_string(),
                py.to_string(),
            ]
        }
        InputAction::Swipe {
            x1,
            y1,
            x2,
            y2,
            duration_ms,
            normalized,
        } => {
            let (ax, ay) =
                resolve_point(*x1, *y1, *normalized, screen, orientation, (1080.0, 2400.0))?;
            let (bx, by) =
                resolve_point(*x2, *y2, *normalized, screen, orientation, (1080.0, 2400.0))?;
            let dur = duration_ms.unwrap_or(250).clamp(50, 5000);
            vec![
                "-s".into(),
                serial.into(),
                "shell".into(),
                "input".into(),
                "swipe".into(),
                ax.to_string(),
                ay.to_string(),
                bx.to_string(),
                by.to_string(),
                dur.to_string(),
            ]
        }
        InputAction::Text { text } => {
            if text.is_empty() || text.chars().count() > MAX_INPUT_TEXT_CHARS {
                return Err(otto_core::Error::Invalid(
                    "text input must be 1..=16384 characters".into(),
                ));
            }
            if text.chars().any(|c| c.is_control()) {
                return Err(otto_core::Error::Invalid(
                    "text input cannot contain control characters; use key_event for Enter/Del"
                        .into(),
                ));
            }
            let escaped = escape_adb_input_text(text);
            vec![
                "-s".into(),
                serial.into(),
                "shell".into(),
                "input".into(),
                "text".into(),
                escaped,
            ]
        }
        InputAction::KeyEvent { key } => {
            let code = validate_android_keycode(key)?;
            vec![
                "-s".into(),
                serial.into(),
                "shell".into(),
                "input".into(),
                "keyevent".into(),
                code,
            ]
        }
    };

    let out = runner
        .run(CmdSpec::new(adb, args).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb input failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn set_orientation(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    orientation: Orientation,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    let rot = match orientation {
        Orientation::Portrait => "0",
        Orientation::Landscape => "1",
        Orientation::PortraitUpsideDown => "2",
        Orientation::LandscapeRight => "3",
        Orientation::Unknown => {
            return Err(otto_core::Error::Invalid("invalid orientation".into()));
        }
    };
    let disable_auto = runner
        .run(
            CmdSpec::new(
                &adb,
                [
                    "-s",
                    serial,
                    "shell",
                    "settings",
                    "put",
                    "system",
                    "accelerometer_rotation",
                    "0",
                ],
            )
            .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !disable_auto.success() {
        return Err(otto_core::Error::Upstream(format!(
            "failed to disable auto-rotation: {}",
            disable_auto.combined_err()
        )));
    }
    let out = runner
        .run(
            CmdSpec::new(
                &adb,
                [
                    "-s",
                    serial,
                    "shell",
                    "settings",
                    "put",
                    "system",
                    "user_rotation",
                    rot,
                ],
            )
            .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "failed to set Android rotation: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn set_appearance(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    appearance: Appearance,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    let val = match appearance {
        Appearance::Dark => "yes",
        Appearance::Light => "no",
    };
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "shell", "cmd", "uimode", "night", val])
                .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "failed to set Android appearance: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn list_port_forwards(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
) -> otto_core::Result<Vec<PortForward>> {
    let adb = require_adb(tools)?;
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "reverse", "--list"])
                .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb reverse --list failed: {}",
            out.combined_err()
        )));
    }
    let mut ports = Vec::new();
    for line in out.stdout_str().lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 3 {
            if let (Some(r), Some(l)) = (
                toks[1]
                    .strip_prefix("tcp:")
                    .and_then(|s| s.parse::<u16>().ok()),
                toks[2]
                    .strip_prefix("tcp:")
                    .and_then(|s| s.parse::<u16>().ok()),
            ) {
                ports.push(PortForward {
                    remote_port: r,
                    local_port: l,
                    protocol: "tcp".into(),
                });
            }
        }
    }
    Ok(ports)
}

pub async fn add_port_forward(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    remote_port: u16,
    local_port: u16,
) -> otto_core::Result<PortForward> {
    if remote_port == 0 || local_port == 0 {
        return Err(otto_core::Error::Invalid("ports must be 1..=65535".into()));
    }
    let adb = require_adb(tools)?;
    let remote = format!("tcp:{remote_port}");
    let local = format!("tcp:{local_port}");
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "reverse", &remote, &local])
                .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb reverse {remote} {local} failed: {}",
            out.combined_err()
        )));
    }
    Ok(PortForward {
        remote_port,
        local_port,
        protocol: "tcp".into(),
    })
}

pub async fn remove_port_forward(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    remote_port: u16,
) -> otto_core::Result<()> {
    if remote_port == 0 {
        return Err(otto_core::Error::Invalid(
            "remote_port must be 1..=65535".into(),
        ));
    }
    let adb = require_adb(tools)?;
    let remote = format!("tcp:{remote_port}");
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "reverse", "--remove", &remote])
                .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb reverse --remove {remote} failed: {}",
            out.combined_err()
        )));
    }
    Ok(())
}

pub async fn install_app(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    tools: &ResolvedAndroidTools,
    serial: &str,
    path: &str,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    let trimmed = path.trim();
    let p = Path::new(trimmed);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("apk"));
    if trimmed.is_empty() || !p.is_absolute() || !ext_ok {
        return Err(otto_core::Error::Invalid(
            "app path must be an absolute path to an .apk file".into(),
        ));
    }
    if !probe.path_exists(p) {
        return Err(otto_core::Error::NotFound(format!(
            "APK not found at {trimmed}"
        )));
    }
    let out = runner
        .run(
            CmdSpec::new(adb, ["-s", serial, "install", "-r", trimmed])
                .with_timeout(Duration::from_secs(120)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb install failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn launch_app(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    package_name: &str,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    if !is_safe_token(package_name) {
        return Err(otto_core::Error::Invalid("invalid package name".into()));
    }
    let out = runner
        .run(
            CmdSpec::new(
                adb,
                [
                    "-s",
                    serial,
                    "shell",
                    "monkey",
                    "-p",
                    package_name,
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
            )
            .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb launch failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn terminate_app(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    package_name: &str,
) -> otto_core::Result<ActionOkResp> {
    let adb = require_adb(tools)?;
    if !is_safe_token(package_name) {
        return Err(otto_core::Error::Invalid("invalid package name".into()));
    }
    let out = runner
        .run(
            CmdSpec::new(
                adb,
                ["-s", serial, "shell", "am", "force-stop", package_name],
            )
            .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb force-stop failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn spawn_logs(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
) -> otto_core::Result<LogStreamHandle> {
    let adb = require_adb(tools)?;
    runner
        .spawn_log_stream(CmdSpec::new(adb, ["-s", serial, "logcat", "-v", "time"]))
        .await
}

/// Single-quote one argument for the Android device shell (`adb shell …`):
/// everything inside `'…'` is literal; an embedded `'` becomes `'\''`.
pub fn quote_device_shell_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Escape text for `adb shell input text <arg>`.
pub fn escape_adb_input_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            ' ' => out.push_str("%s"),
            '\\' | '"' | '\'' | '$' | '`' | '&' | ';' | '|' | '(' | ')' | '<' | '>' | '*' | '?'
            | '[' | ']' | '{' | '}' | '~' | '#' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out
}

pub fn validate_android_keycode(raw: &str) -> otto_core::Result<String> {
    let trimmed = raw.trim();
    if let Ok(num) = trimmed.parse::<u16>() {
        if num <= 320 {
            return Ok(num.to_string());
        }
        return Err(otto_core::Error::Invalid(
            "numeric Android keycode must be 0..=320".into(),
        ));
    }
    let normalized = trimmed
        .to_ascii_uppercase()
        .strip_prefix("KEYCODE_")
        .unwrap_or(&trimmed.to_ascii_uppercase())
        .to_string();
    const ALLOWED: &[&str] = &[
        "HOME",
        "BACK",
        "APP_SWITCH",
        "ENTER",
        "DEL",
        "FORWARD_DEL",
        "TAB",
        "SPACE",
        "ESCAPE",
        "POWER",
        "MENU",
        "VOLUME_UP",
        "VOLUME_DOWN",
        "VOLUME_MUTE",
        "DPAD_UP",
        "DPAD_DOWN",
        "DPAD_LEFT",
        "DPAD_RIGHT",
        "DPAD_CENTER",
        "CAMERA",
        "SEARCH",
        "MEDIA_PLAY_PAUSE",
        "WAKEUP",
        "SLEEP",
    ];
    if ALLOWED.contains(&normalized.as_str()) {
        Ok(format!("KEYCODE_{normalized}"))
    } else {
        Err(otto_core::Error::Invalid(format!(
            "unsupported Android key_event {raw:?}"
        )))
    }
}

/// Fetch recent logcat lines without blocking (`adb -s <serial> logcat -d -v time -t <n>`).
pub async fn recent_logs(
    runner: &dyn CommandRunner,
    tools: &ResolvedAndroidTools,
    serial: &str,
    max_lines: usize,
) -> otto_core::Result<String> {
    let adb = require_adb(tools)?;
    // Over-fetch for the level/filter applied after; the handler caps max_lines at 500.
    let fetch = (max_lines * 4).min(1000);
    let fetch_str = fetch.to_string();
    let out = runner
        .run(
            CmdSpec::new(
                adb,
                ["-s", serial, "logcat", "-d", "-v", "time", "-t", &fetch_str],
            )
            .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "adb logcat failed: {}",
            out.combined_err()
        )));
    }
    Ok(out.stdout_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::tests::ready_android_tools;
    use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};

    #[tokio::test]
    async fn open_url_auto_forwards_and_single_quotes_shell_metacharacters() {
        let runner = FakeCommandRunner::new();
        let tools = ready_android_tools();
        runner.on_exact(
            "adb",
            &["-s", "emulator-5554", "reverse", "tcp:5173", "tcp:5173"],
            ok_out(b"5173\n"),
        );
        runner.on_exact(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                "'http://127.0.0.1:5173/app'",
            ],
            ok_out(b"Starting: Intent\nStatus: ok\n"),
        );
        let resp = open_url(
            &runner,
            &tools,
            "emulator-5554",
            "http://0.0.0.0:5173/app",
            true,
        )
        .await
        .unwrap();
        assert!(resp.ok);
        assert_eq!(resp.effective_url, "http://127.0.0.1:5173/app");
        assert_eq!(resp.forwarded_ports, vec![5173]);

        // Multi-query URL with `&`
        runner.on_exact(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                "'http://localhost:5173/?a=1&b=2'",
            ],
            ok_out(b"Starting: Intent\nStatus: ok\n"),
        );
        open_url(
            &runner,
            &tools,
            "emulator-5554",
            "http://localhost:5173/?a=1&b=2",
            true,
        )
        .await
        .unwrap();

        // Semicolon + embedded single-quote
        runner.on_exact(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "start",
                "-W",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                r"'https://example.com/it'\''s;reboot'",
            ],
            ok_out(b"Starting: Intent\nStatus: ok\n"),
        );
        open_url(
            &runner,
            &tools,
            "emulator-5554",
            "https://example.com/it's;reboot",
            false,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn send_input_swaps_landscape_coords_and_rejects_negative_pixels() {
        let runner = FakeCommandRunner::new();
        let tools = ready_android_tools();
        let screen = ScreenMetrics {
            width: 1080,
            height: 2424,
            scale: Some(2.625),
            dpi: Some(420),
        };
        runner.on_exact(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "input",
                "tap",
                "2060",
                "540",
            ],
            ok_out(b""),
        );
        send_input(
            &runner,
            &tools,
            "emulator-5554",
            Some(&screen),
            Orientation::Landscape,
            &InputAction::Tap {
                x: 0.85,
                y: 0.50,
                normalized: true,
            },
        )
        .await
        .unwrap();

        assert!(matches!(
            send_input(
                &runner,
                &tools,
                "emulator-5554",
                Some(&screen),
                Orientation::Portrait,
                &InputAction::Tap {
                    x: -50.0,
                    y: -10.0,
                    normalized: false,
                },
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn install_app_validates_extension_and_existence() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let tools = ready_android_tools();

        assert!(matches!(
            install_app(&runner, &probe, &tools, "emulator-5554", "/etc/passwd").await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            install_app(&runner, &probe, &tools, "emulator-5554", "/tmp/missing.apk").await,
            Err(otto_core::Error::NotFound(_))
        ));
        probe.add_path("/tmp/app-debug.apk");
        runner.on_exact(
            "adb",
            &["-s", "emulator-5554", "install", "-r", "/tmp/app-debug.apk"],
            ok_out(b"Success\n"),
        );
        install_app(
            &runner,
            &probe,
            &tools,
            "emulator-5554",
            "/tmp/app-debug.apk",
        )
        .await
        .unwrap();
    }

    #[test]
    fn escapes_adb_text_and_validates_keycodes() {
        assert_eq!(
            escape_adb_input_text("hello world$1;rm"),
            r"hello%sworld\$1\;rm"
        );
        assert_eq!(validate_android_keycode("home").unwrap(), "KEYCODE_HOME");
        assert_eq!(
            validate_android_keycode("KEYCODE_BACK").unwrap(),
            "KEYCODE_BACK"
        );
        assert_eq!(validate_android_keycode("66").unwrap(), "66");
        assert!(validate_android_keycode("EVIL;rm").is_err());
        assert!(validate_android_keycode("999").is_err());
    }
}

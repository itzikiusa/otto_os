//! iOS Simulator controls: URLs, screenshots, appearance, orientation,
//! status bar, clipboard, optional `idb` input, app management, and logs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::detect::ResolvedIosTools;
use crate::exec::{CmdSpec, CommandRunner, HostProbe, LogStreamHandle};
use crate::types::{
    image_dimensions, is_safe_token, resolve_point, validate_open_url, ActionOkResp, Appearance,
    ClipboardResp, InputAction, OpenSimUrlResp, Orientation, ScreenMetrics, ScreenshotData,
    ScreenshotFormat, StatusBarReq, MAX_INPUT_TEXT_CHARS,
};

use super::require_xcrun;

const MAX_LAUNCH_ARGS: usize = 64;
const MAX_LAUNCH_ARG_LEN: usize = 4096;

static SHOT_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub async fn open_url(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    raw_url: &str,
) -> otto_core::Result<OpenSimUrlResp> {
    let xcrun = require_xcrun(tools)?;
    let url = validate_open_url(raw_url)?;
    let out = runner
        .run(
            CmdSpec::new(xcrun, ["simctl", "openurl", udid, &url])
                .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl openurl failed: {}",
            out.combined_err()
        )));
    }
    Ok(OpenSimUrlResp {
        ok: true,
        url: url.clone(),
        effective_url: url,
        forwarded_ports: Vec::new(),
    })
}

pub async fn screenshot(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    format: ScreenshotFormat,
) -> otto_core::Result<ScreenshotData> {
    let xcrun = require_xcrun(tools)?;
    let type_flag = format!("--type={}", format.simctl_type());
    let ext = format.simctl_type();
    let seq = SHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = std::env::temp_dir().join(format!(
        "otto-sim-shot-{}-{}-{}.{}",
        std::process::id(),
        ts,
        seq,
        ext
    ));
    let _cleanup = TempFileGuard(tmp_path.clone());
    let tmp_str = tmp_path.to_string_lossy().into_owned();

    // Xcode 27.2's `simctl io <udid> screenshot -` writes a literal `./-` file
    // into CWD instead of stdout, so we always pass an explicit unique temp path.
    let out = runner
        .run(
            CmdSpec::new(
                xcrun,
                ["simctl", "io", udid, "screenshot", &type_flag, &tmp_str],
            )
            .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl screenshot failed: {}",
            out.combined_err()
        )));
    }
    let bytes = match tokio::fs::read(&tmp_path).await {
        Ok(b) if !b.is_empty() => b,
        _ => {
            if out.stdout.starts_with(b"\x89PNG")
                || (out.stdout.len() >= 2 && out.stdout[0] == 0xFF && out.stdout[1] == 0xD8)
            {
                out.stdout
            } else {
                return Err(otto_core::Error::Upstream(
                    "simctl screenshot produced an empty or unreadable image file".into(),
                ));
            }
        }
    };
    let (width, height) = image_dimensions(&bytes).unwrap_or((0, 0));
    Ok(ScreenshotData {
        mime: format.mime().to_string(),
        width,
        height,
        bytes,
        captured_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn set_appearance(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    appearance: Appearance,
) -> otto_core::Result<ActionOkResp> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(
            CmdSpec::new(
                xcrun,
                ["simctl", "ui", udid, "appearance", appearance.as_str()],
            )
            .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl ui appearance failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn set_orientation(
    runner: &dyn CommandRunner,
    _tools: &ResolvedIosTools,
    udid: &str,
    orientation: Orientation,
) -> otto_core::Result<ActionOkResp> {
    let menu_item = match orientation {
        Orientation::Portrait => "Portrait",
        Orientation::Landscape => "Landscape Left",
        Orientation::PortraitUpsideDown => "Portrait Upside Down",
        Orientation::LandscapeRight => "Landscape Right",
        Orientation::Unknown => {
            return Err(otto_core::Error::Invalid("invalid orientation".into()));
        }
    };
    let _ = runner
        .run(
            CmdSpec::new(
                "open",
                ["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid],
            )
            .with_timeout(Duration::from_secs(5)),
        )
        .await;
    let script = format!(
        "tell application \"System Events\"\n\
         if not (exists process \"Simulator\") then error \"Simulator.app is not running\"\n\
         tell process \"Simulator\"\n\
         click menu item \"{menu_item}\" of menu 1 of menu item \"Orientation\" of menu 1 of menu bar item \"Device\" of menu bar 1\n\
         end tell\n\
         end tell"
    );
    let out = runner
        .run(CmdSpec::new("osascript", ["-e", &script]).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "iOS Simulator rotation requires Simulator.app open with Accessibility permission: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::with_detail(format!(
        "rotated {udid} to {menu_item} via Simulator.app"
    )))
}

fn validate_status_bar_str<'a>(raw: &'a str, field: &str) -> otto_core::Result<&'a str> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 32 || t.starts_with('-') || t.chars().any(|c| c.is_control()) {
        return Err(otto_core::Error::Invalid(format!(
            "invalid status_bar {field}: must be 1..=32 non-control chars and not start with '-'"
        )));
    }
    Ok(t)
}

pub async fn set_status_bar(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    req: &StatusBarReq,
) -> otto_core::Result<ActionOkResp> {
    let xcrun = require_xcrun(tools)?;
    let mut args = vec![
        "simctl".to_string(),
        "status_bar".to_string(),
        udid.to_string(),
    ];
    if req.clear {
        args.push("clear".to_string());
    } else {
        args.push("override".to_string());
        if let Some(time) = &req.time {
            let t = validate_status_bar_str(time, "time")?;
            args.push("--time".into());
            args.push(t.to_string());
        }
        if let Some(lvl) = req.battery_level {
            args.push("--batteryLevel".into());
            args.push(lvl.min(100).to_string());
        }
        if let Some(state) = &req.battery_state {
            if matches!(state.as_str(), "charging" | "charged" | "discharging") {
                args.push("--batteryState".into());
                args.push(state.clone());
            } else {
                return Err(otto_core::Error::Invalid(
                    "battery_state must be charging, charged, or discharging".into(),
                ));
            }
        }
        if let Some(bars) = req.wifi_bars {
            args.push("--wifiBars".into());
            args.push(bars.min(3).to_string());
        }
        if let Some(bars) = req.cellular_bars {
            args.push("--cellularBars".into());
            args.push(bars.min(4).to_string());
        }
        if let Some(op) = &req.operator_name {
            let o = validate_status_bar_str(op, "operator_name")?;
            args.push("--operatorName".into());
            args.push(o.to_string());
        }
        if args.len() == 4 {
            return Err(otto_core::Error::Invalid(
                "provide at least one status_bar override field or set clear=true".into(),
            ));
        }
    }
    let out = runner
        .run(CmdSpec::new(xcrun, args).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl status_bar failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn get_clipboard(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
) -> otto_core::Result<ClipboardResp> {
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(CmdSpec::new(xcrun, ["simctl", "pbpaste", udid]).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl pbpaste failed: {}",
            out.combined_err()
        )));
    }
    Ok(ClipboardResp {
        text: out.stdout_str(),
    })
}

pub async fn set_clipboard(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    text: &str,
) -> otto_core::Result<ActionOkResp> {
    if text.chars().count() > MAX_INPUT_TEXT_CHARS {
        return Err(otto_core::Error::Invalid("clipboard text too long".into()));
    }
    let xcrun = require_xcrun(tools)?;
    let out = runner
        .run(
            CmdSpec::new(xcrun, ["simctl", "pbcopy", udid])
                .with_stdin(text.as_bytes().to_vec())
                .with_timeout(Duration::from_secs(10)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl pbcopy failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn send_input(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    screen: Option<&ScreenMetrics>,
    orientation: Orientation,
    action: &InputAction,
) -> otto_core::Result<ActionOkResp> {
    let Some(idb) = &tools.idb else {
        return Err(otto_core::Error::Invalid(
            "Direct touch/key input on iOS Simulator requires the optional `idb` CLI (`brew tap facebook/fb && brew install idb-companion`). Use Open URL, Clipboard, Appearance, or Status Bar, or install idb."
                .into(),
        ));
    };
    let idb_str = idb.to_string_lossy().into_owned();
    let args: Vec<String> = match action {
        InputAction::Tap { x, y, normalized } => {
            let (px, py) = resolve_point(*x, *y, *normalized, screen, orientation, (393.0, 852.0))?;
            vec![
                "ui".into(),
                "tap".into(),
                "--udid".into(),
                udid.into(),
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
                resolve_point(*x1, *y1, *normalized, screen, orientation, (393.0, 852.0))?;
            let (bx, by) =
                resolve_point(*x2, *y2, *normalized, screen, orientation, (393.0, 852.0))?;
            let dur = format!(
                "{:.2}",
                f64::from(duration_ms.unwrap_or(250).clamp(50, 5000)) / 1000.0
            );
            vec![
                "ui".into(),
                "swipe".into(),
                "--udid".into(),
                udid.into(),
                "--duration".into(),
                dur,
                ax.to_string(),
                ay.to_string(),
                bx.to_string(),
                by.to_string(),
            ]
        }
        InputAction::Text { text } => {
            if text.is_empty()
                || text.chars().count() > MAX_INPUT_TEXT_CHARS
                || text.chars().any(|c| c.is_control())
            {
                return Err(otto_core::Error::Invalid(
                    "input text must be 1..=16384 non-control characters".into(),
                ));
            }
            vec![
                "ui".into(),
                "text".into(),
                "--udid".into(),
                udid.into(),
                "--".into(),
                text.clone(),
            ]
        }
        InputAction::KeyEvent { key } => {
            let btn = match key.to_ascii_uppercase().as_str() {
                "HOME" => "HOME",
                "LOCK" | "POWER" => "LOCK",
                "SIRI" => "SIRI",
                "APPLE_PAY" => "APPLE_PAY",
                other => {
                    return Err(otto_core::Error::Invalid(format!(
                        "unsupported iOS idb button {other:?}; supported: HOME, LOCK, SIRI, APPLE_PAY"
                    )));
                }
            };
            vec![
                "ui".into(),
                "button".into(),
                "--udid".into(),
                udid.into(),
                btn.into(),
            ]
        }
    };
    let out = runner
        .run(CmdSpec::new(idb_str, args).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "idb input failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn install_app(
    runner: &dyn CommandRunner,
    probe: &dyn HostProbe,
    tools: &ResolvedIosTools,
    udid: &str,
    path: &str,
) -> otto_core::Result<ActionOkResp> {
    let xcrun = require_xcrun(tools)?;
    let trimmed = path.trim();
    let p = Path::new(trimmed);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("app") || e.eq_ignore_ascii_case("ipa"));
    if trimmed.is_empty() || !p.is_absolute() || !ext_ok {
        return Err(otto_core::Error::Invalid(
            "app path must be an absolute path to a .app bundle or .ipa file".into(),
        ));
    }
    if !probe.path_exists(p) {
        return Err(otto_core::Error::NotFound(format!(
            "app bundle not found at {trimmed}"
        )));
    }
    let out = runner
        .run(
            CmdSpec::new(xcrun, ["simctl", "install", udid, trimmed])
                .with_timeout(Duration::from_secs(120)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl install failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn launch_app(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    bundle_id: &str,
    extra_args: &[String],
) -> otto_core::Result<ActionOkResp> {
    let xcrun = require_xcrun(tools)?;
    if !is_safe_token(bundle_id) {
        return Err(otto_core::Error::Invalid("invalid bundle_id".into()));
    }
    if extra_args.len() > MAX_LAUNCH_ARGS {
        return Err(otto_core::Error::Invalid(format!(
            "launch args exceed limit ({MAX_LAUNCH_ARGS})"
        )));
    }
    for arg in extra_args {
        if arg.len() > MAX_LAUNCH_ARG_LEN || arg.chars().any(|c| c.is_control()) {
            return Err(otto_core::Error::Invalid(
                "launch arg exceeds length limit or contains control characters".into(),
            ));
        }
    }
    let mut args = vec![
        "simctl".to_string(),
        "launch".to_string(),
        udid.to_string(),
        bundle_id.to_string(),
    ];
    args.extend(extra_args.iter().cloned());
    let out = runner
        .run(CmdSpec::new(xcrun, args).with_timeout(Duration::from_secs(30)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl launch failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::with_detail(
        out.stdout_str().trim().to_string(),
    ))
}

pub async fn terminate_app(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    bundle_id: &str,
) -> otto_core::Result<ActionOkResp> {
    let xcrun = require_xcrun(tools)?;
    if !is_safe_token(bundle_id) {
        return Err(otto_core::Error::Invalid("invalid bundle_id".into()));
    }
    let out = runner
        .run(
            CmdSpec::new(xcrun, ["simctl", "terminate", udid, bundle_id])
                .with_timeout(Duration::from_secs(15)),
        )
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl terminate failed: {}",
            out.combined_err()
        )));
    }
    Ok(ActionOkResp::ok())
}

pub async fn spawn_logs(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
) -> otto_core::Result<LogStreamHandle> {
    let xcrun = require_xcrun(tools)?;
    runner
        .spawn_log_stream(CmdSpec::new(
            xcrun,
            [
                "simctl", "spawn", udid, "log", "stream", "--style", "compact",
            ],
        ))
        .await
}

/// Fetch the last 2 minutes of iOS Simulator log lines (`xcrun simctl spawn
/// <udid> log show --style compact --last 2m --info [--debug]`).
pub async fn recent_logs(
    runner: &dyn CommandRunner,
    tools: &ResolvedIosTools,
    udid: &str,
    debug: bool,
) -> otto_core::Result<String> {
    let xcrun = require_xcrun(tools)?;
    let mut args = vec![
        "simctl", "spawn", udid, "log", "show", "--style", "compact", "--last", "2m", "--info",
    ];
    if debug {
        args.push("--debug");
    }
    let out = runner
        .run(CmdSpec::new(xcrun, args).with_timeout(Duration::from_secs(10)))
        .await?;
    if !out.success() {
        return Err(otto_core::Error::Upstream(format!(
            "simctl log show failed: {}",
            out.combined_err()
        )));
    }
    Ok(out.stdout_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::{ok_out, FakeCommandRunner, FakeHostProbe};
    use crate::ios::boot_device;
    use crate::ios::tests::ready_ios_tools;
    use crate::types::BootSimDeviceReq;

    #[tokio::test]
    async fn boot_open_url_screenshot_and_status_bar() {
        let runner = FakeCommandRunner::new();
        let tools = ready_ios_tools(true);
        runner.on_exact("xcrun", &["simctl", "boot", "AAA-111"], ok_out(b""));
        runner.on_exact(
            "xcrun",
            &["simctl", "openurl", "AAA-111", "http://localhost:5173/"],
            ok_out(b""),
        );
        let mut fake_png = Vec::new();
        fake_png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        fake_png.extend_from_slice(&13u32.to_be_bytes());
        fake_png.extend_from_slice(b"IHDR");
        fake_png.extend_from_slice(&1179u32.to_be_bytes());
        fake_png.extend_from_slice(&2556u32.to_be_bytes());
        let fake_png_clone = fake_png.clone();
        runner.on(move |spec| {
            if spec.program.ends_with("xcrun")
                && spec.args.len() == 6
                && spec.args[0] == "simctl"
                && spec.args[1] == "io"
                && spec.args[2] == "AAA-111"
                && spec.args[3] == "screenshot"
                && spec.args[4] == "--type=png"
            {
                assert_ne!(spec.args[5], "-", "must use explicit temp path, not '-'");
                std::fs::write(&spec.args[5], &fake_png_clone).unwrap();
                Some(ok_out(
                    format!("Wrote screenshot to: {}\n", spec.args[5]).into_bytes(),
                ))
            } else {
                None
            }
        });
        runner.on_exact(
            "xcrun",
            &[
                "simctl",
                "status_bar",
                "AAA-111",
                "override",
                "--time",
                "9:41",
                "--batteryLevel",
                "100",
            ],
            ok_out(b""),
        );
        runner.on_exact(
            "idb",
            &["ui", "tap", "--udid", "AAA-111", "197", "426"],
            ok_out(b""),
        );

        boot_device(&runner, &tools, "AAA-111", &BootSimDeviceReq::default())
            .await
            .unwrap();
        let opened = open_url(&runner, &tools, "AAA-111", "http://localhost:5173/")
            .await
            .unwrap();
        assert!(opened.ok);
        assert!(opened.forwarded_ports.is_empty());

        let shot = screenshot(&runner, &tools, "AAA-111", ScreenshotFormat::Png)
            .await
            .unwrap();
        assert_eq!((shot.width, shot.height), (1179, 2556));

        set_status_bar(
            &runner,
            &tools,
            "AAA-111",
            &StatusBarReq {
                time: Some("9:41".into()),
                battery_level: Some(100),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let screen = ScreenMetrics {
            width: 393,
            height: 852,
            scale: Some(3.0),
            dpi: None,
        };
        send_input(
            &runner,
            &tools,
            "AAA-111",
            Some(&screen),
            Orientation::Portrait,
            &InputAction::Tap {
                x: 0.5,
                y: 0.5,
                normalized: true,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn validates_status_bar_launch_args_and_install_paths() {
        let runner = FakeCommandRunner::new();
        let probe = FakeHostProbe::new();
        let tools = ready_ios_tools(false);

        // Oversized, control-char, or leading-dash status_bar fields are rejected with 400.
        assert!(matches!(
            set_status_bar(
                &runner,
                &tools,
                "AAA-111",
                &StatusBarReq {
                    time: Some("--clear".into()),
                    battery_level: Some(80),
                    ..Default::default()
                }
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            set_status_bar(
                &runner,
                &tools,
                "AAA-111",
                &StatusBarReq {
                    time: Some("x".repeat(40)),
                    ..Default::default()
                }
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            set_status_bar(
                &runner,
                &tools,
                "AAA-111",
                &StatusBarReq {
                    operator_name: Some("bad\nline".into()),
                    ..Default::default()
                }
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));

        // launch_app rejects >64 args and NUL/control chars.
        let too_many: Vec<String> = (0..65).map(|i| format!("arg{i}")).collect();
        assert!(matches!(
            launch_app(&runner, &tools, "AAA-111", "com.example.app", &too_many).await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            launch_app(
                &runner,
                &tools,
                "AAA-111",
                "com.example.app",
                &["bad\0arg".into()]
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));

        // install_app rejects bad extension & missing path.
        assert!(matches!(
            install_app(&runner, &probe, &tools, "AAA-111", "/etc/passwd").await,
            Err(otto_core::Error::Invalid(_))
        ));
        assert!(matches!(
            install_app(&runner, &probe, &tools, "AAA-111", "/tmp/Missing.app").await,
            Err(otto_core::Error::NotFound(_))
        ));

        probe.add_path("/tmp/Demo.app");
        runner.on_exact(
            "xcrun",
            &["simctl", "install", "AAA-111", "/tmp/Demo.app"],
            ok_out(b""),
        );
        install_app(&runner, &probe, &tools, "AAA-111", "/tmp/Demo.app")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn send_input_text_rejects_control_chars_and_passes_option_terminator() {
        let runner = FakeCommandRunner::new();
        let tools = ready_ios_tools(true);

        assert!(matches!(
            send_input(
                &runner,
                &tools,
                "AAA-111",
                None,
                Orientation::Portrait,
                &InputAction::Text {
                    text: "hello\nworld".into()
                }
            )
            .await,
            Err(otto_core::Error::Invalid(_))
        ));

        runner.on_exact(
            "idb",
            &["ui", "text", "--udid", "AAA-111", "--", "--help"],
            ok_out(b""),
        );
        send_input(
            &runner,
            &tools,
            "AAA-111",
            None,
            Orientation::Portrait,
            &InputAction::Text {
                text: "--help".into(),
            },
        )
        .await
        .unwrap();
    }
}

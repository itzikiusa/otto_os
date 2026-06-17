//! Daemon supervisor: installs ottod under launchd, health-checks it, and
//! exposes status/restart commands to the SPA.

use std::path::PathBuf;
use std::time::Duration;
use std::{fs::File, io::Read};

use serde::Serialize;

const HEALTH_URL: &str = "http://127.0.0.1:7700/api/v1/health";
const META_URL: &str = "http://127.0.0.1:7700/api/v1/meta";
const LAUNCHD_LABEL: &str = "com.otto.daemon";

#[derive(Debug, Clone, Serialize)]
pub struct DaemonReport {
    pub healthy: bool,
    pub installed: bool,
    pub daemon_version: Option<String>,
    pub app_version: String,
    pub detail: String,
}

fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Application Support/Otto")
}

fn installed_bin() -> PathBuf {
    data_dir().join("bin/ottod")
}

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/LaunchAgents/com.otto.daemon.plist")
}

async fn health() -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    matches!(client.get(HEALTH_URL).send().await, Ok(r) if r.status().is_success())
}

async fn daemon_version() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let v: serde_json::Value = client.get(META_URL).send().await.ok()?.json().await.ok()?;
    v.get("version")?.as_str().map(String::from)
}

/// Locate the ottod binary bundled as a Tauri sidecar next to the app binary.
fn bundled_ottod() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("ottod");
    candidate.exists().then_some(candidate)
}

fn daemon_needs_install() -> Result<bool, String> {
    let src = bundled_ottod().ok_or("bundled ottod not found next to app binary")?;
    let dst = installed_bin();
    if !dst.exists() {
        return Ok(true);
    }
    files_differ(&src, &dst).map_err(|e| format!("compare bundled/installed ottod: {e}"))
}

fn files_differ(a: &PathBuf, b: &PathBuf) -> std::io::Result<bool> {
    let a_meta = std::fs::metadata(a)?;
    let b_meta = std::fs::metadata(b)?;
    if a_meta.len() != b_meta.len() {
        return Ok(true);
    }

    let mut a_file = File::open(a)?;
    let mut b_file = File::open(b)?;
    let mut a_buf = [0_u8; 8192];
    let mut b_buf = [0_u8; 8192];
    loop {
        let a_read = a_file.read(&mut a_buf)?;
        let b_read = b_file.read(&mut b_buf)?;
        if a_read != b_read {
            return Ok(true);
        }
        if a_read == 0 {
            return Ok(false);
        }
        if a_buf[..a_read] != b_buf[..b_read] {
            return Ok(true);
        }
    }
}

/// Copy the bundled daemon into place and (re)write + load the launchd agent.
fn install_daemon() -> Result<String, String> {
    let src = bundled_ottod().ok_or("bundled ottod not found next to app binary")?;
    let dst = installed_bin();
    std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(&src, &dst).map_err(|e| format!("copy ottod: {e}"))?;

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>EnvironmentVariables</key>
    <dict>
        <!-- File-backed secret store (0600 secrets.json). Avoids the macOS
             Keychain re-prompting "ottod wants to access com.otto.daemon"
             on every rebuild, since a re-signed binary fails the Keychain
             ACL match. Plaintext-on-disk, like loom's ~/.loom/.env. -->
        <key>OTTO_SECRETS</key><string>file</string>
    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
</dict>
</plist>
"#,
        dst.display()
    );
    let pp = plist_path();
    std::fs::create_dir_all(pp.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&pp, plist).map_err(|e| e.to_string())?;

    let uid = libc_getuid();
    // Re-bootstrap: boot out any existing instance, then bootstrap the new one.
    // `launchctl bootout` is ASYNCHRONOUS — an immediate `bootstrap` of the same
    // label races with launchd's teardown and fails ("Input/output error"),
    // leaving NO agent loaded. Retry bootstrap with a short backoff until the
    // old instance has finished unloading.
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
        .output();
    let mut last_err = String::new();
    let mut bootstrapped = false;
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(300));
        let out = std::process::Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), pp.to_str().unwrap()])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            bootstrapped = true;
            break;
        }
        last_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // The other side of the race: it's already loaded — that's fine.
        if last_err.contains("already bootstrapped") {
            bootstrapped = true;
            break;
        }
    }
    if !bootstrapped {
        return Err(format!("launchctl bootstrap failed: {last_err}"));
    }
    // RunAtLoad starts it, but kickstart guarantees it's running right now.
    let _ = std::process::Command::new("launchctl")
        .args(["kickstart", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
        .output();
    Ok("installed and started".into())
}

fn libc_getuid() -> u32 {
    // std exposes no getuid; shell out once (cheap, install path only).
    String::from_utf8_lossy(
        &std::process::Command::new("id")
            .arg("-u")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default(),
    )
    .trim()
    .parse()
    .unwrap_or(501)
}

/// Health-check; install + start when the daemon is down. Called at app start.
pub async fn ensure_daemon() -> DaemonReport {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let needs_install = daemon_needs_install();

    if matches!(needs_install, Ok(false)) && health().await {
        return DaemonReport {
            healthy: true,
            installed: installed_bin().exists(),
            daemon_version: daemon_version().await,
            app_version,
            detail: "daemon healthy".into(),
        };
    }

    if let Err(e) = needs_install {
        if health().await {
            return DaemonReport {
                healthy: true,
                installed: installed_bin().exists(),
                daemon_version: daemon_version().await,
                app_version,
                detail: format!("daemon healthy; update check failed: {e}"),
            };
        }
    }

    let detail = match install_daemon() {
        Ok(d) => d,
        Err(e) => format!("install failed: {e}"),
    };

    // Give launchd a moment, then re-check.
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if health().await {
            return DaemonReport {
                healthy: true,
                installed: true,
                daemon_version: daemon_version().await,
                app_version,
                detail,
            };
        }
    }

    DaemonReport {
        healthy: false,
        installed: installed_bin().exists(),
        daemon_version: None,
        app_version,
        detail,
    }
}

#[tauri::command]
pub async fn daemon_status() -> DaemonReport {
    DaemonReport {
        healthy: health().await,
        installed: installed_bin().exists(),
        daemon_version: daemon_version().await,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        detail: String::new(),
    }
}

#[tauri::command]
pub async fn daemon_start() -> Result<DaemonReport, String> {
    Ok(ensure_daemon().await)
}

#[tauri::command]
pub async fn daemon_restart() -> Result<String, String> {
    let uid = libc_getuid();
    let out = std::process::Command::new("launchctl")
        .args(["kickstart", "-k", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok("restarted".into())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Force reinstall (used after an app update ships a newer ottod).
#[tauri::command]
pub async fn daemon_install() -> Result<String, String> {
    install_daemon()
}

//! Launching a daemon-owned Chromium: the command line (pure, tested) and the
//! process spawn with CDP over `--remote-debugging-pipe` — the browser reads
//! commands on fd 3 and writes replies/events on fd 4, so **no TCP debugging
//! port exists** for another local process (or a web page doing DNS
//! rebinding) to reach.
//!
//! Chrome's own macOS sandbox stays ON (no `--no-sandbox`); we don't nest it
//! in `otto-sandbox`'s Seatbelt profile (a sandboxed parent breaks Chrome's
//! own `sandbox_init` of its renderers).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::conn::{CdpConn, CdpEvent};
use super::types::{ChromeBuild, Viewport};

/// How to launch one Chromium process.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub binary: PathBuf,
    pub build: ChromeBuild,
    pub headed: bool,
    pub user_data_dir: PathBuf,
    pub viewport: Viewport,
    /// Extra flags (the guard proxy's `--proxy-server` pair).
    pub extra_args: Vec<String>,
    /// Where stdout/stderr go (truncated per launch).
    pub log_path: PathBuf,
}

/// Features switched off: background services that phone home or open
/// side channels the SSRF guard can't see, plus UI a headless session never
/// needs.
const DISABLED_FEATURES: &str = "Translate,OptimizationHints,MediaRouter,DialMediaRouteProvider,\
AutofillServerCommunication,CertificateTransparencyComponentUpdater,InterestFeedContentSuggestions,\
PrivacySandboxSettings4,CalculateNativeWinOcclusion,HttpsUpgrades,ProcessPerSiteUpToMainFrameThreshold";

/// The full command line (without the binary). Pure — unit-tested.
pub fn chrome_args(spec: &LaunchSpec) -> Vec<String> {
    let v = spec.viewport.clamped();
    let mut a: Vec<String> = vec![
        "--remote-debugging-pipe".into(),
        format!("--user-data-dir={}", spec.user_data_dir.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-background-networking".into(),
        "--disable-component-update".into(),
        "--disable-sync".into(),
        "--disable-extensions".into(),
        "--disable-default-apps".into(),
        "--disable-breakpad".into(),
        "--disable-client-side-phishing-detection".into(),
        "--disable-domain-reliability".into(),
        "--disable-hang-monitor".into(),
        "--disable-ipc-flooding-protection".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--metrics-recording-only".into(),
        "--no-service-autorun".into(),
        // Never touch the login Keychain (no macOS password prompts).
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--mute-audio".into(),
        "--force-color-profile=srgb".into(),
        // WebRTC can't leak LAN/UDP around the guard proxy.
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        "--webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        format!("--disable-features={DISABLED_FEATURES}"),
        format!("--window-size={},{}", v.width, v.height),
    ];
    match spec.build {
        ChromeBuild::Chrome if !spec.headed => a.push("--headless=new".into()),
        // The headless shell is headless by construction; a headed request
        // for it is rejected by settings validation before we get here.
        _ => {}
    }
    a.extend(spec.extra_args.iter().cloned());
    a.push("about:blank".into());
    a
}

/// A spawned Chromium with its CDP pipe connected.
pub struct Launched {
    pub child: Child,
    pub conn: Arc<CdpConn>,
    pub browser_events: mpsc::UnboundedReceiver<CdpEvent>,
}

/// Spawn Chromium with CDP on fds 3 (browser reads) and 4 (browser writes).
pub fn launch(spec: &LaunchSpec) -> std::io::Result<Launched> {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    std::fs::create_dir_all(&spec.user_data_dir)?;
    restrict_dir(&spec.user_data_dir);
    if let Some(parent) = spec.log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log = std::fs::File::create(&spec.log_path)?;
    let log_err = log.try_clone()?;

    // Two socketpairs: (our_w → chrome fd 3) and (chrome fd 4 → our_r).
    let (our_w, chrome_in) = UnixStream::pair()?;
    let (our_r, chrome_out) = UnixStream::pair()?;
    let fd_in = chrome_in.as_raw_fd();
    let fd_out = chrome_out.as_raw_fd();

    let mut cmd = Command::new(&spec.binary);
    cmd.args(chrome_args(spec))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true);
    // SAFETY: only async-signal-safe libc calls (fcntl/dup2) between fork and
    // exec. The source fds are first copied above the 3/4 range (so a source
    // that already IS 3 or 4 can't be clobbered by the other dup2), then
    // dup2'd onto 3/4 — dup2's result never has FD_CLOEXEC, so exactly those
    // two survive the exec.
    unsafe {
        cmd.pre_exec(move || {
            let a = libc::fcntl(fd_in, libc::F_DUPFD_CLOEXEC, 10);
            if a < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let b = libc::fcntl(fd_out, libc::F_DUPFD_CLOEXEC, 10);
            if b < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(a, 3) < 0 || libc::dup2(b, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    // Our copies of the browser's ends must close, or EOF never arrives when
    // the browser dies.
    drop(chrome_in);
    drop(chrome_out);

    our_w.set_nonblocking(true)?;
    our_r.set_nonblocking(true)?;
    let w = tokio::net::UnixStream::from_std(our_w)?;
    let r = tokio::net::UnixStream::from_std(our_r)?;
    let (conn, browser_events) = CdpConn::start(r, w);
    Ok(Launched {
        child,
        conn,
        browser_events,
    })
}

/// Profile / download dirs hold cookies and session state: owner-only.
pub fn restrict_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(build: ChromeBuild, headed: bool) -> LaunchSpec {
        LaunchSpec {
            binary: "/x/chrome".into(),
            build,
            headed,
            user_data_dir: "/data/browser/profiles/w/u/p".into(),
            viewport: Viewport::default(),
            extra_args: vec!["--proxy-server=socks5://127.0.0.1:9".into()],
            log_path: "/tmp/x.log".into(),
        }
    }

    #[test]
    fn pipe_transport_and_no_debugging_port() {
        let a = chrome_args(&spec(ChromeBuild::Chrome, false));
        assert!(a.contains(&"--remote-debugging-pipe".to_string()));
        assert!(!a.iter().any(|x| x.starts_with("--remote-debugging-port")));
        assert!(!a
            .iter()
            .any(|x| x.starts_with("--remote-debugging-address")));
        // Chrome's own sandbox is never disabled.
        assert!(!a
            .iter()
            .any(|x| x == "--no-sandbox" || x == "--disable-web-security"));
        assert!(a.contains(&"--user-data-dir=/data/browser/profiles/w/u/p".to_string()));
        assert!(a.contains(&"--use-mock-keychain".to_string()));
        assert!(a
            .iter()
            .any(|x| x.starts_with("--proxy-server=socks5://127.0.0.1:")));
        assert_eq!(a.last().map(String::as_str), Some("about:blank"));
    }

    #[test]
    fn headless_mode_per_build() {
        let a = chrome_args(&spec(ChromeBuild::Chrome, false));
        assert!(a.contains(&"--headless=new".to_string()));
        // Headed: same binary, no headless flag → a visible window.
        let a = chrome_args(&spec(ChromeBuild::Chrome, true));
        assert!(!a.iter().any(|x| x.starts_with("--headless")));
        // The headless shell needs no flag.
        let a = chrome_args(&spec(ChromeBuild::ChromeHeadlessShell, false));
        assert!(!a.iter().any(|x| x.starts_with("--headless")));
    }

    #[test]
    fn window_size_follows_the_clamped_viewport() {
        let mut s = spec(ChromeBuild::Chrome, false);
        s.viewport = Viewport {
            width: 99_999,
            height: 700,
            device_scale_factor: 2.0,
        };
        let a = chrome_args(&s);
        assert!(a.contains(&"--window-size=3840,700".to_string()));
    }

    #[tokio::test]
    async fn launching_a_missing_binary_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = spec(ChromeBuild::Chrome, false);
        s.binary = "/definitely/not/a/chrome".into();
        s.user_data_dir = tmp.path().join("udd");
        s.log_path = tmp.path().join("chrome.log");
        assert!(launch(&s).is_err());
    }

    /// The fd plumbing, with `/bin/sh` standing in for Chromium: it echoes
    /// fd 3 to fd 4, so a CDP frame we write comes straight back.
    #[tokio::test]
    async fn fds_3_and_4_are_wired_to_the_pipe() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake-chrome.sh");
        std::fs::write(&script, "#!/bin/sh\nexec cat <&3 >&4\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut s = spec(ChromeBuild::ChromeHeadlessShell, false);
        s.binary = script;
        s.user_data_dir = tmp.path().join("udd");
        s.log_path = tmp.path().join("chrome.log");
        let mut launched = launch(&s).unwrap();
        // Write a "reply-shaped" frame: the echo makes the conn resolve it.
        let conn = launched.conn.clone();
        let call = tokio::spawn(async move {
            conn.call("Browser.getVersion", serde_json::json!({}), None)
                .await
        });
        // The echoed command has an `id` → it is treated as the reply (with
        // no `result`), so the call resolves to Null.
        let r = tokio::time::timeout(std::time::Duration::from_secs(10), call)
            .await
            .expect("echo within 10s")
            .unwrap();
        assert!(r.is_ok());
        let _ = launched.child.start_kill();
        let _ = launched.child.wait().await;
        launched.conn.closed().await;
        assert!(launched.browser_events.try_recv().is_err());
    }
}

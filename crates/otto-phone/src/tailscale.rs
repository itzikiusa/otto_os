//! Tailscale detection and `tailscale serve` management for Phone access.
//!
//! Everything goes through the [`TailscaleCli`] trait (fixed argv only — no
//! user input ever reaches the command line) so the logic is tested against a
//! scripted fake. Rules:
//!
//! * **Never `tailscale funnel`.** Funnel publishes to the internet; Otto only
//!   ever runs `tailscale serve` (tailnet-only). A Funnel found on Otto's own
//!   entry is reported as an error and removed by "Turn off".
//! * **Only Otto's entries.** A serve entry is Otto's when its `/` handler
//!   proxies to this daemon's loopback port. Turning on refuses a port another
//!   config already uses; turning off removes only Otto's `host:port` mounts.
//! * **HTTPS first, verified.** `--https=443` needs a Let's Encrypt cert that
//!   `tailscaled` fetches lazily; when that fails (e.g. another VPN's DNS
//!   shadows Tailscale's resolver) the HTTPS entry is removed again and Otto
//!   falls back to `--http=80`, which Tailscale's WireGuard still encrypts.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use otto_core::auth::BoxFuture;
use otto_core::phone::{PhoneAccessStatus, PhoneServing, TailnetDevice};
use serde_json::Value;

/// Where the Tailscale CLI lives on macOS (the Homebrew/standalone shim, the
/// App Store / standalone app bundle). Fixed paths — `$PATH` is not searched.
pub const CLI_CANDIDATES: &[&str] = &[
    "/usr/local/bin/tailscale",
    "/opt/homebrew/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

/// One CLI invocation's result.
#[derive(Debug, Clone, Default)]
pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The Tailscale CLI (real or scripted). `args` are always static strings
/// built by this module.
pub trait TailscaleCli: Send + Sync {
    /// The binary in use, `None` when Tailscale is not installed.
    fn path(&self) -> Option<String>;
    fn run<'a>(&'a self, args: Vec<String>) -> BoxFuture<'a, Result<CliOutput, String>>;
    /// Can `https://<name>` (or `http://`) be fetched through the serve
    /// listener at `ip:port`? Resolves `name` to `ip` directly (no DNS: the
    /// clash this exists for usually breaks resolution on the Mac too).
    fn probe<'a>(
        &'a self,
        https: bool,
        name: &'a str,
        ip: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// The real CLI.
pub struct SystemCli {
    path: Option<PathBuf>,
}

/// Overrides [`CLI_CANDIDATES`] with one exact binary: an isolated daemon
/// (E2E, the tour capture) points it at a scripted stand-in so it never reads
/// or changes the real tailnet. Set but not a file → "not installed".
pub const CLI_ENV: &str = "OTTO_TAILSCALE_CLI";

impl SystemCli {
    pub fn detect() -> Self {
        Self::detect_with(std::env::var_os(CLI_ENV).map(PathBuf::from))
    }

    fn detect_with(over: Option<PathBuf>) -> Self {
        let path = match over {
            Some(p) => Some(p).filter(|p| p.is_file()),
            None => CLI_CANDIDATES
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file()),
        };
        Self { path }
    }
}

impl TailscaleCli for SystemCli {
    fn path(&self) -> Option<String> {
        self.path.as_ref().map(|p| p.display().to_string())
    }

    fn run<'a>(&'a self, args: Vec<String>) -> BoxFuture<'a, Result<CliOutput, String>> {
        Box::pin(async move {
            let path = self.path.clone().ok_or("tailscale is not installed")?;
            let mut cmd = tokio::process::Command::new(path);
            cmd.args(&args)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
                .await
                .map_err(|_| "tailscale did not answer within 30 s".to_string())?
                .map_err(|e| format!("could not run tailscale: {e}"))?;
            Ok(CliOutput {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        })
    }

    fn probe<'a>(
        &'a self,
        https: bool,
        name: &'a str,
        ip: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let addr: std::net::IpAddr = ip.parse().map_err(|_| format!("bad address {ip}"))?;
            let scheme = if https { "https" } else { "http" };
            let client = reqwest::Client::builder()
                .resolve(name, std::net::SocketAddr::new(addr, port))
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?;
            let url = if (https && port == 443) || (!https && port == 80) {
                format!("{scheme}://{name}/api/v1/health")
            } else {
                format!("{scheme}://{name}:{port}/api/v1/health")
            };
            let resp = client.get(url).send().await.map_err(|e| {
                // reqwest hides the TLS cause in the source chain.
                let mut msg = e.to_string();
                let mut src = std::error::Error::source(&e);
                while let Some(s) = src {
                    msg = format!("{msg}: {s}");
                    src = s.source();
                }
                msg
            })?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!(
                    "Otto answered {} through the tailnet",
                    resp.status()
                ))
            }
        })
    }
}

/// The process-wide real CLI (detected once).
pub fn system() -> Arc<dyn TailscaleCli> {
    static CLI: std::sync::OnceLock<Arc<dyn TailscaleCli>> = std::sync::OnceLock::new();
    CLI.get_or_init(|| Arc::new(SystemCli::detect())).clone()
}

/// The daemon's loopback port (`$OTTO_PORT`, default 7700) — what Otto's
/// serve entry proxies to.
pub fn daemon_port() -> u16 {
    std::env::var("OTTO_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7700)
}

/// Whether a serve handler's proxy target is this daemon.
fn is_otto_proxy(target: &str, port: u16) -> bool {
    let t = target.trim().trim_end_matches('/');
    let t = t.strip_prefix("http://").unwrap_or(t);
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
        port.to_string(),
    ]
    .iter()
    .any(|want| t == want)
}

/// What `tailscale serve status --json` says about Otto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeView {
    /// Otto's own `(host, port, https)` mounts.
    pub otto: Vec<(String, u16, bool)>,
    /// Ports used by anything that is not Otto.
    pub foreign_ports: Vec<u16>,
    /// Funnel is enabled on one of Otto's mounts (must never be).
    pub otto_funnel: bool,
}

impl ServeView {
    /// The URL Otto is reached at: HTTPS preferred.
    pub fn serving(&self) -> Option<PhoneServing> {
        let pick = self
            .otto
            .iter()
            .find(|m| m.2)
            .or_else(|| self.otto.first())?;
        let (host, port, https) = pick.clone();
        let default = if https { 443 } else { 80 };
        let scheme = if https { "https" } else { "http" };
        let url = if port == default {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{port}")
        };
        Some(PhoneServing { url, https, port })
    }
}

/// Parse `tailscale serve status --json` (an empty config prints `{}` or
/// nothing at all).
pub fn parse_serve_status(json: &str, daemon_port: u16) -> ServeView {
    let v: Value = serde_json::from_str(json.trim()).unwrap_or(Value::Null);
    let mut view = ServeView::default();
    let tcp = v.get("TCP").and_then(Value::as_object);
    let https_port = |port: u16| {
        tcp.and_then(|t| t.get(&port.to_string()))
            .and_then(|p| p.get("HTTPS"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let mut otto_ports = Vec::new();
    if let Some(web) = v.get("Web").and_then(Value::as_object) {
        for (hostport, cfg) in web {
            let Some((host, port)) = hostport.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            let handlers = cfg.get("Handlers").and_then(Value::as_object);
            let only_root_is_otto = handlers.is_some_and(|h| {
                h.len() == 1
                    && h.get("/")
                        .and_then(|r| r.get("Proxy"))
                        .and_then(Value::as_str)
                        .is_some_and(|t| is_otto_proxy(t, daemon_port))
            });
            if only_root_is_otto {
                view.otto.push((host.to_string(), port, https_port(port)));
                otto_ports.push(port);
                let funnel = v
                    .get("AllowFunnel")
                    .and_then(|f| f.get(hostport))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                view.otto_funnel |= funnel;
            } else {
                view.foreign_ports.push(port);
            }
        }
    }
    if let Some(tcp) = tcp {
        for (port, cfg) in tcp {
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            let web = cfg.get("HTTPS").is_some() || cfg.get("HTTP").is_some();
            if !(web && otto_ports.contains(&port)) && !view.foreign_ports.contains(&port) {
                view.foreign_ports.push(port);
            }
        }
    }
    view.foreign_ports.sort_unstable();
    view.foreign_ports.dedup();
    view
}

/// Fill the tailnet half of the status from `tailscale status --json`.
pub fn apply_status_json(json: &str, out: &mut PhoneAccessStatus) {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        out.error = Some("could not read `tailscale status`".into());
        return;
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let ips = |v: &Value| -> Vec<String> {
        v.get("TailscaleIPs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let dns = |v: &Value| s(v, "DNSName").map(|d| d.trim_end_matches('.').to_string());
    out.backend_state = s(&v, "BackendState");
    out.logged_in = out.backend_state.as_deref() == Some("Running");
    if let Some(t) = v.get("CurrentTailnet") {
        out.tailnet = s(t, "Name");
        out.magic_dns = t
            .get("MagicDNSEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    if let Some(me) = v.get("Self") {
        out.self_name = dns(me).filter(|d| !d.is_empty());
        out.self_ips = ips(me);
    }
    if let Some(peers) = v.get("Peer").and_then(Value::as_object) {
        let mut devices: Vec<TailnetDevice> = peers
            .values()
            .map(|p| TailnetDevice {
                name: dns(p)
                    .and_then(|d| d.split('.').next().map(str::to_string))
                    .or_else(|| s(p, "HostName"))
                    .unwrap_or_default(),
                dns_name: dns(p).unwrap_or_default(),
                os: s(p, "OS").unwrap_or_default(),
                online: p.get("Online").and_then(Value::as_bool).unwrap_or(false),
                ips: ips(p),
            })
            .collect();
        devices.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
        out.devices = devices;
    }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Detect Tailscale + Otto's serve state. Never fails: problems land in
/// `error` (the page renders them).
pub async fn status(cli: &dyn TailscaleCli) -> PhoneAccessStatus {
    let mut out = PhoneAccessStatus {
        cli_path: cli.path(),
        installed: cli.path().is_some(),
        ..Default::default()
    };
    if !out.installed {
        return out;
    }
    match cli.run(args(&["status", "--json"])).await {
        Ok(o) if !o.stdout.trim().is_empty() => apply_status_json(&o.stdout, &mut out),
        Ok(o) => out.error = Some(first_line(&o.stderr, "tailscale is not running")),
        Err(e) => out.error = Some(e),
    }
    if !out.logged_in {
        return out;
    }
    match cli.run(args(&["serve", "status", "--json"])).await {
        Ok(o) => {
            let view = parse_serve_status(&o.stdout, daemon_port());
            out.serving = view.serving();
            out.other_serves = !view.foreign_ports.is_empty();
            if view.otto_funnel {
                out.error = Some(
                    "Tailscale Funnel is ON for Otto — it is reachable from the public internet. \
                     Turn Phone access off (removes it) and on again."
                        .into(),
                );
            }
            if out.serving.as_ref().is_some_and(|s| !s.https) {
                out.note = Some(HTTP_NOTE.into());
            }
        }
        Err(e) => out.error = Some(e),
    }
    out
}

/// Why an HTTP serve has no padlock (shown on the page).
pub const HTTP_NOTE: &str = "Served over plain HTTP inside your tailnet. Traffic is still \
    end-to-end encrypted by Tailscale (WireGuard) and only your tailnet devices can reach it, \
    but the browser shows no padlock and passkeys / WebAuthn need HTTPS. HTTPS failed because \
    this Mac could not get a certificate for its ts.net name — usually another VPN's DNS \
    shadows Tailscale's resolver (see docs/features/phone-access.md → Troubleshooting).";

fn first_line(s: &str, fallback: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(300).collect())
        .unwrap_or_else(|| fallback.to_string())
}

async fn serve_off(cli: &dyn TailscaleCli, port: u16, https: bool) -> Result<(), String> {
    let flag = if https {
        format!("--https={port}")
    } else {
        format!("--http={port}")
    };
    let o = cli
        .run(vec!["serve".into(), "--yes".into(), flag, "off".into()])
        .await?;
    if o.success {
        Ok(())
    } else {
        Err(first_line(&o.stderr, "tailscale serve off failed"))
    }
}

async fn serve_on(cli: &dyn TailscaleCli, port: u16, https: bool) -> Result<(), String> {
    let flag = if https {
        format!("--https={port}")
    } else {
        format!("--http={port}")
    };
    let target = format!("http://127.0.0.1:{}", daemon_port());
    let o = cli
        .run(vec![
            "serve".into(),
            "--bg".into(),
            "--yes".into(),
            flag,
            target,
        ])
        .await?;
    if o.success {
        Ok(())
    } else {
        Err(first_line(&o.stderr, "tailscale serve failed"))
    }
}

/// Turn Phone access on: HTTPS 443 (verified end to end), else HTTP 80.
/// Returns the fresh status (with a `note` explaining an HTTP fallback).
pub async fn enable(cli: &dyn TailscaleCli) -> Result<PhoneAccessStatus, String> {
    let st = status(cli).await;
    if !st.installed {
        return Err("Tailscale is not installed on this Mac".into());
    }
    if !st.logged_in {
        return Err("Tailscale is not signed in on this Mac — open the Tailscale app".into());
    }
    let name = st
        .self_name
        .clone()
        .ok_or("this Mac has no MagicDNS name")?;
    let ip = st
        .self_ips
        .iter()
        .find(|i| !i.contains(':'))
        .or(st.self_ips.first())
        .cloned()
        .ok_or("this Mac has no tailnet address")?;
    let o = cli.run(args(&["serve", "status", "--json"])).await?;
    let view = parse_serve_status(&o.stdout, daemon_port());
    if view.otto_funnel {
        // Remove the public exposure before anything else.
        disable(cli).await?;
    } else if view.serving().is_some_and(|s| s.https) {
        return Ok(status(cli).await);
    }
    let https_err = if view.foreign_ports.contains(&443) {
        "port 443 is already served by another config".to_string()
    } else {
        match serve_on(cli, 443, true).await {
            Ok(()) => match cli.probe(true, &name, &ip, 443).await {
                Ok(()) => {
                    // HTTPS works: drop a leftover plain-HTTP entry of Otto's.
                    if view.otto.iter().any(|m| m.1 == 80 && !m.2) {
                        let _ = serve_off(cli, 80, false).await;
                    }
                    return Ok(status(cli).await);
                }
                Err(e) => {
                    let _ = serve_off(cli, 443, true).await;
                    e
                }
            },
            Err(e) => e,
        }
    };
    if view.foreign_ports.contains(&80) {
        return Err(format!(
            "HTTPS failed ({https_err}) and port 80 is already served by another config"
        ));
    }
    if !view.otto.iter().any(|m| m.1 == 80) {
        serve_on(cli, 80, false).await?;
    }
    cli.probe(false, &name, &ip, 80)
        .await
        .map_err(|e| format!("Otto is served but did not answer through the tailnet: {e}"))?;
    let mut st = status(cli).await;
    st.note = Some(format!(
        "{HTTP_NOTE} (HTTPS error: {})",
        https_err.chars().take(240).collect::<String>()
    ));
    Ok(st)
}

/// Turn Phone access off: remove ONLY Otto's own serve mounts.
pub async fn disable(cli: &dyn TailscaleCli) -> Result<PhoneAccessStatus, String> {
    let o = cli.run(args(&["serve", "status", "--json"])).await?;
    let view = parse_serve_status(&o.stdout, daemon_port());
    for (_, port, https) in &view.otto {
        serve_off(cli, *port, *https).await?;
    }
    Ok(status(cli).await)
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// A scripted CLI: answers by argv prefix and records every call.
    pub struct FakeCli {
        pub installed: bool,
        pub status_json: String,
        pub serve_json: Mutex<String>,
        pub https_probe_ok: bool,
        pub calls: Mutex<Vec<Vec<String>>>,
    }

    pub const STATUS: &str = r#"{"BackendState":"Running","CurrentTailnet":{"Name":"me@example.com","MagicDNSEnabled":true},
      "Self":{"DNSName":"mac.tail1.ts.net.","TailscaleIPs":["100.120.1.1","fd7a:115c:a1e0::1"]},
      "Peer":{"k":{"DNSName":"iphone.tail1.ts.net.","HostName":"localhost","OS":"iOS","Online":true,"TailscaleIPs":["100.111.7.125"]}}}"#;

    impl FakeCli {
        pub fn new(serve: &str) -> Self {
            Self {
                installed: true,
                status_json: STATUS.into(),
                serve_json: Mutex::new(serve.into()),
                https_probe_ok: true,
                calls: Mutex::new(Vec::new()),
            }
        }
        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl TailscaleCli for FakeCli {
        fn path(&self) -> Option<String> {
            self.installed.then(|| "/usr/local/bin/tailscale".into())
        }
        fn run<'a>(&'a self, args: Vec<String>) -> BoxFuture<'a, Result<CliOutput, String>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(args.clone());
                let a: Vec<&str> = args.iter().map(String::as_str).collect();
                let port = daemon_port();
                let stdout = match a.as_slice() {
                    ["status", "--json"] => self.status_json.clone(),
                    ["serve", "status", "--json"] => self.serve_json.lock().unwrap().clone(),
                    ["serve", "--bg", "--yes", flag, _] => {
                        let (https, p) = if let Some(p) = flag.strip_prefix("--https=") {
                            (true, p)
                        } else {
                            (false, flag.strip_prefix("--http=").unwrap())
                        };
                        let kind = if https { "HTTPS" } else { "HTTP" };
                        *self.serve_json.lock().unwrap() = format!(
                            r#"{{"TCP":{{"{p}":{{"{kind}":true}}}},"Web":{{"mac.tail1.ts.net:{p}":{{"Handlers":{{"/":{{"Proxy":"http://127.0.0.1:{port}"}}}}}}}}}}"#
                        );
                        String::new()
                    }
                    ["serve", "--yes", _, "off"] => {
                        *self.serve_json.lock().unwrap() = "{}".into();
                        String::new()
                    }
                    other => return Err(format!("unexpected argv {other:?}")),
                };
                Ok(CliOutput {
                    success: true,
                    stdout,
                    stderr: String::new(),
                })
            })
        }
        fn probe<'a>(
            &'a self,
            https: bool,
            _name: &'a str,
            _ip: &'a str,
            _port: u16,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                if https && !self.https_probe_ok {
                    Err("tls handshake: certificate unavailable".into())
                } else {
                    Ok(())
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    const LIVE_HTTP: &str = r#"{"TCP":{"80":{"HTTP":true}},"Web":{"mac.tail1.ts.net:80":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7700"}}}}}"#;

    #[test]
    fn parses_otto_http_serve() {
        let v = parse_serve_status(LIVE_HTTP, 7700);
        assert_eq!(v.otto, vec![("mac.tail1.ts.net".into(), 80, false)]);
        assert!(v.foreign_ports.is_empty());
        let s = v.serving().unwrap();
        assert_eq!(s.url, "http://mac.tail1.ts.net");
        assert!(!s.https);
        assert_eq!(parse_serve_status("", 7700), ServeView::default());
        assert_eq!(parse_serve_status("{}", 7700), ServeView::default());
    }

    #[test]
    fn foreign_configs_and_funnel_are_recognised() {
        let j = r#"{"TCP":{"443":{"HTTPS":true},"8443":{"HTTPS":true},"22":{"TCPForward":"127.0.0.1:22"}},
          "Web":{"mac.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7700"}}},
                 "mac.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}},
          "AllowFunnel":{"mac.ts.net:443":true}}"#;
        let v = parse_serve_status(j, 7700);
        assert_eq!(v.otto, vec![("mac.ts.net".into(), 443, true)]);
        assert_eq!(v.foreign_ports, vec![22, 8443]);
        assert!(v.otto_funnel);
        assert_eq!(v.serving().unwrap().url, "https://mac.ts.net");
        // A mount with extra paths is not "Otto's" (not ours to remove).
        let mixed = r#"{"Web":{"m:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7700"},"/x":{"Path":"/tmp"}}}}}"#;
        assert_eq!(parse_serve_status(mixed, 7700).foreign_ports, vec![443]);
    }

    #[test]
    fn status_json_fields() {
        let mut s = PhoneAccessStatus::default();
        apply_status_json(STATUS, &mut s);
        assert!(s.logged_in);
        assert_eq!(s.tailnet.as_deref(), Some("me@example.com"));
        assert_eq!(s.self_name.as_deref(), Some("mac.tail1.ts.net"));
        assert_eq!(s.self_ips[0], "100.120.1.1");
        assert_eq!(s.devices.len(), 1);
        assert_eq!(s.devices[0].name, "iphone");
        assert_eq!(s.devices[0].os, "iOS");
    }

    #[test]
    fn env_override_replaces_the_fixed_paths() {
        let dir = std::env::temp_dir().join(format!("otto-ts-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tailscale");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        let cli = SystemCli::detect_with(Some(fake.clone()));
        assert_eq!(cli.path(), Some(fake.display().to_string()));
        // An override that is not a file never falls back to the real CLI.
        assert_eq!(
            SystemCli::detect_with(Some(dir.join("missing"))).path(),
            None
        );
        std::fs::remove_file(&fake).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[tokio::test]
    async fn not_installed_reports_cleanly() {
        let mut cli = FakeCli::new("{}");
        cli.installed = false;
        let s = status(&cli).await;
        assert!(!s.installed);
        assert!(cli.calls().is_empty());
        assert!(enable(&cli).await.is_err());
    }

    #[tokio::test]
    async fn enable_prefers_verified_https() {
        let cli = FakeCli::new("{}");
        let s = enable(&cli).await.unwrap();
        let serving = s.serving.unwrap();
        assert!(serving.https);
        assert_eq!(serving.url, "https://mac.tail1.ts.net");
        // Never funnel, fixed argv.
        for c in cli.calls() {
            assert!(!c.iter().any(|a| a.contains("funnel")), "{c:?}");
        }
        assert!(cli.calls().contains(&vec![
            "serve".into(),
            "--bg".into(),
            "--yes".into(),
            "--https=443".into(),
            format!("http://127.0.0.1:{}", daemon_port())
        ]));
    }

    #[tokio::test]
    async fn https_failure_falls_back_to_http_with_note() {
        let mut cli = FakeCli::new("{}");
        cli.https_probe_ok = false;
        let s = enable(&cli).await.unwrap();
        let serving = s.serving.unwrap();
        assert!(!serving.https);
        assert_eq!(serving.port, 80);
        assert!(s.note.unwrap().contains("certificate unavailable"));
        // The failed HTTPS entry was removed again.
        assert!(cli.calls().contains(&vec![
            "serve".into(),
            "--yes".into(),
            "--https=443".into(),
            "off".into()
        ]));
    }

    #[tokio::test]
    async fn disable_removes_only_otto_entries() {
        let foreign = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"mac:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        let cli = FakeCli::new(foreign);
        disable(&cli).await.unwrap();
        assert!(
            !cli.calls()
                .iter()
                .any(|c| c.last().map(String::as_str) == Some("off")),
            "a foreign config must never be removed"
        );
        let cli = FakeCli::new(LIVE_HTTP);
        disable(&cli).await.unwrap();
        assert!(cli.calls().contains(&vec![
            "serve".into(),
            "--yes".into(),
            "--http=80".into(),
            "off".into()
        ]));
    }

    #[tokio::test]
    async fn enable_refuses_to_clobber_foreign_ports() {
        let both = r#"{"TCP":{"443":{"HTTPS":true},"80":{"HTTP":true}},"Web":{"m:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}},"m:80":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        let cli = FakeCli::new(both);
        assert!(enable(&cli).await.is_err());
        assert!(!cli
            .calls()
            .iter()
            .any(|c| c.get(1).map(String::as_str) == Some("--bg")));
    }
}

//! Embedded headless Kiri container runtime manager (`kiri daemon`).
//!
//! Runs Kiri strictly as a headless sidecar daemon managed by `ottod` — no
//! separate `.app` bundle, window, or tray icon. Managed child processes are
//! owned by an instance-scoped [`crate::ContainersService`] (no process-global
//! mutable registry) and torn down gracefully on daemon shutdown.
//!
//! Binary discovery order:
//! 1. `ContainersConfig::kiri_bin_override` or `OTTO_KIRI_BIN` env var
//! 2. Sibling sidecar next to the current binary (`Otto.app/Contents/MacOS/kiri`)
//! 3. `<data_dir>/bin/kiri`
//! 4. `PATH` entries
//! 5. `/opt/homebrew/bin/kiri`, `/usr/local/bin/kiri`
//!
//! macOS entitlement note: `com.apple.security.virtualization` is required on
//! the `kiri` binary itself (NOT on `ottod` or `Otto.app`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::fs;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout, Instant};

use crate::client::{classify_kiri_vm_state, DockerClient};
use crate::contexts::{normalize_socket_path, save_selection, SavedContextSelection};
use crate::install::{current_install_state, provision_kiri};
use crate::types::{InstallRuntimeReq, RuntimeStatusResp};

const START_READY_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

const ENTITLEMENT_NOTE: &str = "On macOS, the embedded `kiri` binary (not `ottod` or `Otto.app`) \
must be signed with `com.apple.security.virtualization` and accompanied by guest kernel assets \
(`Contents/Resources/guest` or `<data_dir>/runtime/guest`).";

/// Live `kiri daemon` child owned by a [`crate::ContainersService`] instance.
pub(crate) struct ManagedProcess {
    pub(crate) child: Child,
    pub(crate) pid: u32,
    pub(crate) socket_path: PathBuf,
}

/// Instance-scoped registry of managed Kiri daemons keyed by `data_dir`.
pub(crate) type ManagedRegistry = Mutex<HashMap<PathBuf, ManagedProcess>>;

/// Discovered `kiri` binary with its origin tag.
#[derive(Debug, Clone)]
pub struct LocatedKiri {
    pub path: PathBuf,
    pub source: &'static str,
}

/// Locate the headless `kiri` binary in priority order (never shells out to `which`).
pub async fn locate_kiri_binary(
    data_dir: &Path,
    bin_override: Option<&Path>,
    allow_env_and_system: bool,
) -> Option<LocatedKiri> {
    if let Some(ov) = bin_override {
        if is_file(ov).await {
            return Some(LocatedKiri {
                path: ov.to_path_buf(),
                source: "env",
            });
        }
        return None;
    }

    if allow_env_and_system {
        if let Ok(env_path) = std::env::var("OTTO_KIRI_BIN") {
            let trimmed = env_path.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                if is_file(&p).await {
                    return Some(LocatedKiri {
                        path: p,
                        source: "env",
                    });
                }
            }
        }

        // Sibling sidecar next to `ottod` (e.g. `Otto.app/Contents/MacOS/kiri`).
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let sibling = dir.join("kiri");
                if is_file(&sibling).await {
                    return Some(LocatedKiri {
                        path: sibling,
                        source: "sidecar",
                    });
                }
            }
        }
    }

    // Otto-managed binary under `<data_dir>/bin/kiri`.
    let managed = data_dir.join("bin").join("kiri");
    if is_file(&managed).await {
        return Some(LocatedKiri {
            path: managed,
            source: "data_dir",
        });
    }

    if !allow_env_and_system {
        return None;
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("kiri");
            if is_file(&candidate).await {
                return Some(LocatedKiri {
                    path: candidate,
                    source: "path",
                });
            }
        }
    }

    for extra in ["/opt/homebrew/bin/kiri", "/usr/local/bin/kiri"] {
        let candidate = PathBuf::from(extra);
        if is_file(&candidate).await {
            return Some(LocatedKiri {
                path: candidate,
                source: "homebrew",
            });
        }
    }

    None
}

/// Resolve the Unix socket path Kiri listens on (`KIRI_SOCKET` or `~/.kiri/run/docker.sock`).
pub fn resolve_kiri_socket(home: Option<&Path>, socket_override: Option<&Path>) -> PathBuf {
    if let Some(ov) = socket_override {
        return ov.to_path_buf();
    }
    if let Ok(env_sock) = std::env::var("KIRI_SOCKET") {
        if let Ok(p) = normalize_socket_path(&env_sock) {
            return p;
        }
    }
    let resolved_home = home
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    resolved_home.join(".kiri").join("run").join("docker.sock")
}

/// Locate the guest kernel/initramfs asset directory (macOS VM assets).
pub async fn locate_guest_dir(
    data_dir: &Path,
    binary_path: Option<&Path>,
    allow_env: bool,
) -> Option<PathBuf> {
    if allow_env {
        for var in ["OTTO_KIRI_GUEST_DIR", "KIRI_GUEST_DIR"] {
            if let Ok(raw) = std::env::var(var) {
                let p = PathBuf::from(raw.trim());
                if has_guest_assets(&p).await {
                    return Some(p);
                }
            }
        }
    }
    if let Some(bin) = binary_path {
        if let Some(parent) = bin.parent() {
            let res_guest = parent.join("../Resources/guest");
            if has_guest_assets(&res_guest).await {
                return Some(res_guest);
            }
        }
    }
    let data_guest = data_dir.join("runtime").join("guest");
    if has_guest_assets(&data_guest).await {
        return Some(data_guest);
    }
    None
}

async fn has_guest_assets(dir: &Path) -> bool {
    if fs::metadata(dir).await.map(|m| !m.is_dir()).unwrap_or(true) {
        return false;
    }
    let kernel_arm = is_file(&dir.join("Image")).await;
    let kernel_x86 = is_file(&dir.join("bzImage")).await;
    let initramfs = is_file(&dir.join("initramfs.cpio")).await;
    (kernel_arm || kernel_x86) && initramfs
}

async fn is_file(path: &Path) -> bool {
    fs::metadata(path)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false)
}

pub fn log_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("kiri.log")
}

/// Inspect live status of our Otto-managed process entry for `data_dir` (reaping if exited).
pub(crate) fn managed_pid_for(
    registry: &ManagedRegistry,
    data_dir: &Path,
) -> Option<(u32, PathBuf)> {
    let mut map = registry.lock().unwrap_or_else(|p| p.into_inner());
    let entry = map.get_mut(data_dir)?;
    match entry.child.try_wait() {
        Ok(Some(_)) => {
            map.remove(data_dir);
            None
        }
        Ok(None) | Err(_) => Some((entry.pid, entry.socket_path.clone())),
    }
}

/// Inspect embedded Kiri runtime status (`GET /containers/runtime`).
pub(crate) async fn runtime_status_in(
    registry: &ManagedRegistry,
    data_dir: &Path,
    home: Option<&Path>,
    bin_override: Option<&Path>,
    socket_override: Option<&Path>,
    last_error: Option<String>,
) -> RuntimeStatusResp {
    let allow_system = bin_override.is_none() && home.is_none();
    let located = locate_kiri_binary(data_dir, bin_override, allow_system).await;
    let managed = managed_pid_for(registry, data_dir);
    let socket_path = managed
        .as_ref()
        .map(|(_, s)| s.clone())
        .unwrap_or_else(|| resolve_kiri_socket(home, socket_override));

    let guest_dir = locate_guest_dir(
        data_dir,
        located.as_ref().map(|l| l.path.as_path()),
        allow_system,
    )
    .await;
    let guest_assets_present = cfg!(target_os = "linux") || guest_dir.is_some();

    let client = DockerClient::new(socket_path.clone());
    let daemon_reachable = client.ping_ok().await;

    let mut version = None;
    let mut vm_state = "offline".to_string();
    let mut running = false;

    if daemon_reachable {
        if let Some(info) = client.raw_info().await {
            version = info
                .get("KiriVersion")
                .or_else(|| info.get("ServerVersion"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let classified = classify_kiri_vm_state(&info);
            vm_state = classified.to_string();
            running = classified != "stopped";
        } else {
            vm_state = "running".into();
            running = true;
        }
    }

    let installed = located.is_some();
    let managed_by_otto = managed.is_some();
    let pid = managed.map(|(p, _)| p);
    let log_path = log_file_path(data_dir).to_string_lossy().into_owned();
    let inst = current_install_state(data_dir);
    let can_install = cfg!(any(target_os = "macos", target_os = "linux")) && !inst.installing;
    let install_hint = if installed {
        None
    } else {
        Some(format!(
            "Click \"Install & Start Kiri\" to provision the embedded headless Kiri runtime into `{}/bin/kiri` (or bundle `Otto.app/Contents/MacOS/kiri`, or set `OTTO_KIRI_BIN`).",
            data_dir.display()
        ))
    };

    RuntimeStatusResp {
        runtime: "kiri".into(),
        installed,
        binary_path: located
            .as_ref()
            .map(|l| l.path.to_string_lossy().into_owned()),
        binary_source: located.as_ref().map(|l| l.source.to_string()),
        version,
        running,
        daemon_reachable,
        vm_state,
        socket_path: socket_path.to_string_lossy().into_owned(),
        managed_by_otto,
        pid,
        can_start: installed && !running && !inst.installing,
        can_stop: managed_by_otto || (daemon_reachable && running),
        can_install,
        installing: inst.installing,
        install_progress: inst.progress,
        install_error: inst.error,
        guest_dir: guest_dir.map(|d| d.to_string_lossy().into_owned()),
        guest_assets_present,
        log_path,
        install_hint,
        entitlement_note: ENTITLEMENT_NOTE.into(),
        last_error,
    }
}

/// Provision the embedded headless Kiri binary + guest assets directly into
/// `<data_dir>/bin/kiri` and `<data_dir>/runtime/guest/` (and optionally start
/// it immediately when `req.start_after_install == true`).
/// Callers MUST hold the per-`data_dir` runtime lock.
pub(crate) async fn install_runtime_locked(
    registry: &ManagedRegistry,
    data_dir: &Path,
    home: Option<&Path>,
    bin_override: Option<&Path>,
    socket_override: Option<&Path>,
    req: &InstallRuntimeReq,
) -> Result<RuntimeStatusResp> {
    let allow_env = bin_override.is_none() && home.is_none();
    let _bin_path = provision_kiri(
        data_dir,
        req.source_dir.as_deref(),
        req.download_url.as_deref(),
        req.sha256.as_deref(),
        allow_env,
    )
    .await?;

    if req.start_after_install {
        return start_runtime_locked(registry, data_dir, home, bin_override, socket_override).await;
    }
    Ok(runtime_status_in(
        registry,
        data_dir,
        home,
        bin_override,
        socket_override,
        None,
    )
    .await)
}

/// Start the embedded headless `kiri daemon` (or wake its VM if already up).
/// Callers MUST hold the per-`data_dir` runtime lock so concurrent starts cannot
/// spawn duplicate daemons or overwrite each other's `ManagedProcess`.
pub(crate) async fn start_runtime_locked(
    registry: &ManagedRegistry,
    data_dir: &Path,
    home: Option<&Path>,
    bin_override: Option<&Path>,
    socket_override: Option<&Path>,
) -> Result<RuntimeStatusResp> {
    let allow_system = bin_override.is_none() && home.is_none();
    let socket_path = resolve_kiri_socket(home, socket_override);
    let client = DockerClient::new(socket_path.clone());

    // Re-check under the lock: if a Kiri daemon is already answering on the
    // socket, wake its VM and select it without spawning a second daemon.
    if client.ping_ok().await {
        let _ = client.post_control("/kiri/wake").await;
        save_kiri_context(data_dir, &socket_path).await?;
        return Ok(runtime_status_in(
            registry,
            data_dir,
            home,
            bin_override,
            socket_override,
            None,
        )
        .await);
    }

    // If a live managed child already exists for this data_dir (e.g. still booting),
    // wait for it rather than overwriting and orphaning it.
    let log_path = log_file_path(data_dir);
    if managed_pid_for(registry, data_dir).is_none() {
        let located = locate_kiri_binary(data_dir, bin_override, allow_system)
            .await
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "embedded `kiri` binary not found — place it in `{}/bin/kiri` or set `OTTO_KIRI_BIN`",
                    data_dir.display()
                ))
            })?;
        // An install from before the signature check may carry a signature
        // macOS refuses (killed at exec): Otto's own copy is mended first.
        if located.source == "data_dir" {
            crate::install::ensure_macos_virtualization_signature(&located.path).await?;
        }
        let (stdout, stderr) = prepare_kiri_log_fds(data_dir, &socket_path).await?;
        let child = spawn_kiri_daemon(
            &located,
            &socket_path,
            data_dir,
            home,
            allow_system,
            stdout,
            stderr,
        )
        .await?;
        let pid = child.id().unwrap_or(0);
        {
            let mut map = registry.lock().unwrap_or_else(|p| p.into_inner());
            map.insert(
                data_dir.to_path_buf(),
                ManagedProcess {
                    child,
                    pid,
                    socket_path: socket_path.clone(),
                },
            );
        }
    }

    wait_for_kiri_ready(
        registry,
        &client,
        data_dir,
        home,
        bin_override,
        socket_override,
        &socket_path,
        &log_path,
    )
    .await
}

async fn prepare_kiri_log_fds(data_dir: &Path, socket_path: &Path) -> Result<(Stdio, Stdio)> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Internal(format!("create kiri socket dir: {e}")))?;
    }
    let log_path = log_file_path(data_dir);
    if let Some(log_dir) = log_path.parent() {
        fs::create_dir_all(log_dir)
            .await
            .map_err(|e| Error::Internal(format!("create kiri log dir: {e}")))?;
    }
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
        .map_err(|e| Error::Internal(format!("open kiri log {}: {e}", log_path.display())))?;
    let std_log_out = log_file.into_std().await;
    let std_log_err = std_log_out
        .try_clone()
        .map_err(|e| Error::Internal(format!("clone kiri log fd: {e}")))?;
    Ok((Stdio::from(std_log_out), Stdio::from(std_log_err)))
}

async fn spawn_kiri_daemon(
    located: &LocatedKiri,
    socket_path: &Path,
    data_dir: &Path,
    home: Option<&Path>,
    allow_system: bool,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<Child> {
    let mut cmd = Command::new(&located.path);
    cmd.arg("daemon")
        .env("KIRI_SOCKET", socket_path)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true);

    if let Some(h) = home {
        cmd.env("HOME", h);
    }
    if let Some(guest_dir) = locate_guest_dir(data_dir, Some(&located.path), allow_system).await {
        cmd.env("KIRI_GUEST_DIR", guest_dir);
    }

    cmd.spawn().map_err(|e| {
        Error::Upstream(format!(
            "failed to spawn `{}` daemon: {e}",
            located.path.display()
        ))
    })
}

#[allow(clippy::too_many_arguments)]
async fn wait_for_kiri_ready(
    registry: &ManagedRegistry,
    client: &DockerClient,
    data_dir: &Path,
    home: Option<&Path>,
    bin_override: Option<&Path>,
    socket_override: Option<&Path>,
    socket_path: &Path,
    log_path: &Path,
) -> Result<RuntimeStatusResp> {
    let deadline = Instant::now() + START_READY_TIMEOUT;
    loop {
        if client.ping_ok().await {
            save_kiri_context(data_dir, socket_path).await?;
            return Ok(runtime_status_in(
                registry,
                data_dir,
                home,
                bin_override,
                socket_override,
                None,
            )
            .await);
        }

        if let Some(exit_status) = check_exited(registry, data_dir) {
            let tail = read_log_tail(log_path).await;
            // Killed at once: macOS refusing the binary's code signature.
            let why = match exit_status.to_string().contains("signal: 9") {
                true => " — macOS stops a program like this when its code signature does not verify; reinstall the runtime",
                false => "",
            };
            return Err(Error::Upstream(format!(
                "kiri daemon exited early ({exit_status}){why}{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(": {tail}")
                }
            )));
        }

        if Instant::now() >= deadline {
            let tail = read_log_tail(log_path).await;
            let _ =
                stop_runtime_locked(registry, data_dir, home, bin_override, socket_override).await;
            return Err(Error::Upstream(format!(
                "kiri daemon did not answer on {} within {}s{}",
                socket_path.display(),
                START_READY_TIMEOUT.as_secs(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(" (log: {tail})")
                }
            )));
        }
        sleep(POLL_INTERVAL).await;
    }
}

/// Gracefully terminate a single managed Kiri process (`SIGTERM`, then `SIGKILL`).
pub(crate) async fn terminate_managed_process(mut proc: ManagedProcess) {
    if proc.pid > 0 {
        let _ = Command::new("/bin/kill")
            .args(["-TERM", &proc.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    if timeout(STOP_GRACE_TIMEOUT, proc.child.wait())
        .await
        .is_err()
    {
        let _ = proc.child.start_kill();
        let _ = timeout(Duration::from_secs(2), proc.child.wait()).await;
    }
}

/// Gracefully stop the embedded `kiri daemon` (SIGTERM for Otto-managed
/// processes, or `POST /kiri/stop` for an external daemon on the socket).
/// Callers MUST hold the per-`data_dir` runtime lock.
pub(crate) async fn stop_runtime_locked(
    registry: &ManagedRegistry,
    data_dir: &Path,
    home: Option<&Path>,
    bin_override: Option<&Path>,
    socket_override: Option<&Path>,
) -> Result<RuntimeStatusResp> {
    let managed = {
        let mut map = registry.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(data_dir)
    };

    if let Some(proc) = managed {
        terminate_managed_process(proc).await;
        return Ok(runtime_status_in(
            registry,
            data_dir,
            home,
            bin_override,
            socket_override,
            None,
        )
        .await);
    }

    // Not Otto-managed: if a Kiri daemon is reachable on the socket, ask it to stop its VM.
    let socket_path = resolve_kiri_socket(home, socket_override);
    let client = DockerClient::new(socket_path);
    if client.ping_ok().await {
        client.post_control("/kiri/stop").await?;
    }
    Ok(runtime_status_in(
        registry,
        data_dir,
        home,
        bin_override,
        socket_override,
        None,
    )
    .await)
}

/// Gracefully stop every managed Kiri daemon owned by `registry` (called on daemon teardown).
pub(crate) async fn shutdown_all_managed(registry: &ManagedRegistry) {
    let drained: Vec<(PathBuf, ManagedProcess)> = {
        let mut map = registry.lock().unwrap_or_else(|p| p.into_inner());
        map.drain().collect()
    };
    for (_, proc) in drained {
        terminate_managed_process(proc).await;
    }
}

fn check_exited(registry: &ManagedRegistry, data_dir: &Path) -> Option<String> {
    let mut map = registry.lock().unwrap_or_else(|p| p.into_inner());
    let entry = map.get_mut(data_dir)?;
    match entry.child.try_wait() {
        Ok(Some(status)) => {
            map.remove(data_dir);
            Some(status.to_string())
        }
        _ => None,
    }
}

async fn save_kiri_context(data_dir: &Path, socket_path: &Path) -> Result<()> {
    save_selection(
        data_dir,
        &SavedContextSelection {
            name: "kiri".into(),
            socket_path: socket_path.to_string_lossy().into_owned(),
        },
    )
    .await
}

async fn read_log_tail(log_path: &Path) -> String {
    let Ok(raw) = fs::read_to_string(log_path).await else {
        return String::new();
    };
    let lines: Vec<&str> = raw.lines().rev().take(6).collect();
    lines
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
        .chars()
        .take(400)
        .collect()
}

//! Child language-server process spawning, per-language memory caps, scrubbed
//! non-root environments, persistent `jdtls` workspace caching, and RSS sampling.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::io::BufReader;

use super::pool::{PoolKey, ServerIo, SpawnLaunch, Spawner};
use super::servers;
use crate::environments::shell_env::collect_safe_non_root_base_env;
use crate::environments::EnvironmentManager;
use crate::target::ssh_exec::posix_quote;

/// Canonical `jdtls` JVM flags balancing burst headroom (768 MiB so Maven/Gradle
/// indexing never throws `OutOfMemoryError`) with periodic G1 heap uncommit so
/// idle servers settle back to ~180–250 MiB RSS.
pub(crate) const JDTLS_JVM_FLAGS: &[&str] = &[
    "-Xms128m",
    "-Xmx768m",
    "-Xss512k",
    "-XX:TieredStopAtLevel=1",
    "-XX:MaxMetaspaceSize=224m",
    "-XX:ReservedCodeCacheSize=64m",
    "-XX:+UseG1GC",
    "-XX:+UseStringDeduplication",
    "-XX:G1PeriodicGCInterval=15000",
    "-XX:-G1PeriodicGCInvokesConcurrent",
    "-XX:MinHeapFreeRatio=10",
    "-XX:MaxHeapFreeRatio=25",
    "-Dsun.zip.disableMemoryMapping=true",
];

pub(crate) fn jdtls_workspace_data_dir(root: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(root.trim().trim_end_matches('/').as_bytes());
    let hex = hex::encode(hasher.finalize());
    crate::go_build_cache::ide_build_cache_root()
        .join("jdtls")
        .join(&hex[..16])
}

/// Build the remote POSIX shell script that enters `root`, augments `PATH`
/// with `~/.otto/toolchains/**/bin` + standard SDK dirs, applies memory caps,
/// and `exec`s the language server.
pub fn build_remote_lsp_script(root: &str, launch: &SpawnLaunch) -> String {
    let quoted_root = posix_quote(root);
    let invoke_primary = std::iter::once(posix_quote(&launch.cmd))
        .chain(launch.args.iter().map(|a| posix_quote(a)))
        .collect::<Vec<_>>()
        .join(" ");

    let path_setup = concat!(
        "for d in ",
        "\"$HOME\"/.otto/toolchains/bin ",
        "\"$HOME\"/.otto/toolchains/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/current/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/Contents/Home/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/node_modules/.bin ",
        "\"$HOME\"/.otto/toolchains/*/*/lib/node_modules/.bin ",
        "\"$HOME/go/bin\" \"$HOME/.cargo/bin\" \"$HOME/.local/bin\"; ",
        "do [ -d \"$d\" ] && PATH=\"$d:$PATH\"; done; export PATH"
    );

    let mem_exports = concat!(
        "export GOMEMLIMIT=\"${GOMEMLIMIT:-512MiB}\" GOGC=\"${GOGC:-75}\" ",
        "RA_LRU_CAP=\"${RA_LRU_CAP:-128}\"; ",
        "case \"${NODE_OPTIONS:-}\" in *--max-old-space-size*) ;; ",
        "\"\") export NODE_OPTIONS='--max-old-space-size=384' ;; ",
        "*) export NODE_OPTIONS=\"$NODE_OPTIONS --max-old-space-size=384\" ;; esac"
    );

    let inner = if let Some(fb) = &launch.fallback_cmd {
        let invoke_fb = std::iter::once(posix_quote(fb))
            .chain(launch.fallback_args.iter().map(|a| posix_quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let p_check = posix_quote(&launch.cmd);
        let fb_check = posix_quote(fb);
        format!(
            "cd {quoted_root} && {path_setup}; {mem_exports}; \
             if command -v {p_check} >/dev/null 2>&1; then exec {invoke_primary}; \
             elif command -v {fb_check} >/dev/null 2>&1; then exec {invoke_fb}; \
             else echo 'otto-lsp: language server not found on remote host' >&2; exit 127; fi"
        )
    } else {
        format!("cd {quoted_root} && {path_setup}; {mem_exports}; exec {invoke_primary}")
    };
    crate::target::ssh_exec::wrap_remote_stdio_with_eof_kill(&inner)
}

/// The newest `lombok-<v>.jar` in the local Maven repository, if any.
#[allow(clippy::disallowed_methods)] // one small directory listing at server spawn
fn newest_lombok_jar(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let dir = crate::java_workspace::resolve_m2_repo(env)?.join("org/projectlombok/lombok");
    let key = |v: &str| -> Vec<u64> {
        v.split(['.', '-'])
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|v| !v.contains("SNAPSHOT"))
        .map(|v| dir.join(&v).join(format!("lombok-{v}.jar")))
        .filter(|jar| jar.is_file())
        .max_by_key(|jar| {
            key(jar
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(""))
        })
}

pub(crate) fn apply_language_memory_caps(
    command: &mut tokio::process::Command,
    lang: &str,
    root: &str,
    cmd_name: &str,
    existing_args: &[String],
    inherited: &BTreeMap<String, String>,
) {
    let norm = servers::normalize_lsp_lang(lang);
    match norm {
        "rust" => {
            if !inherited.contains_key("RA_LRU_CAP") {
                command.env("RA_LRU_CAP", "128");
            }
        }
        "go" => {
            if !inherited.contains_key("GOGC") {
                command.env("GOGC", "75");
            }
            if !inherited.contains_key("GOMEMLIMIT") {
                command.env("GOMEMLIMIT", "512MiB");
            }
        }
        "java" => {
            for flag in JDTLS_JVM_FLAGS {
                command.arg(format!("--jvm-arg={flag}"));
            }
            // Without the agent jdtls sees no Lombok-generated members (getters,
            // builders, `log`) and flags every use as an error.
            if let Some(jar) = newest_lombok_jar(inherited) {
                command.arg(format!("--jvm-arg=-javaagent:{}", jar.display()));
            }
            if !existing_args.iter().any(|a| a == "-data") {
                let data_dir = jdtls_workspace_data_dir(root);
                #[allow(clippy::disallowed_methods)]
                let _ = std::fs::create_dir_all(&data_dir);
                command.arg("-data").arg(data_dir);
            }
        }
        "typescript" | "javascript" | "shell" | "css" | "html" => {
            set_node_heap_cap(command, inherited);
        }
        "python" if cmd_name.contains("pyright") => {
            set_node_heap_cap(command, inherited);
        }
        _ => {}
    }
}

fn set_node_heap_cap(command: &mut tokio::process::Command, inherited: &BTreeMap<String, String>) {
    let existing = inherited.get("NODE_OPTIONS").cloned().unwrap_or_default();
    if !existing.contains("--max-old-space-size") {
        let merged = if existing.trim().is_empty() {
            "--max-old-space-size=384".to_string()
        } else {
            format!("{} --max-old-space-size=384", existing.trim())
        };
        command.env("NODE_OPTIONS", merged);
    }
}

fn is_daemon_internal_secret_key(k: &str) -> bool {
    let upper = k.to_ascii_uppercase();
    upper.starts_with("OTTO_MCP_")
        || upper == "OTTO_API_TOKEN"
        || upper == "OTTO_ADMIN_TOKEN"
        || upper == "OTTO_SECRET_KEY"
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod unix_sig {
    extern "C" {
        pub fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    pub const SIGKILL: std::ffi::c_int = 9;
    pub const SIGTERM: std::ffi::c_int = 15;
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub(crate) fn term_process_group_sync(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    if let Ok(p) = std::ffi::c_int::try_from(pgid) {
        // SAFETY: Sending SIGTERM to a negative process group ID (-p < -1) is OS-safe.
        unsafe {
            let _ = unix_sig::kill(-p, unix_sig::SIGTERM);
        }
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub(crate) fn kill_process_group_sync(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    if let Ok(p) = std::ffi::c_int::try_from(pgid) {
        // SAFETY: Sending SIGKILL to a negative process group ID (-p < -1) is OS-safe.
        unsafe {
            let _ = unix_sig::kill(-p, unix_sig::SIGKILL);
        }
    }
}

/// Immediate process-group force teardown (no redundant back-to-back SIGTERM
/// immediately before SIGKILL).
#[cfg(unix)]
pub(crate) fn terminate_process_group(pgid: u32) {
    kill_process_group_sync(pgid);
}

pub(crate) fn build_process_spawner(
    data_dir: PathBuf,
    environments: Option<EnvironmentManager>,
    owner_privileges: Arc<DashMap<String, bool>>,
) -> Spawner {
    Arc::new(move |key: &PoolKey, launch: &SpawnLaunch| {
        let owner_is_root = owner_privileges
            .get(&key.owner)
            .map(|r| *r)
            .unwrap_or(false);

        let mut command = if let Some(ssh) = &key.ssh {
            let (ssh_args, dest) =
                crate::target::ssh_exec::build_ssh_cli_args(ssh, None).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
                })?;
            let remote_script = build_remote_lsp_script(&key.root, launch);
            let mut c = tokio::process::Command::new("ssh");
            c.env_clear();
            c.envs(collect_safe_non_root_base_env());
            if owner_is_root {
                if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
                    c.env("SSH_AUTH_SOCK", sock);
                }
            }
            c.args(&ssh_args).arg("--").arg(dest).arg(remote_script);
            #[cfg(unix)]
            c.process_group(0);
            c
        } else {
            let mut effective_env = match &environments {
                Some(envs) => envs.cached_probe_env_for_local_root_as(&key.root, owner_is_root),
                None => collect_safe_non_root_base_env(),
            };
            effective_env.retain(|k, _| {
                !is_daemon_internal_secret_key(k)
                    && !crate::environments::is_dangerous_loader_env_key(k)
            });
            if owner_is_root {
                for go_key in ["GOPRIVATE", "GONOSUMDB", "GOPATH", "GOMODCACHE"] {
                    if !effective_env.contains_key(go_key) {
                        if let Ok(val) = std::env::var(go_key) {
                            if !val.is_empty() {
                                effective_env.insert(go_key.to_string(), val);
                            }
                        }
                    }
                }
                if !effective_env.contains_key("SSH_AUTH_SOCK") {
                    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
                        if !sock.is_empty() {
                            effective_env.insert("SSH_AUTH_SOCK".to_string(), sock);
                        }
                    }
                }
            }

            let shell_snap = if owner_is_root {
                environments.as_ref().map(|e| e.cached_shell_env_snapshot())
            } else {
                None
            };
            let aug_path = servers::augmented_local_path_for_user(
                &data_dir,
                shell_snap.as_ref(),
                owner_is_root,
            );
            let root_path = std::path::Path::new(&key.root);
            #[allow(clippy::disallowed_methods)]
            let canon_ws = std::fs::canonicalize(root_path).ok();
            let mut safe_path_parts: Vec<String> = Vec::with_capacity(32);
            let combined_raw = match effective_env.get("PATH") {
                Some(ep) if !ep.is_empty() => format!("{aug_path}:{ep}"),
                _ => aug_path,
            };
            for seg in combined_raw.split(':').filter(|s| !s.is_empty()) {
                let sp = std::path::Path::new(seg);
                if !sp.is_absolute() || sp.starts_with(root_path) {
                    continue;
                }
                if let Some(ref cw) = canon_ws {
                    if sp.starts_with(cw) {
                        continue;
                    }
                    #[allow(clippy::disallowed_methods)]
                    if std::fs::canonicalize(sp).is_ok_and(|cs| cs.starts_with(cw)) {
                        continue;
                    }
                }
                if !safe_path_parts.iter().any(|existing| existing == seg) {
                    safe_path_parts.push(seg.to_string());
                }
            }
            let merged_path = safe_path_parts.join(":");

            let mut c = tokio::process::Command::new(&launch.cmd);
            c.env_clear();
            c.envs(collect_safe_non_root_base_env());
            c.args(&launch.args)
                .current_dir(&key.root)
                .envs(&effective_env)
                .env("PATH", merged_path);

            if !effective_env.contains_key("JAVA_HOME") {
                if let Some(java_home) = servers::detect_toolchain_java_home(&data_dir) {
                    c.env("JAVA_HOME", java_home);
                }
            }
            apply_language_memory_caps(
                &mut c,
                &key.lang,
                &key.root,
                &launch.cmd,
                &launch.args,
                &effective_env,
            );
            #[cfg(unix)]
            c.process_group(0);
            c
        };

        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let lang = key.lang.clone();
        // Drained to EOF whatever the bytes: a server that logs a non-UTF-8 path
        // must not lose its stderr (and die of SIGPIPE) under every socket.
        crate::line_reader::spawn_line_drain(
            stderr,
            crate::line_reader::MAX_OUTPUT_LINE_BYTES,
            move |line| tracing::debug!(lang = lang.as_str(), "lsp stderr: {line}"),
        );
        Ok(ServerIo {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
        })
    })
}

struct RssCache {
    sampled_at: Instant,
    by_pgid: HashMap<u32, u64>,
}

fn rss_cache() -> &'static Mutex<Option<RssCache>> {
    static CACHE: OnceLock<Mutex<Option<RssCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Sample resident set size (in bytes) for `pids` (summing each server's Unix
/// process group so wrapper scripts like `jdtls` -> `java` report full JVM RSS),
/// cached for 5 seconds so status checks and periodic sweeps never hammer `ps`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn sample_pids_rss_bytes(pids: &[u32]) -> HashMap<u32, u64> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let now = Instant::now();
    if let Ok(guard) = rss_cache().lock() {
        if let Some(cached) = guard.as_ref() {
            if now.duration_since(cached.sampled_at) < Duration::from_secs(5)
                && pids.iter().all(|p| cached.by_pgid.contains_key(p))
            {
                return pids
                    .iter()
                    .map(|p| (*p, cached.by_pgid.get(p).copied().unwrap_or(0)))
                    .collect();
            }
        }
    }

    let mut by_pgid: HashMap<u32, u64> = pids.iter().map(|&p| (p, 0u64)).collect();
    #[cfg(unix)]
    {
        if let Ok(out) = std::process::Command::new("ps")
            .args(["-ax", "-o", "pid=,pgid=,rss="])
            .stdin(std::process::Stdio::null())
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    let mut parts = line.split_whitespace();
                    let Some(pid) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
                        continue;
                    };
                    let Some(pgid) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
                        continue;
                    };
                    let Some(rss_kib) = parts.next().and_then(|s| s.parse::<u64>().ok()) else {
                        continue;
                    };
                    let bytes = rss_kib.saturating_mul(1024);
                    if let Some(slot) = by_pgid.get_mut(&pgid) {
                        *slot = slot.saturating_add(bytes);
                    } else if pid != pgid {
                        if let Some(slot) = by_pgid.get_mut(&pid) {
                            *slot = slot.saturating_add(bytes);
                        }
                    }
                }
            }
        }
    }

    if let Ok(mut guard) = rss_cache().lock() {
        *guard = Some(RssCache {
            sampled_at: now,
            by_pgid: by_pgid.clone(),
        });
    }
    by_pgid
}

#[derive(Clone)]
struct FdCache {
    sampled_at: Instant,
    by_pid: HashMap<u32, u32>,
}

fn fd_cache() -> &'static Mutex<Option<FdCache>> {
    static CACHE: OnceLock<Mutex<Option<FdCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn sample_single_pid_fd_count(pid: u32) -> Option<u32> {
    const PROC_PIDLISTFDS: std::ffi::c_int = 1;
    // sizeof(struct proc_fdinfo) on macOS is 8 bytes (i32 proc_fd + u32 proc_fdtype).
    const PROC_FDINFO_SIZE: std::ffi::c_int = 8;
    extern "C" {
        fn proc_pidinfo(
            pid: std::ffi::c_int,
            flavor: std::ffi::c_int,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            buffersize: std::ffi::c_int,
        ) -> std::ffi::c_int;
    }
    let pid_i32 = std::ffi::c_int::try_from(pid).ok()?;
    if pid_i32 <= 0 {
        return None;
    }
    // SAFETY: Passing null buffer with buffersize=0 queries the required byte
    // size without writing to memory; returns <= 0 when the process exited or
    // is inaccessible.
    let bytes = unsafe { proc_pidinfo(pid_i32, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if bytes <= 0 {
        return None;
    }
    u32::try_from(bytes / PROC_FDINFO_SIZE).ok()
}

#[cfg(target_os = "linux")]
#[allow(clippy::disallowed_methods)]
fn sample_single_pid_fd_count(pid: u32) -> Option<u32> {
    if pid == 0 {
        return None;
    }
    let count = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.count();
    u32::try_from(count).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn sample_single_pid_fd_count(_pid: u32) -> Option<u32> {
    None
}

/// Sample open file-descriptor counts for `pids`, cached for 5 seconds and
/// silently omitting exited or inaccessible processes.
pub(crate) fn sample_pids_fd_counts(pids: &[u32]) -> HashMap<u32, u32> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let now = Instant::now();
    if let Ok(guard) = fd_cache().lock() {
        if let Some(cached) = guard.as_ref() {
            if now.duration_since(cached.sampled_at) < Duration::from_secs(5)
                && pids.iter().all(|p| cached.by_pid.contains_key(p))
            {
                return pids
                    .iter()
                    .filter_map(|p| cached.by_pid.get(p).copied().map(|n| (*p, n)))
                    .collect();
            }
        }
    }
    let mut by_pid = HashMap::with_capacity(pids.len());
    for &pid in pids {
        if let Some(n) = sample_single_pid_fd_count(pid) {
            by_pid.insert(pid, n);
        }
    }
    if let Ok(mut guard) = fd_cache().lock() {
        *guard = Some(FdCache {
            sampled_at: now,
            by_pid: by_pid.clone(),
        });
    }
    by_pid
}

/// Combined live RSS + FD sample for a single OS process (or process group).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProcessResourceSample {
    pub rss_bytes: Option<u64>,
    pub fd_count: Option<u32>,
}

/// Sample RSS and open FD counts off the Tokio runtime via `spawn_blocking`.
pub(crate) async fn sample_pids_metrics_async(
    pids: Vec<u32>,
) -> HashMap<u32, ProcessResourceSample> {
    if pids.is_empty() {
        return HashMap::new();
    }
    tokio::task::spawn_blocking(move || {
        let rss = sample_pids_rss_bytes(&pids);
        let fds = sample_pids_fd_counts(&pids);
        pids.into_iter()
            .map(|p| {
                let rss_bytes = rss.get(&p).copied().filter(|&b| b > 0);
                let fd_count = fds.get(&p).copied();
                (
                    p,
                    ProcessResourceSample {
                        rss_bytes,
                        fd_count,
                    },
                )
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(test)]
mod resource_metric_tests {
    use super::*;

    #[tokio::test]
    async fn sample_pids_metrics_reports_current_process_and_ignores_exited_pid() {
        let self_pid = std::process::id();
        let bogus_pid = u32::MAX / 2;
        let samples = sample_pids_metrics_async(vec![self_pid, bogus_pid]).await;
        let mine = samples.get(&self_pid).copied().unwrap_or_default();
        assert!(
            mine.rss_bytes.unwrap_or(0) > 0,
            "current process should have non-zero RSS: {mine:?}"
        );
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(
            mine.fd_count.unwrap_or(0) >= 3,
            "current process should have >=3 open FDs: {mine:?}"
        );
        let missing = samples.get(&bogus_pid).copied().unwrap_or_default();
        assert_eq!(missing.rss_bytes, None);
        assert_eq!(missing.fd_count, None);
    }
}

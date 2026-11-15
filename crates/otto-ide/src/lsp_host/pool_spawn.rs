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

/// POSIX single-quote escaping (`'abc'` -> `'abc'`, `a'b` -> `'a'\''b'`).
pub fn posix_quote(s: &str) -> String {
    crate::target::ssh_exec::posix_quote(s)
}

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

    if let Some(fb) = &launch.fallback_cmd {
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
    }
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
pub(crate) fn kill_process_group_sync(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    let neg = format!("-{pgid}");
    #[allow(clippy::disallowed_methods)]
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", "--", &neg])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(unix)]
pub(crate) fn terminate_process_group(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    std::thread::spawn(move || {
        let neg = format!("-{pgid}");
        #[allow(clippy::disallowed_methods)]
        let _ = std::process::Command::new("/bin/kill")
            .args(["-TERM", "--", &neg])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        #[allow(clippy::disallowed_methods)]
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", "--", &neg])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
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
                Some(envs) => envs.cached_effective_env_for_local_root_as(&key.root, owner_is_root),
                None => collect_safe_non_root_base_env(),
            };
            effective_env.retain(|k, _| !is_daemon_internal_secret_key(k));

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
            let merged_path = match effective_env.get("PATH") {
                Some(ep) if !ep.is_empty() => format!("{ep}:{aug_path}"),
                _ => aug_path,
            };

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
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(lang = lang.as_str(), "lsp stderr: {line}");
            }
        });
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
/// cached for 1.5 seconds to keep status and attach calls sub-millisecond.
#[allow(clippy::disallowed_methods)]
pub(crate) fn sample_pids_rss_bytes(pids: &[u32]) -> HashMap<u32, u64> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let now = Instant::now();
    if let Ok(guard) = rss_cache().lock() {
        if let Some(cached) = guard.as_ref() {
            if now.duration_since(cached.sampled_at) < Duration::from_millis(600)
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

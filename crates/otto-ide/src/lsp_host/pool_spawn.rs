//! Child language-server process spawning, per-language memory caps, scrubbed
//! non-root environments, persistent `jdtls` workspace caching, and RSS sampling.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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
        "\"$HOME\"/.otto/toolchains/*/current ",
        "\"$HOME\"/.otto/toolchains/dotnet/tools ",
        "\"$HOME\"/.otto/toolchains/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/Contents/Home/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/node_modules/.bin ",
        "\"$HOME\"/.otto/toolchains/*/*/lib/node_modules/.bin ",
        "\"$HOME/go/bin\" \"$HOME/.cargo/bin\" \"$HOME/.zvm/bin\" \"$HOME/.dotnet\" \"$HOME/.dotnet/tools\" \"$HOME/.local/bin\"; ",
        "do [ -d \"$d\" ] && PATH=\"$d:$PATH\"; done; export PATH; ",
        "if [ -z \"${DOTNET_ROOT:-}\" ] && [ -x \"$HOME/.otto/toolchains/dotnet/current/dotnet\" ]; then ",
        "export DOTNET_ROOT=\"$HOME/.otto/toolchains/dotnet/current\"; fi"
    );

    let mem_exports = concat!(
        "export GOMEMLIMIT=\"${GOMEMLIMIT:-512MiB}\" GOGC=\"${GOGC:-75}\" ",
        "RA_LRU_CAP=\"${RA_LRU_CAP:-128}\"; ",
        "case \"${NODE_OPTIONS:-}\" in *--max-old-space-size*) ;; ",
        "\"\") export NODE_OPTIONS='--max-old-space-size=384' ;; ",
        "*) export NODE_OPTIONS=\"$NODE_OPTIONS --max-old-space-size=384\" ;; esac"
    );

    let is_ngserver = Path::new(&launch.cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "ngserver")
        && !launch
            .args
            .iter()
            .any(|a| a == "--tsProbeLocations" || a.starts_with("--tsProbeLocations="));

    // SEC2-1 + BUG-32: Probe ONLY trusted managed and global-installation node_modules
    // (including package-private `<pkg>/node_modules` and outer global `node_modules`
    // resolved through the real ngserver symlink target), never the checkout's $PWD/node_modules.
    let ng_probe_setup = if is_ngserver {
        "NG_PROBES=\"\"; \
         [ -d \"$HOME/.otto/toolchains/node/lsp/node_modules\" ] && NG_PROBES=\"$HOME/.otto/toolchains/node/lsp/node_modules\"; \
         NG_BIN=$(command -v ngserver 2>/dev/null || true); \
         if [ -n \"$NG_BIN\" ]; then \
           NG_REAL=$(readlink -f \"$NG_BIN\" 2>/dev/null || realpath \"$NG_BIN\" 2>/dev/null || printf '%s' \"$NG_BIN\"); \
           NG_CUR=$(dirname \"$NG_REAL\"); \
           while [ \"$NG_CUR\" != \"/\" ] && [ -n \"$NG_CUR\" ]; do \
             if [ \"$(basename \"$NG_CUR\")\" = \"node_modules\" ]; then \
               case \",$NG_PROBES,\" in *\",$NG_CUR,\"*) ;; *) NG_PROBES=\"${NG_PROBES:+$NG_PROBES,}$NG_CUR\" ;; esac; \
               break; \
             fi; \
             if [ -d \"$NG_CUR/node_modules\" ] && { [ -d \"$NG_CUR/node_modules/typescript\" ] || [ -d \"$NG_CUR/node_modules/@angular/language-service\" ]; }; then \
               NG_NM=\"$NG_CUR/node_modules\"; \
               case \",$NG_PROBES,\" in *\",$NG_NM,\"*) ;; *) NG_PROBES=\"${NG_PROBES:+$NG_PROBES,}$NG_NM\" ;; esac; \
             fi; \
             NG_CUR=$(dirname \"$NG_CUR\"); \
           done; \
         fi; "
    } else {
        ""
    };
    let is_zls = Path::new(&launch.cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "zls");

    // BUG-4 + BUG-6 (SSH): When launching ZLS remotely, prepend the project's pinned
    // `zig_home` onto PATH and select the per-minor managed ZLS slot
    // (`$HOME/.otto/toolchains/zig/zls/<maj>.<min>/zls`) matching the selected Zig
    // compiler so installing ZLS for another project's Zig minor cannot clobber this one.
    let zls_slot_setup = if is_zls {
        let mut zig_probes = String::new();
        let mut zig_path_prefix = String::new();
        if let Some(zh) = launch
            .zig_home
            .as_deref()
            .map(str::trim)
            .map(|s| s.trim_end_matches('/'))
            .filter(|s| !s.is_empty())
        {
            let q_dir = posix_quote(zh);
            let q_bin_dir = posix_quote(&format!("{zh}/bin"));
            let q_zig = posix_quote(&format!("{zh}/zig"));
            let q_bin_zig = posix_quote(&format!("{zh}/bin/zig"));
            zig_path_prefix = format!(
                "[ -d {q_bin_dir} ] && PATH={q_bin_dir}:\"$PATH\"; [ -d {q_dir} ] && PATH={q_dir}:\"$PATH\"; export PATH; "
            );
            zig_probes.push_str(&format!("{q_zig} {q_bin_zig} "));
        }
        format!(
            "{zig_path_prefix}ZLS_BIN=\"\"; ZIG_MM=\"\"; \
             for __zb in {zig_probes}\"$HOME/.otto/toolchains/zig/current/zig\" \"$HOME/.otto/toolchains/zig/current/bin/zig\" zig; do \
               if [ -x \"$__zb\" ] || command -v \"$__zb\" >/dev/null 2>&1; then \
                 ZIG_MM=$(\"$__zb\" version 2>/dev/null | head -n 1 | sed -n 's/^\\([0-9][0-9]*\\.[0-9][0-9]*\\).*/\\1/p'); \
                 [ -n \"$ZIG_MM\" ] && break; \
               fi; \
             done; \
             if [ -n \"$ZIG_MM\" ] && [ -x \"$HOME/.otto/toolchains/zig/zls/$ZIG_MM/zls\" ]; then \
               ZLS_BIN=\"$HOME/.otto/toolchains/zig/zls/$ZIG_MM/zls\"; \
             fi; "
        )
    } else {
        String::new()
    };

    let is_csharp_ls = Path::new(&launch.cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "csharp-ls");

    // BUG-22 (SSH): Select the per-SDK-major managed csharp-ls slot (`net8`/`net9`/`net10`/`net11`)
    // matching `installer_remote.rs` and the remote active `dotnet --version`.
    let csharp_slot_setup = if is_csharp_ls {
        "CS_BIN=\"\"; \
         if command -v dotnet >/dev/null 2>&1; then \
           DN_MAJ=$(dotnet --version 2>/dev/null | head -n 1 | cut -d. -f1); \
           case \"$DN_MAJ\" in \
             6|7|8) CS_SLOT='net8' ;; \
             9) CS_SLOT='net9' ;; \
             10) CS_SLOT='net10' ;; \
             11|12|13) CS_SLOT='net11' ;; \
             *) CS_SLOT='net9' ;; \
           esac; \
           if [ -x \"$HOME/.otto/toolchains/dotnet/tools/$CS_SLOT/csharp-ls\" ]; then \
             CS_BIN=\"$HOME/.otto/toolchains/dotnet/tools/$CS_SLOT/csharp-ls\"; \
           fi; \
         fi; "
    } else {
        ""
    };

    let invoke_primary = if is_ngserver {
        format!(
            "{invoke_primary} ${{NG_PROBES:+--tsProbeLocations \"$NG_PROBES\" --ngProbeLocations \"$NG_PROBES\"}}"
        )
    } else if is_zls {
        let extra_args = launch
            .args
            .iter()
            .map(|a| format!(" {}", posix_quote(a)))
            .collect::<String>();
        format!("\"${{ZLS_BIN:-zls}}\"{extra_args}")
    } else if is_csharp_ls {
        let extra_args = launch
            .args
            .iter()
            .map(|a| format!(" {}", posix_quote(a)))
            .collect::<String>();
        format!("\"${{CS_BIN:-csharp-ls}}\"{extra_args}")
    } else {
        invoke_primary
    };

    let inner = if let Some(fb) = &launch.fallback_cmd {
        let invoke_fb = std::iter::once(posix_quote(fb))
            .chain(launch.fallback_args.iter().map(|a| posix_quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let p_check = posix_quote(&launch.cmd);
        let fb_check = posix_quote(fb);
        format!(
            "cd {quoted_root} && {path_setup}; {mem_exports}; {ng_probe_setup}{zls_slot_setup}{csharp_slot_setup}\
             if [ -n \"${{ZLS_BIN:-}}\" ] || [ -n \"${{CS_BIN:-}}\" ] || command -v {p_check} >/dev/null 2>&1; then exec {invoke_primary}; \
             elif command -v {fb_check} >/dev/null 2>&1; then exec {invoke_fb}; \
             else echo 'otto-lsp: language server not found on remote host' >&2; exit 127; fi"
        )
    } else {
        format!(
            "cd {quoted_root} && {path_setup}; {mem_exports}; {ng_probe_setup}{zls_slot_setup}{csharp_slot_setup}exec {invoke_primary}"
        )
    };
    crate::target::ssh_exec::wrap_remote_stdio_with_eof_kill(&inner)
}

/// POSIX shell probe executed on an SSH host to locate a trusted `typescript/lib` (`tsdk`)
/// directory for standalone `@vue/language-server@2.2.10` (`RED-2`) without ever touching
/// workspace `$PWD/node_modules`.
pub(crate) fn build_remote_vue_tsdk_probe_script() -> &'static str {
    concat!(
        "PATH=\"$HOME/.otto/toolchains/node/lsp/node_modules/.bin:$HOME/.otto/toolchains/node/current/bin:$HOME/.npm-global/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"; ",
        "check_tsdk() { [ -f \"$1/tsserverlibrary.js\" ] || [ -f \"$1/typescript.js\" ]; }; ",
        "for d in \"$HOME/.otto/toolchains/node/lsp/node_modules/typescript/lib\"; do ",
        "  if check_tsdk \"$d\"; then printf '%s\\n' \"$d\"; exit 0; fi; ",
        "done; ",
        "VUE_BIN=$(command -v vue-language-server 2>/dev/null || true); ",
        "if [ -n \"$VUE_BIN\" ]; then ",
        "  VUE_REAL=$(readlink -f \"$VUE_BIN\" 2>/dev/null || realpath \"$VUE_BIN\" 2>/dev/null || printf '%s' \"$VUE_BIN\"); ",
        "  CUR=$(dirname \"$VUE_REAL\"); ",
        "  while [ \"$CUR\" != \"/\" ] && [ -n \"$CUR\" ]; do ",
        "    if [ \"$(basename \"$CUR\")\" = \"node_modules\" ]; then ",
        "      if check_tsdk \"$CUR/typescript/lib\"; then printf '%s\\n' \"$CUR/typescript/lib\"; exit 0; fi; ",
        "      break; ",
        "    fi; ",
        "    if check_tsdk \"$CUR/node_modules/typescript/lib\"; then printf '%s\\n' \"$CUR/node_modules/typescript/lib\"; exit 0; fi; ",
        "    CUR=$(dirname \"$CUR\"); ",
        "  done; ",
        "fi; ",
        "for d in /usr/local/lib/node_modules/typescript/lib /usr/lib/node_modules/typescript/lib /opt/homebrew/lib/node_modules/typescript/lib; do ",
        "  if check_tsdk \"$d\"; then printf '%s\\n' \"$d\"; exit 0; fi; ",
        "done; ",
        "exit 1"
    )
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
        "csharp" | "fsharp" => {
            if !inherited.contains_key("DOTNET_CLI_TELEMETRY_OPTOUT") {
                command.env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
            }
            if !inherited.contains_key("DOTNET_NOLOGO") {
                command.env("DOTNET_NOLOGO", "1");
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
        "typescript" | "javascript" | "svelte" | "vue" | "angular" | "shell" | "css" | "html" => {
            set_node_heap_cap(command, inherited);
        }
        "python" if cmd_name.contains("pyright") => {
            set_node_heap_cap(command, inherited);
        }
        "cpp" => {
            if !existing_args.iter().any(|a| a.starts_with("--pch-storage")) {
                command.arg("--pch-storage=disk");
            }
            if !existing_args
                .iter()
                .any(|a| a == "-j" || a.starts_with("-j"))
            {
                command.arg("-j=4");
            }
            if !existing_args
                .iter()
                .any(|a| a.starts_with("--compile-commands-dir"))
            {
                if let Some(cdb_dir) = discover_compile_commands_dir(root) {
                    command.arg(format!("--compile-commands-dir={}", cdb_dir.display()));
                }
            }
        }
        _ => {}
    }
}

/// Locate the directory holding `compile_commands.json` (root, CMake/Meson build
/// dirs, or Unreal Engine `Intermediate/ProjectFiles`).
#[allow(clippy::disallowed_methods)]
pub(crate) fn discover_compile_commands_dir(root: &str) -> Option<PathBuf> {
    let base = std::path::Path::new(root);
    if !base.is_dir() {
        return None;
    }
    for rel in [
        "",
        "build",
        "cmake-build-debug",
        "cmake-build-release",
        "out/build",
        "out/build/debug",
        "builddir",
        "build/debug",
        "build/Debug",
        ".vscode",
        "Intermediate/ProjectFiles",
    ] {
        let dir = if rel.is_empty() {
            base.to_path_buf()
        } else {
            base.join(rel)
        };
        if dir.join("compile_commands.json").is_file() {
            return Some(dir);
        }
    }
    None
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

/// Resolve validated project-pinned SDK executable directories that must take
/// precedence over Otto's global managed `toolchains/*/current` defaults.
fn pinned_local_lsp_sdk_dirs(
    lang: &str,
    root: &str,
    opened_project_root: Option<&str>,
    environments: Option<&EnvironmentManager>,
    owner_is_root: bool,
) -> Vec<PathBuf> {
    if !owner_is_root {
        return Vec::new();
    }
    let Some(envs) = environments else {
        return Vec::new();
    };
    let norm = servers::normalize_lsp_lang(lang);
    if norm == "zig" {
        let Some(zig_home) = envs
            .cached_project_env_for_module(root, opened_project_root)
            .and_then(|e| e.zig_home)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            return Vec::new();
        };
        let hp = PathBuf::from(zig_home);
        if let Some(zig_bin) = crate::toolchains::discovery::zig_bin_in_home(&hp) {
            if let Some(dir) = zig_bin.parent() {
                return vec![dir.to_path_buf()];
            }
        }
    } else if matches!(norm, "csharp" | "fsharp") {
        if let Some(dr) = pinned_validated_dotnet_root(root, envs) {
            if let Some(bin) = crate::toolchains::discovery::dotnet_bin_in_root(&dr) {
                if let Some(dir) = bin.parent() {
                    return vec![dir.to_path_buf()];
                }
            }
        }
    }
    Vec::new()
}

/// Validate a project-pinned `dotnet_root` directory (must contain an executable
/// `dotnet` and lie outside the workspace root so untrusted repo contents cannot
/// hijack `DOTNET_ROOT`).
#[allow(clippy::disallowed_methods)]
fn validate_dotnet_root_outside_workspace(
    candidate: &Path,
    root_path: &Path,
    canon_ws: Option<&Path>,
) -> Option<PathBuf> {
    if !candidate.is_absolute() {
        return None;
    }
    let dr = crate::toolchains::discovery::resolve_dotnet_root_dir(candidate)?;
    if dr.starts_with(root_path) {
        return None;
    }
    if let Some(cw) = canon_ws {
        if dr.starts_with(cw) {
            return None;
        }
        if std::fs::canonicalize(&dr).is_ok_and(|c| c.starts_with(cw)) {
            return None;
        }
    }
    Some(dr)
}

fn pinned_validated_dotnet_root(root: &str, envs: &EnvironmentManager) -> Option<PathBuf> {
    let raw = envs
        .cached_project_env_or_ancestor(root)
        .and_then(|e| e.dotnet_root)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let root_path = Path::new(root);
    #[allow(clippy::disallowed_methods)]
    let canon_ws = std::fs::canonicalize(root_path).ok();
    validate_dotnet_root_outside_workspace(Path::new(&raw), root_path, canon_ws.as_deref())
}

/// Resolve a safe, validated `DOTNET_ROOT` for local LSP spawning after
/// `is_dangerous_loader_env_key` strips untrusted `DOTNET_*` vars (BUG-23).
pub(crate) fn resolve_local_lsp_dotnet_root(
    data_dir: &Path,
    root: &str,
    pinned_dotnet_root: Option<&Path>,
    owner_is_root: bool,
) -> Option<PathBuf> {
    let root_path = Path::new(root);
    #[allow(clippy::disallowed_methods)]
    let canon_ws = std::fs::canonicalize(root_path).ok();
    if owner_is_root {
        if let Some(pinned) = pinned_dotnet_root {
            if let Some(valid) =
                validate_dotnet_root_outside_workspace(pinned, root_path, canon_ws.as_deref())
            {
                return Some(valid);
            }
        }
    }
    servers::detect_toolchain_dotnet_root(data_dir)
}

/// Select the per-SDK-major managed `csharp-ls` slot binary (`toolchains/dotnet/tools/net<major>/csharp-ls`)
/// matching the SDK `dotnet --version` resolves in `project_root` under `dotnet_root`, falling
/// back to `dotnet_dir_version` (BUG-22).
#[allow(clippy::disallowed_methods)]
pub(crate) fn resolve_local_csharp_ls_for_dotnet_root(
    data_dir: &Path,
    dotnet_root: &Path,
    project_root: &Path,
    fallback_cmd: &str,
) -> String {
    let is_csharp_ls = Path::new(fallback_cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "csharp-ls");
    if !is_csharp_ls {
        return fallback_cmd.to_string();
    }
    // `dotnet --version` run in the project resolves its `global.json` (and rollForward)
    // exactly as the SDK itself will for this project.
    let active_ver = crate::toolchains::discovery::dotnet_bin_in_root(dotnet_root).and_then(|bin| {
        let out = std::process::Command::new(&bin)
            .arg("--version")
            .current_dir(project_root)
            .env("DOTNET_ROOT", dotnet_root)
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let first = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        first
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
            .then_some(first)
    });
    let ver_opt = active_ver.or_else(|| {
        Some(crate::toolchains::discovery::dotnet_dir_version(dotnet_root)).filter(|v| v != "system")
    });
    let slot = crate::toolchains_catalog::dotnet_sdk_major_slot(ver_opt.as_deref());
    let cand = data_dir
        .join("toolchains/dotnet/tools")
        .join(slot)
        .join("csharp-ls");
    if crate::toolchains::discovery::is_executable_file(&cand) {
        return cand.to_string_lossy().into_owned();
    }
    fallback_cmd.to_string()
}

/// Compute `--tsProbeLocations` and `--ngProbeLocations` args for Angular's `ngserver`
/// when not already supplied by the caller (SEC2-1, BUG-32).
pub(crate) fn angular_ngserver_probe_args(
    data_dir: &Path,
    lang: &str,
    cmd: &str,
    existing_args: &[String],
) -> Vec<String> {
    if servers::normalize_lsp_lang(lang) != "angular" {
        return Vec::new();
    }
    let is_ngserver = Path::new(cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "ngserver");
    if !is_ngserver {
        return Vec::new();
    }
    if existing_args
        .iter()
        .any(|a| a == "--tsProbeLocations" || a.starts_with("--tsProbeLocations="))
    {
        return Vec::new();
    }
    let probes = servers::resolve_angular_probe_locations(data_dir, cmd);
    if probes.is_empty() {
        return Vec::new();
    }
    let joined = probes.join(",");
    vec![
        "--tsProbeLocations".into(),
        joined.clone(),
        "--ngProbeLocations".into(),
        joined,
    ]
}

/// Merge pinned project SDK dirs, managed/shell augmented PATH, and effective
/// project PATH while stripping relative segments and workspace-local dirs.
pub(crate) fn merge_local_lsp_path(
    pinned_sdk_dirs: &[PathBuf],
    aug_path: &str,
    env_path: Option<&str>,
    root_path: &std::path::Path,
    canon_ws: Option<&std::path::Path>,
) -> String {
    let mut combined = String::new();
    for dir in pinned_sdk_dirs {
        let s = dir.to_string_lossy();
        if !s.is_empty() {
            if !combined.is_empty() {
                combined.push(':');
            }
            combined.push_str(&s);
        }
    }
    if !aug_path.is_empty() {
        if !combined.is_empty() {
            combined.push(':');
        }
        combined.push_str(aug_path);
    }
    if let Some(ep) = env_path.filter(|s| !s.is_empty()) {
        if !combined.is_empty() {
            combined.push(':');
        }
        combined.push_str(ep);
    }
    let mut safe_path_parts: Vec<String> = Vec::with_capacity(32);
    for seg in combined.split(':').filter(|s| !s.is_empty()) {
        let sp = std::path::Path::new(seg);
        if !sp.is_absolute() || sp.starts_with(root_path) {
            continue;
        }
        if let Some(cw) = canon_ws {
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
    safe_path_parts.join(":")
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
            let pinned_sdk_dirs = pinned_local_lsp_sdk_dirs(
                &key.lang,
                &key.root,
                launch.project_root.as_deref(),
                environments.as_ref(),
                owner_is_root,
            );
            let root_path = std::path::Path::new(&key.root);
            #[allow(clippy::disallowed_methods)]
            let canon_ws = std::fs::canonicalize(root_path).ok();
            let merged_path = merge_local_lsp_path(
                &pinned_sdk_dirs,
                &aug_path,
                effective_env.get("PATH").map(String::as_str),
                root_path,
                canon_ws.as_deref(),
            );

            let resolved_dotnet_root =
                if matches!(servers::normalize_lsp_lang(&key.lang), "csharp" | "fsharp") {
                    let pinned_dotnet = environments
                        .as_ref()
                        .and_then(|e| e.cached_project_env_or_ancestor(&key.root))
                        .and_then(|e| e.dotnet_root)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from);
                    resolve_local_lsp_dotnet_root(
                        &data_dir,
                        &key.root,
                        pinned_dotnet.as_deref(),
                        owner_is_root,
                    )
                } else {
                    None
                };
            let spawn_cmd = if servers::normalize_lsp_lang(&key.lang) == "csharp" {
                if let Some(ref dr) = resolved_dotnet_root {
                    resolve_local_csharp_ls_for_dotnet_root(
                        &data_dir,
                        dr,
                        Path::new(&key.root),
                        &launch.cmd,
                    )
                } else {
                    launch.cmd.clone()
                }
            } else {
                launch.cmd.clone()
            };

            let mut c = tokio::process::Command::new(&spawn_cmd);
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
            if let Some(dotnet_root) = resolved_dotnet_root {
                c.env("DOTNET_ROOT", dotnet_root);
            }
            let ng_probe_args =
                angular_ngserver_probe_args(&data_dir, &key.lang, &launch.cmd, &launch.args);
            if !ng_probe_args.is_empty() {
                c.args(&ng_probe_args);
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

    #[test]
    fn merge_local_lsp_path_puts_pinned_project_zig_before_managed_current() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        let pinned = tmp.path().join("sdks/zig-0.13.0");
        let managed = tmp.path().join("data/toolchains/zig/current");
        let aug = format!("{}:/usr/bin", managed.display());
        let env_p = format!("{}:/opt/homebrew/bin", pinned.display());
        let merged =
            merge_local_lsp_path(std::slice::from_ref(&pinned), &aug, Some(&env_p), &ws, None);
        let parts: Vec<&str> = merged.split(':').collect();
        let pin_pos = parts
            .iter()
            .position(|p| *p == pinned.to_string_lossy())
            .expect("pinned zig dir present");
        let mgr_pos = parts
            .iter()
            .position(|p| *p == managed.to_string_lossy())
            .expect("managed current dir present");
        assert!(
            pin_pos < mgr_pos,
            "pinned project Zig ({pin_pos}) must precede managed zig/current ({mgr_pos}): {merged}"
        );
    }

    #[test]
    fn remote_lsp_script_includes_dotnet_tools_dotnet_root_and_angular_probes() {
        let cs_launch = SpawnLaunch {
            cmd: "csharp-ls".into(),
            args: vec![],
            fallback_cmd: None,
            fallback_args: vec![],
            ..Default::default()
        };
        let cs_script = build_remote_lsp_script("/srv/app", &cs_launch);
        assert!(
            cs_script.contains("\"$HOME\"/.otto/toolchains/dotnet/tools"),
            "API-5: remote LSP PATH must include managed dotnet/tools: {cs_script}"
        );
        assert!(
            cs_script.contains("DOTNET_ROOT=\"$HOME/.otto/toolchains/dotnet/current\""),
            "API-5: remote LSP script must export managed DOTNET_ROOT: {cs_script}"
        );

        let ng_launch = SpawnLaunch {
            cmd: "ngserver".into(),
            args: vec!["--stdio".into()],
            fallback_cmd: Some("vscode-html-language-server".into()),
            fallback_args: vec!["--stdio".into()],
            ..Default::default()
        };
        let ng_script = build_remote_lsp_script("/srv/ng-app", &ng_launch);
        assert!(
            ng_script.contains("--tsProbeLocations") && ng_script.contains("--ngProbeLocations"),
            "BUG-32: remote ngserver must pass probe locations: {ng_script}"
        );
        assert!(
            !ng_script.contains("$PWD/node_modules"),
            "SEC2-1: remote ngserver must never auto-probe workspace $PWD/node_modules: {ng_script}"
        );
        assert!(
            ng_script.contains("readlink -f") || ng_script.contains("realpath"),
            "BUG-32: remote ngserver must resolve global symlink node_modules: {ng_script}"
        );
    }

    #[test]
    fn local_lsp_restores_validated_dotnet_root_for_nested_child_modules_and_rejects_workspace_local_dotnet(
    ) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("umbrella");
        let child_mod = ws.join("services/orders");
        let ws_dotnet = ws.join("evil-dotnet");
        let pinned_dotnet = tmp.path().join("sdks/dotnet-9");
        let data_dir = tmp.path().join("data");
        let managed_dotnet = data_dir.join("toolchains/dotnet/current");
        std::fs::create_dir_all(&child_mod).unwrap();
        for dir in [&ws_dotnet, &pinned_dotnet, &managed_dotnet] {
            std::fs::create_dir_all(dir).unwrap();
            let bin = dir.join("dotnet");
            std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let ws_str = ws.to_string_lossy().into_owned();
        let child_str = child_mod.to_string_lossy().into_owned();

        // BUG-23: A nested .NET module inherits the umbrella project's pinned DOTNET_ROOT
        // through EnvironmentManager::cached_project_env_or_ancestor.
        let envs = EnvironmentManager::new(data_dir.clone());
        let proj_env = crate::types::IdeProjectEnvironment {
            dotnet_root: Some(pinned_dotnet.to_string_lossy().into_owned()),
            ..Default::default()
        };
        envs.seed_snapshot(&format!("local:{ws_str}"), proj_env);

        let inherited_dirs =
            pinned_local_lsp_sdk_dirs("csharp", &child_str, Some(&ws_str), Some(&envs), true);
        assert_eq!(inherited_dirs, vec![pinned_dotnet.clone()]);

        let inherited_root = envs
            .cached_project_env_or_ancestor(&child_str)
            .and_then(|e| e.dotnet_root)
            .map(PathBuf::from);
        let picked =
            resolve_local_lsp_dotnet_root(&data_dir, &child_str, inherited_root.as_deref(), true)
                .expect("inherited pinned dotnet root");
        assert_eq!(picked, pinned_dotnet);

        // Workspace-local dotnet_root is rejected and falls back to managed DOTNET_ROOT.
        let fallback = resolve_local_lsp_dotnet_root(&data_dir, &ws_str, Some(&ws_dotnet), true)
            .expect("managed dotnet fallback");
        assert_eq!(fallback, managed_dotnet);

        // Non-root owner gets managed DOTNET_ROOT even if project pinned one.
        let non_root =
            resolve_local_lsp_dotnet_root(&data_dir, &ws_str, Some(&pinned_dotnet), false)
                .expect("managed dotnet for non-root");
        assert_eq!(non_root, managed_dotnet);
    }

    #[test]
    fn angular_ngserver_probe_args_excludes_workspace_and_resolves_global_symlink_node_modules() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let ws = tmp.path().join("ng-workspace");
        let ws_nm = ws.join("node_modules");
        std::fs::create_dir_all(&ws_nm).unwrap();

        // Simulate global `npm install -g @angular/language-server`:
        // `<prefix>/bin/ngserver` -> `../lib/node_modules/@angular/language-server/bin/ngserver`
        // with NO managed toolchain dir installed.
        let prefix = tmp.path().join("global-npm");
        let global_nm = prefix.join("lib/node_modules");
        let real_bin = global_nm.join("@angular/language-server/bin/ngserver");
        let bin_dir = prefix.join("bin");
        std::fs::create_dir_all(real_bin.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(&real_bin, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&real_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sym_bin = bin_dir.join("ngserver");
        std::os::unix::fs::symlink(&real_bin, &sym_bin).unwrap();

        let args = angular_ngserver_probe_args(
            &data_dir,
            "angular",
            sym_bin.to_str().unwrap(),
            &["--stdio".into()],
        );
        assert_eq!(
            args.len(),
            4,
            "BUG-32: global symlink ngserver must receive probes: {args:?}"
        );
        assert_eq!(args[0], "--tsProbeLocations");
        assert_eq!(args[2], "--ngProbeLocations");
        let canon_global_nm = std::fs::canonicalize(&global_nm).unwrap();
        assert!(
            args[1].contains(canon_global_nm.to_str().unwrap()),
            "BUG-32: probes must contain canonical global lib/node_modules: {:?}",
            args[1]
        );
        // SEC2-1: workspace node_modules must NEVER be probed automatically.
        assert!(
            !args[1].contains(ws_nm.to_str().unwrap()),
            "SEC2-1: workspace node_modules must never be included in ngserver probes: {:?}",
            args[1]
        );

        // Fallback html server must not receive ngserver-only flags.
        let fb_args = angular_ngserver_probe_args(
            &data_dir,
            "angular",
            "/usr/local/bin/vscode-html-language-server",
            &["--stdio".into()],
        );
        assert!(fb_args.is_empty());
    }

    #[test]
    fn umbrella_zig_pin_wins_over_managed_current_for_child_build_zig_module() {
        use std::os::unix::fs::PermissionsExt;

        // BUG-6: When an umbrella project pins `zig_home` and a nested child module
        // has its own `build.zig` with no separate environment or module_sdks entry,
        // `pinned_local_lsp_sdk_dirs` must inherit the umbrella pin and place it ahead
        // of Otto's managed `toolchains/zig/current`.
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let umbrella = tmp.path().join("umbrella");
        let child_mod = umbrella.join("packages/worker");
        let pinned_zig = tmp.path().join("sdks/zig-0.13.0");
        let managed_zig = data_dir.join("toolchains/zig/current");
        std::fs::create_dir_all(&child_mod).unwrap();
        std::fs::write(child_mod.join("build.zig"), "").unwrap();
        for dir in [&pinned_zig, &managed_zig] {
            std::fs::create_dir_all(dir).unwrap();
            let bin = dir.join("zig");
            std::fs::write(&bin, "#!/bin/sh\necho 0.13.0\n").unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let umbrella_s = umbrella.to_string_lossy().into_owned();
        let child_s = child_mod.to_string_lossy().into_owned();
        let envs = EnvironmentManager::new(data_dir);
        envs.seed_snapshot(
            &format!("local:{umbrella_s}"),
            crate::types::IdeProjectEnvironment {
                zig_home: Some(pinned_zig.to_string_lossy().into_owned()),
                ..Default::default()
            },
        );

        let pinned_dirs =
            pinned_local_lsp_sdk_dirs("zig", &child_s, Some(&umbrella_s), Some(&envs), true);
        assert_eq!(pinned_dirs, vec![pinned_zig.clone()]);

        let aug = format!("{}:/usr/bin:/bin", managed_zig.display());
        let merged = merge_local_lsp_path(&pinned_dirs, &aug, None, &child_mod, None);
        let pin_pos = merged
            .find(&pinned_zig.to_string_lossy().to_string())
            .unwrap();
        let mgr_pos = merged
            .find(&managed_zig.to_string_lossy().to_string())
            .unwrap();
        assert!(
            pin_pos < mgr_pos,
            "inherited umbrella Zig pin ({pin_pos}) must precede managed zig/current ({mgr_pos}): {merged}"
        );

        // Unrelated directory outside the umbrella must NOT inherit the umbrella pin.
        let other = tmp.path().join("other-project");
        std::fs::create_dir_all(&other).unwrap();
        let other_dirs = pinned_local_lsp_sdk_dirs(
            "zig",
            &other.to_string_lossy(),
            Some(&umbrella_s),
            Some(&envs),
            true,
        );
        assert!(other_dirs.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn remote_zig_lsp_script_selects_pinned_minor_zls_slot_even_after_global_link_clobbered() {
        use std::os::unix::fs::PermissionsExt;

        // BUG-4 (SSH): After installing ZLS 0.14 overwrites `$HOME/.otto/toolchains/zig/bin/zls`,
        // launching ZLS for an SSH project pinned to Zig 0.13 must still exec the 0.13 slot.
        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("home");
        let proj = tmp.path().join("remote-zig-013");
        let sdk_13 = fake_home.join(".otto/toolchains/zig/sdks/0.13.0");
        let zls_13_dir = fake_home.join(".otto/toolchains/zig/zls/0.13");
        let zls_14_dir = fake_home.join(".otto/toolchains/zig/zls/0.14");
        let global_bin = fake_home.join(".otto/toolchains/zig/bin");
        for d in [&proj, &sdk_13, &zls_13_dir, &zls_14_dir, &global_bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        let write_exec = |p: &Path, body: &str| {
            std::fs::write(p, body).unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_exec(&sdk_13.join("zig"), "#!/bin/sh\necho 0.13.0\n");
        write_exec(&zls_13_dir.join("zls"), "#!/bin/sh\necho ZLS_SLOT_0_13\n");
        write_exec(&zls_14_dir.join("zls"), "#!/bin/sh\necho ZLS_SLOT_0_14\n");
        // Global symlink points at 0.14 (clobbered by second project's install).
        std::os::unix::fs::symlink(zls_14_dir.join("zls"), global_bin.join("zls")).unwrap();

        let launch = SpawnLaunch {
            cmd: "zls".into(),
            args: vec![],
            fallback_cmd: None,
            fallback_args: vec![],
            project_root: Some(proj.to_string_lossy().into_owned()),
            zig_home: Some(sdk_13.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let script = build_remote_lsp_script(&proj.to_string_lossy(), &launch);
        #[allow(clippy::disallowed_methods)]
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", &fake_home)
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "remote zig lsp script failed: {:?}",
            out
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("ZLS_SLOT_0_13") && !stdout.contains("ZLS_SLOT_0_14"),
            "BUG-4: expected SSH launcher to exec 0.13 ZLS slot, got: {stdout}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn csharp_ls_selects_per_sdk_major_slot_locally_and_remotely() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let tools = data_dir.join("toolchains/dotnet/tools");
        let net8_dir = tools.join("net8");
        let net9_dir = tools.join("net9");
        let net11_dir = tools.join("net11");
        let shared_root = tmp.path().join("shared-dotnet");
        let sdk11 = tmp.path().join("sdk11");
        let proj8_gj = tmp.path().join("proj-global-net8");
        let proj9_def = tmp.path().join("proj-default-net9");
        for d in [
            &net8_dir,
            &net9_dir,
            &net11_dir,
            &shared_root.join("sdk/8.0.408"),
            &shared_root.join("sdk/9.0.200"),
            &sdk11,
            &proj8_gj,
            &proj9_def,
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        let write_exec = |p: &Path, body: &str| {
            std::fs::write(p, body).unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // Fake SDK resolution in the cwd, as the real `dotnet --version` does: a
        // global.json rolling 8.0.100 forward to the latest major resolves to 9.
        write_exec(
            &shared_root.join("dotnet"),
            "#!/bin/sh\nif grep -q latestMajor global.json 2>/dev/null; then echo 9.0.200; elif grep -q '\"8\\.' global.json 2>/dev/null; then echo 8.0.408; else echo 9.0.200; fi\n",
        );
        write_exec(&sdk11.join("dotnet"), "#!/bin/sh\necho 11.0.100\n");
        write_exec(&net8_dir.join("csharp-ls"), "#!/bin/sh\necho CSHARP_NET8\n");
        write_exec(&net9_dir.join("csharp-ls"), "#!/bin/sh\necho CSHARP_NET9\n");
        write_exec(
            &net11_dir.join("csharp-ls"),
            "#!/bin/sh\necho CSHARP_NET11\n",
        );
        std::fs::write(
            proj8_gj.join("global.json"),
            r#"{"sdk":{"version":"8.0.408"}}"#,
        )
        .unwrap();
        let proj_roll = tmp.path().join("proj-roll-forward");
        std::fs::create_dir_all(&proj_roll).unwrap();
        std::fs::write(
            proj_roll.join("global.json"),
            r#"{"sdk":{"version":"8.0.100","rollForward":"latestMajor"}}"#,
        )
        .unwrap();

        // BUG-22 (local): Even with a shared DOTNET_ROOT containing both 8.0 and 9.0 SDKs,
        // a project pinning 8.0.408 via global.json gets net8/csharp-ls while an unpinned
        // project gets net9/csharp-ls.
        let picked8 = resolve_local_csharp_ls_for_dotnet_root(
            &data_dir,
            &shared_root,
            &proj8_gj,
            "csharp-ls",
        );
        let picked9 = resolve_local_csharp_ls_for_dotnet_root(
            &data_dir,
            &shared_root,
            &proj9_def,
            "csharp-ls",
        );
        assert_eq!(picked8, net8_dir.join("csharp-ls").to_string_lossy());
        assert_eq!(picked9, net9_dir.join("csharp-ls").to_string_lossy());
        // The slot follows the SDK the project resolves to, not global.json's literal version.
        let picked_roll = resolve_local_csharp_ls_for_dotnet_root(
            &data_dir,
            &shared_root,
            &proj_roll,
            "csharp-ls",
        );
        assert_eq!(picked_roll, net9_dir.join("csharp-ls").to_string_lossy());

        // BUG-22 (SSH): Remote SSH script maps .NET 11 to net11 (matching installer_remote.rs)
        // instead of misrouting .NET 11 to net10.
        let fake_home = tmp.path().join("remote-home");
        let r_net10 = fake_home.join(".otto/toolchains/dotnet/tools/net10");
        let r_net11 = fake_home.join(".otto/toolchains/dotnet/tools/net11");
        let proj = tmp.path().join("proj11");
        for d in [&r_net10, &r_net11, &proj] {
            std::fs::create_dir_all(d).unwrap();
        }
        write_exec(
            &r_net10.join("csharp-ls"),
            "#!/bin/sh\necho REMOTE_CS_NET10\n",
        );
        write_exec(
            &r_net11.join("csharp-ls"),
            "#!/bin/sh\necho REMOTE_CS_NET11\n",
        );
        let launch = SpawnLaunch {
            cmd: "csharp-ls".into(),
            args: vec![],
            fallback_cmd: None,
            fallback_args: vec![],
            project_root: Some(proj.to_string_lossy().into_owned()),
            zig_home: None,
            ..Default::default()
        };
        let script = build_remote_lsp_script(&proj.to_string_lossy(), &launch);
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", &fake_home)
            .env("PATH", format!("{}:/usr/bin:/bin", sdk11.display()))
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "remote csharp-ls script failed: {:?}",
            out
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("REMOTE_CS_NET11") && !stdout.contains("REMOTE_CS_NET10"),
            "BUG-22: expected SSH launcher to pick net11 csharp-ls slot for .NET 11, got: {stdout}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remote_angular_probe_script_includes_package_private_and_outer_node_modules_never_workspace(
    ) {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("home");
        let global_bin = tmp.path().join("global/bin");
        let pkg_root = tmp
            .path()
            .join("global/lib/node_modules/@angular/language-server");
        let pkg_bin = pkg_root.join("bin");
        let pkg_private_nm = pkg_root.join("node_modules");
        let ws = tmp.path().join("workspace-ng");
        let ws_nm = ws.join("node_modules/@angular/language-service");
        for d in [
            &fake_home,
            &global_bin,
            &pkg_bin,
            &pkg_private_nm.join("@angular/language-service"),
            &pkg_private_nm.join("typescript"),
            &ws_nm,
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        let real_ng = pkg_bin.join("ngserver");
        std::fs::write(&real_ng, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&real_ng, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(
            "../lib/node_modules/@angular/language-server/bin/ngserver",
            global_bin.join("ngserver"),
        )
        .unwrap();

        let launch = SpawnLaunch {
            cmd: "ngserver".into(),
            args: vec!["--stdio".into()],
            fallback_cmd: Some("vscode-html-language-server".into()),
            fallback_args: vec!["--stdio".into()],
            ..Default::default()
        };
        let script = build_remote_lsp_script(&ws.to_string_lossy(), &launch);
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", &fake_home)
            .env("PATH", format!("{}:/usr/bin:/bin", global_bin.display()))
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "remote ngserver script failed: {out:?}"
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let canon_priv = std::fs::canonicalize(&pkg_private_nm).unwrap();
        let canon_outer =
            std::fs::canonicalize(tmp.path().join("global/lib/node_modules")).unwrap();
        assert!(
            stdout.contains(canon_priv.to_str().unwrap()),
            "BUG-32: SSH ngserver probes must include package-private node_modules ({}):\n{stdout}",
            canon_priv.display()
        );
        assert!(
            stdout.contains(canon_outer.to_str().unwrap()),
            "BUG-32: SSH ngserver probes must include outer global node_modules ({}):\n{stdout}",
            canon_outer.display()
        );
        assert!(
            !stdout.contains(&ws.join("node_modules").to_string_lossy().to_string()),
            "SEC2-1: SSH ngserver probes must NEVER include workspace node_modules:\n{stdout}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remote_vue_tsdk_probe_script_resolves_managed_and_global_tsdk_never_workspace() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("home");
        let ws = tmp.path().join("vue-repo");
        let ws_tsdk = ws.join("node_modules/typescript/lib");
        let global_bin = tmp.path().join("global/bin");
        let vue_pkg_bin = tmp
            .path()
            .join("global/lib/node_modules/@vue/language-server/bin");
        let global_tsdk = tmp.path().join("global/lib/node_modules/typescript/lib");
        for d in [
            &fake_home,
            &ws_tsdk,
            &global_bin,
            &vue_pkg_bin,
            &global_tsdk,
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(ws_tsdk.join("tsserverlibrary.js"), "// untrusted").unwrap();
        std::fs::write(global_tsdk.join("tsserverlibrary.js"), "// global").unwrap();
        let real_vue = vue_pkg_bin.join("vue-language-server.js");
        std::fs::write(&real_vue, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&real_vue, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(
            "../lib/node_modules/@vue/language-server/bin/vue-language-server.js",
            global_bin.join("vue-language-server"),
        )
        .unwrap();

        // Global symlinked vue-language-server finds outer global typescript/lib, never workspace.
        let out_global = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(build_remote_vue_tsdk_probe_script())
            .current_dir(&ws)
            .env("HOME", &fake_home)
            .env("PATH", format!("{}:/usr/bin:/bin", global_bin.display()))
            .output()
            .await
            .unwrap();
        assert!(out_global.status.success(), "{out_global:?}");
        let picked_global = String::from_utf8_lossy(&out_global.stdout)
            .trim()
            .to_string();
        assert_eq!(
            std::fs::canonicalize(&picked_global).unwrap(),
            std::fs::canonicalize(&global_tsdk).unwrap(),
            "RED-2: remote probe must resolve global typescript/lib"
        );
        assert!(!picked_global.contains("vue-repo"));

        // Managed ~/.otto/toolchains/node/lsp/node_modules/typescript/lib takes precedence.
        let managed_tsdk = fake_home.join(".otto/toolchains/node/lsp/node_modules/typescript/lib");
        std::fs::create_dir_all(&managed_tsdk).unwrap();
        std::fs::write(managed_tsdk.join("tsserverlibrary.js"), "// managed").unwrap();
        let out_managed = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(build_remote_vue_tsdk_probe_script())
            .current_dir(&ws)
            .env("HOME", &fake_home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .await
            .unwrap();
        assert!(out_managed.status.success(), "{out_managed:?}");
        let picked_managed = String::from_utf8_lossy(&out_managed.stdout)
            .trim()
            .to_string();
        assert_eq!(
            std::fs::canonicalize(&picked_managed).unwrap(),
            std::fs::canonicalize(&managed_tsdk).unwrap(),
            "RED-2: remote probe must prefer managed ~/.otto typescript/lib"
        );
    }
}

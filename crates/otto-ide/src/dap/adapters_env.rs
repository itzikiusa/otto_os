//! Path normalization, workspace-display symlink mapping, non-root launch/attach
//! security validation, and effective environment assembly for DAP debug sessions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

use super::DapStartContext;
use crate::environments::EnvironmentManager;
use crate::types::{IdeStartDebugReq, IdeTargetKind, IdeTargetSpec};

/// Canonicalize a path on `Local` targets (expanding `~` and resolving macOS `/var/folders` ->
/// `/private/var/folders` symlinks) so breakpoints and DWARF/CDP/JDWP stack frames never mismatch.
pub fn normalize_target_path(target: &IdeTargetSpec, raw: &str) -> String {
    if raw.trim().is_empty() {
        return raw.to_string();
    }
    let mut clean = crate::target::sdk_paths::strip_file_uri(raw.trim());
    if target.kind == IdeTargetKind::Local {
        if clean == "~" {
            if let Some(home) = dirs::home_dir() {
                clean = home.to_string_lossy().into_owned();
            }
        } else if let Some(rest) = clean.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                clean = home.join(rest).to_string_lossy().into_owned();
            }
        }
        let candidate = if Path::new(&clean).is_absolute() {
            PathBuf::from(&clean)
        } else {
            Path::new(&target.root).join(&clean)
        };
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            return canon.to_string_lossy().into_owned();
        }
        if let Some(prospective) = otto_core::secret_paths::canonicalize_prospective(&candidate) {
            return prospective.to_string_lossy().into_owned();
        }
        #[cfg(target_os = "macos")]
        if let Some(rest) = clean.strip_prefix("/var/") {
            if Path::new("/private/var").is_dir() {
                return format!("/private/var/{rest}");
            }
        }
    }
    clean
}

/// Map a canonicalized debugger frame path back onto the workspace's logical root prefix
/// when `target.root` (or an authorized secondary root) traverses a symlink such as
/// `/var/folders/...` -> `/private/var/folders/...`, keeping `paused_file` aligned 1:1
/// with editor tab paths while `setBreakpoints` continues sending canonical DWARF paths.
pub fn display_target_path(target: &IdeTargetSpec, raw: &str) -> String {
    let canon = normalize_target_path(target, raw);
    if target.kind != IdeTargetKind::Local || canon.is_empty() {
        return canon;
    }
    for logical_root in std::iter::once(target.root.clone()).chain(target.merged_extra_roots()) {
        let logical_trimmed = if logical_root == "/" {
            "/"
        } else {
            logical_root.trim_end_matches('/')
        };
        if logical_trimmed.is_empty() {
            continue;
        }
        let Ok(canon_root_pb) = std::fs::canonicalize(logical_trimmed) else {
            continue;
        };
        let canon_root_str = canon_root_pb.to_string_lossy();
        let canon_trimmed = if canon_root_str == "/" {
            "/"
        } else {
            canon_root_str.trim_end_matches('/')
        };
        if canon_trimmed == logical_trimmed {
            continue;
        }
        if canon == canon_trimmed {
            return logical_trimmed.to_string();
        }
        if let Some(rel) = canon
            .strip_prefix(canon_trimmed)
            .and_then(|s| s.strip_prefix('/'))
        {
            return if logical_trimmed == "/" {
                format!("/{rel}")
            } else {
                format!("{logical_trimmed}/{rel}")
            };
        }
    }
    canon
}

fn is_bare_java_fqcn(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty()
        || s.contains('/')
        || s.contains('\\')
        || s.starts_with('.')
        || s.starts_with('~')
        || s.ends_with(".java")
    {
        return false;
    }
    s.split('.').all(|seg| {
        let mut chars = seg.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    })
}

/// Canonicalized workspace roots authorized for `target`.
pub(crate) fn canonical_target_roots(target: &IdeTargetSpec) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in std::iter::once(target.root.clone()).chain(target.merged_extra_roots()) {
        if r.trim().is_empty() {
            continue;
        }
        let pb = PathBuf::from(&r);
        let canon = std::fs::canonicalize(&pb)
            .or_else(|_| {
                otto_core::secret_paths::canonicalize_prospective(&pb)
                    .ok_or_else(|| std::io::Error::other("unresolved"))
            })
            .unwrap_or(pb);
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

/// Enforce non-root debug launch/attach invariants on local targets:
/// - Reject local PID attach (`DLV_ATTACH_PID`), arbitrary loopback `attach_port`, and
///   `OTTO_JAVA_DEBUG_DAP_PORT`.
/// - Confine `cwd` and `program` within authorized workspace roots and non-secret paths.
pub(crate) fn validate_non_root_debug_request(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    effective_cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    if req.target.kind != IdeTargetKind::Local || ctx.caller_is_root {
        return Ok(());
    }
    if req.launch.attach_port.is_some()
        || env.contains_key("DLV_ATTACH_PID")
        || req.launch.env.contains_key("DLV_ATTACH_PID")
        || env.contains_key("OTTO_JAVA_DEBUG_DAP_PORT")
        || req.launch.env.contains_key("OTTO_JAVA_DEBUG_DAP_PORT")
    {
        return Err(Error::Forbidden(
            "attaching to local host processes or ports requires root privileges".into(),
        ));
    }

    let allowed_roots = canonical_target_roots(&req.target);
    let check_within_roots = |label: &str, raw_path: &str| -> Result<()> {
        let norm = normalize_target_path(&req.target, raw_path);
        let p = Path::new(&norm);
        crate::target::enforce_non_root_path(p)?;
        if !allowed_roots.iter().any(|root| p.starts_with(root)) {
            return Err(Error::Forbidden(format!(
                "debug {label} '{}' is outside authorized workspace roots",
                raw_path
            )));
        }
        Ok(())
    };

    check_within_roots("cwd", effective_cwd)?;
    let prog = req.launch.program.trim();
    if !prog.is_empty() && !(req.language.eq_ignore_ascii_case("java") && is_bare_java_fqcn(prog)) {
        check_within_roots("program", prog)?;
    }
    Ok(())
}

/// Resolve the effective per-launch SDK override:
/// `req.launch.sdk_path` -> `ctx.run_config.sdk_path` -> `ctx.project_env.module_sdks`
/// longest-prefix match against `req.launch.program` and `cwd`.
pub(super) fn resolve_debug_sdk_override(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
) -> Option<String> {
    if let Some(s) = req
        .launch
        .sdk_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_string());
    }
    if let Some(s) = ctx
        .run_config
        .as_ref()
        .and_then(|rc| rc.sdk_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_string());
    }
    if let Some(pe) = &ctx.project_env {
        let mut cands = Vec::new();
        if !req.launch.program.trim().is_empty() {
            cands.push(normalize_target_path(&req.target, &req.launch.program));
        }
        if !cwd.trim().is_empty() {
            cands.push(cwd.to_string());
        }
        if let Some((_, sdk)) = crate::environments::module_sdks::match_longest_module_sdk(
            &pe.module_sdks,
            &cands,
            req.target.kind == IdeTargetKind::Local,
        ) {
            return Some(sdk);
        }
    }
    None
}

fn load_debug_dotenv_file(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    effective_cwd: &str,
    raw_env_file: &str,
) -> Option<BTreeMap<String, String>> {
    if req.target.kind != IdeTargetKind::Local {
        return None;
    }
    let trimmed = raw_env_file.trim();
    if trimmed.is_empty() || trimmed.contains("..") {
        return None;
    }
    let mut allowed_roots = canonical_target_roots(&req.target);
    if ctx.caller_is_root {
        for extra in req.project_folders.iter().chain(
            ctx.project_env
                .iter()
                .flat_map(|pe| pe.additional_roots.iter()),
        ) {
            if !extra.is_empty() {
                if let Ok(cr) = std::fs::canonicalize(extra) {
                    allowed_roots.push(cr);
                }
            }
        }
    }

    let cand = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else if !effective_cwd.is_empty() && Path::new(effective_cwd).join(trimmed).is_file() {
        Path::new(effective_cwd).join(trimmed)
    } else {
        Path::new(&req.target.root).join(trimmed)
    };
    let canon = std::fs::canonicalize(&cand).ok()?;
    if !allowed_roots.iter().any(|cr| canon.starts_with(cr)) {
        return None;
    }
    let content = std::fs::read_to_string(&canon).ok()?;
    Some(crate::environments::parse_dotenv(&content))
}

/// Merge shell/base env, project SDK paths, `.env`/secret vars, run-config overrides,
/// and launch-specific overrides while scrubbing daemon secrets for non-root callers.
pub fn build_effective_env(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> BTreeMap<String, String> {
    build_effective_env_with_manager(data_dir, req, ctx, None)
}

pub(crate) fn build_effective_env_with_manager(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    env_mgr: Option<&EnvironmentManager>,
) -> BTreeMap<String, String> {
    let raw_cwd = req
        .launch
        .cwd
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            ctx.run_config
                .as_ref()
                .and_then(|r| r.cwd.as_deref())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or(req.target.root.as_str());
    let effective_cwd = normalize_target_path(&req.target, raw_cwd);

    let inherit_shell = req.launch.inherit_shell_env.unwrap_or_else(|| {
        ctx.project_env
            .as_ref()
            .map(|p| p.inherit_shell_env)
            .unwrap_or(true)
            && ctx
                .run_config
                .as_ref()
                .map(|r| r.inherit_shell_env)
                .unwrap_or(true)
    });

    let mut env = if !inherit_shell && req.launch.inherit_shell_env == Some(false) {
        let mut base = if req.target.kind == IdeTargetKind::Local {
            if ctx.caller_is_root {
                crate::environments::shell_env::collect_process_base_env()
            } else {
                crate::environments::shell_env::collect_safe_non_root_base_env()
            }
        } else {
            BTreeMap::new()
        };
        for (k, v) in &ctx.effective_env {
            if ctx
                .project_env
                .as_ref()
                .is_some_and(|pe| pe.env_vars.iter().any(|ev| ev.key == *k))
            {
                base.insert(k.clone(), v.clone());
            }
        }
        base
    } else if ctx.caller_is_root {
        ctx.effective_env.clone()
    } else {
        let mut scrubbed = ctx.effective_env.clone();
        scrubbed.retain(|k, _| {
            let explicit = ctx
                .project_env
                .as_ref()
                .is_some_and(|pe| pe.env_vars.iter().any(|ev| ev.key == *k));
            explicit || !crate::environments::shell_env::is_sensitive_daemon_env_key(k)
        });
        scrubbed
    };

    if req.target.kind == IdeTargetKind::Local {
        if ctx.caller_is_root {
            if inherit_shell {
                if let Some(mgr) = env_mgr {
                    for (k, v) in mgr.cached_effective_env_for_local_root_as(&req.target.root, true)
                    {
                        env.entry(k).or_insert(v);
                    }
                    for (k, v) in mgr.cached_shell_env_snapshot() {
                        env.entry(k).or_insert(v);
                    }
                }
                for (k, v) in &ctx.shell_env_snapshot {
                    env.entry(k.clone()).or_insert_with(|| v.clone());
                }
                for (k, v) in std::env::vars() {
                    env.entry(k).or_insert(v);
                }
            } else {
                for (k, v) in crate::environments::shell_env::collect_process_base_env() {
                    env.entry(k).or_insert(v);
                }
            }
        } else {
            if let Some(mgr) = env_mgr {
                for (k, v) in mgr.cached_effective_env_for_local_root_as(&req.target.root, false) {
                    env.entry(k).or_insert(v);
                }
            }
            for (k, v) in crate::environments::shell_env::collect_safe_non_root_base_env() {
                env.entry(k).or_insert(v);
            }
        }
    }

    let mut effective_proj = ctx.project_env.clone().unwrap_or_default();
    if let Some(sdk_override) = resolve_debug_sdk_override(req, ctx, &effective_cwd) {
        crate::environments::module_sdks::apply_module_sdk_to_project_env(
            &mut effective_proj,
            &effective_cwd,
            &sdk_override,
            ctx.run_config.as_ref().map(|rc| rc.runner),
            Some(req.language.as_str()),
        );
    }

    let mut path_entries: Vec<String> = Vec::new();
    for (home_opt, key) in [
        (effective_proj.java_home.as_deref(), Some("JAVA_HOME")),
        (effective_proj.go_root.as_deref(), Some("GOROOT")),
        (effective_proj.node_home.as_deref(), None),
    ] {
        if let Some(h) = home_opt.filter(|s| !s.is_empty()) {
            if let Some(k) = key {
                env.insert(k.into(), h.to_string());
            }
            path_entries.push(format!("{}/bin", h.trim_end_matches('/')));
        }
    }
    if let Some(rt) = effective_proj
        .rust_toolchain
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        if rt.starts_with('/') {
            path_entries.push(format!("{}/bin", rt.trim_end_matches('/')));
        } else {
            env.insert("RUSTUP_TOOLCHAIN".into(), rt.to_string());
        }
    }
    if let Some(py) = effective_proj
        .python_interpreter
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        if let Some(parent) = Path::new(py).parent() {
            path_entries.push(parent.to_string_lossy().into_owned());
            if let Some(venv) = parent.parent() {
                if venv.join("pyvenv.cfg").is_file() || req.target.kind == IdeTargetKind::Ssh {
                    env.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                }
            }
        }
    }
    if let Some(extra) = effective_proj
        .extra_path
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        path_entries.extend(extra.split(':').filter(|s| !s.is_empty()).map(String::from));
    }
    for entry in effective_proj.env_vars.iter().filter(|e| !e.secret) {
        if let Some(val) = &entry.value {
            env.insert(entry.key.clone(), val.clone());
        }
    }

    if req.target.kind == IdeTargetKind::Local {
        let tc = data_dir.join("toolchains");
        for sub in [
            "go/bin",
            "go/current/bin",
            "dlv",
            "rust/cargo/bin",
            "rust/bin",
            "node/current/bin",
            "python/current/bin",
            "python/bin",
            "java/current/bin",
        ] {
            let p = tc.join(sub);
            if p.is_dir() {
                path_entries.push(p.to_string_lossy().into_owned());
            }
        }
        if let Some(home) = dirs::home_dir() {
            for rel in ["go/bin", ".cargo/bin", ".local/bin"] {
                let p = home.join(rel);
                if p.is_dir() {
                    path_entries.push(p.to_string_lossy().into_owned());
                }
            }
        }
        for sys_bin in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/go/bin"] {
            if Path::new(sys_bin).is_dir() {
                path_entries.push(sys_bin.to_string());
            }
        }
    }

    if let Some(existing_path) = env.get("PATH").cloned() {
        for seg in existing_path.split(':').filter(|s| !s.is_empty()) {
            if !path_entries.iter().any(|e| e == seg) {
                path_entries.push(seg.to_string());
            }
        }
    }
    if !path_entries.is_empty() {
        env.insert("PATH".into(), path_entries.join(":"));
    }

    if let Some(rc) = &ctx.run_config {
        if let Some(rc_ef) = rc.env_file.as_deref().filter(|s| !s.trim().is_empty()) {
            if let Some(parsed) = load_debug_dotenv_file(req, ctx, &effective_cwd, rc_ef) {
                env.extend(parsed);
            }
        }
        env.extend(rc.env_overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if let Some(launch_ef) = req
        .launch
        .env_file
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        if let Some(parsed) = load_debug_dotenv_file(req, ctx, &effective_cwd, launch_ef) {
            env.extend(parsed);
        }
    }
    env.extend(req.launch.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

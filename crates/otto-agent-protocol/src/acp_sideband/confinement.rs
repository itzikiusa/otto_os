//! Lexical and on-disk path confinement for ACP background task logs and
//! `agy` subagent transcripts tailed by `ottod`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::agent_runtime::RuntimeProvider;

use crate::driver::ActiveTailKind;

const CANON_ROOT_TTL: Duration = Duration::from_secs(30);
const CANON_ROOT_MAX: usize = 64;

fn canonicalize_cached(root: &Path) -> Option<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, PathBuf)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, canon)) = guard.get(root) {
            if at.elapsed() < CANON_ROOT_TTL {
                return Some(canon.clone());
            }
        }
    }
    let canon = std::fs::canonicalize(root).ok()?;
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= CANON_ROOT_MAX {
        guard.retain(|_, (at, _)| at.elapsed() < CANON_ROOT_TTL);
        if guard.len() >= CANON_ROOT_MAX {
            guard.clear();
        }
    }
    guard.insert(root.to_path_buf(), (Instant::now(), canon.clone()));
    Some(canon)
}

fn claude_task_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home_dir_path() {
        out.push(home.join(".claude").join("tasks"));
    }
    if let Ok(raw) = std::env::var("CLAUDE_CONFIG_DIR") {
        let expanded = otto_core::paths::expand_tilde(raw.trim());
        let p = PathBuf::from(expanded);
        if is_clean_absolute_path(&p) {
            let tasks = p.join("tasks");
            if !out.contains(&tasks) {
                out.push(tasks);
            }
        }
    }
    out
}

/// Lexically confine a candidate background command `.log` path before
/// recording it in sans-IO driver state. Disk-open time re-verifies symlink
/// and canonical containment via [`validate_confined_tail_path`].
pub(crate) fn validate_task_log_candidate(
    raw_path: &Path,
    session_cwd: &str,
    sideband_root: Option<&str>,
    provider: RuntimeProvider,
) -> Option<PathBuf> {
    if !is_clean_absolute_path(raw_path) || is_forbidden_tail_target(raw_path) {
        return None;
    }
    let fname = raw_path.file_name()?.to_str()?;
    otto_core::paths::safe_component(fname)?;

    let agy_root = if provider == RuntimeProvider::Agy {
        sideband_root
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(default_agy_brain_path)
    } else {
        default_agy_brain_path()
    };
    if let Some(brain) = agy_root.filter(|p| is_clean_absolute_path(p)) {
        if is_agy_task_file_under(raw_path, &brain, None) {
            return Some(raw_path.to_path_buf());
        }
    }

    for claude_tasks in claude_task_roots() {
        if let Ok(rel) = raw_path.strip_prefix(&claude_tasks) {
            if rel.components().count() == 1 {
                return Some(raw_path.to_path_buf());
            }
        }
    }

    if is_otto_temp_file(raw_path, None) {
        return Some(raw_path.to_path_buf());
    }

    let cwd_trimmed = session_cwd.trim();
    if !cwd_trimmed.is_empty() {
        let cwd_path = Path::new(cwd_trimmed);
        if is_safe_workspace_cwd_root(cwd_path)
            && raw_path.starts_with(cwd_path)
            && !otto_core::secret_paths::protected_set().in_protected_dir(raw_path)
        {
            if let Ok(rel) = raw_path.strip_prefix(cwd_path) {
                if rel.components().count() >= 1 {
                    return Some(raw_path.to_path_buf());
                }
            }
        }
    }

    None
}

pub(crate) fn validate_existing_or_pending_tail_path_scoped(
    path: &Path,
    kind: ActiveTailKind,
    allowed_cwd: Option<&Path>,
    agy_brain_dir: Option<&Path>,
    allowed_agy_conv_ids: Option<&[String]>,
    otto_session_id: Option<&str>,
) -> Option<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => validate_confined_tail_path_scoped(
            path,
            kind,
            allowed_cwd,
            agy_brain_dir,
            allowed_agy_conv_ids,
            otto_session_id,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !is_clean_absolute_path(path) || is_forbidden_tail_target(path) {
                return None;
            }
            if kind == ActiveTailKind::AgySubagentTranscript {
                let root = agy_brain_dir
                    .map(Path::to_path_buf)
                    .or_else(default_agy_brain_path)?;
                let rel = path.strip_prefix(&root).ok()?;
                let comps: Vec<&str> = rel
                    .components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => s.to_str(),
                        _ => None,
                    })
                    .collect();
                let conv_ok = allowed_agy_conv_ids
                    .is_none_or(|ids| comps.first().is_some_and(|c| ids.iter().any(|x| x == c)));
                let ok = comps.len() == 4
                    && conv_ok
                    && otto_core::paths::safe_component(comps[0]).is_some()
                    && comps[1] == ".system_generated"
                    && comps[2] == "logs"
                    && matches!(comps[3], "transcript.jsonl" | "transcript_full.jsonl");
                return ok.then(|| path.to_path_buf());
            }
            None
        }
        Err(_) => None,
    }
}

/// Canonicalize `path` on disk, reject symlinks and denied secret/`otto.db`
/// paths, and verify `canon` resides strictly inside one of the authorized
/// provider task-log or subagent transcript roots.
pub fn validate_confined_tail_path(
    path: &Path,
    kind: ActiveTailKind,
    allowed_cwd: Option<&Path>,
    agy_brain_dir: Option<&Path>,
) -> Option<PathBuf> {
    validate_confined_tail_path_scoped(path, kind, allowed_cwd, agy_brain_dir, None, None)
}

pub(crate) fn validate_confined_tail_path_scoped(
    path: &Path,
    kind: ActiveTailKind,
    allowed_cwd: Option<&Path>,
    agy_brain_dir: Option<&Path>,
    allowed_agy_conv_ids: Option<&[String]>,
    otto_session_id: Option<&str>,
) -> Option<PathBuf> {
    if !is_clean_absolute_path(path) || is_forbidden_tail_target(path) {
        return None;
    }
    let lmeta = std::fs::symlink_metadata(path).ok()?;
    if !lmeta.is_file() || lmeta.file_type().is_symlink() {
        return None;
    }
    let canon = std::fs::canonicalize(path).ok()?;
    if is_forbidden_tail_target(&canon) {
        return None;
    }
    let fname = canon.file_name()?.to_str()?;
    otto_core::paths::safe_component(fname)?;

    let agy_root = agy_brain_dir
        .map(Path::to_path_buf)
        .or_else(default_agy_brain_path);

    match kind {
        ActiveTailKind::AgySubagentTranscript => {
            let root = agy_root?;
            let canon_root = canonicalize_cached(&root)?;
            if has_symlink_between(&root, path) {
                return None;
            }
            let rel = canon.strip_prefix(&canon_root).ok()?;
            let comps: Vec<&str> = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => s.to_str(),
                    _ => None,
                })
                .collect();
            let conv_ok = allowed_agy_conv_ids
                .is_none_or(|ids| comps.first().is_some_and(|c| ids.iter().any(|x| x == c)));
            let ok = comps.len() == 4
                && conv_ok
                && otto_core::paths::safe_component(comps[0]).is_some()
                && comps[1] == ".system_generated"
                && comps[2] == "logs"
                && matches!(comps[3], "transcript.jsonl" | "transcript_full.jsonl");
            ok.then_some(canon)
        }
        ActiveTailKind::CommandLog => {
            if let Some(root) = agy_root.as_ref() {
                if let Some(canon_root) = canonicalize_cached(root) {
                    if !has_symlink_between(root, path)
                        && is_agy_task_file_under(&canon, &canon_root, allowed_agy_conv_ids)
                    {
                        return Some(canon);
                    }
                }
            }
            for claude_tasks in claude_task_roots() {
                if let Some(canon_tasks) = canonicalize_cached(&claude_tasks) {
                    if !has_symlink_between(&claude_tasks, path) {
                        if let Ok(rel) = canon.strip_prefix(&canon_tasks) {
                            if rel.components().count() == 1 {
                                return Some(canon);
                            }
                        }
                    }
                }
            }
            if is_otto_temp_file(&canon, otto_session_id) && !has_otto_temp_symlink(path) {
                return Some(canon);
            }
            if let Some(cwd) = allowed_cwd.filter(|c| is_safe_workspace_cwd_root(c)) {
                if let Some(canon_cwd) = canonicalize_cached(cwd) {
                    if is_safe_workspace_cwd_root(&canon_cwd)
                        && canon.starts_with(&canon_cwd)
                        && canon != canon_cwd
                        && !has_symlink_between(cwd, path)
                        && !otto_core::secret_paths::protected_set().in_protected_dir(&canon)
                    {
                        return Some(canon);
                    }
                }
            }
            None
        }
    }
}

pub(crate) fn is_clean_absolute_path(p: &Path) -> bool {
    if !p.is_absolute() {
        return false;
    }
    p.components().all(|c| {
        matches!(
            c,
            Component::RootDir | Component::Prefix(_) | Component::Normal(_)
        )
    })
}

pub(crate) fn is_forbidden_tail_target(p: &Path) -> bool {
    if otto_core::secret_paths::is_denied_file(p) {
        return true;
    }
    let comps: Vec<String> = p
        .components()
        .filter_map(|c| match c {
            Component::Normal(os) => Some(os.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    for (idx, lower) in comps.iter().enumerate() {
        if lower == "otto.db"
            || lower.starts_with("otto.db-")
            || lower.starts_with("otto.db.")
            || lower == ".env"
            || lower.starts_with(".env.")
            || matches!(lower.as_str(), ".ssh" | ".aws" | ".gnupg" | ".kube")
        {
            return true;
        }
        if lower == ".git" {
            if let Some(next) = comps.get(idx + 1).map(String::as_str) {
                if matches!(next, "config" | "hooks" | "credentials") {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn is_agy_task_file_under(
    path: &Path,
    brain_root: &Path,
    allowed_agy_conv_ids: Option<&[String]>,
) -> bool {
    let Ok(rel) = path.strip_prefix(brain_root) else {
        return false;
    };
    let comps: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let conv_ok = allowed_agy_conv_ids
        .is_none_or(|ids| comps.first().is_some_and(|c| ids.iter().any(|x| x == c)));
    comps.len() == 4
        && conv_ok
        && otto_core::paths::safe_component(comps[0]).is_some()
        && comps[1] == ".system_generated"
        && comps[2] == "tasks"
        && otto_core::paths::safe_component(comps[3]).is_some()
}

fn temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")];
    let env_tmp = std::env::temp_dir();
    if is_clean_absolute_path(&env_tmp) {
        roots.push(env_tmp.clone());
        if let Some(canon) = canonicalize_cached(&env_tmp) {
            roots.push(canon);
        }
    }
    roots
}

fn has_otto_temp_symlink(raw_path: &Path) -> bool {
    for root in temp_roots() {
        if raw_path.starts_with(&root) && has_symlink_between(&root, raw_path) {
            return true;
        }
    }
    false
}

pub(crate) fn is_otto_temp_file(path: &Path, otto_session_id: Option<&str>) -> bool {
    for root in temp_roots() {
        if let Ok(rel) = path.strip_prefix(&root) {
            let mut comps = rel.components();
            if let Some(Component::Normal(first)) = comps.next() {
                if first.to_str() != Some("otto-acp-bg") {
                    continue;
                }
                let Some(Component::Normal(second)) = comps.next() else {
                    continue;
                };
                let Some(sid_comp) = second.to_str().and_then(otto_core::paths::safe_component)
                else {
                    continue;
                };
                if let Some(expected_sid) = otto_session_id.filter(|s| !s.is_empty()) {
                    if sid_comp != expected_sid {
                        continue;
                    }
                }
                if comps.next().is_some() {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn is_safe_workspace_cwd_root(cwd: &Path) -> bool {
    if !is_clean_absolute_path(cwd) {
        return false;
    }
    let normal_count = cwd
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if normal_count < 1 {
        return false;
    }
    if matches!(
        cwd.to_str(),
        Some("/" | "/tmp" | "/private" | "/private/tmp" | "/Users" | "/home" | "/var" | "/etc")
    ) {
        return false;
    }
    if let Some(home) = home_dir_path() {
        if cwd == home {
            return false;
        }
    }
    true
}

pub(crate) fn default_agy_brain_path() -> Option<PathBuf> {
    home_dir_path().map(|h| h.join(".gemini").join("antigravity-cli").join("brain"))
}

pub(crate) fn home_dir_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn has_symlink_between(root: &Path, leaf: &Path) -> bool {
    let Ok(rel) = leaf.strip_prefix(root) else {
        return false;
    };
    let mut cur = root.to_path_buf();
    for comp in rel.components() {
        cur.push(comp);
        if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

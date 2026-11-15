//! Lexical and on-disk path confinement for ACP background task logs and
//! `agy` subagent transcripts tailed by `ottod`.

use std::path::{Component, Path, PathBuf};

use otto_core::agent_runtime::RuntimeProvider;

use crate::driver::ActiveTailKind;

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
        if is_agy_task_file_under(raw_path, &brain) {
            return Some(raw_path.to_path_buf());
        }
    }

    if let Some(home) = home_dir_path() {
        let claude_tasks = home.join(".claude").join("tasks");
        if let Ok(rel) = raw_path.strip_prefix(&claude_tasks) {
            if rel.components().count() == 1 {
                return Some(raw_path.to_path_buf());
            }
        }
    }

    if is_otto_temp_file(raw_path) {
        return Some(raw_path.to_path_buf());
    }

    let cwd_trimmed = session_cwd.trim();
    if !cwd_trimmed.is_empty() {
        let cwd_path = Path::new(cwd_trimmed);
        if is_safe_workspace_cwd_root(cwd_path) && raw_path.starts_with(cwd_path) {
            if let Ok(rel) = raw_path.strip_prefix(cwd_path) {
                if rel.components().count() >= 1 {
                    return Some(raw_path.to_path_buf());
                }
            }
        }
    }

    None
}

pub(crate) fn validate_existing_or_pending_tail_path(
    path: &Path,
    kind: ActiveTailKind,
    allowed_cwd: Option<&Path>,
    agy_brain_dir: Option<&Path>,
) -> Option<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => validate_confined_tail_path(path, kind, allowed_cwd, agy_brain_dir),
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
                let ok = comps.len() == 4
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
            let canon_root = std::fs::canonicalize(&root).ok()?;
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
            let ok = comps.len() == 4
                && otto_core::paths::safe_component(comps[0]).is_some()
                && comps[1] == ".system_generated"
                && comps[2] == "logs"
                && matches!(comps[3], "transcript.jsonl" | "transcript_full.jsonl");
            ok.then_some(canon)
        }
        ActiveTailKind::CommandLog => {
            if let Some(root) = agy_root.as_ref() {
                if let Ok(canon_root) = std::fs::canonicalize(root) {
                    if !has_symlink_between(root, path)
                        && is_agy_task_file_under(&canon, &canon_root)
                    {
                        return Some(canon);
                    }
                }
            }
            if let Some(home) = home_dir_path() {
                let claude_tasks = home.join(".claude").join("tasks");
                if let Ok(canon_tasks) = std::fs::canonicalize(&claude_tasks) {
                    if !has_symlink_between(&claude_tasks, path) {
                        if let Ok(rel) = canon.strip_prefix(&canon_tasks) {
                            if rel.components().count() == 1 {
                                return Some(canon);
                            }
                        }
                    }
                }
            }
            if is_otto_temp_file(&canon) {
                return Some(canon);
            }
            if let Some(cwd) = allowed_cwd.filter(|c| is_safe_workspace_cwd_root(c)) {
                if let Ok(canon_cwd) = std::fs::canonicalize(cwd) {
                    if is_safe_workspace_cwd_root(&canon_cwd)
                        && canon.starts_with(&canon_cwd)
                        && canon != canon_cwd
                        && !has_symlink_between(cwd, path)
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
    for comp in p.components() {
        if let Component::Normal(os) = comp {
            let s = os.to_string_lossy();
            let lower = s.to_ascii_lowercase();
            if lower == "otto.db"
                || lower.starts_with("otto.db-")
                || lower.starts_with("otto.db.")
                || matches!(lower.as_str(), ".ssh" | ".aws" | ".gnupg" | ".kube")
            {
                return true;
            }
        }
    }
    false
}

pub(crate) fn is_agy_task_file_under(path: &Path, brain_root: &Path) -> bool {
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
    comps.len() == 4
        && otto_core::paths::safe_component(comps[0]).is_some()
        && comps[1] == ".system_generated"
        && comps[2] == "tasks"
        && otto_core::paths::safe_component(comps[3]).is_some()
}

pub(crate) fn is_otto_temp_file(path: &Path) -> bool {
    let mut roots = vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")];
    let env_tmp = std::env::temp_dir();
    if is_clean_absolute_path(&env_tmp) {
        roots.push(env_tmp.clone());
        if let Ok(canon) = std::fs::canonicalize(&env_tmp) {
            roots.push(canon);
        }
    }
    for root in roots {
        if let Ok(rel) = path.strip_prefix(&root) {
            let mut comps = rel.components();
            if let Some(Component::Normal(first)) = comps.next() {
                if first.to_string_lossy().starts_with("otto-") {
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

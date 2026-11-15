//! Ultra-fast (`< 15ms`) `@` file & path completion (`POST /ide/fs/complete`)
//! supporting workspace fuzzy search, aliases, `@~/...`, `@/...`, `@../...`,
//! and `@./...` with a bounded short-TTL `git ls-files` cache and strict latency caps.

#![allow(clippy::disallowed_methods)] // synchronous @ completion invoked inside spawn_blocking

#[path = "complete_query.rs"]
mod complete_query;
#[path = "complete_scan.rs"]
mod complete_scan;

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crate::target::enforce_non_root_path;
use crate::types::{IdeFsCompleteReq, IdeFsCompleteResp};

use self::complete_query::resolve_scoped_query;
pub(super) use self::complete_query::TokenFormat;
use self::complete_scan::{complete_in_dir, search_across_roots};

const BLOCKED_ABS_PREFIXES: &[&str] = &["/proc", "/dev", "/System/Volumes"];

pub(super) fn expand_tilde(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

pub(super) fn clean_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn is_blocked_system_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    BLOCKED_ABS_PREFIXES
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}/")))
}

fn has_secret_dir_component(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(".ssh" | ".aws" | ".gnupg" | ".kube" | ".codex" | ".claude" | ".gemini")
        )
    })
}

fn is_allowed_lexical_or_canonical(path: &Path, is_root_user: bool) -> bool {
    if is_blocked_system_path(path) || otto_core::secret_paths::is_denied_dir(path) {
        return false;
    }
    is_root_user || (!has_secret_dir_component(path) && enforce_non_root_path(path).is_ok())
}

pub(super) fn is_within_workspace_roots(canon: &Path, workspace_roots: &[PathBuf]) -> bool {
    workspace_roots.iter().any(|r| canon.starts_with(r))
}

fn canonical_allowed_dir(path: &Path, is_root_user: bool) -> Option<PathBuf> {
    if !is_allowed_lexical_or_canonical(path, is_root_user) {
        return None;
    }
    let canon = fs::canonicalize(path).ok()?;
    if !canon.is_dir() || !is_allowed_lexical_or_canonical(&canon, is_root_user) {
        return None;
    }
    Some(canon)
}

pub(super) fn is_allowed_entry_in_canonical_dir(
    entry_path: &Path,
    ft: &fs::FileType,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
) -> bool {
    if !is_allowed_lexical_or_canonical(entry_path, is_root_user) {
        return false;
    }
    if ft.is_symlink() {
        let Ok(canon) = fs::canonicalize(entry_path) else {
            return false;
        };
        if !is_allowed_lexical_or_canonical(&canon, is_root_user) {
            return false;
        }
        if !is_root_user && !is_within_workspace_roots(&canon, workspace_roots) {
            return false;
        }
    }
    true
}

pub(super) fn is_allowed_path(
    path: &Path,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
) -> bool {
    if !is_allowed_lexical_or_canonical(path, is_root_user) {
        return false;
    }
    let Ok(canon) = fs::canonicalize(path) else {
        return false;
    };
    if !is_allowed_lexical_or_canonical(&canon, is_root_user) {
        return false;
    }
    is_root_user || is_within_workspace_roots(&canon, workspace_roots)
}

pub(super) fn score_candidate(
    name: &str,
    rel_path: &str,
    query_lower: &str,
    is_dir: bool,
) -> Option<i32> {
    if query_lower.is_empty() {
        return Some(if is_dir { 520 } else { 500 });
    }
    let name_lower = name.to_ascii_lowercase();
    let rel_lower = rel_path.to_ascii_lowercase();
    let dir_bonus = if is_dir { 15 } else { 0 };
    if name_lower == query_lower {
        return Some(1000 + dir_bonus);
    }
    if name_lower.starts_with(query_lower) {
        return Some(820 + dir_bonus - (name.len() as i32).min(40));
    }
    if name_lower.contains(&format!("_{query_lower}"))
        || name_lower.contains(&format!("-{query_lower}"))
        || name_lower.contains(&format!(".{query_lower}"))
    {
        return Some(680 + dir_bonus);
    }
    if name_lower.contains(query_lower) {
        return Some(540 + dir_bonus - (name.len() as i32).min(40));
    }
    if rel_lower.contains(query_lower) {
        return Some(360 + dir_bonus - (rel_path.len() as i32).min(60));
    }
    if query_lower.len() >= 2 && is_subsequence(query_lower.as_bytes(), name_lower.as_bytes()) {
        return Some(180 + dir_bonus - (name.len() as i32).min(40));
    }
    None
}

fn is_subsequence(needle: &[u8], haystack: &[u8]) -> bool {
    let mut ni = 0;
    for &b in haystack {
        if ni < needle.len() && b == needle[ni] {
            ni += 1;
        }
    }
    ni == needle.len()
}

/// Execute `POST /ide/fs/complete` with strict latency bounds and workspace confinement.
pub fn complete_fs_paths(req: &IdeFsCompleteReq, is_root_user: bool) -> IdeFsCompleteResp {
    complete_fs_paths_for_roots(req, is_root_user, None)
}

/// Execute `POST /ide/fs/complete`, restricting non-root callers to `authorized_roots`
/// when provided by the HTTP RBAC layer.
pub fn complete_fs_paths_for_roots(
    req: &IdeFsCompleteReq,
    is_root_user: bool,
    authorized_roots: Option<&[PathBuf]>,
) -> IdeFsCompleteResp {
    let started = Instant::now();
    let limit = req.limit.clamp(1, 100);
    let home = dirs::home_dir();
    let raw_q = req
        .query
        .trim()
        .strip_prefix('@')
        .unwrap_or(req.query.trim());

    let cwd_path = req
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(expand_tilde)
        .or_else(|| req.roots.first().map(|r| expand_tilde(r.trim())))
        .or_else(|| {
            req.target
                .as_ref()
                .map(|t| t.root.trim())
                .filter(|s| !s.is_empty())
                .map(expand_tilde)
        });

    let mut workspace_roots: Vec<PathBuf> = Vec::new();
    let mut push_root = |cand: &Path| {
        if let Some(canon) = canonical_allowed_dir(cand, is_root_user) {
            if !is_root_user {
                if let Some(auth) = authorized_roots {
                    if !is_within_workspace_roots(&canon, auth) {
                        return;
                    }
                }
            }
            if !workspace_roots.contains(&canon) {
                workspace_roots.push(canon);
            }
        }
    };
    if let Some(c) = &cwd_path {
        push_root(c);
    }
    for r in &req.roots {
        push_root(&clean_lexical(&expand_tilde(r.trim())));
    }
    if let Some(t) = &req.target {
        if !t.root.trim().is_empty() {
            push_root(&clean_lexical(&expand_tilde(t.root.trim())));
        }
        for er in t.merged_extra_roots() {
            push_root(&clean_lexical(&expand_tilde(er.trim())));
        }
    }
    if authorized_roots.is_some() {
        for alias_val in req.aliases.values() {
            let trimmed = alias_val.trim();
            if !trimmed.is_empty() {
                push_root(&clean_lexical(&expand_tilde(trimmed)));
            }
        }
    }

    if !is_root_user && workspace_roots.is_empty() {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    if !raw_q.starts_with('~')
        && !raw_q.starts_with('/')
        && !raw_q.starts_with("./")
        && !raw_q.starts_with("../")
        && raw_q != "."
        && raw_q != ".."
        && !raw_q.contains('/')
    {
        let base_dir = workspace_roots
            .first()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let multi = workspace_roots.len() > 1;
        let items = search_across_roots(
            &workspace_roots,
            raw_q,
            multi,
            limit,
            is_root_user,
            home.as_deref(),
        );
        return IdeFsCompleteResp {
            items,
            base_dir,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    let Some((dir_prefix, leaf_query, fmt)) = resolve_scoped_query(
        raw_q,
        cwd_path.as_deref(),
        &workspace_roots,
        &req.aliases,
        is_root_user,
        home.as_deref(),
    ) else {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    };

    let Some(canon_dir) = canonical_allowed_dir(&dir_prefix, is_root_user) else {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    };

    if !is_root_user && !is_within_workspace_roots(&canon_dir, &workspace_roots) {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    let owning_root = workspace_roots
        .iter()
        .find(|r| canon_dir.starts_with(r))
        .unwrap_or(&canon_dir);
    let items = complete_in_dir(
        &dir_prefix,
        owning_root,
        &leaf_query,
        &fmt,
        limit,
        is_root_user,
        &workspace_roots,
        home.as_deref(),
    );
    IdeFsCompleteResp {
        items,
        base_dir: dir_prefix.to_string_lossy().into_owned(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

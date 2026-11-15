//! Directory, `git ls-files`, and bounded recursive scanning helpers for
//! [`super::complete_fs_paths`].

#![allow(clippy::disallowed_methods)] // synchronous @ completion invoked inside spawn_blocking

#[path = "complete_git.rs"]
mod complete_git;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::types::IdeFsCompleteItem;

use self::complete_git::{collect_git_ls_files, has_fresh_git_ls_cache};
use super::{
    clean_lexical, is_allowed_entry_in_canonical_dir, is_within_workspace_roots, score_candidate,
    TokenFormat,
};

const ALWAYS_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "vendor",
    ".m2",
    ".gradle",
    ".next",
    ".svelte-kit",
    "__pycache__",
];

const SHALLOW_MAX_DEPTH: usize = 5;
const SHALLOW_MAX_VISITED: usize = 8_000;
const SHALLOW_DEADLINE_MS: u64 = 25;

fn should_skip_complete_dir(dir_path: &Path, name: &str, owning_root: &Path) -> bool {
    if ALWAYS_SKIP_DIRS.contains(&name) {
        return true;
    }
    if matches!(name, "target" | "build" | "dist") {
        return crate::target::search::is_generated_build_dir(dir_path, name, owning_root);
    }
    false
}

fn find_git_root(
    dir: &Path,
    home: Option<&Path>,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if Some(d) == home || d.parent().is_none() {
            break;
        }
        if !is_root_user && !is_within_workspace_roots(d, workspace_roots) {
            break;
        }
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_in_dir(
    dir_prefix: &Path,
    owning_root: &Path,
    leaf_query: &str,
    fmt: &TokenFormat,
    limit: usize,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
    home: Option<&Path>,
) -> Vec<IdeFsCompleteItem> {
    let q_lower = leaf_query.trim().to_ascii_lowercase();
    let mut scored: Vec<(i32, PathBuf, bool)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if let Ok(rd) = fs::read_dir(dir_prefix) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (name.starts_with('.') || name.starts_with("__debug_bin"))
                && !q_lower.starts_with('.')
            {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user, workspace_roots) {
                continue;
            }
            let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
            if is_dir && should_skip_complete_dir(&path, &name, owning_root) {
                continue;
            }
            if let Some(sc) = score_candidate(&name, &name, &q_lower, is_dir) {
                seen.insert(path.clone());
                scored.push((sc + 25, path, is_dir));
            }
        }
    }

    let home_canon = home.and_then(|h| fs::canonicalize(h).ok());
    let dir_canon = fs::canonicalize(dir_prefix).ok();
    let is_home_or_root = dir_prefix.parent().is_none()
        || home.is_some_and(|h| clean_lexical(h) == dir_prefix)
        || (home_canon.is_some() && home_canon == dir_canon);
    if !is_home_or_root && !q_lower.is_empty() {
        if let Some(repo_root) = find_git_root(dir_prefix, home, is_root_user, workspace_roots) {
            collect_git_ls_files(
                &repo_root,
                dir_prefix,
                &q_lower,
                limit,
                is_root_user,
                workspace_roots,
                &mut seen,
                &mut scored,
            );
        } else if dir_prefix.components().count() >= 3 {
            let deadline = Instant::now() + Duration::from_millis(SHALLOW_DEADLINE_MS);
            let mut visited = 0usize;
            walk_bounded(
                dir_prefix,
                owning_root,
                dir_prefix,
                1,
                &q_lower,
                is_root_user,
                workspace_roots,
                deadline,
                &mut visited,
                &mut seen,
                &mut scored,
            );
        }
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.as_os_str().len().cmp(&b.1.as_os_str().len()))
            .then_with(|| a.1.cmp(&b.1))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, path, is_dir)| build_item(path, is_dir, fmt, home))
        .collect()
}

pub(super) fn search_across_roots(
    roots: &[PathBuf],
    raw_q: &str,
    multi_root: bool,
    limit: usize,
    is_root_user: bool,
    home: Option<&Path>,
) -> Vec<IdeFsCompleteItem> {
    if roots.is_empty() {
        return Vec::new();
    }
    let capped_roots = &roots[..roots.len().min(36)];
    let all_cached = capped_roots.len() <= 2
        || capped_roots.iter().all(|r| {
            find_git_root(r, home, is_root_user, roots)
                .and_then(|gr| fs::canonicalize(gr).ok())
                .is_some_and(|canon| has_fresh_git_ls_cache(&canon))
        });

    let mut merged: Vec<(i32, IdeFsCompleteItem)> = if all_cached {
        capped_roots
            .iter()
            .flat_map(|root| {
                let fmt = TokenFormat::Workspace {
                    multi_root,
                    root: root.clone(),
                };
                complete_in_dir_scored(root, raw_q, &fmt, limit, is_root_user, roots, home)
            })
            .collect()
    } else {
        let worker_count = capped_roots.len().clamp(1, 4);
        let chunk_size = capped_roots.len().div_ceil(worker_count);
        std::thread::scope(|s| {
            let handles: Vec<_> = capped_roots
                .chunks(chunk_size)
                .map(|chunk| {
                    s.spawn(move || {
                        let mut local = Vec::new();
                        for root in chunk {
                            let fmt = TokenFormat::Workspace {
                                multi_root,
                                root: root.clone(),
                            };
                            local.extend(complete_in_dir_scored(
                                root,
                                raw_q,
                                &fmt,
                                limit,
                                is_root_user,
                                roots,
                                home,
                            ));
                        }
                        local
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .flatten()
                .collect()
        })
    };

    merged.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.display.len().cmp(&b.1.display.len()))
            .then_with(|| a.1.display.cmp(&b.1.display))
    });
    let mut seen_paths = HashSet::new();
    merged
        .into_iter()
        .filter(|(_, it)| seen_paths.insert(it.path.clone()))
        .take(limit)
        .map(|(_, it)| it)
        .collect()
}

fn complete_in_dir_scored(
    root: &Path,
    raw_q: &str,
    fmt: &TokenFormat,
    limit: usize,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
    home: Option<&Path>,
) -> Vec<(i32, IdeFsCompleteItem)> {
    let q_lower = raw_q.trim().to_ascii_lowercase();
    let mut scored: Vec<(i32, PathBuf, bool)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if let Ok(rd) = fs::read_dir(root) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (name.starts_with('.') || name.starts_with("__debug_bin"))
                && !q_lower.starts_with('.')
            {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user, workspace_roots) {
                continue;
            }
            let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
            if is_dir && should_skip_complete_dir(&path, &name, root) {
                continue;
            }
            if let Some(sc) = score_candidate(&name, &name, &q_lower, is_dir) {
                seen.insert(path.clone());
                scored.push((sc + 20, path, is_dir));
            }
        }
    }

    if let Some(repo_root) = find_git_root(root, home, is_root_user, workspace_roots) {
        collect_git_ls_files(
            &repo_root,
            root,
            &q_lower,
            limit,
            is_root_user,
            workspace_roots,
            &mut seen,
            &mut scored,
        );
    } else if root.components().count() >= 3 {
        let deadline = Instant::now() + Duration::from_millis(SHALLOW_DEADLINE_MS);
        let mut visited = 0usize;
        walk_bounded(
            root,
            root,
            root,
            1,
            &q_lower,
            is_root_user,
            workspace_roots,
            deadline,
            &mut visited,
            &mut seen,
            &mut scored,
        );
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(sc, path, is_dir)| (sc, build_item(path, is_dir, fmt, home)))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn walk_bounded(
    base: &Path,
    owning_root: &Path,
    cur: &Path,
    depth: usize,
    q_lower: &str,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
    deadline: Instant,
    visited: &mut usize,
    seen: &mut HashSet<PathBuf>,
    scored: &mut Vec<(i32, PathBuf, bool)>,
) {
    if depth > SHALLOW_MAX_DEPTH || *visited >= SHALLOW_MAX_VISITED || Instant::now() >= deadline {
        return;
    }
    let Ok(rd) = fs::read_dir(cur) else { return };
    for entry in rd.flatten() {
        *visited += 1;
        if *visited >= SHALLOW_MAX_VISITED || Instant::now() >= deadline {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.starts_with("__debug_bin") {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user, workspace_roots) {
            continue;
        }
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if is_dir && should_skip_complete_dir(&path, &name, owning_root) {
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());
        if !seen.contains(&path) {
            if let Some(sc) = score_candidate(&name, &rel, q_lower, is_dir) {
                seen.insert(path.clone());
                scored.push((sc, path.clone(), is_dir));
            }
        }
        if ft.is_dir() && depth < SHALLOW_MAX_DEPTH {
            walk_bounded(
                base,
                owning_root,
                &path,
                depth + 1,
                q_lower,
                is_root_user,
                workspace_roots,
                deadline,
                visited,
                seen,
                scored,
            );
        }
    }
}

fn build_item(
    path: PathBuf,
    is_dir: bool,
    fmt: &TokenFormat,
    home: Option<&Path>,
) -> IdeFsCompleteItem {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let (display, insert_token) = fmt.render(&path, is_dir, home);
    IdeFsCompleteItem {
        path: path.to_string_lossy().into_owned(),
        display,
        insert_token,
        name,
        kind: if is_dir { "dir".into() } else { "file".into() },
        is_dir,
    }
}

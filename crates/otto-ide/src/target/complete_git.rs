//! Bounded `git ls-files` cache and scoring helper for [`super::search_across_roots`].

#![allow(clippy::disallowed_methods)] // synchronous @ completion invoked inside spawn_blocking

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;

use super::super::{is_allowed_path, score_candidate};

const GIT_LS_CACHE_TTL: Duration = Duration::from_secs(8);
const GIT_LS_CACHE_CAP: usize = 64;

struct CachedGitFiles {
    fetched_at: Instant,
    index_mtime: Option<SystemTime>,
    files: Arc<Vec<String>>,
}

fn git_ls_cache() -> &'static DashMap<PathBuf, CachedGitFiles> {
    static CACHE: OnceLock<DashMap<PathBuf, CachedGitFiles>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

pub(super) fn has_fresh_git_ls_cache(canon_repo: &Path) -> bool {
    git_ls_cache()
        .get(canon_repo)
        .is_some_and(|c| c.fetched_at.elapsed() < GIT_LS_CACHE_TTL)
}

fn load_or_query_git_ls_files(repo_root: &Path) -> Option<(PathBuf, Arc<Vec<String>>)> {
    let canon_repo = fs::canonicalize(repo_root).ok()?;
    let index_mtime = fs::metadata(canon_repo.join(".git/index"))
        .and_then(|m| m.modified())
        .ok();
    let cache = git_ls_cache();
    if let Some(hit) = cache.get(&canon_repo) {
        if hit.fetched_at.elapsed() < GIT_LS_CACHE_TTL && hit.index_mtime == index_mtime {
            return Some((canon_repo, Arc::clone(&hit.files)));
        }
    }

    let mut cmd = otto_git_cmd::hardened_std_command();
    cmd.arg("-C").arg(&canon_repo).args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ]);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let files: Vec<String> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|raw| !raw.is_empty())
        .filter_map(|raw| std::str::from_utf8(raw).ok().map(str::to_string))
        .collect();
    let arc_files = Arc::new(files);

    cache.retain(|_, v| v.fetched_at.elapsed() < GIT_LS_CACHE_TTL);
    if cache.len() >= GIT_LS_CACHE_CAP {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|e| e.value().fetched_at)
            .map(|e| e.key().clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(
        canon_repo.clone(),
        CachedGitFiles {
            fetched_at: Instant::now(),
            index_mtime,
            files: Arc::clone(&arc_files),
        },
    );
    Some((canon_repo, arc_files))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_git_ls_files(
    repo_root: &Path,
    dir_prefix: &Path,
    q_lower: &str,
    limit: usize,
    is_root_user: bool,
    workspace_roots: &[PathBuf],
    seen: &mut HashSet<PathBuf>,
    scored: &mut Vec<(i32, PathBuf, bool)>,
) {
    let Some((canon_repo, rel_files)) = load_or_query_git_ls_files(repo_root) else {
        return;
    };
    let canon_prefix = fs::canonicalize(dir_prefix).unwrap_or_else(|_| dir_prefix.to_path_buf());
    let sub_prefix = canon_prefix
        .strip_prefix(&canon_repo)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| format!("{}/", p.to_string_lossy().replace('\\', "/")));

    let mut prelim: Vec<(i32, &str)> = Vec::new();
    for rel_repo in rel_files.iter() {
        if let Some(sp) = &sub_prefix {
            if !rel_repo.starts_with(sp.as_str()) {
                continue;
            }
        }
        if q_lower.is_empty() && prelim.len() >= 120 {
            break;
        }
        let name = rel_repo.rsplit('/').next().unwrap_or(rel_repo.as_str());
        if let Some(sc) = score_candidate(name, rel_repo, q_lower, false) {
            prelim.push((sc, rel_repo.as_str()));
        }
    }
    prelim.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.len().cmp(&b.1.len()))
            .then_with(|| a.1.cmp(b.1))
    });

    let cap = limit.max(80);
    let mut accepted = 0usize;
    for (sc, rel_repo) in prelim {
        if accepted >= cap {
            break;
        }
        let lexical_full = if dir_prefix == canon_prefix {
            canon_repo.join(rel_repo)
        } else {
            let rel_under_prefix = match &sub_prefix {
                Some(sp) => rel_repo.strip_prefix(sp.as_str()).unwrap_or(rel_repo),
                None => rel_repo,
            };
            dir_prefix.join(rel_under_prefix)
        };
        if seen.contains(&lexical_full) {
            continue;
        }
        if !is_allowed_path(&lexical_full, is_root_user, workspace_roots) {
            continue;
        }
        seen.insert(lexical_full.clone());
        scored.push((sc, lexical_full, false));
        accepted += 1;
    }
}

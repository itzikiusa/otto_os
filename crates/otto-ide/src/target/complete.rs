//! Ultra-fast (`< 15ms`) `@` file & path completion (`POST /ide/fs/complete`)
//! supporting workspace fuzzy search, aliases, `@~/...`, `@/...`, `@../...`,
//! and `@./...` with a bounded short-TTL `git ls-files` cache and strict latency caps.

#![allow(clippy::disallowed_methods)] // synchronous @ completion invoked inside spawn_blocking

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;

use crate::target::enforce_non_root_path;

fn expand_tilde(raw: &str) -> PathBuf {
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
use crate::types::{IdeFsCompleteItem, IdeFsCompleteReq, IdeFsCompleteResp};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "vendor",
    ".m2",
    ".gradle",
    ".next",
    ".svelte-kit",
    "__pycache__",
];

const BLOCKED_ABS_PREFIXES: &[&str] = &["/proc", "/dev", "/System/Volumes"];
const SHALLOW_MAX_DEPTH: usize = 5;
const SHALLOW_MAX_VISITED: usize = 8_000;
const SHALLOW_DEADLINE_MS: u64 = 25;
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

#[derive(Clone)]
enum TokenFormat {
    Home,
    Absolute,
    Relative { prefix: String, base: PathBuf },
    Workspace { multi_root: bool, root: PathBuf },
}

impl TokenFormat {
    fn render(&self, abs_path: &Path, is_dir: bool, home: Option<&Path>) -> (String, String) {
        let trail = if is_dir { "/" } else { "" };
        match self {
            Self::Home => {
                let s = shorten_home(abs_path, home);
                let tok = format!("{}{}", s.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Absolute => {
                let s = abs_path.to_string_lossy();
                let tok = format!("{}{}", s.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Relative { prefix, base } => {
                let rel = abs_path
                    .strip_prefix(base)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned());
                let joined = if prefix.is_empty() {
                    rel
                } else {
                    format!("{}{}", prefix, rel.trim_start_matches('/'))
                };
                let tok = format!("{}{}", joined.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Workspace { multi_root, root } => {
                let rel = abs_path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| shorten_home(abs_path, home));
                let base_tok = if *multi_root {
                    let rname = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if rname.is_empty() || rel.is_empty() {
                        rel
                    } else {
                        format!("{rname}/{rel}")
                    }
                } else {
                    rel
                };
                let tok = format!("{}{}", base_tok.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
        }
    }
}

fn shorten_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(h) = home {
        if path == h {
            return "~".to_string();
        }
        if let Ok(rel) = path.strip_prefix(h) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.to_string_lossy().into_owned()
}

fn clean_lexical(path: &Path) -> PathBuf {
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

fn is_allowed_entry_in_canonical_dir(
    entry_path: &Path,
    ft: &fs::FileType,
    is_root_user: bool,
) -> bool {
    if !is_allowed_lexical_or_canonical(entry_path, is_root_user) {
        return false;
    }
    if ft.is_symlink() {
        let Ok(canon) = fs::canonicalize(entry_path) else {
            return false;
        };
        return is_allowed_lexical_or_canonical(&canon, is_root_user);
    }
    true
}

fn is_allowed_path(path: &Path, is_root_user: bool) -> bool {
    if !is_allowed_lexical_or_canonical(path, is_root_user) {
        return false;
    }
    let Ok(canon) = fs::canonicalize(path) else {
        return false;
    };
    is_allowed_lexical_or_canonical(&canon, is_root_user)
}

fn score_candidate(name: &str, rel_path: &str, query_lower: &str, is_dir: bool) -> Option<i32> {
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

/// Execute `POST /ide/fs/complete` with strict latency bounds and zero persistent caches.
pub fn complete_fs_paths(req: &IdeFsCompleteReq, is_root_user: bool) -> IdeFsCompleteResp {
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
    if let Some(c) = &cwd_path {
        if let Some(canon) = canonical_allowed_dir(c, is_root_user) {
            workspace_roots.push(canon);
        }
    }
    for r in &req.roots {
        let p = clean_lexical(&expand_tilde(r.trim()));
        if let Some(canon) = canonical_allowed_dir(&p, is_root_user) {
            if !workspace_roots.contains(&canon) {
                workspace_roots.push(canon);
            }
        }
    }
    if let Some(t) = &req.target {
        if !t.root.trim().is_empty() {
            let p = clean_lexical(&expand_tilde(t.root.trim()));
            if let Some(canon) = canonical_allowed_dir(&p, is_root_user) {
                if !workspace_roots.contains(&canon) {
                    workspace_roots.push(canon);
                }
            }
        }
        for er in t.merged_extra_roots() {
            let p = clean_lexical(&expand_tilde(er.trim()));
            if let Some(canon) = canonical_allowed_dir(&p, is_root_user) {
                if !workspace_roots.contains(&canon) {
                    workspace_roots.push(canon);
                }
            }
        }
    }

    // 1. Plain workspace query (no path prefix like `~`, `/`, `./`, `../`, or `seg/rest`)
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

    // 2. Scoped path / alias completion
    let Some((dir_prefix, leaf_query, fmt)) = resolve_scoped_query(
        raw_q,
        cwd_path.as_deref(),
        &workspace_roots,
        &req.aliases,
        home.as_deref(),
    ) else {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    };

    let Some(_canon_dir) = canonical_allowed_dir(&dir_prefix, is_root_user) else {
        return IdeFsCompleteResp {
            items: Vec::new(),
            base_dir: dir_prefix.to_string_lossy().into_owned(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    };

    let items = complete_in_dir(
        &dir_prefix,
        &leaf_query,
        &fmt,
        limit,
        is_root_user,
        home.as_deref(),
    );
    IdeFsCompleteResp {
        items,
        base_dir: dir_prefix.to_string_lossy().into_owned(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn resolve_scoped_query(
    raw_q: &str,
    cwd: Option<&Path>,
    roots: &[PathBuf],
    aliases: &BTreeMap<String, String>,
    home: Option<&Path>,
) -> Option<(PathBuf, String, TokenFormat)> {
    let trailing_slash = raw_q.ends_with('/');
    if raw_q == "~" {
        let h = home?;
        return Some((h.to_path_buf(), String::new(), TokenFormat::Home));
    }
    if let Some(rest) = raw_q.strip_prefix("~/") {
        let h = home?;
        let (dir, leaf) = split_existing_dir(&h.join(rest), h, trailing_slash);
        return Some((dir, leaf, TokenFormat::Home));
    }
    if raw_q.starts_with('/') {
        let target = clean_lexical(Path::new(raw_q));
        let root = Path::new("/");
        let (dir, leaf) = split_existing_dir(&target, root, trailing_slash);
        return Some((dir, leaf, TokenFormat::Absolute));
    }
    if raw_q == "." || raw_q == ".." || raw_q.starts_with("./") || raw_q.starts_with("../") {
        let anchor = cwd.or_else(|| roots.first().map(PathBuf::as_path))?;
        let (rel_dir_tok, leaf_part) = if raw_q == "." {
            ("./".to_string(), "")
        } else if raw_q == ".." {
            ("../".to_string(), "")
        } else if trailing_slash {
            (raw_q.to_string(), "")
        } else if let Some((d, l)) = raw_q.rsplit_once('/') {
            (format!("{d}/"), l)
        } else {
            (String::new(), raw_q)
        };
        let resolved_base = clean_lexical(&anchor.join(&rel_dir_tok));
        return Some((
            resolved_base.clone(),
            leaf_part.to_string(),
            TokenFormat::Relative {
                prefix: rel_dir_tok,
                base: resolved_base,
            },
        ));
    }
    // `<first_seg>/<rest>` alias / repo basename / `$HOME/<first_seg>` / `<cwd>/<first_seg>`
    let (first_seg, rest) = raw_q.split_once('/')?;
    let base_repo = aliases
        .get(first_seg)
        .or_else(|| {
            aliases
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(first_seg))
                .map(|(_, v)| v)
        })
        .map(|s| clean_lexical(&expand_tilde(s)))
        .or_else(|| {
            roots
                .iter()
                .find(|r| {
                    r.file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(first_seg))
                })
                .cloned()
        })
        .or_else(|| {
            let cand = cwd
                .or_else(|| roots.first().map(PathBuf::as_path))?
                .join(first_seg);
            cand.is_dir().then(|| clean_lexical(&cand))
        })
        .or_else(|| {
            let cand = home?.join(first_seg);
            cand.is_dir().then_some(cand)
        })?;

    let full = if rest.is_empty() {
        base_repo.clone()
    } else {
        base_repo.join(rest)
    };
    let (dir, leaf) = split_existing_dir(&full, &base_repo, trailing_slash || rest.is_empty());
    let fmt = if home.is_some_and(|h| dir.starts_with(h)) {
        TokenFormat::Home
    } else {
        TokenFormat::Absolute
    };
    Some((dir, leaf, fmt))
}

fn split_existing_dir(full: &Path, floor: &Path, trailing_slash: bool) -> (PathBuf, String) {
    if trailing_slash && full.is_dir() {
        return (clean_lexical(full), String::new());
    }
    if let Some(parent) = full.parent() {
        let leaf = full
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent.is_dir() {
            return (clean_lexical(parent), leaf);
        }
        // Walk up to longest existing directory prefix so `@~/go_admission/sub/query` searches inside `go_admission`.
        let mut cur = parent;
        while !cur.is_dir() && cur != floor {
            let Some(p) = cur.parent() else {
                break;
            };
            cur = p;
        }
        if cur.is_dir() {
            let rem = full
                .strip_prefix(cur)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or(leaf);
            return (clean_lexical(cur), rem);
        }
    }
    (clean_lexical(floor), String::new())
}

fn find_git_root(dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if Some(d) == home || d.parent().is_none() {
            break;
        }
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

fn complete_in_dir(
    dir_prefix: &Path,
    leaf_query: &str,
    fmt: &TokenFormat,
    limit: usize,
    is_root_user: bool,
    home: Option<&Path>,
) -> Vec<IdeFsCompleteItem> {
    let q_lower = leaf_query.trim().to_ascii_lowercase();
    let mut scored: Vec<(i32, PathBuf, bool)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    // Stage 1: Instant single `read_dir(dir_prefix)` (`< 0.3ms`)
    if let Ok(rd) = fs::read_dir(dir_prefix) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !q_lower.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user) {
                continue;
            }
            let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if let Some(sc) = score_candidate(&name, &name, &q_lower, is_dir) {
                seen.insert(path.clone());
                scored.push((sc + 25, path, is_dir));
            }
        }
    }

    // Stage 2: Deep search ONLY if `dir_prefix` is NOT `$HOME` and NOT `/` AND `q_lower` is non-empty!
    let home_canon = home.and_then(|h| fs::canonicalize(h).ok());
    let dir_canon = fs::canonicalize(dir_prefix).ok();
    let is_home_or_root = dir_prefix.parent().is_none()
        || home.is_some_and(|h| clean_lexical(h) == dir_prefix)
        || (home_canon.is_some() && home_canon == dir_canon);
    if !is_home_or_root && !q_lower.is_empty() {
        if let Some(repo_root) = find_git_root(dir_prefix, home) {
            collect_git_ls_files(
                &repo_root,
                dir_prefix,
                &q_lower,
                limit,
                is_root_user,
                &mut seen,
                &mut scored,
            );
        } else if dir_prefix.components().count() >= 3 {
            let deadline = Instant::now() + Duration::from_millis(SHALLOW_DEADLINE_MS);
            let mut visited = 0usize;
            walk_bounded(
                dir_prefix,
                dir_prefix,
                1,
                &q_lower,
                is_root_user,
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

fn search_across_roots(
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
    let per_root_limit = limit;
    let all_cached = capped_roots.len() <= 2
        || capped_roots.iter().all(|r| {
            find_git_root(r, home)
                .and_then(|gr| fs::canonicalize(gr).ok())
                .is_some_and(|canon| {
                    git_ls_cache()
                        .get(&canon)
                        .is_some_and(|c| c.fetched_at.elapsed() < GIT_LS_CACHE_TTL)
                })
        });

    let mut merged: Vec<(i32, IdeFsCompleteItem)> = if all_cached {
        capped_roots
            .iter()
            .flat_map(|root| {
                let fmt = TokenFormat::Workspace {
                    multi_root,
                    root: root.clone(),
                };
                complete_in_dir_scored(root, raw_q, &fmt, per_root_limit, is_root_user, home)
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
                                per_root_limit,
                                is_root_user,
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
    home: Option<&Path>,
) -> Vec<(i32, IdeFsCompleteItem)> {
    let q_lower = raw_q.trim().to_ascii_lowercase();
    let mut scored: Vec<(i32, PathBuf, bool)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if let Ok(rd) = fs::read_dir(root) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !q_lower.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user) {
                continue;
            }
            let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if let Some(sc) = score_candidate(&name, &name, &q_lower, is_dir) {
                seen.insert(path.clone());
                scored.push((sc + 20, path, is_dir));
            }
        }
    }

    if let Some(repo_root) = find_git_root(root, home) {
        collect_git_ls_files(
            &repo_root,
            root,
            &q_lower,
            limit,
            is_root_user,
            &mut seen,
            &mut scored,
        );
    } else if root.components().count() >= 3 {
        let deadline = Instant::now() + Duration::from_millis(SHALLOW_DEADLINE_MS);
        let mut visited = 0usize;
        walk_bounded(
            root,
            root,
            1,
            &q_lower,
            is_root_user,
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

fn collect_git_ls_files(
    repo_root: &Path,
    dir_prefix: &Path,
    q_lower: &str,
    limit: usize,
    is_root_user: bool,
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
        if !is_allowed_path(&lexical_full, is_root_user) {
            continue;
        }
        seen.insert(lexical_full.clone());
        scored.push((sc, lexical_full, false));
        accepted += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_bounded(
    base: &Path,
    cur: &Path,
    depth: usize,
    q_lower: &str,
    is_root_user: bool,
    deadline: Instant,
    visited: &mut usize,
    seen: &mut HashSet<PathBuf>,
    scored: &mut Vec<(i32, PathBuf, bool)>,
) {
    if depth > SHALLOW_MAX_DEPTH || *visited >= SHALLOW_MAX_VISITED || Instant::now() >= deadline {
        return;
    }
    let Ok(rd) = fs::read_dir(cur) else {
        return;
    };
    for entry in rd.flatten() {
        *visited += 1;
        if *visited >= SHALLOW_MAX_VISITED || Instant::now() >= deadline {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !is_allowed_entry_in_canonical_dir(&path, &ft, is_root_user) {
            continue;
        }
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
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
                &path,
                depth + 1,
                q_lower,
                is_root_user,
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

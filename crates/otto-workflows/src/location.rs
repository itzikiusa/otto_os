//! Where a workflow run may work, with no daemon context.
//!
//! Callers pass the workspace root, the registered `(repo id, path)` pairs,
//! and whether an existing host directory is allowed. Repo listing, the
//! "is this name registered" check, and who may set the host-folder flag stay
//! in the server.

use std::path::{Component, Path, PathBuf};

/// Expand a leading `~` / `~/` the same way the engine does (`dirs::home_dir`).
fn expand_tilde(p: &str) -> String {
    if p == "~" {
        return dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

/// Whether a `repos[]` entry is written as a path (`/…`, `~…`, `./…`, `../…`)
/// rather than a repo name or id.
pub fn looks_like_path(repo: &str) -> bool {
    let r = repo.trim();
    r.starts_with('/') || r.starts_with('~') || r.starts_with('.')
}

/// One path of a run location: absolute (`~` expanded), and inside
/// `ws_root`, one of `repos` (`(id, path)` of every registered repo), or a
/// linked worktree whose origin is one of them.
///
/// `allow_existing_host_folder` adds one more acceptance: an absolute
/// directory that already exists, with no `..` component, and that is not the
/// filesystem root. Relative paths, `..`, missing paths, and files stay
/// refused. Registered repos and the workspace root do not need the flag.
pub async fn ensure_path_in_workspace(
    ws_root: &str,
    repos: &[(String, String)],
    raw: &str,
    allow_existing_host_folder: bool,
) -> otto_core::Result<()> {
    let expanded = expand_tilde(raw.trim());
    let path = Path::new(&expanded);
    if !path.is_absolute() {
        return Err(run_location_error(
            allow_existing_host_folder,
            format!("working directory '{raw}' must be an absolute path"),
            ws_root,
        ));
    }
    // A path that does not exist yet is judged by its deepest existing
    // ancestor (canonical) plus the rest — so it may not climb out with `..`.
    let canonicalized = std::fs::canonicalize(path);
    if canonicalized.is_err() && path.components().any(|c| c == Component::ParentDir) {
        return Err(run_location_error(
            allow_existing_host_folder,
            format!("working directory '{raw}' may not contain '..'"),
            ws_root,
        ));
    }
    let existed = canonicalized.is_ok();
    let target = match canonicalized {
        Ok(p) => p,
        Err(_) => {
            let mut base = path.to_path_buf();
            let mut rest = Vec::new();
            let canon = loop {
                if let Ok(c) = std::fs::canonicalize(&base) {
                    break c;
                }
                match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
                    (Some(name), Some(parent)) => {
                        rest.push(name);
                        base = parent.to_path_buf();
                    }
                    _ => break base,
                }
            };
            rest.iter().rev().fold(canon, |acc, n| acc.join(n))
        }
    };
    let root = std::fs::canonicalize(ws_root).unwrap_or_else(|_| PathBuf::from(ws_root));
    if target.starts_with(&root) {
        return Ok(());
    }
    let target_s = target.to_string_lossy().into_owned();
    if match_repo_path(&target_s, repos).is_some() {
        return Ok(());
    }
    if let Some(main) = git_main_worktree(&target_s).await {
        if match_repo_path(&main, repos).is_some() {
            return Ok(());
        }
    }
    // Root's manual run: a real directory on this machine. `..` is not
    // rewritten into an exemption — a path that climbs out still has to land
    // in a registered repo or the workspace root above. `/` stays refused
    // here; session spawn has its own cwd guard for `$HOME` and Otto's data
    // dir.
    if allow_existing_host_folder && existing_host_folder(path, existed, &target) {
        return Ok(());
    }
    Err(run_location_error(
        allow_existing_host_folder,
        if allow_existing_host_folder {
            format!(
                "working directory '{raw}' is not a registered repo or an existing host directory"
            )
        } else {
            format!(
                "working directory '{raw}' is not a registered repo — a run may only work in a repo \
                 registered in Otto (any workspace) or this workspace's root folder ({ws_root})"
            )
        },
        ws_root,
    ))
}

/// Non-root and external starts keep `specific` alone. A human root manual
/// run adds the host-directory rule, because that caller may use a folder
/// that is not a registered repo.
fn run_location_error(
    allow_existing_host_folder: bool,
    specific: String,
    ws_root: &str,
) -> otto_core::Error {
    otto_core::Error::Invalid(if allow_existing_host_folder {
        format!(
            "{specific} — a folder outside registered repos must already exist, be a directory, \
             contain no '..', and not be the filesystem root; otherwise a run may only work in a \
             repo registered in Otto (any workspace) or this workspace's root folder ({ws_root})"
        )
    } else {
        specific
    })
}

/// An absolute directory that exists, does not contain `..`, and is not the
/// filesystem root. Used only when the manual caller is root.
fn existing_host_folder(path: &Path, existed: bool, target: &Path) -> bool {
    existed
        && target.is_dir()
        && target.parent().is_some()
        && !path.components().any(|c| c == Component::ParentDir)
}

/// Pure path matcher (canonicalized, component-wise).
/// `target` matches a repo when it equals the repo path or is nested under it;
/// the deepest (most specific) registered repo wins. Component-wise so a sibling
/// like `…/foo_wt` does NOT match a repo at `…/foo`.
pub fn match_repo_path(target: &str, repos: &[(String, String)]) -> Option<String> {
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let t = canon(target);
    let mut best: Option<(usize, String)> = None;
    for (id, rp) in repos {
        let r = canon(rp);
        if t == r || t.starts_with(&r) {
            let depth = r.components().count();
            if best.as_ref().map(|(d, _)| depth > *d).unwrap_or(true) {
                best = Some((depth, id.clone()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// For a `git worktree` checkout at `path`, the main worktree directory (the
/// origin repo): `git -C <path> rev-parse --path-format=absolute
/// --git-common-dir` yields the shared `…/.git`, whose parent is the origin repo.
/// `None` when `path` isn't a git repo / git is unavailable. Runs on a blocking
/// thread so it never stalls the async runtime.
#[allow(clippy::disallowed_methods)] // the git subprocess runs inside spawn_blocking
pub async fn git_main_worktree(path: &str) -> Option<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let out = otto_git::hardened_std_command()
            .arg("-C")
            .arg(&path)
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let common = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if common.is_empty() {
            return None;
        }
        Path::new(&common)
            .parent()
            .map(|x| x.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

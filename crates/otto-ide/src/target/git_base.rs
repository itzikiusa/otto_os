//! Clean `HEAD` blob reader (`GET /ide/fs/git-base`) for IDE gutter change markers.
//!
//! Resolves the enclosing Git repository for a confined local file, refuses broad
//! non-project roots (`$HOME`, `/tmp`, `/Users`, …), reads `HEAD:<rel_path>` with a
//! `2s` timeout and `4 MiB` byte cap, and follows staged renames (`R`) so renamed
//! files highlight only edited lines instead of painting the entire file green.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::io::AsyncReadExt;

use crate::lsp_host::pool_spawn::kill_process_group_sync;
use crate::types::IdeGitBaseResp;

use super::fs_ops::{enforce_non_root_path, trim_incomplete_utf8_tail};
use super::sdk_paths::{is_virtual_archive_uri, resolve_local_readable};
use super::{resolve_local_prospective_in_roots, ResolvedTarget, MAX_FILE_READ_BYTES};

const GIT_BASE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PORCELAIN_BYTES: u64 = 2 * 1024 * 1024;

struct PgKillGuard(Option<u32>);

impl Drop for PgKillGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.0.take() {
            #[cfg(unix)]
            kill_process_group_sync(pgid);
            #[cfg(not(unix))]
            let _ = pgid;
        }
    }
}

fn normalize_git_rel_path(rel: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg.to_str()?;
                if s.is_empty() || s == "." || s == ".." {
                    return None;
                }
                parts.push(s);
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join("/");
    if joined.trim_start().starts_with('-')
        || joined.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
    {
        return None;
    }
    Some(joined)
}

async fn resolve_git_toplevel(parent_dir: &Path) -> Option<PathBuf> {
    let mut cmd = otto_git_cmd::hardened_command();
    cmd.arg("-C")
        .arg(parent_dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().ok()?;
    let mut guard = PgKillGuard(child.id());
    let stdout = child.stdout.take()?;
    let mut buf = Vec::new();
    let ok = tokio::time::timeout(GIT_BASE_TIMEOUT, async {
        stdout.take(16 * 1024).read_to_end(&mut buf).await.ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(())
    })
    .await
    .ok()
    .flatten();
    ok?;
    guard.0 = None;
    let raw = std::str::from_utf8(&buf).ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(raw);
    if otto_core::paths::is_broad_non_project_root(&p) {
        return None;
    }
    let canon = std::fs::canonicalize(&p).ok()?;
    if otto_core::paths::is_broad_non_project_root(&canon) {
        return None;
    }
    Some(canon)
}

fn is_missing_head_blob(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "not a valid object name",
        "does not exist in",
        "exists on disk, but not in",
        "invalid object name",
        "bad revision",
        "unknown revision",
        "ambiguous argument",
        "is a tree",
        "bad file",
    ];
    MARKERS.iter().any(|m| low.contains(m))
}

struct HeadBlobData {
    content: Option<String>,
    binary: bool,
    truncated: bool,
}

async fn read_head_blob(toplevel: &Path, rel_path: &str) -> Result<Option<HeadBlobData>> {
    let spec = format!("HEAD:{rel_path}");
    let mut cmd = otto_git_cmd::hardened_command();
    cmd.arg("-C")
        .arg(toplevel)
        .args(["cat-file", "blob", &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn git cat-file: {e}")))?;
    let mut guard = PgKillGuard(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing git cat-file stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("missing git cat-file stderr".into()))?;

    let cap = MAX_FILE_READ_BYTES as usize;
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let outcome = tokio::time::timeout(GIT_BASE_TIMEOUT, async {
        let read_out = async {
            stdout
                .take((cap + 1) as u64)
                .read_to_end(&mut out_buf)
                .await
        };
        let read_err = async { stderr.take(8192).read_to_end(&mut err_buf).await };
        let (ro, re) = tokio::join!(read_out, read_err);
        ro?;
        re?;
        if out_buf.len() > cap {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Ok::<Option<std::process::ExitStatus>, std::io::Error>(None);
        }
        let status = child.wait().await?;
        Ok(Some(status))
    })
    .await
    .map_err(|_| Error::Upstream("git cat-file timed out after 2s".into()))?
    .map_err(|e| Error::Internal(format!("read git cat-file: {e}")))?;

    guard.0 = None;
    if let Some(status) = outcome {
        if !status.success() {
            let err_str = String::from_utf8_lossy(&err_buf);
            if is_missing_head_blob(&err_str) {
                return Ok(None);
            }
            return Ok(None);
        }
    }

    let truncated = out_buf.len() > cap;
    if truncated {
        out_buf.truncate(cap);
    }
    let binary = out_buf.contains(&0);
    if binary {
        return Ok(Some(HeadBlobData {
            content: None,
            binary: true,
            truncated,
        }));
    }
    let slice = if truncated {
        trim_incomplete_utf8_tail(&out_buf)
    } else {
        out_buf.as_slice()
    };
    let content = match std::str::from_utf8(slice) {
        Ok(s) => s.to_owned(),
        Err(_) => String::from_utf8_lossy(slice).into_owned(),
    };
    Ok(Some(HeadBlobData {
        content: Some(content),
        binary: false,
        truncated,
    }))
}

/// Inspect `git status --porcelain=v1 -z --untracked-files=no` to see if `rel_path`
/// was renamed (`R` / `C`) from `<orig_rel_path>` in the index.
/// Note: passing `-- <rel_path>` to `git status` suppresses rename pairing because
/// git's pathspec excludes `<orig_rel_path>`; `--untracked-files=no` scans only the
/// index vs `HEAD` (`<5ms`) while preserving full `R  <new>\0<orig>\0` pairs.
async fn detect_porcelain_rename_orig(toplevel: &Path, rel_path: &str) -> Option<String> {
    let mut cmd = otto_git_cmd::hardened_command();
    cmd.arg("-C")
        .arg(toplevel)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=no"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().ok()?;
    let mut guard = PgKillGuard(child.id());
    let stdout = child.stdout.take()?;
    let mut buf = Vec::new();
    let ok = tokio::time::timeout(GIT_BASE_TIMEOUT, async {
        stdout
            .take(MAX_PORCELAIN_BYTES)
            .read_to_end(&mut buf)
            .await
            .ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(())
    })
    .await
    .ok()
    .flatten();
    ok?;
    guard.0 = None;

    let records: Vec<&[u8]> = buf.split(|&b| b == 0).collect();
    let mut idx = 0;
    while idx < records.len() {
        let rec = records[idx];
        if rec.len() < 4 {
            idx += 1;
            continue;
        }
        let x = rec[0];
        let sep = rec[2];
        if sep != b' ' {
            idx += 1;
            continue;
        }
        let path_bytes = &rec[3..];
        if x == b'R' || x == b'C' {
            let orig_bytes = records.get(idx + 1).copied()?;
            if path_bytes == rel_path.as_bytes() {
                let orig_str = std::str::from_utf8(orig_bytes).ok()?;
                return normalize_git_rel_path(Path::new(orig_str));
            }
            idx += 2;
        } else {
            idx += 1;
        }
    }
    None
}

fn orig_path_allowed_for_non_root(
    orig_abs: &Path,
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
) -> bool {
    let check_path = orig_abs.canonicalize().ok().or_else(|| {
        orig_abs
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .and_then(|cp| orig_abs.file_name().map(|n| cp.join(n)))
    });
    let Some(cand) = check_path else {
        return false;
    };
    if enforce_non_root_path(&cand).is_err() {
        return false;
    }
    std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .chain(readonly_roots.iter().map(PathBuf::as_path))
        .any(|allowed| cand.starts_with(allowed))
}

pub(super) async fn read_git_base_for_target(
    target: &ResolvedTarget,
    path: &str,
) -> Result<IdeGitBaseResp> {
    match target {
        ResolvedTarget::Ssh { .. } => Ok(IdeGitBaseResp::not_in_repo()),
        ResolvedTarget::Local {
            root,
            additional_roots,
            readonly_roots,
            is_root_user,
            ..
        } => {
            if is_virtual_archive_uri(path) {
                return Ok(IdeGitBaseResp::not_in_repo());
            }
            let file_path = match resolve_local_readable(
                root,
                additional_roots,
                readonly_roots,
                *is_root_user,
                path,
            ) {
                Ok((canon, _)) => canon,
                Err(Error::NotFound(_)) => {
                    let (prospective, _) = resolve_local_prospective_in_roots(
                        root,
                        additional_roots,
                        *is_root_user,
                        path,
                        false,
                    )
                    .await?;
                    prospective
                }
                Err(other) => return Err(other),
            };
            let Some(parent_dir) = file_path.parent() else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            let Some(toplevel) = resolve_git_toplevel(parent_dir).await else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            let Ok(raw_rel) = file_path.strip_prefix(&toplevel) else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            let Some(rel_path) = normalize_git_rel_path(raw_rel) else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            let repo_root_str = toplevel.to_string_lossy().into_owned();

            if let Some(blob) = read_head_blob(&toplevel, &rel_path).await? {
                return Ok(IdeGitBaseResp {
                    in_repo: true,
                    repo_root: Some(repo_root_str),
                    rel_path: Some(rel_path),
                    head_exists: true,
                    content: blob.content,
                    binary: blob.binary,
                    truncated: blob.truncated,
                });
            }

            if let Some(orig_rel) = detect_porcelain_rename_orig(&toplevel, &rel_path).await {
                let orig_abs = toplevel.join(&orig_rel);
                let allowed = *is_root_user
                    || orig_path_allowed_for_non_root(
                        &orig_abs,
                        root,
                        additional_roots,
                        readonly_roots,
                    );
                if allowed {
                    if let Some(blob) = read_head_blob(&toplevel, &orig_rel).await? {
                        return Ok(IdeGitBaseResp {
                            in_repo: true,
                            repo_root: Some(repo_root_str),
                            rel_path: Some(rel_path),
                            head_exists: true,
                            content: blob.content,
                            binary: blob.binary,
                            truncated: blob.truncated,
                        });
                    }
                }
            }

            Ok(IdeGitBaseResp {
                in_repo: true,
                repo_root: Some(repo_root_str),
                rel_path: Some(rel_path),
                head_exists: false,
                content: None,
                binary: false,
                truncated: false,
            })
        }
    }
}

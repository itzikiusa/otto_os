//! Clean `HEAD` blob reader (`GET /ide/fs/git-base`) for IDE gutter change markers.
//!
//! Resolves the enclosing Git repository starting from the file's own parent
//! directory (supporting multi-root secondary project folders, git worktrees, and
//! submodules across both Local and SSH targets), passes `GIT_OPTIONAL_LOCKS=0` on
//! every read-only `git` invocation, guards staged rename scans behind a fast
//! `git ls-files --stage -- <rel_path>` check, propagates real `git cat-file`
//! errors, and caps blob reads at `MAX_FILE_READ_BYTES`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::{Error, Result};
use tokio::io::AsyncReadExt;

use crate::lsp_host::pool_spawn::kill_process_group_sync;
use crate::types::IdeGitBaseResp;

use super::fs_ops::enforce_non_root_path;
use super::path_resolve::{remote_parent, resolve_remote_in_roots_or_sdk};
use super::sdk_paths::{is_virtual_archive_uri, resolve_local_readable};
use super::ssh_exec::write_ssh_env_preamble;
use super::{
    resolve_local_prospective_in_roots, ResolvedTarget, TargetManager, MAX_FILE_READ_BYTES,
};

const GIT_BASE_TIMEOUT: Duration = Duration::from_secs(2);
const SSH_GIT_BASE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PORCELAIN_BYTES: u64 = 2 * 1024 * 1024;
const TOPLEVEL_CACHE_TTL: Duration = Duration::from_secs(30);
const TOPLEVEL_CACHE_CAP: usize = 128;

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

fn toplevel_cache() -> &'static DashMap<PathBuf, (Instant, Option<PathBuf>)> {
    static CACHE: OnceLock<DashMap<PathBuf, (Instant, Option<PathBuf>)>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

fn hardened_git_ro() -> tokio::process::Command {
    let mut cmd = otto_git_cmd::hardened_command();
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

fn ssh_git_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
    env
}

pub(crate) fn normalize_git_rel_path(rel: &Path) -> Option<String> {
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

fn nearest_existing_ancestor(file_path: &Path) -> Option<PathBuf> {
    let mut cur = file_path.parent();
    while let Some(d) = cur {
        if d.is_dir() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

async fn resolve_git_toplevel(probe_dir: &Path) -> Option<PathBuf> {
    if let Some(hit) = toplevel_cache().get(probe_dir) {
        if hit.0.elapsed() < TOPLEVEL_CACHE_TTL {
            return hit.1.clone();
        }
    }

    let mut cmd = hardened_git_ro();
    cmd.arg("-C")
        .arg(probe_dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    // Only an answer git actually gave is cached: a spawn failure or timeout
    // must not pin "not a repo" (and an empty gutter baseline) for the TTL.
    let mut definitive = false;
    let resolved = async {
        let mut child = cmd.spawn().ok()?;
        let mut guard = PgKillGuard(child.id());
        let stdout = child.stdout.take()?;
        let mut buf = Vec::new();
        let ok = tokio::time::timeout(GIT_BASE_TIMEOUT, async {
            stdout.take(16 * 1024).read_to_end(&mut buf).await.ok()?;
            let status = child.wait().await.ok()?;
            definitive = true;
            status.success().then_some(())
        })
        .await
        .ok()
        .flatten();
        ok?;
        guard.0 = None;
        let raw = std::str::from_utf8(&buf).ok()?.trim().to_string();
        if raw.is_empty() {
            return None;
        }
        tokio::task::spawn_blocking(move || {
            let p = PathBuf::from(&raw);
            if otto_core::paths::is_broad_non_project_root(&p) {
                return None;
            }
            #[allow(clippy::disallowed_methods)]
            let canon = std::fs::canonicalize(&p).ok()?;
            if otto_core::paths::is_broad_non_project_root(&canon) {
                return None;
            }
            Some(canon)
        })
        .await
        .ok()
        .flatten()
    }
    .await;

    if !definitive {
        return resolved;
    }
    let cache = toplevel_cache();
    if cache.len() >= TOPLEVEL_CACHE_CAP {
        cache.retain(|_, (ts, _)| ts.elapsed() < TOPLEVEL_CACHE_TTL);
        if cache.len() >= TOPLEVEL_CACHE_CAP {
            if let Some(k) = cache.iter().next().map(|e| e.key().clone()) {
                cache.remove(&k);
            }
        }
    }
    cache.insert(probe_dir.to_path_buf(), (Instant::now(), resolved.clone()));
    resolved
}

pub(crate) fn is_missing_head_blob(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    const FATAL_CORRUPTION_OR_PERM: &[&str] = &[
        "corrupt",
        "permission denied",
        "dubious ownership",
        "unable to open",
        "unable to read",
        "unable to unpack",
        "inflate:",
        "short read",
        "bad object",
        "packfile",
    ];
    if FATAL_CORRUPTION_OR_PERM.iter().any(|m| low.contains(m)) {
        return false;
    }
    const MISSING_MARKERS: &[&str] = &[
        "not a valid object name",
        "does not exist in",
        "exists on disk, but not in",
        "invalid object name",
        "bad revision",
        "unknown revision",
        "ambiguous argument",
        "is a tree",
    ];
    MISSING_MARKERS.iter().any(|m| low.contains(m))
}

struct HeadBlobData {
    content: Option<String>,
    binary: bool,
    truncated: bool,
}

fn decode_head_blob_bytes(mut out_buf: Vec<u8>, cap: usize) -> HeadBlobData {
    let truncated = out_buf.len() > cap;
    if truncated {
        out_buf.truncate(cap);
    }
    if out_buf.contains(&0) {
        return HeadBlobData {
            content: None,
            binary: true,
            truncated,
        };
    }
    let slice = if truncated {
        otto_core::text::trim_incomplete_utf8_tail(&out_buf)
    } else {
        out_buf.as_slice()
    };
    let Ok(s) = std::str::from_utf8(slice) else {
        return HeadBlobData {
            content: None,
            binary: true,
            truncated,
        };
    };
    HeadBlobData {
        content: Some(s.to_owned()),
        binary: false,
        truncated,
    }
}

async fn read_head_blob(toplevel: &Path, rel_path: &str) -> Result<Option<HeadBlobData>> {
    let spec = format!("HEAD:{rel_path}");
    let mut cmd = hardened_git_ro();
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
            return Err(Error::Internal(format!(
                "git cat-file HEAD:{rel_path} failed: {}",
                err_str.trim()
            )));
        }
    }

    Ok(Some(decode_head_blob_bytes(out_buf, cap)))
}

async fn is_local_staged_in_index(toplevel: &Path, rel_path: &str) -> bool {
    let mut cmd = hardened_git_ro();
    cmd.arg("-C")
        .arg(toplevel)
        .args(["ls-files", "--stage", "--", rel_path])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let mut guard = PgKillGuard(child.id());
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    let mut buf = Vec::new();
    let ok = tokio::time::timeout(GIT_BASE_TIMEOUT, async {
        stdout.take(8192).read_to_end(&mut buf).await.ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(())
    })
    .await
    .ok()
    .flatten();
    if ok.is_none() {
        return false;
    }
    guard.0 = None;
    !buf.iter().all(u8::is_ascii_whitespace)
}

pub(crate) fn parse_porcelain_rename_orig(buf: &[u8], rel_path: &str) -> Option<String> {
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

async fn detect_porcelain_rename_orig(toplevel: &Path, rel_path: &str) -> Option<String> {
    let mut cmd = hardened_git_ro();
    cmd.arg("-C")
        .arg(toplevel)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "-M",
            "--untracked-files=no",
        ])
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

    parse_porcelain_rename_orig(&buf, rel_path)
}

fn orig_path_allowed_for_non_root(
    orig_abs: &Path,
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
) -> bool {
    #[allow(clippy::disallowed_methods)]
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

fn is_broad_remote_git_root(top: &str, remote_home: &str) -> bool {
    let clean = top.trim_end_matches('/');
    if clean.is_empty() {
        return true;
    }
    const BROAD_REMOTE: &[&str] = &[
        "/",
        "/Users",
        "/home",
        "/root",
        "/tmp",
        "/private/tmp",
        "/var",
        "/private/var",
        "/etc",
        "/private/etc",
        "/opt",
        "/usr",
    ];
    if BROAD_REMOTE.contains(&clean) {
        return true;
    }
    let h = remote_home.trim_end_matches('/');
    !h.is_empty() && clean == h
}

/// `Ok(None)` = genuinely not in a repo (the probe script exits 0 then); an
/// SSH failure is an error, so the UI never caches a missing baseline for it.
async fn resolve_ssh_git_toplevel_and_rel(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    remote_file: &str,
) -> Result<Option<(String, String)>> {
    let Some(parent) = remote_parent(remote_file) else {
        return Ok(None);
    };
    let q_file = shell_words::quote(remote_file);
    let q_parent = shell_words::quote(&parent);
    let script = format!(
        "export GIT_OPTIONAL_LOCKS=0; \
         f={q_file}; d={q_parent}; suffix=\"\"; \
         while [ ! -d \"$d\" ] && [ \"$d\" != \"/\" ] && [ -n \"$d\" ]; do \
           b=$(basename \"$d\"); \
           if [ -n \"$suffix\" ]; then suffix=\"$b/$suffix\"; else suffix=\"$b\"; fi; \
           d=$(dirname \"$d\"); \
         done; \
         [ -d \"$d\" ] || exit 0; \
         top=$(git -C \"$d\" rev-parse --show-toplevel 2>/dev/null) || exit 0; \
         [ -n \"$top\" ] || exit 0; \
         pd=$(cd \"$d\" 2>/dev/null && pwd -P) || pd=\"$d\"; \
         fn=$(basename \"$f\"); \
         if [ -n \"$suffix\" ]; then pf=\"$pd/$suffix/$fn\"; else pf=\"$pd/$fn\"; fi; \
         ph=$(cd \"$HOME\" 2>/dev/null && pwd -P || printf '%s' \"$HOME\"); \
         printf 'TOP:%s\\nFILE:%s\\nHOME:%s\\n' \"$top\" \"$pf\" \"$ph\""
    );
    let out = mgr
        .exec_shell(
            target,
            &script,
            None,
            &BTreeMap::new(),
            SSH_GIT_BASE_TIMEOUT,
        )
        .await?;
    if out.exit_code != 0 {
        return Err(Error::Upstream(format!(
            "remote git probe failed (exit {})",
            out.exit_code
        )));
    }
    Ok(parse_ssh_git_probe(&out.stdout, remote_file))
}

fn parse_ssh_git_probe(stdout: &str, remote_file: &str) -> Option<(String, String)> {
    let mut top = String::new();
    let mut phys_file = String::new();
    let mut r_home = String::new();
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("TOP:") {
            top = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("FILE:") {
            phys_file = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("HOME:") {
            r_home = v.trim().to_string();
        }
    }
    if top.is_empty() || is_broad_remote_git_root(&top, &r_home) {
        return None;
    }
    let top_clean = top.trim_end_matches('/');
    let rel_raw = phys_file
        .strip_prefix(&format!("{top_clean}/"))
        .or_else(|| remote_file.strip_prefix(&format!("{top_clean}/")))?;
    let rel_norm = normalize_git_rel_path(Path::new(rel_raw))?;
    Some((top_clean.to_string(), rel_norm))
}

async fn read_ssh_head_blob(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    toplevel: &str,
    rel_path: &str,
) -> Result<Option<HeadBlobData>> {
    let args = vec![
        "-C".to_string(),
        toplevel.to_string(),
        "cat-file".to_string(),
        "blob".to_string(),
        format!("HEAD:{rel_path}"),
    ];
    let env = ssh_git_env();
    let (mut cmd, preamble) =
        mgr.build_piped_command_with_preamble(target, "git", &args, None, &env)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("spawn ssh git cat-file: {e}")))?;
    write_ssh_env_preamble(&mut child, preamble, true).await?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing ssh git cat-file stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("missing ssh git cat-file stderr".into()))?;

    let cap = MAX_FILE_READ_BYTES as usize;
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let outcome = tokio::time::timeout(SSH_GIT_BASE_TIMEOUT, async {
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
    .map_err(|_| Error::Upstream("ssh git cat-file timed out".into()))?
    .map_err(|e| Error::Upstream(format!("read ssh git cat-file: {e}")))?;

    if let Some(status) = outcome {
        if !status.success() {
            let err_str = String::from_utf8_lossy(&err_buf);
            if is_missing_head_blob(&err_str) {
                return Ok(None);
            }
            return Err(Error::Internal(format!(
                "git cat-file HEAD:{rel_path} failed: {}",
                err_str.trim()
            )));
        }
    }
    Ok(Some(decode_head_blob_bytes(out_buf, cap)))
}

async fn detect_ssh_staged_rename_orig(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    toplevel: &str,
    rel_path: &str,
) -> Option<String> {
    let env = ssh_git_env();
    let ls_args = vec![
        "-C".to_string(),
        toplevel.to_string(),
        "ls-files".to_string(),
        "--stage".to_string(),
        "--".to_string(),
        rel_path.to_string(),
    ];
    let ls_out = mgr
        .exec_command(target, "git", &ls_args, None, &env, SSH_GIT_BASE_TIMEOUT)
        .await
        .ok()?;
    if ls_out.exit_code != 0 || ls_out.stdout.trim().is_empty() {
        return None;
    }

    let st_args = vec![
        "-C".to_string(),
        toplevel.to_string(),
        "status".to_string(),
        "--porcelain=v1".to_string(),
        "-z".to_string(),
        "-M".to_string(),
        "--untracked-files=no".to_string(),
    ];
    let (mut cmd, preamble) = mgr
        .build_piped_command_with_preamble(target, "git", &st_args, None, &env)
        .ok()?;
    let mut child = cmd.spawn().ok()?;
    write_ssh_env_preamble(&mut child, preamble, true)
        .await
        .ok()?;
    let stdout = child.stdout.take()?;
    let mut buf = Vec::new();
    let ok = tokio::time::timeout(SSH_GIT_BASE_TIMEOUT, async {
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
    parse_porcelain_rename_orig(&buf, rel_path)
}

pub(super) async fn read_git_base_for_target(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    path: &str,
) -> Result<IdeGitBaseResp> {
    if is_virtual_archive_uri(path) {
        return Ok(IdeGitBaseResp::not_in_repo());
    }
    match target {
        ResolvedTarget::Ssh {
            root,
            additional_roots,
            ..
        } => {
            let (remote_file, _) = resolve_remote_in_roots_or_sdk(root, additional_roots, path)?;
            let Some((toplevel, rel_path)) =
                resolve_ssh_git_toplevel_and_rel(mgr, target, &remote_file).await?
            else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            if let Some(blob) = read_ssh_head_blob(mgr, target, &toplevel, &rel_path).await? {
                return Ok(IdeGitBaseResp {
                    in_repo: true,
                    repo_root: Some(toplevel),
                    rel_path: Some(rel_path),
                    head_exists: true,
                    content: blob.content,
                    binary: blob.binary,
                    truncated: blob.truncated,
                });
            }
            if let Some(orig_rel) =
                detect_ssh_staged_rename_orig(mgr, target, &toplevel, &rel_path).await
            {
                let orig_remote = format!("{toplevel}/{orig_rel}");
                if resolve_remote_in_roots_or_sdk(root, additional_roots, &orig_remote).is_ok() {
                    if let Some(blob) =
                        read_ssh_head_blob(mgr, target, &toplevel, &orig_rel).await?
                    {
                        return Ok(IdeGitBaseResp {
                            in_repo: true,
                            repo_root: Some(toplevel),
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
                repo_root: Some(toplevel),
                rel_path: Some(rel_path),
                head_exists: false,
                content: None,
                binary: false,
                truncated: false,
            })
        }
        ResolvedTarget::Local {
            root,
            additional_roots,
            readonly_roots,
            is_root_user,
            ..
        } => {
            let r_c = root.clone();
            let ar_c = additional_roots.clone();
            let ro_c = readonly_roots.clone();
            let is_root = *is_root_user;
            let p_c = path.to_string();
            let local_read = tokio::task::spawn_blocking(move || {
                resolve_local_readable(&r_c, &ar_c, &ro_c, is_root, &p_c)
            })
            .await
            .map_err(|e| Error::Internal(format!("join resolve_local_readable: {e}")))?;

            let file_path = match local_read {
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
            let Some(probe_dir) = nearest_existing_ancestor(&file_path) else {
                return Ok(IdeGitBaseResp::not_in_repo());
            };
            let Some(toplevel) = resolve_git_toplevel(&probe_dir).await else {
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

            if is_local_staged_in_index(&toplevel, &rel_path).await {
                if let Some(orig_rel) = detect_porcelain_rename_orig(&toplevel, &rel_path).await {
                    let orig_abs = toplevel.join(&orig_rel);
                    let r_c = root.clone();
                    let ar_c = additional_roots.clone();
                    let ro_c = readonly_roots.clone();
                    let allowed = if *is_root_user {
                        true
                    } else {
                        tokio::task::spawn_blocking(move || {
                            orig_path_allowed_for_non_root(&orig_abs, &r_c, &ar_c, &ro_c)
                        })
                        .await
                        .unwrap_or(false)
                    };
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

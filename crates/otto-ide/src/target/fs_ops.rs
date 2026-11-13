//! Platform normalization and local/remote filesystem tree, read, atomic write,
//! and mutation helpers for [`super::TargetManager`].
//!
//! Performance & memory invariants:
//! - `list_local_dir_sync` performs a shallow 1-level `read_dir` (`<1ms` for
//!   typical directories) with ZERO subprocess spawns and ZERO retained caches.
//! - File reads stream bounded slices (`<= 4 MiB`) and drop buffers immediately.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use otto_core::{Error, Result};
use otto_ssh::SftpSession;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::types::{
    detect_language, IdeFileReadResp, IdeFsEntry, IdeFsEntryKind, IdeFsMutateOp, IdeFsMutateResp,
    IdePlatformInfo,
};

use super::sdk_paths::strip_file_uri;
use super::{join_remote, map_io_err, resolve_local_prospective_in_roots, resolve_remote_subpath};

pub(crate) fn enforce_non_root_path(canonical: &Path) -> Result<()> {
    use otto_core::secret_paths as sp;
    if sp::in_protected_dir(canonical)
        || sp::is_denied_dir(canonical)
        || sp::is_denied_file(canonical)
    {
        return Err(Error::Forbidden(format!(
            "{} holds protected credentials or Otto state; access denied",
            canonical.display()
        )));
    }
    Ok(())
}

/// Resolve an existing path within `root` or any attached `additional_roots`,
/// returning `(canonical_path, matched_root)`.
pub(crate) fn resolve_local_existing_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
) -> Result<(PathBuf, PathBuf)> {
    let decoded = strip_file_uri(raw);
    let trimmed = decoded.trim();
    let candidate = if trimmed.is_empty() || trimmed == "." {
        root.to_path_buf()
    } else if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else {
        let p = Path::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|e| map_io_err("resolve path", &candidate, e))?;

    let matched_root = std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .find(|r| canonical.starts_with(r))
        .ok_or_else(|| {
            Error::Forbidden(format!(
                "path '{}' is outside workspace root '{}'",
                canonical.display(),
                root.display()
            ))
        })?;

    if !is_root_user {
        enforce_non_root_path(&canonical)?;
    }
    Ok((canonical, matched_root.to_path_buf()))
}

pub(crate) fn resolve_local_existing(
    root: &Path,
    is_root_user: bool,
    raw: &str,
) -> Result<PathBuf> {
    resolve_local_existing_in_roots(root, &[], is_root_user, raw).map(|(c, _)| c)
}

pub(super) fn normalize_os_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "macos" | "darwin" => "macos".into(),
        "linux" => "linux".into(),
        other => other.to_string(),
    }
}

pub(super) fn normalize_arch_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "aarch64" | "arm64" => "aarch64".into(),
        "x86_64" | "amd64" | "x64" => "x86_64".into(),
        other => other.to_string(),
    }
}

pub(super) fn parse_uname_output(stdout: &str) -> IdePlatformInfo {
    let mut parts = stdout.split_whitespace();
    let os_raw = parts.next().unwrap_or("linux");
    let arch_raw = parts.next().unwrap_or("x86_64");
    IdePlatformInfo {
        os: normalize_os_name(os_raw),
        arch: normalize_arch_name(arch_raw),
    }
}

fn detect_entry_language(path: &str) -> Option<String> {
    if let Some(lang) = detect_language(path) {
        return Some(lang.to_string());
    }
    let clean = path.split('?').next().unwrap_or(path);
    if clean.starts_with("jdt://") || clean.ends_with(".class") {
        return Some("java".into());
    }
    let base = clean.rsplit('/').next().unwrap_or(clean);
    match base {
        ".zshrc" | ".bashrc" | ".zshenv" | ".zprofile" | ".bash_profile" => {
            return Some("shell".into());
        }
        _ => {}
    }
    let ext = base.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "csv" => Some("csv".into()),
        "tsv" => Some("tsv".into()),
        "jsonl" | "ndjson" => Some("jsonl".into()),
        "bats" => Some("shell".into()),
        _ => None,
    }
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking (pure 1-level read_dir, <1ms)
pub(super) fn list_local_dir_sync(
    dir_path: &Path,
    show_hidden: bool,
    restricted: bool,
) -> Result<Vec<IdeFsEntry>> {
    let rd = std::fs::read_dir(dir_path).map_err(|e| map_io_err("list directory", dir_path, e))?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let entry_path = item.path();
        if restricted {
            if otto_core::secret_paths::is_denied_file(&entry_path) {
                continue;
            }
            if let Ok(canon) = entry_path.canonicalize() {
                if enforce_non_root_path(&canon).is_err() {
                    continue;
                }
            }
        }
        let Ok(ft) = item.file_type() else { continue };
        let meta = if ft.is_symlink() {
            std::fs::metadata(&entry_path).ok()
        } else {
            item.metadata().ok()
        };
        let kind = if ft.is_symlink() {
            match &meta {
                Some(m) if m.is_dir() => IdeFsEntryKind::Dir,
                _ => IdeFsEntryKind::Symlink,
            }
        } else if ft.is_dir() {
            IdeFsEntryKind::Dir
        } else if ft.is_file() {
            IdeFsEntryKind::File
        } else {
            continue;
        };
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        let path_str = entry_path.to_string_lossy().into_owned();
        let language = if kind == IdeFsEntryKind::Dir {
            None
        } else {
            detect_entry_language(&path_str)
        };
        let entry = IdeFsEntry {
            name,
            path: path_str,
            kind,
            size,
            mtime,
            git_status: None,
            language,
        };
        if kind == IdeFsEntryKind::Dir {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }

    dirs.sort_by_cached_key(|e| e.name.to_ascii_lowercase());
    files.sort_by_cached_key(|e| e.name.to_ascii_lowercase());
    dirs.extend(files);
    Ok(dirs)
}

pub(super) fn build_remote_tree_entries(
    remote_dir: &str,
    raw_entries: Vec<otto_ssh::SftpEntry>,
    show_hidden: bool,
) -> Vec<IdeFsEntry> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for item in raw_entries {
        if item.name == "." || item.name == ".." {
            continue;
        }
        if !show_hidden && item.name.starts_with('.') {
            continue;
        }
        let kind = match item.kind.as_str() {
            "dir" => IdeFsEntryKind::Dir,
            "symlink" => IdeFsEntryKind::Symlink,
            "file" => IdeFsEntryKind::File,
            _ => continue,
        };
        let full_path = join_remote(remote_dir, &item.name);
        let language = if kind == IdeFsEntryKind::Dir {
            None
        } else {
            detect_entry_language(&full_path)
        };
        let entry = IdeFsEntry {
            name: item.name,
            path: full_path,
            kind,
            size: item.size,
            mtime: item.mtime,
            git_status: None,
            language,
        };
        if kind == IdeFsEntryKind::Dir {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }
    dirs.sort_by_cached_key(|e| e.name.to_ascii_lowercase());
    files.sort_by_cached_key(|e| e.name.to_ascii_lowercase());
    dirs.extend(files);
    dirs
}

pub(super) async fn read_bounded_bytes(path: &Path, max_bytes: u64) -> Result<(Vec<u8>, bool)> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| map_io_err("open file", path, e))?;
    let mut reader = file.take(max_bytes + 1);
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .await
        .map_err(|e| map_io_err("read file", path, e))?;
    if buf.len() as u64 > max_bytes {
        buf.truncate(max_bytes as usize);
        Ok((buf, true))
    } else {
        Ok((buf, false))
    }
}

pub(crate) fn build_file_read_resp(
    path: String,
    bytes: &[u8],
    size: u64,
    read_only: bool,
    truncated: bool,
) -> Result<IdeFileReadResp> {
    let sample_len = bytes.len().min(8192);
    let is_binary = bytes[..sample_len].contains(&0);
    let sha256 = sha256_hex(bytes);
    let language = detect_entry_language(&path).unwrap_or_else(|| "plaintext".to_string());
    if is_binary {
        return Ok(IdeFileReadResp {
            path,
            content: String::new(),
            language,
            size,
            sha256,
            read_only: true,
            truncated: true,
            line_count: 0,
        });
    }
    let content = String::from_utf8_lossy(bytes).into_owned();
    let line_count = if content.is_empty() {
        0
    } else {
        content.lines().count()
    };
    Ok(IdeFileReadResp {
        path,
        content,
        language,
        size,
        sha256,
        read_only,
        truncated,
        line_count,
    })
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub(super) async fn verify_local_expected_sha(dest: &Path, expected: &str) -> Result<()> {
    match tokio::fs::read(dest).await {
        Ok(existing) => {
            let actual = sha256_hex(&existing);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(Error::Conflict(format!(
                    "file '{}' changed on disk (expected sha256 {expected}, actual {actual})",
                    dest.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let empty_sha = sha256_hex(&[]);
            if !expected.eq_ignore_ascii_case(&empty_sha) {
                return Err(Error::Conflict(format!(
                    "file '{}' was deleted on disk before save",
                    dest.display()
                )));
            }
            Ok(())
        }
        Err(e) => Err(map_io_err("read existing file for conflict check", dest, e)),
    }
}

pub(super) async fn verify_remote_expected_sha(
    sftp: &SftpSession,
    remote_path: &str,
    expected: &str,
) -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()
        .map_err(|e| Error::Internal(format!("temp conflict file: {e}")))?;
    let tmp_str = tmp.path().to_string_lossy().into_owned();
    if sftp.download(remote_path, &tmp_str).await.is_ok() {
        let existing = tokio::fs::read(tmp.path())
            .await
            .map_err(|e| Error::Internal(format!("read conflict file: {e}")))?;
        let actual = sha256_hex(&existing);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::Conflict(format!(
                "remote file '{remote_path}' changed (expected sha256 {expected}, actual {actual})"
            )));
        }
    }
    Ok(())
}

pub(super) async fn write_local_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = dest
        .parent()
        .ok_or_else(|| Error::Invalid("file path has no parent directory".into()))?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".otto-ide-save-{}-{seq}.tmp", std::process::id());
    let tmp_path = parent.join(tmp_name);
    // Preserve existing file permissions when overwriting an executable script.
    let existing_perms = tokio::fs::metadata(dest)
        .await
        .ok()
        .map(|m| m.permissions());
    tokio::fs::write(&tmp_path, bytes)
        .await
        .map_err(|e| map_io_err("write temporary file", &tmp_path, e))?;
    if let Some(perms) = existing_perms {
        let _ = tokio::fs::set_permissions(&tmp_path, perms).await;
    }
    if let Err(e) = tokio::fs::rename(&tmp_path, dest).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(map_io_err("commit file save", dest, e));
    }
    crate::lsp_host::pool::notify_watched_file_if_build_manifest(dest, 2);
    Ok(())
}

fn is_protected_root(target_path: &Path, root: &Path, additional_roots: &[PathBuf]) -> bool {
    target_path == root || additional_roots.iter().any(|r| r == target_path)
}

pub(super) async fn mutate_local_fs_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    op: IdeFsMutateOp,
    path: &str,
    dest_path: Option<&str>,
) -> Result<IdeFsMutateResp> {
    match op {
        IdeFsMutateOp::Mkdir => {
            let (dir, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                path,
                true,
            )
            .await?;
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| map_io_err("create directory", &dir, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dir.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::CreateFile => {
            let (file, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                path,
                true,
            )
            .await?;
            if tokio::fs::metadata(&file).await.is_ok() {
                return Err(Error::Conflict(format!(
                    "'{}' already exists",
                    file.display()
                )));
            }
            // Auto-populate Go `package <pkg>` header when creating a new `.go` file!
            let initial_bytes = default_new_file_template(&file);
            write_local_atomic(&file, initial_bytes.as_bytes()).await?;
            crate::lsp_host::pool::notify_watched_file_if_build_manifest(&file, 1);
            Ok(IdeFsMutateResp {
                ok: true,
                path: file.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Rename => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for rename".into()))?;
            let (src, _) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            if is_protected_root(&src, root, additional_roots) {
                return Err(Error::Forbidden("cannot rename workspace root".into()));
            }
            let (dst, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                dst_raw,
                true,
            )
            .await?;
            tokio::fs::rename(&src, &dst)
                .await
                .map_err(|e| map_io_err("rename path", &src, e))?;
            crate::lsp_host::pool::notify_watched_file_if_build_manifest(&src, 3);
            crate::lsp_host::pool::notify_watched_file_if_build_manifest(&dst, 1);
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Delete => {
            let (target_path, _) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            if is_protected_root(&target_path, root, additional_roots) {
                return Err(Error::Forbidden("cannot delete workspace root".into()));
            }
            let meta = tokio::fs::symlink_metadata(&target_path)
                .await
                .map_err(|e| map_io_err("stat path for delete", &target_path, e))?;
            if meta.is_dir() {
                tokio::fs::remove_dir_all(&target_path)
                    .await
                    .map_err(|e| map_io_err("delete directory", &target_path, e))?;
            } else {
                tokio::fs::remove_file(&target_path)
                    .await
                    .map_err(|e| map_io_err("delete file", &target_path, e))?;
                crate::lsp_host::pool::notify_watched_file_if_build_manifest(&target_path, 3);
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: target_path.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Duplicate => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for duplicate".into()))?;
            let (src, _) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            let (dst, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                dst_raw,
                true,
            )
            .await?;
            tokio::fs::copy(&src, &dst)
                .await
                .map_err(|e| map_io_err("duplicate file", &src, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst.to_string_lossy().into_owned(),
            })
        }
    }
}

/// Infer an idiomatic `package <name>\n` header when creating a new `.go` file
/// (matching sibling `.go` files in the same directory or directory name).
#[allow(clippy::disallowed_methods)]
fn default_new_file_template(file: &Path) -> String {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("go") {
        return String::new();
    }
    let Some(dir) = file.parent() else {
        return "package main\n".into();
    };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten().take(16) {
            let p = entry.path();
            if p != file
                && p.extension().and_then(|e| e.to_str()) == Some("go")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_test.go"))
            {
                if let Ok(src) = std::fs::read_to_string(&p) {
                    for line in src.lines().take(20) {
                        let t = line.trim();
                        if let Some(rest) = t.strip_prefix("package ") {
                            let pkg = rest.split_whitespace().next().unwrap_or("");
                            if !pkg.is_empty() {
                                return format!("package {pkg}\n\n");
                            }
                        }
                    }
                }
            }
        }
    }
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("main");
    let pkg: String =
        if matches!(dir_name, "cmd" | "main" | "server" | "cli") || dir.join("go.mod").is_file() {
            "main".into()
        } else {
            let cleaned: String = dir_name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if cleaned.is_empty() || cleaned.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                "main".into()
            } else {
                cleaned
            }
        };
    format!("package {pkg}\n\n")
}

pub(super) async fn mutate_remote_fs(
    sftp: &SftpSession,
    root: &str,
    op: IdeFsMutateOp,
    path: &str,
    dest_path: Option<&str>,
) -> Result<IdeFsMutateResp> {
    let remote = resolve_remote_subpath(root, path)?;
    match op {
        IdeFsMutateOp::Mkdir => {
            sftp.mkdir(&remote).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::CreateFile => {
            let tmp = tempfile::NamedTempFile::new()
                .map_err(|e| Error::Internal(format!("temp file: {e}")))?;
            let tmp_str = tmp.path().to_string_lossy().into_owned();
            sftp.upload(&tmp_str, &remote).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::Rename => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for rename".into()))?;
            let dst = resolve_remote_subpath(root, dst_raw)?;
            sftp.rename(&remote, &dst).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst,
            })
        }
        IdeFsMutateOp::Delete => {
            if remote == root {
                return Err(Error::Forbidden("cannot delete workspace root".into()));
            }
            if sftp.remove(&remote).await.is_err() {
                sftp.rmdir(&remote).await?;
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::Duplicate => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for duplicate".into()))?;
            let dst = resolve_remote_subpath(root, dst_raw)?;
            let tmp = tempfile::NamedTempFile::new()
                .map_err(|e| Error::Internal(format!("temp duplicate file: {e}")))?;
            let tmp_str = tmp.path().to_string_lossy().into_owned();
            sftp.download(&remote, &tmp_str).await?;
            sftp.upload(&tmp_str, &dst).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst,
            })
        }
    }
}

//! Platform normalization, git porcelain status parsing, and local/remote
//! filesystem tree, read, atomic write, and mutation helpers for [`super::TargetManager`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use otto_core::{Error, Result};
use otto_ssh::SftpSession;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::types::{
    detect_language, IdeFileReadResp, IdeFsEntry, IdeFsEntryKind, IdeFsMutateOp, IdeFsMutateResp,
    IdePlatformInfo,
};

use super::{
    enforce_non_root_path, join_remote, map_io_err, resolve_local_existing,
    resolve_local_prospective, resolve_remote_subpath,
};

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

pub(super) fn parse_git_porcelain(stdout: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let status_pair = &line[..2];
        let mut rel = line[3..].trim();
        if let Some((_, renamed_to)) = rel.split_once(" -> ") {
            rel = renamed_to.trim();
        }
        let rel = rel.trim_matches('"');
        let code = if status_pair == "??" {
            "?"
        } else if status_pair.contains('D') {
            "D"
        } else if status_pair.contains('A') {
            "A"
        } else {
            "M"
        };
        map.insert(rel.to_string(), code.to_string());
        // Propagate modified indicator to ancestor folders so collapsed dirs show state.
        let mut cur = rel;
        while let Some((parent, _)) = cur.rsplit_once('/') {
            map.entry(parent.to_string())
                .or_insert_with(|| code.to_string());
            cur = parent;
        }
    }
    map
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn list_local_dir_sync(
    root: &Path,
    dir_path: &Path,
    show_hidden: bool,
    restricted: bool,
    git_map: &HashMap<String, String>,
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
            if let Ok(canon) = entry_path.canonicalize() {
                if enforce_non_root_path(&canon).is_err() {
                    continue;
                }
            } else if otto_core::secret_paths::is_denied_file(&entry_path) {
                continue;
            }
        }
        let Ok(ft) = item.file_type() else { continue };
        let meta = std::fs::metadata(&entry_path).ok();
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
        let rel = entry_path
            .strip_prefix(root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let git_status = git_map.get(&rel).cloned();
        let path_str = entry_path.to_string_lossy().into_owned();
        let language = if kind == IdeFsEntryKind::Dir {
            None
        } else {
            detect_language(&path_str).map(str::to_string)
        };
        let entry = IdeFsEntry {
            name,
            path: path_str,
            kind,
            size,
            mtime,
            git_status,
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
    root: &str,
    remote_dir: &str,
    raw_entries: Vec<otto_ssh::SftpEntry>,
    show_hidden: bool,
    git_map: &HashMap<String, String>,
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
        let rel = full_path
            .strip_prefix(root)
            .map(|s| s.trim_start_matches('/'))
            .unwrap_or("");
        let git_status = git_map.get(rel).cloned();
        let language = if kind == IdeFsEntryKind::Dir {
            None
        } else {
            detect_language(&full_path).map(str::to_string)
        };
        let entry = IdeFsEntry {
            name: item.name,
            path: full_path,
            kind,
            size: item.size,
            mtime: item.mtime,
            git_status,
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

pub(super) fn build_file_read_resp(
    path: String,
    bytes: &[u8],
    size: u64,
    read_only: bool,
    truncated: bool,
) -> Result<IdeFileReadResp> {
    let sample_len = bytes.len().min(8192);
    let is_binary = bytes[..sample_len].contains(&0);
    let sha256 = sha256_hex(bytes);
    let language = detect_language(&path).unwrap_or("plaintext").to_string();
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
    Ok(())
}

pub(super) async fn mutate_local_fs(
    root: &Path,
    is_root_user: bool,
    op: IdeFsMutateOp,
    path: &str,
    dest_path: Option<&str>,
) -> Result<IdeFsMutateResp> {
    match op {
        IdeFsMutateOp::Mkdir => {
            let dir = resolve_local_prospective(root, is_root_user, path, true).await?;
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| map_io_err("create directory", &dir, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dir.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::CreateFile => {
            let file = resolve_local_prospective(root, is_root_user, path, true).await?;
            if tokio::fs::metadata(&file).await.is_ok() {
                return Err(Error::Conflict(format!(
                    "'{}' already exists",
                    file.display()
                )));
            }
            write_local_atomic(&file, b"").await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: file.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Rename => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for rename".into()))?;
            let src = resolve_local_existing(root, is_root_user, path)?;
            if src == root {
                return Err(Error::Forbidden("cannot rename workspace root".into()));
            }
            let dst = resolve_local_prospective(root, is_root_user, dst_raw, true).await?;
            tokio::fs::rename(&src, &dst)
                .await
                .map_err(|e| map_io_err("rename path", &src, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Delete => {
            let target_path = resolve_local_existing(root, is_root_user, path)?;
            if target_path == root {
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
            let src = resolve_local_existing(root, is_root_user, path)?;
            let dst = resolve_local_prospective(root, is_root_user, dst_raw, true).await?;
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

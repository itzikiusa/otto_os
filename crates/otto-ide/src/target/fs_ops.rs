//! Platform normalization and local/remote filesystem tree, read, and atomic write
//! helpers for [`super::TargetManager`].
//!
//! Performance & memory invariants:
//! - `list_local_dir_sync` performs a shallow 1-level `read_dir` (`<1ms` for
//!   typical directories) with ZERO subprocess spawns and ZERO retained caches.
//! - File reads stream bounded slices (`<= 4 MiB`) and drop buffers immediately.

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use otto_ssh::SftpSession;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

pub(super) use crate::jsonrpc_framing::sha256_hex;
use crate::lsp_host::pool::LspPool;
use crate::types::{detect_language, IdeFileReadResp, IdeFsEntry, IdeFsEntryKind, IdePlatformInfo};

pub use super::fs_mutate::HistoryWriteOpts;
pub(super) use super::fs_mutate::{mutate_local_fs_in_roots, mutate_remote_fs};
use super::sdk_paths::strip_file_uri;
use super::{join_remote, map_io_err};

pub fn enforce_non_root_path(canonical: &Path) -> Result<()> {
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
    canonical_in_roots(
        &local_candidate(root, raw)?,
        root,
        additional_roots,
        is_root_user,
    )
}

/// [`resolve_local_existing_in_roots`] for an entry Delete/Rename acts on: a
/// symlink stays the link itself (only its parent is resolved and confined),
/// so the operation never reaches what the link points to.
pub(crate) fn resolve_local_entry_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
) -> Result<(PathBuf, PathBuf)> {
    let candidate = local_candidate(root, raw)?;
    let is_link = std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink());
    let (true, Some(parent), Some(name)) = (is_link, candidate.parent(), candidate.file_name())
    else {
        // Delete/Rename source: moving `.git` aside (then editing it) is a
        // write to git's own files too.
        let (entry, matched_root) =
            canonical_in_roots(&candidate, root, additional_roots, is_root_user)?;
        super::control_paths::enforce_control_path(&entry, is_root_user)?;
        return Ok((entry, matched_root));
    };
    let (parent, matched_root) = canonical_in_roots(parent, root, additional_roots, is_root_user)?;
    let entry = parent.join(name);
    if !is_root_user {
        enforce_non_root_path(&entry)?;
    }
    super::control_paths::enforce_control_path(&entry, is_root_user)?;
    Ok((entry, matched_root))
}

fn local_candidate(root: &Path, raw: &str) -> Result<PathBuf> {
    let decoded = strip_file_uri(raw);
    Ok(if decoded.is_empty() || decoded == "." {
        root.to_path_buf()
    } else if decoded.starts_with('~') {
        otto_core::paths::expand_home_strict(&decoded).map_err(Error::Invalid)?
    } else {
        let p = Path::new(&decoded);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    })
}

fn canonical_in_roots(
    candidate: &Path,
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
) -> Result<(PathBuf, PathBuf)> {
    let canonical = candidate
        .canonicalize()
        .map_err(|e| map_io_err("resolve path", candidate, e))?;

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

/// Hide dotfiles (`.DS_Store`, `.git`, …) and legacy Delve `__debug_bin*` scratch
/// binaries from default tree listings unless `show_hidden` is enabled.
fn is_hidden_tree_entry(name: &str) -> bool {
    name.starts_with('.') || name.starts_with("__debug_bin")
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
        if !show_hidden && is_hidden_tree_entry(&name) {
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
        if !show_hidden && is_hidden_tree_entry(&item.name) {
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
    let is_binary = bytes.contains(&0);
    let language = detect_entry_language(&path).unwrap_or_else(|| "plaintext".to_string());
    if is_binary {
        let sha256 = sha256_hex(bytes);
        return Ok(IdeFileReadResp {
            path,
            content: String::new(),
            language,
            size,
            sha256,
            binary: true,
            read_only: true,
            truncated: true,
            line_count: 0,
        });
    }
    let effective_bytes = if truncated {
        otto_core::text::trim_incomplete_utf8_tail(bytes)
    } else {
        bytes
    };
    let sha256 = sha256_hex(effective_bytes);
    let (content, invalid_utf8) = match std::str::from_utf8(effective_bytes) {
        Ok(s) => (s.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(effective_bytes).into_owned(), true),
    };
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
        binary: invalid_utf8,
        read_only: read_only || truncated || invalid_utf8,
        truncated,
        line_count,
    })
}

pub(super) fn is_remote_sftp_not_found(err: &Error) -> bool {
    match err {
        Error::NotFound(_) => true,
        Error::Upstream(msg) => {
            let lower = msg.to_ascii_lowercase();
            lower.contains("not found") || lower.contains("no such file")
        }
        _ => false,
    }
}

pub(super) async fn verify_local_expected_sha(dest: &Path, expected: &str) -> Result<()> {
    const MAX_HASH_CHECK_BYTES: u64 = 256 * 1024 * 1024;
    let mut file = match tokio::fs::File::open(dest).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let empty_sha = sha256_hex(&[]);
            if !expected.eq_ignore_ascii_case(&empty_sha) {
                return Err(Error::Conflict(format!(
                    "file '{}' was deleted on disk before save",
                    dest.display()
                )));
            }
            return Ok(());
        }
        Err(e) => return Err(map_io_err("read existing file for conflict check", dest, e)),
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| map_io_err("read existing file for conflict check", dest, e))?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > MAX_HASH_CHECK_BYTES {
            return Err(Error::Invalid(format!(
                "file '{}' exceeds maximum conflict-check size ({MAX_HASH_CHECK_BYTES} bytes)",
                dest.display()
            )));
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::Conflict(format!(
            "file '{}' changed on disk (expected sha256 {expected}, actual {actual})",
            dest.display()
        )));
    }
    Ok(())
}

pub(super) async fn verify_remote_expected_sha(
    sftp: &SftpSession,
    remote_path: &str,
    expected: &str,
) -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()
        .map_err(|e| Error::Internal(format!("temp conflict file: {e}")))?;
    let tmp_str = tmp.path().to_string_lossy().into_owned();
    match sftp.download(remote_path, &tmp_str).await {
        Ok(()) => {
            let existing = tokio::fs::read(tmp.path())
                .await
                .map_err(|e| Error::Internal(format!("read conflict file: {e}")))?;
            let actual = sha256_hex(&existing);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(Error::Conflict(format!(
                    "remote file '{remote_path}' changed (expected sha256 {expected}, actual {actual})"
                )));
            }
            Ok(())
        }
        Err(err) if is_remote_sftp_not_found(&err) => {
            let empty_sha = sha256_hex(&[]);
            if !expected.eq_ignore_ascii_case(&empty_sha) {
                return Err(Error::Conflict(format!(
                    "remote file '{remote_path}' was deleted before save"
                )));
            }
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub(super) async fn write_local_atomic(
    dest: &Path,
    bytes: &[u8],
    lsp_pool: Option<&LspPool>,
) -> Result<()> {
    let dest_buf = dest.to_path_buf();
    let payload = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::Write;
        let parent = dest_buf
            .parent()
            .ok_or_else(|| Error::Invalid("file path has no parent directory".into()))?;
        let existing_perms = std::fs::metadata(&dest_buf).ok().map(|m| m.permissions());
        let mut tmp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| map_io_err("create temporary save file", &dest_buf, e))?;
        tmp.write_all(&payload)
            .map_err(|e| map_io_err("write temporary file", tmp.path(), e))?;
        let _ = tmp.as_file().sync_all();
        if let Some(perms) = existing_perms {
            let _ = std::fs::set_permissions(tmp.path(), perms);
        }
        tmp.persist(&dest_buf)
            .map_err(|e| map_io_err("commit file save", &dest_buf, e.error))?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Internal(format!("atomic save worker failed: {e}")))??;
    if let Some(pool) = lsp_pool {
        pool.notify_watched_file_if_build_manifest(dest, 2);
    }
    Ok(())
}

//! Index persistence, atomic blob writes, retention pruning, blob GC, and
//! workspace path-confinement helpers for [`super::LocalHistoryStore`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::target::enforce_non_root_path;
use crate::types::{IdeHistoryEntryResp, IdeHistoryRestoreResp, IdeLocalHistoryEntry};

use super::{
    LocalHistoryStore, DEFAULT_HISTORY_RETENTION_DAYS, MAX_HISTORY_ENTRIES_PER_ROOT,
    MAX_HISTORY_FILE_BYTES,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredEntry {
    #[serde(default)]
    pub(super) seq: u64,
    #[serde(flatten)]
    pub(super) entry: IdeLocalHistoryEntry,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct RootIndex {
    #[serde(default)]
    pub(super) next_seq: u64,
    #[serde(default)]
    pub(super) entries: Vec<StoredEntry>,
}

pub(super) struct LoadedIndex {
    pub(super) index: RootIndex,
    pub(super) allow_gc: bool,
}

pub(super) fn is_denied_secret_snapshot_path(path: &Path) -> bool {
    otto_core::secret_paths::is_denied_dir(path) || otto_core::secret_paths::is_denied_file(path)
}

pub(super) fn normalize_snapshot_target(canon_root: &Path, file_path: &Path) -> Result<PathBuf> {
    let resolved = file_path
        .canonicalize()
        .ok()
        .or_else(|| otto_core::secret_paths::canonicalize_prospective(file_path))
        .ok_or_else(|| Error::Invalid("invalid local history snapshot path".into()))?;
    if !resolved.starts_with(canon_root) {
        return Err(Error::Forbidden(
            "local history snapshot path escapes workspace root".into(),
        ));
    }
    Ok(resolved)
}

pub(super) fn normalize_target_filter(
    canon_root: &Path,
    raw: Option<&str>,
) -> Result<Option<String>> {
    let Some(raw_str) = raw.filter(|s| !s.is_empty() && *s != ".") else {
        return Ok(None);
    };
    let decoded = crate::target::sdk_paths::strip_file_uri(raw_str);
    if decoded.is_empty() || decoded == "." {
        return Ok(None);
    }
    let candidate = if decoded.starts_with('~') {
        otto_core::paths::expand_home_strict(&decoded).map_err(Error::Invalid)?
    } else {
        let p = Path::new(&decoded);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            canon_root.join(p)
        }
    };
    let norm = candidate
        .canonicalize()
        .ok()
        .or_else(|| otto_core::secret_paths::canonicalize_prospective(&candidate))
        .unwrap_or(candidate);
    Ok(Some(norm.to_string_lossy().into_owned()))
}

pub(super) fn scan_root_overlaps_filter(scan_root: &Path, filter_path: Option<&str>) -> bool {
    let Some(fp) = filter_path else {
        return true;
    };
    let sr_str = scan_root.to_string_lossy();
    let sr = sr_str.trim_end_matches('/');
    let fp_clean = fp.trim_end_matches('/');
    fp_clean == sr
        || fp_clean
            .strip_prefix(sr)
            .is_some_and(|rest| rest.starts_with('/'))
        || sr
            .strip_prefix(fp_clean)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn entry_path_allowed(
    canon_root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw_entry_path: &str,
) -> Result<PathBuf> {
    let raw = Path::new(raw_entry_path);
    let resolved = raw
        .canonicalize()
        .ok()
        .or_else(|| otto_core::secret_paths::canonicalize_prospective(raw))
        .ok_or_else(|| Error::Invalid("invalid local history entry path".into()))?;
    let in_roots = std::iter::once(canon_root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .any(|r| resolved.starts_with(r));
    if !in_roots {
        return Err(Error::Forbidden(
            "local history entry path escapes workspace root".into(),
        ));
    }
    if is_denied_secret_snapshot_path(raw) || is_denied_secret_snapshot_path(&resolved) {
        return Err(Error::Forbidden(
            "local history entry targets a protected secret path".into(),
        ));
    }
    if !is_root_user {
        enforce_non_root_path(raw)?;
        enforce_non_root_path(&resolved)?;
    }
    Ok(resolved)
}

pub(super) fn is_binary_or_invalid_utf8(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn read_index(root_dir: &Path) -> LoadedIndex {
    let path = root_dir.join("index.json");
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadedIndex {
            index: RootIndex::default(),
            allow_gc: true,
        },
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "local history index unreadable; preserving existing blobs"
            );
            LoadedIndex {
                index: RootIndex::default(),
                allow_gc: false,
            }
        }
        Ok(bytes) => match serde_json::from_slice::<RootIndex>(&bytes) {
            Ok(index) => LoadedIndex {
                index,
                allow_gc: true,
            },
            Err(e) => {
                let backup = root_dir.join(format!("index.corrupt-{}.json", now_ms()));
                let _ = std::fs::rename(&path, &backup);
                tracing::warn!(
                    path = %path.display(),
                    backup = %backup.display(),
                    error = %e,
                    "quarantined corrupt local history index; preserving existing blobs"
                );
                LoadedIndex {
                    index: RootIndex::default(),
                    allow_gc: false,
                }
            }
        },
    }
}

pub(super) fn write_index(root_dir: &Path, idx: &RootIndex) -> Result<()> {
    let bytes = serde_json::to_vec(idx)
        .map_err(|e| Error::Internal(format!("serialize local history index: {e}")))?;
    write_atomic_sync(&root_dir.join("index.json"), &bytes)
}

pub(super) fn write_atomic_sync(dest: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = dest
        .parent()
        .ok_or_else(|| Error::Invalid("path has no parent directory".into()))?;
    let existing_perms = std::fs::metadata(dest).ok().map(|m| m.permissions());
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| Error::Internal(format!("create temp file in '{}': {e}", parent.display())))?;
    tmp.write_all(bytes)
        .map_err(|e| Error::Internal(format!("write temp file '{}': {e}", tmp.path().display())))?;
    let _ = tmp.as_file().sync_all();
    if let Some(perms) = existing_perms {
        let _ = std::fs::set_permissions(tmp.path(), perms);
    }
    tmp.persist(dest).map_err(|e| {
        Error::Internal(format!(
            "commit atomic write '{}': {}",
            dest.display(),
            e.error
        ))
    })?;
    Ok(())
}

pub(super) fn prune_entries(idx: &mut RootIndex, now: u64, retention_days: Option<u32>) {
    let days = retention_days
        .unwrap_or(DEFAULT_HISTORY_RETENTION_DAYS)
        .clamp(1, 365) as u64;
    let cutoff_ms = now.saturating_sub(days.saturating_mul(86_400_000));
    idx.entries.retain(|s| s.entry.timestamp_ms >= cutoff_ms);
    if idx.entries.len() > MAX_HISTORY_ENTRIES_PER_ROOT {
        let excess = idx.entries.len() - MAX_HISTORY_ENTRIES_PER_ROOT;
        idx.entries.drain(0..excess);
    }
}

#[allow(clippy::disallowed_methods)] // bounded blob directory GC (<= 251 files)
pub(super) fn gc_unreferenced_blobs(blobs_dir: &Path, idx: &RootIndex) {
    let live_hashes: HashSet<&str> = idx
        .entries
        .iter()
        .map(|s| s.entry.sha256.as_str())
        .collect();
    let Ok(rd) = std::fs::read_dir(blobs_dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(sha) = name.strip_suffix(".blob") {
            if !live_hashes.contains(sha) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

impl LocalHistoryStore {
    /// Read a single Local History entry's blob content alongside current disk content.
    pub fn get_entry(
        &self,
        canon_root: &Path,
        additional_roots: &[PathBuf],
        is_root_user: bool,
        entry_id: &str,
    ) -> Result<IdeHistoryEntryResp> {
        let mut seen_buckets = HashSet::new();
        let mut found_in_bucket = None;
        for scan_root in
            std::iter::once(canon_root).chain(additional_roots.iter().map(PathBuf::as_path))
        {
            let (bucket, root_dir) = self.bucket_dir(scan_root);
            if !seen_buckets.insert(bucket.clone()) {
                continue;
            }
            let _guard = self.lock_bucket(&bucket);
            let idx = read_index(&root_dir).index;
            if let Some(found) = idx
                .entries
                .into_iter()
                .find(|s| s.entry.id == entry_id)
                .map(|s| s.entry)
            {
                found_in_bucket = Some((found, root_dir));
                break;
            }
        }
        let (found, root_dir) = found_in_bucket.ok_or_else(|| {
            Error::NotFound(format!("local history entry '{entry_id}' not found"))
        })?;
        let disk_path =
            entry_path_allowed(canon_root, additional_roots, is_root_user, &found.path)?;
        let blob_path = root_dir
            .join("blobs")
            .join(format!("{}.blob", found.sha256));
        let bytes = std::fs::read(&blob_path).map_err(|_| {
            Error::NotFound(format!(
                "local history blob for entry '{entry_id}' was pruned or missing"
            ))
        })?;
        let content = String::from_utf8(bytes)
            .map_err(|_| Error::Internal("stored local history blob is not UTF-8".into()))?;

        let (current_exists, current_content) = match std::fs::metadata(&disk_path) {
            Ok(m) if m.is_file() && m.len() <= MAX_HISTORY_FILE_BYTES as u64 => {
                match std::fs::read(&disk_path) {
                    Ok(b) if !is_binary_or_invalid_utf8(&b) => (true, String::from_utf8(b).ok()),
                    _ => (true, None),
                }
            }
            Ok(m) if m.is_file() => (true, None),
            _ => (false, None),
        };

        Ok(IdeHistoryEntryResp {
            entry: found,
            content,
            current_content,
            current_exists,
        })
    }

    /// Restore a snapshot entry back to disk (recreating deleted files & parent dirs).
    pub fn restore_entry(
        &self,
        canon_root: &Path,
        additional_roots: &[PathBuf],
        is_root_user: bool,
        entry_id: &str,
    ) -> Result<IdeHistoryRestoreResp> {
        let detail = self.get_entry(canon_root, additional_roots, is_root_user, entry_id)?;
        let target_file = PathBuf::from(&detail.entry.path);
        let prospective = otto_core::secret_paths::canonicalize_prospective(&target_file)
            .ok_or_else(|| Error::Invalid("invalid restore target path".into()))?;

        let matched_root = std::iter::once(canon_root)
            .chain(additional_roots.iter().map(PathBuf::as_path))
            .find(|r| prospective.starts_with(r))
            .ok_or_else(|| {
                Error::Forbidden(format!(
                    "restore target '{}' escapes workspace root '{}'",
                    prospective.display(),
                    canon_root.display()
                ))
            })?;
        if is_denied_secret_snapshot_path(&prospective) {
            return Err(Error::Forbidden(
                "cannot restore into a protected secret path".into(),
            ));
        }
        if !is_root_user {
            enforce_non_root_path(&prospective)?;
        }

        let existed_before = prospective.is_file();
        if existed_before {
            let _ = self.record_existing_file_sync(
                matched_root,
                &prospective,
                "external",
                "Before restore",
                true,
            );
        }
        if let Some(parent) = prospective.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("create parent directories: {e}")))?;
        }
        write_atomic_sync(&prospective, detail.content.as_bytes())?;
        let _ = self.record_bytes_sync(
            matched_root,
            &prospective,
            detail.content.as_bytes(),
            "restored",
            "Restored",
            false,
        )?;

        Ok(IdeHistoryRestoreResp {
            ok: true,
            path: prospective.to_string_lossy().into_owned(),
            content: detail.content,
            recreated: !existed_before,
        })
    }
}

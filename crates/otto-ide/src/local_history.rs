//! Bounded, zero-cache, content-addressed Local History store (`ide-local-history`).
//!
//! Captures file creation, pre-edit external state, saves, renames, and
//! pre-delete file/directory snapshots under `<data_dir>/ide-local-history/`
//! so users can diff or restore any edited or deleted file without Git commits.
//!
//! Memory & disk bounds:
//! - Max [`MAX_HISTORY_ENTRIES_PER_ROOT`] (`250`) entries per project root.
//! - Default [`DEFAULT_HISTORY_RETENTION_DAYS`] (`14` days, configurable `1..=365`).
//! - Max [`MAX_HISTORY_FILE_BYTES`] (`2 MiB`) per UTF-8 text file (binary & secret files skipped).
//! - Content-addressed SHA-256 blobs: identical saves consume `0` extra bytes,
//!   and unreferenced blobs are garbage-collected automatically on prune.
//! - Zero persistent in-memory content cache (`< 1 MiB` steady-state RAM).

mod storage;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use otto_core::{new_id, Error, Result};
use sha2::{Digest, Sha256};

use crate::jsonrpc_framing::sha256_hex;
use crate::target::enforce_non_root_path;
use crate::types::{IdeHistoryListResp, IdeLocalHistoryEntry};

use self::storage::{
    entry_path_allowed, gc_unreferenced_blobs, is_binary_or_invalid_utf8,
    is_denied_secret_snapshot_path, normalize_snapshot_target, normalize_target_filter, now_ms,
    prune_entries, read_index, scan_root_overlaps_filter, write_atomic_sync, write_index,
    StoredEntry,
};

/// Maximum history timeline entries retained per project root.
pub const MAX_HISTORY_ENTRIES_PER_ROOT: usize = 250;
/// Default retention window for Local History entries (`14` days).
pub const DEFAULT_HISTORY_RETENTION_DAYS: u32 = 14;
/// Maximum file size eligible for Local History snapshots (`2 MiB`).
pub const MAX_HISTORY_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum files snapshotted when deleting a directory tree.
pub const MAX_DELETED_DIR_FILES: usize = 200;
/// Default entry count returned by `GET /ide/history/list`.
const DEFAULT_LIST_LIMIT: usize = 100;

/// Thread-safe, disk-backed Local History store with zero retained blob cache.
#[derive(Clone)]
pub struct LocalHistoryStore {
    base_dir: PathBuf,
    stripes: Arc<[Mutex<()>; 16]>,
}

impl LocalHistoryStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            base_dir: data_dir.join("ide-local-history"),
            stripes: Arc::new(std::array::from_fn(|_| Mutex::new(()))),
        }
    }

    fn lock_bucket(&self, bucket: &str) -> MutexGuard<'_, ()> {
        let idx = bucket
            .get(..2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .unwrap_or(0) as usize
            & 15;
        self.stripes[idx].lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bucket_dir(&self, canon_root: &Path) -> (String, PathBuf) {
        let key = canon_root.to_string_lossy();
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let bucket = hex::encode(hasher.finalize())[..16].to_string();
        let dir = self.base_dir.join(&bucket);
        (bucket, dir)
    }

    /// Record a UTF-8 snapshot if `bytes` is non-binary and `<= 2 MiB`.
    pub fn record_bytes_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        bytes: &[u8],
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
    ) -> Result<Option<IdeLocalHistoryEntry>> {
        self.record_bytes_with_retention_sync(
            canon_root,
            canon_file,
            bytes,
            event_kind,
            label,
            skip_if_unchanged,
            None,
        )
    }

    /// Record a UTF-8 snapshot with an optional custom retention window (`days`).
    #[allow(clippy::too_many_arguments)]
    pub fn record_bytes_with_retention_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        bytes: &[u8],
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
        retention_days: Option<u32>,
    ) -> Result<Option<IdeLocalHistoryEntry>> {
        if bytes.len() > MAX_HISTORY_FILE_BYTES || is_binary_or_invalid_utf8(bytes) {
            return Ok(None);
        }
        let verified_file = match normalize_snapshot_target(canon_root, canon_file) {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        if is_denied_secret_snapshot_path(canon_file)
            || is_denied_secret_snapshot_path(&verified_file)
        {
            return Ok(None);
        }
        let sha = sha256_hex(bytes);
        let root_str = canon_root.to_string_lossy().into_owned();
        let file_str = verified_file.to_string_lossy().into_owned();
        let rel_path = verified_file
            .strip_prefix(canon_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| file_str.clone());

        let (bucket, root_dir) = self.bucket_dir(canon_root);
        let _guard = self.lock_bucket(&bucket);

        let blobs_dir = root_dir.join("blobs");
        std::fs::create_dir_all(&blobs_dir)
            .map_err(|e| Error::Internal(format!("create local history dir: {e}")))?;

        let loaded = read_index(&root_dir);
        let mut idx = loaded.index;
        if skip_if_unchanged {
            if let Some(latest) = idx.entries.iter().rev().find(|s| s.entry.path == file_str) {
                if latest.entry.sha256 == sha && latest.entry.event_kind != "deleted" {
                    return Ok(Some(latest.entry.clone()));
                }
            }
        }

        let blob_path = blobs_dir.join(format!("{sha}.blob"));
        if !blob_path.is_file() {
            write_atomic_sync(&blob_path, bytes)?;
        }

        let ts = now_ms();
        idx.next_seq = idx.next_seq.saturating_add(1);
        let entry = IdeLocalHistoryEntry {
            id: format!("lh_{}", new_id()),
            root: root_str,
            path: file_str,
            rel_path,
            event_kind: event_kind.to_string(),
            label: label.to_string(),
            timestamp_ms: ts,
            size_bytes: bytes.len() as u64,
            sha256: sha,
        };
        idx.entries.push(StoredEntry {
            seq: idx.next_seq,
            entry: entry.clone(),
        });

        prune_entries(&mut idx, ts, retention_days);
        write_index(&root_dir, &idx)?;
        if loaded.allow_gc {
            gc_unreferenced_blobs(&blobs_dir, &idx);
        }
        Ok(Some(entry))
    }

    /// Snapshot an existing regular file on disk before edit/delete/restore if eligible.
    pub fn record_existing_file_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
    ) -> Option<IdeLocalHistoryEntry> {
        self.record_existing_file_with_retention_sync(
            canon_root,
            canon_file,
            event_kind,
            label,
            skip_if_unchanged,
            None,
        )
    }

    /// Snapshot an existing regular file on disk with optional retention days.
    pub fn record_existing_file_with_retention_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
        retention_days: Option<u32>,
    ) -> Option<IdeLocalHistoryEntry> {
        let sym_meta = std::fs::symlink_metadata(canon_file).ok()?;
        if sym_meta.file_type().is_symlink() || !sym_meta.is_file() {
            return None;
        }
        if sym_meta.len() > MAX_HISTORY_FILE_BYTES as u64 {
            return None;
        }
        let real_file = std::fs::canonicalize(canon_file).ok()?;
        if !real_file.starts_with(canon_root)
            || is_denied_secret_snapshot_path(canon_file)
            || is_denied_secret_snapshot_path(&real_file)
        {
            return None;
        }
        let bytes = std::fs::read(&real_file).ok()?;
        self.record_bytes_with_retention_sync(
            canon_root,
            &real_file,
            &bytes,
            event_kind,
            label,
            skip_if_unchanged,
            retention_days,
        )
        .ok()
        .flatten()
    }

    /// Walk up to [`MAX_DELETED_DIR_FILES`] non-symlink UTF-8 files inside `canon_dir`
    /// before deletion, strictly confining canonicalized targets to workspace roots.
    pub fn record_deleted_dir_sync(&self, canon_root: &Path, canon_dir: &Path) -> usize {
        self.record_deleted_dir_with_retention_sync(canon_root, &[], canon_dir, false, None)
    }

    pub fn record_deleted_dir_with_retention_sync(
        &self,
        canon_root: &Path,
        additional_roots: &[PathBuf],
        canon_dir: &Path,
        is_root_user: bool,
        retention_days: Option<u32>,
    ) -> usize {
        let walker = ignore::WalkBuilder::new(canon_dir)
            .hidden(false)
            .git_ignore(false)
            .follow_links(false)
            .max_depth(Some(12))
            .filter_entry(|e| {
                let name = e.file_name().to_str().unwrap_or("");
                !matches!(
                    name,
                    ".git" | "node_modules" | "target" | ".otto-local-history" | ".DS_Store"
                )
            })
            .build();

        let mut recorded = 0usize;
        for item in walker.flatten() {
            if recorded >= MAX_DELETED_DIR_FILES {
                break;
            }
            if !item.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let p = item.path();
            let Ok(canon_p) = std::fs::canonicalize(p) else {
                continue;
            };
            let Some(matched_root) = std::iter::once(canon_root)
                .chain(additional_roots.iter().map(PathBuf::as_path))
                .find(|r| canon_p.starts_with(r))
            else {
                continue;
            };
            if is_denied_secret_snapshot_path(p) || is_denied_secret_snapshot_path(&canon_p) {
                continue;
            }
            if !is_root_user
                && (enforce_non_root_path(p).is_err() || enforce_non_root_path(&canon_p).is_err())
            {
                continue;
            }
            if self
                .record_existing_file_with_retention_sync(
                    matched_root,
                    &canon_p,
                    "deleted",
                    "Deleted",
                    false,
                    retention_days,
                )
                .is_some()
            {
                recorded += 1;
            }
        }
        recorded
    }

    /// List Local History entries for a specific file or folder/root (skipping disjoint
    /// multi-root buckets and memoizing path validation per unique path).
    #[allow(clippy::too_many_arguments)]
    pub fn list_entries(
        &self,
        canon_root: &Path,
        additional_roots: &[PathBuf],
        is_root_user: bool,
        target_path: Option<&str>,
        include_deleted: bool,
        limit: Option<usize>,
        search_query: Option<&str>,
    ) -> Result<IdeHistoryListResp> {
        let filter_path = normalize_target_filter(canon_root, target_path)?;
        if !is_root_user {
            if let Some(fp) = &filter_path {
                enforce_non_root_path(Path::new(fp))?;
            }
        }
        let cap = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, 500);
        let q_lower = search_query
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        let folder_prefix = filter_path
            .as_ref()
            .map(|p| format!("{}/", p.trim_end_matches('/')));

        let mut seen_buckets = HashSet::new();
        let mut allowed_memo: HashMap<String, bool> = HashMap::new();
        let mut matched: Vec<StoredEntry> = Vec::new();

        for scan_root in
            std::iter::once(canon_root).chain(additional_roots.iter().map(PathBuf::as_path))
        {
            if !scan_root_overlaps_filter(scan_root, filter_path.as_deref()) {
                continue;
            }
            let (bucket, root_dir) = self.bucket_dir(scan_root);
            if !seen_buckets.insert(bucket.clone()) {
                continue;
            }
            let idx = {
                let _guard = self.lock_bucket(&bucket);
                read_index(&root_dir).index
            };
            let scan_root_str = scan_root.to_string_lossy();
            let blobs_dir = root_dir.join("blobs");

            matched.extend(idx.entries.into_iter().filter(|stored| {
                if !include_deleted && stored.entry.event_kind == "deleted" {
                    return false;
                }
                let path_ok = match (&filter_path, &folder_prefix) {
                    (None, _) => true,
                    (Some(fp), _) if fp == scan_root_str.as_ref() => true,
                    (Some(fp), Some(prefix)) => {
                        &stored.entry.path == fp || stored.entry.path.starts_with(prefix)
                    }
                    _ => true,
                };
                if !path_ok {
                    return false;
                }
                let allowed = *allowed_memo
                    .entry(stored.entry.path.clone())
                    .or_insert_with(|| {
                        entry_path_allowed(
                            canon_root,
                            additional_roots,
                            is_root_user,
                            &stored.entry.path,
                        )
                        .is_ok()
                    });
                if !allowed {
                    return false;
                }
                if let Some(q) = &q_lower {
                    if stored.entry.label.to_ascii_lowercase().contains(q)
                        || stored.entry.rel_path.to_ascii_lowercase().contains(q)
                    {
                        return true;
                    }
                    let blob_path = blobs_dir.join(format!("{}.blob", stored.entry.sha256));
                    let Ok(bytes) = std::fs::read(blob_path) else {
                        return false;
                    };
                    let Ok(text) = std::str::from_utf8(&bytes) else {
                        return false;
                    };
                    return text.to_ascii_lowercase().contains(q);
                }
                true
            }));
        }

        matched.sort_by(|a, b| {
            b.entry
                .timestamp_ms
                .cmp(&a.entry.timestamp_ms)
                .then_with(|| b.seq.cmp(&a.seq))
        });
        matched.truncate(cap);

        Ok(IdeHistoryListResp {
            entries: matched.into_iter().map(|s| s.entry).collect(),
        })
    }
}

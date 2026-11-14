//! Bounded, zero-cache, content-addressed Local History store (`ide-local-history`).
//!
//! Captures file creation, pre-edit external state, saves, renames, and
//! pre-delete file/directory snapshots under `<data_dir>/ide-local-history/`
//! so users can diff or restore any edited or deleted file without Git commits.
//!
//! Memory & disk bounds:
//! - Max [`MAX_HISTORY_ENTRIES_PER_ROOT`] (`250`) entries per project root.
//! - Max [`MAX_HISTORY_FILE_BYTES`] (`2 MiB`) per UTF-8 text file (binary files skipped).
//! - Content-addressed SHA-256 blobs: identical saves consume `0` extra bytes,
//!   and unreferenced blobs are garbage-collected automatically on prune.
//! - Zero persistent in-memory content cache (`< 1 MiB` steady-state RAM).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use otto_core::{new_id, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::target::enforce_non_root_path;
use crate::types::{
    IdeHistoryEntryResp, IdeHistoryListResp, IdeHistoryRestoreResp, IdeLocalHistoryEntry,
};

/// Maximum history timeline entries retained per project root.
pub const MAX_HISTORY_ENTRIES_PER_ROOT: usize = 250;
/// Maximum file size eligible for Local History snapshots (`2 MiB`).
pub const MAX_HISTORY_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum files snapshotted when deleting a directory tree.
pub const MAX_DELETED_DIR_FILES: usize = 200;
/// Default entry count returned by `GET /ide/history/list`.
const DEFAULT_LIST_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    #[serde(default)]
    seq: u64,
    #[serde(flatten)]
    entry: IdeLocalHistoryEntry,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RootIndex {
    #[serde(default)]
    next_seq: u64,
    #[serde(default)]
    entries: Vec<StoredEntry>,
}

/// Thread-safe, disk-backed Local History store with zero retained blob cache.
#[derive(Clone)]
pub struct LocalHistoryStore {
    base_dir: PathBuf,
    root_locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl LocalHistoryStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            base_dir: data_dir.join("ide-local-history"),
            root_locks: Arc::new(DashMap::new()),
        }
    }

    fn lock_for_bucket(&self, bucket: &str) -> Arc<Mutex<()>> {
        if self.root_locks.len() > 128 {
            self.root_locks.retain(|_, v| Arc::strong_count(v) > 1);
        }
        self.root_locks
            .entry(bucket.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
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
    /// When `skip_if_unchanged` is true, skips recording if the latest entry for
    /// `canon_file` already has the exact same `sha256` and `event_kind != "deleted"`.
    pub fn record_bytes_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        bytes: &[u8],
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
    ) -> Result<Option<IdeLocalHistoryEntry>> {
        if bytes.len() > MAX_HISTORY_FILE_BYTES || is_binary_or_invalid_utf8(bytes) {
            return Ok(None);
        }
        let sha = sha256_hex(bytes);
        let root_str = canon_root.to_string_lossy().into_owned();
        let file_str = canon_file.to_string_lossy().into_owned();
        let rel_path = canon_file
            .strip_prefix(canon_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| file_str.clone());

        let (bucket, root_dir) = self.bucket_dir(canon_root);
        let lock = self.lock_for_bucket(&bucket);
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        let blobs_dir = root_dir.join("blobs");
        std::fs::create_dir_all(&blobs_dir)
            .map_err(|e| Error::Internal(format!("create local history dir: {e}")))?;

        let mut idx = read_index(&root_dir);
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

        idx.next_seq = idx.next_seq.saturating_add(1);
        let entry = IdeLocalHistoryEntry {
            id: format!("lh_{}", new_id()),
            root: root_str,
            path: file_str,
            rel_path,
            event_kind: event_kind.to_string(),
            label: label.to_string(),
            timestamp_ms: now_ms(),
            size_bytes: bytes.len() as u64,
            sha256: sha,
        };
        idx.entries.push(StoredEntry {
            seq: idx.next_seq,
            entry: entry.clone(),
        });

        prune_and_gc(&root_dir, &blobs_dir, &mut idx);
        write_index(&root_dir, &idx)?;
        Ok(Some(entry))
    }

    /// Snapshot an existing file on disk before edit/delete/restore if eligible.
    pub fn record_existing_file_sync(
        &self,
        canon_root: &Path,
        canon_file: &Path,
        event_kind: &str,
        label: &str,
        skip_if_unchanged: bool,
    ) -> Option<IdeLocalHistoryEntry> {
        let meta = std::fs::metadata(canon_file).ok()?;
        if !meta.is_file() || meta.len() > MAX_HISTORY_FILE_BYTES as u64 {
            return None;
        }
        let bytes = std::fs::read(canon_file).ok()?;
        self.record_bytes_sync(
            canon_root,
            canon_file,
            &bytes,
            event_kind,
            label,
            skip_if_unchanged,
        )
        .ok()
        .flatten()
    }

    /// Walk up to [`MAX_DELETED_DIR_FILES`] UTF-8 files inside `canon_dir`
    /// before deletion (skipping `.git`, `node_modules`, `target`) and snapshot each.
    pub fn record_deleted_dir_sync(&self, canon_root: &Path, canon_dir: &Path) -> usize {
        let walker = ignore::WalkBuilder::new(canon_dir)
            .hidden(false)
            .git_ignore(false)
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
            let p = item.path();
            if p.is_file()
                && self
                    .record_existing_file_sync(canon_root, p, "deleted", "Deleted", false)
                    .is_some()
            {
                recorded += 1;
            }
        }
        recorded
    }

    /// List Local History entries for a specific file or folder/root (including deleted files
    /// and optional case-insensitive `search_query` across metadata and snapshot blobs).
    pub fn list_entries(
        &self,
        canon_root: &Path,
        target_path: Option<&str>,
        include_deleted: bool,
        limit: Option<usize>,
        search_query: Option<&str>,
    ) -> Result<IdeHistoryListResp> {
        let (bucket, root_dir) = self.bucket_dir(canon_root);
        let lock = self.lock_for_bucket(&bucket);
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        let idx = read_index(&root_dir);
        let filter_path = normalize_target_filter(canon_root, target_path)?;
        let root_str = canon_root.to_string_lossy();
        let cap = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, 500);
        let q_lower = search_query
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        let blobs_dir = root_dir.join("blobs");

        let folder_prefix = filter_path
            .as_ref()
            .map(|p| format!("{}/", p.trim_end_matches('/')));

        let mut matched: Vec<StoredEntry> = idx
            .entries
            .into_iter()
            .filter(|stored| {
                if !include_deleted && stored.entry.event_kind == "deleted" {
                    return false;
                }
                let path_ok = match (&filter_path, &folder_prefix) {
                    (None, _) => true,
                    (Some(fp), _) if fp == root_str.as_ref() => true,
                    (Some(fp), Some(prefix)) => {
                        &stored.entry.path == fp || stored.entry.path.starts_with(prefix)
                    }
                    _ => true,
                };
                if !path_ok {
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
            })
            .collect();

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

    /// Read a single Local History entry's blob content alongside current disk content.
    pub fn get_entry(&self, canon_root: &Path, entry_id: &str) -> Result<IdeHistoryEntryResp> {
        let (bucket, root_dir) = self.bucket_dir(canon_root);
        let (entry, content) = {
            let lock = self.lock_for_bucket(&bucket);
            let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            let idx = read_index(&root_dir);
            let found = idx
                .entries
                .into_iter()
                .find(|s| s.entry.id == entry_id)
                .map(|s| s.entry)
                .ok_or_else(|| {
                    Error::NotFound(format!("local history entry '{entry_id}' not found"))
                })?;
            let blob_path = root_dir
                .join("blobs")
                .join(format!("{}.blob", found.sha256));
            let bytes = std::fs::read(&blob_path).map_err(|_| {
                Error::NotFound(format!(
                    "local history blob for entry '{entry_id}' was pruned or missing"
                ))
            })?;
            let text = String::from_utf8(bytes)
                .map_err(|_| Error::Internal("stored local history blob is not UTF-8".into()))?;
            (found, text)
        };

        let disk_path = Path::new(&entry.path);
        let (current_exists, current_content) = match std::fs::metadata(disk_path) {
            Ok(m) if m.is_file() && m.len() <= MAX_HISTORY_FILE_BYTES as u64 => {
                match std::fs::read(disk_path) {
                    Ok(b) if !is_binary_or_invalid_utf8(&b) => (true, String::from_utf8(b).ok()),
                    _ => (true, None),
                }
            }
            Ok(m) if m.is_file() => (true, None),
            _ => (false, None),
        };

        Ok(IdeHistoryEntryResp {
            entry,
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
        let detail = self.get_entry(canon_root, entry_id)?;
        let target_file = PathBuf::from(&detail.entry.path);
        let prospective = otto_core::secret_paths::canonicalize_prospective(&target_file)
            .ok_or_else(|| Error::Invalid("invalid restore target path".into()))?;

        let allowed = std::iter::once(canon_root)
            .chain(additional_roots.iter().map(PathBuf::as_path))
            .any(|r| prospective.starts_with(r));
        if !allowed {
            return Err(Error::Forbidden(format!(
                "restore target '{}' escapes workspace root '{}'",
                prospective.display(),
                canon_root.display()
            )));
        }
        if !is_root_user {
            enforce_non_root_path(&prospective)?;
        }

        let existed_before = prospective.is_file();
        if existed_before {
            let _ = self.record_existing_file_sync(
                canon_root,
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
            canon_root,
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

fn normalize_target_filter(canon_root: &Path, raw: Option<&str>) -> Result<Option<String>> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty() && *s != ".") else {
        return Ok(None);
    };
    let candidate = if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else {
        let p = Path::new(trimmed);
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

fn is_binary_or_invalid_utf8(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_index(root_dir: &Path) -> RootIndex {
    let path = root_dir.join("index.json");
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<RootIndex>(&b).ok())
        .unwrap_or_default()
}

fn write_index(root_dir: &Path, idx: &RootIndex) -> Result<()> {
    let bytes = serde_json::to_vec(idx)
        .map_err(|e| Error::Internal(format!("serialize local history index: {e}")))?;
    write_atomic_sync(&root_dir.join("index.json"), &bytes)
}

fn write_atomic_sync(dest: &Path, bytes: &[u8]) -> Result<()> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = dest
        .parent()
        .ok_or_else(|| Error::Invalid("path has no parent directory".into()))?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".otto-lh-{}-{seq}.tmp", std::process::id()));
    let existing_perms = std::fs::metadata(dest).ok().map(|m| m.permissions());
    std::fs::write(&tmp, bytes)
        .map_err(|e| Error::Internal(format!("write temp file '{}': {e}", tmp.display())))?;
    if let Some(perms) = existing_perms {
        let _ = std::fs::set_permissions(&tmp, perms);
    }
    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Internal(format!(
            "commit atomic write '{}': {e}",
            dest.display()
        )));
    }
    Ok(())
}

#[allow(clippy::disallowed_methods)] // bounded blob directory GC (<= 251 files)
fn prune_and_gc(root_dir: &Path, blobs_dir: &Path, idx: &mut RootIndex) {
    let _ = root_dir;
    if idx.entries.len() > MAX_HISTORY_ENTRIES_PER_ROOT {
        let excess = idx.entries.len() - MAX_HISTORY_ENTRIES_PER_ROOT;
        idx.entries.drain(0..excess);
    }
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

//! Hand-edit protection for the provider copies [`super`] writes.
//!
//! People (and agents such as Claude Code's skill-creator) edit the copy in
//! `~/.claude/skills/<name>/` directly. Otto therefore records, per provider
//! skills dir, the tree hash of every copy it wrote
//! (`<dir>/.otto-managed-hashes.json`). Before it replaces or removes an owned
//! copy whose tree no longer hashes to that record — or that has no record
//! (installed by an older Otto) and differs from what is about to be written —
//! the copy is MOVED to `<library>/provider-skills-backup/<provider>/
//! <name>-<secs>-<id>/` (newest [`PROVIDER_BACKUPS_KEPT`] per provider + name).
//! That folder is separate from `skills-backup/`, whose own pruning would
//! otherwise push out the Library's delete backups.
//!
//! A missing or unreadable hash record only ever costs an extra backup, never
//! a lost edit: every doubt resolves to "keep it".

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

/// `<library>/provider-skills-backup/`: where replaced or removed hand-edited
/// provider copies are kept.
pub const PROVIDER_BACKUP_DIR: &str = "provider-skills-backup";

/// Backups kept per provider + skill name; older ones are pruned.
pub const PROVIDER_BACKUPS_KEPT: usize = 3;

/// The per-dir record of the tree hash Otto last wrote for each owned copy.
const HASHES_FILE: &str = ".otto-managed-hashes.json";

/// A hand-edited provider copy Otto moved aside before replacing or removing
/// it. Paths are display strings: this is what the UI tells the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeptCopy {
    /// The provider copy that was replaced or removed (`~/.claude/skills/<name>`).
    pub copy: String,
    /// Where its tree now lives (`<library>/provider-skills-backup/<provider>/<name>-<secs>-<id>`).
    pub kept_at: String,
}

/// Entries never copied into (nor hashed in) a provider copy: the improvement
/// engine's `.<file>.bak-<timestamp>` rollback trail, the Library's
/// `.<file>.otto-tmp-…` write temps (left by a crash) and Finder's
/// `.DS_Store`. Every other dot-file a skill ships (`.gitignore`, `.gitkeep`,
/// `.env.example`) is copied as is.
pub(super) fn is_skipped(name: &OsStr) -> bool {
    name == ".DS_Store" || is_otto_artifact(name)
}

/// Files Otto itself leaves inside a Library skill (`.<file>.bak-<digits>…`,
/// `.<file>.otto-tmp-…`). Older Otto builds copied them into provider copies.
fn is_otto_artifact(name: &OsStr) -> bool {
    let Some(name) = name.to_str().filter(|n| n.starts_with('.')) else {
        return false;
    };
    name.contains(".otto-tmp-")
        || name.rsplit_once(".bak-").is_some_and(|(stem, stamp)| {
            stem.len() > 1 && stamp.starts_with(|c: char| c.is_ascii_digit())
        })
}

/// Whether a provider copy still carries [`is_otto_artifact`] files.
fn has_otto_artifacts(dir: &Path) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        is_otto_artifact(&e.file_name())
            || (e.file_type().is_ok_and(|t| t.is_dir()) && has_otto_artifacts(&e.path()))
    })
}

/// Recursively copy a skill tree from `src` to `dest`, creating parents and
/// leaving out [`is_skipped`] entries. `std::fs::copy` preserves the source
/// mode, so `.sh` exec bits survive.
pub(super) fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if is_skipped(&entry.file_name()) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// A content hash over exactly the files [`copy_tree`] copies: each one's
/// tree-relative path and bytes, in path order. So the hash of a Library tree
/// equals the hash of the copy made from it.
pub fn tree_hash(dir: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut manifest = String::new();
    for (rel, path) in files {
        let bytes = fs::read(&path)?;
        manifest.push_str(&format!(
            "{rel}\0{}\n",
            otto_core::text::bytes_sha256(&bytes)
        ));
    }
    Ok(otto_core::text::content_sha256(&manifest))
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if is_skipped(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect(base, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), path));
        }
    }
    Ok(())
}

/// The hash record of `dir`. Missing or malformed reads as empty: every copy
/// then counts as unrecorded, which only ever means an extra backup.
fn read_hashes(dir: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(dir.join(HASHES_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(super) fn recorded_hash(dir: &Path, name: &str) -> Option<String> {
    read_hashes(dir).remove(name)
}

/// Set (`Some`) or drop (`None`) the record for `name`, atomically.
fn set_recorded_hash(dir: &Path, name: &str, hash: Option<&str>) -> io::Result<()> {
    let mut map = read_hashes(dir);
    let changed = match hash {
        Some(h) => map.insert(name.to_string(), h.to_string()).as_deref() != Some(h),
        None => map.remove(name).is_some(),
    };
    if !changed {
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    crate::library::write_atomic(&dir.join(HASHES_FILE), json.as_bytes())
}

/// Record what Otto just wrote at `dir/<name>` (`None`: Otto no longer has a
/// copy there). Best effort: a failed record only costs a later extra backup.
pub(super) fn note(dir: &Path, name: &str, hash: Option<&str>) {
    if let Err(e) = set_recorded_hash(dir, name, hash) {
        tracing::warn!(dir = %dir.display(), skill = %name, error = %e, "record provider skill hash failed");
    }
}

/// Put a fresh copy of `src` at the Otto-owned copy `dest` (`dir/<name>`).
/// A copy that already matches `src` is left as it is. A copy the person
/// edited since Otto wrote it (or one with no record that differs from `src`)
/// is moved to the backup folder first, and returned. The new tree is copied
/// beside `dest` before anything moves, so a failed copy changes nothing.
pub(super) fn replace_owned(
    library_root: &Path,
    dir: &Path,
    name: &str,
    src: &Path,
    dest: &Path,
) -> io::Result<Option<KeptCopy>> {
    let new_hash = tree_hash(src)?;
    let current = tree_hash(dest).ok();
    let hand_edited = match current.as_deref() {
        // Already what would be written. Rewritten only to drop the `.bak`
        // files an older Otto copied in; those are Otto's, never an edit.
        Some(now) if now == new_hash => {
            if !has_otto_artifacts(dest) {
                note(dir, name, Some(&new_hash));
                return Ok(None);
            }
            false
        }
        Some(_) => recorded_hash(dir, name) != current,
        None => true, // unreadable: keep it rather than guess
    };
    let fresh = stage(src, dest)?;
    let mut kept = None;
    if hand_edited {
        match keep_aside(library_root, dir, name, dest) {
            Ok((target, true)) => {
                // `dest` is gone; on a failed rename put the person's tree back.
                if let Err(e) = fs::rename(&fresh, dest) {
                    let _ = fs::rename(&target, dest);
                    let _ = fs::remove_dir_all(&fresh);
                    return Err(e);
                }
                note(dir, name, Some(&new_hash));
                return Ok(Some(kept_copy(dest, &target)));
            }
            Ok((target, false)) => kept = Some(kept_copy(dest, &target)),
            Err(e) => {
                let _ = fs::remove_dir_all(&fresh);
                return Err(e);
            }
        }
    }
    swap(&fresh, dest)?;
    note(dir, name, Some(&new_hash));
    Ok(kept)
}

/// Remove the Otto-owned copy `dest`; a hand-edited (or unrecorded) one is
/// moved to the backup folder instead, and returned.
pub(super) fn remove_owned(
    library_root: &Path,
    dir: &Path,
    name: &str,
    dest: &Path,
) -> io::Result<Option<KeptCopy>> {
    let current = tree_hash(dest).ok();
    if current.is_some() && current == recorded_hash(dir, name) {
        fs::remove_dir_all(dest)?;
        return Ok(None);
    }
    let (target, moved) = keep_aside(library_root, dir, name, dest)?;
    if !moved {
        fs::remove_dir_all(dest)?;
    }
    Ok(Some(kept_copy(dest, &target)))
}

/// Copy `src` into a hidden sibling of `dest` (the provider dir is created on
/// demand) and return it. A failed copy leaves nothing behind.
pub(super) fn stage(src: &Path, dest: &Path) -> io::Result<PathBuf> {
    let (parent, leaf) = split(dest)?;
    fs::create_dir_all(parent)?;
    let fresh = parent.join(format!(".{leaf}.otto-new-{}", tag()));
    if let Err(e) = copy_tree(src, &fresh) {
        let _ = fs::remove_dir_all(&fresh);
        return Err(e);
    }
    Ok(fresh)
}

/// Swap the staged tree `fresh` in for `dest` with renames, then drop the old
/// tree (Otto's own, unedited copy). Any failure keeps `dest` as it was.
fn swap(fresh: &Path, dest: &Path) -> io::Result<()> {
    let (parent, leaf) = split(dest)?;
    let old = parent.join(format!(".{leaf}.otto-old-{}", tag()));
    fs::rename(dest, &old).inspect_err(|_| {
        let _ = fs::remove_dir_all(fresh);
    })?;
    if let Err(e) = fs::rename(fresh, dest) {
        let _ = fs::rename(&old, dest);
        let _ = fs::remove_dir_all(fresh);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

/// Move `dest` to a new `<library>/provider-skills-backup/<provider>/<name>-
/// <secs>-<id>` dir. `true` = moved. When the rename fails (the provider dir
/// is on another volume) the tree is copied whole instead (`false`: `dest`
/// is still there for the caller to replace or remove).
fn keep_aside(
    library_root: &Path,
    dir: &Path,
    name: &str,
    dest: &Path,
) -> io::Result<(PathBuf, bool)> {
    let parent = library_root
        .join(PROVIDER_BACKUP_DIR)
        .join(provider_label(dir));
    fs::create_dir_all(&parent)?;
    let secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target = parent.join(format!("{name}-{secs}-{}", otto_core::new_id()));
    let moved = match fs::rename(dest, &target) {
        Ok(()) => true,
        Err(_) => {
            if let Err(e) = copy_exact(dest, &target) {
                let _ = fs::remove_dir_all(&target);
                return Err(e);
            }
            false
        }
    };
    tracing::info!(copy = %dest.display(), kept_at = %target.display(), "kept a hand-edited provider skill copy");
    prune(&parent, name, PROVIDER_BACKUPS_KEPT, &target);
    Ok((target, moved))
}

/// A byte-for-byte copy for a backup: every entry, dot-files included, and
/// symlinks recreated as links (never followed).
fn copy_exact(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_exact(&from, &to)?;
        } else if ty.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(&from)?, &to)?;
            #[cfg(not(unix))]
            fs::copy(&from, &to).map(|_| ())?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Keep only the newest `keep` `<name>-<secs>-<id>` backups in `parent`
/// (best effort), `newest` always among them: ids from one millisecond do not
/// sort by time. Another skill whose name merely starts with this one
/// (`vault` vs `vault-2-<secs>-<id>`) never matches the exact shape.
fn prune(parent: &Path, name: &str, keep: usize, newest: &Path) {
    let Ok(rd) = fs::read_dir(parent) else {
        return;
    };
    let prefix = format!("{name}-");
    let mut dirs: Vec<((u64, String), PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let file = e.file_name();
            let (secs, id) = file.to_str()?.strip_prefix(&prefix)?.split_once('-')?;
            let ulid = id.len() == 26 && id.chars().all(|c| c.is_ascii_alphanumeric());
            if !ulid || secs.is_empty() || !secs.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let secs = secs.parse::<u64>().ok()?;
            if e.path() == newest {
                return None;
            }
            e.file_type()
                .ok()?
                .is_dir()
                .then(|| ((secs, id.to_string()), e.path()))
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in dirs.into_iter().skip(keep.saturating_sub(1)) {
        let _ = fs::remove_dir_all(path);
    }
}

/// `~/.claude/skills` → `claude`, `~/.codex/skills` → `codex`,
/// `~/.gemini/skills` → `gemini`; any other dir → its own name.
fn provider_label(dir: &Path) -> String {
    let leaf = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().trim_start_matches('.').to_string())
    };
    let label = if dir.file_name().is_some_and(|n| n == "skills") {
        dir.parent().and_then(leaf)
    } else {
        leaf(dir)
    };
    label
        .filter(|l| otto_core::paths::safe_component(l).is_some())
        .unwrap_or_else(|| "provider".to_string())
}

fn kept_copy(dest: &Path, target: &Path) -> KeptCopy {
    KeptCopy {
        copy: dest.to_string_lossy().into_owned(),
        kept_at: target.to_string_lossy().into_owned(),
    }
}

fn split(dest: &Path) -> io::Result<(&Path, String)> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent dir"))?;
    let leaf = dest
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no dir name"))?
        .to_string_lossy()
        .into_owned();
    Ok((parent, leaf))
}

/// A process-unique suffix for staging/swap siblings.
fn tag() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

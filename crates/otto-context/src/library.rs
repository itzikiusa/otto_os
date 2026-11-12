//! The Otto library store on disk: skills, souls, and reusable context
//! snippets, plus the global-default-soul pointer.
//!
//! Layout under `root`:
//! - `skills/<name>/SKILL.md`
//! - `souls/<name>.md`
//! - `context/<name>.md`
//! - `default-soul.txt`  (single line, the default soul name)
//!
//! Entry names are validated as safe single segments (alphanumeric / `-` / `_`,
//! non-empty, not `.` or `..`), mirroring `otto-improve::pathsafe`, to prevent
//! path traversal into (or out of) the library tree.
//!
//! ## Skill cache
//!
//! `list_skills` and `get_skill` parse the YAML frontmatter (description,
//! category, version) on every call. With many skills or frequent spawns this
//! adds up. We keep an in-process `Arc<Mutex<HashMap<name, LibrarySkill>>>` that
//! is populated on first parse and invalidated (evicted) when a skill is written
//! or deleted. Each entry also keeps the `SKILL.md` stamp (mtime, len) it was
//! read at, and `get_skill` re-stats the file: writers that bypass `Library`
//! (the improve engine, bundled `install_into`, a hand edit) are picked up on
//! the next read instead of serving the pre-edit body until a restart. Reads
//! hold the lock only for the map lookup; the actual `fs::read_to_string` +
//! parse happen outside the lock, followed by a brief re-acquire to insert.
//! The cache is entirely best-effort: a poisoned mutex falls back to the
//! direct-disk path.
//!
//! ## Writes
//!
//! The library is the source of truth (provider bundles are regenerated from
//! it), so every overwrite goes through [`write_atomic`]: a crash or a full
//! disk mid-write leaves the old file, never a truncated one.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use otto_core::api::{LibraryContext, LibrarySkill, LibrarySoul, SkillFileEntry};
use otto_core::paths::confine_join;

/// Largest single skill file read/written through the editor (2 MiB).
const MAX_SKILL_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Largest total uncompressed size accepted from a skill-package import (16 MiB).
const MAX_SKILL_IMPORT_BYTES: u64 = 16 * 1024 * 1024;

/// In-process skill cache: keyed by skill name, holds the last-read
/// `LibrarySkill` and the `SKILL.md` stamp it was read at. Wrapped in
/// `Arc<Mutex<…>>` so `Library::clone` shares the same cache across all handle
/// copies (e.g., the Axum state clone).
type SkillCache = Arc<Mutex<HashMap<String, (FileStamp, LibrarySkill)>>>;

/// What `get_skill` compares to decide a cached entry is still the file on
/// disk: modification time + length.
type FileStamp = (Option<SystemTime>, u64);

fn stamp_of(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

/// Replace `path` with `bytes` without ever leaving it truncated: write a temp
/// sibling, `sync_all`, rename it over the target, then fsync the directory.
/// A symlinked target keeps its link (the link's target is replaced), and an
/// existing file keeps its permissions (an executable `*.sh` stays executable).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent dir"))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy();
    let tmp = dir.join(format!(
        ".{file_name}.otto-tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, bytes)?;
        if let Ok(meta) = fs::metadata(&target) {
            let _ = f.set_permissions(meta.permissions());
        }
        f.sync_all()?;
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Persist the rename itself (best-effort: not every FS allows it).
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// A conditional skill-file write lost: the file is no longer the version the
/// writer read. `current` is its hash now (`None` = the file is gone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleSkillFile {
    pub current: Option<String>,
}

/// Serializes the editor's check-then-write so the `if_hash` test and the
/// write are one step for every in-process writer that goes through Library.
static SKILL_WRITES: Mutex<()> = Mutex::new(());

fn skill_write_guard() -> std::sync::MutexGuard<'static, ()> {
    SKILL_WRITES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Backups kept per skill in `<library>/skills-backup/`; older ones are pruned
/// after each new backup (bundled updates and deletes share the folder).
pub const SKILL_BACKUPS_KEPT: usize = 3;

/// Atomically reserve a unique, empty `<root>/skills-backup/<name>-<secs>-<id>`
/// dir; two backups in one second must not merge their safety copies. Legacy
/// `<name>-<secs>` directories remain valid.
pub fn reserve_skill_backup(root: &Path, name: &str) -> io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let parent = root.join("skills-backup");
    fs::create_dir_all(&parent)?;
    loop {
        let dest = parent.join(format!("{name}-{secs}-{}", otto_core::new_id()));
        match fs::create_dir(&dest) {
            Ok(()) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Keep only the newest `keep` `skills-backup/<name>-<secs>` dirs for `name`
/// (best effort: a failed removal is left for the next backup).
pub fn prune_skill_backups(root: &Path, name: &str, keep: usize) {
    let Ok(rd) = fs::read_dir(root.join("skills-backup")) else {
        return;
    };
    let prefix = format!("{name}-");
    let mut dirs: Vec<((u64, String), PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let file = e.file_name();
            let suffix = file.to_str()?.strip_prefix(&prefix)?;
            let (timestamp, unique) = suffix.split_once('-').unwrap_or((suffix, ""));
            // Exactly `<secs>` (legacy) or `<secs>-<ULID>`: anything else is
            // another skill whose name merely starts with this one ("vault"
            // vs "vault-2-<secs>-<id>"), whose backups must never be pruned.
            let ulid = unique.len() == 26 && unique.chars().all(|c| c.is_ascii_alphanumeric());
            if !(unique.is_empty() || ulid) || !timestamp.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let secs = timestamp.parse::<u64>().ok()?;
            e.file_type()
                .ok()?
                .is_dir()
                .then(|| ((secs, unique.to_string()), e.path()))
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, d) in dirs.into_iter().skip(keep) {
        let _ = fs::remove_dir_all(d);
    }
}

/// Recursively copy a directory tree (files keep their mode via `fs::copy`).
fn copy_dir(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Handle to the on-disk library rooted at `root`.
#[derive(Clone)]
pub struct Library {
    pub root: PathBuf,
    /// Shared across clones — invalidated on writes/deletes.
    skill_cache: SkillCache,
}

/// An entry name must be a single safe path segment.
pub(crate) fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Validate a relative path *within* a skill dir: non-empty, no absolute root,
/// no `.`/`..` segments, and each segment restricted to safe filename chars
/// (alphanumeric plus `-`, `_`, `.`). Returns the assembled relative `PathBuf`,
/// or `None` when the path is unsafe. Unlike [`is_safe_segment`], this permits
/// the `/` separators of nested paths like `references/rubric.md`.
pub(crate) fn safe_rel(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    // Unix-style separators only; a backslash is treated as an (illegal) segment
    // char, so Windows-style input is rejected here — the zip importer normalizes
    // `\`→`/` before calling this.
    let mut out = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return None;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return None;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Join a [`safe_rel`]-vetted relative path under `root` through the shared
/// [`confine_join`] barrier. The charset policy stays ours (`safe_rel`); the
/// confinement proof (no escape from `root`) comes from the shared helper.
pub(crate) fn confined(root: &std::path::Path, rel: &str) -> Option<PathBuf> {
    safe_rel(rel)?;
    confine_join(root, rel)
}

/// Best-effort binary sniff: a NUL byte in the leading window ⇒ binary.
pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

/// Recursively collect files under `root` as [`SkillFileEntry`] with paths
/// relative to `base`. Symlinks and unreadable entries are skipped.
pub(crate) fn collect_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<SkillFileEntry>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            collect_files(base, &path, out);
        } else if ft.is_file() {
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            // Sniff only a small head for the binary flag.
            let binary = {
                let mut head = [0u8; 512];
                fs::File::open(&path)
                    .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
                    .map(|n| is_binary(&head[..n]))
                    .unwrap_or(false)
            };
            out.push(SkillFileEntry {
                path: rel.to_string_lossy().replace('\\', "/"),
                size,
                binary,
            });
        }
    }
}

/// A minimal starter `SKILL.md` for a newly created library skill.
fn starter_skill_md(name: &str, category: &str, description: &str) -> String {
    let category = if category.trim().is_empty() {
        "development"
    } else {
        category.trim()
    };
    let description = if description.trim().is_empty() {
        format!("What {name} does and when to use it.")
    } else {
        description.trim().to_string()
    };
    format!(
        "---\ndescription: {description}\ncategory: {category}\nversion: 1\n---\n\n# {name}\n\nDescribe the method here.\n"
    )
}

/// Parse a single scalar `<key>:` value out of a YAML frontmatter block, if
/// present. Only looks inside a leading `---` / `---` fenced block, takes the
/// first matching key, and strips surrounding quotes. Returns `None` when there
/// is no frontmatter or the key is absent.
fn parse_frontmatter(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let val = rest.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val);
            return Some(val.to_string());
        }
    }
    None
}

/// Parse the `description:` value from frontmatter; `""` when absent.
pub(crate) fn parse_description(body: &str) -> String {
    parse_frontmatter(body, "description").unwrap_or_default()
}

/// Parse the `category:` value from frontmatter; `""` when absent.
pub(crate) fn parse_category(body: &str) -> String {
    parse_frontmatter(body, "category").unwrap_or_default()
}

/// Parse the `version:` value from frontmatter; defaults to `1` when absent or
/// unparseable.
fn parse_version(body: &str) -> u32 {
    parse_frontmatter(body, "version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

impl Library {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            skill_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // -- skills --------------------------------------------------------------

    fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Absolute path of a skill file, regardless of whether it exists. Returns
    /// `None` for unsafe names. Used by the self-improvement repoint to target
    /// the library copy of a skill.
    pub fn skill_path(&self, name: &str) -> Option<PathBuf> {
        Some(self.skill_dir(name)?.join("SKILL.md"))
    }

    pub fn list_skills(&self) -> Vec<LibrarySkill> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(self.skills_dir()) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_safe_segment(&name) {
                continue;
            }
            if let Some(skill) = self.get_skill(&name) {
                out.push(skill);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn get_skill(&self, name: &str) -> Option<LibrarySkill> {
        let path = self.skill_path(name)?;

        // Cache hit only while the file still has the stamp it was read at:
        // one `stat`, no read/parse.
        let Some(stamp) = stamp_of(&path) else {
            self.evict(name);
            return None;
        };
        if let Ok(cache) = self.skill_cache.lock() {
            if let Some((cached_at, cached)) = cache.get(name) {
                if *cached_at == stamp {
                    return Some(cached.clone());
                }
            }
        }

        // Miss or stale: read and parse outside the lock. The entry is stored
        // under the stamp taken BEFORE the read, so a write that lands after
        // it (even one whose evict ran before our insert) changes the stamp
        // and the next call re-reads instead of pinning this body.
        let body = fs::read_to_string(&path).ok()?;
        let description = parse_description(&body);
        let category = parse_category(&body);
        let version = parse_version(&body);
        let skill = LibrarySkill {
            name: name.to_string(),
            category,
            version,
            description,
            body,
        };

        // Insert into cache (best-effort — a poisoned mutex is ignored).
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.insert(name.to_string(), (stamp, skill.clone()));
        }
        Some(skill)
    }

    pub fn put_skill(&self, name: &str, body: &str) -> io::Result<()> {
        let path = self
            .skill_path(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&path, body.as_bytes())?;
        // Evict the stale cached entry so the next read picks up the new content.
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.remove(name);
        }
        Ok(())
    }

    /// Delete a skill from the Library by MOVING its whole tree into
    /// `<library>/skills-backup/<name>-<secs>-<id>/` (the same folder the
    /// bundled-skill updates back up into), so a deleted skill can be put back
    /// by hand. Returns that folder, or `None` when there was no such skill.
    pub fn delete_skill(&self, name: &str) -> io::Result<Option<PathBuf>> {
        let dir = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let dest = reserve_skill_backup(&self.root, name)?;
        // The reserved dir is empty, so a rename may replace it (same volume).
        // A failed rename falls back to copy-then-remove; the copy must finish
        // before the original goes.
        if fs::rename(&dir, &dest).is_err() {
            copy_dir(&dir, &dest)?;
            fs::remove_dir_all(&dir)?;
        }
        self.evict(name);
        prune_skill_backups(&self.root, name, SKILL_BACKUPS_KEPT);
        Ok(Some(dest))
    }

    /// SHA-256 (hex) of one file in a library skill exactly as it is on disk,
    /// or `None` when it does not exist. The Skills Lab editor sends it back as
    /// `if_hash` so a save never silently overwrites a newer edit.
    pub fn skill_file_hash(&self, name: &str, rel: &str) -> Option<String> {
        let target = confined(&self.skill_dir(name)?, rel)?;
        fs::read(target)
            .ok()
            .map(|b| otto_core::text::bytes_sha256(&b))
    }

    /// [`Self::write_skill_file`], but only when the file on disk still hashes
    /// to `if_hash` (what the writer read). A mismatch writes nothing and
    /// returns `Ok(Err(StaleSkillFile))` carrying the current hash. The check
    /// and the write hold one lock, so two in-process editors can't both win.
    pub fn write_skill_file_if(
        &self,
        name: &str,
        rel: &str,
        content: &str,
        if_hash: &str,
    ) -> io::Result<Result<(), StaleSkillFile>> {
        let _write = skill_write_guard();
        let current = self.skill_file_hash(name, rel);
        if current.as_deref() != Some(if_hash) {
            return Ok(Err(StaleSkillFile { current }));
        }
        self.write_skill_file_unlocked(name, rel, content)?;
        Ok(Ok(()))
    }

    // -- skill files (multi-file editor) -------------------------------------

    /// Absolute path of a skill's root dir in the library, or `None` for an
    /// unsafe name. The dir need not exist.
    pub fn skill_dir(&self, name: &str) -> Option<PathBuf> {
        if !is_safe_segment(name) {
            return None;
        }
        // The name is already a vetted single segment; the join still goes
        // through the shared confine_join barrier so the result is provably
        // under the skills dir.
        confine_join(&self.skills_dir(), name)
    }

    /// List every file inside a library skill (recursively), relative to the
    /// skill dir, sorted by path. Empty when the skill does not exist.
    pub fn list_skill_files(&self, name: &str) -> Vec<SkillFileEntry> {
        let Some(root) = self.skill_dir(name) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        collect_files(&root, &root, &mut out);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Read one file inside a library skill. Returns `(content, binary)`; binary
    /// files are returned lossily so the UI can display them read-only. Reads at
    /// most `MAX_SKILL_FILE_BYTES`.
    pub fn read_skill_file(&self, name: &str, rel: &str) -> Option<(String, bool)> {
        let root = self.skill_dir(name)?;
        let target = confined(&root, rel)?;
        let bytes = fs::read(&target).ok()?;
        let bytes = &bytes[..bytes.len().min(MAX_SKILL_FILE_BYTES)];
        let binary = is_binary(bytes);
        Some((String::from_utf8_lossy(bytes).into_owned(), binary))
    }

    /// Create or overwrite one file inside an existing library skill. Rejects
    /// unsafe paths and content over the size cap. Creates intermediate dirs and
    /// evicts the skill cache (in case `SKILL.md` changed).
    /// Remove one file of a library skill, but only while it still hashes to
    /// `if_hash` (same lock and contract as [`Self::write_skill_file_if`]).
    /// Unlike [`Self::delete_skill_file`] it may remove `SKILL.md`: the
    /// self-improvement engine applies (and rolls back) whole-file removals.
    pub fn remove_skill_file_if(
        &self,
        name: &str,
        rel: &str,
        if_hash: &str,
    ) -> io::Result<Result<(), StaleSkillFile>> {
        let _write = skill_write_guard();
        let current = self.skill_file_hash(name, rel);
        if current.as_deref() != Some(if_hash) {
            return Ok(Err(StaleSkillFile { current }));
        }
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let target = confined(&root, rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        fs::remove_file(target)?;
        self.evict(name);
        Ok(Ok(()))
    }

    pub fn write_skill_file(&self, name: &str, rel: &str, content: &str) -> io::Result<()> {
        let _write = skill_write_guard();
        self.write_skill_file_unlocked(name, rel, content)
    }

    fn write_skill_file_unlocked(&self, name: &str, rel: &str, content: &str) -> io::Result<()> {
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let target = confined(&root, rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        if content.len() > MAX_SKILL_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file too large",
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&target, content.as_bytes())?;
        self.evict(name);
        Ok(())
    }

    /// Create one NEW file inside a library skill: like [`Self::write_skill_file`]
    /// but fails with `AlreadyExists` (→ 409) instead of overwriting, so "New
    /// file" on an existing path can't truncate it. The existence check and the
    /// create are one `create_new` open, never a check-then-write race.
    pub fn create_skill_file(&self, name: &str, rel: &str, content: &str) -> io::Result<()> {
        use std::io::Write as _;
        let _write = skill_write_guard();
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let target = confined(&root, rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        if content.len() > MAX_SKILL_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file too large",
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        f.write_all(content.as_bytes())?;
        self.evict(name);
        Ok(())
    }

    /// Delete one file inside a library skill. `SKILL.md` cannot be deleted (use
    /// `delete_skill` to remove the whole skill). Missing file is a no-op.
    pub fn delete_skill_file(&self, name: &str, rel: &str) -> io::Result<()> {
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let rel_path = safe_rel(rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        if rel_path.as_os_str() == "SKILL.md" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SKILL.md cannot be deleted",
            ));
        }
        let target = confined(&root, rel)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        match fs::remove_file(target) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.evict(name);
        Ok(())
    }

    /// Create a new library skill with a synthesized (or provided) `SKILL.md`.
    /// Fails with `AlreadyExists` if the skill dir already exists.
    pub fn create_skill(
        &self,
        name: &str,
        category: &str,
        description: &str,
        body: Option<&str>,
    ) -> io::Result<()> {
        let dir = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        if dir.join("SKILL.md").exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "skill already exists",
            ));
        }
        let content = match body {
            Some(b) if !b.trim().is_empty() => b.to_string(),
            _ => starter_skill_md(name, category, description),
        };
        self.put_skill(name, &content)
    }

    /// Import a skill package from an in-memory zip. The archive must contain a
    /// `SKILL.md`; the skill name is `name_override` or the wrapping directory
    /// name. Zip-slip and size limits are enforced. Returns the skill name.
    pub fn import_zip(&self, bytes: &[u8], name_override: Option<&str>) -> io::Result<String> {
        use std::io::Cursor;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad zip: {e}")))?;

        // Locate the top-most SKILL.md to find the package root inside the zip.
        let mut best: Option<(String, usize)> = None; // (prefix, depth)
        for i in 0..zip.len() {
            let f = zip
                .by_index(i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            let Some(name) = f.enclosed_name() else {
                continue;
            };
            if name.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
                let prefix = name
                    .parent()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let depth = name.components().count();
                if best.as_ref().map(|(_, d)| depth < *d).unwrap_or(true) {
                    best = Some((prefix, depth));
                }
            }
        }
        let (prefix, _) =
            best.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no SKILL.md in zip"))?;

        // Resolve the skill name: explicit override, else the wrapper dir name.
        let derived = prefix.rsplit('/').next().filter(|s| !s.is_empty());
        let name = name_override
            .filter(|s| !s.is_empty())
            .or(derived)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cannot derive skill name"))?
            .to_string();
        let dest_root = self
            .skill_dir(&name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        // Strip the wrapping prefix so entries land at the skill root.
        let strip = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };

        let mut total: u64 = 0;
        for i in 0..zip.len() {
            let mut f = zip
                .by_index(i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if f.is_dir() {
                continue;
            }
            let Some(entry) = f.enclosed_name() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsafe zip entry",
                ));
            };
            let entry = entry.to_string_lossy().replace('\\', "/");
            let Some(rel) = entry.strip_prefix(&strip) else {
                continue; // outside the package root
            };
            let Some(out) = confined(&dest_root, rel) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsafe zip entry",
                ));
            };
            total += f.size();
            if total > MAX_SKILL_IMPORT_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "zip too large"));
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut buf)?;
            fs::write(&out, &buf)?;
            #[cfg(unix)]
            if out.extension().and_then(|e| e.to_str()) == Some("sh") {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(0o755));
            }
        }
        self.evict(&name);
        Ok(name)
    }

    /// Evict a skill's cached metadata (best-effort).
    fn evict(&self, name: &str) {
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.remove(name);
        }
    }

    // -- souls ---------------------------------------------------------------

    fn souls_dir(&self) -> PathBuf {
        self.root.join("souls")
    }

    pub fn list_souls(&self) -> Vec<LibrarySoul> {
        list_md_entries(&self.souls_dir())
            .into_iter()
            .map(|(name, body)| LibrarySoul { name, body })
            .collect()
    }

    pub fn get_soul(&self, name: &str) -> Option<LibrarySoul> {
        let body = fs::read_to_string(md_entry_path(&self.souls_dir(), name)?).ok()?;
        Some(LibrarySoul {
            name: name.to_string(),
            body,
        })
    }

    pub fn put_soul(&self, name: &str, body: &str) -> io::Result<()> {
        write_md_entry(&self.souls_dir(), name, body)
    }

    pub fn delete_soul(&self, name: &str) -> io::Result<()> {
        delete_md_entry(&self.souls_dir(), name)
    }

    // -- context -------------------------------------------------------------

    fn context_dir(&self) -> PathBuf {
        self.root.join("context")
    }

    pub fn list_context(&self) -> Vec<LibraryContext> {
        list_md_entries(&self.context_dir())
            .into_iter()
            .map(|(name, body)| LibraryContext { name, body })
            .collect()
    }

    pub fn get_context(&self, name: &str) -> Option<LibraryContext> {
        let body = fs::read_to_string(md_entry_path(&self.context_dir(), name)?).ok()?;
        Some(LibraryContext {
            name: name.to_string(),
            body,
        })
    }

    pub fn put_context(&self, name: &str, body: &str) -> io::Result<()> {
        write_md_entry(&self.context_dir(), name, body)
    }

    pub fn delete_context(&self, name: &str) -> io::Result<()> {
        delete_md_entry(&self.context_dir(), name)
    }

    // -- default soul --------------------------------------------------------

    fn default_soul_path(&self) -> PathBuf {
        self.root.join("default-soul.txt")
    }

    /// The configured global default soul name, or `None` when unset/empty.
    pub fn default_soul(&self) -> Option<String> {
        let raw = fs::read_to_string(self.default_soul_path()).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub fn set_default_soul(&self, name: &str) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        write_atomic(&self.default_soul_path(), name.trim().as_bytes())
    }
}

/// List `<dir>/<name>.md` entries as `(name, body)`, sorted by name. Unsafe or
/// non-`.md` files are skipped.
fn list_md_entries(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = file.strip_suffix(".md") else {
            continue;
        };
        if !is_safe_segment(name) {
            continue;
        }
        if let Ok(body) = fs::read_to_string(entry.path()) {
            out.push((name.to_string(), body));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The confined `<dir>/<name>.md` path for a soul/context entry, or `None`
/// for an unsafe name. Validation is [`is_safe_segment`]; the join goes through
/// the shared [`confine_join`] barrier.
fn md_entry_path(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    if !is_safe_segment(name) {
        return None;
    }
    confine_join(dir, &format!("{name}.md"))
}

fn write_md_entry(dir: &std::path::Path, name: &str, body: &str) -> io::Result<()> {
    let path = md_entry_path(dir, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe entry name"))?;
    fs::create_dir_all(dir)?;
    write_atomic(&path, body.as_bytes())
}

fn delete_md_entry(dir: &std::path::Path, name: &str) -> io::Result<()> {
    let path = md_entry_path(dir, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe entry name"))?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn lib() -> (TempDir, Library) {
        let dir = TempDir::new().unwrap();
        let lib = Library::new(dir.path());
        (dir, lib)
    }

    #[test]
    fn skill_round_trip_and_description() {
        let (_d, lib) = lib();
        let body =
            "---\ndescription: Triage support tickets\ncategory: review\nversion: 4\n---\n# body\n";
        lib.put_skill("support-triage", body).unwrap();

        let got = lib.get_skill("support-triage").unwrap();
        assert_eq!(got.name, "support-triage");
        assert_eq!(got.description, "Triage support tickets");
        assert_eq!(got.category, "review");
        assert_eq!(got.version, 4);
        assert_eq!(got.body, body);

        let listed = lib.list_skills();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "support-triage");

        lib.delete_skill("support-triage").unwrap();
        assert!(lib.get_skill("support-triage").is_none());
        assert!(lib.list_skills().is_empty());
    }

    #[test]
    fn skill_without_frontmatter_has_empty_description() {
        let (_d, lib) = lib();
        lib.put_skill("plain", "# just markdown\n").unwrap();
        let got = lib.get_skill("plain").unwrap();
        assert_eq!(got.description, "");
        assert_eq!(got.category, "");
        assert_eq!(got.version, 1);
    }

    #[test]
    fn skill_path_is_independent_of_existence() {
        let (_d, lib) = lib();
        let p = lib.skill_path("ghost").unwrap();
        assert!(p.ends_with("skills/ghost/SKILL.md"));
        assert!(lib.skill_path("../x").is_none());
        assert!(lib.skill_path("a/b").is_none());
    }

    #[test]
    fn soul_round_trip() {
        let (_d, lib) = lib();
        lib.put_soul("otto", "Be terse.").unwrap();
        assert_eq!(lib.get_soul("otto").unwrap().body, "Be terse.");
        assert_eq!(lib.list_souls().len(), 1);
        lib.delete_soul("otto").unwrap();
        assert!(lib.get_soul("otto").is_none());
    }

    #[test]
    fn context_round_trip() {
        let (_d, lib) = lib();
        lib.put_context("house-rules", "No emojis.").unwrap();
        assert_eq!(lib.get_context("house-rules").unwrap().body, "No emojis.");
        assert_eq!(lib.list_context().len(), 1);
        lib.delete_context("house-rules").unwrap();
        assert!(lib.get_context("house-rules").is_none());
    }

    #[test]
    fn default_soul_file() {
        let (_d, lib) = lib();
        assert!(lib.default_soul().is_none());
        lib.set_default_soul("  otto  ").unwrap();
        assert_eq!(lib.default_soul().as_deref(), Some("otto"));
        lib.set_default_soul("").unwrap();
        assert!(lib.default_soul().is_none());
    }

    #[test]
    fn unsafe_names_are_rejected() {
        let (_d, lib) = lib();
        assert!(lib.put_skill("../x", "b").is_err());
        assert!(lib.put_skill("a/b", "b").is_err());
        assert!(lib.put_soul("..", "b").is_err());
        assert!(lib.put_context("", "b").is_err());
        assert!(lib.get_skill("../x").is_none());
        assert!(lib.get_soul("a/b").is_none());
        assert!(lib.delete_skill("../x").is_err());
    }

    #[test]
    fn safe_rel_accepts_nested_and_rejects_traversal() {
        assert!(safe_rel("SKILL.md").is_some());
        assert!(safe_rel("references/rubric.md").is_some());
        assert!(safe_rel("scripts/a/b/c.sh").is_some());
        // Rejected:
        assert!(safe_rel("").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("../secret").is_none());
        assert!(safe_rel("a/../../b").is_none());
        assert!(safe_rel("a/./b").is_none());
        assert!(safe_rel("a//b").is_none());
        assert!(safe_rel("a\\b").is_none());
        assert!(safe_rel("references/..").is_none());
    }

    /// A writer that bypasses `Library` (improve engine, bundled install, a
    /// hand edit) is visible to the next `get_skill` — the cache re-stats.
    #[test]
    fn external_skill_md_write_is_seen_by_the_next_get_skill() {
        let (_d, lib) = lib();
        lib.put_skill("ext", "---\ndescription: Old\n---\nold body")
            .unwrap();
        assert_eq!(lib.get_skill("ext").unwrap().description, "Old");
        // Cached now; rewrite the file behind the Library's back.
        std::fs::write(
            lib.skill_path("ext").unwrap(),
            "---\ndescription: Applied improvement\n---\nnew, longer body",
        )
        .unwrap();
        let got = lib.get_skill("ext").unwrap();
        assert_eq!(got.description, "Applied improvement");
        assert!(got.body.ends_with("new, longer body"));
        // And an external delete is not served from the cache either.
        std::fs::remove_file(lib.skill_path("ext").unwrap()).unwrap();
        assert!(lib.get_skill("ext").is_none());
    }

    #[test]
    fn write_atomic_replaces_whole_file_and_leaves_no_temp() {
        let d = TempDir::new().unwrap();
        let p = d.path().join("SKILL.md");
        std::fs::write(&p, "a much longer original body").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        write_atomic(&p, b"short").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "short");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "permissions survive the replace");
        }
        let names: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["SKILL.md".to_string()]);
    }

    #[test]
    fn skill_file_crud_round_trip_and_cache_evict() {
        let (_d, lib) = lib();
        lib.create_skill("editable", "review", "Reviews stuff", None)
            .unwrap();
        // Starter SKILL.md is well-formed and cached.
        let got = lib.get_skill("editable").unwrap();
        assert_eq!(got.category, "review");
        assert_eq!(got.description, "Reviews stuff");

        // Add a reference file, then it shows up in the tree.
        lib.write_skill_file("editable", "references/notes.md", "# notes")
            .unwrap();
        let files: Vec<String> = lib
            .list_skill_files("editable")
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert!(files.contains(&"SKILL.md".to_string()));
        assert!(files.contains(&"references/notes.md".to_string()));
        assert_eq!(
            lib.read_skill_file("editable", "references/notes.md")
                .unwrap()
                .0,
            "# notes"
        );

        // Overwriting SKILL.md evicts the cache so metadata re-parses.
        lib.write_skill_file(
            "editable",
            "SKILL.md",
            "---\ndescription: New desc\ncategory: design\nversion: 2\n---\n# x",
        )
        .unwrap();
        let got2 = lib.get_skill("editable").unwrap();
        assert_eq!(got2.description, "New desc");
        assert_eq!(got2.category, "design");
        assert_eq!(got2.version, 2);

        // SKILL.md cannot be deleted; other files can.
        assert!(lib.delete_skill_file("editable", "SKILL.md").is_err());
        lib.delete_skill_file("editable", "references/notes.md")
            .unwrap();
        assert!(lib
            .read_skill_file("editable", "references/notes.md")
            .is_none());

        // S17-10: create-only never truncates an existing file (SKILL.md included).
        let before = lib.read_skill_file("editable", "SKILL.md").unwrap().0;
        let err = lib
            .create_skill_file("editable", "SKILL.md", "")
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            lib.read_skill_file("editable", "SKILL.md").unwrap().0,
            before
        );
        lib.create_skill_file("editable", "references/new.md", "")
            .unwrap();
        assert_eq!(
            lib.read_skill_file("editable", "references/new.md")
                .unwrap()
                .0,
            ""
        );
        assert!(lib
            .create_skill_file("editable", "../evil.md", "x")
            .is_err());

        // Unsafe paths rejected.
        assert!(lib.write_skill_file("editable", "../evil.md", "x").is_err());
        assert!(lib
            .read_skill_file("editable", "../../etc/passwd")
            .is_none());
    }

    /// Deleting a skill moves the whole tree into `skills-backup/`, never
    /// destroys it; deleting a missing skill reports no backup.
    #[test]
    fn delete_skill_moves_the_tree_into_the_backup_folder() {
        let (d, lib) = lib();
        lib.create_skill("gone", "review", "Soon deleted", None)
            .unwrap();
        lib.write_skill_file("gone", "references/notes.md", "keep me")
            .unwrap();
        let backup = lib.delete_skill("gone").unwrap().expect("backed up");
        assert!(backup.starts_with(d.path().join("skills-backup")));
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gone-"));
        assert_eq!(
            std::fs::read_to_string(backup.join("references/notes.md")).unwrap(),
            "keep me"
        );
        assert!(backup.join("SKILL.md").is_file());
        assert!(lib.get_skill("gone").is_none());
        assert!(!d.path().join("skills/gone").exists());
        assert_eq!(lib.delete_skill("gone").unwrap(), None);
    }

    /// Pruning "vault" backups never touches those of "vault-2" (modern
    /// `<name>-<secs>-<ULID>` or legacy `<name>-<secs>` names).
    #[test]
    fn prune_matches_the_exact_skill_name_only() {
        let (d, _lib) = lib();
        let b = d.path().join("skills-backup");
        let ulid = |n: u32| format!("01JC{n:022}");
        let mut vault2 = Vec::new();
        for secs in [1, 2, 3, 4] {
            let modern = format!("vault-2-{secs}-{}", ulid(secs));
            fs::create_dir_all(b.join(&modern)).unwrap();
            vault2.push(modern);
        }
        fs::create_dir_all(b.join("vault-2-5")).unwrap();
        vault2.push("vault-2-5".to_string());
        for secs in [10, 11, 12, 13] {
            fs::create_dir_all(b.join(format!("vault-{secs}-{}", ulid(secs)))).unwrap();
        }
        prune_skill_backups(d.path(), "vault", 2);
        let left: Vec<String> = fs::read_dir(&b)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        for name in &vault2 {
            assert!(left.contains(name), "{name} pruned as a vault backup");
        }
        assert_eq!(left.iter().filter(|n| n.starts_with("vault-1")).count(), 2);
        assert!(left.contains(&format!("vault-13-{}", ulid(13))));
        assert!(left.contains(&format!("vault-12-{}", ulid(12))));
        // And pruning "vault-2" keeps its own newest two only.
        prune_skill_backups(d.path(), "vault-2", 2);
        let left2 = fs::read_dir(&b).unwrap().flatten().count();
        assert_eq!(left2, 4);
    }

    /// `if_hash` guards the editor: a save based on an older read is refused
    /// with the current hash and leaves the newer text alone.
    #[test]
    fn conditional_write_refuses_a_stale_hash() {
        let (_d, lib) = lib();
        lib.create_skill("guarded", "review", "d", None).unwrap();
        let read_hash = lib.skill_file_hash("guarded", "SKILL.md").unwrap();
        assert_eq!(
            read_hash,
            otto_core::text::content_sha256(&lib.read_skill_file("guarded", "SKILL.md").unwrap().0)
        );
        // The improve engine (or a review fix) writes in between.
        lib.write_skill_file("guarded", "SKILL.md", "theirs")
            .unwrap();
        let theirs = lib.skill_file_hash("guarded", "SKILL.md").unwrap();
        let stale = lib
            .write_skill_file_if("guarded", "SKILL.md", "mine", &read_hash)
            .unwrap()
            .expect_err("stale base");
        assert_eq!(stale.current.as_deref(), Some(theirs.as_str()));
        assert_eq!(
            lib.read_skill_file("guarded", "SKILL.md").unwrap().0,
            "theirs"
        );
        // "Keep mine" re-sends with their hash and wins.
        lib.write_skill_file_if("guarded", "SKILL.md", "mine", &theirs)
            .unwrap()
            .unwrap();
        assert_eq!(
            lib.read_skill_file("guarded", "SKILL.md").unwrap().0,
            "mine"
        );
        // A file that vanished is stale too (current = None).
        let gone = lib
            .write_skill_file_if("guarded", "references/x.md", "y", &theirs)
            .unwrap()
            .expect_err("missing file");
        assert_eq!(gone.current, None);
    }

    #[test]
    fn create_skill_conflicts_on_existing() {
        let (_d, lib) = lib();
        lib.create_skill("dup", "review", "d", None).unwrap();
        let err = lib.create_skill("dup", "review", "d", None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn import_zip_strips_wrapper_and_lands_files() {
        use std::io::Write;
        let (_d, lib) = lib();
        // Build a wrapped skill package in memory.
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("my-skill/SKILL.md", opts).unwrap();
            zw.write_all(b"---\ndescription: Imported\ncategory: review\nversion: 1\n---\n# body")
                .unwrap();
            zw.start_file("my-skill/references/x.md", opts).unwrap();
            zw.write_all(b"ref").unwrap();
            zw.finish().unwrap();
        }
        let name = lib.import_zip(&buf, None).unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(lib.get_skill("my-skill").unwrap().description, "Imported");
        assert_eq!(
            lib.read_skill_file("my-skill", "references/x.md")
                .unwrap()
                .0,
            "ref"
        );
    }

    #[test]
    fn import_zip_requires_skill_md() {
        use std::io::Write;
        let (_d, lib) = lib();
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("nope/readme.md", opts).unwrap();
            zw.write_all(b"x").unwrap();
            zw.finish().unwrap();
        }
        assert!(lib.import_zip(&buf, None).is_err());
    }
}

//! The vault engine: registration, scanning (incremental, coalesced), note
//! CRUD with trash + rename-rewrites-links, search/switcher/tags/backlinks,
//! and the graph payload. Files on disk are the source of truth throughout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use otto_core::{Error, Result};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::parse::{self, parse_note};
use crate::resolve::ResolveIndex;
use crate::scan::{self, WalkResult, MAX_FTS_BYTES};
use crate::store::{NoteRow, Store};
use crate::types::*;

/// Reads served without a rescan for this long after the last one (freshness
/// window — external edits surface within it; API/MCP writes rescan eagerly).
const STALE_AFTER_SECS: i64 = 5;
/// `mode=full` graph default edge budget (override via `edge_budget`).
const DEFAULT_EDGE_BUDGET: usize = 2_000_000;

/// In-memory switcher index (rebuilt after every scan): fuzzy match over
/// title, aliases and path without shipping the note list to the client.
struct SwitcherIx {
    /// (path, title, aliases)
    rows: Vec<(String, String, Vec<String>)>,
}

type VaultWriteKey = (i64, String);
type VaultWriteLock = Arc<tokio::sync::Mutex<()>>;

pub struct VaultEngine {
    store: Store,
    /// Per-vault scan serialization + coalescing (a kick while a scan runs is
    /// dropped — the running scan picks up the changes anyway).
    scans: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// Unix seconds of the last completed scan per vault (staleness probe).
    last_scan: Mutex<HashMap<i64, Arc<AtomicI64>>>,
    /// Hash-check + replace serialization for each writable vault path.
    writes: Mutex<HashMap<VaultWriteKey, VaultWriteLock>>,
    switcher: RwLock<HashMap<i64, Arc<SwitcherIx>>>,
    fts_ok: std::sync::atomic::AtomicU8, // 0 unknown / 1 yes / 2 no
}

impl VaultEngine {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            store: Store::new(pool),
            scans: Mutex::new(HashMap::new()),
            last_scan: Mutex::new(HashMap::new()),
            writes: Mutex::new(HashMap::new()),
            switcher: RwLock::new(HashMap::new()),
            fts_ok: std::sync::atomic::AtomicU8::new(0),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    async fn fts_ready(&self) -> bool {
        match self.fts_ok.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => {
                let ok = self.store.ensure_fts().await;
                self.fts_ok.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
                ok
            }
        }
    }

    // -- registration ---------------------------------------------------------

    /// Register an existing directory (created if `root` is None → a fresh
    /// vault under `~/.otto/vault/<slug>`). Kicks a full scan in the background.
    pub async fn register(
        self: &Arc<Self>,
        ws: &str,
        name: &str,
        root: Option<String>,
        okf: bool,
    ) -> Result<VaultRec> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("vault name is required".into()));
        }
        let root_path = match root {
            Some(r) if !r.trim().is_empty() => {
                let p = PathBuf::from(shellexpand_home(r.trim()));
                if p.is_file() {
                    return Err(Error::Invalid(format!("not a directory: {}", p.display())));
                }
                if !p.is_dir() {
                    // A path that doesn't exist yet is a request for a fresh
                    // vault there (Obsidian's "create vault" behavior) — never
                    // touches existing data.
                    std::fs::create_dir_all(&p)
                        .map_err(|e| Error::Invalid(format!("create {}: {e}", p.display())))?;
                }
                p
            }
            _ => {
                let base = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".otto")
                    .join("vault")
                    .join(slug(name));
                std::fs::create_dir_all(&base)
                    .map_err(|e| Error::Internal(format!("create vault dir: {e}")))?;
                base
            }
        };
        let canon = root_path
            .canonicalize()
            .map_err(|e| Error::Invalid(format!("vault root: {e}")))?;
        let id = self
            .store
            .create_vault(ws, name, &canon.to_string_lossy(), okf)
            .await?;
        self.kick_scan(id);
        self.store.get_vault(id).await
    }

    pub async fn list(&self, ws: &str) -> Result<Vec<VaultRec>> {
        // Vaults are a GLOBAL library (like connections): every workspace sees
        // them all. `ws` stays in the signature for the route shape/RBAC only.
        let _ = ws;
        self.store.list_vaults().await
    }

    /// Vault fetch. Vaults are GLOBAL: `ws` is the caller's workspace (already
    /// role-checked by the route) and no longer restricts which vault an id
    /// may address — `ws_id` on the row is provenance, not a boundary.
    pub async fn get_scoped(&self, ws: &str, id: i64) -> Result<VaultRec> {
        let _ = ws;
        self.store.get_vault(id).await
    }

    pub async fn patch(&self, ws: &str, id: i64, name: Option<&str>, okf: Option<bool>) -> Result<VaultRec> {
        self.get_scoped(ws, id).await?;
        self.store.patch_vault(id, name, okf).await?;
        self.store.get_vault(id).await
    }

    /// Unregister — index rows only; the files on disk are untouched.
    pub async fn unregister(&self, ws: &str, id: i64) -> Result<()> {
        self.get_scoped(ws, id).await?;
        self.store.delete_vault(id).await
    }

    // -- scanning ---------------------------------------------------------------

    fn scan_lock(&self, id: i64) -> Arc<tokio::sync::Mutex<()>> {
        self.scans.lock().unwrap().entry(id).or_default().clone()
    }

    fn last_scan_cell(&self, id: i64) -> Arc<AtomicI64> {
        self.last_scan.lock().unwrap().entry(id).or_default().clone()
    }

    fn write_lock(&self, id: i64, path: &str) -> VaultWriteLock {
        self.writes
            .lock()
            .unwrap()
            .entry((id, path.to_string()))
            .or_default()
            .clone()
    }

    /// Fire-and-forget scan kick (coalesced).
    pub fn kick_scan(self: &Arc<Self>, id: i64) {
        let eng = self.clone();
        tokio::spawn(async move {
            let _ = eng.scan(id).await;
        });
    }

    /// Ensure freshness before a read: if the last completed scan is older than
    /// the staleness window, kick a background scan (non-blocking).
    pub fn ensure_fresh(self: &Arc<Self>, id: i64) {
        let cell = self.last_scan_cell(id);
        let now = chrono::Utc::now().timestamp();
        if now - cell.load(Ordering::Relaxed) > STALE_AFTER_SECS {
            self.kick_scan(id);
        }
    }

    /// Incremental scan (parse changed, drop removed, re-resolve links).
    /// Serialized per vault and COALESCED: if another scan completes after we
    /// were called (it saw our changes — fs writes happen before scan()), the
    /// queued pass is skipped instead of walking again.
    pub async fn scan(&self, id: i64) -> Result<()> {
        let asked_at = chrono::Utc::now().timestamp();
        let lock = self.scan_lock(id);
        let _guard = lock.lock().await;
        if self.last_scan_cell(id).load(Ordering::Relaxed) > asked_at {
            return Ok(());
        }
        let v = self.store.get_vault(id).await?;
        let root = PathBuf::from(&v.root_path);
        if !root.is_dir() {
            self.store
                .set_scan_state(id, &format!("error: vault root missing: {}", v.root_path), false)
                .await?;
            return Err(Error::Conflict(format!("vault root missing: {}", v.root_path)));
        }
        self.store.set_scan_state(id, "scanning", false).await?;
        let res = self.scan_inner(id, &root).await;
        match &res {
            Ok(()) => {
                self.store.set_scan_state(id, "idle", true).await?;
                self.last_scan_cell(id).store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
            }
            Err(e) => {
                let _ = self.store.set_scan_state(id, &format!("error: {e}"), false).await;
            }
        }
        res
    }

    async fn scan_inner(&self, id: i64, root: &Path) -> Result<()> {
        let root_owned = root.to_path_buf();
        let walk: WalkResult = tokio::task::spawn_blocking(move || scan::walk(&root_owned))
            .await
            .map_err(|e| Error::Internal(format!("walk join: {e}")))?
            .map_err(|e| Error::Internal(format!("walk: {e}")))?;

        let note_sigs = self.store.note_sigs(id).await?;
        let file_sigs = self.store.file_sigs(id).await?;
        let (changed_notes, removed_notes) = scan::diff(&walk.notes, &note_sigs);
        let (changed_files, removed_files) = scan::diff(&walk.files, &file_sigs);
        let structure_changed = !changed_notes.is_empty()
            || !removed_notes.is_empty()
            || !changed_files.is_empty()
            || !removed_files.is_empty();

        let fts = self.fts_ready().await;
        let sizes: HashMap<&str, (i64, i64)> =
            walk.notes.iter().map(|e| (e.rel.as_str(), (e.size, e.mtime_ns))).collect();

        // Parse + upsert changed notes; collect their links for resolution.
        let mut pending_links: Vec<(String, Vec<OutgoingLink>)> = Vec::new();
        for rel in &changed_notes {
            let abs = root.join(rel);
            let (size, mtime_ns) = sizes.get(rel.as_str()).copied().unwrap_or((0, 0));
            let content = match tokio::fs::read(&abs).await {
                Ok(b) => b,
                Err(_) => continue, // raced away — the removal shows next scan
            };
            let text = String::from_utf8_lossy(&content).into_owned();
            let parsed = parse_note(&text);
            let base = rel.rsplit('/').next().unwrap_or(rel);
            let stem = base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base);
            let reserved = matches!(stem.to_ascii_lowercase().as_str(), "index" | "log")
                && base.to_ascii_lowercase().ends_with(".md");
            let title = parse::derive_title(&parsed, rel);
            let hash = hex_sha256(&content);
            let row = NoteRow {
                path: rel.clone(),
                title: title.clone(),
                okf_type: parsed.okf_type.clone(),
                description: parsed.description.clone(),
                frontmatter_json: serde_json::to_string(&parsed.frontmatter).unwrap_or_else(|_| "null".into()),
                tags_json: serde_json::to_string(&parsed.tags).unwrap_or_else(|_| "[]".into()),
                aliases_json: serde_json::to_string(&parsed.aliases).unwrap_or_else(|_| "[]".into()),
                headings_json: serde_json::to_string(&parsed.headings).unwrap_or_else(|_| "[]".into()),
                word_count: parsed.word_count as i64,
                size,
                mtime_ns,
                hash,
                reserved,
                has_frontmatter: parsed.has_frontmatter,
                parse_error: parsed.parse_error,
            };
            self.store.upsert_note(id, &row).await?;
            self.store.replace_tags(id, rel, &parsed.tags).await?;
            if fts {
                let body = if size as u64 <= MAX_FTS_BYTES { text.as_str() } else { "" };
                self.store.fts_index(id, rel, &title, body).await;
            }
            pending_links.push((rel.clone(), parsed.links));
        }
        for rel in &removed_notes {
            self.store.remove_note(id, rel).await?;
        }
        let file_sizes: HashMap<&str, (i64, i64)> =
            walk.files.iter().map(|e| (e.rel.as_str(), (e.size, e.mtime_ns))).collect();
        for rel in &changed_files {
            let (size, mtime) = file_sizes.get(rel.as_str()).copied().unwrap_or((0, 0));
            self.store.upsert_file(id, rel, size, mtime).await?;
        }
        for rel in &removed_files {
            self.store.remove_file(id, rel).await?;
        }

        // Resolution index over the CURRENT tree.
        let mut ix = ResolveIndex::default();
        for e in &walk.notes {
            ix.insert(e.rel.clone());
        }
        for e in &walk.files {
            ix.insert(e.rel.clone());
        }

        // Store the changed notes' links (resolved).
        for (src, mut links) in pending_links {
            for l in &mut links {
                l.dst_path = ix.resolve(&src, &l.raw_target);
            }
            self.store.replace_links(id, &src, &links).await?;
        }

        // Global re-resolve when the file SET changed: a new file can fix a
        // broken link OR make a previously-unique basename ambiguous; a removal
        // breaks links pointing at it. Only changed rows are written.
        if structure_changed {
            for (rowid, src, raw, dst) in self.store.all_links_full(id).await? {
                let new_dst = ix.resolve(&src, &raw);
                if new_dst != dst {
                    self.store.update_link_dst(rowid, new_dst.as_deref()).await?;
                }
            }
        }

        // Rebuild the switcher index.
        let notes = self.store.all_notes(id).await?;
        let aliases: HashMap<String, Vec<String>> = self
            .store
            .all_aliases(id)
            .await?
            .into_iter()
            .map(|(p, a)| (p, serde_json::from_str(&a).unwrap_or_default()))
            .collect();
        let rows = notes
            .into_iter()
            .filter(|(_, _, _, reserved)| !reserved)
            .map(|(p, t, _, _)| {
                let al = aliases.get(&p).cloned().unwrap_or_default();
                (p, t, al)
            })
            .collect();
        self.switcher.write().unwrap().insert(id, Arc::new(SwitcherIx { rows }));
        Ok(())
    }

    pub async fn status(self: &Arc<Self>, ws: &str, id: i64) -> Result<VaultStatus> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        self.store.status(id).await
    }

    // -- path safety -------------------------------------------------------------

    /// Validate a client-supplied vault-relative path: no absolutes, no `..`,
    /// no backslashes, no NUL, not into `.trash`/hidden dirs.
    fn check_rel(path: &str) -> Result<String> {
        let p = path.trim().trim_start_matches("./");
        if p.is_empty() {
            return Err(Error::Invalid("empty path".into()));
        }
        if p.starts_with('/') || p.contains('\\') || p.contains('\0') || p.len() > 1024 {
            return Err(Error::Invalid(format!("invalid path: {path}")));
        }
        for seg in p.split('/') {
            if seg == ".." || seg == "." || seg.is_empty() {
                return Err(Error::Invalid(format!("invalid path: {path}")));
            }
            if seg.starts_with('.') {
                return Err(Error::Invalid(format!("hidden segments are not allowed: {path}")));
            }
        }
        Ok(p.to_string())
    }

    /// Absolute path of `rel` inside the vault, symlink-escape-guarded: the
    /// canonicalized parent must stay under the canonicalized root.
    fn abs_guarded(root: &str, rel: &str) -> Result<PathBuf> {
        let rootc = Path::new(root)
            .canonicalize()
            .map_err(|e| Error::Conflict(format!("vault root missing: {e}")))?;
        let target = rootc.join(rel);
        let mut existing = target.parent().unwrap_or(&rootc);
        while !existing.exists() && existing != rootc {
            existing = existing.parent().unwrap_or(&rootc);
        }
        if let Ok(pc) = existing.canonicalize() {
            if !pc.starts_with(&rootc) {
                return Err(Error::Forbidden("path escapes the vault".into()));
            }
        }
        Ok(target)
    }

    /// Open (and create where absent) every parent component relative to a held
    /// vault directory capability. `NOFOLLOW` on each hop prevents a concurrent
    /// symlink swap from redirecting later reads or the final rename.
    fn text_parent(root: &str, rel: &str) -> Result<(OwnedFd, String)> {
        let mut parts = rel.split('/').peekable();
        let mut parent = rustix::fs::open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| Error::Conflict(format!("open vault root: {e}")))?;
        let dir_mode = Mode::RWXU | Mode::RGRP | Mode::XGRP | Mode::ROTH | Mode::XOTH;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return Ok((parent, part.to_string()));
            }
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let next = match rustix::fs::openat(&parent, part, flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(Errno::NOENT) => {
                    match rustix::fs::mkdirat(&parent, part, dir_mode) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(e) => {
                            return Err(Error::Internal(format!(
                                "create text artifact directory {part}: {e}"
                            )))
                        }
                    }
                    rustix::fs::openat(&parent, part, flags, Mode::empty()).map_err(|e| {
                        if matches!(e, Errno::LOOP | Errno::NOTDIR) {
                            Error::Forbidden("path escapes the vault".into())
                        } else {
                            Error::Internal(format!("open text artifact directory {part}: {e}"))
                        }
                    })?
                }
                Err(Errno::LOOP | Errno::NOTDIR) => {
                    return Err(Error::Forbidden("path escapes the vault".into()))
                }
                Err(e) => {
                    return Err(Error::Internal(format!(
                        "open text artifact directory {part}: {e}"
                    )))
                }
            };
            parent = next;
        }
        Err(Error::Invalid(format!("invalid path: {rel}")))
    }

    async fn text_file_bytes(parent: &OwnedFd, name: &str) -> Result<Option<Vec<u8>>> {
        match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode).is_symlink() => {
                return Err(Error::Forbidden(
                    "text artifact target must not be a symlink".into(),
                ))
            }
            Ok(_) => {}
            Err(Errno::NOENT) => return Ok(None),
            Err(e) => {
                return Err(Error::Internal(format!(
                    "inspect text artifact {name}: {e}"
                )))
            }
        }
        let fd = match rustix::fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(Errno::LOOP) => {
                return Err(Error::Forbidden(
                    "text artifact target must not be a symlink".into(),
                ))
            }
            Err(e) => {
                return Err(Error::Internal(format!(
                    "open text artifact {name}: {e}"
                )))
            }
        };
        let mut file = tokio::fs::File::from_std(std::fs::File::from(fd));
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .await
            .map_err(|e| Error::Internal(format!("read text artifact {name}: {e}")))?;
        Ok(Some(bytes))
    }

    /// Write a unique same-directory temp and rename it relative to the held
    /// parent capability. Parent path replacement cannot redirect either step.
    async fn atomic_replace_at(parent: &OwnedFd, name: &str, bytes: &[u8]) -> Result<()> {
        let temp = format!(".{name}.otto-tmp-{}", otto_core::new_id());
        let mode = Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH;
        let fd = rustix::fs::openat(
            parent,
            temp.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            mode,
        )
        .map_err(|e| Error::Internal(format!("create text artifact temp {temp}: {e}")))?;
        let write = async {
            let mut file = tokio::fs::File::from_std(std::fs::File::from(fd));
            file.write_all(bytes)
                .await
                .map_err(|e| Error::Internal(format!("write text artifact temp {temp}: {e}")))?;
            file.sync_all()
                .await
                .map_err(|e| Error::Internal(format!("flush text artifact temp {temp}: {e}")))?;
            drop(file);
            rustix::fs::renameat(parent, temp.as_str(), parent, name)
                .map_err(|e| Error::Internal(format!("replace text artifact {name}: {e}")))?;
            rustix::fs::fsync(parent)
                .map_err(|e| Error::Internal(format!("flush text artifact directory: {e}")))
        }
        .await;
        if write.is_err() {
            let _ = rustix::fs::unlinkat(parent, temp.as_str(), AtFlags::empty());
        }
        write
    }

    // -- notes ---------------------------------------------------------------------

    pub async fn dir(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<DirListing> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rel = if path.trim().is_empty() { String::new() } else { Self::check_rel(path)? };
        let notes = self.store.all_notes(id).await?;
        let files = self.store.all_file_paths(id).await?;
        let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
        let mut dirs: HashMap<String, i64> = HashMap::new();
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();
        let note_meta: HashMap<&str, (&str, Option<&str>, bool)> = notes
            .iter()
            .map(|(p, t, ty, r)| (p.as_str(), (t.as_str(), ty.as_deref(), *r)))
            .collect();
        for p in notes.iter().map(|(p, ..)| p.as_str()).chain(files.iter().map(|s| s.as_str())) {
            let Some(rest) = p.strip_prefix(&prefix) else { continue };
            if rest.is_empty() {
                continue;
            }
            match rest.split_once('/') {
                Some((d, _)) => {
                    *dirs.entry(d.to_string()).or_default() += 1;
                    seen_dirs.insert(d.to_string());
                }
                None => {
                    let is_note = p.to_ascii_lowercase().ends_with(".md");
                    let (title, ty, reserved) = note_meta
                        .get(p)
                        .map(|(t, ty, r)| (Some(t.to_string()), ty.map(String::from), *r))
                        .unwrap_or((None, None, false));
                    entries.push(DirEntry {
                        name: rest.to_string(),
                        path: p.to_string(),
                        kind: if is_note { "note" } else { "file" }.to_string(),
                        children: 0,
                        title,
                        okf_type: ty,
                        reserved,
                    });
                }
            }
        }
        let mut out: Vec<DirEntry> = dirs
            .into_iter()
            .map(|(d, n)| DirEntry {
                name: d.clone(),
                path: if prefix.is_empty() { d.clone() } else { format!("{prefix}{d}") },
                kind: "dir".to_string(),
                children: n,
                title: None,
                okf_type: None,
                reserved: false,
            })
            .collect();
        out.sort_by_key(|e| e.name.to_lowercase());
        // Reserved scaffolding (index.md, log.md) leads its folder — it's the
        // entry point a reader wants first, not an alphabetical mid-list row.
        entries.sort_by_key(|e| (!e.reserved, e.name.to_lowercase()));
        out.extend(entries);
        Ok(DirListing { path: rel, entries: out })
    }

    pub async fn note(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<NoteFull> {
        let v = self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        let raw = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|_| Error::NotFound(format!("note {rel}")))?;
        // Serve meta from the index when fresh; fall back to a live parse for a
        // note the scanner hasn't seen yet.
        let meta = match self.store.note_meta(id, &rel).await {
            Ok(m) => m,
            Err(_) => {
                self.scan(id).await.ok();
                self.store.note_meta(id, &rel).await?
            }
        };
        let outgoing = self.store.outgoing(id, &rel).await?;
        Ok(NoteFull { meta, raw, outgoing })
    }

    pub async fn write_note(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        path: &str,
        content: &str,
        if_hash: Option<&str>,
    ) -> Result<NoteMeta> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        if !rel.to_ascii_lowercase().ends_with(".md") {
            return Err(Error::Invalid("notes must end in .md".into()));
        }
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        if let Some(expected) = if_hash {
            let current = match tokio::fs::read(&abs).await {
                Ok(b) => hex_sha256(&b),
                Err(_) => String::new(), // creating — expected must be "" too
            };
            if current != expected {
                return Err(Error::Conflict(format!("note changed on disk (hash {current})")));
            }
        }
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("mkdir: {e}")))?;
        }
        tokio::fs::write(&abs, content)
            .await
            .map_err(|e| Error::Internal(format!("write: {e}")))?;
        self.scan(id).await?;
        self.store.note_meta(id, &rel).await
    }

    /// Write a guarded, UTF-8 documentation artifact that is not a Markdown
    /// note. This keeps OpenAPI/D2/JSON deliverables inside the same traversal,
    /// symlink, optimistic-concurrency, size, and rescan boundary as notes.
    pub async fn write_text_file(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        path: &str,
        content: &str,
        if_hash: Option<&str>,
    ) -> Result<VaultTextFile> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        let ext = std::path::Path::new(&rel)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "yaml" | "yml" | "json" | "d2" | "mmd" | "txt" | "csv") {
            return Err(Error::UnsupportedMedia(format!(
                "text artifacts must end in .yaml, .yml, .json, .d2, .mmd, .txt, or .csv (got {path})"
            )));
        }
        let bytes = content.as_bytes();
        if bytes.len() as u64 > MAX_FTS_BYTES {
            return Err(Error::PayloadTooLarge(format!(
                "text artifact is {} bytes; maximum is {MAX_FTS_BYTES}",
                bytes.len()
            )));
        }
        let lock = self.write_lock(id, &rel);
        let _guard = lock.lock().await;
        let (parent, name) = Self::text_parent(&v.root_path, &rel)?;
        if let Some(expected) = if_hash {
            let current = match Self::text_file_bytes(&parent, &name).await? {
                Some(bytes) => hex_sha256(&bytes),
                None => String::new(),
            };
            if current != expected {
                return Err(Error::Conflict(format!(
                    "text artifact changed on disk (hash {current})"
                )));
            }
        } else if rustix::fs::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode).is_symlink())
        {
            return Err(Error::Forbidden(
                "text artifact target must not be a symlink".into(),
            ));
        }
        Self::atomic_replace_at(&parent, &name, bytes).await?;
        self.scan(id).await?;
        Ok(VaultTextFile {
            path: rel,
            size: bytes.len() as i64,
            hash: hex_sha256(bytes),
        })
    }

    /// Soft delete → `<vault>/.trash/<path>` (never destroys user files).
    pub async fn delete_note(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<()> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        if !abs.exists() {
            return Err(Error::NotFound(format!("note {rel}")));
        }
        let mut dest = Path::new(&v.root_path).join(".trash").join(&rel);
        if dest.exists() {
            let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
            dest = dest.with_file_name(format!(
                "{}-{stamp}",
                dest.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("trash mkdir: {e}")))?;
        }
        tokio::fs::rename(&abs, &dest)
            .await
            .map_err(|e| Error::Internal(format!("trash move: {e}")))?;
        self.scan(id).await?;
        Ok(())
    }

    pub async fn create_folder(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<()> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        tokio::fs::create_dir_all(&abs)
            .await
            .map_err(|e| Error::Internal(format!("mkdir: {e}")))?;
        Ok(())
    }

    // -- rename (file or folder) + link rewrite ------------------------------------

    pub async fn rename(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        from: &str,
        to: &str,
    ) -> Result<RenameResult> {
        let v = self.get_scoped(ws, id).await?;
        let from_rel = Self::check_rel(from)?;
        let to_rel = Self::check_rel(to)?;
        if from_rel == to_rel {
            return Err(Error::Invalid("from and to are the same path".into()));
        }
        let root = v.root_path.clone();
        let from_abs = Self::abs_guarded(&root, &from_rel)?;
        let to_abs = Self::abs_guarded(&root, &to_rel)?;
        if !from_abs.exists() {
            return Err(Error::NotFound(from_rel));
        }
        let is_dir = from_abs.is_dir();
        let case_only = from_rel.to_lowercase() == to_rel.to_lowercase();
        if to_abs.exists() && !case_only {
            return Err(Error::Conflict(format!("target exists: {to_rel}")));
        }
        if let Some(parent) = to_abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("mkdir: {e}")))?;
        }
        if case_only {
            // Case-insensitive APFS: two-step move via a temp name.
            let tmp = to_abs.with_file_name(format!(
                ".otto-rename-{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ));
            tokio::fs::rename(&from_abs, &tmp)
                .await
                .map_err(|e| Error::Internal(format!("rename: {e}")))?;
            tokio::fs::rename(&tmp, &to_abs)
                .await
                .map_err(|e| Error::Internal(format!("rename: {e}")))?;
        } else {
            tokio::fs::rename(&from_abs, &to_abs)
                .await
                .map_err(|e| Error::Internal(format!("rename: {e}")))?;
        }

        // Moved-path map (old rel → new rel).
        let mut moved: HashMap<String, String> = HashMap::new();
        if is_dir {
            for (p, ..) in self.store.all_notes(id).await? {
                if let Some(rest) = p.strip_prefix(&format!("{from_rel}/")) {
                    moved.insert(p.clone(), format!("{to_rel}/{rest}"));
                }
            }
            for p in self.store.all_file_paths(id).await? {
                if let Some(rest) = p.strip_prefix(&format!("{from_rel}/")) {
                    moved.insert(p.clone(), format!("{to_rel}/{rest}"));
                }
            }
        } else {
            moved.insert(from_rel.clone(), to_rel.clone());
        }

        // Sources that link to any moved path (their links need rewriting) +
        // moved notes themselves (their RELATIVE md links now start elsewhere).
        let mut affected: HashSet<String> = HashSet::new();
        for old in moved.keys() {
            for src in self.store.linking_sources(id, old).await? {
                affected.insert(src);
            }
        }
        for (old, new) in &moved {
            if old.to_ascii_lowercase().ends_with(".md") {
                affected.remove(old); // it moved — rewrite at its NEW path
                affected.insert(new.clone());
            }
        }

        // Resolution snapshot BEFORE the move (the index still holds the old
        // paths — the scan below refreshes it), built once for every source.
        let mut ix_before = ResolveIndex::default();
        for (p, ..) in self.store.all_notes(id).await? {
            ix_before.insert(p);
        }
        for p in self.store.all_file_paths(id).await? {
            ix_before.insert(p);
        }
        let moved_new_to_old: HashMap<&String, &String> =
            moved.iter().map(|(o, n)| (n, o)).collect();

        let mut links_updated = 0i64;
        for src in &affected {
            let src_now = src.clone();
            let abs = Self::abs_guarded(&root, &src_now)?;
            let Ok(content) = tokio::fs::read_to_string(&abs).await else { continue };
            // The source itself may have moved: resolve raw targets from its OLD
            // location (that is how they were written).
            let src_before = moved_new_to_old
                .get(&src_now)
                .map(|o| (*o).clone())
                .unwrap_or_else(|| src_now.clone());
            let mut count_here = 0i64;
            let new_content = parse::rewrite_links(&content, |kind, raw| {
                let dst_old = ix_before.resolve(&src_before, raw)?;
                let moved_to = moved.get(&dst_old);
                let src_dir_now = src_now.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let src_moved = src_before != src_now;
                match (moved_to, src_moved, kind) {
                    // Target moved → point at its new home, preserving style.
                    (Some(new_dst), _, k) => {
                        count_here += 1;
                        Some(new_raw_for(raw, k, src_dir_now, new_dst))
                    }
                    // Target stayed, but THIS note moved and uses a relative md
                    // link → recompute the relative path from the new folder.
                    (None, true, "md") => {
                        if raw.starts_with('/') {
                            None
                        } else {
                            count_here += 1;
                            Some(relative_path(src_dir_now, &dst_old))
                        }
                    }
                    _ => None,
                }
            });
            if new_content != content && count_here > 0 {
                tokio::fs::write(&abs, new_content)
                    .await
                    .map_err(|e| Error::Internal(format!("rewrite {src_now}: {e}")))?;
                links_updated += count_here;
            }
        }

        // One scan picks up the moved files, rewritten sources, and re-resolves
        // everything (including newly-ambiguous basenames).
        self.scan(id).await?;
        Ok(RenameResult { from: from_rel, to: to_rel, links_updated })
    }

    // -- search / switcher / tags / backlinks ---------------------------------------

    pub async fn search(self: &Arc<Self>, ws: &str, id: i64, req: &SearchReq) -> Result<Vec<SearchHit>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let limit = if req.limit == 0 || req.limit > 200 { 50 } else { req.limit } as i64;
        // Operator syntax inside the query string: tag:x path:y type:z.
        let mut tag = req.tag.clone();
        let mut path_prefix = req.path_prefix.clone();
        let mut okf_type = req.okf_type.clone();
        let mut terms: Vec<String> = Vec::new();
        for tok in req.query.split_whitespace() {
            if let Some(v) = tok.strip_prefix("tag:") {
                tag = Some(v.trim_start_matches('#').to_string());
            } else if let Some(v) = tok.strip_prefix("path:") {
                path_prefix = Some(v.to_string());
            } else if let Some(v) = tok.strip_prefix("type:") {
                okf_type = Some(v.to_string());
            } else {
                terms.push(tok.to_string());
            }
        }
        let text = terms.join(" ");

        let mut hits: Vec<(String, String, f32)> = if !text.trim().is_empty() && self.fts_ready().await {
            let expr = fts_expr(&text);
            let got = self.store.fts_search(id, &expr, limit * 4).await.unwrap_or_default();
            if got.is_empty() {
                self.store.like_search(id, &text, limit * 4).await?
            } else {
                got
            }
        } else if !text.trim().is_empty() {
            self.store.like_search(id, &text, limit * 4).await?
        } else {
            // Pure filter query (tag:/path:/type: only).
            self.store
                .all_notes(id)
                .await?
                .into_iter()
                .map(|(p, t, _, _)| (p, t, 0.0f32))
                .collect()
        };

        // Filters.
        if let Some(t) = &tag {
            let tagged: HashSet<String> = self
                .store
                .all_note_tags(id)
                .await?
                .into_iter()
                .filter(|(_, tg)| tg == t || tg.starts_with(&format!("{t}/")))
                .map(|(p, _)| p)
                .collect();
            hits.retain(|(p, _, _)| tagged.contains(p));
        }
        if let Some(pp) = &path_prefix {
            let pref = pp.trim_start_matches('/');
            hits.retain(|(p, _, _)| p.starts_with(pref));
        }
        let notes_meta: HashMap<String, (String, Option<String>, bool)> = self
            .store
            .all_notes(id)
            .await?
            .into_iter()
            .map(|(p, t, ty, r)| (p, (t, ty, r)))
            .collect();
        if let Some(ty) = &okf_type {
            hits.retain(|(p, _, _)| {
                notes_meta
                    .get(p)
                    .and_then(|(_, t, _)| t.as_deref())
                    .is_some_and(|t| t.eq_ignore_ascii_case(ty))
            });
        }
        hits.truncate(limit as usize);
        Ok(hits
            .into_iter()
            .map(|(p, snip, score)| {
                let (title, _, reserved) =
                    notes_meta.get(&p).cloned().unwrap_or((p.clone(), None, false));
                SearchHit { path: p, title, snippet: snip, score, reserved }
            })
            .collect())
    }

    pub async fn switcher(self: &Arc<Self>, ws: &str, id: i64, q: &str) -> Result<Vec<SwitchHit>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let ix = { self.switcher.read().unwrap().get(&id).cloned() };
        let ix = match ix {
            Some(ix) => ix,
            None => {
                self.scan(id).await.ok();
                self.switcher
                    .read()
                    .unwrap()
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(SwitcherIx { rows: Vec::new() }))
            }
        };
        let ql = q.trim().to_lowercase();
        let mut out: Vec<SwitchHit> = Vec::new();
        for (path, title, aliases) in &ix.rows {
            if ql.is_empty() {
                out.push(SwitchHit { path: path.clone(), title: title.clone(), alias: None, score: 0.0 });
                if out.len() >= 50 {
                    break;
                }
                continue;
            }
            let mut best: Option<(f32, Option<String>)> = None;
            for (cand, alias) in std::iter::once((title.clone(), None))
                .chain(std::iter::once((path.clone(), None)))
                .chain(aliases.iter().map(|a| (a.clone(), Some(a.clone()))))
            {
                if let Some(s) = fuzzy_score(&ql, &cand.to_lowercase()) {
                    if best.as_ref().map(|(b, _)| s > *b).unwrap_or(true) {
                        best = Some((s, alias));
                    }
                }
            }
            if let Some((score, alias)) = best {
                out.push(SwitchHit { path: path.clone(), title: title.clone(), alias, score });
            }
        }
        if !ql.is_empty() {
            out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            out.truncate(50);
        }
        Ok(out)
    }

    pub async fn tags(self: &Arc<Self>, ws: &str, id: i64) -> Result<Vec<TagCount>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        self.store.tag_counts(id).await
    }

    pub async fn backlinks(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<Vec<Backlink>> {
        let v = self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rel = Self::check_rel(path)?;
        let mut out = Vec::new();
        for (src, title, kind) in self.store.backlinks(id, &rel).await? {
            // Context: first line mentioning the target (wikilink or md link).
            let abs = Self::abs_guarded(&v.root_path, &src)?;
            let context = tokio::fs::read_to_string(&abs)
                .await
                .ok()
                .and_then(|c| {
                    let stem = rel.rsplit('/').next().unwrap_or(&rel);
                    let stem_noext = stem.strip_suffix(".md").unwrap_or(stem).to_lowercase();
                    c.lines()
                        .find(|l| {
                            let ll = l.to_lowercase();
                            ll.contains(&stem_noext) || ll.contains(&rel.to_lowercase())
                        })
                        .map(|l| {
                            let t = l.trim();
                            if t.chars().count() > 240 {
                                t.chars().take(240).collect::<String>()
                            } else {
                                t.to_string()
                            }
                        })
                })
                .unwrap_or_default();
            out.push(Backlink { path: src, title, context, kind });
        }
        Ok(out)
    }

    /// Absolute, guarded path of an attachment for streaming.
    pub async fn asset_path(&self, ws: &str, id: i64, path: &str) -> Result<PathBuf> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        if !abs.is_file() {
            return Err(Error::NotFound(format!("asset {rel}")));
        }
        Ok(abs)
    }

    // -- graph -----------------------------------------------------------------

    pub async fn graph(self: &Arc<Self>, ws: &str, id: i64, o: &GraphOpts) -> Result<GraphPayload> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let notes = self.store.all_notes(id).await?;
        let edges_raw = self.store.all_edges(id).await?;
        let include_reserved = o.reserved;
        let orphans_ok = o.orphans.unwrap_or(true);
        let group_by_type = o.group_by == "type";

        // Node table: notes first.
        let mut index: HashMap<String, u32> = HashMap::new();
        let mut paths: Vec<String> = Vec::new();
        let mut titles: Vec<String> = Vec::new();
        let mut flags: Vec<u8> = Vec::new();
        let mut group_key: Vec<String> = Vec::new();
        for (p, t, ty, reserved) in &notes {
            if *reserved && !include_reserved {
                continue;
            }
            let gk = if group_by_type {
                ty.clone().unwrap_or_else(|| "—".to_string())
            } else {
                p.split_once('/').map(|(d, _)| d.to_string()).unwrap_or_else(|| "/".to_string())
            };
            index.insert(p.clone(), paths.len() as u32);
            paths.push(p.clone());
            titles.push(t.clone());
            flags.push(if *reserved { NODE_RESERVED } else { 0 });
            group_key.push(gk);
        }

        let mut edge_list: Vec<(u32, u32)> = Vec::new();
        for (s, d, _kind) in &edges_raw {
            let (Some(&si), Some(&di)) = (index.get(s), index.get(d)) else { continue };
            if si == di {
                continue;
            }
            edge_list.push((si, di));
        }

        // Ghost nodes for unresolved targets.
        if o.ghosts {
            let mut ghost_ix: HashMap<String, u32> = HashMap::new();
            for (src, raw) in self.store.all_ghost_edges(id).await? {
                let Some(&si) = index.get(&src) else { continue };
                let key = raw.trim().to_lowercase();
                let gi = *ghost_ix.entry(key).or_insert_with(|| {
                    let i = paths.len() as u32;
                    paths.push(format!("ghost:{raw}"));
                    titles.push(raw.clone());
                    flags.push(NODE_GHOST);
                    group_key.push("unresolved".to_string());
                    i
                });
                edge_list.push((si, gi));
            }
        }

        // Tag nodes.
        if o.tags {
            let mut tag_ix: HashMap<String, u32> = HashMap::new();
            for (p, tag) in self.store.all_note_tags(id).await? {
                let Some(&si) = index.get(&p) else { continue };
                let ti = *tag_ix.entry(tag.clone()).or_insert_with(|| {
                    let i = paths.len() as u32;
                    paths.push(format!("tag:{tag}"));
                    titles.push(format!("#{tag}"));
                    flags.push(NODE_TAG);
                    group_key.push("tags".to_string());
                    i
                });
                edge_list.push((si, ti));
            }
        }

        // Local mode: BFS from the focus.
        if o.mode == "local" {
            let focus = o
                .path
                .as_deref()
                .ok_or_else(|| Error::Invalid("local graph requires `path`".into()))?;
            let focus_rel = Self::check_rel(focus)?;
            let Some(&fi) = index.get(&focus_rel) else {
                return Err(Error::NotFound(format!("note {focus_rel}")));
            };
            let depth = o.depth.clamp(1, 3);
            let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
            for (a, b) in &edge_list {
                adj.entry(*a).or_default().push(*b);
                adj.entry(*b).or_default().push(*a);
            }
            let mut keep: HashSet<u32> = HashSet::from([fi]);
            let mut frontier = vec![fi];
            for _ in 0..depth {
                let mut next = Vec::new();
                for n in frontier {
                    for m in adj.get(&n).into_iter().flatten() {
                        if keep.insert(*m) {
                            next.push(*m);
                        }
                    }
                }
                frontier = next;
            }
            let mut remap: HashMap<u32, u32> = HashMap::new();
            let mut np = Vec::new();
            let mut nt = Vec::new();
            let mut nf = Vec::new();
            let mut ng = Vec::new();
            for i in 0..paths.len() as u32 {
                if keep.contains(&i) {
                    remap.insert(i, np.len() as u32);
                    np.push(paths[i as usize].clone());
                    nt.push(titles[i as usize].clone());
                    nf.push(flags[i as usize]);
                    ng.push(group_key[i as usize].clone());
                }
            }
            edge_list.retain(|(a, b)| keep.contains(a) && keep.contains(b));
            let edges: Vec<u32> = edge_list
                .iter()
                .flat_map(|(a, b)| [remap[a], remap[b]])
                .collect();
            return Ok(finish_graph(np, nt, nf, ng, edges, false, orphans_ok));
        }

        // Full mode: edge budget (degree-prioritized, deterministic).
        let budget = if o.edge_budget == 0 { DEFAULT_EDGE_BUDGET } else { o.edge_budget };
        let mut truncated = false;
        if edge_list.len() > budget {
            truncated = true;
            let mut deg: Vec<u32> = vec![0; paths.len()];
            for (a, b) in &edge_list {
                deg[*a as usize] += 1;
                deg[*b as usize] += 1;
            }
            edge_list.sort_by_key(|(a, b)| {
                std::cmp::Reverse(deg[*a as usize] + deg[*b as usize])
            });
            edge_list.truncate(budget);
        }
        let edges: Vec<u32> = edge_list.iter().flat_map(|(a, b)| [*a, *b]).collect();
        Ok(finish_graph(paths, titles, flags, group_key, edges, truncated, orphans_ok))
    }
}

/// Compact group keys into u16 ids + drop orphans when asked.
fn finish_graph(
    paths: Vec<String>,
    titles: Vec<String>,
    flags: Vec<u8>,
    group_key: Vec<String>,
    edges: Vec<u32>,
    truncated: bool,
    orphans_ok: bool,
) -> GraphPayload {
    let (paths, titles, flags, group_key, edges) = if orphans_ok {
        (paths, titles, flags, group_key, edges)
    } else {
        let mut connected: HashSet<u32> = HashSet::new();
        for e in &edges {
            connected.insert(*e);
        }
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut np = Vec::new();
        let mut nt = Vec::new();
        let mut nf = Vec::new();
        let mut ng = Vec::new();
        for i in 0..paths.len() as u32 {
            if connected.contains(&i) {
                remap.insert(i, np.len() as u32);
                np.push(paths[i as usize].clone());
                nt.push(titles[i as usize].clone());
                nf.push(flags[i as usize]);
                ng.push(group_key[i as usize].clone());
            }
        }
        let edges = edges.iter().map(|e| remap[e]).collect();
        (np, nt, nf, ng, edges)
    };
    let mut label_ix: HashMap<String, u16> = HashMap::new();
    let mut group_labels: Vec<String> = Vec::new();
    let groups: Vec<u16> = group_key
        .into_iter()
        .map(|g| {
            *label_ix.entry(g.clone()).or_insert_with(|| {
                group_labels.push(g);
                (group_labels.len() - 1) as u16
            })
        })
        .collect();
    GraphPayload { paths, titles, groups, group_labels, flags, edges, truncated }
}

/// New raw target for a link whose destination moved. Preserves the author's
/// style: wiki path/basename form + extension presence; md links become a
/// fresh relative path.
fn new_raw_for(old_raw: &str, kind: &str, src_dir_now: &str, new_dst: &str) -> String {
    match kind {
        "md" => {
            if old_raw.starts_with('/') {
                format!("/{new_dst}")
            } else {
                relative_path(src_dir_now, new_dst)
            }
        }
        _ => {
            let had_ext = old_raw.to_ascii_lowercase().ends_with(".md");
            let stripped = if had_ext {
                new_dst.to_string()
            } else {
                new_dst.strip_suffix(".md").unwrap_or(new_dst).to_string()
            };
            if old_raw.starts_with('/') {
                format!("/{stripped}")
            } else if old_raw.contains('/') {
                stripped
            } else {
                // Basename style — keep it short.
                let base = stripped.rsplit('/').next().unwrap_or(&stripped);
                base.to_string()
            }
        }
    }
}

/// Relative path from `from_dir` (vault-relative dir, "" = root) to `to`.
fn relative_path(from_dir: &str, to: &str) -> String {
    let from_parts: Vec<&str> = if from_dir.is_empty() { vec![] } else { from_dir.split('/').collect() };
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from_parts.len() - common;
    let mut out: Vec<String> = std::iter::repeat_n("..".to_string(), ups).collect();
    out.extend(to_parts[common..].iter().map(|s| s.to_string()));
    out.join("/")
}

/// Subsequence fuzzy score (mirrors ui/src/lib/fuzzy.ts): boundary + consecutive
/// bonuses, gap penalty. `None` when not a subsequence.
fn fuzzy_score(query: &str, cand: &str) -> Option<f32> {
    let q: Vec<char> = query.chars().collect();
    let c: Vec<char> = cand.chars().collect();
    if q.is_empty() {
        return Some(0.0);
    }
    let mut score = 0.0f32;
    let mut qi = 0usize;
    let mut last_hit: Option<usize> = None;
    for (i, ch) in c.iter().enumerate() {
        if qi < q.len() && ch.eq_ignore_ascii_case(&q[qi]) {
            let boundary = i == 0 || matches!(c[i - 1], ' ' | '/' | '-' | '_' | '.');
            score += 1.0
                + if boundary { 1.5 } else { 0.0 }
                + if last_hit == Some(i.wrapping_sub(1)) { 1.0 } else { 0.0 };
            last_hit = Some(i);
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    // Shorter candidates win ties.
    Some(score - (c.len() as f32) * 0.01)
}

/// Build a safe FTS5 MATCH expression: bare terms, quoted, prefix-matched.
fn fts_expr(text: &str) -> String {
    text.split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex_sha256(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    format!("{:x}", h.finalize())
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

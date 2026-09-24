//! The vault engine: registration, scanning (incremental, coalesced), note
//! CRUD with trash + rename-rewrites-links, search/switcher/tags/backlinks,
//! and the graph payload. Files on disk are the source of truth throughout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use otto_core::{Error, Result};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::parse;
use crate::resolve::ResolveIndex;
use crate::scan::{self, WalkResult, MAX_FTS_BYTES};
use crate::store::Store;
use crate::types::*;

/// Reads served without a rescan for this long after the last one (freshness
/// window — external edits surface within it; API/MCP writes rescan eagerly).
const STALE_AFTER_SECS: i64 = 5;
/// `mode=full` graph default edge budget (override via `edge_budget`).
const DEFAULT_EDGE_BUDGET: usize = 2_000_000;

type VaultWriteKey = (i64, String);
type VaultWriteLock = Arc<tokio::sync::Mutex<()>>;

/// Created only while publication is held, and dropped before that lock. Errors
/// or cancellation after a durable write invalidate derived caches, never source.
struct IndexRepair<'a> {
    state: &'a crate::index::IndexState,
    last_scan: Arc<AtomicI64>,
    complete: bool,
}
impl Drop for IndexRepair<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.state.invalidate();
            self.state.links_dirty.store(true, Ordering::Relaxed);
            self.last_scan.store(0, Ordering::Relaxed);
        }
    }
}

pub struct VaultEngine {
    store: Store,
    preparation: crate::prepare::Preparation,
    indexes: Mutex<HashMap<i64, Arc<crate::index::IndexState>>>,
    #[cfg(test)]
    walks: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    scan_pause: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
    #[cfg(test)]
    scan_override: Mutex<Option<WalkResult>>,
    /// Per-vault scan serialization + coalescing (a kick while a scan runs is
    /// dropped — the running scan picks up the changes anyway).
    scans: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    pending_scans: Mutex<HashSet<i64>>,
    /// Unix seconds of the last completed scan per vault (staleness probe).
    last_scan: Mutex<HashMap<i64, Arc<AtomicI64>>>,
    generations: Mutex<HashMap<i64, Arc<AtomicI64>>>,
    /// Serialize vault mutations, including folder moves and link rewrites.
    /// Scans have their own lock; mutation holders may await a scan.
    writes: Mutex<HashMap<VaultWriteKey, VaultWriteLock>>,
    fts_ok: std::sync::atomic::AtomicU8, // 0 unknown / 1 yes / 2 no
}

impl VaultEngine {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            store: Store::new(pool),
            preparation: Default::default(),
            indexes: Default::default(),
            #[cfg(test)]
            walks: Default::default(),
            #[cfg(test)]
            scan_pause: Default::default(),
            #[cfg(test)]
            scan_override: Default::default(),
            scans: Mutex::new(HashMap::new()),
            pending_scans: Mutex::new(HashSet::new()),
            last_scan: Mutex::new(HashMap::new()),
            generations: Mutex::new(HashMap::new()),
            writes: Mutex::new(HashMap::new()),
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

    pub async fn patch(
        &self,
        ws: &str,
        id: i64,
        name: Option<&str>,
        okf: Option<bool>,
    ) -> Result<VaultRec> {
        self.get_scoped(ws, id).await?;
        self.store.patch_vault(id, name, okf).await?;
        self.store.get_vault(id).await
    }

    /// Unregister — index rows only; the files on disk are untouched.
    pub async fn unregister(&self, ws: &str, id: i64) -> Result<()> {
        self.get_scoped(ws, id).await?;
        let write = self.write_lock(id, "");
        let _write = write.lock().await;
        let state = self.index_state(id);
        let _publication = state.publication.lock().await;
        self.store.delete_vault(id).await?;
        state.retire();
        self.indexes.lock().unwrap().remove(&id);
        self.last_scan.lock().unwrap().remove(&id);
        self.generations.lock().unwrap().remove(&id);
        Ok(())
    }

    pub(crate) fn index_state(&self, id: i64) -> Arc<crate::index::IndexState> {
        self.indexes.lock().unwrap().entry(id).or_default().clone()
    }

    /// Caller owns publication. Ordinary note updates never load global links.
    async fn index_prepared(
        &self,
        id: i64,
        state: &crate::index::IndexState,
        mut note: crate::prepare::PreparedNote,
        reconcile_added: bool,
    ) -> Result<bool> {
        state.check_active()?;
        let (added, resolver) = {
            let cached = state.cache.read().unwrap();
            let index = cached
                .as_ref()
                .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?;
            let added = !index.records.contains_key(&note.row.path);
            if added && reconcile_added {
                let mut resolver = index.resolver.clone();
                resolver.insert(note.row.path.clone());
                for link in &mut note.links {
                    link.dst_path = resolver.resolve(&note.row.path, &link.raw_target);
                }
                (true, Some(resolver))
            } else {
                for link in &mut note.links {
                    link.dst_path = index.resolver.resolve(&note.row.path, &link.raw_target);
                }
                (added, None)
            }
        };
        let mut incoming = vec![];
        if added && reconcile_added {
            let resolver = resolver.as_ref().unwrap();
            for (rowid, source, raw, dst) in self.store.all_links_full(id).await? {
                if source == note.row.path {
                    continue;
                }
                let next = resolver.resolve(&source, &raw);
                if next != dst {
                    incoming.push((rowid, next));
                }
            }
        }
        if added && !reconcile_added {
            state.links_dirty.store(true, Ordering::Relaxed);
        }
        let record = crate::index::IndexRecord::note(&note.row);
        let mut repair = IndexRepair {
            state,
            last_scan: self.last_scan_cell(id),
            complete: false,
        };
        self.store
            .index_note(id, &note, &incoming, self.fts_ready().await)
            .await?;
        #[cfg(test)]
        Self::pause_publication(state).await;
        state
            .cache
            .write()
            .unwrap()
            .as_mut()
            .expect("publication owns cache")
            .upsert(record);
        self.generation(id).fetch_add(1, Ordering::Relaxed);
        repair.complete = true;
        Ok(added)
    }

    #[cfg(test)]
    async fn pause_publication(state: &crate::index::IndexState) {
        let pause = state.publication_pause.lock().unwrap().take();
        if let Some((started, resume)) = pause {
            let _ = started.send(());
            let _ = resume.await;
        }
    }

    async fn reconcile_links(&self, id: i64, state: &crate::index::IndexState) -> Result<()> {
        let rows = self.store.all_links_full(id).await?;
        let changed = {
            let cache = state.cache.read().unwrap();
            let index = cache
                .as_ref()
                .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?;
            rows.into_iter()
                .filter_map(|(rowid, src, raw, dst)| {
                    let next = index.resolver.resolve(&src, &raw);
                    (next != dst).then_some((rowid, next))
                })
                .collect::<Vec<_>>()
        };
        self.store.update_link_destinations(id, &changed).await
    }

    // -- scanning ---------------------------------------------------------------

    fn scan_lock(&self, id: i64) -> Arc<tokio::sync::Mutex<()>> {
        self.scans.lock().unwrap().entry(id).or_default().clone()
    }

    fn last_scan_cell(&self, id: i64) -> Arc<AtomicI64> {
        self.last_scan
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .clone()
    }

    fn generation(&self, id: i64) -> Arc<AtomicI64> {
        self.generations
            .lock()
            .unwrap()
            .entry(id)
            .or_insert_with(|| {
                Arc::new(AtomicI64::new(
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                ))
            })
            .clone()
    }

    pub(crate) fn write_lock(&self, id: i64, _path: &str) -> VaultWriteLock {
        self.writes
            .lock()
            .unwrap()
            .entry((id, String::new()))
            .or_default()
            .clone()
    }

    /// Fire-and-forget scan kick (coalesced).
    pub fn kick_scan(self: &Arc<Self>, id: i64) {
        if !self.pending_scans.lock().unwrap().insert(id) {
            return;
        }
        let eng = self.clone();
        tokio::spawn(async move {
            let _ = eng.scan(id).await;
            eng.pending_scans.lock().unwrap().remove(&id);
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

    /// Rescan after a filesystem mutation that has ALREADY succeeded (move to
    /// trash, rename, restore). A scan failure here must not turn the call
    /// into an error: the UI would stay on the old path and later recreate
    /// the moved note there. Log it and retry in the background instead.
    pub(crate) async fn rescan_after_mutation(self: &Arc<Self>, id: i64) {
        if let Err(e) = self.scan(id).await {
            tracing::warn!(vault = id, error = %e, "vault rescan after mutation failed; retrying in background");
            self.kick_scan(id);
        }
    }

    /// Incremental scan (parse changed, drop removed, re-resolve links).
    /// Explicit scans always run after acquiring the lock: an older scan may
    /// have walked a path before our write, even if it completed after it.
    /// Background read kicks are coalesced separately in kick_scan().
    pub async fn scan(&self, id: i64) -> Result<()> {
        let lock = self.scan_lock(id);
        let _guard = lock.lock().await;
        let v = self.store.get_vault(id).await?;
        let root = PathBuf::from(&v.root_path);
        if !root.is_dir() {
            self.store
                .set_scan_state(
                    id,
                    &format!("error: vault root missing: {}", v.root_path),
                    false,
                )
                .await?;
            return Err(Error::Conflict(format!(
                "vault root missing: {}",
                v.root_path
            )));
        }
        self.store.set_scan_state(id, "scanning", false).await?;
        let res = self.scan_inner(id, &root).await;
        match &res {
            Ok(()) => {
                self.store.set_scan_state(id, "idle", true).await?;
                self.last_scan_cell(id)
                    .store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
            }
            Err(e) => {
                let _ = self
                    .store
                    .set_scan_state(id, &format!("error: {e}"), false)
                    .await;
            }
        }
        res
    }

    async fn scan_inner(&self, id: i64, root: &Path) -> Result<()> {
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let epoch = {
            let _publication = state.publication.lock().await;
            state.start_scan()
        };
        struct Finish(Arc<crate::index::IndexState>);
        impl Drop for Finish {
            fn drop(&mut self) {
                self.0.finish_scan();
            }
        }
        let _finish = Finish(state.clone());
        #[cfg(test)]
        self.walks.fetch_add(1, Ordering::Relaxed);
        let root_owned = root.to_path_buf();
        let walk: WalkResult = tokio::task::spawn_blocking(move || scan::walk(&root_owned))
            .await
            .map_err(|e| Error::Internal(format!("walk join: {e}")))?
            .map_err(|e| Error::Internal(format!("walk: {e}")))?;
        #[cfg(test)]
        let walk = self.scan_override.lock().unwrap().take().unwrap_or(walk);
        #[cfg(test)]
        {
            let pause = self.scan_pause.lock().unwrap().take();
            if let Some((started, resume)) = pause {
                let _ = started.send(());
                let _ = resume.await;
            }
        }
        let note_sigs = self.store.note_sigs(id).await?;
        let file_sigs = self.store.file_sigs(id).await?;
        let (mut changed_notes, removed_notes) = scan::diff(&walk.notes, &note_sigs);
        let (changed_files, removed_files) = scan::diff(&walk.files, &file_sigs);
        let legacy: Vec<String>=sqlx::query_scalar("SELECT path FROM vault_notes WHERE vault_id=? AND size>? AND content_index_status='full'").bind(id).bind(MAX_FTS_BYTES as i64).fetch_all(self.store.pool()).await.map_err(|e| Error::Internal(e.to_string()))?;
        let present: HashSet<_> = walk.notes.iter().map(|e| e.rel.as_str()).collect();
        let mut changed: HashSet<_> = changed_notes.iter().cloned().collect();
        for path in legacy {
            if present.contains(path.as_str()) && changed.insert(path.clone()) {
                changed_notes.push(path);
            }
        }
        let mut incomplete = !walk.complete;
        // A sequential stream retains one prepared body, never N tasks/bodies.
        // Added paths resolve all incoming links once after the entire scan.
        for rel in changed_notes {
            if state.changed_since(&rel, epoch) {
                continue;
            }
            let prepared = match self.preparation.file(root.join(&rel), rel.clone()).await {
                Ok(note) => note,
                // Transient (changed mid-read, canceled): retry the scan.
                Err(Error::Conflict(_)) => {
                    incomplete = true;
                    continue;
                }
                // A persistent per-file problem (unreadable, not a regular
                // file) must not fail every scan forever: the path is present
                // in the walk, so skipping it cannot prune anything wrongly.
                Err(e) => {
                    tracing::warn!(vault = id, path = %rel, error = %e, "vault note skipped by scan");
                    continue;
                }
            };
            let _publication = state.publication.lock().await;
            state.check_active()?;
            if state.changed_since(&rel, epoch) {
                continue;
            }
            if state.cache.read().unwrap().is_none() {
                incomplete = true;
                continue;
            }
            self.index_prepared(id, &state, prepared, false).await?;
        }
        let file_sizes: HashMap<_, _> = walk
            .files
            .iter()
            .map(|e| (e.rel.as_str(), (e.size, e.mtime_ns)))
            .collect();
        for rel in changed_files {
            let _publication = state.publication.lock().await;
            state.check_active()?;
            if state.changed_since(&rel, epoch) {
                continue;
            }
            let Some(&(size, mtime)) = file_sizes.get(rel.as_str()) else {
                continue;
            };
            let current = match tokio::fs::symlink_metadata(root.join(&rel)).await {
                Ok(meta) => meta,
                Err(_) => {
                    incomplete = true;
                    continue;
                }
            };
            if current.len() as i64 != size || crate::prepare::mtime(&current) != mtime {
                incomplete = true;
                continue;
            }
            let added = state
                .cache
                .read()
                .unwrap()
                .as_ref()
                .is_some_and(|cache| !cache.records.contains_key(&rel));
            if added {
                state.links_dirty.store(true, Ordering::Relaxed);
            }
            let mut repair = IndexRepair {
                state: &state,
                last_scan: self.last_scan_cell(id),
                complete: false,
            };
            self.store.upsert_file(id, &rel, size, mtime).await?;
            #[cfg(test)]
            Self::pause_publication(&state).await;
            let mut cached = state.cache.write().unwrap();
            if let Some(cache) = cached.as_mut() {
                cache.upsert(crate::index::IndexRecord::file(rel));
            } else {
                incomplete = true;
            }
            self.generation(id).fetch_add(1, Ordering::Relaxed);
            repair.complete = true;
        }
        // Any enumeration/preparation error suppresses pruning for this scan.
        if !incomplete {
            for (rel, note) in removed_notes
                .into_iter()
                .map(|p| (p, true))
                .chain(removed_files.into_iter().map(|p| (p, false)))
            {
                let _publication = state.publication.lock().await;
                state.check_active()?;
                if state.changed_since(&rel, epoch) {
                    continue;
                }
                match tokio::fs::symlink_metadata(root.join(&rel)).await {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    // Something answers at this path: only a byte-exact
                    // regular file is the indexed entry reappearing. A case
                    // variant (APFS case-only rename) or a symlink is not.
                    Ok(_) => {
                        let (root_owned, rel_owned) = (root.to_path_buf(), rel.clone());
                        let exact = tokio::task::spawn_blocking(move || {
                            scan::exact_regular_file(&root_owned, &rel_owned)
                        })
                        .await;
                        if !matches!(exact, Ok(Ok(false))) {
                            incomplete = true;
                            continue;
                        }
                    }
                    Err(_) => {
                        incomplete = true;
                        continue;
                    }
                }
                state.links_dirty.store(true, Ordering::Relaxed);
                let mut repair = IndexRepair {
                    state: &state,
                    last_scan: self.last_scan_cell(id),
                    complete: false,
                };
                if note {
                    self.store.remove_note(id, &rel).await?;
                } else {
                    self.store.remove_file(id, &rel).await?;
                }
                #[cfg(test)]
                Self::pause_publication(&state).await;
                if let Some(cache) = state.cache.write().unwrap().as_mut() {
                    cache.remove(&rel);
                }
                self.generation(id).fetch_add(1, Ordering::Relaxed);
                repair.complete = true;
            }
        }
        if state.links_dirty.load(Ordering::Relaxed) {
            let _publication = state.publication.lock().await;
            state.check_active()?;
            self.reconcile_links(id, &state).await?;
            state.links_dirty.store(false, Ordering::Relaxed);
        }
        if incomplete {
            return Err(Error::Conflict(
                "Vault scan incomplete; existing index entries were preserved, retry scan".into(),
            ));
        }
        Ok(())
    }

    pub async fn status(self: &Arc<Self>, ws: &str, id: i64) -> Result<VaultStatus> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let mut status = self.store.status(id).await?;
        status.generation = Some(self.generation(id).load(Ordering::Relaxed).to_string());
        Ok(status)
    }

    // -- path safety -------------------------------------------------------------

    /// Validate a client-supplied vault-relative path: no absolutes, no `..`,
    /// no backslashes, no NUL, not into `.trash`/hidden dirs.
    pub(crate) fn check_rel(path: &str) -> Result<String> {
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
                return Err(Error::Invalid(format!(
                    "hidden segments are not allowed: {path}"
                )));
            }
        }
        Ok(p.to_string())
    }

    /// Absolute path of `rel` inside the vault, symlink-escape-guarded: the
    /// canonicalized parent must stay under the canonicalized root.
    pub(crate) fn abs_guarded(root: &str, rel: &str) -> Result<PathBuf> {
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

    /// Fully canonicalized path of an EXISTING `rel` for reads that follow the
    /// path (asset streaming, backlink context). `abs_guarded` only vets the
    /// parent, so a final-component symlink (`logo.png -> ~/.ssh/id_rsa`)
    /// escaped the vault; here the resolved target itself must stay inside.
    pub(crate) fn abs_confined(root: &str, rel: &str) -> Result<PathBuf> {
        let rootc = Path::new(root)
            .canonicalize()
            .map_err(|e| Error::Conflict(format!("vault root missing: {e}")))?;
        let resolved = rootc
            .join(rel)
            .canonicalize()
            .map_err(|_| Error::NotFound(rel.to_string()))?;
        if !resolved.starts_with(&rootc) {
            return Err(Error::Forbidden("path escapes the vault".into()));
        }
        Ok(resolved)
    }

    /// Open (and create where absent) every parent component relative to a held
    /// vault directory capability. `NOFOLLOW` on each hop prevents a concurrent
    /// symlink swap from redirecting later reads or the final rename.
    pub(crate) fn text_parent(root: &str, rel: &str) -> Result<(OwnedFd, String)> {
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

    pub(crate) async fn text_file_bytes(parent: &OwnedFd, name: &str) -> Result<Option<Vec<u8>>> {
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
            Err(e) => return Err(Error::Internal(format!("open text artifact {name}: {e}"))),
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
    pub(crate) async fn atomic_replace_at(
        parent: &OwnedFd,
        name: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let temp = format!(".{name}.otto-tmp-{}", otto_core::new_id());
        let mode = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
            .map(|stat| Mode::from_bits_truncate(stat.st_mode & 0o777))
            .unwrap_or(Mode::RUSR | Mode::WUSR);
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
        let rel = if path.trim().is_empty() {
            String::new()
        } else {
            Self::check_rel(path)?
        };
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let entries = state
            .cache
            .read()
            .unwrap()
            .as_ref()
            .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?
            .dir(&rel);
        Ok(DirListing { path: rel, entries })
    }

    pub async fn note(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<NoteFull> {
        let v = self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        if !abs.exists() {
            return Err(Error::NotFound(format!("note {rel}")));
        }
        let (parent, name) = Self::text_parent(&v.root_path, &rel)?;
        let bytes = Self::text_file_bytes(&parent, &name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("note {rel}")))?;
        let raw =
            String::from_utf8(bytes).map_err(|_| Error::Invalid("note is not UTF-8".into()))?;
        // Metadata and hash describe these exact response bytes, even if an
        // external scan has not indexed them yet. Parsing uses the same limit.
        let mut prepared = self
            .preparation
            .bytes(rel.clone(), raw.as_bytes(), 0)
            .await?;
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        {
            let cache = state.cache.read().unwrap();
            let index = cache
                .as_ref()
                .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?;
            if index.records.contains_key(&rel) {
                for link in &mut prepared.links {
                    link.dst_path = index.resolver.resolve(&rel, &link.raw_target);
                }
            } else {
                let mut resolver = index.resolver.clone();
                resolver.insert(rel.clone());
                for link in &mut prepared.links {
                    link.dst_path = resolver.resolve(&rel, &link.raw_target);
                }
            }
        }
        Ok(NoteFull {
            meta: prepared.meta(),
            raw,
            outgoing: prepared.links,
        })
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
        let lock = self.write_lock(id, &rel);
        let _guard = lock.lock().await;
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let mut prepared = self
            .preparation
            .bytes(rel.clone(), content.as_bytes(), 0)
            .await?;
        let _publication = state.publication.lock().await;
        if state.cache.read().unwrap().is_none() {
            return Err(Error::Conflict(
                "Vault index is refreshing; retry save".into(),
            ));
        }
        let (parent, name) = Self::text_parent(&v.root_path, &rel)?;
        let before = Self::text_file_bytes(&parent, &name).await?;
        if let Some(expected) = if_hash {
            let current = before.as_deref().map(hex_sha256).unwrap_or_default();
            if current != expected {
                return Err(Error::Conflict(format!(
                    "note changed on disk (hash {current})"
                )));
            }
        }
        let revision = Self::prepare_revision(
            &v.root_path,
            &rel,
            before.as_deref(),
            content.as_bytes(),
            "note write",
        )
        .await?;
        state.mutated(&[&rel]);
        let mut repair = IndexRepair {
            state: &state,
            last_scan: self.last_scan_cell(id),
            complete: false,
        };
        Self::atomic_replace_at(&parent, &name, content.as_bytes()).await?;
        Self::commit_revision(&v.root_path, revision).await?;
        let written = rustix::fs::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| Error::Internal(format!("written note metadata: {e}")))?;
        prepared.row.mtime_ns = stat_mtime_ns(&written);
        self.index_prepared(id, &state, prepared, true).await?;
        repair.complete = true;
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
        if !matches!(
            ext.as_str(),
            "yaml" | "yml" | "json" | "d2" | "mmd" | "txt" | "csv"
        ) {
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
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let _publication = state.publication.lock().await;
        let added = !state
            .cache
            .read()
            .unwrap()
            .as_ref()
            .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?
            .records
            .contains_key(&rel);
        let (parent, name) = Self::text_parent(&v.root_path, &rel)?;
        let before = Self::text_file_bytes(&parent, &name).await?;
        if let Some(expected) = if_hash {
            let current = before.as_deref().map(hex_sha256).unwrap_or_default();
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
        let revision = Self::prepare_revision(
            &v.root_path,
            &rel,
            before.as_deref(),
            bytes,
            "artifact write",
        )
        .await?;
        state.mutated(&[&rel]);
        let mut repair = IndexRepair {
            state: &state,
            last_scan: self.last_scan_cell(id),
            complete: false,
        };
        Self::atomic_replace_at(&parent, &name, bytes).await?;
        Self::commit_revision(&v.root_path, revision).await?;
        let written = rustix::fs::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| Error::Internal(format!("written artifact metadata: {e}")))?;
        self.store
            .upsert_file(id, &rel, written.st_size, stat_mtime_ns(&written))
            .await?;
        state
            .cache
            .write()
            .unwrap()
            .as_mut()
            .expect("publication owns cache")
            .upsert(crate::index::IndexRecord::file(rel.clone()));
        if added {
            self.reconcile_links(id, &state).await?;
        }
        self.generation(id).fetch_add(1, Ordering::Relaxed);
        repair.complete = true;
        Ok(VaultTextFile {
            path: rel,
            size: bytes.len() as i64,
            hash: hex_sha256(bytes),
        })
    }

    /// Soft delete → `<vault>/.trash/<path>` (never destroys user files).
    pub async fn delete_note(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<()> {
        let v = self.get_scoped(ws, id).await?;
        let lock = self.write_lock(id, "");
        let _guard = lock.lock().await;
        let rel = Self::check_rel(path)?;
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let publication = state.publication.lock().await;
        state.mutated(&[&rel]);
        let abs = Self::abs_guarded(&v.root_path, &rel)?;
        if !abs.exists() {
            return Err(Error::NotFound(format!("note {rel}")));
        }
        let mut dest = Path::new(&v.root_path).join(".trash").join(&rel);
        if dest.exists() {
            let stamp = otto_core::new_id();
            dest = dest.with_file_name(format!(
                "{}-{stamp}",
                dest.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        // Write the manifest before moving: a crash after the move still leaves
        // the original destination and deletion timestamp recoverable.
        let entry = VaultTrashEntry {
            id: otto_core::new_id().to_string(),
            original_path: rel.clone(),
            stored_path: dest
                .strip_prefix(Path::new(&v.root_path).join(".trash"))
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            deleted_at: chrono::Utc::now().to_rfc3339(),
            kind: if abs.is_dir() { "dir" } else { "file" }.into(),
        };
        Self::record_trash(&v.root_path, &entry).await?;
        let (source_parent, source_name) = Self::text_parent(&v.root_path, &rel)?;
        let (trash_parent, trash_name) =
            Self::text_parent(&v.root_path, &format!(".trash/{}", entry.stored_path))?;
        rustix::fs::renameat_with(
            &source_parent,
            source_name.as_str(),
            &trash_parent,
            trash_name.as_str(),
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| Error::Internal(format!("trash move: {e}")))?;
        drop(publication);
        self.rescan_after_mutation(id).await;
        Ok(())
    }

    pub async fn create_folder(self: &Arc<Self>, ws: &str, id: i64, path: &str) -> Result<()> {
        let v = self.get_scoped(ws, id).await?;
        let lock = self.write_lock(id, "");
        let _guard = lock.lock().await;
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
        let lock = self.write_lock(id, "");
        let _guard = lock.lock().await;
        let from_rel = Self::check_rel(from)?;
        let to_rel = Self::check_rel(to)?;
        if from_rel == to_rel {
            return Err(Error::Invalid("from and to are the same path".into()));
        }
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let publication = state.publication.lock().await;
        state.mutated(&[&from_rel, &to_rel]);
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
        // …and AFTER it: every rewritten raw is checked against this so a
        // shortened form can never land on a different note (a same-folder
        // or now-ambiguous basename wins over the moved target).
        let mut ix_after = ix_before.clone();
        for (old, new) in &moved {
            ix_after.remove(old);
            ix_after.insert(new.clone());
        }

        let mut links_updated = 0i64;
        for src in &affected {
            let src_now = src.clone();
            let abs = Self::abs_guarded(&root, &src_now)?;
            let Ok(content) = tokio::fs::read_to_string(&abs).await else {
                continue;
            };
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
                let src_moved = src_before != src_now;
                if moved_to.is_none() && !src_moved {
                    return None;
                }
                let desired = moved_to.unwrap_or(&dst_old);
                // Still lands on the same note from where the source now
                // lives (unchanged basename, `/`-absolute to a stayed note…).
                if ix_after.resolve(&src_now, raw).as_ref() == Some(desired) {
                    return None;
                }
                let src_dir_now = src_now.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                // Target moved → its new home in the link's own style; target
                // stayed but THIS note moved → recompute relative forms (md,
                // wiki and embeds alike) from the new folder.
                let styled = new_raw_for(raw, kind, src_dir_now, desired);
                let fixed = resolving_raw(&ix_after, &src_now, kind, styled, desired);
                if fixed == raw {
                    return None;
                }
                count_here += 1;
                Some(fixed)
            });
            if new_content != content && count_here > 0 {
                let revision = Self::prepare_revision(
                    &root,
                    &src_now,
                    Some(content.as_bytes()),
                    new_content.as_bytes(),
                    "rename links",
                )
                .await?;
                let (parent, name) = Self::text_parent(&root, &src_now)?;
                state.mutated(&[&src_now]);
                Self::atomic_replace_at(&parent, &name, new_content.as_bytes()).await?;
                Self::commit_revision(&root, revision).await?;
                links_updated += count_here;
            }
        }

        // One scan picks up the moved files, rewritten sources, and re-resolves
        // everything (including newly-ambiguous basenames).
        drop(publication);
        self.rescan_after_mutation(id).await;
        Ok(RenameResult {
            from: from_rel,
            to: to_rel,
            links_updated,
        })
    }

    // -- search / switcher / tags / backlinks ---------------------------------------

    pub async fn search(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        req: &SearchReq,
    ) -> Result<Vec<SearchHit>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let limit = if req.limit == 0 || req.limit > 200 {
            50
        } else {
            req.limit
        } as i64;
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

        let mut hits: Vec<(String, String, f32)> =
            if !text.trim().is_empty() && self.fts_ready().await {
                let expr = fts_expr(&text);
                let got = self
                    .store
                    .fts_search(id, &expr, limit * 4)
                    .await
                    .unwrap_or_default();
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
                    notes_meta
                        .get(&p)
                        .cloned()
                        .unwrap_or((p.clone(), None, false));
                SearchHit {
                    path: p,
                    title,
                    snippet: snip,
                    score,
                    reserved,
                }
            })
            .collect())
    }

    pub async fn switcher(self: &Arc<Self>, ws: &str, id: i64, q: &str) -> Result<Vec<SwitchHit>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let state = self.index_state(id);
        state.ensure(&self.store, id).await?;
        let cached = state.cache.read().unwrap();
        let ix = cached
            .as_ref()
            .ok_or_else(|| Error::Conflict("Vault index is refreshing; retry".into()))?;
        let ql = q.trim().to_lowercase();
        let mut out: Vec<SwitchHit> = Vec::new();
        for record in ix
            .records
            .values()
            .filter(|r| r.kind == "note" && !r.reserved)
        {
            let (path, title, aliases) = (
                &record.path,
                record.title.as_ref().unwrap(),
                &record.aliases,
            );
            if ql.is_empty() {
                out.push(SwitchHit {
                    path: path.clone(),
                    title: title.clone(),
                    alias: None,
                    score: 0.0,
                });
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
                out.push(SwitchHit {
                    path: path.clone(),
                    title: title.clone(),
                    alias,
                    score,
                });
            }
        }
        if !ql.is_empty() {
            out.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            out.truncate(50);
        }
        Ok(out)
    }

    pub async fn tags(self: &Arc<Self>, ws: &str, id: i64) -> Result<Vec<TagCount>> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        self.store.tag_counts(id).await
    }

    pub async fn backlinks(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        path: &str,
    ) -> Result<Vec<Backlink>> {
        let v = self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rel = Self::check_rel(path)?;
        let mut out = Vec::new();
        for (src, title, kind) in self.store.backlinks(id, &rel).await? {
            // Context: first line mentioning the target (wikilink or md link).
            // A source that resolves outside the vault contributes no context.
            let abs = Self::abs_confined(&v.root_path, &src).ok();
            let context = match abs {
                Some(abs) => tokio::fs::read_to_string(&abs).await.ok(),
                None => None,
            }
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
            out.push(Backlink {
                path: src,
                title,
                context,
                kind,
            });
        }
        Ok(out)
    }

    /// Absolute, guarded path of an attachment for streaming.
    pub async fn asset_path(&self, ws: &str, id: i64, path: &str) -> Result<PathBuf> {
        let v = self.get_scoped(ws, id).await?;
        let rel = Self::check_rel(path)?;
        let abs = Self::abs_confined(&v.root_path, &rel).map_err(|e| match e {
            Error::NotFound(_) => Error::NotFound(format!("asset {rel}")),
            other => other,
        })?;
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

        // Per-note tags. Loaded unconditionally: they are a filterable node
        // ATTRIBUTE regardless of whether `tags` also draws them as nodes.
        let mut note_tags: HashMap<&str, Vec<String>> = HashMap::new();
        let tag_rows = self.store.all_note_tags(id).await?;
        for (p, tag) in &tag_rows {
            note_tags.entry(p.as_str()).or_default().push(tag.clone());
        }

        // Node table: notes first.
        let mut index: HashMap<String, u32> = HashMap::new();
        let mut nodes = NodeTable::default();
        for (p, t, ty, reserved) in &notes {
            if *reserved && !include_reserved {
                continue;
            }
            let service = p
                .split_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| SERVICE_ROOT.into());
            let i = nodes.push(
                p.clone(),
                t.clone(),
                if *reserved { NODE_RESERVED } else { 0 },
                ty.clone().unwrap_or_default(),
                service,
                note_tags.remove(p.as_str()).unwrap_or_default(),
            );
            index.insert(p.clone(), i);
        }

        let mut edge_list: Vec<(u32, u32)> = Vec::new();
        for (s, d, _kind) in &edges_raw {
            let (Some(&si), Some(&di)) = (index.get(s), index.get(d)) else {
                continue;
            };
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
                    nodes.push(
                        format!("ghost:{raw}"),
                        raw.clone(),
                        NODE_GHOST,
                        TYPE_GHOST.into(),
                        SERVICE_GHOST.into(),
                        Vec::new(),
                    )
                });
                edge_list.push((si, gi));
            }
        }

        // Tag nodes.
        if o.tags {
            let mut tag_ix: HashMap<String, u32> = HashMap::new();
            for (p, tag) in &tag_rows {
                let Some(&si) = index.get(p) else { continue };
                let ti = *tag_ix.entry(tag.clone()).or_insert_with(|| {
                    nodes.push(
                        format!("tag:{tag}"),
                        format!("#{tag}"),
                        NODE_TAG,
                        TYPE_TAG.into(),
                        SERVICE_TAGS.into(),
                        Vec::new(),
                    )
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
            let remap = nodes.retain(&keep);
            edge_list.retain(|(a, b)| keep.contains(a) && keep.contains(b));
            let edges: Vec<u32> = edge_list
                .iter()
                .flat_map(|(a, b)| [remap[a], remap[b]])
                .collect();
            return Ok(finish_graph(nodes, edges, false, orphans_ok));
        }

        // Full mode: edge budget (degree-prioritized, deterministic).
        let budget = if o.edge_budget == 0 {
            DEFAULT_EDGE_BUDGET
        } else {
            o.edge_budget
        };
        let mut truncated = false;
        if edge_list.len() > budget {
            truncated = true;
            let mut deg: Vec<u32> = vec![0; nodes.len()];
            for (a, b) in &edge_list {
                deg[*a as usize] += 1;
                deg[*b as usize] += 1;
            }
            edge_list.sort_by_key(|(a, b)| std::cmp::Reverse(deg[*a as usize] + deg[*b as usize]));
            edge_list.truncate(budget);
        }
        let edges: Vec<u32> = edge_list.iter().flat_map(|(a, b)| [*a, *b]).collect();
        Ok(finish_graph(nodes, edges, truncated, orphans_ok))
    }
}

/// The node table under construction — parallel arrays kept in lockstep so a
/// single `retain` can prune every attribute at once.
#[derive(Default)]
struct NodeTable {
    paths: Vec<String>,
    titles: Vec<String>,
    flags: Vec<u8>,
    /// Raw frontmatter `type` (`""` = untyped); normalized at wire time.
    type_raw: Vec<String>,
    service: Vec<String>,
    tags: Vec<Vec<String>>,
}

impl NodeTable {
    fn len(&self) -> usize {
        self.paths.len()
    }

    fn push(
        &mut self,
        path: String,
        title: String,
        flags: u8,
        type_raw: String,
        service: String,
        tags: Vec<String>,
    ) -> u32 {
        let i = self.paths.len() as u32;
        self.paths.push(path);
        self.titles.push(title);
        self.flags.push(flags);
        self.type_raw.push(type_raw);
        self.service.push(service);
        self.tags.push(tags);
        i
    }

    /// Keep only `keep` (order-preserving); returns the old → new index map.
    fn retain(&mut self, keep: &HashSet<u32>) -> HashMap<u32, u32> {
        let mut remap = HashMap::with_capacity(keep.len());
        let mut next = 0u32;
        let mut i = 0u32;
        self.paths.retain(|_| {
            let k = keep.contains(&i);
            if k {
                remap.insert(i, next);
                next += 1;
            }
            i += 1;
            k
        });
        self.titles = take_kept(std::mem::take(&mut self.titles), &remap);
        self.flags = take_kept(std::mem::take(&mut self.flags), &remap);
        self.type_raw = take_kept(std::mem::take(&mut self.type_raw), &remap);
        self.service = take_kept(std::mem::take(&mut self.service), &remap);
        self.tags = take_kept(std::mem::take(&mut self.tags), &remap);
        remap
    }
}

/// Keep the entries whose original index survived into `remap`.
fn take_kept<T>(v: Vec<T>, remap: &HashMap<u32, u32>) -> Vec<T> {
    v.into_iter()
        .enumerate()
        .filter(|(i, _)| remap.contains_key(&(*i as u32)))
        .map(|(_, x)| x)
        .collect()
}

/// Intern a per-node string attribute into ids + a label table.
fn intern(values: &[String]) -> (Vec<u16>, Vec<String>) {
    let mut ids: HashMap<&str, u16> = HashMap::new();
    let mut labels: Vec<String> = Vec::new();
    let out = values
        .iter()
        .map(|v| {
            *ids.entry(v.as_str()).or_insert_with(|| {
                labels.push(v.clone());
                (labels.len() - 1) as u16
            })
        })
        .collect();
    (out, labels)
}

/// Intern OKF types case-insensitively, so `Flow` and `flow` land in ONE bucket.
/// The label is the most frequent original casing (ties → lexicographically
/// smallest, keeping the payload deterministic across scans).
fn intern_types(values: &[String]) -> (Vec<u16>, Vec<String>) {
    let mut ids: HashMap<String, u16> = HashMap::new();
    let mut casings: Vec<HashMap<String, usize>> = Vec::new();
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        let display = if v.trim().is_empty() {
            TYPE_UNTYPED
        } else {
            v.trim()
        };
        let id = *ids.entry(display.to_lowercase()).or_insert_with(|| {
            casings.push(HashMap::new());
            (casings.len() - 1) as u16
        });
        *casings[id as usize].entry(display.to_string()).or_default() += 1;
        out.push(id);
    }
    let labels = casings
        .iter()
        .map(|c| {
            c.iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(s, _)| s.clone())
                .unwrap_or_default()
        })
        .collect();
    (out, labels)
}

/// Flatten per-node tag lists into CSR: node `i` owns `ids[off[i]..off[i+1]]`.
fn intern_tags(per_node: &[Vec<String>]) -> (Vec<u32>, Vec<u16>, Vec<String>) {
    let mut ix: HashMap<&str, u16> = HashMap::new();
    let mut labels: Vec<String> = Vec::new();
    let mut off: Vec<u32> = Vec::with_capacity(per_node.len() + 1);
    let mut ids: Vec<u16> = Vec::new();
    off.push(0);
    for tags in per_node {
        for t in tags {
            let id = *ix.entry(t.as_str()).or_insert_with(|| {
                labels.push(t.clone());
                (labels.len() - 1) as u16
            });
            ids.push(id);
        }
        off.push(ids.len() as u32);
    }
    (off, ids, labels)
}

/// Drop orphans when asked, then compact the node attributes into id + label
/// tables for the wire.
fn finish_graph(
    mut nodes: NodeTable,
    edges: Vec<u32>,
    truncated: bool,
    orphans_ok: bool,
) -> GraphPayload {
    let edges = if orphans_ok {
        edges
    } else {
        let connected: HashSet<u32> = edges.iter().copied().collect();
        let remap = nodes.retain(&connected);
        edges.iter().map(|e| remap[e]).collect()
    };
    let (types, type_labels) = intern_types(&nodes.type_raw);
    let (services, service_labels) = intern(&nodes.service);
    let (tag_off, tag_ids, tag_labels) = intern_tags(&nodes.tags);
    GraphPayload {
        paths: nodes.paths,
        titles: nodes.titles,
        types,
        type_labels,
        services,
        service_labels,
        tag_off,
        tag_ids,
        tag_labels,
        flags: nodes.flags,
        edges,
        truncated,
    }
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

/// `styled` if it resolves (from `src`) to `dst`; otherwise the full vault
/// path, then the `/`-absolute form — so a rewrite never retargets a link to a
/// different note that happens to share the shortened name.
fn resolving_raw(ix: &ResolveIndex, src: &str, kind: &str, styled: String, dst: &str) -> String {
    let lands = |cand: &str| ix.resolve(src, cand).as_deref() == Some(dst);
    if lands(&styled) {
        return styled;
    }
    let full = if kind == "md" || styled.to_ascii_lowercase().ends_with(".md") {
        dst.to_string()
    } else {
        dst.strip_suffix(".md").unwrap_or(dst).to_string()
    };
    let absolute = format!("/{full}");
    if lands(&full) {
        full
    } else if lands(&absolute) {
        absolute
    } else {
        styled
    }
}

/// Relative path from `from_dir` (vault-relative dir, "" = root) to `to`.
fn relative_path(from_dir: &str, to: &str) -> String {
    let from_parts: Vec<&str> = if from_dir.is_empty() {
        vec![]
    } else {
        from_dir.split('/').collect()
    };
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
                + if last_hit == Some(i.wrapping_sub(1)) {
                    1.0
                } else {
                    0.0
                };
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

/// rustix uses signed nanoseconds on macOS, unsigned c_ulong/u32 on Linux.
/// Valid stat nanoseconds are below one billion, so both fit our signed index.
#[allow(clippy::unnecessary_cast)] // Stat field widths/signedness vary by target.
fn stat_mtime_ns(stat: &rustix::fs::Stat) -> i64 {
    (stat.st_mtime as i64)
        .saturating_mul(1_000_000_000)
        .saturating_add(stat.st_mtime_nsec as i64)
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

#[cfg(test)]
#[path = "performance_tests.rs"]
mod performance_tests;

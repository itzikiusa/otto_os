//! Warm, mutable lookup indexes and the publication boundary shared by API
//! mutations and external scans. No source files are owned by these caches.
use crate::{
    resolve::ResolveIndex,
    store::{NoteRow, Store},
    types::DirEntry,
};
use otto_core::{Error, Result};
use sqlx::Row;
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, RwLock,
};

#[derive(Clone)]
pub(crate) struct IndexRecord {
    pub path: String,
    pub title: Option<String>,
    pub kind: String,
    pub okf_type: Option<String>,
    pub reserved: bool,
    pub aliases: Vec<String>,
}
impl IndexRecord {
    pub fn note(row: &NoteRow) -> Self {
        Self {
            path: row.path.clone(),
            title: Some(row.title.clone()),
            kind: "note".into(),
            okf_type: row.okf_type.clone(),
            reserved: row.reserved,
            aliases: serde_json::from_str(&row.aliases_json).unwrap_or_default(),
        }
    }
    pub fn file(path: String) -> Self {
        Self {
            path,
            title: None,
            kind: "file".into(),
            okf_type: None,
            reserved: false,
            aliases: vec![],
        }
    }
    fn entry(&self) -> DirEntry {
        DirEntry {
            path: self.path.clone(),
            name: self.path.rsplit('/').next().unwrap_or(&self.path).into(),
            title: self.title.clone(),
            kind: self.kind.clone(),
            okf_type: self.okf_type.clone(),
            reserved: self.reserved,
            children: 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct VaultIndexes {
    pub records: BTreeMap<String, IndexRecord>,
    pub resolver: ResolveIndex,
    directories: HashMap<String, BTreeMap<String, DirEntry>>,
}
impl VaultIndexes {
    pub fn upsert(&mut self, record: IndexRecord) {
        let added = !self.records.contains_key(&record.path);
        let parent = record.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        self.directories
            .entry(parent.into())
            .or_default()
            .insert(record.path.clone(), record.entry());
        if added {
            self.resolver.insert(record.path.clone());
            for (i, _) in record.path.match_indices('/') {
                let path = &record.path[..i];
                let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
                let entry = self
                    .directories
                    .entry(parent.into())
                    .or_default()
                    .entry(path.into())
                    .or_insert_with(|| DirEntry {
                        path: path.into(),
                        name: name.into(),
                        kind: "dir".into(),
                        children: 0,
                        title: None,
                        okf_type: None,
                        reserved: false,
                    });
                entry.children += 1;
            }
        }
        self.records.insert(record.path.clone(), record);
    }
    pub fn remove(&mut self, path: &str) {
        if self.records.remove(path).is_none() {
            return;
        }
        self.resolver.remove(path);
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        if let Some(entries) = self.directories.get_mut(parent) {
            entries.remove(path);
        }
        for (i, _) in path.match_indices('/').rev() {
            let prefix = &path[..i];
            let parent = prefix.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            if let Some(entries) = self.directories.get_mut(parent) {
                if let Some(entry) = entries.get_mut(prefix) {
                    entry.children -= 1;
                    if entry.children == 0 {
                        entries.remove(prefix);
                        self.directories.remove(prefix);
                    }
                }
            }
        }
    }
    pub fn dir(&self, path: &str) -> Vec<DirEntry> {
        let mut entries: Vec<_> = self
            .directories
            .get(path)
            .into_iter()
            .flat_map(|m| m.values().cloned())
            .collect();
        entries.sort_by_key(|e| (e.kind != "dir", !e.reserved, e.name.to_lowercase()));
        entries
    }
}

#[derive(Default)]
struct ScanFence {
    active: bool,
    paths: HashMap<String, u64>,
}
#[derive(Default)]
pub(crate) struct IndexState {
    pub publication: tokio::sync::Mutex<()>,
    pub cache: RwLock<Option<VaultIndexes>>,
    epoch: AtomicU64,
    retired: AtomicBool,
    pub links_dirty: AtomicBool,
    fence: Mutex<ScanFence>,
    #[cfg(test)]
    pub(crate) hydration_pause: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
    #[cfg(test)]
    pub(crate) publication_pause: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
}
impl IndexState {
    /// The caller must not already hold publication: hydration owns that gate
    /// through its consistent DB snapshot and cache publication.
    pub async fn ensure(&self, store: &Store, vault: i64) -> Result<()> {
        self.check_active()?;
        if self.cache.read().unwrap().is_some() {
            return Ok(());
        }
        let _guard = self.publication.lock().await;
        self.check_active()?;
        if self.cache.read().unwrap().is_some() {
            return Ok(());
        }
        store.get_vault(vault).await?;
        #[cfg(test)]
        store.all_reads.fetch_add(1, Ordering::Relaxed);
        let mut tx = store.pool().begin().await.map_err(db_error)?;
        let notes = sqlx::query("SELECT path,title,okf_type,reserved,aliases_json FROM vault_notes WHERE vault_id=? ORDER BY path").bind(vault).fetch_all(&mut *tx).await.map_err(db_error)?;
        let files = sqlx::query_scalar::<_, String>(
            "SELECT path FROM vault_files WHERE vault_id=? ORDER BY path",
        )
        .bind(vault)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        #[cfg(test)]
        {
            let pause = self.hydration_pause.lock().unwrap().take();
            if let Some((started, resume)) = pause {
                let _ = started.send(());
                let _ = resume.await;
            }
        }

        let records: Vec<_> = notes
            .into_iter()
            .map(|r| IndexRecord {
                path: r.get("path"),
                title: Some(r.get("title")),
                kind: "note".into(),
                okf_type: r.get("okf_type"),
                reserved: r.get::<i64, _>("reserved") != 0,
                aliases: serde_json::from_str(&r.get::<String, _>("aliases_json"))
                    .unwrap_or_default(),
            })
            .chain(files.into_iter().map(IndexRecord::file))
            .collect();
        let cache = tokio::task::spawn_blocking(move || {
            let mut index = VaultIndexes::default();
            for record in records {
                index.upsert(record);
            }
            index
        })
        .await
        .map_err(|e| Error::Internal(format!("Vault index hydration: {e}")))?;
        // A cold rebuild also repairs an interrupted previous process scan.
        self.links_dirty.store(true, Ordering::Relaxed);
        *self.cache.write().unwrap() = Some(cache);
        Ok(())
    }
    pub fn start_scan(&self) -> u64 {
        let mut fence = self.fence.lock().unwrap();
        fence.active = true;
        fence.paths.clear();
        self.epoch.load(Ordering::Relaxed)
    }
    pub fn finish_scan(&self) {
        let mut fence = self.fence.lock().unwrap();
        fence.active = false;
        fence.paths.clear();
    }
    /// Prefixes match directory boundaries. Call before every structural move
    /// and include rewritten sources outside the moved directory.
    pub fn mutated(&self, paths: &[&str]) {
        let epoch = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let mut fence = self.fence.lock().unwrap();
        if fence.active {
            for path in paths {
                fence.paths.insert((*path).into(), epoch);
            }
        }
    }
    pub fn changed_since(&self, path: &str, epoch: u64) -> bool {
        self.fence
            .lock()
            .unwrap()
            .paths
            .iter()
            .any(|(prefix, version)| {
                *version > epoch
                    && (path == prefix
                        || path
                            .strip_prefix(prefix)
                            .is_some_and(|s| s.starts_with('/')))
            })
    }
    pub fn check_active(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            return Err(Error::Conflict("Vault was unregistered".into()));
        }
        Ok(())
    }
    /// Caller owns publication; existing scan/read handles cannot republish.
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.invalidate();
    }
    pub fn invalidate(&self) {
        *self.cache.write().unwrap() = None;
    }
}
fn db_error(e: sqlx::Error) -> Error {
    Error::Internal(format!("Vault index: {e}"))
}

//! File-backed recovery records. Hidden directories are deliberately excluded
//! from the derived index. No retention cleanup silently removes user history.
//!
//! **Revision path index (SD-15, listing half).** History for one note used to
//! read `meta.json` of every revision, newest first, until 200 matched — up to
//! 50k file reads per open in a busy vault. `.otto-history/.path-index.jsonl`
//! maps revision id → path (append-only, one JSON line per revision) and an
//! in-process copy is kept per vault root. It is purely a CACHE: the directory
//! listing stays authoritative, any revision the index doesn't know yet (older
//! history, a crash between the revision write and the append, another
//! process) has its meta read once and is appended. Nothing is ever deleted —
//! coalescing and retention are a separate, approval-gated decision.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use otto_core::{Error, Result};
use rustix::fs::{AtFlags, RenameFlags};
use sha2::{Digest, Sha256};

use crate::{types::*, VaultEngine};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(Error::Invalid("invalid recovery entry id".into()));
    }
    Ok(())
}

const PATH_INDEX: &str = ".path-index.jsonl";

/// Vault root → (revision id → note path). See the module docs.
type PathIndex = Arc<HashMap<String, String>>;

fn path_index_cache() -> &'static Mutex<HashMap<String, PathIndex>> {
    static CACHE: OnceLock<Mutex<HashMap<String, PathIndex>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `meta.json` reads made to (re)build the path index — a regression counter.
#[cfg(test)]
pub(crate) static INDEX_META_READS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(serde::Serialize, serde::Deserialize)]
struct PathIndexLine {
    id: String,
    path: String,
}

impl VaultEngine {
    /// Blocking read of one hidden recovery file through the symlink-refusing
    /// directory capability (for the index builder on the blocking pool).
    fn recovery_read_sync(root: &str, rel: &str) -> Result<Option<Vec<u8>>> {
        let (parent, name) = Self::text_parent(root, rel)?;
        let fd = match rustix::fs::openat(
            &parent,
            name.as_str(),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(Error::Internal(format!("open recovery record: {e}"))),
        };
        let mut bytes = Vec::new();
        std::fs::File::from(fd)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Internal(format!("read recovery record: {e}")))?;
        Ok(Some(bytes))
    }

    /// Append learned `(id, path)` pairs to the on-disk index (best-effort: a
    /// failed append only means they are re-learned next time).
    fn append_path_index(root: &str, lines: &[PathIndexLine]) {
        if lines.is_empty() {
            return;
        }
        let Ok((parent, name)) = Self::text_parent(root, &format!(".otto-history/{PATH_INDEX}"))
        else {
            return;
        };
        let Ok(fd) = rustix::fs::openat(
            &parent,
            name.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::APPEND
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        ) else {
            return;
        };
        let mut buf = Vec::with_capacity(lines.len() * 64);
        for line in lines {
            if serde_json::to_writer(&mut buf, line).is_ok() {
                buf.push(b'\n');
            }
        }
        // One write: O_APPEND keeps concurrent appenders' lines whole.
        let _ = std::fs::File::from(fd).write_all(&buf);
    }

    /// The id → path map for every revision in `names` (blocking). Loads the
    /// in-process copy, else the on-disk index, then reads `meta.json` only for
    /// revisions neither knows.
    fn revision_paths(root: &str, names: &[String]) -> Result<PathIndex> {
        let cached = path_index_cache()
            .lock()
            .ok()
            .and_then(|m| m.get(root).cloned());
        if let Some(map) = &cached {
            if names.iter().all(|n| map.contains_key(n)) {
                return Ok(map.clone()); // warm: an Arc bump, no copy
            }
        }
        let mut map: HashMap<String, String> = match cached {
            Some(map) => Arc::unwrap_or_clone(map),
            None => {
                let mut map = HashMap::new();
                if let Some(bytes) =
                    Self::recovery_read_sync(root, &format!(".otto-history/{PATH_INDEX}"))?
                {
                    for line in bytes.split(|b| *b == b'\n') {
                        if let Ok(entry) = serde_json::from_slice::<PathIndexLine>(line) {
                            map.insert(entry.id, entry.path);
                        }
                    }
                }
                map
            }
        };
        let mut learned = Vec::new();
        for name in names {
            if map.contains_key(name) {
                continue;
            }
            #[cfg(test)]
            INDEX_META_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // No meta yet = a revision mid-write: skip it, learn it next time.
            let Some(bytes) =
                Self::recovery_read_sync(root, &format!(".otto-history/{name}/meta.json"))?
            else {
                continue;
            };
            let Ok(revision) = serde_json::from_slice::<VaultRevision>(&bytes) else {
                continue;
            };
            map.insert(name.clone(), revision.path.clone());
            learned.push(PathIndexLine {
                id: name.clone(),
                path: revision.path,
            });
        }
        Self::append_path_index(root, &learned);
        let map = Arc::new(map);
        if let Ok(mut m) = path_index_cache().lock() {
            m.insert(root.to_string(), map.clone());
        }
        Ok(map)
    }

    /// Newest-first revision ids for `path` (all paths when `None`) older than
    /// `before`, at most `limit` — names + index only, off the runtime.
    async fn revision_ids(
        root: &str,
        path: Option<&str>,
        before: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let (root, path, before) = (
            root.to_string(),
            path.map(str::to_string),
            before.map(str::to_string),
        );
        tokio::task::spawn_blocking(move || {
            let names = Self::recovery_names(&root, ".otto-history")?;
            let paths = Self::revision_paths(&root, &names)?;
            Ok(names
                .into_iter()
                .filter(|name| {
                    before
                        .as_deref()
                        .is_none_or(|cursor| name.as_str() < cursor)
                })
                .filter(|name| match (&path, paths.get(name)) {
                    (None, Some(_)) => true,
                    (Some(want), Some(have)) => want == have,
                    (_, None) => false,
                })
                .take(limit)
                .collect())
        })
        .await
        .map_err(|e| Error::Internal(format!("list revisions: {e}")))?
    }

    async fn recovery_write(root: &str, path: &str, bytes: &[u8]) -> Result<()> {
        let (parent, name) = Self::text_parent(root, path)?;
        // Recovery copies can contain private notes; only the owner may enter
        // per-revision directories and the trash manifest directory.
        rustix::fs::fchmod(&parent, rustix::fs::Mode::RWXU)
            .map_err(|e| Error::Internal(format!("protect recovery directory: {e}")))?;
        Self::atomic_replace_at(&parent, &name, bytes).await
    }

    async fn recovery_read(root: &str, path: &str) -> Result<Vec<u8>> {
        let (parent, name) = Self::text_parent(root, path)?;
        Self::text_file_bytes(&parent, &name)
            .await?
            .ok_or_else(|| Error::NotFound("recovery entry no longer exists".into()))
    }

    /// Directory capabilities reject symlinks on every hidden storage hop too.
    fn recovery_names(root: &str, dir: &str) -> Result<Vec<String>> {
        if !Path::new(root).join(dir).exists() {
            return Ok(Vec::new());
        }
        let _capability = Self::text_parent(root, &format!("{dir}/entry"))?;
        let mut names = std::fs::read_dir(Path::new(root).join(dir))
            .map_err(|e| Error::Internal(format!("read recovery directory: {e}")))?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| valid_id(name.trim_end_matches(".json")).is_ok())
            .collect::<Vec<_>>();
        names.sort();
        names.reverse();
        Ok(names)
    }

    /// [`Self::recovery_names`] off the async runtime: the history directory
    /// grows by one entry per save and is listed with blocking `read_dir`.
    async fn recovery_names_off_runtime(root: &str, dir: &str) -> Result<Vec<String>> {
        let (root, dir) = (root.to_string(), dir.to_string());
        tokio::task::spawn_blocking(move || Self::recovery_names(&root, &dir))
            .await
            .map_err(|e| Error::Internal(format!("list recovery directory: {e}")))?
    }

    pub(crate) async fn prepare_revision(
        root: &str,
        path: &str,
        before: Option<&[u8]>,
        after: &[u8],
        reason: &str,
    ) -> Result<Option<VaultRevision>> {
        if before == Some(after) {
            return Ok(None);
        }
        let revision = VaultRevision {
            id: otto_core::new_id().to_string(),
            path: path.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            before_hash: before.map(hash),
            after_hash: hash(after),
            reason: reason.into(),
            committed: false,
        };
        let dir = format!(".otto-history/{}", revision.id);
        if let Some(bytes) = before {
            Self::recovery_write(root, &format!("{dir}/before"), bytes).await?;
        }
        Self::recovery_write(root, &format!("{dir}/after"), after).await?;
        Self::recovery_write(
            root,
            &format!("{dir}/meta.json"),
            &serde_json::to_vec(&revision).unwrap(),
        )
        .await?;
        Ok(Some(revision))
    }

    pub(crate) async fn commit_revision(root: &str, revision: Option<VaultRevision>) -> Result<()> {
        if let Some(mut revision) = revision {
            revision.committed = true;
            Self::recovery_write(
                root,
                &format!(".otto-history/{}/meta.json", revision.id),
                &serde_json::to_vec(&revision).unwrap(),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn revisions(
        &self,
        ws: &str,
        id: i64,
        path: Option<&str>,
    ) -> Result<Vec<VaultRevision>> {
        self.revisions_page(ws, id, path, None).await
    }

    pub async fn revisions_page(
        &self,
        ws: &str,
        id: i64,
        path: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<VaultRevision>> {
        if let Some(cursor) = before {
            valid_id(cursor)?;
        }
        let v = self.get_scoped(ws, id).await?;
        if let Some(path) = path {
            Self::check_rel(path)?;
        }
        let mut out = Vec::new();
        // The index narrows the walk to this path's newest 200 ids; only their
        // metas are read (SD-15 listing half).
        for name in Self::revision_ids(&v.root_path, path, before, 200).await? {
            let bytes =
                match Self::recovery_read(&v.root_path, &format!(".otto-history/{name}/meta.json"))
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(Error::NotFound(_)) => continue,
                    Err(e) => return Err(e),
                };
            let revision: VaultRevision = serde_json::from_slice(&bytes)
                .map_err(|e| Error::Invalid(format!("invalid revision record: {e}")))?;
            if path.is_none_or(|p| p == revision.path) {
                out.push(revision);
            }
            if out.len() == 200 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn revision(&self, ws: &str, id: i64, entry: &str) -> Result<VaultRevisionDetail> {
        valid_id(entry)?;
        let v = self.get_scoped(ws, id).await?;
        let dir = format!(".otto-history/{entry}");
        let revision: VaultRevision = serde_json::from_slice(
            &Self::recovery_read(&v.root_path, &format!("{dir}/meta.json")).await?,
        )
        .map_err(|e| Error::Invalid(format!("invalid revision record: {e}")))?;
        let before = if revision.before_hash.is_some() {
            Some(
                String::from_utf8(
                    Self::recovery_read(&v.root_path, &format!("{dir}/before")).await?,
                )
                .map_err(|_| Error::Invalid("revision contains non-UTF-8 content".into()))?,
            )
        } else {
            None
        };
        let after =
            String::from_utf8(Self::recovery_read(&v.root_path, &format!("{dir}/after")).await?)
                .map_err(|_| Error::Invalid("revision contains non-UTF-8 content".into()))?;
        if before.as_deref().map(|raw| hash(raw.as_bytes())) != revision.before_hash
            || hash(after.as_bytes()) != revision.after_hash
        {
            return Err(Error::Conflict(
                "revision content failed its checksum; restore was refused".into(),
            ));
        }
        Ok(VaultRevisionDetail {
            revision,
            before,
            after,
        })
    }

    pub async fn restore_revision(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        entry: &str,
        version: &str,
        if_hash: &str,
    ) -> Result<()> {
        let detail = self.revision(ws, id, entry).await?;
        let content = match version {
            "before" => detail.before.as_deref().ok_or_else(|| {
                Error::Invalid("this revision created the file; no prior content exists".into())
            })?,
            "after" => &detail.after,
            _ => return Err(Error::Invalid("version must be before or after".into())),
        };
        // Restores go through normal guarded writes and create another revision.
        if detail.revision.path.to_ascii_lowercase().ends_with(".md") {
            self.write_note(ws, id, &detail.revision.path, content, Some(if_hash))
                .await?;
        } else {
            self.write_text_file(ws, id, &detail.revision.path, content, Some(if_hash))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn record_trash(root: &str, entry: &VaultTrashEntry) -> Result<()> {
        Self::recovery_write(
            root,
            &format!(".trash/.otto-index/{}.json", entry.id),
            &serde_json::to_vec(entry).unwrap(),
        )
        .await
    }

    pub async fn trash_entries(&self, ws: &str, id: i64) -> Result<Vec<VaultTrashEntry>> {
        let v = self.get_scoped(ws, id).await?;
        let mut out = Vec::new();
        for name in Self::recovery_names_off_runtime(&v.root_path, ".trash/.otto-index").await? {
            let entry: VaultTrashEntry = serde_json::from_slice(
                &Self::recovery_read(&v.root_path, &format!(".trash/.otto-index/{name}")).await?,
            )
            .map_err(|e| Error::Invalid(format!("invalid trash record: {e}")))?;
            Self::check_rel(&entry.stored_path)?;
            if Path::new(&v.root_path)
                .join(".trash")
                .join(&entry.stored_path)
                .symlink_metadata()
                .is_ok()
            {
                out.push(entry);
            }
        }
        // Older versions kept files without manifests. Surface those too,
        // without mutating disk during a read. Folder records already cover
        // all their descendants, so do not list those files a second time.
        let trash_root = Path::new(&v.root_path).join(".trash");
        if trash_root.exists() {
            let _guard = Self::text_parent(&v.root_path, ".trash/entry")?;
            let walk = crate::scan::walk(&trash_root)
                .map_err(|e| Error::Internal(format!("list legacy trash: {e}")))?;
            for item in walk.notes.into_iter().chain(walk.files) {
                if out.iter().any(|e| {
                    item.rel == e.stored_path
                        || item.rel.starts_with(&format!("{}/", e.stored_path))
                }) {
                    continue;
                }
                out.push(VaultTrashEntry {
                    id: format!("legacy-{}", hash(item.rel.as_bytes())),
                    original_path: item.rel.clone(),
                    stored_path: item.rel,
                    deleted_at: chrono::DateTime::from_timestamp_nanos(item.mtime_ns).to_rfc3339(),
                    kind: "file".into(),
                });
            }
        }
        out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
        Ok(out)
    }

    pub async fn restore_trash(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        entry: &str,
        destination: Option<&str>,
    ) -> Result<String> {
        valid_id(entry)?;
        let v = self.get_scoped(ws, id).await?;
        let lock = self.write_lock(id, "");
        let _guard = lock.lock().await;
        let manifest = format!(".trash/.otto-index/{entry}.json");
        let legacy = entry.starts_with("legacy-");
        let item: VaultTrashEntry = if legacy {
            self.trash_entries(ws, id)
                .await?
                .into_iter()
                .find(|item| item.id == entry)
                .ok_or_else(|| Error::NotFound("trash entry no longer exists".into()))?
        } else {
            serde_json::from_slice(&Self::recovery_read(&v.root_path, &manifest).await?)
                .map_err(|e| Error::Invalid(format!("invalid trash record: {e}")))?
        };
        Self::check_rel(&item.stored_path)?;
        let dest = Self::check_rel(destination.unwrap_or(&item.original_path))?;
        let state = self.index_state(id);
        state.ensure(self.store(), id).await?;
        let publication = state.publication.lock().await;
        state.mutated(&[&dest]);
        let (source_parent, source_name) =
            Self::text_parent(&v.root_path, &format!(".trash/{}", item.stored_path))?;
        let (target_parent, target_name) = Self::text_parent(&v.root_path, &dest)?;
        rustix::fs::renameat_with(
            &source_parent,
            source_name.as_str(),
            &target_parent,
            target_name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| match e {
            rustix::io::Errno::EXIST => {
                Error::Conflict(format!("target exists: {dest}; choose another destination"))
            }
            rustix::io::Errno::NOENT => Error::NotFound("trash entry no longer exists".into()),
            _ => Error::Internal(format!("restore trash: {e}")),
        })?;
        if !legacy {
            let (parent, name) = Self::text_parent(&v.root_path, &manifest)?;
            rustix::fs::unlinkat(parent, name, AtFlags::empty())
                .map_err(|e| Error::Internal(format!("remove restored trash marker: {e}")))?;
        }
        drop(publication);
        self.rescan_after_mutation(id).await;
        Ok(dest)
    }
}

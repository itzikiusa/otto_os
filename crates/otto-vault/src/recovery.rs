//! File-backed recovery records. Hidden directories are deliberately excluded
//! from the derived index. No retention cleanup silently removes user history.
//!
//! **Not committed with the vault.** Vaults are often git repositories (git
//! sync, shared docs). `.otto-history/` and `.trash/` hold private pre-edit
//! copies and deleted notes, so each gets a `.gitignore` of `*` when Otto
//! first writes into it ([`VaultEngine::ignore_recovery_dir`]): history and
//! trash never ride along into a commit or a push.
//!
//! **Revision path index (SD-15, listing half).** History for one note used to
//! read `meta.json` of every revision, newest first, until 200 matched — up to
//! 50k file reads per open in a busy vault. `.otto-history/.path-index.jsonl`
//! maps revision id → path (append-only, one JSON line per revision) and an
//! in-process copy is kept per vault root. It is purely a CACHE: the directory
//! listing stays authoritative, any revision the index doesn't know yet (older
//! history, a crash between the revision write and the append, another
//! process) has its meta read once and is appended. Nothing is ever deleted;
//! retention stays a separate, approval-gated decision.
//!
//! **Storage (F1).** Revision bodies are deduped (`before` is a content-
//! addressed blob, hard-linked from the previous revision's `after`) and user
//! autosaves of one note within [`COALESCE_SECS`] extend ONE revision instead
//! of minting a directory per typing pause. See [`VaultEngine::prepare_revision`].
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use otto_core::{Error, Result};
use rustix::fs::{AtFlags, RenameFlags};
use sha2::{Digest, Sha256};

use crate::{types::*, VaultEngine};

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
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
/// Content-addressed revision bodies (F1). Hidden + not a valid revision id,
/// so neither the index nor the revision listing ever sees it.
const BLOBS: &str = ".otto-history/.blobs";
/// User autosaves of one path inside this window share one revision (F1),
/// mirroring the canvas `USER_SNAPSHOT_EVERY_SECS` cadence.
pub(crate) const COALESCE_SECS: u64 = 300;
/// Bound on the in-process open-revision map (pruned of expired entries).
const OPEN_REVISIONS_CAP: usize = 4096;

/// A revision snapshotted by [`VaultEngine::prepare_revision`], committed by
/// [`VaultEngine::commit_revision`] once the write landed.
pub(crate) struct PendingRevision {
    revision: VaultRevision,
    coalesce: bool,
    opened: std::time::Instant,
    /// A coalesced revision's previous `after` sha, retired at commit.
    superseded: Option<String>,
}

/// The newest committed revision per (vault root, path) — the coalescing
/// candidate, and the hard-link source for the next revision's `before`.
#[derive(Clone)]
struct OpenRevision {
    revision: VaultRevision,
    opened: std::time::Instant,
    coalescable: bool,
}

fn open_revisions() -> &'static Mutex<HashMap<(String, String), OpenRevision>> {
    static OPEN: OnceLock<Mutex<HashMap<(String, String), OpenRevision>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Internal(format!("recovery write task: {e}")))?
}

/// Revision-file writes / coalesced saves — regression counters (F1).
#[cfg(test)]
pub(crate) static REVISION_FILE_WRITES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static REVISIONS_COALESCED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

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

/// `(vault root, recovery dir)` pairs whose `.gitignore` was written or found
/// — saves the `openat` dance per save. NOT trusted alone: see
/// [`known_ignored`].
fn ignored_dirs() -> &'static Mutex<std::collections::HashSet<(String, &'static str)>> {
    static DONE: OnceLock<Mutex<std::collections::HashSet<(String, &'static str)>>> =
        OnceLock::new();
    DONE.get_or_init(Default::default)
}

/// Ignores everything in its directory (itself included).
const RECOVERY_GITIGNORE: &[u8] =
    b"# Otto recovery data (private note history / deleted notes): never commit.\n*\n";

/// Is `<root>/<dir>/.gitignore` known present? The process-wide cache only
/// short-cuts the create; one `lstat` confirms the file is still there, so a
/// user deleting `.otto-history` (or just its `.gitignore`) gets it recreated
/// on the next write instead of after a daemon restart (S7-07).
fn known_ignored(key: &(String, &'static str)) -> bool {
    if !ignored_dirs().lock().is_ok_and(|d| d.contains(key)) {
        return false;
    }
    let path = std::path::Path::new(&key.0).join(key.1).join(".gitignore");
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return true;
    }
    if let Ok(mut d) = ignored_dirs().lock() {
        d.remove(key);
    }
    false
}

impl VaultEngine {
    /// Make sure `<root>/<dir>/.gitignore` exists (blocking), created through
    /// the symlink-refusing directory capability and never overwriting a
    /// file the user put there. Best-effort: a failure only means the dir
    /// isn't ignored yet — it is retried on the next write.
    pub(crate) fn ignore_recovery_dir(root: &str, dir: &'static str) {
        let key = (root.to_string(), dir);
        if known_ignored(&key) {
            return;
        }
        let Ok((parent, name)) = Self::text_parent(root, &format!("{dir}/.gitignore")) else {
            return;
        };
        let done = match rustix::fs::openat(
            &parent,
            name.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::RGRP,
        ) {
            Ok(fd) => std::fs::File::from(fd)
                .write_all(RECOVERY_GITIGNORE)
                .is_ok(),
            Err(rustix::io::Errno::EXIST) => true,
            Err(_) => false,
        };
        if done {
            if let Ok(mut d) = ignored_dirs().lock() {
                d.insert(key);
            }
        }
    }

    /// [`Self::ignore_recovery_dir`] off the runtime, skipped without a
    /// task hop once the dir is known ignored.
    async fn ignore_recovery_dir_async(root: &str, dir: &'static str) {
        let key = (root.to_string(), dir);
        if known_ignored(&key) {
            return;
        }
        let _ = blocking(move || {
            Self::ignore_recovery_dir(&key.0, dir);
            Ok(())
        })
        .await;
    }

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

    /// Snapshot `before`/`after` for a write that is about to happen (F1).
    ///
    /// Layout: `before` lives content-addressed in `.otto-history/.blobs/<sha>`
    /// (hard-linked from the previous revision's `after` when that is the same
    /// content, so the copy costs no bytes); `after` lives in the revision dir
    /// as `a-<sha>`. With `coalesce`, a user autosave that continues the open
    /// revision of the same path (same reason, its `after` == this `before`,
    /// opened < [`COALESCE_SECS`] ago) REUSES that revision: only a new `a-<sha>`
    /// is written, the meta is repointed at commit and the superseded `a-<sha>`
    /// is unlinked. Nothing else is ever deleted. `meta.json` is written once,
    /// by [`Self::commit_revision`], after the note itself was replaced; a crash
    /// before that leaves an invisible, harmless orphan (and for a coalesced
    /// revision the old meta still points at the old, still-present file).
    /// Revision files use a plain `fsync` — the note keeps F_FULLFSYNC.
    pub(crate) async fn prepare_revision(
        root: &str,
        path: &str,
        before: Option<&[u8]>,
        after: &[u8],
        reason: &str,
        coalesce: bool,
    ) -> Result<Option<PendingRevision>> {
        if before == Some(after) {
            return Ok(None);
        }
        Self::ignore_recovery_dir_async(root, ".otto-history").await;
        let before_hash = before.map(hash);
        let after_hash = hash(after);
        let key = (root.to_string(), path.to_string());
        let last = open_revisions()
            .lock()
            .ok()
            .and_then(|m| m.get(&key).cloned());
        let reuse = last.as_ref().filter(|open| {
            coalesce
                && open.coalescable
                && open.revision.reason == reason
                && open.opened.elapsed().as_secs() < COALESCE_SECS
                && before_hash.as_deref() == Some(open.revision.after_hash.as_str())
        });
        let (root_s, after_v) = (root.to_string(), after.to_vec());
        if let Some(open) = reuse {
            let mut revision = open.revision.clone();
            let superseded = std::mem::replace(&mut revision.after_hash, after_hash.clone());
            revision.committed = false;
            let dir = format!(".otto-history/{}", revision.id);
            let name = format!("{dir}/a-{after_hash}");
            blocking(move || Self::write_light(&root_s, &name, &after_v)).await?;
            #[cfg(test)]
            REVISIONS_COALESCED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(PendingRevision {
                revision,
                coalesce,
                opened: open.opened,
                superseded: Some(superseded),
            }));
        }
        let revision = VaultRevision {
            id: otto_core::new_id().to_string(),
            path: path.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            before_hash: before_hash.clone(),
            after_hash: after_hash.clone(),
            reason: reason.into(),
            committed: false,
        };
        let dir = format!(".otto-history/{}", revision.id);
        let before_v = before.map(<[u8]>::to_vec);
        // The previous revision of this path already holds `before` as its
        // `a-<sha>`: link it into the blob store instead of writing it again.
        let link_from = last
            .filter(|open| before_hash.as_deref() == Some(open.revision.after_hash.as_str()))
            .map(|open| {
                format!(
                    ".otto-history/{}/a-{}",
                    open.revision.id, open.revision.after_hash
                )
            });
        blocking(move || {
            if let (Some(bytes), Some(sha)) = (before_v.as_deref(), before_hash.as_deref()) {
                Self::write_blob(&root_s, sha, bytes, link_from.as_deref())?;
            }
            Self::write_light(&root_s, &format!("{dir}/a-{after_hash}"), &after_v)
        })
        .await?;
        Ok(Some(PendingRevision {
            revision,
            coalesce,
            opened: std::time::Instant::now(),
            superseded: None,
        }))
    }

    /// Write `meta.json` (once, `committed:true`) after the note was replaced,
    /// then retire a coalesced revision's superseded `a-<sha>`.
    pub(crate) async fn commit_revision(
        root: &str,
        pending: Option<PendingRevision>,
    ) -> Result<()> {
        let Some(mut pending) = pending else {
            return Ok(());
        };
        pending.revision.committed = true;
        let id = pending.revision.id.clone();
        let meta = serde_json::to_vec(&pending.revision).unwrap();
        let superseded = pending
            .superseded
            .take()
            .filter(|sha| *sha != pending.revision.after_hash);
        let root_s = root.to_string();
        blocking(move || {
            Self::write_light(&root_s, &format!(".otto-history/{id}/meta.json"), &meta)?;
            if let Some(sha) = superseded {
                // Private to this revision dir (a blob-store link is its own
                // name), so dropping it never touches another revision.
                if let Ok((parent, name)) =
                    Self::text_parent(&root_s, &format!(".otto-history/{id}/a-{sha}"))
                {
                    let _ = rustix::fs::unlinkat(&parent, name.as_str(), AtFlags::empty());
                }
            }
            Ok(())
        })
        .await?;
        if let Ok(mut m) = open_revisions().lock() {
            if m.len() > OPEN_REVISIONS_CAP {
                m.retain(|_, open| open.opened.elapsed().as_secs() < COALESCE_SECS);
            }
            m.insert(
                (root.to_string(), pending.revision.path.clone()),
                OpenRevision {
                    revision: pending.revision,
                    opened: pending.opened,
                    coalescable: pending.coalesce,
                },
            );
        }
        Ok(())
    }

    /// Blocking create-or-replace of a hidden revision file: owner-only
    /// parent, temp + rename, plain `fsync` (not F_FULLFSYNC).
    fn write_light(root: &str, rel: &str, bytes: &[u8]) -> Result<()> {
        let (parent, name) = Self::text_parent(root, rel)?;
        rustix::fs::fchmod(&parent, rustix::fs::Mode::RWXU)
            .map_err(|e| Error::Internal(format!("protect recovery directory: {e}")))?;
        let temp = format!(".{name}.otto-tmp-{}", otto_core::new_id());
        let fd = rustix::fs::openat(
            &parent,
            temp.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(|e| Error::Internal(format!("create recovery temp: {e}")))?;
        let res = (|| {
            let mut file = std::fs::File::from(fd);
            file.write_all(bytes)
                .map_err(|e| Error::Internal(format!("write recovery record: {e}")))?;
            rustix::fs::fsync(&file)
                .map_err(|e| Error::Internal(format!("flush recovery record: {e}")))?;
            drop(file);
            rustix::fs::renameat(&parent, temp.as_str(), &parent, name.as_str())
                .map_err(|e| Error::Internal(format!("replace recovery record: {e}")))
        })();
        if res.is_err() {
            let _ = rustix::fs::unlinkat(&parent, temp.as_str(), AtFlags::empty());
        }
        #[cfg(test)]
        if res.is_ok() {
            REVISION_FILE_WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        res
    }

    /// Content-addressed blob: no-op when present, else a hard link from
    /// `link_from` (same content, verified by its name) or a fresh write.
    fn write_blob(root: &str, sha: &str, bytes: &[u8], link_from: Option<&str>) -> Result<()> {
        let rel = format!("{BLOBS}/{sha}");
        let (parent, name) = Self::text_parent(root, &rel)?;
        if rustix::fs::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
            return Ok(());
        }
        if let Some(src) = link_from {
            if let Ok((src_parent, src_name)) = Self::text_parent(root, src) {
                if rustix::fs::linkat(
                    &src_parent,
                    src_name.as_str(),
                    &parent,
                    name.as_str(),
                    AtFlags::empty(),
                )
                .is_ok()
                {
                    return Ok(());
                }
            }
        }
        Self::write_light(root, &rel, bytes)
    }

    /// One revision body: legacy in-dir copy first, then the F1 layout.
    async fn revision_body(root: &str, dir: &str, legacy: &str, sha: &str) -> Result<Vec<u8>> {
        for rel in [
            format!("{dir}/{legacy}"),
            format!("{dir}/a-{sha}"),
            format!("{BLOBS}/{sha}"),
        ] {
            match Self::recovery_read(root, &rel).await {
                Ok(bytes) => return Ok(bytes),
                Err(Error::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::NotFound("recovery entry no longer exists".into()))
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
            let sha = revision.before_hash.as_deref().unwrap_or_default();
            Some(
                String::from_utf8(Self::revision_body(&v.root_path, &dir, "before", sha).await?)
                    .map_err(|_| Error::Invalid("revision contains non-UTF-8 content".into()))?,
            )
        } else {
            None
        };
        let after = String::from_utf8(
            Self::revision_body(&v.root_path, &dir, "after", &revision.after_hash).await?,
        )
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
        Self::ignore_recovery_dir_async(root, ".trash").await;
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

#[cfg(test)]
mod gitignore_tests {
    use super::*;

    /// S7-07: the process-wide "already ignored" cache used to be trusted
    /// forever — deleting `.otto-history` left history files un-ignored until
    /// a restart. The cache is now confirmed with a stat.
    #[test]
    fn a_deleted_recovery_dir_gets_its_gitignore_back() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().canonicalize().unwrap();
        let root = root.to_string_lossy().into_owned();
        let gi = std::path::Path::new(&root).join(".otto-history/.gitignore");
        VaultEngine::ignore_recovery_dir(&root, ".otto-history");
        assert_eq!(std::fs::read(&gi).unwrap(), RECOVERY_GITIGNORE);
        std::fs::remove_dir_all(gi.parent().unwrap()).unwrap();
        VaultEngine::ignore_recovery_dir(&root, ".otto-history");
        assert_eq!(std::fs::read(&gi).unwrap(), RECOVERY_GITIGNORE, "recreated");
        // A user's own .gitignore is never overwritten.
        std::fs::write(&gi, b"custom\n").unwrap();
        VaultEngine::ignore_recovery_dir(&root, ".otto-history");
        assert_eq!(std::fs::read(&gi).unwrap(), b"custom\n");
    }
}

//! Content-addressed blob store: `<data>/design/blobs/<sha256>`.
//!
//! Every committed version (and every thumbnail) is one immutable blob named by
//! the lowercase hex SHA-256 of its bytes, so an unchanged save costs nothing
//! and identical content across artifacts is stored once. Writes go to a
//! sibling temp file and are `rename`d into place (atomic on one filesystem),
//! so a crash never leaves a truncated blob under a valid name. Nothing here
//! deletes a blob on its own — removal is only the prune's GC step (opt-in
//! admin route, or the scheduled retention job — see `retention`).
//!
//! GC fence: `put` skips an existing blob, so a save can "store" a blob the
//! GC already decided was unreferenced and is about to delete — the new
//! version would then point at a missing file. Every writer holds
//! [`BlobStore::reference_guard`] (shared) from its `put` until the DB row
//! that references the blob is committed; the GC takes
//! [`BlobStore::gc_guard`] (exclusive) and re-checks `blob_in_use` under it
//! before deleting. Process-wide per canonical blob root, like the
//! artifact `commit_lock`, so request-scoped services share it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use otto_core::{Error, Result};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A blob name is exactly 64 lowercase hex chars — anything else is refused
/// before it can become a path.
pub fn is_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, sha: &str) -> Result<PathBuf> {
        if !is_sha(sha) {
            return Err(Error::Invalid(format!("bad blob id {sha:?}")));
        }
        Ok(self.root.join(sha))
    }

    /// Store `bytes`, returning their sha256. Idempotent: an existing blob with
    /// the same name is left untouched (its content is identical by definition).
    pub async fn put(&self, bytes: &[u8]) -> Result<String> {
        self.put_hashed(bytes, sha256_hex(bytes)).await
    }

    /// [`Self::put`] with the sha256 the caller already computed (off the
    /// runtime, D10) — hashing a 4 MB blob twice per save was pure waste.
    /// `sha` MUST be `sha256_hex(bytes)`.
    pub(crate) async fn put_hashed(&self, bytes: &[u8], sha: String) -> Result<String> {
        debug_assert_eq!(sha, sha256_hex(bytes));
        let dest = self.path(&sha)?;
        if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            return Ok(sha);
        }
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| Error::Internal(format!("create blob dir: {e}")))?;
        let tmp = self
            .root
            .join(format!(".tmp-{}-{}", &sha[..16], otto_core::new_id()));
        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|e| Error::Internal(format!("write blob: {e}")))?;
        if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(Error::Internal(format!("publish blob: {e}")));
        }
        Ok(sha)
    }

    /// Read a blob. A missing blob is `NotFound` (a restored DB without its
    /// blob directory degrades to a clear error, never a panic).
    pub async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        let p = self.path(sha)?;
        tokio::fs::read(&p)
            .await
            .map_err(|_| Error::NotFound(format!("design blob {sha}")))
    }

    pub async fn exists(&self, sha: &str) -> bool {
        match self.path(sha) {
            Ok(p) => tokio::fs::try_exists(&p).await.unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Every blob file name (sha) under the store root — temp files and
    /// strays excluded. A missing root is empty. For the orphan sweep.
    pub async fn list(&self) -> Vec<String> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(rd) = std::fs::read_dir(&root) else {
                return Vec::new();
            };
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| is_sha(n))
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// Storage gauge: `(blob files, total bytes)` under the store root. A walk
    /// of one flat directory, off the runtime. A missing root is `(0, 0)`.
    pub async fn usage(&self) -> (u64, u64) {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(rd) = std::fs::read_dir(&root) else {
                return (0, 0);
            };
            let (mut n, mut bytes) = (0u64, 0u64);
            for e in rd.flatten() {
                if !is_sha(&e.file_name().to_string_lossy()) {
                    continue; // temp files
                }
                if let Ok(m) = e.metadata() {
                    n += 1;
                    bytes += m.len();
                }
            }
            (n, bytes)
        })
        .await
        .unwrap_or((0, 0))
    }

    /// The GC fence of this store's root (see the module docs).
    async fn fence(&self) -> Result<Arc<RwLock<()>>> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| Error::Internal(format!("create blob dir: {e}")))?;
        let root = tokio::fs::canonicalize(&self.root)
            .await
            .map_err(|e| Error::Internal(format!("blob root identity: {e}")))?;
        type Fences = HashMap<PathBuf, Weak<RwLock<()>>>;
        static FENCES: OnceLock<Mutex<Fences>> = OnceLock::new();
        let mut fences = FENCES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        fences.retain(|_, f| f.strong_count() > 0);
        if let Some(f) = fences.get(&root).and_then(Weak::upgrade) {
            return Ok(f);
        }
        let f = Arc::new(RwLock::new(()));
        fences.insert(root, Arc::downgrade(&f));
        Ok(f)
    }

    /// Hold (shared) from `put` until the DB row referencing the blob is
    /// committed, so the GC can't delete it in between. Never hold it across
    /// [`Self::gc_guard`] (the fence is fair: that would deadlock).
    pub async fn reference_guard(&self) -> Result<OwnedRwLockReadGuard<()>> {
        Ok(self.fence().await?.read_owned().await)
    }

    /// Hold (exclusive) around the GC's "still unreferenced?" re-check and
    /// [`Self::remove`]: no writer is mid-reference while it is held.
    pub async fn gc_guard(&self) -> Result<OwnedRwLockWriteGuard<()>> {
        Ok(self.fence().await?.write_owned().await)
    }

    /// Remove a blob — ONLY called by the GC (prune, hard delete, thumbnail
    /// replacement, orphan sweep) after it proved no version/thumbnail
    /// references the blob, under [`Self::gc_guard`].
    /// Returns whether a file existed.
    pub async fn remove(&self, sha: &str) -> Result<bool> {
        let p = self.path(sha)?;
        match tokio::fs::remove_file(&p).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::Internal(format!("remove blob: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_is_content_addressed_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let a = store.put(b"hello").await.unwrap();
        let b = store.put(b"hello").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a, sha256_hex(b"hello"));
        assert_eq!(store.get(&a).await.unwrap(), b"hello");
        assert!(store.exists(&a).await);
        // Exactly one blob file, no leftover temp files.
        let names: Vec<String> = std::fs::read_dir(dir.path().join("blobs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![a.clone()]);

        assert_eq!(store.usage().await, (1, 5));
        assert!(store.remove(&a).await.unwrap());
        assert!(!store.remove(&a).await.unwrap());
        assert_eq!(store.usage().await, (0, 0));
        assert!(matches!(store.get(&a).await, Err(Error::NotFound(_))));
    }

    /// The fence is shared by every store over the same (canonical) root:
    /// GC waits for an in-flight reference and vice versa.
    #[tokio::test]
    async fn gc_guard_waits_for_in_flight_references_across_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let a = BlobStore::new(dir.path().join("blobs"));
        let alias = dir.path().join("alias");
        std::fs::create_dir_all(dir.path().join("blobs")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("blobs"), &alias).unwrap();
        let b = BlobStore::new(&alias);
        let reference = a.reference_guard().await.unwrap();
        let gc = b.gc_guard();
        tokio::pin!(gc);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut gc)
                .await
                .is_err(),
            "GC must wait while a writer is between put and its DB reference"
        );
        drop(reference);
        let held = gc.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), a.reference_guard())
                .await
                .is_err(),
            "a writer waits while the GC re-checks and deletes"
        );
        drop(held);
        a.reference_guard().await.unwrap();
    }

    #[tokio::test]
    async fn refuses_non_sha_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        assert!(store.get("../etc/passwd").await.is_err());
        assert!(!store.exists("ABC").await);
        assert!(is_sha(&sha256_hex(b"")));
        assert!(!is_sha(&sha256_hex(b"").to_uppercase()));
    }
}

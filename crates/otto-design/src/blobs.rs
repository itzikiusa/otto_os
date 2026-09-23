//! Content-addressed blob store: `<data>/design/blobs/<sha256>`.
//!
//! Every committed version (and every thumbnail) is one immutable blob named by
//! the lowercase hex SHA-256 of its bytes, so an unchanged save costs nothing
//! and identical content across artifacts is stored once. Writes go to a
//! sibling temp file and are `rename`d into place (atomic on one filesystem),
//! so a crash never leaves a truncated blob under a valid name. Nothing here
//! deletes a blob on its own — removal is only the opt-in prune's GC step.

use std::path::{Path, PathBuf};

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
        let sha = sha256_hex(bytes);
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

    /// Remove a blob — ONLY called by the opt-in prune after it proved no
    /// version/thumbnail references the blob. Returns whether a file existed.
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

        assert!(store.remove(&a).await.unwrap());
        assert!(!store.remove(&a).await.unwrap());
        assert!(matches!(store.get(&a).await, Err(Error::NotFound(_))));
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

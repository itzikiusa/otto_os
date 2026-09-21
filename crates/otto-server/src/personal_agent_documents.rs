//! Fixed-path, versioned access to personal-agent memory. User context is stored
//! separately in SQLite and never shares the agent-maintained memory file.

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentDocument {
    pub content: String,
    pub version: String,
    pub exists: bool,
    pub path: Option<String>,
}

fn io_error(e: impl std::fmt::Display) -> Error {
    Error::Internal(format!("agent memory: {e}"))
}

/// The caller supplies a resolved agent directory, never a request path. Reject
/// symlinks in the fixed descendants so memory cannot escape that selected root.
fn memory_path(root: &Path) -> Result<PathBuf> {
    for path in [root.join("memory"), root.join("memory/notes.md")] {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::Forbidden(
                    "agent memory cannot follow a symlink".into(),
                ))
            }
            Ok(meta) if path.ends_with("notes.md") && !meta.is_file() => {
                return Err(Error::Invalid("agent memory must be a regular file".into()))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    Ok(root.join("memory/notes.md"))
}

fn memory_version(path: &Path, content: Option<&[u8]>) -> String {
    let mut hash = Sha256::new();
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update([0, u8::from(content.is_some())]);
    if let Some(bytes) = content { hash.update(bytes); }
    format!("{:x}", hash.finalize())
}

fn read_sync(root: &Path) -> Result<AgentDocument> {
    let path = memory_path(root)?;
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentDocument {
                content: String::new(),
                version: memory_version(&path, None),
                exists: false,
                path: Some(path.to_string_lossy().into_owned()),
            })
        }
        Err(e) => return Err(io_error(e)),
    };
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(Error::Invalid("agent memory must be a regular file".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Invalid("agent memory exceeds 1 MiB".into()));
    }
    let version = memory_version(&path, Some(&bytes));
    let content = String::from_utf8(bytes)
        .map_err(|_| Error::Invalid("agent memory must be UTF-8 text".into()))?;
    Ok(AgentDocument {
        content,
        version,
        exists: true,
        path: Some(path.to_string_lossy().into_owned()),
    })
}

/// Seed only a missing file. `create_new` prevents a concurrent editor's first
/// save from being replaced by the initial notes template.
pub async fn seed_memory(root: &Path, seed: &str) -> Result<()> {
    let path = memory_path(root)?;
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(io_error)?;
    memory_path(root)?;
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            file.write_all(seed.as_bytes()).await.map_err(io_error)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_error(e)),
    }
    Ok(())
}

pub async fn read_memory(root: &Path) -> Result<AgentDocument> {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || read_sync(&root))
        .await
        .map_err(io_error)?
}

pub async fn save_memory(root: &Path, version: &str, content: &str) -> Result<AgentDocument> {
    if content.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Invalid("agent memory exceeds 1 MiB".into()));
    }
    // Serialize UI saves (including agents sharing the same custom cwd). The
    // hash is checked again immediately before rename for external agent edits.
    static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let (root, version, content) = (root.to_owned(), version.to_owned(), content.to_owned());
    tokio::task::spawn_blocking(move || {
        // Keep the lock inside the blocking operation: cancellation of the HTTP
        // future must not unlock an in-flight filesystem replacement.
        let _guard = SAVE_LOCK.lock().map_err(io_error)?;
        let current = read_sync(&root)?;
        if current.version != version {
            return Err(Error::Conflict(
                "Memory changed since you opened it. Reload and reconcile your edits.".into(),
            ));
        }
        let path = memory_path(&root)?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(io_error)?;
        memory_path(&root)?;
        let mut staged =
            tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(io_error)?;
        staged.write_all(content.as_bytes()).map_err(io_error)?;
        staged.as_file().sync_all().map_err(io_error)?;
        if read_sync(&root)?.version != version {
            return Err(Error::Conflict(
                "Memory changed while saving. Reload and reconcile your edits.".into(),
            ));
        }
        staged.persist(&path).map_err(io_error)?;
        read_sync(&root)
    })
    .await
    .map_err(io_error)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reading_missing_memory_does_not_provision_it() {
        let dir = tempfile::tempdir().unwrap();
        let doc = read_memory(dir.path()).await.unwrap();
        assert!(!doc.exists);
        assert_eq!(doc.content, "");
        assert!(!dir.path().join("memory").exists());
    }

    #[tokio::test]
    async fn memory_save_preserves_existing_notes_and_rejects_stale_writes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("memory")).unwrap();
        let path = dir.path().join("memory/notes.md");
        std::fs::write(&path, "existing notes\n").unwrap();
        let original = read_memory(dir.path()).await.unwrap();
        assert_eq!(original.content, "existing notes\n");
        let saved = save_memory(dir.path(), &original.version, "edited notes\n")
            .await
            .unwrap();
        assert_eq!(saved.content, "edited notes\n");
        seed_memory(dir.path(), "fresh seed must not replace edits")
            .await
            .unwrap();
        assert_eq!(
            read_memory(dir.path()).await.unwrap().content,
            "edited notes\n"
        );
        assert_ne!(saved.version, original.version);
        std::fs::write(&path, "agent wrote new information\n").unwrap();
        assert!(matches!(
            save_memory(dir.path(), &saved.version, "stale edit").await,
            Err(otto_core::Error::Conflict(_))
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "agent wrote new information\n"
        );
    }

    #[tokio::test]
    async fn memory_version_binds_the_original_directory_even_for_missing_or_identical_notes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let original = read_memory(a.path()).await.unwrap();
        assert!(matches!(save_memory(b.path(), &original.version, "old editor").await, Err(Error::Conflict(_))));
        assert!(!b.path().join("memory/notes.md").exists());
        seed_memory(a.path(), "same notes").await.unwrap();
        seed_memory(b.path(), "same notes").await.unwrap();
        let original = read_memory(a.path()).await.unwrap();
        assert!(matches!(save_memory(b.path(), &original.version, "old editor").await, Err(Error::Conflict(_))));
        assert_eq!(read_memory(b.path()).await.unwrap().content, "same notes");
    }

    #[tokio::test]
    async fn memory_rejects_symlink_escape_and_oversized_content() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("notes.md"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("memory")).unwrap();
        assert!(read_memory(root.path()).await.is_err());
        assert!(save_memory(root.path(), "missing", "overwrite")
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("notes.md")).unwrap(),
            "outside"
        );
        let clean = tempfile::tempdir().unwrap();
        assert!(
            save_memory(clean.path(), "missing", &"x".repeat(MAX_DOCUMENT_BYTES + 1))
                .await
                .is_err()
        );
    }
}

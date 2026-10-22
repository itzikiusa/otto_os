//! `<data>/sessions/<id>/` — files a session owns on disk (the composer's
//! pasted images land in its `inbox/`, up to 10 MB each). They die with the
//! session: [`SessionManager::remove`] deletes the folder after the row, and
//! [`SessionManager::sweep_orphan_session_files`] removes folders whose row
//! is gone (sessions deleted before removal cleaned up, or a failed delete)
//! — data-integrity-4-03. Only with a configured data dir
//! ([`SessionManager::with_data_dir`]): never a guessed default.

use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Id};

use super::SessionManager;

/// `<root>/<id>` when `id` is one plain path component, else `None` — a
/// lexical confinement so an odd id can never point the delete outside
/// `root`.
fn session_dir_in(root: &Path, id: &str) -> Option<PathBuf> {
    let mut comps = Path::new(id).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(c)), None) if c == id => Some(root.join(c)),
        _ => None,
    }
}

/// Remove `path` (a session folder): a symlink is unlinked, never followed;
/// a missing folder is fine.
async fn remove_session_dir(path: &Path) -> std::io::Result<()> {
    let res = match tokio::fs::symlink_metadata(path).await {
        Ok(m) if m.is_dir() => tokio::fs::remove_dir_all(path).await,
        Ok(_) => tokio::fs::remove_file(path).await,
        Err(e) => Err(e),
    };
    match res {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

impl SessionManager {
    /// `<data>/sessions`, when the daemon configured a data dir.
    fn session_files_root(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("sessions"))
    }

    /// Delete `<data>/sessions/<id>/` (best-effort: a failure is a warning,
    /// the orphan sweep retries it later).
    pub(super) async fn remove_session_files(&self, id: &Id) {
        let Some(dir) = self
            .session_files_root()
            .and_then(|root| session_dir_in(&root, id))
        else {
            return;
        };
        if let Err(e) = remove_session_dir(&dir).await {
            tracing::warn!(session = %id, "remove session files: {e}");
        }
    }

    /// Remove `<data>/sessions/<dir>` entries older than `grace` whose id has
    /// no `sessions` row. A lookup error other than "not found" keeps the
    /// folder (can't tell — remove nothing). Returns folders removed.
    pub async fn sweep_orphan_session_files(&self, grace: Duration) -> usize {
        let Some(root) = self.session_files_root() else {
            return 0;
        };
        let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
            return 0;
        };
        let mut removed = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(path) = session_dir_in(&root, &id) else {
                continue;
            };
            let old = tokio::fs::symlink_metadata(&path)
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age >= grace);
            if !old || !matches!(self.repo.get(&id).await, Err(Error::NotFound(_))) {
                continue;
            }
            match remove_session_dir(&path).await {
                Ok(()) => removed += 1,
                Err(e) => {
                    tracing::warn!(path = %path.display(), "remove orphan session files: {e}")
                }
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_dir_is_confined_to_one_component() {
        let root = Path::new("/data/sessions");
        assert_eq!(
            session_dir_in(root, "01ABC"),
            Some(PathBuf::from("/data/sessions/01ABC"))
        );
        for bad in ["", ".", "..", "../x", "a/b", "/etc", "a/.."] {
            assert_eq!(session_dir_in(root, bad), None, "{bad:?}");
        }
    }
}

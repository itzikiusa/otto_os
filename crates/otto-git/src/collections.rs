//! API-collections files in a registered repo's `collections/` directory
//! (`/repos/{id}/api-collections/{pull,push}`).
//!
//! The repo is not trusted (data-integrity-5-01): a shared collections repo
//! can commit `collections/X.postman_collection.json -> ~/.zshrc`, or make
//! `collections` itself a link. `tokio::fs::write` follows both, so a pull →
//! edit → push replaced a file OUTSIDE the repo with Postman JSON while
//! `git add` staged only the unchanged link. So the read skips every symlink,
//! and the write resolves each target through
//! [`LocalGit::confined_worktree_file`] (inside the tree, out of `$GIT_DIR`,
//! final component not a link), validates ALL of them before writing any, and
//! lands each file through a sibling temp file + `rename`.

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

use crate::local::LocalGit;

/// The repo-relative directory the API client syncs.
pub const COLLECTIONS_DIR: &str = "collections";

/// `collections/<name>.json` for a pushed collection name: path separators
/// flattened (one directory, never a subpath), `.json` appended when missing.
pub fn collection_rel_path(name: &str) -> String {
    let safe = name.replace(['/', '\\'], "_");
    if safe.ends_with(".json") {
        format!("{COLLECTIONS_DIR}/{safe}")
    } else {
        format!("{COLLECTIONS_DIR}/{safe}.json")
    }
}

/// Every regular `collections/*.json` file as `(file name, content)`. A
/// symlinked `collections` dir or entry is skipped, never followed: its
/// target may be any file the daemon user can read.
pub async fn read_collection_files(repo: &Path) -> Vec<(String, String)> {
    let dir = repo.join(COLLECTIONS_DIR);
    let mut files = Vec::new();
    match tokio::fs::symlink_metadata(&dir).await {
        Ok(m) if m.is_dir() => {}
        _ => return files,
    }
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return files;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        // `DirEntry::file_type` does not follow links.
        match entry.file_type().await {
            Ok(t) if t.is_file() => {}
            _ => continue,
        }
        if let Ok(content) = tokio::fs::read_to_string(&p).await {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            files.push((name.to_string(), content));
        }
    }
    files
}

impl LocalGit {
    /// Write each `(repo-relative path, content)` pair. Every target is
    /// resolved and refused up front (a symlinked `collections` dir or file,
    /// anything outside the tree), so a refusal leaves the work tree
    /// untouched; each file is then written to a sibling temp file and
    /// renamed into place, so a failed write never leaves a half file.
    pub async fn write_collection_files(&self, files: &[(String, String)]) -> Result<()> {
        let dir = self.path().join(COLLECTIONS_DIR);
        if let Ok(m) = tokio::fs::symlink_metadata(&dir).await {
            if !m.is_dir() {
                return Err(Error::Invalid(format!(
                    "refusing to write {COLLECTIONS_DIR}/: it is a symlink or not a directory"
                )));
            }
        }
        let mut targets: Vec<PathBuf> = Vec::with_capacity(files.len());
        for (rel, _) in files {
            targets.push(self.confined_worktree_file(rel, false).await?);
        }
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::Internal(format!("create collections dir: {e}")))?;
        for ((rel, content), abs) in files.iter().zip(&targets) {
            write_via_rename(abs, content)
                .await
                .map_err(|e| Error::Internal(format!("write {rel}: {e}")))?;
        }
        Ok(())
    }
}

/// Write `content` to a sibling temp file, then `rename` it over `abs`
/// (atomic on one filesystem; replaces a link rather than following it).
async fn write_via_rename(abs: &Path, content: &str) -> std::io::Result<()> {
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = abs.with_file_name(format!(".{name}.otto-{}-{seq}.tmp", std::process::id()));
    if let Err(e) = tokio::fs::write(&tmp, content).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&tmp, abs).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        let mut cmd = std::process::Command::new("git");
        otto_git_cmd::test_hermetic(&mut cmd);
        let out = cmd
            .current_dir(&dir)
            .args(["init", "-q", "-b", "main"])
            .output()
            .unwrap();
        assert!(out.status.success());
        (tmp, dir)
    }

    #[test]
    fn rel_path_flattens_separators_and_appends_json() {
        assert_eq!(collection_rel_path("a/b\\c"), "collections/a_b_c.json");
        assert_eq!(collection_rel_path("x.json"), "collections/x.json");
    }

    /// data-integrity-5-01: a committed symlink at the push target is refused,
    /// its target stays byte-identical, and no earlier file was written.
    #[cfg(unix)]
    #[tokio::test]
    async fn push_refuses_a_symlinked_collection_file_and_writes_nothing() {
        let (tmp, dir) = repo();
        let outside = tmp.path().join("victim.txt");
        std::fs::write(&outside, b"precious\n").unwrap();
        std::fs::create_dir(dir.join("collections")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("collections/Pay.json")).unwrap();
        let git = LocalGit::new(&dir);
        let files = vec![
            (collection_rel_path("First"), "{\"a\":1}".to_string()),
            (collection_rel_path("Pay"), "{\"b\":2}".to_string()),
        ];
        let err = git.write_collection_files(&files).await.unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert_eq!(std::fs::read(&outside).unwrap(), b"precious\n");
        assert!(!dir.join("collections/First.json").exists());
    }

    /// A symlinked `collections` directory is refused for writes and skipped
    /// for reads; a symlinked entry inside a real directory is skipped too.
    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_collections_dir_is_refused_and_links_are_not_read() {
        let (tmp, dir) = repo();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("secret.json"), "s").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("collections")).unwrap();
        let git = LocalGit::new(&dir);
        let files = vec![(collection_rel_path("New"), "{}".to_string())];
        assert!(git.write_collection_files(&files).await.is_err());
        assert!(!elsewhere.join("New.json").exists());
        assert!(read_collection_files(&dir).await.is_empty());

        std::fs::remove_file(dir.join("collections")).unwrap();
        std::fs::create_dir(dir.join("collections")).unwrap();
        std::fs::write(dir.join("collections/ok.json"), "{\"ok\":1}").unwrap();
        std::os::unix::fs::symlink(
            elsewhere.join("secret.json"),
            dir.join("collections/leak.json"),
        )
        .unwrap();
        let read = read_collection_files(&dir).await;
        assert_eq!(
            read,
            vec![("ok.json".to_string(), "{\"ok\":1}".to_string())]
        );
    }

    /// The normal flow still works: the directory is created, files land
    /// whole and no temp file is left behind.
    #[tokio::test]
    async fn push_writes_every_file_and_leaves_no_temp_files() {
        let (_tmp, dir) = repo();
        let git = LocalGit::new(&dir);
        let files = vec![
            (collection_rel_path("A"), "{\"a\":1}".to_string()),
            (collection_rel_path("B.json"), "{\"b\":2}".to_string()),
        ];
        git.write_collection_files(&files).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("collections/A.json")).unwrap(),
            "{\"a\":1}"
        );
        let names: Vec<_> = std::fs::read_dir(dir.join("collections"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }
}

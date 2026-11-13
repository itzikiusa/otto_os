//! Parent directories a move creates for its target. A move that then fails
//! (target exists, source vanished) takes back exactly those — never a
//! directory that existed before, and never one something was put in since.

use std::io;
use std::path::{Path, PathBuf};

/// Create the missing parent directories of `target`, whose vault-relative
/// path is `rel` (so only the `rel` levels below the vault root are ever
/// touched), one level at a time. Returns the ones THIS call created,
/// shallowest first; a level that already exists (or appears concurrently)
/// is not ours.
pub(crate) fn create_parents(target: &Path, rel: &str) -> io::Result<Vec<PathBuf>> {
    let depth = rel.matches('/').count();
    let mut levels: Vec<&Path> = target.ancestors().skip(1).take(depth).collect();
    levels.reverse();
    let mut created = Vec::new();
    for dir in levels {
        match std::fs::create_dir(dir) {
            Ok(()) => created.push(dir.to_path_buf()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                remove_created(&created);
                return Err(e);
            }
        }
    }
    Ok(created)
}

/// Remove `created` (from [`create_parents`]) deepest first. `remove_dir`
/// refuses a non-empty directory, so one that gained an entry meanwhile
/// stays, and so do its ancestors.
pub(crate) fn remove_created(created: &[PathBuf]) {
    for dir in created.iter().rev() {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_levels_this_call_created_are_taken_back() {
        let td = tempfile::tempdir().unwrap();
        std::fs::create_dir(td.path().join("had")).unwrap();
        let target = td.path().join("had/new/deeper/x.md");
        let created = create_parents(&target, "had/new/deeper/x.md").unwrap();
        assert_eq!(
            created,
            vec![td.path().join("had/new"), td.path().join("had/new/deeper")]
        );
        remove_created(&created);
        assert!(td.path().join("had").is_dir(), "pre-existing stays");
        assert!(!td.path().join("had/new").exists());
    }

    #[test]
    fn a_created_directory_that_gained_an_entry_stays() {
        let td = tempfile::tempdir().unwrap();
        let target = td.path().join("a/b/x.md");
        let created = create_parents(&target, "a/b/x.md").unwrap();
        std::fs::write(td.path().join("a/keep.md"), "x").unwrap();
        remove_created(&created);
        assert!(!td.path().join("a/b").exists(), "empty and ours: removed");
        assert!(td.path().join("a/keep.md").is_file());
    }

    #[test]
    fn nothing_above_the_vault_relative_path_is_created() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("vault-root-missing");
        // `rel` has no parent levels: the missing root is not ours to make.
        assert!(create_parents(&root.join("x.md"), "x.md")
            .unwrap()
            .is_empty());
        assert!(!root.exists());
    }
}

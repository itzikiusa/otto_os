//! Self-improvement writes to an Otto **Library** skill go through
//! [`otto_context::Library`], not straight to disk: the conditional write
//! (`if_hash`) and its process-wide lock are the same ones the Skills Lab
//! editor uses, so an applied (or rolled-back) edit and a person's save can't
//! silently overwrite each other. After a write the skill is re-copied into
//! the provider skill folders whose manifest says Otto owns that copy
//! ([`otto_context::user_skills::refresh_owned`]), so the CLIs run the edit.
//!
//! Workspace skills and memory files keep the engine's own atomic write.

use std::io;
use std::path::{Component, Path};

use chrono::Utc;
use otto_context::Library;
use otto_core::{Error, Result};

/// A resolved edit target that is a file of a Library skill.
pub(crate) struct LibrarySkillFile {
    library: Library,
    name: String,
    rel: String,
}

/// `Some` when `target` is `<library_root>/skills/<name>/<rel>` (compared
/// literally, then with both sides canonicalized: `/var` vs `/private/var`).
pub(crate) fn skill_target(library_root: &Path, target: &Path) -> Option<LibrarySkillFile> {
    split(library_root, target).or_else(|| {
        let root = std::fs::canonicalize(library_root).ok()?;
        let parent = std::fs::canonicalize(target.parent()?).ok()?;
        split(&root, &parent.join(target.file_name()?))
    })
}

fn split(library_root: &Path, target: &Path) -> Option<LibrarySkillFile> {
    let rest = target.strip_prefix(library_root.join("skills")).ok()?;
    let mut parts = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    if parts.len() < 2 {
        return None;
    }
    let name = parts.remove(0);
    let library = Library::new(library_root);
    library.skill_dir(&name)?; // unsafe names never take this path
    Some(LibrarySkillFile {
        library,
        name,
        rel: parts.join("/"),
    })
}

/// Move the file from `expected` (what the decision was based on; `None` =
/// absent) to `new` (`None` = removed) through the Library. `Ok(false)` = the
/// file is no longer `expected` (conflict): nothing was written. A replaced or
/// removed file is first saved as a hidden `.<file>.bak-<ts>` sibling, the
/// engine's usual rollback trail.
pub(crate) async fn apply(
    target: LibrarySkillFile,
    expected: Option<String>,
    new: Option<String>,
) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        // The engine's own unit tests must never touch the real ~/.claude,
        // $CODEX_HOME or ~/.gemini skill folders.
        let providers = if cfg!(test) {
            Vec::new()
        } else {
            otto_context::user_skills::provider_skill_dirs()
        };
        apply_blocking(&target, expected.as_deref(), new.as_deref(), &providers)
    })
    .await
    .map_err(|e| Error::Internal(format!("library skill write task: {e}")))?
    .map_err(|e| Error::Internal(format!("write library skill file: {e}")))
}

fn apply_blocking(
    t: &LibrarySkillFile,
    expected: Option<&str>,
    new: Option<&str>,
    provider_dirs: &[std::path::PathBuf],
) -> io::Result<bool> {
    let lib = &t.library;
    let written = match (expected, new) {
        (None, None) => return Ok(true), // absent and meant to be absent
        (None, Some(content)) => match lib.create_skill_file(&t.name, &t.rel, content) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e),
        },
        (Some(prev), Some(content)) => {
            let base = otto_core::text::content_sha256(prev);
            if lib.skill_file_hash(&t.name, &t.rel).as_deref() != Some(base.as_str()) {
                return Ok(false);
            }
            backup(t, prev)?;
            lib.write_skill_file_if(&t.name, &t.rel, content, &base)?
                .is_ok()
        }
        (Some(prev), None) => {
            let base = otto_core::text::content_sha256(prev);
            if lib.skill_file_hash(&t.name, &t.rel).as_deref() != Some(base.as_str()) {
                return Ok(false);
            }
            backup(t, prev)?;
            lib.remove_skill_file_if(&t.name, &t.rel, &base)?.is_ok()
        }
    };
    if written {
        if let Some(src) = lib.skill_dir(&t.name).filter(|d| d.is_dir()) {
            let refreshed =
                otto_context::user_skills::refresh_owned_in_dirs(provider_dirs, &t.name, &src);
            if let Err(e) = refreshed {
                tracing::warn!(skill = %t.name, error = %e, "refresh user-level skill copies failed");
            }
        }
    }
    Ok(written)
}

/// The engine's `.bak` trail, written next to the Library file.
fn backup(t: &LibrarySkillFile, prev: &str) -> io::Result<()> {
    let Some(path) = t.library.skill_dir(&t.name).map(|d| d.join(&t.rel)) else {
        return Ok(());
    };
    let (Some(parent), Some(file)) = (path.parent(), path.file_name()) else {
        return Ok(());
    };
    let bak = parent.join(format!(
        ".{}.bak-{}",
        file.to_string_lossy(),
        Utc::now().format("%Y%m%dT%H%M%S%.6f")
    ));
    std::fs::write(bak, prev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_with_skill(root: &Path) -> std::path::PathBuf {
        let lib = Library::new(root);
        lib.put_skill("grill", "---\nversion: 1\n---\nOLD\n")
            .unwrap();
        lib.skill_path("grill").unwrap()
    }

    #[test]
    fn only_library_skill_files_take_the_library_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let path = library_with_skill(&root);
        let t = skill_target(&root, &path).expect("library skill");
        assert_eq!((t.name.as_str(), t.rel.as_str()), ("grill", "SKILL.md"));
        assert!(skill_target(
            &root,
            &dir.path().join("repo/.claude/skills/grill/SKILL.md")
        )
        .is_none());
        assert!(skill_target(&root, &root.join("skills/SKILL.md")).is_none());
        assert!(skill_target(&root, &root.join("souls/x.md")).is_none());
    }

    /// An applied edit reaches the provider copy Otto owns, never one it does
    /// not own; a stale snapshot writes nothing anywhere.
    #[test]
    fn applied_edit_refreshes_owned_provider_copy_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let path = library_with_skill(&root);
        let owned = dir.path().join("claude");
        let mine = dir.path().join("codex");
        otto_context::user_skills::install_into_dirs(
            std::slice::from_ref(&owned),
            "grill",
            &root.join("skills/grill"),
        )
        .unwrap();
        std::fs::create_dir_all(mine.join("grill")).unwrap();
        std::fs::write(mine.join("grill/SKILL.md"), "my own\n").unwrap();
        let providers = [owned.clone(), mine.clone()];

        let t = skill_target(&root, &path).unwrap();
        let old = std::fs::read_to_string(&path).unwrap();
        assert!(!apply_blocking(&t, Some("not on disk"), Some("NEW\n"), &providers).unwrap());
        assert_eq!(
            std::fs::read_to_string(owned.join("grill/SKILL.md")).unwrap(),
            old
        );

        assert!(apply_blocking(&t, Some(&old), Some("NEW\n"), &providers).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "NEW\n");
        assert_eq!(
            std::fs::read_to_string(owned.join("grill/SKILL.md")).unwrap(),
            "NEW\n"
        );
        assert_eq!(
            std::fs::read_to_string(mine.join("grill/SKILL.md")).unwrap(),
            "my own\n"
        );
    }

    #[test]
    fn apply_writes_backs_up_and_detects_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        let path = library_with_skill(&root);
        let old = std::fs::read_to_string(&path).unwrap();
        let t = skill_target(&root, &path).unwrap();
        assert!(apply_blocking(&t, Some(&old), Some("NEW\n"), &[]).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "NEW\n");
        let baks = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".SKILL.md.bak-")
            })
            .count();
        assert_eq!(baks, 1);
        // Rolling back against a stale "after" is a conflict.
        assert!(!apply_blocking(&t, Some("OTHER\n"), Some(&old), &[]).unwrap());
        // New file: create_new; an existing file is a conflict, not a clobber.
        assert!(apply_blocking(&t, None, Some("x"), &[])
            .map(|w| !w)
            .unwrap());
        let refs = skill_target(&root, &root.join("skills/grill/references/a.md")).unwrap();
        assert!(apply_blocking(&refs, None, Some("a\n"), &[]).unwrap());
        assert!(apply_blocking(&refs, Some("a\n"), None, &[]).unwrap());
        assert!(!root.join("skills/grill/references/a.md").exists());
    }
}

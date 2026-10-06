//! User-level (global) skill materialization for the provider CLIs.
//!
//! When a bundled skill is installed into the Otto Library, Otto ALSO drops a
//! full copy into each provider's *user-level* skills directory — the native
//! global skills dir that CLI scans on every run — so the skill is discoverable
//! everywhere, independent of any per-session context bundle:
//!
//! - **claude** → `~/.claude/skills/<name>/`
//! - **codex**  → `$CODEX_HOME/skills/<name>/` (`CODEX_HOME` if set, else `~/.codex/skills/<name>/`)
//! - **agy**    → `~/.gemini/skills/<name>/` (agy's home is `~/.gemini`)
//!
//! All three share the same multi-file `SKILL.md` (+ `references/`/`assets/`/
//! `scripts/`) layout, so a single recursive copy works for each. We **only**
//! write inside these user-level skills dirs — never into a working/repo tree.
//!
//! Each provider dir carries a `.otto-managed.json` manifest (the same convention
//! [`crate::merge`] uses for the add-dir bundle) listing the skill dirs Otto owns
//! there. Install records ownership; uninstall/update reconcile against it so a
//! removal only ever touches a skill Otto installed — a user-authored skill of
//! the same name is left untouched.
//!
//! Note: the `.claude/skills` / `.gemini/skills` *user-level* dirs here are a
//! different thing from the per-spawn add-dir bundle subdirs in
//! [`crate::materialize`] (`.claude/skills`, `.agents/skills`, …); that bundle is
//! materialized fresh per session, whereas these are the CLIs' persistent global
//! libraries.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::merge;

/// Resolve the user-level skills dir for every provider CLI that has a native
/// global skills registry. Order is stable (claude, codex, agy) for
/// deterministic manifests/tests. Returned dirs may not exist yet — [`install`]
/// creates them on demand.
pub fn provider_skill_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.push(home.join(".claude").join("skills"));
    }
    dirs.push(codex_home().join("skills"));
    if let Some(home) = home_dir() {
        dirs.push(home.join(".gemini").join("skills"));
    }
    dirs
}

/// `$HOME` as a path, or `None` when unset/empty (mirrors `materialize`).
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Codex's home: `$CODEX_HOME` if set, else `~/.codex` (mirrors
/// `otto-sessions::codex_sessions_root`). Falls back to `.codex` under the temp
/// dir only when `$HOME` is also unset, so a path always resolves.
fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().unwrap_or_else(std::env::temp_dir).join(".codex"))
}

/// Install (clean-overwrite) skill `name` into every user-level provider skills
/// dir, copying the full multi-file tree from `src` (the freshly-installed
/// Library copy). Records `name` in each dir's Otto manifest so a later
/// uninstall/update only ever touches skills Otto owns. Used for both first
/// install and updates — the clean-overwrite makes an update replace cleanly.
///
/// A provider dir that already holds a `name` entry Otto does NOT own (absent
/// from its manifest — a hand-authored skill, or a symlink into the user's own
/// synced skill repo) is left untouched (S7-304); those dirs are returned so
/// the caller can report them.
pub fn install(name: &str, src: &Path) -> io::Result<Vec<PathBuf>> {
    install_into_dirs(&provider_skill_dirs(), name, src)
}

/// Remove skill `name` from every user-level provider skills dir **iff** that
/// dir's Otto manifest owns it, then drop it from the manifest. A skill Otto
/// never installed (absent from the manifest — e.g. user-authored) is left
/// untouched.
pub fn uninstall(name: &str) -> io::Result<()> {
    uninstall_from_dirs(&provider_skill_dirs(), name)
}

// ---------------------------------------------------------------------------
// Core (dir-explicit, so the home/CODEX_HOME resolution stays out of tests)
// ---------------------------------------------------------------------------

fn install_into_dirs(dirs: &[PathBuf], name: &str, src: &Path) -> io::Result<Vec<PathBuf>> {
    // `name` arrives from the HTTP layer — refuse anything that is not a
    // single path component before it is joined under the provider dirs.
    let name = otto_core::paths::safe_component(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    let mut user_owned = Vec::new();
    for dir in dirs {
        let dest = dir.join(name);
        // `symlink_metadata`: a dangling or live symlink is "present" too, and
        // is never followed or replaced.
        if let Ok(meta) = fs::symlink_metadata(&dest) {
            let owned = merge::read_manifest(dir).iter().any(|n| n == name);
            if !owned || meta.file_type().is_symlink() || !meta.is_dir() {
                user_owned.push(dir.clone());
                continue; // the user's own skill of this name — never clobber it
            }
            fs::remove_dir_all(&dest)?; // clean-overwrite so updates replace cleanly
        }
        copy_tree(src, &dest)?;
        // Record ownership (idempotent set-insert), keeping the list sorted.
        let mut owned = merge::read_manifest(dir);
        if !owned.iter().any(|n| n == name) {
            owned.push(name.to_string());
            owned.sort();
            merge::write_manifest(dir, &owned)?;
        }
    }
    Ok(user_owned)
}

fn uninstall_from_dirs(dirs: &[PathBuf], name: &str) -> io::Result<()> {
    // Same gate as install: the name must be a single safe path component.
    let name = otto_core::paths::safe_component(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    for dir in dirs {
        let mut owned = merge::read_manifest(dir);
        if !owned.iter().any(|n| n == name) {
            continue; // not Otto-managed here — never clobber a user-authored skill
        }
        match fs::remove_dir_all(dir.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        owned.retain(|n| n != name);
        merge::write_manifest(dir, &owned)?;
    }
    Ok(())
}

/// Recursively copy a directory tree from `src` to `dest`, creating parents.
/// `std::fs::copy` preserves the source mode, so `.sh` exec bits survive.
fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A minimal multi-file source skill under `<root>/src/grill`.
    fn src_skill(root: &Path) -> PathBuf {
        let s = root.join("src").join("grill");
        fs::create_dir_all(s.join("references")).unwrap();
        fs::write(s.join("SKILL.md"), "---\nversion: 2\n---\n# grill\n").unwrap();
        fs::write(s.join("references").join("notes.md"), "notes\n").unwrap();
        s
    }

    #[test]
    fn install_copies_full_tree_and_records_manifest() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let d1 = tmp.path().join("claude").join("skills");
        let d2 = tmp.path().join("codex").join("skills");
        let dirs = vec![d1.clone(), d2.clone()];

        install_into_dirs(&dirs, "grill", &src).unwrap();

        for d in [&d1, &d2] {
            assert!(d.join("grill/SKILL.md").is_file());
            assert!(d.join("grill/references/notes.md").is_file());
            assert_eq!(merge::read_manifest(d), vec!["grill".to_string()]);
        }
    }

    #[test]
    fn update_clean_overwrites_and_manifest_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let d = tmp.path().join("claude").join("skills");
        let dirs = vec![d.clone()];

        install_into_dirs(&dirs, "grill", &src).unwrap();
        // A stale file present before re-install must be gone after.
        fs::write(d.join("grill/STALE.md"), "x").unwrap();
        install_into_dirs(&dirs, "grill", &src).unwrap();

        assert!(!d.join("grill/STALE.md").exists());
        assert!(d.join("grill/SKILL.md").is_file());
        assert_eq!(merge::read_manifest(&d), vec!["grill".to_string()]);
    }

    #[test]
    fn install_never_replaces_an_unmanaged_or_symlinked_provider_skill() {
        // S7-304: a hand-authored skill of the same name survives install.
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let mine = tmp.path().join("claude").join("skills");
        let linked = tmp.path().join("codex").join("skills");
        let fresh = tmp.path().join("gemini").join("skills");
        fs::create_dir_all(mine.join("grill")).unwrap();
        fs::write(mine.join("grill/SKILL.md"), "my own grill\n").unwrap();
        let repo = tmp.path().join("ai_skills/grill");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("SKILL.md"), "synced\n").unwrap();
        fs::create_dir_all(&linked).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&repo, linked.join("grill")).unwrap();
        // Even a manifest claim does not make Otto replace a symlink.
        merge::write_manifest(&linked, &["grill".to_string()]).unwrap();

        let skipped = install_into_dirs(
            &[mine.clone(), linked.clone(), fresh.clone()],
            "grill",
            &src,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(mine.join("grill/SKILL.md")).unwrap(),
            "my own grill\n"
        );
        assert!(merge::read_manifest(&mine).is_empty(), "not claimed");
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(linked.join("grill"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                fs::read_to_string(repo.join("SKILL.md")).unwrap(),
                "synced\n"
            );
            assert!(skipped.contains(&linked));
        }
        assert!(skipped.contains(&mine));
        assert!(fresh.join("grill/references/notes.md").is_file());
        assert!(!skipped.contains(&fresh));
    }

    #[test]
    fn uninstall_only_removes_managed_skills() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let d = tmp.path().join("claude").join("skills");
        let dirs = vec![d.clone()];
        install_into_dirs(&dirs, "grill", &src).unwrap();

        // A user-authored skill Otto never installed sits alongside it.
        fs::create_dir_all(d.join("mine")).unwrap();
        fs::write(d.join("mine/SKILL.md"), "mine\n").unwrap();

        uninstall_from_dirs(&dirs, "grill").unwrap();
        assert!(!d.join("grill").exists());
        assert!(merge::read_manifest(&d).is_empty());
        // The user-authored skill is untouched.
        assert!(d.join("mine/SKILL.md").is_file());

        // Uninstalling a name Otto doesn't own is a no-op that preserves it.
        uninstall_from_dirs(&dirs, "mine").unwrap();
        assert!(d.join("mine/SKILL.md").is_file());
    }
}

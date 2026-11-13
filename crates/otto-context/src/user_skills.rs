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
//! People edit those copies in place too (by hand, or an agent's
//! skill-creator). Otto records the tree hash of each copy it writes, and an
//! owned copy that no longer matches is moved to
//! `<library>/provider-skills-backup/<provider>/` before Otto replaces or
//! removes it (`user_skills/copies.rs`); callers get it back as a [`KeptCopy`].
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

mod copies;

pub use copies::{tree_hash, KeptCopy, PROVIDER_BACKUPS_KEPT, PROVIDER_BACKUP_DIR};

/// What one install / refresh did across the provider dirs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CopyReport {
    /// Provider dirs whose copy now matches the Library tree.
    pub written: Vec<PathBuf>,
    /// Provider dirs left untouched because they hold a `name` entry Otto does
    /// not own (S7-304).
    pub user_owned: Vec<PathBuf>,
    /// Hand-edited copies moved to `<library>/provider-skills-backup/` before
    /// Otto replaced them.
    pub kept: Vec<KeptCopy>,
}

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

/// Install (overwrite) skill `name` into every user-level provider skills
/// dir, copying the full multi-file tree from `src` (the freshly-installed
/// Library copy). Records `name` in each dir's Otto manifest so a later
/// uninstall/update only ever touches skills Otto owns. Used for both first
/// install and updates. An owned copy the person edited is first moved to
/// `<library_root>/provider-skills-backup/` (see [`CopyReport::kept`]).
///
/// A provider dir that already holds a `name` entry Otto does NOT own (absent
/// from its manifest — a hand-authored skill, or a symlink into the user's own
/// synced skill repo) is left untouched (S7-304); those dirs are reported in
/// [`CopyReport::user_owned`].
pub fn install(library_root: &Path, name: &str, src: &Path) -> io::Result<CopyReport> {
    install_into_dirs(&provider_skill_dirs(), library_root, name, src)
}

/// Remove skill `name` from every user-level provider skills dir **iff** that
/// dir's Otto manifest owns it, then drop it from the manifest. A skill Otto
/// never installed (absent from the manifest — e.g. user-authored) is left
/// untouched; an owned copy the person edited is moved to
/// `<library_root>/provider-skills-backup/` instead of deleted, and returned.
pub fn uninstall(library_root: &Path, name: &str) -> io::Result<Vec<KeptCopy>> {
    uninstall_from_dirs(&provider_skill_dirs(), library_root, name)
}

/// Re-copy skill `name` from its Library tree `src` into every provider dir
/// whose manifest says Otto owns that copy, after the Library copy was edited
/// (Skills Lab save, new/deleted file, import). Unlike [`install`] this never
/// creates a copy where there is none and never touches a copy Otto does not
/// own, so a skill the person only keeps in Otto stays out of the CLIs.
pub fn refresh_owned(library_root: &Path, name: &str, src: &Path) -> io::Result<CopyReport> {
    refresh_owned_in_dirs(&provider_skill_dirs(), library_root, name, src)
}

// ---------------------------------------------------------------------------
// Core (dir-explicit, so the home/CODEX_HOME resolution stays out of tests)
// ---------------------------------------------------------------------------

/// [`refresh_owned`] over explicit provider roots. Each owned copy is replaced
/// whole: the new tree is copied into a hidden sibling first and swapped in
/// with renames, so a failed copy leaves the previous tree in place. A copy
/// the person edited is moved to the backup folder first.
pub fn refresh_owned_in_dirs(
    dirs: &[PathBuf],
    library_root: &Path,
    name: &str,
    src: &Path,
) -> io::Result<CopyReport> {
    let name = otto_core::paths::safe_component(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    let _edit = manifest_edit_guard();
    let mut report = CopyReport::default();
    for dir in dirs {
        if !merge::read_manifest_checked(dir)?.iter().any(|n| n == name) {
            continue; // not Otto's copy (or no copy) — never written
        }
        let dest = dir.join(name);
        match fs::symlink_metadata(&dest) {
            // A copy the person removed stays removed; a symlink is theirs.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => continue,
            Ok(_) => {}
        }
        report
            .kept
            .extend(copies::replace_owned(library_root, dir, name, src, &dest)?);
        report.written.push(dir.clone());
    }
    Ok(report)
}

/// Serializes every manifest read-modify-write (install / uninstall) in this
/// process, so two concurrent requests on one skills dir can't lose an
/// entry. One process-wide lock rather than `KeyedLocks<PathBuf>`: these are
/// sync fns called straight from async handlers, and `KeyedLocks` is a tokio
/// mutex (`blocking_lock` would panic there). Installs and uninstalls are
/// rare, root-only operations, so per-dir granularity buys nothing.
static MANIFEST_EDITS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn manifest_edit_guard() -> std::sync::MutexGuard<'static, ()> {
    MANIFEST_EDITS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Install into explicit provider roots, preserving user-owned entries.
/// Used by callers that need one stable set of roots for a bulk operation.
pub fn install_into_dirs(
    dirs: &[PathBuf],
    library_root: &Path,
    name: &str,
    src: &Path,
) -> io::Result<CopyReport> {
    // `name` arrives from the HTTP layer — refuse anything that is not a
    // single path component before it is joined under the provider dirs.
    let name = otto_core::paths::safe_component(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    let _edit = manifest_edit_guard();
    let mut report = CopyReport::default();
    for dir in dirs {
        let dest = dir.join(name);
        // `symlink_metadata`: a dangling or live symlink is "present" too, and
        // is never followed or replaced.
        match fs::symlink_metadata(&dest) {
            Ok(meta) => {
                let owned = merge::read_manifest_checked(dir)?.iter().any(|n| n == name);
                if !owned || meta.file_type().is_symlink() || !meta.is_dir() {
                    report.user_owned.push(dir.clone());
                    continue; // the user's own skill of this name — never clobber it
                }
                // Otto's copy: swapped, never deleted first — a failed copy
                // keeps the previous tree; an edited one is kept aside.
                report
                    .kept
                    .extend(copies::replace_owned(library_root, dir, name, src, &dest)?);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                install_new(dir, name, src, &dest)?;
            }
            Err(error) => return Err(error),
        }
        report.written.push(dir.clone());
    }
    Ok(report)
}

/// First copy of `name` into `dir`: staged beside `dest`, renamed in, then
/// claimed in the manifest. On any failure only this attempt's tree is
/// removed — never a tree that appeared at `dest` meanwhile.
fn install_new(dir: &Path, name: &str, src: &Path, dest: &Path) -> io::Result<()> {
    let hash = copies::tree_hash(src)?;
    let fresh = copies::stage(src, dest)?;
    fs::rename(&fresh, dest).inspect_err(|_| {
        let _ = fs::remove_dir_all(&fresh);
    })?;
    // Record ownership only after a complete copy.
    let claimed: io::Result<()> = (|| {
        let mut owned = merge::read_manifest_checked(dir)?;
        if !owned.iter().any(|n| n == name) {
            owned.push(name.to_string());
            owned.sort();
            merge::write_manifest(dir, &owned)?;
        }
        Ok(())
    })();
    if let Err(e) = claimed {
        let _ = fs::remove_dir_all(dest);
        return Err(e);
    }
    copies::note(dir, name, Some(&hash));
    Ok(())
}

fn uninstall_from_dirs(
    dirs: &[PathBuf],
    library_root: &Path,
    name: &str,
) -> io::Result<Vec<KeptCopy>> {
    // Same gate as install: the name must be a single safe path component.
    let name = otto_core::paths::safe_component(name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    let _edit = manifest_edit_guard();
    let mut kept = Vec::new();
    for dir in dirs {
        let mut owned = merge::read_manifest_checked(dir)?;
        if !owned.iter().any(|n| n == name) {
            continue; // not Otto-managed here — never clobber a user-authored skill
        }
        let dest = dir.join(name);
        match fs::symlink_metadata(&dest) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            // A symlink (or a file) under the name is never a tree Otto wrote.
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {}
            Ok(_) => kept.extend(copies::remove_owned(library_root, dir, name, &dest)?),
        }
        owned.retain(|n| n != name);
        merge::write_manifest(dir, &owned)?;
        copies::note(dir, name, None);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// The Library root the backups go under, inside the test's temp dir.
    fn lib(root: &Path) -> PathBuf {
        root.join("library")
    }

    /// Every backup dir kept for provider `label`, newest last by name.
    fn backups(root: &Path, label: &str) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = fs::read_dir(lib(root).join(PROVIDER_BACKUP_DIR).join(label))
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        out.sort();
        out
    }

    /// No staging/swap sibling is ever left in a provider dir.
    fn assert_no_swap_leftovers(dir: &Path) {
        let leftovers: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.contains(".otto-new-") || n.contains(".otto-old-")
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn failed_new_install_does_not_leave_an_unowned_partial_tree() {
        let root = tempfile::tempdir().unwrap();
        let src = src_skill(root.path());
        let provider = root.path().join("provider");
        fs::create_dir_all(provider.join(".otto-managed.json")).unwrap();
        // The copy succeeds but ownership publication fails (manifest is a dir).
        let l = lib(root.path());
        assert!(install_into_dirs(std::slice::from_ref(&provider), &l, "grill", &src).is_err());
        assert!(!provider.join("grill").exists());
        fs::remove_dir(provider.join(".otto-managed.json")).unwrap();
        assert!(
            install_into_dirs(std::slice::from_ref(&provider), &l, "grill", &src)
                .unwrap()
                .user_owned
                .is_empty()
        );
        assert!(provider.join("grill/SKILL.md").is_file());
        assert_eq!(merge::read_manifest(&provider), ["grill"]);
        assert_no_swap_leftovers(&provider);
    }

    #[test]
    fn failure_in_another_provider_preserves_user_owned_content() {
        let root = tempfile::tempdir().unwrap();
        let src = src_skill(root.path());
        let mine = root.path().join("mine");
        fs::create_dir_all(mine.join("grill")).unwrap();
        fs::write(mine.join("grill/SKILL.md"), "user-authored").unwrap();
        let broken = root.path().join("broken");
        fs::create_dir_all(broken.join(".otto-managed.json")).unwrap();
        let dirs = [mine.clone(), broken.clone()];
        assert!(install_into_dirs(&dirs, &lib(root.path()), "grill", &src).is_err());
        assert_eq!(
            fs::read_to_string(mine.join("grill/SKILL.md")).unwrap(),
            "user-authored"
        );
        assert!(!broken.join("grill").exists());
        assert!(merge::read_manifest(&mine).is_empty());
        assert!(!lib(root.path()).join(PROVIDER_BACKUP_DIR).exists());
    }

    /// data-integrity-4-04: a malformed manifest is an error, never "all
    /// user-owned" (skip) and never overwritten with a one-entry list.
    #[test]
    fn malformed_manifest_refuses_instead_of_disowning() {
        let root = tempfile::tempdir().unwrap();
        let src = src_skill(root.path());
        let l = lib(root.path());
        let d = root.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        let other = root.path().join("src").join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("SKILL.md"), "# other\n").unwrap();
        fs::write(d.join(".otto-managed.json"), "{\"skills\": [\"gri").unwrap();
        let err = install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(install_into_dirs(std::slice::from_ref(&d), &l, "other", &other).is_err());
        assert!(!d.join("other").exists(), "new tree cleaned up");
        assert!(uninstall_from_dirs(std::slice::from_ref(&d), &l, "grill").is_err());
        assert!(d.join("grill/SKILL.md").is_file(), "nothing removed");
        assert_eq!(
            fs::read_to_string(d.join(".otto-managed.json")).unwrap(),
            "{\"skills\": [\"gri",
            "a malformed manifest is never rewritten"
        );
    }

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

        let report = install_into_dirs(&dirs, &lib(tmp.path()), "grill", &src).unwrap();
        assert_eq!(report.written, dirs);
        assert!(report.kept.is_empty());

        for d in [&d1, &d2] {
            assert!(d.join("grill/SKILL.md").is_file());
            assert!(d.join("grill/references/notes.md").is_file());
            assert_eq!(merge::read_manifest(d), vec!["grill".to_string()]);
            assert_eq!(
                copies::recorded_hash(d, "grill"),
                Some(tree_hash(&src).unwrap()),
                "what Otto wrote is recorded"
            );
        }
    }

    /// A file added to Otto's copy is an edit: the re-install still replaces
    /// the tree cleanly, but the edited tree is kept aside first.
    #[test]
    fn update_clean_overwrites_and_manifest_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let d = tmp.path().join("claude").join("skills");
        let dirs = vec![d.clone()];
        let l = lib(tmp.path());

        install_into_dirs(&dirs, &l, "grill", &src).unwrap();
        // A stale file present before re-install must be gone after.
        fs::write(d.join("grill/STALE.md"), "x").unwrap();
        let report = install_into_dirs(&dirs, &l, "grill", &src).unwrap();

        assert!(!d.join("grill/STALE.md").exists());
        assert!(d.join("grill/SKILL.md").is_file());
        assert_eq!(merge::read_manifest(&d), vec!["grill".to_string()]);
        assert_eq!(report.kept.len(), 1);
        let kept = PathBuf::from(&report.kept[0].kept_at);
        assert_eq!(fs::read_to_string(kept.join("STALE.md")).unwrap(), "x");
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

        let report = install_into_dirs(
            &[mine.clone(), linked.clone(), fresh.clone()],
            &lib(tmp.path()),
            "grill",
            &src,
        )
        .unwrap();
        let skipped = &report.user_owned;

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
            // Uninstall drops the claim but leaves the person's link alone.
            assert!(
                uninstall_from_dirs(std::slice::from_ref(&linked), &lib(tmp.path()), "grill")
                    .unwrap()
                    .is_empty()
            );
            assert!(fs::symlink_metadata(linked.join("grill")).is_ok());
            assert!(repo.join("SKILL.md").is_file());
        }
        assert!(skipped.contains(&mine));
        assert!(fresh.join("grill/references/notes.md").is_file());
        assert!(!skipped.contains(&fresh));
        assert!(
            report.kept.is_empty(),
            "nothing Otto does not own is backed up"
        );
        assert!(!lib(tmp.path()).join(PROVIDER_BACKUP_DIR).exists());
    }

    /// A Library edit reaches the copies Otto installed, and only those: a
    /// user-owned copy, a symlink and a dir with no copy are left as they are.
    #[test]
    fn refresh_updates_owned_copies_and_never_unowned_ones() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let owned = tmp.path().join("claude");
        let mine = tmp.path().join("codex");
        let empty = tmp.path().join("gemini");
        install_into_dirs(std::slice::from_ref(&owned), &l, "grill", &src).unwrap();
        fs::create_dir_all(mine.join("grill")).unwrap();
        fs::write(mine.join("grill/SKILL.md"), "my own grill\n").unwrap();
        fs::create_dir_all(&empty).unwrap();

        // The person edits the Library copy (and adds a file).
        fs::write(src.join("SKILL.md"), "---\nversion: 2\n---\n# edited\n").unwrap();
        fs::write(src.join("references/new.md"), "added\n").unwrap();
        let report = refresh_owned_in_dirs(
            &[owned.clone(), mine.clone(), empty.clone()],
            &l,
            "grill",
            &src,
        )
        .unwrap();

        assert_eq!(report.written, vec![owned.clone()]);
        assert_eq!(
            fs::read_to_string(owned.join("grill/SKILL.md")).unwrap(),
            "---\nversion: 2\n---\n# edited\n"
        );
        assert!(owned.join("grill/references/new.md").is_file());
        assert_eq!(
            fs::read_to_string(mine.join("grill/SKILL.md")).unwrap(),
            "my own grill\n"
        );
        assert!(!empty.join("grill").exists(), "no copy is ever created");
        assert_no_swap_leftovers(&owned);
    }

    /// An owned copy the person deleted by hand is not brought back.
    #[test]
    fn refresh_does_not_resurrect_a_removed_owned_copy() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        fs::remove_dir_all(d.join("grill")).unwrap();
        assert!(
            refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src)
                .unwrap()
                .written
                .is_empty()
        );
        assert!(!d.join("grill").exists());
    }

    /// A copy that fails midway keeps the previous provider tree whole.
    #[test]
    fn failed_refresh_keeps_the_previous_copy() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        let missing = tmp.path().join("no-such-source");
        assert!(refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &missing).is_err());
        assert!(d.join("grill/SKILL.md").is_file());
        assert!(d.join("grill/references/notes.md").is_file());
    }

    #[test]
    fn uninstall_only_removes_managed_skills() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude").join("skills");
        let dirs = vec![d.clone()];
        install_into_dirs(&dirs, &l, "grill", &src).unwrap();

        // A user-authored skill Otto never installed sits alongside it.
        fs::create_dir_all(d.join("mine")).unwrap();
        fs::write(d.join("mine/SKILL.md"), "mine\n").unwrap();

        assert!(uninstall_from_dirs(&dirs, &l, "grill").unwrap().is_empty());
        assert!(!d.join("grill").exists());
        assert!(merge::read_manifest(&d).is_empty());
        assert_eq!(copies::recorded_hash(&d, "grill"), None);
        // The user-authored skill is untouched.
        assert!(d.join("mine/SKILL.md").is_file());

        // Uninstalling a name Otto doesn't own is a no-op that preserves it.
        uninstall_from_dirs(&dirs, &l, "mine").unwrap();
        assert!(d.join("mine/SKILL.md").is_file());
        assert!(
            !l.join(PROVIDER_BACKUP_DIR).exists(),
            "an untouched copy needs no backup"
        );
    }

    /// The defect this guards: a Skills Lab save replaced the person's edit in
    /// `~/.claude/skills/<name>` with no copy left anywhere.
    #[test]
    fn refresh_backs_up_a_hand_edited_owned_copy() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join(".claude").join("skills");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        fs::write(d.join("grill/SKILL.md"), "edited in the CLI folder\n").unwrap();

        fs::write(src.join("SKILL.md"), "saved in Skills Lab\n").unwrap();
        let report = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();

        assert_eq!(
            fs::read_to_string(d.join("grill/SKILL.md")).unwrap(),
            "saved in Skills Lab\n"
        );
        assert_eq!(report.kept.len(), 1);
        let kept = &report.kept[0];
        assert_eq!(PathBuf::from(&kept.copy), d.join("grill"));
        let at = PathBuf::from(&kept.kept_at);
        assert!(
            at.starts_with(l.join(PROVIDER_BACKUP_DIR).join("claude")),
            "{at:?}"
        );
        assert!(at
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("grill-"));
        assert_eq!(
            fs::read_to_string(at.join("SKILL.md")).unwrap(),
            "edited in the CLI folder\n"
        );
        assert!(
            at.join("references/notes.md").is_file(),
            "the whole tree is kept"
        );
        assert!(
            !l.join("skills-backup").exists(),
            "never the Library's own backup folder"
        );
        // The new copy is recorded: the next save makes no second backup.
        fs::write(src.join("SKILL.md"), "saved again\n").unwrap();
        let again = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert!(again.kept.is_empty());
        assert_eq!(backups(tmp.path(), "claude").len(), 1);
        assert_no_swap_leftovers(&d);
    }

    #[test]
    fn refresh_of_an_untouched_copy_makes_no_backup() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        // Finder's `.DS_Store` in the copy is not an edit either.
        fs::write(d.join("grill/.DS_Store"), "finder").unwrap();
        fs::write(src.join("SKILL.md"), "v2\n").unwrap();
        let report = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert!(report.kept.is_empty());
        assert_eq!(
            fs::read_to_string(d.join("grill/SKILL.md")).unwrap(),
            "v2\n"
        );
        assert!(!l.join(PROVIDER_BACKUP_DIR).exists());
        // Unchanged Library: the copy already matches and is not rewritten.
        let before = fs::metadata(d.join("grill/SKILL.md"))
            .unwrap()
            .modified()
            .unwrap();
        let same = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert!(same.kept.is_empty());
        assert_eq!(
            fs::metadata(d.join("grill/SKILL.md"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
    }

    #[test]
    fn uninstall_backs_up_hand_edit() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("gemini").join("skills");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        fs::write(d.join("grill/references/notes.md"), "my notes\n").unwrap();

        let kept = uninstall_from_dirs(std::slice::from_ref(&d), &l, "grill").unwrap();

        assert!(!d.join("grill").exists());
        assert!(merge::read_manifest(&d).is_empty());
        assert_eq!(kept.len(), 1);
        let at = PathBuf::from(&kept[0].kept_at);
        assert!(at.starts_with(l.join(PROVIDER_BACKUP_DIR).join("gemini")));
        assert_eq!(
            fs::read_to_string(at.join("references/notes.md")).unwrap(),
            "my notes\n"
        );
    }

    /// The install overwrite is a swap: a copy that cannot be made leaves the
    /// previous tree (here a hand-edited one) exactly where it was.
    #[test]
    fn failed_install_overwrite_keeps_previous_tree() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        fs::write(d.join("grill/SKILL.md"), "edited\n").unwrap();
        let missing = tmp.path().join("no-such-source");

        assert!(install_into_dirs(std::slice::from_ref(&d), &l, "grill", &missing).is_err());

        assert_eq!(
            fs::read_to_string(d.join("grill/SKILL.md")).unwrap(),
            "edited\n"
        );
        assert!(d.join("grill/references/notes.md").is_file());
        assert_eq!(merge::read_manifest(&d), ["grill"]);
        assert_no_swap_leftovers(&d);
        // The source is fine but the staged copy cannot be written (the
        // provider dir is read-only): still nothing moves, nothing is kept.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&d, fs::Permissions::from_mode(0o555)).unwrap();
            let failed = install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src);
            fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(failed.is_err());
            assert!(!l.join(PROVIDER_BACKUP_DIR).exists());
            assert_eq!(
                fs::read_to_string(d.join("grill/SKILL.md")).unwrap(),
                "edited\n"
            );
            assert_no_swap_leftovers(&d);
        }
    }

    #[test]
    fn backups_are_pruned_to_three_per_provider_and_name() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("codex").join("skills");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        // Another skill whose name starts with this one keeps its backup.
        let label = l.join(PROVIDER_BACKUP_DIR).join("codex");
        let other = label.join("grill-2-100-01ARZ3NDEKTSV4RRFFQ69G5FAV");
        fs::create_dir_all(&other).unwrap();

        let mut last = None;
        for round in 0..5 {
            fs::write(d.join("grill/SKILL.md"), format!("hand edit {round}\n")).unwrap();
            fs::write(src.join("SKILL.md"), format!("library {round}\n")).unwrap();
            let report =
                refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
            assert_eq!(report.kept.len(), 1, "round {round}");
            last = Some(PathBuf::from(&report.kept[0].kept_at));
        }

        let mine: Vec<_> = backups(tmp.path(), "codex")
            .into_iter()
            .filter(|p| *p != other)
            .collect();
        assert_eq!(mine.len(), PROVIDER_BACKUPS_KEPT, "{mine:?}");
        let last = last.unwrap();
        assert!(mine.contains(&last), "the newest backup is never pruned");
        assert_eq!(
            fs::read_to_string(last.join("SKILL.md")).unwrap(),
            "hand edit 4\n"
        );
        assert!(other.is_dir());
    }

    /// A copy an older Otto installed has no hash record: it counts as edited
    /// only when it differs from what is about to be written.
    #[test]
    fn a_copy_with_no_hash_record_is_backed_up_when_it_differs() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        let d = tmp.path().join("claude");
        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        fs::remove_file(d.join(".otto-managed-hashes.json")).unwrap();

        // Same content as the Library: nothing to keep.
        let same = install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert!(same.kept.is_empty());
        fs::remove_file(d.join(".otto-managed-hashes.json")).unwrap();

        // Different content and no record: kept.
        fs::write(src.join("SKILL.md"), "newer library\n").unwrap();
        let report = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert_eq!(report.kept.len(), 1);
        let at = PathBuf::from(&report.kept[0].kept_at);
        assert_eq!(
            fs::read_to_string(at.join("SKILL.md")).unwrap(),
            "---\nversion: 2\n---\n# grill\n"
        );
        assert_eq!(
            fs::read_to_string(d.join("grill/SKILL.md")).unwrap(),
            "newer library\n"
        );
    }

    /// The improvement engine's `.bak-*` trail and the Library's write temps
    /// stay in the Library; a dot-file the skill ships is copied.
    #[test]
    fn copy_skips_improvement_backups() {
        let tmp = TempDir::new().unwrap();
        let src = src_skill(tmp.path());
        let l = lib(tmp.path());
        fs::write(src.join(".SKILL.md.bak-20261113T101500.123456"), "old").unwrap();
        fs::write(
            src.join("references/.notes.md.bak-20261113T101501.000001"),
            "old",
        )
        .unwrap();
        fs::write(src.join(".SKILL.md.otto-tmp-42-0"), "temp").unwrap();
        fs::write(src.join(".gitignore"), "out/\n").unwrap();
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(src.join("assets/.gitkeep"), "").unwrap();
        let d = tmp.path().join("claude");

        install_into_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();

        let copy = d.join("grill");
        assert!(!copy.join(".SKILL.md.bak-20261113T101500.123456").exists());
        assert!(!copy
            .join("references/.notes.md.bak-20261113T101501.000001")
            .exists());
        assert!(!copy.join(".SKILL.md.otto-tmp-42-0").exists());
        assert!(copy.join(".gitignore").is_file());
        assert!(copy.join("assets/.gitkeep").is_file());
        // The hash covers exactly what is copied.
        assert_eq!(tree_hash(&src).unwrap(), tree_hash(&copy).unwrap());

        // A copy an older Otto filled with `.bak` files is cleaned on the next
        // write, with no backup: those files are Otto's, not an edit.
        fs::write(copy.join(".SKILL.md.bak-20261101T000000.000000"), "old").unwrap();
        let report = refresh_owned_in_dirs(std::slice::from_ref(&d), &l, "grill", &src).unwrap();
        assert!(report.kept.is_empty());
        assert!(!copy.join(".SKILL.md.bak-20261101T000000.000000").exists());
    }

    #[test]
    fn otto_artifact_names_are_exact() {
        use std::ffi::OsStr;
        for skipped in [
            ".SKILL.md.bak-20261113T101500.123456",
            ".x.bak-1",
            ".SKILL.md.otto-tmp-1-2",
            ".DS_Store",
        ] {
            assert!(copies::is_skipped(OsStr::new(skipped)), "{skipped}");
        }
        for kept in [
            ".gitignore",
            ".gitkeep",
            "notes.bak-2026",
            ".bak-2026",
            ".config.bak-old",
            "SKILL.md",
        ] {
            assert!(!copies::is_skipped(OsStr::new(kept)), "{kept}");
        }
    }
}

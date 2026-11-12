//! Swapping staged skill packages into a SHARED bundle safely. Every review
//! run (and every per-agent retry) re-stages the whole skill library into one
//! fixed `~/.otto/context/review-skills`, which running Claude reviewers load
//! through `--add-dir`. Staging used to delete each package and copy it back
//! in place with no lock: two reviews starting together failed each other's
//! copies ("Directory not empty", "No such file") while the first review's
//! reviewers were starting, and a reviewer could lose its lens skill mid-run.
//!
//! Now a package is built complete in a scratch directory beside the bundle
//! and moved into place by rename under one process-wide lock, and only when
//! its content changed — so a running reviewer sees either the old complete
//! package or the new complete one, and in the usual case (nothing changed)
//! nothing at all. The server keeps the source lookup and copy
//! (`otto-server` `modules::skill_staging`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serialises every swap into any bundle in this process (the daemon is the
/// only writer). Held only for the compare + two renames per target.
static SWAP_LOCK: Mutex<()> = Mutex::new(());

/// A private scratch directory inside `bundle` (created if missing) — the
/// same filesystem, so the final move is a rename. Removed when dropped,
/// together with any package version swapped out into it. Its dot-prefixed
/// name sits beside `skills/` and `.claude/`, never inside a skills dir a
/// CLI scans.
pub fn scratch_in(bundle: &Path) -> std::io::Result<tempfile::TempDir> {
    std::fs::create_dir_all(bundle)?;
    tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(bundle)
}

/// Install the complete package `fresh` (a directory inside `scratch`) at
/// every one of `targets` (the neutral and the Claude views). A target whose
/// content already equals `fresh` is left untouched. Otherwise a full copy is
/// made in `scratch` first, then, under [`SWAP_LOCK`], the live target is
/// renamed out into `scratch` and the copy renamed in. A symlink at a target
/// is moved away like a directory, never followed.
#[allow(clippy::disallowed_methods)] // sync by contract: staging runs on the blocking pool
pub fn install(fresh: &Path, scratch: &Path, targets: &[PathBuf]) -> Result<(), String> {
    for (i, target) in targets.iter().enumerate() {
        if same_tree(fresh, target) {
            continue;
        }
        let incoming = scratch.join(format!("in-{i}"));
        copy_tree(fresh, &incoming)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let outgoing = scratch.join(format!("out-{i}"));
        let _held = SWAP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Re-checked under the lock: a concurrent stager may just have
        // installed this very content.
        if same_tree(fresh, target) {
            continue;
        }
        match std::fs::rename(target, &outgoing) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("move out {}: {e}", target.display())),
        }
        if let Err(e) = std::fs::rename(&incoming, target) {
            // Put the previous version back rather than leave a hole.
            let _ = std::fs::rename(&outgoing, target);
            return Err(format!("move in {}: {e}", target.display()));
        }
    }
    Ok(())
}

/// Whether `b` is a real directory holding exactly `a`'s files, byte for
/// byte. Any error (missing, symlink, unreadable) reads as "differs".
#[allow(clippy::disallowed_methods)] // sync by contract: staging runs on the blocking pool
fn same_tree(a: &Path, b: &Path) -> bool {
    fn entries(dir: &Path) -> Option<Vec<(std::ffi::OsString, std::fs::FileType)>> {
        let mut out = std::fs::read_dir(dir)
            .ok()?
            .map(|e| {
                e.ok()
                    .and_then(|e| Some((e.file_name(), e.file_type().ok()?)))
            })
            .collect::<Option<Vec<_>>>()?;
        out.sort_by(|x, y| x.0.cmp(&y.0));
        Some(out)
    }
    let real_dir = |p: &Path| {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    };
    if !real_dir(a) || !real_dir(b) {
        return false;
    }
    let (Some(ea), Some(eb)) = (entries(a), entries(b)) else {
        return false;
    };
    ea.len() == eb.len()
        && ea.iter().zip(&eb).all(|((na, ta), (nb, tb))| {
            na == nb
                && if ta.is_dir() && tb.is_dir() {
                    same_tree(&a.join(na), &b.join(nb))
                } else if ta.is_file() && tb.is_file() {
                    matches!(
                        (std::fs::read(a.join(na)), std::fs::read(b.join(nb))),
                        (Ok(x), Ok(y)) if x == y
                    )
                } else {
                    false
                }
        })
}

/// Copy a staged package (plain directories and files only — it was built
/// by the stager, which resolves links) into `dest`, which must not exist.
#[allow(clippy::disallowed_methods)] // sync by contract: staging runs on the blocking pool
fn copy_tree(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (from, to) = (entry.path(), dest.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain threads, no runtime
mod tests {
    use super::*;

    fn package(root: &Path, body: &str) -> PathBuf {
        let p = root.join("package");
        std::fs::create_dir_all(p.join("references")).unwrap();
        std::fs::write(p.join("SKILL.md"), body).unwrap();
        std::fs::write(p.join("references/rubric.md"), format!("rubric for {body}")).unwrap();
        p
    }

    fn read_pair(target: &Path) -> (String, String) {
        (
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            std::fs::read_to_string(target.join("references/rubric.md")).unwrap(),
        )
    }

    #[test]
    fn installs_both_views_and_leaves_an_unchanged_package_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("bundle");
        let targets = [
            bundle.join("skills/lens"),
            bundle.join(".claude/skills/lens"),
        ];
        let scratch = scratch_in(&bundle).unwrap();
        let fresh = package(scratch.path(), "v1");
        install(&fresh, scratch.path(), &targets).unwrap();
        drop(scratch);
        for t in &targets {
            assert_eq!(read_pair(t), ("v1".into(), "rubric for v1".into()));
        }
        // The scratch dir (and the swapped-out versions in it) is gone.
        let leftovers: Vec<_> = std::fs::read_dir(&bundle)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
            .collect();
        assert!(leftovers.is_empty());

        // Same content again: the live directory is not replaced (a running
        // reviewer's view is not even touched).
        let before = std::fs::metadata(&targets[1]).unwrap().modified().unwrap();
        let scratch = scratch_in(&bundle).unwrap();
        let fresh = package(scratch.path(), "v1");
        install(&fresh, scratch.path(), &targets).unwrap();
        assert_eq!(
            std::fs::metadata(&targets[1]).unwrap().modified().unwrap(),
            before
        );
        assert!(same_tree(&fresh, &targets[1]));
        // A symlink planted at a target is moved away, never followed.
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), "keep").unwrap();
        std::fs::remove_dir_all(&targets[0]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &targets[0]).unwrap();
        install(&fresh, scratch.path(), &targets).unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "keep"
        );
        assert!(same_tree(&fresh, &targets[0]));
    }

    /// Two reviews staging the shared bundle at once (the 2026-11-12 log:
    /// "Directory not empty", "No such file"): every staging succeeds, and
    /// the package a reader sees is always one complete version — never a
    /// half-copied or missing one.
    #[test]
    fn concurrent_stagings_never_fail_or_expose_a_partial_package() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("bundle");
        let targets = vec![
            bundle.join("skills/lens"),
            bundle.join(".claude/skills/lens"),
        ];
        {
            let scratch = scratch_in(&bundle).unwrap();
            install(&package(scratch.path(), "v0"), scratch.path(), &targets).unwrap();
        }
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (target, stop) = (targets[1].clone(), stop.clone());
            std::thread::spawn(move || {
                let mut seen = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    // The swap is two renames; a read can fall between them
                    // (not found) but must never see mixed versions.
                    let skill = std::fs::read_to_string(target.join("SKILL.md"));
                    let rubric = std::fs::read_to_string(target.join("references/rubric.md"));
                    if let (Ok(s), Ok(r)) = (skill, rubric) {
                        assert!(["v0", "a", "b"].contains(&s.as_str()), "{s}");
                        assert!(r.starts_with("rubric for "), "{r}");
                        seen += 1;
                    }
                }
                seen
            })
        };
        let stagers: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|body| {
                let (bundle, targets) = (bundle.clone(), targets.clone());
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        let scratch = scratch_in(&bundle).unwrap();
                        let fresh = package(scratch.path(), body);
                        install(&fresh, scratch.path(), &targets).unwrap();
                    }
                })
            })
            .collect();
        for s in stagers {
            s.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        for t in &targets {
            let (skill, rubric) = read_pair(t);
            assert!(skill == "a" || skill == "b");
            assert_eq!(rubric, format!("rubric for {skill}"));
        }
    }
}

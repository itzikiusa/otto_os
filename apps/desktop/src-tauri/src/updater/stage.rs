//! Staging: unpack the verified archive into a private (0700) directory
//! OUTSIDE /Applications (App Management only ever sees the final swap), then
//! check the signature of every binary against its OWN designated requirement
//! and the version the manifest announced (D-014, D-015).

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// The bundle the release archive carries, and its identifier. Tests use an
/// inert name and id, so nothing they build can be mistaken for Otto (or
/// open `otto://` links): see `testkit`.
#[cfg(not(test))]
pub const BUNDLE: &str = "Otto.app";
#[cfg(not(test))]
const APP_ID: &str = "com.otto.app";
#[cfg(test)]
pub const BUNDLE: &str = "OttoUpdateTest.app";
#[cfg(test)]
pub const APP_ID: &str = "com.otto.updatetest";
/// The bundle's other signed binaries: (identifier, path, required).
const BINARIES: [(&str, &str, bool); 2] = [
    ("ottod", "Contents/MacOS/ottod", true),
    ("kiri", "Contents/MacOS/kiri", false),
];
/// Unpacked size and entry-count ceilings (the bundle is ~312 MB, a few
/// hundred entries): a decompression bomb stops here.
const UNPACKED_MAX: u64 = 2 * 1024 * 1024 * 1024;
const ENTRIES_MAX: usize = 50_000;
/// Headroom on top of the measured need.
const DISK_MARGIN: u64 = 256 * 1024 * 1024;

/// `<updates>/staging/<BUNDLE>`, ready for the finisher's swap.
pub fn staged_app(updates: &Path) -> PathBuf {
    updates.join("staging").join(BUNDLE)
}

/// Create `dir` (and parents) and make it 0700.
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// Unpack `archive` (gzip tar, one top-level [`BUNDLE`]) into a fresh
/// staging dir and verify it. Leaves nothing behind on failure.
pub fn stage(
    archive: &[u8],
    updates: &Path,
    version: &str,
    signers: &[String],
) -> Result<PathBuf, String> {
    let root = updates.join("staging");
    let _ = fs::remove_dir_all(&root);
    private_dir(updates)
        .and_then(|_| private_dir(&root))
        .map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let app = staged_app(updates);
    let res = extract(archive, &root).and_then(|_| {
        // Not `is_dir()`: that follows a top-level link.
        if fs::symlink_metadata(&app).is_ok_and(|m| m.is_dir()) {
            verify(&app, version, signers)
        } else {
            Err(format!("the archive has no {BUNDLE} at its top"))
        }
    });
    match res {
        Ok(()) => Ok(app),
        Err(e) => {
            let _ = fs::remove_dir_all(&root);
            Err(e)
        }
    }
}

/// Unpack every entry under `dest`, refusing absolute paths, `..`, hard
/// links, devices, anything outside the bundle and symlinks whose target
/// leaves the bundle. No xattrs are written (so no quarantine either), and
/// setuid/setgid bits are dropped.
pub fn extract(archive: &[u8], dest: &Path) -> Result<(), String> {
    extract_within(archive, dest, UNPACKED_MAX)
}

/// [`extract`] with `max` unpacked bytes. The decompressed stream itself is
/// bounded too (contents + one header block per entry), so a bomb stops
/// there even before an entry claims its size.
fn extract_within(archive: &[u8], dest: &Path, max: u64) -> Result<(), String> {
    use std::io::Read;
    let stream_max = max + (ENTRIES_MAX as u64 + 2) * 1024;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(archive).take(stream_max));
    let (mut total, mut count) = (0u64, 0usize);
    for entry in ar
        .entries()
        .map_err(|e| format!("unreadable archive: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("unreadable archive entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("bad entry path: {e}"))?
            .into_owned();
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        if !inside_bundle(&path) {
            return Err(format!(
                "the archive has an entry outside {BUNDLE}: {}",
                path.display()
            ));
        }
        if kind.is_symlink()
            && path
                .components()
                .filter(|c| !matches!(c, Component::CurDir))
                .count()
                == 1
        {
            return Err(format!("the archive's {BUNDLE} is a link"));
        }
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| format!("bad link in {}: {e}", path.display()))?
                .ok_or_else(|| format!("link without a target: {}", path.display()))?;
            if !link_stays_inside(&path, &target) {
                return Err(format!(
                    "the archive has a link leaving {BUNDLE}: {} -> {}",
                    path.display(),
                    target.display()
                ));
            }
        } else if !(kind.is_file() || kind.is_dir()) {
            return Err(format!(
                "the archive has an unsupported entry ({kind:?}): {}",
                path.display()
            ));
        }
        total = total.saturating_add(entry.size());
        count += 1;
        if total > max || count > ENTRIES_MAX {
            return Err("the archive unpacks to more than Otto can be".into());
        }
        match entry.unpack_in(dest) {
            Ok(true) => {}
            Ok(false) => return Err(format!("refused entry {}", path.display())),
            Err(e) => return Err(format!("could not unpack {}: {e}", path.display())),
        }
    }
    Ok(())
}

/// A relative path made of plain names whose first one is [`BUNDLE`].
fn inside_bundle(path: &Path) -> bool {
    let mut parts = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir));
    parts.next() == Some(Component::Normal(BUNDLE.as_ref()))
        && parts.all(|c| matches!(c, Component::Normal(_)))
}

/// A relative link target that, resolved from the link's own directory,
/// never climbs out of the bundle.
fn link_stays_inside(link: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth: Vec<_> = link
        .parent()
        .map(|p| {
            p.components()
                .filter(|c| !matches!(c, Component::CurDir))
                .collect()
        })
        .unwrap_or_default();
    for c in target.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                depth.pop();
                if depth.is_empty() {
                    return false;
                }
            }
            Component::Normal(_) => depth.push(c),
            _ => return false,
        }
    }
    !depth.is_empty()
}

/// `identifier "<id>" and (<signer> or <signer> …)`.
pub fn requirement(id: &str, signers: &[String]) -> String {
    format!("identifier \"{id}\" and ({})", signers.join(" or "))
}

/// Every binary satisfies its own designated requirement with an accepted
/// signer, the bundle seal is intact, and the bundle carries `version`.
pub fn verify(app: &Path, version: &str, signers: &[String]) -> Result<(), String> {
    if signers.is_empty() {
        return Err("no accepted signing identity is configured".into());
    }
    codesign(app, true, &requirement(APP_ID, signers))?;
    for (id, rel, required) in BINARIES {
        let bin = app.join(rel);
        if bin.exists() {
            codesign(&bin, false, &requirement(id, signers))?;
        } else if required {
            return Err(format!("the update has no {rel}"));
        }
    }
    let got = bundle_version(app)?;
    if got != version {
        return Err(format!(
            "the update says {version} but the app inside is {got}"
        ));
    }
    Ok(())
}

fn codesign(path: &Path, deep: bool, req: &str) -> Result<(), String> {
    let mut cmd = Command::new("/usr/bin/codesign");
    cmd.args(["--verify", "--strict"]);
    if deep {
        cmd.arg("--deep");
    }
    let out = cmd
        .arg(format!("-R={req}"))
        .arg(path)
        .output()
        .map_err(|e| format!("could not run codesign: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    Err(format!(
        "{name} is not signed by an accepted identity: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// `CFBundleShortVersionString` of a bundle.
pub fn bundle_version(app: &Path) -> Result<String, String> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print CFBundleShortVersionString"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .map_err(|e| format!("could not read the bundle version: {e}"))?;
    if !out.status.success() {
        return Err("the update has no readable version".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Bytes the update will write: the staged bundle plus the daemon the new
/// app's supervisor installs (~94 % of the bundle), measured from the running
/// bundle, plus a margin. The old app moves to the rollback copy, which only
/// replaces the one before it.
pub fn needed_bytes(current_app: &Path) -> u64 {
    2 * tree_size(current_app).max(400 * 1024 * 1024) + DISK_MARGIN
}

fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|it| it.flatten().map(|e| tree_size(&e.path())).sum())
        .unwrap_or(0)
}

/// Free bytes on the volume holding `path`.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a writable statfs.
    (unsafe { libc::statfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail * st.f_bsize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::testkit::{tar_gz, Kit, TarEntry};

    #[test]
    fn paths_must_stay_in_the_bundle() {
        assert!(inside_bundle(Path::new("OttoUpdateTest.app")));
        assert!(inside_bundle(Path::new(
            "./OttoUpdateTest.app/Contents/MacOS/ottod"
        )));
        assert!(!inside_bundle(Path::new("Other.app/x")));
        assert!(!inside_bundle(Path::new("/OttoUpdateTest.app/x")));
        assert!(!inside_bundle(Path::new("OttoUpdateTest.app/../x")));
        assert!(!inside_bundle(Path::new("x")));
        let l = Path::new("OttoUpdateTest.app/Contents/Frameworks/A.framework/A");
        assert!(link_stays_inside(l, Path::new("Versions/Current/A")));
        assert!(link_stays_inside(l, Path::new("../../MacOS/ottod")));
        assert!(!link_stays_inside(l, Path::new("../../../../etc/passwd")));
        assert!(!link_stays_inside(l, Path::new("/etc/passwd")));
        assert!(!link_stays_inside(
            Path::new("OttoUpdateTest.app/l"),
            Path::new("..")
        ));
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("otto-upd-stage-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hostile_archives_are_refused_and_leave_nothing() {
        let cases: Vec<(&str, Vec<TarEntry>, &str)> = vec![
            // C1-028: a "./"-prefixed link climbing to the bundle's parent,
            // and the bundle itself as a link.
            (
                "dot-link",
                vec![
                    TarEntry::dir("./OttoUpdateTest.app"),
                    TarEntry::link("./OttoUpdateTest.app/l", ".."),
                ],
                "link leaving",
            ),
            (
                "top-link",
                vec![TarEntry::link("OttoUpdateTest.app", "elsewhere")],
                "is a link",
            ),
            (
                "dotdot",
                vec![TarEntry::file("OttoUpdateTest.app/../../evil", b"x")],
                "outside OttoUpdateTest.app",
            ),
            (
                "abs",
                vec![TarEntry::file("/tmp/evil", b"x")],
                "outside OttoUpdateTest.app",
            ),
            (
                "sibling",
                vec![TarEntry::file("Evil.app/x", b"x")],
                "outside OttoUpdateTest.app",
            ),
            (
                "link-out",
                vec![
                    TarEntry::dir("OttoUpdateTest.app"),
                    TarEntry::link("OttoUpdateTest.app/l", "../../../etc"),
                ],
                "link leaving",
            ),
            (
                "link-abs",
                vec![TarEntry::link("OttoUpdateTest.app/l", "/etc")],
                "link leaving",
            ),
            (
                "hardlink",
                vec![TarEntry::hard(
                    "OttoUpdateTest.app/h",
                    "OttoUpdateTest.app/x",
                )],
                "unsupported",
            ),
        ];
        for (tag, entries, want) in cases {
            let dir = tmp(tag);
            let err = stage(&tar_gz(&entries), &dir, "1.0.0", &["x".into()]).unwrap_err();
            assert!(err.contains(want), "{tag}: {err}");
            assert!(!dir.join("staging").exists(), "{tag}: staging left behind");
            assert!(!std::env::temp_dir().join("evil").exists());
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// C1-028: the unpacked-size ceiling holds for one big entry and for
    /// the decompressed stream (tested with a small ceiling).
    #[test]
    fn an_archive_bigger_than_the_ceiling_is_refused() {
        let dir = tmp("bomb");
        let big = tar_gz(&[TarEntry::file(
            "OttoUpdateTest.app/big",
            &vec![0u8; 256 * 1024],
        )]);
        assert!(extract_within(&big, &dir, 64 * 1024).is_err());
        assert!(extract_within(&big, &dir, 1024 * 1024).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_staging_dir_is_private() {
        let dir = tmp("mode");
        let _ = stage(
            &tar_gz(&[TarEntry::file("OttoUpdateTest.app/x", b"x")]),
            &dir,
            "1.0.0",
            &["x".into()],
        );
        let _ = private_dir(&dir.join("staging"));
        for d in [dir.clone(), dir.join("staging")] {
            assert_eq!(
                fs::metadata(&d).unwrap().permissions().mode() & 0o777,
                0o700,
                "{}",
                d.display()
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_need_follows_the_real_bundle_size() {
        let dir = tmp("disk");
        fs::write(dir.join("a"), vec![0u8; 1024]).unwrap();
        assert_eq!(tree_size(&dir), 1024);
        // A small bundle still budgets for a real one (400 MB floor).
        assert_eq!(needed_bytes(&dir), 2 * 400 * 1024 * 1024 + DISK_MARGIN);
        assert!(free_bytes(&dir).unwrap() > 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The real thing: bundles signed by TEST identities in a temp keychain,
    /// archived the way the release does it, unpacked and verified.
    #[test]
    fn a_signed_release_archive_stages_and_each_binary_is_checked() {
        let kit = Kit::new("stage");
        let a = kit.identity("Otto Updater TEST A");
        let b = kit.identity("Otto Updater TEST B");
        let ok = kit.bundle("ok", "1.2.3", "Otto Updater TEST A", true);
        let dir = tmp("signed");
        let signers = vec![format!("certificate leaf = H\"{a}\"")];

        // Happy path: bundle, ottod and kiri all verify; the version matches.
        let app = stage(&kit.archive(&ok), &dir, "1.2.3", &signers).expect("stages");
        assert_eq!(bundle_version(&app).unwrap(), "1.2.3");

        // The manifest's version must be the app's.
        let err = stage(&kit.archive(&ok), &dir, "1.2.4", &signers).unwrap_err();
        assert!(err.contains("1.2.4") && err.contains("1.2.3"), "{err}");

        // Another identity is refused…
        let other = kit.bundle("other", "1.2.3", "Otto Updater TEST B", false);
        let err = stage(&kit.archive(&other), &dir, "1.2.3", &signers).unwrap_err();
        assert!(err.contains("not signed by an accepted identity"), "{err}");
        // …unless the allowlist names it (a later identity change).
        let both = vec![signers[0].clone(), format!("certificate leaf = H\"{b}\"")];
        stage(&kit.archive(&other), &dir, "1.2.3", &both).expect("allowlisted identity");

        // A sidecar signed by another identity inside an A-signed bundle is
        // caught by ITS OWN requirement.
        let mixed = kit.bundle_mixed(
            "mixed",
            "1.2.3",
            "Otto Updater TEST A",
            "Otto Updater TEST B",
        );
        let err = stage(&kit.archive(&mixed), &dir, "1.2.3", &signers).unwrap_err();
        assert!(err.contains("not signed by an accepted identity"), "{err}");

        // A byte changed after signing breaks the seal.
        let tampered = kit.bundle("tampered", "1.2.3", "Otto Updater TEST A", false);
        fs::write(tampered.join("Contents/Resources/extra.txt"), b"added").unwrap();
        assert!(stage(&kit.archive(&tampered), &dir, "1.2.3", &signers).is_err());

        // No configured signer fails closed.
        assert!(stage(&kit.archive(&ok), &dir, "1.2.3", &[])
            .unwrap_err()
            .contains("no accepted signing identity"));
        let _ = fs::remove_dir_all(&dir);
    }
}

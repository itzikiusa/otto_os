//! otto-skills — Otto's bundled, versioned skill library.
//!
//! Skills are organized by category (`product | project | development | review |
//! design`) under `assets/skills/<category>/<name>/` and embedded into the binary.
//!
//! Unlike `otto-product` (which auto-seeds its lenses into the Library on daemon
//! start), these skills are **never auto-installed**. They are surfaced in
//! **Skills Lab → Skills → Bundled** and installed/updated into the Library only on explicit
//! user action, with drift (bundled-vs-installed `version:`) shown so the user
//! decides whether to keep their copy or sync the new one. This module is the
//! read-only catalogue + the install primitive; consent/backup is the caller's job.

use std::io;
use std::path::Path;

use include_dir::{include_dir, Dir};
use otto_context::Library;
use otto_core::paths::safe_component;

pub mod http;
pub mod product;

/// The embedded skill tree: `assets/skills/<category>/<name>/{SKILL.md,…}`.
static BUNDLED: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/skills");

/// Metadata for one bundled skill, read from its `SKILL.md` frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledSkill {
    /// Globally-unique skill name (the directory name).
    pub name: String,
    /// One of the five categories (from frontmatter, falling back to the dir).
    pub category: String,
    /// Bundled content version; bumped when the skill changes. Drives drift.
    pub version: u32,
    /// Frontmatter `description:` — the selector signal.
    pub description: String,
}

/// Drift state of a bundled skill relative to what's installed in the Library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallState {
    /// Not present in the Library.
    NotInstalled,
    /// Installed at the same version as the bundle.
    UpToDate,
    /// Installed, but the bundle is newer (`installed` → `bundled`).
    UpdateAvailable { installed: u32, bundled: u32 },
    /// Installed at a version >= the bundle (user-edited / ahead). Never auto-touch.
    Ahead { installed: u32, bundled: u32 },
    /// The installed tree is not the one Otto installed (its recorded hash no
    /// longer matches): the person edited it without bumping `version:`.
    /// Install-all leaves it alone; a single update asks and always backs up.
    Modified { installed: u32, bundled: u32 },
}

/// Read a single scalar `key:` from the first frontmatter block of a SKILL.md body.
/// Returns the trimmed, unquoted value, or `None` if absent.
fn frontmatter_value(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    // The body must open with a `---` fence to have frontmatter.
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        let t = line.trim();
        if t == "---" {
            return None; // end of frontmatter
        }
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    None
}

/// Read the skill's content version from either Otto's legacy integer field or
/// the Agent Skills `metadata.version` semver string. Otto's drift model tracks
/// major content revisions, so `3` and `3.0.0` both map to version 3.
fn frontmatter_version(body: &str) -> Option<u32> {
    frontmatter_value(body, "version")?
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Locate the embedded directory for skill `name` (searched across all categories).
fn bundled_dir(name: &str) -> Option<(&'static Dir<'static>, String)> {
    for cat in BUNDLED.dirs() {
        let category = dir_name(cat);
        for skill in cat.dirs() {
            if dir_name(skill) == name {
                return Some((skill, category));
            }
        }
    }
    None
}

fn dir_name(d: &Dir<'_>) -> String {
    d.path()
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// The `SKILL.md` file inside a skill directory, if present.
fn skill_md<'a>(skill_dir: &'a Dir<'a>) -> Option<&'a include_dir::File<'a>> {
    skill_dir
        .files()
        .find(|f| f.path().file_name().and_then(|s| s.to_str()) == Some("SKILL.md"))
}

/// Build the [`BundledSkill`] metadata for an embedded skill directory.
fn meta(skill_dir: &Dir<'_>, category_dir: &str) -> Option<BundledSkill> {
    let name = dir_name(skill_dir);
    let body = skill_md(skill_dir)?.contents_utf8().unwrap_or("");
    Some(BundledSkill {
        name,
        category: frontmatter_value(body, "category").unwrap_or_else(|| category_dir.to_string()),
        version: frontmatter_version(body).unwrap_or(1),
        description: frontmatter_value(body, "description").unwrap_or_default(),
    })
}

/// Every bundled skill, sorted by `(category, name)`.
pub fn list_bundled() -> Vec<BundledSkill> {
    let mut out = Vec::new();
    for cat in BUNDLED.dirs() {
        let category = dir_name(cat);
        for skill in cat.dirs() {
            if let Some(m) = meta(skill, &category) {
                out.push(m);
            }
        }
    }
    out.sort_by(|a, b| (a.category.as_str(), a.name.as_str()).cmp(&(&b.category, &b.name)));
    out
}

/// The bundled version for `name`, if it exists.
pub fn bundled_version(name: &str) -> Option<u32> {
    let (dir, cat) = bundled_dir(name)?;
    meta(dir, &cat).map(|m| m.version)
}

/// The installed version for `name` (parsed from `<library>/skills/<name>/SKILL.md`),
/// if installed.
pub fn installed_version(library: &Library, name: &str) -> Option<u32> {
    // `name` can arrive off the wire; a separator/`..` would read outside the
    // Library — treat it as not-installed.
    let name = safe_component(name)?;
    let path = library.root.join("skills").join(name).join("SKILL.md");
    let body = std::fs::read_to_string(path).ok()?;
    frontmatter_version(&body).or(Some(1))
}

/// The bundled `SKILL.md` body for `name`, if it exists. Lets callers (the skill
/// review engine, the Skills Lab viewer) read a bundled skill's method without
/// first installing it into the Library.
pub fn bundled_body(name: &str) -> Option<String> {
    let (dir, _cat) = bundled_dir(name)?;
    skill_md(dir)?.contents_utf8().map(str::to_string)
}

/// Read one bundled file inside skill `name` by its skill-relative path
/// (e.g. `references/review-rubric.md`). Returns `None` for unsafe paths or
/// missing files.
pub fn bundled_file(name: &str, rel: &str) -> Option<String> {
    if rel
        .split(['/', '\\'])
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return None;
    }
    let (dir, _cat) = bundled_dir(name)?;
    let full = dir.path().join(rel);
    dir.get_file(&full)?.contents_utf8().map(str::to_string)
}

/// List every embedded file inside skill `name`, as skill-relative paths.
pub fn bundled_files(name: &str) -> Vec<String> {
    let Some((dir, _cat)) = bundled_dir(name) else {
        return Vec::new();
    };
    let base = dir.path();
    let mut out = Vec::new();
    collect_bundled(dir, base, &mut out);
    out.sort();
    out
}

fn collect_bundled(dir: &Dir<'_>, base: &Path, out: &mut Vec<String>) {
    for f in dir.files() {
        if let Ok(rel) = f.path().strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    for sub in dir.dirs() {
        collect_bundled(sub, base, out);
    }
}

/// Compute the [`InstallState`] of bundled skill `name` against the Library.
///
/// "Edited or not" is decided by content, not by the version number alone: the
/// install records a hash of the tree it wrote (see [`install_into`]), and a
/// tree that no longer hashes to it is [`InstallState::Modified`]. A tree equal
/// to the current bundle is up to date whatever its `version:` says. Copies
/// installed before the record existed fall back to the version comparison.
pub fn install_state(library: &Library, name: &str) -> Option<InstallState> {
    let bundled = bundled_version(name)?;
    let installed = installed_version(library, name);
    let Some(installed) = installed else {
        return Some(InstallState::NotInstalled);
    };
    let dir = library.root.join("skills").join(safe_component(name)?);
    let on_disk = tree_hash(&dir).ok();
    if on_disk.is_some() && on_disk == bundled_tree_hash(name) {
        return Some(InstallState::UpToDate);
    }
    if let (Some(recorded), Some(now)) = (recorded_hash(library, name), on_disk) {
        if recorded != now {
            return Some(InstallState::Modified { installed, bundled });
        }
    }
    Some(if installed == bundled {
        InstallState::UpToDate
    } else if installed < bundled {
        InstallState::UpdateAvailable { installed, bundled }
    } else {
        InstallState::Ahead { installed, bundled }
    })
}

/// Where the hash of each bundled install is recorded: outside `skills/` so it
/// is never listed as a skill file, copied to a provider dir or hashed itself.
fn record_path(library: &Library, name: &str) -> Option<std::path::PathBuf> {
    Some(
        library
            .root
            .join(".bundled-installs")
            .join(format!("{}.sha256", safe_component(name)?)),
    )
}

fn recorded_hash(library: &Library, name: &str) -> Option<String> {
    let raw = std::fs::read_to_string(record_path(library, name)?).ok()?;
    Some(raw.trim().to_string()).filter(|h| !h.is_empty())
}

fn record_hash(library: &Library, name: &str, hash: &str) -> io::Result<()> {
    let path = record_path(library, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    otto_context::library::write_atomic(&path, hash.as_bytes())
}

/// A content hash of a skill tree: every regular file's skill-relative path and
/// bytes, in path order. Hidden entries (editor temp files, `.DS_Store`) are
/// skipped so they never read as an edit.
pub fn tree_hash(dir: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    collect_disk(dir, dir, &mut files)?;
    files.sort();
    let mut manifest = String::new();
    for rel in files {
        let bytes = std::fs::read(dir.join(&rel))?;
        manifest.push_str(&format!(
            "{rel}\0{}\n",
            otto_core::text::bytes_sha256(&bytes)
        ));
    }
    Ok(otto_core::text::content_sha256(&manifest))
}

fn collect_disk(base: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let ty = entry.file_type()?;
        let path = entry.path();
        if ty.is_dir() {
            collect_disk(base, &path, out)?;
        } else if ty.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// [`tree_hash`] of the embedded tree for `name`, computed the same way.
fn bundled_tree_hash(name: &str) -> Option<String> {
    let (dir, _cat) = bundled_dir(name)?;
    let mut manifest = String::new();
    for rel in bundled_files(name) {
        if rel.split('/').any(|seg| seg.starts_with('.')) {
            continue;
        }
        let bytes = dir.get_file(dir.path().join(&rel))?.contents();
        manifest.push_str(&format!(
            "{rel}\0{}\n",
            otto_core::text::bytes_sha256(bytes)
        ));
    }
    Some(otto_core::text::content_sha256(&manifest))
}

/// Install (or overwrite) bundled skill `name` into the Library at
/// `<library>/skills/<name>/`, copying the full multi-file tree. Returns `false`
/// if `name` is not a bundled skill.
///
/// This OVERWRITES any existing installed copy — the Settings layer is responsible
/// for getting user consent and offering a backup first (never override silently).
pub fn install_into(library: &Library, name: &str) -> io::Result<bool> {
    // A name with separators/`..` would aim the replace below outside the
    // Library — reject before any path is built (same `false` as unknown).
    let Some(name) = safe_component(name) else {
        return Ok(false);
    };
    let Some((skill_dir, _cat)) = bundled_dir(name) else {
        return Ok(false);
    };
    let dest = library.root.join("skills").join(name);
    replace_tree(&dest, |tmp| seed_dir(skill_dir, tmp))?;
    // Record what Otto wrote, so a later edit reads as "modified" even when
    // the person did not bump `version:`.
    record_hash(library, name, &tree_hash(&dest)?)?;
    Ok(true)
}

/// Replace the directory `dest` with the tree `fill` writes, without ever
/// leaving half a skill: `fill` writes into a hidden sibling, the old tree is
/// moved aside, the new one renamed in, and only then is the old one dropped.
/// A `fill` error (disk full, permissions) leaves `dest` exactly as it was.
fn replace_tree(dest: &Path, fill: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent dir"))?;
    std::fs::create_dir_all(parent)?;
    let leaf = dest
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no dir name"))?
        .to_string_lossy();
    let tag = format!(
        "{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let fresh = parent.join(format!(".{leaf}.otto-install-{tag}"));
    let old = parent.join(format!(".{leaf}.otto-old-{tag}"));
    if let Err(e) = fill(&fresh) {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(e);
    }
    let had_old = match std::fs::rename(dest, &old) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&fresh);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&fresh, dest) {
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(e);
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

/// Materialize one compiled-in skill's complete file tree at `dest`.
///
/// Unlike [`install_into`], this does not touch the Library and never removes
/// an existing destination. It is used for per-run, out-of-tree skill bundles
/// where references, scripts, examples, and eval fixtures must travel with
/// `SKILL.md`. Callers own destination cleanup.
pub fn copy_bundled_into(name: &str, dest: &Path) -> io::Result<bool> {
    if !is_safe_skill_name(name) {
        return Ok(false);
    }
    let Some((skill_dir, _cat)) = bundled_dir(name) else {
        return Ok(false);
    };
    match std::fs::symlink_metadata(dest) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("skill destination is a symlink: {}", dest.display()),
            ));
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("skill destination already exists: {}", dest.display()),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    seed_dir(skill_dir, dest)?;
    Ok(true)
}

fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Recursively copy an embedded skill `Dir` into `dest`, creating parents.
/// `.sh` files get the executable bit on Unix.
fn seed_dir(src: &Dir<'_>, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for file in src.files() {
        let Some(name) = file.path().file_name() else {
            continue;
        };
        let out = dest.join(name);
        std::fs::write(&out, file.contents())?;
        #[cfg(unix)]
        if out.extension().and_then(|e| e.to_str()) == Some("sh") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    for sub in src.dirs() {
        let Some(name) = sub.path().file_name() else {
            continue;
        };
        seed_dir(sub, &dest.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parses_scalars() {
        let body = "---\ndescription: Hunt bugs\ncategory: review\nversion: 3\n---\n# body";
        assert_eq!(
            frontmatter_value(body, "category").as_deref(),
            Some("review")
        );
        assert_eq!(frontmatter_value(body, "version").as_deref(), Some("3"));
        assert_eq!(
            frontmatter_value(body, "description").as_deref(),
            Some("Hunt bugs")
        );
        assert_eq!(frontmatter_value(body, "missing"), None);
        assert_eq!(frontmatter_value("no frontmatter", "version"), None);
    }

    #[test]
    fn skill_spec_metadata_semver_drives_bundled_version() {
        let body = concat!(
            "---\n",
            "name: okf-authoring\n",
            "description: Use when authoring OKF bundles.\n",
            "metadata:\n",
            "  version: \"3.0.0\"\n",
            "---\n",
            "# OKF authoring\n",
        );

        assert_eq!(frontmatter_version(body), Some(3));
    }

    #[test]
    fn okf_authoring_bundles_hard_static_enforcement() {
        let skill = list_bundled()
            .into_iter()
            .find(|skill| skill.name == "okf-authoring")
            .expect("okf-authoring bundled");
        assert_eq!(skill.version, 3);

        let body = bundled_body("okf-authoring").expect("okf-authoring body");
        assert!(body.contains("## Hard static enforcement"));
        assert!(body.contains("scripts/check_vault.py"));
        assert!(body.contains("validate_okf.py ROOT --strict"));

        let files = bundled_files("okf-authoring");
        for expected in [
            "scripts/test_skill_contract.py",
            "evals/fixtures/static-link-violations/index.md",
            "evals/fixtures/static-link-violations/broken.md",
            "evals/fixtures/static-link-violations/targets/index.md",
        ] {
            assert!(
                files.iter().any(|path| path == expected),
                "missing {expected}"
            );
        }

        let evals = bundled_file("okf-authoring", "evals/evals.json").expect("evals");
        assert!(evals.contains("static-link-enforcement"));
        assert!(evals.contains("strict-completion-gate"));
    }

    #[test]
    fn bundled_review_skills_are_well_formed() {
        let all = list_bundled();
        // The review category exists and includes grill.
        assert!(all
            .iter()
            .any(|s| s.name == "grill" && s.category == "review"));
        // Every bundled skill has a non-empty description and a version >= 1.
        for s in &all {
            assert!(!s.description.is_empty(), "{} missing description", s.name);
            assert!(s.version >= 1, "{} bad version", s.name);
            assert!(!s.category.is_empty(), "{} missing category", s.name);
        }
    }

    #[test]
    fn skills_reviewer_is_bundled_and_readable() {
        let all = list_bundled();
        let sr = all
            .iter()
            .find(|s| s.name == "skills-reviewer")
            .expect("skills-reviewer bundled");
        assert_eq!(sr.category, "review");
        assert!(sr.version >= 1);
        assert!(sr.description.to_lowercase().contains("skill"));
        // Body + references are reachable without installing.
        let body = bundled_body("skills-reviewer").expect("body");
        assert!(body.contains("# Skills Reviewer"));
        let files = bundled_files("skills-reviewer");
        assert!(files.iter().any(|f| f == "SKILL.md"));
        assert!(files.iter().any(|f| f == "references/review-rubric.md"));
        assert!(bundled_file("skills-reviewer", "references/review-rubric.md").is_some());
        // Path safety on the bundled reader.
        assert!(bundled_file("skills-reviewer", "../grill/SKILL.md").is_none());
    }

    #[test]
    fn bundled_development_commit_pr_skills_present() {
        let all = list_bundled();
        for name in ["commit-message", "pull-request", "d2-diagram"] {
            let s = all
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{name} not bundled"));
            assert_eq!(s.category, "development", "{name} wrong category");
            assert!(!s.description.is_empty(), "{name} missing description");
        }
        // d2-diagram must trigger on generic "work on a diagram" asks and defer
        // Canvas-scene work to otto-canvas (the two skills split that boundary).
        let d2 = all.iter().find(|s| s.name == "d2-diagram").unwrap();
        assert!(
            d2.description.contains("diagram"),
            "d2-diagram trigger wording"
        );
        assert!(
            d2.description.contains("otto-canvas"),
            "d2-diagram canvas deferral"
        );
    }

    #[test]
    fn reviewer_skills_are_complete_packages() {
        let names = [
            "vault-docs-review",
            "vault-api-review",
            "vault-data-review",
            "vault-runtime-review",
            "vault-evidence-review",
        ];
        let all = list_bundled();
        for name in names {
            let skill = all
                .iter()
                .find(|skill| skill.name == name)
                .unwrap_or_else(|| panic!("{name} not bundled"));
            assert_eq!(skill.category, "development", "{name} wrong category");
            let files = bundled_files(name);
            assert!(
                files.iter().any(|path| path == "SKILL.md"),
                "{name} missing SKILL.md"
            );
            assert!(
                files.iter().any(|path| path.starts_with("references/")),
                "{name} missing checklist"
            );
            assert!(
                files.iter().any(|path| path.starts_with("examples/")),
                "{name} missing example"
            );
            assert!(
                files.iter().any(|path| path == "evals/evals.json"),
                "{name} missing evals"
            );
            let body = bundled_body(name).expect("reviewer body");
            assert!(
                body.contains("JSON"),
                "{name} missing JSON finding contract"
            );
            assert!(body.contains("[]"), "{name} missing clean verdict contract");
        }
    }

    #[test]
    fn install_commit_message_skill_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert!(install_into(&lib, "commit-message").unwrap());
        // The full multi-file tree lands, and the helper script is executable.
        assert!(dir.path().join("skills/commit-message/SKILL.md").exists());
        assert!(dir
            .path()
            .join("skills/commit-message/references/commit-conventions.md")
            .exists());
        let script = dir
            .path()
            .join("skills/commit-message/scripts/prepare-commit-context.sh");
        assert!(script.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o111 != 0);
        }
    }

    #[test]
    fn install_and_state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());

        assert_eq!(
            install_state(&lib, "grill"),
            Some(InstallState::NotInstalled)
        );
        assert!(install_into(&lib, "grill").unwrap());

        // SKILL.md + references + scripts copied; script is executable.
        let md = dir.path().join("skills/grill/SKILL.md");
        assert!(md.exists());
        let script = dir.path().join("skills/grill/scripts/scope-change.sh");
        assert!(script.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o111 != 0);
        }

        assert_eq!(install_state(&lib, "grill"), Some(InstallState::UpToDate));
        assert!(!install_into(&lib, "not-a-skill").unwrap());
    }

    /// An edit that keeps `version:` is still "modified" (the recorded tree
    /// hash no longer matches), and putting the bundled text back clears it.
    #[test]
    fn edit_without_version_bump_reads_as_modified() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert!(install_into(&lib, "grill").unwrap());
        assert_eq!(install_state(&lib, "grill"), Some(InstallState::UpToDate));
        let md = dir.path().join("skills/grill/SKILL.md");
        let original = std::fs::read_to_string(&md).unwrap();
        std::fs::write(&md, format!("{original}\nMy own house rule.\n")).unwrap();
        let v = bundled_version("grill").unwrap();
        assert_eq!(
            install_state(&lib, "grill"),
            Some(InstallState::Modified {
                installed: v,
                bundled: v
            })
        );
        // A new file counts too; editor temp files (hidden) do not.
        std::fs::write(&md, &original).unwrap();
        assert_eq!(install_state(&lib, "grill"), Some(InstallState::UpToDate));
        std::fs::write(dir.path().join("skills/grill/.SKILL.md.otto-tmp-1"), "x").unwrap();
        assert_eq!(install_state(&lib, "grill"), Some(InstallState::UpToDate));
        std::fs::write(dir.path().join("skills/grill/extra.md"), "x").unwrap();
        assert!(matches!(
            install_state(&lib, "grill"),
            Some(InstallState::Modified { .. })
        ));
    }

    /// A replace that fails midway leaves the previous tree whole, with no
    /// hidden temp dirs left behind; a successful one swaps the tree.
    #[test]
    fn replace_tree_is_atomic_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("skills/grill");
        std::fs::create_dir_all(dest.join("references")).unwrap();
        std::fs::write(dest.join("SKILL.md"), "previous").unwrap();
        std::fs::write(dest.join("references/r.md"), "ref").unwrap();

        let err = replace_tree(&dest, |tmp| {
            std::fs::create_dir_all(tmp)?;
            std::fs::write(tmp.join("SKILL.md"), "half")?;
            Err(io::Error::other("disk full"))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "disk full");
        assert_eq!(
            std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "previous"
        );
        assert!(dest.join("references/r.md").is_file());
        let names: Vec<String> = std::fs::read_dir(dir.path().join("skills"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["grill".to_string()]);

        replace_tree(&dest, |tmp| {
            std::fs::create_dir_all(tmp)?;
            std::fs::write(tmp.join("SKILL.md"), "next")
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "next"
        );
        assert!(!dest.join("references").exists(), "old tree dropped");
    }

    #[test]
    fn bundled_skill_tree_materializes_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("skills-reviewer");
        assert!(copy_bundled_into("skills-reviewer", &dest).unwrap());
        assert!(dest.join("SKILL.md").is_file());
        assert!(dest.join("references/review-rubric.md").is_file());
        assert!(dest.join("scripts/skill_review.py").is_file());
        assert!(dest.join("examples/excellent-skill/SKILL.md").is_file());
        assert!(!copy_bundled_into("../skills-reviewer", &dest).unwrap());
        assert!(!copy_bundled_into("/skills-reviewer", &dest).unwrap());
    }

    #[test]
    fn bundled_skill_tree_rejects_symlink_destination() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let dest = dir.path().join("skill-link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &dest).unwrap();

        #[cfg(unix)]
        {
            let err = copy_bundled_into("skills-reviewer", &dest).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(!outside.join("SKILL.md").exists());
        }
    }

    #[test]
    fn bundled_skill_tree_preserves_executable_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("commit-message");
        assert!(copy_bundled_into("commit-message", &dest).unwrap());
        let script = dest.join("scripts/prepare-commit-context.sh");
        assert!(script.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(script).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }
}

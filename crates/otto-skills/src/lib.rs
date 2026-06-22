//! otto-skills — Otto's bundled, versioned skill library.
//!
//! Skills are organized by category (`product | project | development | review |
//! design`) under `assets/skills/<category>/<name>/` and embedded into the binary.
//!
//! Unlike `otto-product` (which auto-seeds its lenses into the Library on daemon
//! start), these skills are **never auto-installed**. They are surfaced in
//! **Settings → Skills** and installed/updated into the Library only on explicit
//! user action, with drift (bundled-vs-installed `version:`) shown so the user
//! decides whether to keep their copy or sync the new one. This module is the
//! read-only catalogue + the install primitive; consent/backup is the caller's job.

use std::io;
use std::path::Path;

use include_dir::{include_dir, Dir};
use otto_context::Library;

pub mod http;

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
        version: frontmatter_value(body, "version")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
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
    let path = library.root.join("skills").join(name).join("SKILL.md");
    let body = std::fs::read_to_string(path).ok()?;
    frontmatter_value(&body, "version")
        .and_then(|v| v.parse().ok())
        .or(Some(1))
}

/// Compute the [`InstallState`] of bundled skill `name` against the Library.
pub fn install_state(library: &Library, name: &str) -> Option<InstallState> {
    let bundled = bundled_version(name)?;
    Some(match installed_version(library, name) {
        None => InstallState::NotInstalled,
        Some(installed) if installed == bundled => InstallState::UpToDate,
        Some(installed) if installed < bundled => InstallState::UpdateAvailable { installed, bundled },
        Some(installed) => InstallState::Ahead { installed, bundled },
    })
}

/// Install (or overwrite) bundled skill `name` into the Library at
/// `<library>/skills/<name>/`, copying the full multi-file tree. Returns `false`
/// if `name` is not a bundled skill.
///
/// This OVERWRITES any existing installed copy — the Settings layer is responsible
/// for getting user consent and offering a backup first (never override silently).
pub fn install_into(library: &Library, name: &str) -> io::Result<bool> {
    let Some((skill_dir, _cat)) = bundled_dir(name) else {
        return Ok(false);
    };
    let dest = library.root.join("skills").join(name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    seed_dir(skill_dir, &dest)?;
    Ok(true)
}

/// Recursively copy an embedded skill `Dir` into `dest`, creating parents.
/// `.sh` files get the executable bit on Unix.
fn seed_dir(src: &Dir<'_>, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for file in src.files() {
        let Some(name) = file.path().file_name() else { continue };
        let out = dest.join(name);
        std::fs::write(&out, file.contents())?;
        #[cfg(unix)]
        if out.extension().and_then(|e| e.to_str()) == Some("sh") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    for sub in src.dirs() {
        let Some(name) = sub.path().file_name() else { continue };
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
        assert_eq!(frontmatter_value(body, "category").as_deref(), Some("review"));
        assert_eq!(frontmatter_value(body, "version").as_deref(), Some("3"));
        assert_eq!(frontmatter_value(body, "description").as_deref(), Some("Hunt bugs"));
        assert_eq!(frontmatter_value(body, "missing"), None);
        assert_eq!(frontmatter_value("no frontmatter", "version"), None);
    }

    #[test]
    fn bundled_review_skills_are_well_formed() {
        let all = list_bundled();
        // The review category exists and includes grill.
        assert!(all.iter().any(|s| s.name == "grill" && s.category == "review"));
        // Every bundled skill has a non-empty description and a version >= 1.
        for s in &all {
            assert!(!s.description.is_empty(), "{} missing description", s.name);
            assert!(s.version >= 1, "{} bad version", s.name);
            assert!(!s.category.is_empty(), "{} missing category", s.name);
        }
    }

    #[test]
    fn bundled_development_commit_pr_skills_present() {
        let all = list_bundled();
        for name in ["commit-message", "pull-request"] {
            let s = all
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{name} not bundled"));
            assert_eq!(s.category, "development", "{name} wrong category");
            assert!(!s.description.is_empty(), "{name} missing description");
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

        assert_eq!(install_state(&lib, "grill"), Some(InstallState::NotInstalled));
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
}

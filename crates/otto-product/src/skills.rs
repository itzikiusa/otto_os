//! The specialized agent skills used by product-story analysis, rewrite,
//! test-case generation, and task/plan breakdown. They are seeded into the Otto
//! skill library on daemon startup so they are editable in the UI and
//! self-improvable by `otto-improve`.
//!
//! Seeding is VERSION-GATED: a `.product-skills-version` marker file in the
//! library root controls whether (re)seeding is needed. Bump
//! `PRODUCT_SKILLS_SEED_VERSION` to force an overwrite of all skill dirs on
//! the next daemon start — useful when skill content changes between releases.
//! Between version bumps the seed is a no-op so user / self-improvement edits
//! are never clobbered on routine restart.

use std::fs;
use std::path::Path;

use include_dir::{include_dir, Dir};
use otto_context::Library;

/// The embedded multi-file skill tree (SKILL.md + references/ + assets/ + scripts/).
static PRODUCT_SKILLS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/skills");

/// Version that governs the seeded skill dirs. Increment this when the bundled
/// skill content changes and existing installs need to be upgraded. A future
/// version bump will re-seed (overwrite) all skill dirs. Bumped to 3 to seed the
/// new `story-task-breakdown` skill.
const PRODUCT_SKILLS_SEED_VERSION: u32 = 3;

/// The skills this feature owns, by library name.
pub const SKILL_NAMES: [&str; 7] = [
    "po-story-overview",
    "story-clarifying-questions",
    "story-architecture-overview",
    "story-test-cases",
    "jira-story-writer",
    "rfc-writer",
    "story-task-breakdown",
];

/// The bundled body of a product skill's `SKILL.md`, or `None` if the name is
/// not one of ours (or the embedded file is missing / not valid UTF-8).
pub fn skill_body(name: &str) -> Option<&'static str> {
    if !SKILL_NAMES.contains(&name) {
        return None;
    }
    PRODUCT_SKILLS
        .get_file(&format!("{name}/SKILL.md"))
        .and_then(|f| f.contents_utf8())
}

/// Seed each product skill into the library, version-gated.
///
/// - Reads `<library.root>/.product-skills-version`.
/// - If the file is missing or contains a version number less than
///   `PRODUCT_SKILLS_SEED_VERSION`, all skill dirs are (re)written from the
///   embedded tree (OVERWRITE — this upgrades existing single-SKILL.md installs
///   to the v2 multi-file layout). `.sh` files are made executable on Unix.
/// - After seeding the version marker is updated.
/// - If the marker already equals `PRODUCT_SKILLS_SEED_VERSION` the function
///   returns immediately (no-op) — user edits and self-improvement edits that
///   happen between version bumps are preserved.
pub fn seed_skills(library: &Library) -> std::io::Result<()> {
    let marker_path = library.root.join(".product-skills-version");
    let current_version = read_version_marker(&marker_path);

    if current_version >= PRODUCT_SKILLS_SEED_VERSION {
        // Already at the current version — nothing to do.
        return Ok(());
    }

    // (Re)seed all skill dirs from the embedded tree.
    let skills_root = library.root.join("skills");
    for skill_name in SKILL_NAMES {
        let Some(skill_dir) = PRODUCT_SKILLS.get_dir(skill_name) else {
            continue;
        };
        seed_dir(skill_dir, &skills_root.join(skill_name))?;
    }

    // Write the version marker so subsequent boots are no-ops.
    write_version_marker(&marker_path, PRODUCT_SKILLS_SEED_VERSION)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse the version marker file. Returns 0 when absent or unparseable.
fn read_version_marker(path: &Path) -> u32 {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn write_version_marker(path: &Path, version: u32) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, version.to_string())
}

/// Recursively copy every file in `src_dir` (an embedded `Dir`) into `dest_dir`
/// on disk, creating all parent directories. Existing files are OVERWRITTEN.
/// On Unix, `.sh` files receive mode 0o755 (executable).
fn seed_dir(src_dir: &Dir<'_>, dest_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest_dir)?;

    // Write files directly inside this dir.
    for file in src_dir.files() {
        let rel = file
            .path()
            .file_name()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no filename"))?;
        let dest = dest_dir.join(rel);
        fs::write(&dest, file.contents())?;

        // Make shell scripts executable on Unix.
        #[cfg(unix)]
        if dest.extension().and_then(|e| e.to_str()) == Some("sh") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
        }
    }

    // Recurse into subdirectories.
    for sub in src_dir.dirs() {
        let sub_name = sub
            .path()
            .file_name()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no dir name"))?;
        seed_dir(sub, &dest_dir.join(sub_name))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_body_known_and_unknown() {
        for name in SKILL_NAMES {
            let body = skill_body(name).expect("known skill has a body");
            assert!(
                body.contains("description:"),
                "skill {name} must have frontmatter description"
            );
            assert!(body.len() > 200, "skill {name} body should be substantive");
        }
        assert!(skill_body("not-a-real-skill").is_none());
    }

    #[test]
    fn seed_writes_full_dir_tree_and_version_marker() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::new(dir.path());

        seed_skills(&library).unwrap();

        // SKILL.md seeded.
        let skill_md = dir.path().join("skills/po-story-overview/SKILL.md");
        assert!(skill_md.exists(), "SKILL.md must be seeded");
        let content = fs::read_to_string(&skill_md).unwrap();
        assert!(content.contains("description:"), "SKILL.md must have frontmatter");

        // references/ directory seeded.
        let refs_dir = dir.path().join("skills/po-story-overview/references");
        assert!(refs_dir.is_dir(), "references/ dir must exist");
        let refs: Vec<_> = fs::read_dir(&refs_dir).unwrap().flatten().collect();
        assert!(!refs.is_empty(), "references/ must contain files");

        // scripts/ and executable bit for story-architecture-overview.
        let script = dir
            .path()
            .join("skills/story-architecture-overview/scripts/repo-scan.sh");
        assert!(script.exists(), "repo-scan.sh must be seeded");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&script).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "repo-scan.sh must be executable");
        }

        // Version marker written.
        let marker = dir.path().join(".product-skills-version");
        assert!(marker.exists(), "version marker must be written");
        let v: u32 = fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
        assert_eq!(v, PRODUCT_SKILLS_SEED_VERSION);
    }

    #[test]
    fn second_seed_is_noop_when_marker_matches() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::new(dir.path());

        // First seed.
        seed_skills(&library).unwrap();

        // Overwrite SKILL.md with a custom value to detect re-seeding.
        let skill_md = dir.path().join("skills/po-story-overview/SKILL.md");
        fs::write(&skill_md, "custom edit by user").unwrap();

        // Second seed must be a no-op (marker already at current version).
        seed_skills(&library).unwrap();
        let after = fs::read_to_string(&skill_md).unwrap();
        assert_eq!(after, "custom edit by user", "second seed must not clobber user edits");
    }
}

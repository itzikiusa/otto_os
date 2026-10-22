//! The Otto-region marker merge and the claude skills manifest.
//!
//! Otto only ever rewrites the bytes between `<!-- OTTO:START -->` and
//! `<!-- OTTO:END -->`; everything outside those markers is preserved exactly.

use std::fs;
use std::io;
use std::path::Path;

pub const START: &str = "<!-- OTTO:START -->";
pub const END: &str = "<!-- OTTO:END -->";

/// Manifest file name under a claude skills dir, listing the skill dirs Otto
/// manages (so deactivations clean up without touching user-authored skills).
const MANIFEST_FILE: &str = ".otto-managed.json";

/// Merge `new_block` into the Otto region of `existing`.
///
/// - If `existing` already contains an Otto region, its contents are replaced
///   with `new_block`.
/// - Otherwise an Otto region is appended (after a blank-line separator if
///   `existing` is non-empty).
/// - If `existing` is empty, the result is just the region.
///
/// The region is formatted as:
/// ```text
/// <!-- OTTO:START -->
/// {new_block}
/// <!-- OTTO:END -->
/// ```
pub fn merge_otto_region(existing: &str, new_block: &str) -> String {
    let region = format!("{START}\n{new_block}\n{END}");

    if let (Some(start_idx), Some(end_idx)) = (existing.find(START), existing.find(END)) {
        if start_idx < end_idx {
            let before = &existing[..start_idx];
            let after = &existing[end_idx + END.len()..];
            return format!("{before}{region}{after}");
        }
    }

    if existing.is_empty() {
        region
    } else if existing.ends_with('\n') {
        format!("{existing}\n{region}")
    } else {
        format!("{existing}\n\n{region}")
    }
}

/// Read the managed-skill manifest from `<skills_dir>/.otto-managed.json`.
/// Missing or malformed manifests yield an empty list (a lenient view for
/// display); ownership decisions use [`read_manifest_checked`].
pub fn read_manifest(skills_dir: &Path) -> Vec<String> {
    read_manifest_checked(skills_dir).unwrap_or_default()
}

/// The manifest for an ownership decision: a MISSING file is "Otto owns
/// nothing here" (`Ok(empty)`), but an unreadable or malformed one is an
/// error. Degrading it to empty would make every Otto-installed skill in the
/// dir "user-owned" for good, and the next write would replace the list with
/// a single entry.
pub fn read_manifest_checked(skills_dir: &Path) -> io::Result<Vec<String>> {
    let path = skills_dir.join(MANIFEST_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    serde_json::from_str::<Manifest>(&raw)
        .map(|m| m.skills)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "malformed {} ({e}); fix or delete it to let Otto manage skills there",
                    path.display()
                ),
            )
        })
}

/// Write the managed-skill manifest to `<skills_dir>/.otto-managed.json`
/// atomically: a temp file in the same dir, `sync_all`, then a rename, so a
/// crash or full disk mid-write leaves the previous manifest intact instead
/// of a truncated one.
pub fn write_manifest(skills_dir: &Path, names: &[String]) -> io::Result<()> {
    use std::io::Write;
    fs::create_dir_all(skills_dir)?;
    let manifest = Manifest {
        skills: names.to_vec(),
    };
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = skills_dir.join(format!("{MANIFEST_FILE}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, skills_dir.join(MANIFEST_FILE))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    skills: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// data-integrity-4-04: missing ≠ malformed, and the write is atomic
    /// (no temp file left behind, content replaced whole).
    #[test]
    fn manifest_missing_vs_malformed_and_atomic_write() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        assert!(read_manifest_checked(dir).unwrap().is_empty());
        write_manifest(dir, &["a".into(), "b".into()]).unwrap();
        assert_eq!(read_manifest_checked(dir).unwrap(), ["a", "b"]);
        let leftovers: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        fs::write(dir.join(MANIFEST_FILE), "{\"skills\": [\"a\"").unwrap();
        let err = read_manifest_checked(dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(read_manifest(dir).is_empty(), "lenient view stays lenient");
    }

    #[test]
    fn create_from_empty() {
        let out = merge_otto_region("", "hello");
        assert_eq!(out, "<!-- OTTO:START -->\nhello\n<!-- OTTO:END -->");
    }

    #[test]
    fn append_when_absent() {
        let out = merge_otto_region("# My file\n\nHand-written.\n", "block");
        assert!(out.starts_with("# My file\n\nHand-written.\n"));
        assert!(out.contains("<!-- OTTO:START -->\nblock\n<!-- OTTO:END -->"));
        // Hand-written content survives.
        assert!(out.contains("Hand-written."));
    }

    #[test]
    fn replace_in_region() {
        let existing = "before\n<!-- OTTO:START -->\nold content\n<!-- OTTO:END -->\nafter\n";
        let out = merge_otto_region(existing, "new content");
        assert!(out.contains("new content"));
        assert!(!out.contains("old content"));
        // Outside-region bytes preserved.
        assert!(out.starts_with("before\n"));
        assert!(out.ends_with("\nafter\n"));
    }

    #[test]
    fn preserve_outside_region_exactly() {
        let existing = "TOP LINE\n<!-- OTTO:START -->\nx\n<!-- OTTO:END -->\nBOTTOM LINE";
        let out = merge_otto_region(existing, "y");
        assert_eq!(
            out,
            "TOP LINE\n<!-- OTTO:START -->\ny\n<!-- OTTO:END -->\nBOTTOM LINE"
        );
    }

    #[test]
    fn manifest_round_trip() {
        let dir = TempDir::new().unwrap();
        assert!(read_manifest(dir.path()).is_empty());
        let names = vec!["a".to_string(), "b".to_string()];
        write_manifest(dir.path(), &names).unwrap();
        assert_eq!(read_manifest(dir.path()), names);
    }
}

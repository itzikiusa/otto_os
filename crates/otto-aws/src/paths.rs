//! Otto-owned file locations under the daemon data dir.
//!
//! Every file this crate writes (imported kubeconfigs, S3 preview scratch)
//! lives under `<data_dir>/<subdir>/<server-generated name>`. No request
//! field ever becomes a path component — names are fresh ULIDs minted here —
//! and [`owned_file`] re-asserts that invariant at the join: the leaf must be
//! a single ULID-shaped token and the resolved parent must be exactly the
//! Otto-owned directory. This is belt-and-braces (and keeps static analysis
//! honest about where the path comes from), not a substitute for never
//! putting user input in a path in the first place.
use std::path::{Component, Path, PathBuf};

use otto_core::{Error, Result};

/// `<data_dir>/<subdir>`, created with mode 0700. `subdir` is a compile-time
/// constant chosen by the caller (`"kube"`, `"tmp"`), never request data.
pub fn owned_dir(data_dir: &Path, subdir: &'static str) -> Result<PathBuf> {
    debug_assert!(!subdir.contains(['/', '\\']));
    let dir = data_dir.join(subdir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::Internal(format!("create {}: {e}", dir.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// `<dir>/<stem>.<ext>` where `stem` MUST be a fresh ULID (26 Crockford
/// base32 chars) and `ext` a short alphanumeric extension (or empty). Any
/// other shape — separators, dots, `..`, control chars — is rejected, and the
/// joined path is verified to sit directly inside `dir`.
pub fn owned_file(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf> {
    let ulid_ok = stem.len() == 26
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)));
    let ext_ok = ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    if !ulid_ok || !ext_ok {
        return Err(Error::Internal("refusing non-ULID owned file name".into()));
    }
    let leaf = if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    };
    let path = dir.join(&leaf);
    let mut comps = path.components().rev();
    let last_is_leaf = matches!(comps.next(), Some(Component::Normal(c)) if c == leaf.as_str());
    if !last_is_leaf || path.parent() != Some(dir) {
        return Err(Error::Internal("owned file escaped its directory".into()));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_file_accepts_ulids_and_rejects_everything_else() {
        let dir = Path::new("/data/kube");
        let id = otto_core::new_id();
        let p = owned_file(dir, &id, "yaml").unwrap();
        assert_eq!(p.parent(), Some(dir));
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), format!("{id}.yaml"));
        assert!(owned_file(dir, &id, "").unwrap().ends_with(&id));
        for bad in ["../etc", "x/y", "01ARZ3NDEKTSV4RRFFQ69G5FA.", "", "abc"] {
            assert!(owned_file(dir, bad, "yaml").is_err(), "{bad}");
        }
        assert!(owned_file(dir, &id, "y/a").is_err());
        assert!(owned_file(dir, &id, "toolongext").is_err());
    }

    #[test]
    fn owned_dir_creates_under_data_dir() {
        let base = std::env::temp_dir().join(format!("otto-paths-{}", otto_core::new_id()));
        let d = owned_dir(&base, "kube").unwrap();
        assert!(d.is_dir());
        assert_eq!(d.parent(), Some(base.as_path()));
        let _ = std::fs::remove_dir_all(&base);
    }
}

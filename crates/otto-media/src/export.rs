//! Workspace-confined atomic asset exporter for 3D meshes and TTS audio clips.
//!
//! Security invariants:
//! - `relative_path` must be strictly relative (no leading `/` or `\`, no drive
//!   prefix, no `..`, no NUL bytes, non-empty filename).
//! - File extension must match the asset allowlist (e.g. `glb|gltf|obj|json` for
//!   3D, `wav|mp3|m4a|aiff` for TTS).
//! - Every existing path component is canonicalized and verified to stay inside
//!   the canonical workspace root (blocks symlink escapes).
//! - Refuses writing into Otto's `data_dir` or any `otto_core::secret_paths`
//!   denied directory/file.
//! - Refuses overwriting an existing file unless `overwrite: true`, and refuses
//!   if the target itself is a symlink.
//! - Writes atomically via a unique temp file in the target directory + rename.

use std::path::{Component, Path, PathBuf};

use otto_core::{Error, Result};
use sha2::{Digest, Sha256};

use crate::types::ExportWorkspaceResp;

pub const ALLOWED_3D_EXPORT_EXTS: &[&str] = &["glb", "gltf", "obj", "json"];
pub const ALLOWED_AUDIO_EXPORT_EXTS: &[&str] = &["wav", "mp3", "m4a", "aiff"];

/// Validate `relative_path` lexically and return normalized relative `PathBuf` +
/// lowercase extension.
pub(crate) fn parse_safe_relative(
    relative_path: &str,
    allowed_exts: &[&str],
) -> Result<(PathBuf, String)> {
    let trimmed = relative_path.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("relative_path cannot be empty".into()));
    }
    if trimmed.contains('\0') || trimmed.contains('\\') {
        return Err(Error::Invalid(
            "relative_path contains disallowed characters".into(),
        ));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return Err(Error::Invalid(
            "relative_path must be relative to the workspace root".into(),
        ));
    }
    let raw = Path::new(trimmed);
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::Normal(part) => {
                let s = part.to_string_lossy();
                if s.contains(':') {
                    return Err(Error::Invalid(
                        "relative_path cannot contain drive or colon segments".into(),
                    ));
                }
                clean.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::Invalid(
                    "relative_path cannot escape the workspace root".into(),
                ));
            }
        }
    }
    let file_name = clean
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| Error::Invalid("relative_path must name a file".into()))?;
    if file_name.starts_with('.') {
        return Err(Error::Invalid(
            "export file name cannot start with a dot".into(),
        ));
    }
    let ext = clean
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "relative_path must end with an allowed extension ({})",
                allowed_exts.join(", ")
            ))
        })?;
    if !allowed_exts.contains(&ext.as_str()) {
        return Err(Error::Invalid(format!(
            "extension '.{ext}' is not allowed (expected one of: {})",
            allowed_exts.join(", ")
        )));
    }
    Ok((clean, ext))
}

fn check_not_denied(path: &Path, canon_data_dir: Option<&Path>) -> Result<()> {
    if let Some(dd) = canon_data_dir {
        if path == dd || path.starts_with(dd) {
            return Err(Error::Forbidden(
                "cannot export into Otto's internal data directory".into(),
            ));
        }
    }
    if otto_core::secret_paths::is_denied_dir(path) || otto_core::secret_paths::is_denied_file(path)
    {
        return Err(Error::Forbidden(
            "cannot export into a protected credential directory".into(),
        ));
    }
    Ok(())
}

/// Atomically export `bytes` into `workspace_root / relative_path` with full
/// symlink-escape and secret-path confinement.
pub async fn export_bytes_to_workspace(
    workspace_root: &Path,
    data_dir: &Path,
    relative_path: &str,
    allowed_exts: &[&str],
    bytes: &[u8],
    overwrite: bool,
) -> Result<ExportWorkspaceResp> {
    let (rel, ext) = parse_safe_relative(relative_path, allowed_exts)?;
    let ws_root = workspace_root.to_path_buf();
    let data_dir = data_dir.to_path_buf();
    let payload = bytes.to_vec();

    tokio::task::spawn_blocking(move || {
        export_blocking(&ws_root, &data_dir, &rel, &ext, &payload, overwrite)
    })
    .await
    .map_err(|e| Error::Internal(format!("workspace export task failed: {e}")))?
}

fn export_blocking(
    workspace_root: &Path,
    data_dir: &Path,
    rel: &Path,
    ext: &str,
    bytes: &[u8],
    overwrite: bool,
) -> Result<ExportWorkspaceResp> {
    let canon_root = std::fs::canonicalize(workspace_root).map_err(|e| {
        Error::Invalid(format!(
            "workspace root '{}' is not accessible: {e}",
            workspace_root.display()
        ))
    })?;
    if !canon_root.is_dir() {
        return Err(Error::Invalid("workspace root is not a directory".into()));
    }
    let canon_data_dir = std::fs::canonicalize(data_dir).ok();
    check_not_denied(&canon_root, canon_data_dir.as_deref())?;

    // Walk every ancestor component of `rel.parent()` before creating dirs so
    // an existing symlink inside the workspace can NEVER redirect `create_dir_all`
    // outside `canon_root`.
    let rel_parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let mut cur = canon_root.clone();
    for comp in rel_parent.components() {
        let Component::Normal(seg) = comp else {
            return Err(Error::Invalid("invalid relative parent component".into()));
        };
        let next = cur.join(seg);
        match std::fs::symlink_metadata(&next) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(Error::Forbidden(
                        "refusing to traverse a symlink during workspace export".into(),
                    ));
                }
                if !meta.is_dir() {
                    return Err(Error::Invalid(format!(
                        "'{}' exists and is not a directory",
                        next.display()
                    )));
                }
                let next_canon = std::fs::canonicalize(&next)
                    .map_err(|e| Error::Internal(format!("canonicalize parent: {e}")))?;
                if !next_canon.starts_with(&canon_root) {
                    return Err(Error::Forbidden(
                        "export path escapes workspace root".into(),
                    ));
                }
                check_not_denied(&next_canon, canon_data_dir.as_deref())?;
                cur = next_canon;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&next)
                    .map_err(|ce| Error::Internal(format!("create export directory: {ce}")))?;
                let next_canon = std::fs::canonicalize(&next)
                    .map_err(|ce| Error::Internal(format!("canonicalize created dir: {ce}")))?;
                if !next_canon.starts_with(&canon_root) {
                    return Err(Error::Forbidden(
                        "export path escapes workspace root".into(),
                    ));
                }
                check_not_denied(&next_canon, canon_data_dir.as_deref())?;
                cur = next_canon;
            }
            Err(e) => {
                return Err(Error::Internal(format!("stat export parent: {e}")));
            }
        }
    }

    let file_name = rel
        .file_name()
        .ok_or_else(|| Error::Invalid("missing export file name".into()))?;
    let target = cur.join(file_name);
    check_not_denied(&target, canon_data_dir.as_deref())?;

    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err(Error::Forbidden(
                "refusing to overwrite a symlink target".into(),
            ));
        }
        if meta.is_dir() {
            return Err(Error::Conflict(
                "export target already exists as a directory".into(),
            ));
        }
        if !overwrite {
            return Err(Error::Conflict(format!(
                "'{}' already exists — pass overwrite: true to replace it",
                rel.to_string_lossy()
            )));
        }
    }

    // Atomic write inside `cur` (same filesystem as `target`).
    let mut tmp = tempfile::Builder::new()
        .prefix(".otto-export-")
        .tempfile_in(&cur)
        .map_err(|e| Error::Internal(format!("create export temp file: {e}")))?;
    use std::io::Write;
    tmp.write_all(bytes)
        .map_err(|e| Error::Internal(format!("write export bytes: {e}")))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| Error::Internal(format!("sync export file: {e}")))?;
    if overwrite {
        tmp.persist(&target)
            .map_err(|e| Error::Internal(format!("persist export file: {e}")))?;
    } else {
        tmp.persist_noclobber(&target).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Conflict(format!(
                    "'{}' already exists — pass overwrite: true to replace it",
                    rel.to_string_lossy()
                ))
            } else {
                Error::Internal(format!("persist export file: {}", e.error))
            }
        })?;
    }

    let sha256 = hex::encode(Sha256::digest(bytes));
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    Ok(ExportWorkspaceResp {
        path: rel_str,
        absolute_path: target.to_string_lossy().to_string(),
        size_bytes: bytes.len() as u64,
        format: ext.to_string(),
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn confines_exports_and_rejects_traversal_symlinks_and_clobber() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();

        // Reject traversal & absolute paths & bad extensions.
        for bad in [
            "../escape.glb",
            "/tmp/escape.glb",
            "assets/../../escape.glb",
            "assets/script.sh",
            ".hidden.glb",
            "",
        ] {
            assert!(
                export_bytes_to_workspace(
                    ws.path(),
                    data_dir.path(),
                    bad,
                    ALLOWED_3D_EXPORT_EXTS,
                    b"glTF12345678",
                    true,
                )
                .await
                .is_err(),
                "should reject {bad}"
            );
        }

        // Reject symlink directory escape.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), ws.path().join("linked")).unwrap();
            let err = export_bytes_to_workspace(
                ws.path(),
                data_dir.path(),
                "linked/model.glb",
                ALLOWED_3D_EXPORT_EXTS,
                b"glTF12345678",
                true,
            )
            .await
            .unwrap_err();
            assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
        }

        // Valid export succeeds, second export without overwrite returns Conflict.
        let ok = export_bytes_to_workspace(
            ws.path(),
            data_dir.path(),
            "assets/models/shield.glb",
            ALLOWED_3D_EXPORT_EXTS,
            b"glTF12345678",
            false,
        )
        .await
        .unwrap();
        assert_eq!(ok.path, "assets/models/shield.glb");
        assert_eq!(ok.size_bytes, 12);
        assert!(ws.path().join("assets/models/shield.glb").is_file());

        let conflict = export_bytes_to_workspace(
            ws.path(),
            data_dir.path(),
            "assets/models/shield.glb",
            ALLOWED_3D_EXPORT_EXTS,
            b"glTF99999999",
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(conflict, Error::Conflict(_)), "{conflict:?}");

        // Overwrite replaces atomically.
        let replaced = export_bytes_to_workspace(
            ws.path(),
            data_dir.path(),
            "assets/models/shield.glb",
            ALLOWED_3D_EXPORT_EXTS,
            b"glTF99999999",
            true,
        )
        .await
        .unwrap();
        assert_ne!(replaced.sha256, ok.sha256);
    }
}

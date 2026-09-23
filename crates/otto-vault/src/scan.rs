//! Filesystem walk + change detection. Pure with respect to the DB — returns
//! listings; the engine decides what to (re)parse.

use std::path::{Path, PathBuf};

/// Directories never entered. `.trash` is the vault's own soft-delete bin;
/// `.obsidian` is Obsidian's config; hidden dirs cover `.git` and friends.
pub fn is_skipped_dir(name: &str) -> bool {
    name.starts_with('.') || name == "node_modules"
}

/// Notes over this size are indexed metadata-only (no FTS body).
pub const MAX_FTS_BYTES: u64 = 4 * 1024 * 1024;

pub struct WalkEntry {
    pub rel: String,
    pub size: i64,
    pub mtime_ns: i64,
}

pub struct WalkResult {
    pub complete: bool,
    pub notes: Vec<WalkEntry>,
    pub files: Vec<WalkEntry>,
}

/// Recursively list the vault. Blocking — call from `spawn_blocking`.
pub fn walk(root: &Path) -> std::io::Result<WalkResult> {
    let mut complete = true;
    let mut notes = Vec::new();
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                complete = false;
                continue;
            } // Do not infer removals from an incomplete walk.
        };
        for result in entries {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                complete = false;
                continue;
            };
            if meta.is_dir() {
                if !is_skipped_dir(&name) {
                    stack.push(path);
                }
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            // `DirEntry::metadata` is an lstat on unix: a symlink (or a FIFO,
            // socket, …) is not a regular file. Symlinked notes are never
            // readable through the NOFOLLOW preparation path, so listing them
            // failed every scan forever (and blocked all pruning).
            if !meta.is_file() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let size = meta.len() as i64;
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            let e = WalkEntry {
                rel,
                size,
                mtime_ns,
            };
            if name.to_lowercase().ends_with(".md") {
                notes.push(e);
            } else {
                files.push(e);
            }
        }
    }
    notes.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(WalkResult {
        complete,
        notes,
        files,
    })
}

/// True only when `rel` names a regular file whose every path component
/// matches a directory entry byte-for-byte. `symlink_metadata` alone is not
/// enough: on case-insensitive APFS `note.md` still stats after a rename to
/// `Note.md`, and a symlink stats although the walk skips it — both left a
/// removed index row "present" forever. Blocking — call from `spawn_blocking`.
pub fn exact_regular_file(root: &Path, rel: &str) -> std::io::Result<bool> {
    let mut dir = root.to_path_buf();
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let mut found = None;
        for entry in entries {
            let entry = entry?;
            if entry.file_name().as_os_str() == std::ffi::OsStr::new(part) {
                found = Some(entry.file_type()?);
                break;
            }
        }
        let Some(kind) = found else {
            return Ok(false);
        };
        if parts.peek().is_none() {
            return Ok(kind.is_file());
        }
        if !kind.is_dir() {
            return Ok(false);
        }
        dir.push(part);
    }
    Ok(false)
}

/// Diff a walk against the indexed signatures → (added_or_changed, removed).
pub fn diff(on_disk: &[WalkEntry], indexed: &[(String, i64, i64)]) -> (Vec<String>, Vec<String>) {
    use std::collections::HashMap;
    let idx: HashMap<&str, (i64, i64)> = indexed
        .iter()
        .map(|(p, s, m)| (p.as_str(), (*s, *m)))
        .collect();
    let mut changed = Vec::new();
    for e in on_disk {
        match idx.get(e.rel.as_str()) {
            Some((s, m)) if *s == e.size && *m == e.mtime_ns => {}
            _ => changed.push(e.rel.clone()),
        }
    }
    let disk: std::collections::HashSet<&str> = on_disk.iter().map(|e| e.rel.as_str()).collect();
    let removed = indexed
        .iter()
        .filter(|(p, _, _)| !disk.contains(p.as_str()))
        .map(|(p, _, _)| p.clone())
        .collect();
    (changed, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_skips_hidden_trash_and_lists_notes_vs_files() {
        let td = tempfile::tempdir().unwrap();
        let r = td.path();
        std::fs::create_dir_all(r.join("sub/.git")).unwrap();
        std::fs::create_dir_all(r.join(".trash")).unwrap();
        std::fs::create_dir_all(r.join(".obsidian")).unwrap();
        std::fs::write(r.join("a.md"), "x").unwrap();
        std::fs::write(r.join("sub/b.MD"), "y").unwrap();
        std::fs::write(r.join("sub/pic.png"), [1, 2, 3]).unwrap();
        std::fs::write(r.join(".trash/gone.md"), "z").unwrap();
        std::fs::write(r.join(".DS_Store"), "m").unwrap();
        let w = walk(r).unwrap();
        assert_eq!(
            w.notes.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
            vec!["a.md", "sub/b.MD"]
        );
        assert_eq!(
            w.files.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
            vec!["sub/pic.png"]
        );
    }

    #[test]
    fn walk_skips_symlinks() {
        let td = tempfile::tempdir().unwrap();
        let r = td.path();
        std::fs::create_dir_all(r.join("docs")).unwrap();
        std::fs::write(r.join("README.md"), "x").unwrap();
        std::os::unix::fs::symlink("../README.md", r.join("docs/README.md")).unwrap();
        std::os::unix::fs::symlink("README.md", r.join("link.png")).unwrap();
        let w = walk(r).unwrap();
        assert!(w.complete);
        assert_eq!(
            w.notes.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
            vec!["README.md"]
        );
        assert!(w.files.is_empty());
    }

    #[test]
    fn exact_regular_file_matches_bytes_not_case_or_symlinks() {
        let td = tempfile::tempdir().unwrap();
        let r = td.path();
        std::fs::create_dir_all(r.join("Docs")).unwrap();
        std::fs::write(r.join("Docs/Note.md"), "x").unwrap();
        std::os::unix::fs::symlink("Docs/Note.md", r.join("link.md")).unwrap();
        assert!(exact_regular_file(r, "Docs/Note.md").unwrap());
        // On case-insensitive APFS these stat fine, yet are not the entry.
        assert!(!exact_regular_file(r, "Docs/note.md").unwrap());
        assert!(!exact_regular_file(r, "docs/Note.md").unwrap());
        assert!(!exact_regular_file(r, "link.md").unwrap());
        assert!(!exact_regular_file(r, "Docs").unwrap());
        assert!(!exact_regular_file(r, "missing/x.md").unwrap());
    }

    #[test]
    fn diff_detects_add_change_remove() {
        let disk = vec![
            WalkEntry {
                rel: "a.md".into(),
                size: 5,
                mtime_ns: 100,
            },
            WalkEntry {
                rel: "b.md".into(),
                size: 9,
                mtime_ns: 300,
            },
        ];
        let indexed = vec![
            ("a.md".to_string(), 5i64, 100i64),
            ("c.md".to_string(), 1, 1),
        ];
        let (changed, removed) = diff(&disk, &indexed);
        assert_eq!(changed, vec!["b.md"]);
        assert_eq!(removed, vec!["c.md"]);
    }
}

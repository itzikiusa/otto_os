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
    pub notes: Vec<WalkEntry>,
    pub files: Vec<WalkEntry>,
}

/// Recursively list the vault. Blocking — call from `spawn_blocking`.
pub fn walk(root: &Path) -> std::io::Result<WalkResult> {
    let mut notes = Vec::new();
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue, // permission race — skip subtree, never abort
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if !is_skipped_dir(&name) {
                    stack.push(path);
                }
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let size = meta.len() as i64;
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            let e = WalkEntry { rel, size, mtime_ns };
            if name.to_lowercase().ends_with(".md") {
                notes.push(e);
            } else {
                files.push(e);
            }
        }
    }
    notes.sort_by(|a, b| a.rel.cmp(&b.rel));
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(WalkResult { notes, files })
}

/// Diff a walk against the indexed signatures → (added_or_changed, removed).
pub fn diff(
    on_disk: &[WalkEntry],
    indexed: &[(String, i64, i64)],
) -> (Vec<String>, Vec<String>) {
    use std::collections::HashMap;
    let idx: HashMap<&str, (i64, i64)> =
        indexed.iter().map(|(p, s, m)| (p.as_str(), (*s, *m))).collect();
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
        assert_eq!(w.notes.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(), vec!["a.md", "sub/b.MD"]);
        assert_eq!(w.files.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(), vec!["sub/pic.png"]);
    }

    #[test]
    fn diff_detects_add_change_remove() {
        let disk = vec![
            WalkEntry { rel: "a.md".into(), size: 5, mtime_ns: 100 },
            WalkEntry { rel: "b.md".into(), size: 9, mtime_ns: 300 },
        ];
        let indexed = vec![("a.md".to_string(), 5i64, 100i64), ("c.md".to_string(), 1, 1)];
        let (changed, removed) = diff(&disk, &indexed);
        assert_eq!(changed, vec!["b.md"]);
        assert_eq!(removed, vec!["c.md"]);
    }
}

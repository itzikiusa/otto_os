//! One-time, READ-ONLY seed of assistant memories from a Hermes install
//! (`~/.hermes/memories/*.md`, entries separated by a `§` line — plan §2.3,
//! Decision 7). This module only ever OPENS FILES FOR READING: it never
//! creates, writes, renames, locks or deletes anything under `~/.hermes`, and
//! it is only reached from the user-triggered import endpoint (never a tick).
//! Imported entries land as `pending` memories for review, never accepted.

use std::path::{Path, PathBuf};

/// Per-file read cap. Hermes caps its memory files well below this.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Per-entry cap (chars) — a longer entry is truncated, not dropped.
pub const MAX_ENTRY_CHARS: usize = 4000;
/// Total entries one import will queue.
pub const MAX_ENTRIES: usize = 500;

/// The entries of one Hermes memory file.
#[derive(Debug, Clone, PartialEq)]
pub struct HermesFile {
    /// File name only (`MEMORY.md`, `USER.md`), never a path.
    pub name: String,
    pub entries: Vec<String>,
}

/// `$HOME/.hermes/memories` (tests pass their own dir to [`scan_dir`]).
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join(".hermes").join("memories"))
}

/// Split one file's text into entries: `§` separates entries (Hermes writes
/// it on a line of its own), each entry is trimmed, empty ones are dropped,
/// and overlong ones are truncated to [`MAX_ENTRY_CHARS`].
pub fn parse_entries(content: &str) -> Vec<String> {
    content
        .split('§')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| {
            if e.chars().count() > MAX_ENTRY_CHARS {
                e.chars().take(MAX_ENTRY_CHARS).collect()
            } else {
                e.to_string()
            }
        })
        .collect()
}

/// Read every `*.md` regular file directly in `dir` (no recursion, no
/// symlinks, no lock files), sorted by name. A missing dir is an empty result.
/// Read-only: `symlink_metadata` + `File::open` for reading, nothing else.
pub fn scan_dir(dir: &Path) -> Vec<HermesFile> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.ends_with(".md") && !name.starts_with('.')).then(|| (name, e.path()))
        })
        .collect();
    names.sort();
    let mut out = Vec::new();
    let mut total = 0usize;
    for (name, path) in names {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Some(text) = read_capped(&path) else {
            continue;
        };
        let mut entries = parse_entries(&text);
        entries.truncate(MAX_ENTRIES.saturating_sub(total));
        total += entries.len();
        out.push(HermesFile { name, entries });
        if total >= MAX_ENTRIES {
            break;
        }
    }
    out
}

fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_FILE_BYTES).read_to_end(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// The memory `kind` an entry imports as: `USER.md` holds facts about the
/// user (profile-like), everything else general facts. Both are `fact`; the
/// file name travels in `source.file` and a `hermes` tag.
pub fn tags_for(file: &str) -> Vec<String> {
    let mut t = vec!["hermes".to_string()];
    if file.eq_ignore_ascii_case("USER.md") {
        t.push("profile".into());
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_the_section_sign_and_trims() {
        let text = "Prefers aisle seats.\n§\nLives in Tel Aviv; works at Techch.\n§\n\n§\n  Uses Codex for scripts.  \n";
        assert_eq!(
            parse_entries(text),
            vec![
                "Prefers aisle seats.".to_string(),
                "Lives in Tel Aviv; works at Techch.".to_string(),
                "Uses Codex for scripts.".to_string(),
            ]
        );
        assert!(parse_entries("").is_empty());
        assert!(parse_entries("§\n§").is_empty());
        // A file without separators is one entry.
        assert_eq!(parse_entries("just one fact").len(), 1);
    }

    #[test]
    fn overlong_entries_are_truncated_not_dropped() {
        let long = "x".repeat(MAX_ENTRY_CHARS + 50);
        let e = parse_entries(&long);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].chars().count(), MAX_ENTRY_CHARS);
    }

    #[test]
    fn scan_reads_only_md_files_and_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MEMORY.md"), "a\n§\nb").unwrap();
        std::fs::write(dir.path().join("USER.md"), "c").unwrap();
        std::fs::write(dir.path().join("MEMORY.md.lock"), "").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        std::fs::create_dir(dir.path().join("sub.md")).unwrap();
        let before: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| (e.file_name(), e.metadata().unwrap().modified().unwrap()))
            .collect();
        let files = scan_dir(dir.path());
        assert_eq!(
            files,
            vec![
                HermesFile {
                    name: "MEMORY.md".into(),
                    entries: vec!["a".into(), "b".into()]
                },
                HermesFile {
                    name: "USER.md".into(),
                    entries: vec!["c".into()]
                },
            ]
        );
        // Nothing was created or touched.
        let after: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| (e.file_name(), e.metadata().unwrap().modified().unwrap()))
            .collect();
        assert_eq!(before.len(), after.len());
        for b in &before {
            assert!(after.contains(b));
        }
    }

    #[test]
    fn a_missing_dir_is_empty_and_symlinks_are_skipped() {
        assert!(scan_dir(Path::new("/nonexistent/hermes/memories")).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("LINK.md"))
            .unwrap();
        assert!(scan_dir(dir.path()).is_empty());
    }

    #[test]
    fn user_md_is_tagged_profile() {
        assert_eq!(tags_for("USER.md"), vec!["hermes", "profile"]);
        assert_eq!(tags_for("MEMORY.md"), vec!["hermes"]);
    }
}

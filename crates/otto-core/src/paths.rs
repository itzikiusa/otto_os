//! Small path helpers shared across crates.

use std::path::{Component, Path, PathBuf};

/// Validate a user-supplied string that must name a SINGLE path component
/// (an id, a file name) before it is joined under a trusted directory.
/// Rejects empty names, `.`/`..`, separators, and NULs — anything that could
/// make `dir.join(name)` land outside `dir`.
pub fn safe_component(name: &str) -> Option<&str> {
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.contains(['/', '\\', '\0']) {
        return None;
    }
    Some(name)
}

/// Join a user-supplied RELATIVE path under a trusted `root`, refusing any
/// input that could escape it: absolute paths, drive prefixes, and `..`
/// components are all rejected (lexically, so it also covers paths that do
/// not exist yet). Returns the confined path on success.
pub fn confine_join(root: &Path, candidate: &str) -> Option<PathBuf> {
    let rel = Path::new(candidate);
    if rel.is_absolute() {
        return None;
    }
    let mut out = root.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => {
                let s = c.to_str()?;
                if s.contains('\0') {
                    return None;
                }
                out.push(s);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // Belt-and-braces: the lexical build above cannot escape, but keep the
    // invariant explicit so a future edit can't silently drop it.
    if !out.starts_with(root) {
        return None;
    }
    Some(out)
}

/// Verify an EXISTING path resolves inside `root` after symlink resolution.
/// Use for read paths where the target must already live under a trusted
/// directory; for not-yet-created files, confine the parent instead.
pub fn resolves_under(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root_canon = root.canonicalize().ok()?;
    let canon = candidate.canonicalize().ok()?;
    if !canon.starts_with(&root_canon) {
        return None;
    }
    Some(canon)
}

/// Expand a leading `~` / `~/` to the user's home directory.
///
/// Paths typed into repo/project fields routinely arrive tilde-form, but the
/// filesystem treats a literal `~` as a relative directory named "~": spawning
/// a session there silently falls back to `$HOME`, `canonicalize` fails, and
/// anything derived from the string (e.g. the claude transcript project dir)
/// points at a directory that never exists. Expand before persisting or using
/// a user-supplied path as a cwd. Unknown home (no `$HOME`/`$USERPROFILE`)
/// returns the input unchanged.
pub fn expand_tilde(p: &str) -> String {
    let home = || std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    if p == "~" {
        return home().unwrap_or_else(|| p.to_string());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = home() {
            return format!("{}/{rest}", h.trim_end_matches('/'));
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_forms() {
        let home = std::env::var("HOME").expect("HOME set in tests");
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/ikariam_style4"), format!("{home}/ikariam_style4"));
    }

    #[test]
    fn safe_component_filters_traversal() {
        assert_eq!(safe_component("snip-01ab"), Some("snip-01ab"));
        assert_eq!(safe_component("a.png"), Some("a.png"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b", "../x"] {
            assert_eq!(safe_component(bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn confine_join_stays_under_root() {
        let root = Path::new("/data/store");
        assert_eq!(confine_join(root, "a/b.txt"), Some(PathBuf::from("/data/store/a/b.txt")));
        assert_eq!(confine_join(root, "./a"), Some(PathBuf::from("/data/store/a")));
        for bad in ["../a", "a/../../b", "/etc/passwd", "a/../../../b"] {
            assert_eq!(confine_join(root, bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn resolves_under_rejects_escapes() {
        let dir = std::env::temp_dir().join(format!("pathguard-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner/f.txt"), "x").unwrap();
        assert!(resolves_under(&dir, &dir.join("inner/f.txt")).is_some());
        assert!(resolves_under(&dir.join("inner"), &dir.join("inner/../inner")).is_some());
        assert!(resolves_under(&dir.join("inner"), &dir).is_none());
        assert!(resolves_under(&dir, &dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_other_paths_alone() {
        assert_eq!(expand_tilde("/tmp/repo"), "/tmp/repo");
        assert_eq!(expand_tilde("rel/dir"), "rel/dir");
        // `~user` expansion is intentionally unsupported — pass through.
        assert_eq!(expand_tilde("~bob/x"), "~bob/x");
    }
}

//! Small path helpers shared across crates.

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
    fn leaves_other_paths_alone() {
        assert_eq!(expand_tilde("/tmp/repo"), "/tmp/repo");
        assert_eq!(expand_tilde("rel/dir"), "rel/dir");
        // `~user` expansion is intentionally unsupported — pass through.
        assert_eq!(expand_tilde("~bob/x"), "~bob/x");
    }
}

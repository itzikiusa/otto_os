//! Small path helpers shared across crates.

use std::path::{Component, Path, PathBuf};

/// Files Otto's session launchers write into an agent's working directory,
/// root-relative: the claude/codex `.mcp.json`, agy's
/// `.agents/mcp_config.json` and grok's `.grok/config.toml`. They carry the
/// absolute `ottod` path and the resolved env (secrets included) of every
/// enabled MCP server, so they are runtime state, never work product: the
/// workflow auto-commit excludes them and the Otto Run proof gate ignores
/// them while untracked. One list so a new provider's file is added once.
pub const LAUNCHER_ARTIFACTS: [&str; 3] =
    [".mcp.json", ".agents/mcp_config.json", ".grok/config.toml"];

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
    let home = || {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()
    };
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

/// [`expand_tilde`] for a user-supplied path, as a `PathBuf`, refusing
/// `~user/…`: it names another account's home, and the old any-`~` prefix
/// strip copies turned `~bob/notes.txt` into `$HOMEbob/notes.txt` — a
/// confusing not-found (or a write somewhere unexpected) for what the user
/// typed. `Err` is a user-facing message for a 400.
pub fn expand_home_strict(p: &str) -> Result<PathBuf, String> {
    if p.starts_with('~') && p != "~" && !p.starts_with("~/") {
        return Err(format!(
            "unsupported home path {p:?}: use ~/… or an absolute path"
        ));
    }
    Ok(PathBuf::from(expand_tilde(p)))
}

/// True when `path` is the filesystem root, an OS/system container directory,
/// a shared temp root, or the current user's `$HOME` itself — directories that
/// must never be auto-registered, recursively watched, or scanned as a single
/// standalone Git project root (e.g. when a dotfile repo `.git` lives in
/// `$HOME`). Subdirectories *inside* `$HOME` or `/tmp` (such as
/// `~/koala-common-go` or `/tmp/my-repo`) return `false`.
pub fn is_broad_non_project_root(path: &Path) -> bool {
    const BROAD_ROOTS: &[&str] = &[
        "/",
        "/Users",
        "/home",
        "/System",
        "/Library",
        "/Applications",
        "/Volumes",
        "/Network",
        "/cores",
        "/dev",
        "/etc",
        "/private/etc",
        "/opt",
        "/proc",
        "/sbin",
        "/bin",
        "/usr",
        "/tmp",
        "/private/tmp",
        "/var",
        "/private/var",
        "/var/folders",
        "/private/var/folders",
    ];
    let check_one = |p: &Path| -> bool {
        let trimmed = p.to_string_lossy();
        let norm = if trimmed.len() > 1 {
            trimmed.trim_end_matches('/')
        } else {
            trimmed.as_ref()
        };
        if BROAD_ROOTS.contains(&norm) {
            return true;
        }
        for var in ["HOME", "USERPROFILE"] {
            if let Ok(home) = std::env::var(var) {
                let h = home.trim_end_matches('/');
                if !h.is_empty() && norm == h {
                    return true;
                }
                if let Ok(hc) = Path::new(h).canonicalize() {
                    if p == hc.as_path() {
                        return true;
                    }
                }
            }
        }
        false
    };
    if check_one(path) {
        return true;
    }
    if let Ok(canon) = path.canonicalize() {
        if check_one(&canon) {
            return true;
        }
    }
    false
}

/// Encode an absolute filesystem path into an RFC 3986 `file://` URI,
/// percent-encoding spaces, `#`, `?`, `%`, brackets, and non-ASCII UTF-8 bytes
/// so LSP servers and URI parsers never truncate query/fragment characters or
/// reject unencoded whitespace.
pub fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut normalized = raw.replace('\\', "/");
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    let mut out = String::with_capacity("file://".len() + normalized.len() + 16);
    out.push_str("file://");
    let bytes = normalized.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        let is_win_drive_colon =
            idx == 2 && b == b':' && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic();
        let unreserved_or_sep = b.is_ascii_alphanumeric()
            || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~')
            || is_win_drive_colon;
        if unreserved_or_sep {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Decode an RFC 3986 / RFC 8089 `file:` URI (`file:///path`,
/// `file://localhost/path`, `file:/path`, or `file:///C:/path`, with
/// ASCII-case-insensitive scheme and `localhost` authority) into a `PathBuf`
/// with full UTF-8 percent-decoding. Returns `None` if `uri` is not a local
/// `file:` URI or decodes to invalid UTF-8 / NUL bytes.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let after_scheme = uri
        .as_bytes()
        .get(..5)
        .filter(|p| p.eq_ignore_ascii_case(b"file:"))
        .map(|_| &uri[5..])?;
    let path_part = if let Some(rest) = after_scheme.strip_prefix("//") {
        if rest.starts_with('/') {
            rest.to_string()
        } else if rest
            .as_bytes()
            .get(..10)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"localhost/"))
        {
            format!("/{}", &rest[10..])
        } else {
            return None;
        }
    } else if after_scheme.starts_with('/') {
        after_scheme.to_string()
    } else {
        return None;
    };
    let decoded = percent_decode_utf8(&path_part)?;
    if decoded.contains('\0') {
        return None;
    }
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return Some(PathBuf::from(&decoded[1..]));
    }
    Some(PathBuf::from(decoded))
}

/// Decode `%XX` sequences as raw bytes and validate UTF-8 (never `b as char`
/// Latin-1 widening).
pub fn percent_decode_utf8(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Recursively copy a directory (otto-server: installs a local plugin
/// into the home and stages review-lens skills into a shared `--add-dir` bundle),
/// skipping `.git`, `node_modules` and `target`.
#[allow(clippy::disallowed_methods)] // sync helper: blocking by contract — install runs it via offload::blocking
pub fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        // Skip VCS + heavy build dirs.
        if matches!(
            name.to_str(),
            Some(".git") | Some("node_modules") | Some("target")
        ) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_strict_expands_only_the_callers_own_home() {
        let home = std::env::var("HOME").expect("HOME set in tests");
        assert_eq!(expand_home_strict("~").unwrap(), PathBuf::from(&home));
        assert_eq!(
            expand_home_strict("~/notes.txt").unwrap(),
            PathBuf::from(format!("{}/notes.txt", home.trim_end_matches('/')))
        );
        assert!(expand_home_strict("~bob/notes.txt").is_err());
        assert!(expand_home_strict("~bob").is_err());
        assert_eq!(
            expand_home_strict("/abs/x").unwrap(),
            PathBuf::from("/abs/x")
        );
        assert_eq!(
            expand_home_strict("rel/~x").unwrap(),
            PathBuf::from("rel/~x")
        );
    }

    #[test]
    fn expands_home_forms() {
        let home = std::env::var("HOME").expect("HOME set in tests");
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(
            expand_tilde("~/ikariam_style4"),
            format!("{home}/ikariam_style4")
        );
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
        assert_eq!(
            confine_join(root, "a/b.txt"),
            Some(PathBuf::from("/data/store/a/b.txt"))
        );
        assert_eq!(
            confine_join(root, "./a"),
            Some(PathBuf::from("/data/store/a"))
        );
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

    #[test]
    fn broad_non_project_root_rejects_home_and_system_roots_but_allows_subprojects() {
        let home = std::env::var("HOME").expect("HOME set in tests");
        assert!(is_broad_non_project_root(Path::new("/")));
        assert!(is_broad_non_project_root(Path::new("/Users")));
        assert!(is_broad_non_project_root(Path::new("/tmp")));
        assert!(is_broad_non_project_root(Path::new("/private/tmp")));
        assert!(is_broad_non_project_root(Path::new(&home)));
        assert!(!is_broad_non_project_root(
            &Path::new(&home).join("koala-common-go")
        ));
        assert!(!is_broad_non_project_root(Path::new("/tmp/my-test-repo")));
    }

    #[test]
    fn file_uri_round_trips_spaces_fragments_unicode_and_localhost() {
        let p = Path::new("/tmp/my dir/c#?%file 🚀 שלום.go");
        let uri = path_to_file_uri(p);
        assert!(uri.starts_with("file:///tmp/my%20dir/"));
        assert!(!uri.contains(' ') && !uri.contains('#') && !uri.contains('?'));
        assert_eq!(file_uri_to_path(&uri).as_deref(), Some(p));
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/a%20b.rs").as_deref(),
            Some(Path::new("/tmp/a b.rs"))
        );
        assert_eq!(
            file_uri_to_path("file:///C:/Users/dev/main.go").as_deref(),
            Some(Path::new("C:/Users/dev/main.go"))
        );
        assert_eq!(
            file_uri_to_path("file:/tmp/rfc8089%20single.go").as_deref(),
            Some(Path::new("/tmp/rfc8089 single.go"))
        );
        assert_eq!(
            file_uri_to_path("FILE://LOCALHOST/tmp/case.rs").as_deref(),
            Some(Path::new("/tmp/case.rs"))
        );
        assert!(file_uri_to_path("file://evil.example/tmp/a").is_none());
        assert!(file_uri_to_path("https://example.com/a").is_none());
    }
}

//! Credential / Otto-state locations that NON-root callers may never reach
//! through a daemon file surface (`/fs/*`, session artifacts, vault roots and
//! vault assets). Shared so every surface refuses the same set: Otto's own
//! data dir (state DB, backups, `secrets.json`, TLS keys, logs), the home
//! credential dirs (`~/.ssh`, `~/.aws`, the login Keychain, …), system secret
//! prefixes, and key-like file names.
//!
//! Every check takes an ALREADY-CANONICAL path (symlinks + `..` resolved) —
//! callers canonicalize first so a link can't dodge a prefix.

use std::path::{Path, PathBuf};

/// Directories (relative to `$HOME`) that hold credentials/secrets.
pub const HOME_DENY_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".config/gcloud",
    ".config/gh",
    ".azure",
    ".password-store",
    // S8-09 / S11-08: more credential stores a non-root caller must not read.
    ".terraform.d",
    ".config/op",
    // Otto's per-user home (agent context, MCP config). Its `vault/` subtree is
    // the managed vault home and stays a valid vault root — see
    // [`VAULT_HOME`].
    ".otto",
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Firefox",
    "Library/Application Support/Arc",
];

/// Individual credential FILES (relative to `$HOME`) whose directories stay
/// browsable (agent CLI homes, git and cargo config).
pub const HOME_DENY_FILES: &[&str] = &[
    ".codex/auth.json",
    ".claude/.credentials.json",
    ".claude.json",
    ".git-credentials",
    ".config/git/credentials",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gemini/oauth_creds.json",
];

/// The managed vault home (relative to `$HOME`): `register` with no root
/// creates `~/.otto/vault/<slug>`, so a root strictly inside it is not refused
/// even though `~/.otto` is a protected dir.
pub const VAULT_HOME: &str = ".otto/vault";

/// Absolute prefixes that are system secret stores.
pub const ABS_DENY_PREFIXES: &[&str] = &[
    "/etc",
    "/private/etc",
    "/root",
    "/var/root",
    "/proc",
    "/sys",
];

/// Exact (case-insensitive) file names that are known secret stores.
pub const DENY_FILE_NAMES: &[&str] = &[
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "credentials",
    ".env",
    ".netrc",
    ".pgpass",
    ".npmrc",
    ".pypirc",
    ".dockercfg",
    ".git-credentials",
    // Shell / REPL histories routinely hold pasted tokens and passwords.
    ".bash_history",
    ".zsh_history",
    ".sh_history",
    ".python_history",
    ".node_repl_history",
    ".psql_history",
    ".mysql_history",
    ".rediscli_history",
];

/// File-name suffixes marking a likely secret (private keys, keystores).
pub const DENY_FILE_SUFFIXES: &[&str] = &[".pem", ".key", ".pfx", ".p12", ".keystore"];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Canonical form when the path exists, the lexical one otherwise (a data dir
/// that isn't created yet must still be refused by prefix).
fn canon_or_lexical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// The canonical `$HOME`, when known.
pub fn home_dir() -> Option<PathBuf> {
    home().map(|h| canon_or_lexical(&h))
}

/// Otto's data dirs: `$OTTO_DATA_DIR` when set AND the default
/// `~/Library/Application Support/Otto` (refused even when the env points
/// elsewhere — an older install's state lives there).
pub fn otto_data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("OTTO_DATA_DIR").filter(|d| !d.is_empty()) {
        out.push(canon_or_lexical(Path::new(&d)));
    }
    if let Some(h) = home() {
        out.push(canon_or_lexical(
            &h.join("Library/Application Support/Otto"),
        ));
    }
    out
}

/// Every protected directory: Otto's data dirs, the home credential dirs and
/// the absolute system prefixes.
pub fn protected_dirs() -> Vec<PathBuf> {
    let mut out = otto_data_dirs();
    if let Some(h) = home_dir() {
        out.extend(HOME_DENY_DIRS.iter().map(|rel| h.join(rel)));
    }
    out.extend(ABS_DENY_PREFIXES.iter().map(PathBuf::from));
    out
}

/// True when the canonical `path` is (inside) one of Otto's data dirs.
pub fn in_otto_data_dir(canonical: &Path) -> bool {
    otto_data_dirs().iter().any(|d| canonical.starts_with(d))
}

/// True when `canonical` is inside a home credential dir or a system secret
/// prefix.
pub fn is_denied_dir(canonical: &Path) -> bool {
    if let Some(h) = home_dir() {
        if HOME_DENY_DIRS
            .iter()
            .any(|rel| canonical.starts_with(h.join(rel)))
        {
            return true;
        }
    }
    ABS_DENY_PREFIXES
        .iter()
        .any(|p| canonical.starts_with(Path::new(p)))
}

/// True when `canonical` names a known secret file (by exact name or suffix,
/// or as one of the home credential files in [`HOME_DENY_FILES`]).
pub fn is_denied_file(canonical: &Path) -> bool {
    if let Some(h) = home_dir() {
        if HOME_DENY_FILES.iter().any(|rel| canonical == h.join(rel)) {
            return true;
        }
    }
    let name = match canonical.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return false,
    };
    if DENY_FILE_NAMES
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&name))
    {
        return true;
    }
    DENY_FILE_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// True when `canonical` lies in a protected DIRECTORY (data dir, credential
/// dir, system prefix) — file names aside.
pub fn in_protected_dir(canonical: &Path) -> bool {
    in_otto_data_dir(canonical) || is_denied_dir(canonical)
}

/// Why a directory may not become a ROOT that serves its whole subtree to
/// other users (a vault root): it is `/` or `$HOME`, lies inside a protected
/// dir, or CONTAINS one (an ancestor of `~/.ssh` or of the data dir exposes
/// it). `None` = allowed. `canonical` must be absolute and canonical.
pub fn subtree_root_denial(canonical: &Path) -> Option<String> {
    if canonical.parent().is_none() {
        return Some("the filesystem root cannot be a shared root".into());
    }
    if home_dir().is_some_and(|h| canonical == h) {
        return Some("the home directory cannot be a shared root".into());
    }
    let in_vault_home = home_dir().is_some_and(|h| {
        let vh = h.join(VAULT_HOME);
        canonical != vh && canonical.starts_with(&vh)
    });
    if in_protected_dir(canonical) && !in_vault_home {
        return Some(format!(
            "{} holds credentials or Otto's own state",
            canonical.display()
        ));
    }
    if let Some(p) = protected_dirs().iter().find(|p| p.starts_with(canonical)) {
        return Some(format!(
            "{} contains {}, which holds credentials or Otto's own state",
            canonical.display(),
            p.display()
        ));
    }
    None
}

/// Canonicalize a path that may not exist yet: the deepest EXISTING ancestor
/// is canonicalized (resolving symlinks) and the missing tail re-joined
/// lexically. Refuses relative input and a `..`/`.` in the missing tail so the
/// result can't be steered after the check. `None` = unusable path.
pub fn canonicalize_prospective(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut existing = path;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if existing.exists() {
            break;
        }
        tail.push(existing.file_name()?);
        existing = existing.parent()?;
    }
    if path
        .strip_prefix(existing)
        .ok()?
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let mut out = existing.canonicalize().ok()?;
    for seg in tail.into_iter().rev() {
        out.push(seg);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_home_and_their_contents_are_refused_as_subtree_roots() {
        assert!(subtree_root_denial(Path::new("/")).is_some());
        if let Some(h) = home_dir() {
            assert!(subtree_root_denial(&h).is_some(), "home");
            assert!(subtree_root_denial(&h.join(".ssh")).is_some(), "~/.ssh");
            assert!(
                subtree_root_denial(&h.join("Library")).is_some(),
                "~/Library contains the data dir + Keychains"
            );
            assert!(
                subtree_root_denial(&h.join("Library/Application Support/Otto/backups")).is_some(),
                "inside the data dir"
            );
            if !in_protected_dir(&h) {
                assert!(subtree_root_denial(&h.join("Documents/notes")).is_none());
                assert!(
                    subtree_root_denial(&h.join(VAULT_HOME).join("notes")).is_none(),
                    "a default vault under the managed vault home"
                );
                assert!(subtree_root_denial(&h.join(VAULT_HOME)).is_some());
                assert!(subtree_root_denial(&h.join(".otto/context")).is_some());
            }
        }
        assert!(
            subtree_root_denial(Path::new("/private")).is_some(),
            "contains /private/etc"
        );
    }

    #[test]
    fn prospective_canonicalization_refuses_relative_and_dotdot_tails() {
        assert!(canonicalize_prospective(Path::new("relative/dir")).is_none());
        let td = std::env::temp_dir();
        assert!(canonicalize_prospective(&td.join("nope-xyz/../../etc")).is_none());
        let p = canonicalize_prospective(&td.join("otto-prospective-xyz/a")).unwrap();
        assert!(p.ends_with("otto-prospective-xyz/a"));
        assert!(p.starts_with(td.canonicalize().unwrap()));
    }

    #[test]
    fn key_like_names_are_denied_files() {
        assert!(is_denied_file(Path::new("/x/id_rsa")));
        assert!(is_denied_file(Path::new("/x/server.PEM")));
        assert!(!is_denied_file(Path::new("/x/notes.md")));
        assert!(is_denied_file(Path::new("/x/.zsh_history")));
        if let Some(h) = home_dir() {
            assert!(is_denied_file(&h.join(".claude.json")));
        }
    }
}

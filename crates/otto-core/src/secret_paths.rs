//! Credential / Otto-state locations that NON-root callers may never reach
//! through a daemon file surface (`/fs/*`, session artifacts, vault roots and
//! vault assets). Shared so every surface refuses the same set: Otto's own
//! data dir (state DB, backups, `secrets.json`, TLS keys, logs), the home
//! credential dirs (`~/.ssh`, `~/.aws`, the login Keychain, …), system secret
//! prefixes, and key-like file names.
//!
//! Every check takes an ALREADY-CANONICAL path (symlinks + `..` resolved) —
//! callers canonicalize first so a link can't dodge a prefix. Canonical is not
//! unique on macOS (`/System/Volumes/Data/Users/…` firmlinks, case-insensitive
//! APFS), so the dir checks compare `(st_dev, st_ino)` identities too (S7-301).

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
    // S7-302: the agent CLIs Otto drives keep OAuth/API tokens, transcripts
    // and env-bearing settings in their homes; `~/.config` holds many more
    // tool tokens than the gh/gcloud/op entries above.
    ".codex",
    ".gemini",
    ".claude",
    ".cursor",
    ".config",
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

/// Individual credential FILES (relative to `$HOME`) — kept although most of
/// their directories are now protected dirs too (git and cargo config stay
/// browsable).
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
    "/private/var/root",
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

/// The APFS data-volume firmlink root: `/System/Volumes/Data/Users/<u>` IS
/// `/Users/<u>`, and `realpath` keeps the long prefix (S7-301).
const FIRMLINK_DATA: &str = "/System/Volumes/Data";

/// `p` with a leading `/System/Volumes/Data` firmlink prefix folded away
/// (`/System/Volumes/Data` itself → `/`), so the lexical prefix checks see
/// the same spelling as `$HOME`-derived paths. Other paths are unchanged.
pub fn strip_firmlink(p: &Path) -> PathBuf {
    match p.strip_prefix(FIRMLINK_DATA) {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => p.to_path_buf(),
    }
}

/// Canonical form when the path exists, the lexical one otherwise (a data dir
/// that isn't created yet must still be refused by prefix) — firmlink-folded.
fn canon_or_lexical(p: &Path) -> PathBuf {
    strip_firmlink(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

/// The canonical `$HOME`, when known.
pub fn home_dir() -> Option<PathBuf> {
    home().map(|h| canon_or_lexical(&h))
}

/// Otto's log dirs: `$OTTO_LOG_DIR` when set and the installed daemon's
/// `~/Library/Logs/Otto` (`GET /logs/daemon` is root-only; a vault rooted
/// there must not serve `ottod.log` to Viewers — S7-302).
fn otto_log_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("OTTO_LOG_DIR").filter(|d| !d.is_empty()) {
        out.push(canon_or_lexical(Path::new(&d)));
    }
    if let Some(h) = home() {
        out.push(canon_or_lexical(&h.join("Library/Logs/Otto")));
    }
    out
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

/// Every protected directory: Otto's data + log dirs, the home credential
/// dirs and the absolute system prefixes.
pub fn protected_dirs() -> Vec<PathBuf> {
    protected_set()
        .entries
        .iter()
        .map(|e| e.path.clone())
        .collect()
}

/// A file's identity: `(st_dev, st_ino)`. Two spellings of one directory
/// (firmlink, case variant on case-insensitive APFS, a mount alias) share it.
type FileId = (u64, u64);

#[cfg(unix)]
fn file_id(p: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn file_id(_p: &Path) -> Option<FileId> {
    None
}

/// The identity of an already-read `metadata` (a scan's `DirEntry`), so a
/// walk can test each directory without another syscall.
#[cfg(unix)]
pub fn metadata_id(m: &std::fs::Metadata) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    Some((m.dev(), m.ino()))
}

#[cfg(not(unix))]
pub fn metadata_id(_m: &std::fs::Metadata) -> Option<FileId> {
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Otto's own state: data dirs + log dirs.
    Otto,
    /// Credential dirs and system secret prefixes.
    Credential,
}

struct Entry {
    /// Canonical (or lexical when missing), firmlink-folded.
    path: PathBuf,
    kind: Kind,
    /// Identity of the dir itself, when it exists.
    id: Option<FileId>,
    /// Identities of the dir and every existing ancestor — "does `x`
    /// CONTAIN this protected dir" is `id(x) ∈ ancestors`.
    ancestors: Vec<FileId>,
}

/// A snapshot of every protected dir, compared BY IDENTITY as well as by
/// (firmlink-folded) prefix: a lexical `starts_with` alone is dodged by
/// `/System/Volumes/Data/Users/<u>/…` or a case variant (S7-301). Get one with
/// [`protected_set`] (cached a few seconds) and reuse it across a walk.
pub struct ProtectedSet {
    entries: Vec<Entry>,
    home: Option<(PathBuf, Option<FileId>)>,
}

impl ProtectedSet {
    fn build() -> Self {
        let mut raw: Vec<(PathBuf, Kind)> = Vec::new();
        raw.extend(otto_data_dirs().into_iter().map(|p| (p, Kind::Otto)));
        raw.extend(otto_log_dirs().into_iter().map(|p| (p, Kind::Otto)));
        let home = home_dir();
        if let Some(h) = &home {
            raw.extend(
                HOME_DENY_DIRS
                    .iter()
                    .map(|rel| (canon_or_lexical(&h.join(rel)), Kind::Credential)),
            );
        }
        // Both the literal prefix and its canonical form (`/etc` →
        // `/private/etc`, `/var/root` → `/private/var/root`).
        for p in ABS_DENY_PREFIXES {
            raw.push((PathBuf::from(p), Kind::Credential));
            raw.push((canon_or_lexical(Path::new(p)), Kind::Credential));
        }
        raw.extend(
            extra_dirs()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|p| (canon_or_lexical(p), Kind::Otto)),
        );
        raw.sort();
        raw.dedup();
        let entries = raw
            .into_iter()
            .map(|(path, kind)| {
                let id = file_id(&path);
                let ancestors = path.ancestors().filter_map(file_id).collect();
                Entry {
                    path,
                    kind,
                    id,
                    ancestors,
                }
            })
            .collect();
        ProtectedSet {
            entries,
            home: home.map(|h| {
                let id = file_id(&h);
                (h, id)
            }),
        }
    }

    /// True when `canonical` is (inside) a protected dir of `kind` (any kind
    /// when `None`): a firmlink-folded prefix match, or any existing ancestor
    /// of it (itself included) IS a protected dir by identity.
    fn inside(&self, canonical: &Path, kind: Option<Kind>) -> bool {
        let wanted = |e: &&Entry| kind.is_none_or(|k| e.kind == k);
        let folded = strip_firmlink(canonical);
        if self
            .entries
            .iter()
            .filter(wanted)
            .any(|e| folded.starts_with(&e.path))
        {
            return true;
        }
        let ids: Vec<FileId> = self
            .entries
            .iter()
            .filter(wanted)
            .filter_map(|e| e.id)
            .collect();
        canonical
            .ancestors()
            .filter_map(file_id)
            .any(|a| ids.contains(&a))
    }

    /// True when `canonical` is inside any protected dir.
    pub fn in_protected_dir(&self, canonical: &Path) -> bool {
        self.inside(canonical, None)
    }

    /// True when the directory `dir` (with identity `id`, e.g. from a scan's
    /// `DirEntry` metadata) IS a protected dir. A walk that never descends
    /// into one only needs this, not the ancestor walk.
    pub fn is_protected_dir(&self, dir: &Path, id: Option<FileId>) -> bool {
        let folded = strip_firmlink(dir);
        self.entries
            .iter()
            .any(|e| e.path == folded || (id.is_some() && e.id == id))
    }

    /// The protected dir that `canonical` CONTAINS (or is), if any — renaming,
    /// trashing or serving `canonical` as a subtree would take it along.
    pub fn contained_in(&self, canonical: &Path) -> Option<PathBuf> {
        let folded = strip_firmlink(canonical);
        if let Some(e) = self.entries.iter().find(|e| e.path.starts_with(&folded)) {
            return Some(e.path.clone());
        }
        let id = file_id(canonical)?;
        self.entries
            .iter()
            .find(|e| e.ancestors.contains(&id))
            .map(|e| e.path.clone())
    }

    /// True when `canonical` is `$HOME` (by spelling or identity).
    fn is_home(&self, canonical: &Path) -> bool {
        let Some((h, hid)) = &self.home else {
            return false;
        };
        strip_firmlink(canonical) == *h || (hid.is_some() && file_id(canonical) == *hid)
    }
}

fn extra_dirs() -> &'static std::sync::Mutex<Vec<PathBuf>> {
    static EXTRA: std::sync::OnceLock<std::sync::Mutex<Vec<PathBuf>>> = std::sync::OnceLock::new();
    EXTRA.get_or_init(Default::default)
}

static EXTRA_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tests only: protect one more (Otto-state) dir for the rest of the process —
/// a stand-in for `$OTTO_DATA_DIR` that needs no process-env mutation (which
/// races parallel tests). Additive, so a unique temp dir never affects
/// another test.
#[doc(hidden)]
pub fn protect_dir_for_tests(dir: &Path) {
    extra_dirs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(dir.to_path_buf());
    EXTRA_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// The current [`ProtectedSet`]. Rebuilt when `$HOME` / `$OTTO_DATA_DIR` /
/// `$OTTO_LOG_DIR` change and at most every few seconds otherwise (a
/// credential dir created later gets its identity on the next rebuild; its
/// lexical prefix applies at once).
pub fn protected_set() -> std::sync::Arc<ProtectedSet> {
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Key = ([Option<std::ffi::OsString>; 3], u64);
    type Cached = (Key, Instant, Arc<ProtectedSet>);
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    const TTL: Duration = Duration::from_secs(5);
    let key: Key = (
        [
            std::env::var_os("HOME"),
            std::env::var_os("OTTO_DATA_DIR"),
            std::env::var_os("OTTO_LOG_DIR"),
        ],
        EXTRA_GENERATION.load(std::sync::atomic::Ordering::SeqCst),
    );
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Some((k, at, set)) = cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if *k == key && at.elapsed() < TTL {
            return set.clone();
        }
    }
    let set = Arc::new(ProtectedSet::build());
    *cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, Instant::now(), set.clone()));
    set
}

/// True when the canonical `path` is (inside) one of Otto's data / log dirs.
pub fn in_otto_data_dir(canonical: &Path) -> bool {
    protected_set().inside(canonical, Some(Kind::Otto))
}

/// True when `canonical` is inside a home credential dir or a system secret
/// prefix.
pub fn is_denied_dir(canonical: &Path) -> bool {
    protected_set().inside(canonical, Some(Kind::Credential))
}

/// True when `canonical` names a known secret file (by exact name or suffix,
/// or as one of the home credential files in [`HOME_DENY_FILES`]).
pub fn is_denied_file(canonical: &Path) -> bool {
    if let Some(h) = home_dir() {
        let folded = strip_firmlink(canonical);
        if HOME_DENY_FILES.iter().any(|rel| folded == h.join(rel)) {
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

/// True when `canonical` lies in a protected DIRECTORY (data dir, log dir,
/// credential dir, system prefix) — file names aside.
pub fn in_protected_dir(canonical: &Path) -> bool {
    protected_set().in_protected_dir(canonical)
}

/// The protected dir `canonical` contains (or is), if any — see
/// [`ProtectedSet::contained_in`].
pub fn contains_protected_dir(canonical: &Path) -> Option<PathBuf> {
    protected_set().contained_in(canonical)
}

/// Why a directory may not become a ROOT that serves its whole subtree to
/// other users (a vault root): it is `/` or `$HOME`, lies inside a protected
/// dir, CONTAINS one (an ancestor of `~/.ssh` or of the data dir exposes it),
/// or sits under a hidden dir of `$HOME` (`~/.codex`, `~/.cursor`, … — agent
/// and tool homes hold tokens; only the managed vault home is exempt).
/// `None` = allowed. `canonical` must be absolute and canonical.
pub fn subtree_root_denial(canonical: &Path) -> Option<String> {
    let set = protected_set();
    let folded = strip_firmlink(canonical);
    if folded.parent().is_none() {
        return Some("the filesystem root cannot be a shared root".into());
    }
    if set.is_home(canonical) {
        return Some("the home directory cannot be a shared root".into());
    }
    let home = home_dir();
    let in_vault_home = home.as_ref().is_some_and(|h| {
        let vh = h.join(VAULT_HOME);
        folded != vh && folded.starts_with(&vh)
    });
    if set.in_protected_dir(canonical) && !in_vault_home {
        return Some(format!(
            "{} holds credentials or Otto's own state",
            canonical.display()
        ));
    }
    if let Some(p) = set.contained_in(canonical) {
        return Some(format!(
            "{} contains {}, which holds credentials or Otto's own state",
            canonical.display(),
            p.display()
        ));
    }
    let hidden_home_dir = home.as_ref().and_then(|h| {
        folded
            .strip_prefix(h)
            .ok()?
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            .then_some(())
    });
    if hidden_home_dir.is_some() && !in_vault_home {
        return Some(format!(
            "{} is inside a hidden directory of the home folder (tool and agent homes hold tokens)",
            canonical.display()
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
    fn firmlink_and_private_aliases_are_refused() {
        // S7-301: `/System/Volumes/Data/...` is the same directory as `/...`
        // on APFS, and `realpath` keeps the long spelling.
        assert!(subtree_root_denial(Path::new("/System/Volumes/Data")).is_some());
        assert!(subtree_root_denial(Path::new("/System/Volumes/Data/private/etc")).is_some());
        assert!(subtree_root_denial(Path::new("/private/etc")).is_some());
        assert!(subtree_root_denial(Path::new("/private/var/root")).is_some());
        assert!(in_protected_dir(Path::new(
            "/System/Volumes/Data/private/etc/hosts"
        )));
        if let Some(h) = home_dir() {
            let fl = Path::new(FIRMLINK_DATA).join(h.strip_prefix("/").unwrap());
            assert!(subtree_root_denial(&fl).is_some(), "firmlinked $HOME");
            assert!(subtree_root_denial(&fl.join("Library/Application Support/Otto")).is_some());
            assert!(in_protected_dir(
                &fl.join("Library/Application Support/Otto/otto.db")
            ));
            assert!(in_protected_dir(&fl.join(".ssh/config")));
            assert!(in_protected_dir(
                &fl.join("Library/Keychains/login.keychain-db")
            ));
            assert!(contains_protected_dir(&fl.join("Library")).is_some());
            assert!(is_denied_file(&fl.join(".claude.json")));
            // Case-insensitive APFS: a case variant of an EXISTING protected
            // dir is the same inode — caught by identity, not spelling.
            let data = h.join("Library/Application Support/Otto");
            if data.is_dir() && h.join("LIBRARY").is_dir() {
                assert!(in_protected_dir(
                    &h.join("LIBRARY/Application Support/OTTO/otto.db")
                ));
            }
        }
    }

    #[test]
    fn agent_cli_homes_logs_and_hidden_home_dirs_are_protected() {
        // S7-302.
        let Some(h) = home_dir() else { return };
        if in_protected_dir(&h) {
            return;
        }
        for rel in [
            ".codex/auth.json",
            ".gemini/oauth_creds.json",
            ".claude/settings.json",
            ".cursor/mcp.json",
            ".config/anything/token",
            "Library/Logs/Otto/ottod.log",
        ] {
            assert!(in_protected_dir(&h.join(rel)), "{rel}");
        }
        for rel in [
            ".codex",
            ".gemini",
            ".claude",
            "Library/Logs/Otto",
            ".some-tool/x",
        ] {
            assert!(subtree_root_denial(&h.join(rel)).is_some(), "{rel}");
        }
        assert!(subtree_root_denial(&h.join("Documents/.hidden/notes")).is_some());
        assert!(subtree_root_denial(&h.join("Library/Mobile Documents/notes")).is_none());
    }

    #[test]
    fn a_registered_test_dir_is_protected_by_prefix_and_identity() {
        let base = std::env::temp_dir().join(format!("otto-sp-{}", std::process::id()));
        let data = base.join("state");
        std::fs::create_dir_all(&data).unwrap();
        let data = data.canonicalize().unwrap();
        protect_dir_for_tests(&data);
        assert!(in_protected_dir(&data.join("otto.db")));
        assert!(contains_protected_dir(&data).is_some());
        assert!(contains_protected_dir(data.parent().unwrap()).is_some());
        assert!(subtree_root_denial(data.parent().unwrap()).is_some());
        let set = protected_set();
        let id = std::fs::metadata(&data).ok().and_then(|m| metadata_id(&m));
        assert!(set.is_protected_dir(Path::new("/elsewhere/alias"), id));
        let _ = std::fs::remove_dir_all(&base);
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

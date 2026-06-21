//! `GET /api/v1/fs/browse` — daemon-side filesystem browser for folder pickers.
//! `GET /api/v1/fs/read`   — read a file's contents (read-only, ~400KB cap).
//!
//! Both endpoints expose the daemon host's filesystem, so they are sandboxed
//! (audit S2): an authenticated [`CurrentUser`] is required, and every resolved
//! (canonicalized, symlink- and `..`-free) path is run through [`sandbox`].
//!
//! The sandbox is an **allow-list** (primary) plus a **deny-list** (defense in
//! depth). A canonical path is served only if it falls under one of the
//! permitted roots — the user's `$HOME` subtree, the daemon `data_dir`, and a
//! few env-configured workspace/scratch dirs (see [`sandbox::allowed_roots`]).
//! Anything outside every permitted root (another user's home, `/var/root`,
//! system dirs, …) is `403` even when it isn't named in the deny-list. On top
//! of that the deny-list still hard-blocks known secret stores (`~/.ssh`,
//! `~/.aws`, cloud-cred dirs, `/etc`, …) and secret filenames (`id_rsa`,
//! `credentials`, `.env`, …) — so a secret store nested inside an allowed root
//! (e.g. `~/.ssh`) stays blocked.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use otto_core::domain::User;
use otto_core::Error;

mod sandbox {
    //! Allow-list + deny-list sandbox for the host-file endpoints. Operates on a
    //! path that has ALREADY been canonicalized (symlinks + `..` resolved), so a
    //! symlink pointing outside the allowed roots (or into `~/.ssh`) is judged by
    //! its resolved target, not its name.

    use std::path::{Path, PathBuf};

    /// Directories (relative to `$HOME`) that hold credentials/secrets and must
    /// never be browsed or read.
    const HOME_DENY_DIRS: &[&str] = &[
        ".ssh",
        ".aws",
        ".gnupg",
        ".kube",
        ".docker",
        ".config/gcloud",
        ".config/gh",
        ".azure",
        ".password-store",
    ];

    /// Absolute path prefixes that must never be browsed or read (system secret
    /// stores / credential material). Defense in depth — these already fall
    /// outside the allow-list, but we block them explicitly too.
    const ABS_DENY_PREFIXES: &[&str] = &[
        "/etc",
        "/private/etc",
        "/root",
        "/var/root",
        "/proc",
        "/sys",
    ];

    /// Exact (case-insensitive) filenames that are known secret stores and are
    /// never served by `/fs/read` even outside the denied dirs.
    const DENY_FILE_NAMES: &[&str] = &[
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
    ];

    /// Filename substrings whose presence marks a likely secret (private keys,
    /// keystores). Matched case-insensitively against the file name only.
    const DENY_FILE_SUFFIXES: &[&str] = &[".pem", ".key", ".pfx", ".p12", ".keystore"];

    /// Env vars naming extra single roots the daemon legitimately manages
    /// (log dir, skill-eval scratch). Added to the allow-list when set+resolvable.
    const EXTRA_ROOT_ENV: &[&str] = &["OTTO_LOG_DIR", "OTTO_SKILLEVAL_DIR"];

    /// Colon-separated operator escape hatch for additional permitted roots
    /// (e.g. a workspaces tree created outside `$HOME`). Empty entries ignored.
    const EXTRA_ROOTS_LIST_ENV: &str = "OTTO_FS_EXTRA_ROOTS";

    fn home() -> Option<PathBuf> {
        std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    }

    /// Canonicalize a candidate root, dropping it if it can't be resolved (a
    /// non-existent root can never contain a *canonical* target anyway, so it's
    /// safe to omit). Returning canonical roots means the allow-list comparison
    /// is symlink-stable on both sides.
    fn canon(p: &Path) -> Option<PathBuf> {
        p.canonicalize().ok()
    }

    /// The set of permitted root directories (already canonicalized). A target
    /// path is allowed iff it equals or is nested under one of these. `data_dir`
    /// is supplied by the caller (it lives in [`crate::state::ServerCtx`]); the
    /// rest come from `$HOME` and env config.
    ///
    /// Deliberately conservative: when nothing resolves (no `$HOME`, no
    /// `data_dir`) the list is empty and every path is denied — fail closed.
    pub(super) fn allowed_roots(data_dir: &Path) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();

        // The user's home subtree — the picker's primary playground.
        if let Some(h) = home() {
            if let Some(c) = canon(&h) {
                roots.push(c);
            }
        }
        // The daemon data dir (library, swarm worktrees/scratch). Under $HOME by
        // default but may be relocated via $OTTO_DATA_DIR, so add it explicitly.
        if let Some(c) = canon(data_dir) {
            roots.push(c);
        }
        // Single-dir env roots the daemon manages.
        for var in EXTRA_ROOT_ENV {
            if let Some(val) = std::env::var_os(var) {
                if !val.is_empty() {
                    if let Some(c) = canon(Path::new(&val)) {
                        roots.push(c);
                    }
                }
            }
        }
        // Operator-configured extra roots (colon-separated).
        if let Some(list) = std::env::var_os(EXTRA_ROOTS_LIST_ENV) {
            for part in list.to_string_lossy().split(':') {
                let part = part.trim();
                if !part.is_empty() {
                    if let Some(c) = canon(Path::new(part)) {
                        roots.push(c);
                    }
                }
            }
        }

        roots.sort();
        roots.dedup();
        roots
    }

    /// True when `canonical` is inside (or equal to) one of the permitted roots.
    /// With an empty root set this is always false → fail closed.
    pub(super) fn is_within_allowed(canonical: &Path, roots: &[PathBuf]) -> bool {
        roots
            .iter()
            .any(|root| canonical == root.as_path() || canonical.starts_with(root))
    }

    /// True when `canonical` (an already-resolved path) is inside a denied
    /// directory or under a denied absolute prefix. Used for both browse and
    /// read so a denied directory can neither be listed nor descended.
    pub(super) fn is_denied_dir(canonical: &Path) -> bool {
        // Home-relative secret dirs.
        if let Some(home) = home() {
            if let Ok(home_canon) = home.canonicalize() {
                for rel in HOME_DENY_DIRS {
                    let denied = home_canon.join(rel);
                    if canonical == denied || canonical.starts_with(&denied) {
                        return true;
                    }
                }
            }
        }
        // Absolute system prefixes.
        for prefix in ABS_DENY_PREFIXES {
            let p = Path::new(prefix);
            if canonical == p || canonical.starts_with(p) {
                return true;
            }
        }
        false
    }

    /// True when `canonical` names a known secret file (by exact name or
    /// extension). Applied to `/fs/read` on top of [`is_denied_dir`].
    pub(super) fn is_denied_file(canonical: &Path) -> bool {
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
}

/// Reject an already-canonicalized directory path that is outside every
/// permitted root OR falls inside a denied secret store. `data_dir` seeds the
/// allow-list; `_user` is required (the route is authenticated) and reserved
/// for future per-user root scoping — root vs. non-root relaxes neither the
/// allow-list nor the secret-store deny-list.
fn guard_dir(
    canonical: &std::path::Path,
    data_dir: &std::path::Path,
    _user: &User,
) -> Result<(), ApiError> {
    let roots = sandbox::allowed_roots(data_dir);
    if !sandbox::is_within_allowed(canonical, &roots) {
        return Err(ApiError(Error::Forbidden("path is not permitted".into())));
    }
    if sandbox::is_denied_dir(canonical) {
        return Err(ApiError(Error::Forbidden("path is not permitted".into())));
    }
    Ok(())
}

/// Reject an already-canonicalized file path that is outside every permitted
/// root, is inside a denied dir, OR is itself a known secret file.
fn guard_file(
    canonical: &std::path::Path,
    data_dir: &std::path::Path,
    _user: &User,
) -> Result<(), ApiError> {
    let roots = sandbox::allowed_roots(data_dir);
    if !sandbox::is_within_allowed(canonical, &roots) {
        return Err(ApiError(Error::Forbidden("file is not permitted".into())));
    }
    if sandbox::is_denied_dir(canonical) || sandbox::is_denied_file(canonical) {
        return Err(ApiError(Error::Forbidden("file is not permitted".into())));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct BrowseParams {
    /// Absolute path or one starting with `~`. Empty/absent → home dir.
    path: Option<String>,
    /// When `true` (or `1`), include regular files in addition to directories.
    /// Defaults to false so existing callers (GraphView/FolderPicker) are unaffected.
    #[serde(default)]
    files: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_git_repo: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsBrowse {
    /// The canonical path that was browsed.
    pub path: String,
    /// Parent directory path, or null at the filesystem root.
    pub parent: Option<String>,
    /// True when the browsed directory is ITSELF a git repo. Lets a `gitOnly`
    /// folder-picker offer "use this folder" once you've descended into a repo
    /// (otherwise you could only pick a repo from its parent listing).
    pub is_git_repo: bool,
    /// Directory entries (sorted: dirs first, then files, case-insensitively).
    pub entries: Vec<FsEntry>,
}

#[derive(Deserialize)]
pub struct ReadParams {
    /// Absolute path or one starting with `~`.
    path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsRead {
    /// The canonical path that was read.
    pub path: String,
    /// File contents (UTF-8). Empty string when `truncated` is true and binary
    /// check triggered.
    pub content: String,
    /// Language hint derived from file extension (e.g. "rust", "typescript").
    pub language: String,
    /// True when the file was larger than ~400 KB or appeared binary.
    pub truncated: bool,
}

/// Expand a leading `~` to the user's home directory.
fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}{rest}")
    } else {
        path.to_string()
    }
}

/// Simple extension → language hint used by the front-end syntax highlighter.
fn lang_from_path(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "svelte" | "html" | "vue" | "xml" => "xml",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "md" => "markdown",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "toml" => "ini",
        "sql" => "sql",
        _ => "",
    }
    .to_string()
}

/// `GET /api/v1/fs/browse?path=<abs-or-~-path>[&files=true]`
pub async fn browse(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<BrowseParams>,
) -> ApiResult<Json<FsBrowse>> {
    // Resolve the target path.
    let raw = params
        .path
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or("~");
    let expanded = expand_home(raw);
    let target = std::path::Path::new(&expanded);

    if !target.exists() || !target.is_dir() {
        return Err(ApiError(Error::Invalid(format!(
            "not a directory: {}",
            target.display()
        ))));
    }

    let canonical = target
        .canonicalize()
        .map_err(|e| ApiError(Error::Invalid(format!("cannot resolve path: {e}"))))?;

    // Sandbox (audit S2): allow-list (HOME + data_dir + configured roots) with a
    // secret-store deny-list on top. Done on the canonical (symlink/`..`-resolved)
    // path so escapes — including a path that's merely *outside* the roots — are
    // caught.
    guard_dir(&canonical, &ctx.data_dir, &user)?;

    let path_str = canonical.to_string_lossy().into_owned();

    // Parent: None at filesystem root ("/").
    let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());

    // Read directory entries — exclude `.git`.
    let mut dirs: Vec<FsEntry> = Vec::new();
    let mut files: Vec<FsEntry> = Vec::new();

    let read_dir = std::fs::read_dir(&canonical)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot read directory: {e}"))))?;

    for entry_res in read_dir {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().into_owned();

        // Skip `.git` itself and hidden files/dirs starting with `.` that are
        // `.git` specifically. We keep other dotfiles visible.
        if name == ".git" {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let entry_path = entry.path();
        let entry_path_str = entry_path.to_string_lossy().into_owned();

        if file_type.is_dir() {
            let is_git_repo = entry_path.join(".git").is_dir();
            dirs.push(FsEntry {
                name,
                path: entry_path_str,
                is_dir: true,
                is_git_repo,
            });
        } else if file_type.is_file() && params.files {
            files.push(FsEntry {
                name,
                path: entry_path_str,
                is_dir: false,
                is_git_repo: false,
            });
        }
    }

    // Sort each group case-insensitively, then concatenate dirs before files.
    dirs.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by_key(|a| a.name.to_lowercase());
    dirs.extend(files);

    // Whether the browsed directory is itself a git repo (so the picker can let
    // you select it once you've navigated inside).
    let is_git_repo = canonical.join(".git").is_dir();

    Ok(Json(FsBrowse {
        path: path_str,
        parent,
        is_git_repo,
        entries: dirs,
    }))
}

/// Max file size we'll serve in full (~400 KB).
const MAX_READ_BYTES: u64 = 400 * 1024;
/// Number of bytes to probe for binary detection (NUL character check).
const BINARY_PROBE_BYTES: usize = 8 * 1024;

/// `GET /api/v1/fs/read?path=<abs-or-~-path>`
pub async fn read_file(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<ReadParams>,
) -> ApiResult<Json<FsRead>> {
    let expanded = expand_home(&params.path);
    let target = std::path::Path::new(&expanded);

    if target.is_dir() {
        return Err(ApiError(Error::Invalid(format!(
            "path is a directory: {}",
            target.display()
        ))));
    }

    if !target.exists() {
        return Err(ApiError(Error::Invalid(format!(
            "file not found: {}",
            target.display()
        ))));
    }

    let canonical = target
        .canonicalize()
        .map_err(|e| ApiError(Error::Invalid(format!("cannot resolve path: {e}"))))?;

    // Sandbox (audit S2): allow-list (HOME + data_dir + configured roots) plus a
    // secret-file/secret-store deny-list. Canonical path → a symlink pointing
    // outside the roots (or into `~/.ssh`) is caught by its resolved target.
    guard_file(&canonical, &ctx.data_dir, &user)?;

    let path_str = canonical.to_string_lossy().into_owned();
    let language = lang_from_path(&canonical);

    // Check file size.
    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot stat file: {e}"))))?;
    let file_size = metadata.len();

    // Read (up to MAX_READ_BYTES + 1 to detect truncation).
    let read_limit = (MAX_READ_BYTES + 1) as usize;
    let raw_bytes = read_at_most(&canonical, read_limit)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot read file: {e}"))))?;

    let truncated_by_size = file_size > MAX_READ_BYTES;

    // Binary detection: NUL byte in first BINARY_PROBE_BYTES.
    let probe_len = raw_bytes.len().min(BINARY_PROBE_BYTES);
    if raw_bytes[..probe_len].contains(&0u8) {
        return Ok(Json(FsRead {
            path: path_str,
            content: String::new(),
            language,
            truncated: true,
        }));
    }

    // Convert to UTF-8 (lossy so we never 500 on weird encodings).
    let content_full = String::from_utf8_lossy(&raw_bytes).into_owned();

    // If we read more than the cap, trim to MAX_READ_BYTES worth.
    let content = if truncated_by_size {
        // Trim to MAX_READ_BYTES at a char boundary.
        let max = MAX_READ_BYTES as usize;
        if content_full.len() > max {
            let mut end = max;
            while end > 0 && !content_full.is_char_boundary(end) {
                end -= 1;
            }
            content_full[..end].to_string()
        } else {
            content_full
        }
    } else {
        content_full
    };

    Ok(Json(FsRead {
        path: path_str,
        content,
        language,
        truncated: truncated_by_size,
    }))
}

/// Read at most `limit` bytes from a file.
fn read_at_most(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::sandbox;
    use std::path::Path;
    use std::sync::Mutex;

    /// Several tests mutate process-global env vars (`HOME`, `OTTO_FS_EXTRA_ROOTS`)
    /// which `sandbox` reads; serialize them so the parallel test runner can't
    /// interleave one test's env with another's reads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn denies_system_secret_dirs() {
        assert!(sandbox::is_denied_dir(Path::new("/etc")));
        assert!(sandbox::is_denied_dir(Path::new("/etc/ssh")));
        assert!(sandbox::is_denied_dir(Path::new("/private/etc/passwd")));
        assert!(sandbox::is_denied_dir(Path::new("/root/.bashrc")));
        // A normal project dir is allowed.
        assert!(!sandbox::is_denied_dir(Path::new("/Users/me/code/otto")));
        assert!(!sandbox::is_denied_dir(Path::new("/tmp")));
    }

    #[test]
    fn denies_home_secret_dirs() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Use a temp dir as HOME so the test is hermetic. The dir must exist so
        // it canonicalizes; .ssh under it need not exist (starts_with on the
        // canonical home is enough).
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        std::env::set_var("HOME", &home);
        let canon_home = home.canonicalize().expect("canon home");
        assert!(sandbox::is_denied_dir(&canon_home.join(".ssh")));
        assert!(sandbox::is_denied_dir(
            &canon_home.join(".aws").join("credentials")
        ));
        assert!(sandbox::is_denied_dir(&canon_home.join(".config/gcloud")));
        assert!(!sandbox::is_denied_dir(&canon_home.join("projects")));
    }

    #[test]
    fn denies_secret_files_by_name_and_ext() {
        assert!(sandbox::is_denied_file(Path::new("/home/u/proj/id_rsa")));
        assert!(sandbox::is_denied_file(Path::new("/home/u/.netrc")));
        assert!(sandbox::is_denied_file(Path::new("/home/u/server.pem")));
        assert!(sandbox::is_denied_file(Path::new("/home/u/keystore.p12")));
        assert!(sandbox::is_denied_file(Path::new("/srv/app/.env")));
        // Ordinary source files are fine.
        assert!(!sandbox::is_denied_file(Path::new("/home/u/main.rs")));
        assert!(!sandbox::is_denied_file(Path::new("/home/u/README.md")));
    }

    // --- Allow-list (audit S2 tightening) -----------------------------------

    #[test]
    fn allow_list_admits_in_root_denies_out_of_root() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A hermetic HOME with a project subdir, plus a SEPARATE data dir.
        let home_tmp = tempfile::tempdir().expect("home tmp");
        let home = home_tmp.path().canonicalize().expect("canon home");
        std::env::set_var("HOME", &home);
        std::env::remove_var("OTTO_FS_EXTRA_ROOTS");
        std::env::remove_var("OTTO_LOG_DIR");
        std::env::remove_var("OTTO_SKILLEVAL_DIR");

        let proj = home.join("projects");
        std::fs::create_dir_all(&proj).expect("mk proj");

        let data_tmp = tempfile::tempdir().expect("data tmp");
        let data_dir = data_tmp.path().canonicalize().expect("canon data");

        let roots = sandbox::allowed_roots(&data_dir);

        // In-root: HOME itself, a nested project dir, and the data dir.
        assert!(sandbox::is_within_allowed(&home, &roots), "HOME root allowed");
        assert!(
            sandbox::is_within_allowed(&proj, &roots),
            "nested project under HOME allowed"
        );
        assert!(
            sandbox::is_within_allowed(&data_dir, &roots),
            "data_dir allowed"
        );

        // Out-of-root: another user's home, a system dir, and `/var/root` are
        // denied EVEN THOUGH they aren't necessarily in the deny-list — purely
        // because they fall outside every permitted root.
        assert!(
            !sandbox::is_within_allowed(Path::new("/var/root"), &roots),
            "/var/root is outside roots"
        );
        assert!(
            !sandbox::is_within_allowed(Path::new("/Users/someone-else/Documents"), &roots),
            "another user's home is outside roots"
        );
        assert!(
            !sandbox::is_within_allowed(Path::new("/opt/secret"), &roots),
            "arbitrary system dir is outside roots"
        );
        // A sibling of HOME that merely shares a name prefix must NOT match
        // (path component boundary, not string prefix).
        let sibling = home.with_file_name(format!(
            "{}-evil",
            home.file_name().unwrap().to_string_lossy()
        ));
        assert!(
            !sandbox::is_within_allowed(&sibling, &roots),
            "prefix-sibling of HOME is not a subpath"
        );
    }

    #[test]
    fn allow_list_honors_extra_roots_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home_tmp = tempfile::tempdir().expect("home tmp");
        let home = home_tmp.path().canonicalize().expect("canon home");
        std::env::set_var("HOME", &home);

        // A workspaces tree OUTSIDE home, opted-in via the operator env knob.
        let extra_tmp = tempfile::tempdir().expect("extra tmp");
        let extra = extra_tmp.path().canonicalize().expect("canon extra");
        std::env::set_var("OTTO_FS_EXTRA_ROOTS", extra.as_os_str());

        let data_tmp = tempfile::tempdir().expect("data tmp");
        let data_dir = data_tmp.path().canonicalize().expect("canon data");

        let roots = sandbox::allowed_roots(&data_dir);
        assert!(
            sandbox::is_within_allowed(&extra.join("ws1"), &roots),
            "configured extra root admits its subtree"
        );

        std::env::remove_var("OTTO_FS_EXTRA_ROOTS");
        // After removal it's no longer permitted (fresh root computation).
        let roots2 = sandbox::allowed_roots(&data_dir);
        assert!(
            !sandbox::is_within_allowed(&extra.join("ws1"), &roots2),
            "extra root revoked once env is cleared"
        );
    }

    #[test]
    fn allow_list_fails_closed_with_no_roots() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("HOME");
        std::env::remove_var("OTTO_FS_EXTRA_ROOTS");
        std::env::remove_var("OTTO_LOG_DIR");
        std::env::remove_var("OTTO_SKILLEVAL_DIR");
        // A non-existent data dir won't canonicalize, so NO roots resolve.
        let roots = sandbox::allowed_roots(Path::new("/this/does/not/exist/anywhere"));
        assert!(roots.is_empty(), "no resolvable roots → empty list");
        assert!(
            !sandbox::is_within_allowed(Path::new("/tmp"), &roots),
            "empty root set denies everything (fail closed)"
        );
    }
}

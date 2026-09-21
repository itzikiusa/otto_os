//! `GET /api/v1/fs/browse` — daemon-side filesystem browser for folder pickers.
//! `GET /api/v1/fs/read`   — read a file's contents (read-only, ~400KB cap).
//!
//! Both endpoints require an authenticated caller and use the filesystem permissions
//! of the OS account running ottod. Paths are canonicalized; there is no additional
//! root allow-list or secret-name deny-list for these two routes. Existing token
//! endpoint scopes remain enforced by the server middleware. Browsing returns
//! metadata; reading returns bounded regular-file content, never device/FIFO data.

use axum::extract::Query;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use otto_core::Error;

pub(crate) mod sandbox {
    //! Secret-store checks retained for the separate session artifact endpoint.
    //! The general `/fs/browse` and `/fs/read` endpoints do not use this policy.
    //! Input paths have already been canonicalized (symlinks + `..` resolved).

    use std::path::{Path, PathBuf};

    /// Directories (relative to `$HOME`) that hold credentials/secrets and must
    /// never be served through the session artifact endpoint.
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

    /// Absolute prefixes excluded from session artifacts (system secret stores).
    const ABS_DENY_PREFIXES: &[&str] = &[
        "/etc",
        "/private/etc",
        "/root",
        "/var/root",
        "/proc",
        "/sys",
    ];

    /// Exact (case-insensitive) filenames that are known secret stores and are
    /// never served as session artifacts, even outside the denied directories.
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

    fn home() -> Option<PathBuf> {
        std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    }

    /// True when `canonical` (an already-resolved path) is inside a denied
    /// directory or under a denied absolute prefix for session artifacts.
    pub(crate) fn is_denied_dir(canonical: &Path) -> bool {
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
    /// extension). Applied to session artifacts on top of [`is_denied_dir`].
    pub(crate) fn is_denied_file(canonical: &Path) -> bool {
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

/// Keep OS errors actionable, rather than turning denied access into "not found".
fn filesystem_error(action: &str, path: &std::path::Path, error: std::io::Error) -> ApiError {
    let message = format!("cannot {action} {}: {error}", path.display());
    ApiError(match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            Error::Forbidden(format!("OS permission denied: {message}"))
        }
        std::io::ErrorKind::NotFound => Error::NotFound(message),
        _ => Error::Invalid(message),
    })
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

/// Request lifetime is separate from the lifetime of an in-flight OS syscall.
struct BrowseCancel(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for BrowseCancel {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn filesystem_work<T, F>(
    admission: std::sync::Arc<tokio::sync::Semaphore>,
    deadline: std::time::Duration,
    work: F,
) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce(std::sync::Arc<std::sync::atomic::AtomicBool>) -> ApiResult<T> + Send + 'static,
{
    let permit = admission.try_acquire_owned().map_err(|_| {
        ApiError(Error::Conflict(
            "Filesystem access is busy. Retry shortly.".into(),
        ))
    })?;
    let cancel = BrowseCancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let flag = cancel.0.clone();
    let task = tokio::task::spawn_blocking(move || {
        // A canceled/timed-out request cannot release capacity while its actual
        // blocking operation is still running. There is no unbounded task queue.
        let _permit = permit;
        work(flag)
    });
    match tokio::time::timeout(deadline, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(ApiError(Error::Internal(format!(
            "Filesystem task failed: {error}"
        )))),
        Err(_) => Err(ApiError(Error::Upstream(
            "Filesystem access timed out. Retry or choose another path.".into(),
        ))),
    }
}

fn browse_canceled(flag: &std::sync::atomic::AtomicBool) -> ApiResult<()> {
    if flag.load(std::sync::atomic::Ordering::Relaxed) {
        Err(ApiError(Error::Conflict("Folder browsing canceled".into())))
    } else {
        Ok(())
    }
}

/// `GET /api/v1/fs/browse?path=<abs-or-~-path>[&files=true]`
pub async fn browse(
    CurrentUser(_user): CurrentUser,
    Query(params): Query<BrowseParams>,
) -> ApiResult<Json<FsBrowse>> {
    static ADMISSION: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let admission = ADMISSION
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)))
        .clone();
    filesystem_work(
        admission,
        std::time::Duration::from_secs(10),
        move |cancel| browse_sync(params, &cancel),
    )
    .await
    .map(Json)
}

fn browse_sync(
    params: BrowseParams,
    cancel: &std::sync::atomic::AtomicBool,
) -> ApiResult<FsBrowse> {
    browse_canceled(cancel)?;
    // Resolve the target path.
    let raw = params
        .path
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or("~");
    let expanded = expand_home(raw);
    let target = std::path::Path::new(&expanded);

    let metadata =
        std::fs::metadata(target).map_err(|e| filesystem_error("access directory", target, e))?;
    if !metadata.is_dir() {
        return Err(ApiError(Error::Invalid(format!(
            "not a directory: {}",
            target.display()
        ))));
    }
    let canonical = target
        .canonicalize()
        .map_err(|e| filesystem_error("resolve directory", target, e))?;
    browse_canceled(cancel)?;

    let path_str = canonical.to_string_lossy().into_owned();

    // Parent: None at filesystem root ("/").
    let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());

    // Return all accessible directory/file entries; the picker controls hidden names.
    let mut dirs: Vec<FsEntry> = Vec::new();
    let mut files: Vec<FsEntry> = Vec::new();

    let read_dir = std::fs::read_dir(&canonical)
        .map_err(|e| filesystem_error("list directory", &canonical, e))?;

    for entry_res in read_dir {
        browse_canceled(cancel)?;
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().into_owned();

        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            // Follow directory/file symlinks just as the OS does on open. Broken
            // links, inaccessible targets and special files are not selectable.
            Ok(ft) if ft.is_symlink() => match std::fs::metadata(&entry_path) {
                Ok(metadata) => metadata.file_type(),
                Err(_) => continue,
            },
            Ok(ft) => ft,
            Err(_) => continue,
        };
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
    dirs.sort_by_cached_key(|a| a.name.to_lowercase());
    files.sort_by_cached_key(|a| a.name.to_lowercase());
    dirs.extend(files);

    // Whether the browsed directory is itself a git repo (so the picker can let
    // you select it once you've navigated inside).
    let is_git_repo = canonical.join(".git").is_dir();

    browse_canceled(cancel)?;
    Ok(FsBrowse {
        path: path_str,
        parent,
        is_git_repo,
        entries: dirs,
    })
}

/// Max file size we'll serve in full (~400 KB).
const MAX_READ_BYTES: u64 = 400 * 1024;
/// Number of bytes to probe for binary detection (NUL character check).
const BINARY_PROBE_BYTES: usize = 8 * 1024;

/// `GET /api/v1/fs/read?path=<abs-or-~-path>`
pub async fn read_file(
    CurrentUser(_user): CurrentUser,
    Query(params): Query<ReadParams>,
) -> ApiResult<Json<FsRead>> {
    static ADMISSION: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let admission = ADMISSION
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)))
        .clone();
    filesystem_work(
        admission,
        std::time::Duration::from_secs(10),
        move |cancel| {
            browse_canceled(&cancel)?;
            let result = read_file_sync(&params.path);
            browse_canceled(&cancel)?;
            result
        },
    )
    .await
    .map(Json)
}

fn read_file_sync(path: &str) -> ApiResult<FsRead> {
    let expanded = expand_home(path);
    let target = std::path::Path::new(&expanded);
    let canonical = target
        .canonicalize()
        .map_err(|e| filesystem_error("resolve file", target, e))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| filesystem_error("inspect file", &canonical, e))?;
    if !metadata.is_file() {
        return Err(ApiError(Error::Invalid(format!(
            "not a regular file: {}",
            canonical.display()
        ))));
    }
    let path_str = canonical.to_string_lossy().into_owned();
    let language = lang_from_path(&canonical);
    let raw_bytes = read_at_most(&canonical, (MAX_READ_BYTES + 1) as usize)
        .map_err(|e| filesystem_error("read file", &canonical, e))?;
    let truncated_by_size = raw_bytes.len() > MAX_READ_BYTES as usize;

    // Binary detection: NUL byte in first BINARY_PROBE_BYTES.
    let probe_len = raw_bytes.len().min(BINARY_PROBE_BYTES);
    if raw_bytes[..probe_len].contains(&0u8) {
        return Ok(FsRead {
            path: path_str,
            content: String::new(),
            language,
            truncated: true,
        });
    }

    // Convert to UTF-8 (lossy so we never 500 on weird encodings).
    let content_full = String::from_utf8_lossy(&raw_bytes).into_owned();

    // If we read more than the cap, trim to MAX_READ_BYTES worth.
    let truncated_by_size = truncated_by_size || content_full.len() > MAX_READ_BYTES as usize;
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

    Ok(FsRead {
        path: path_str,
        content,
        language,
        truncated: truncated_by_size,
    })
}

/// Read at most `limit` bytes from a file.
fn read_at_most(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A path can change between metadata and open. Nonblocking open avoids a
    // replaced FIFO hanging a worker; verify the actual open handle as well.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut buf = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    fn fixture_user() -> otto_core::domain::User {
        otto_core::domain::User {
            id: "fixture".into(),
            username: "fixture".into(),
            display_name: "Fixture".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn os_permissions_allow_listing_outside_configured_roots() {
        let outside = tempfile::tempdir().unwrap();
        let folder = outside.path().join(".ssh");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("id_ed25519"), "synthetic fixture, not a key").unwrap();
        let result = super::browse_sync(
            super::BrowseParams {
                path: Some(folder.to_string_lossy().into()),
                files: true,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.entries[0].name, "id_ed25519");
    }

    #[test]
    fn os_permissions_allow_reading_synthetic_key_filename() {
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("id_ed25519");
        std::fs::write(&path, "synthetic fixture, not a key").unwrap();
        assert_eq!(
            super::read_file_sync(path.to_str().unwrap())
                .unwrap()
                .content,
            "synthetic fixture, not a key"
        );
    }
    #[test]
    fn browse_sync_keeps_complete_sorted_listing_and_canonical_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for name in ["zeta", "Alpha", ".hidden", ".git"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        std::fs::write(root.join("Bravo.txt"), "fixture").unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = super::browse_sync(
            super::BrowseParams {
                path: Some(root.to_string_lossy().into_owned()),
                files: true,
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(
            result
                .entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec![".git", ".hidden", "Alpha", "zeta", "Bravo.txt"]
        );
        assert!(result.is_git_repo);
        assert_eq!(result.path, root.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn browse_sync_lists_accessible_symlinks_and_skips_broken_or_special_targets() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let content = target.path().join("synthetic.key");
        std::fs::write(&content, "synthetic fixture").unwrap();
        std::os::unix::fs::symlink(target.path(), root.path().join("directory-link")).unwrap();
        std::os::unix::fs::symlink(&content, root.path().join("file-link")).unwrap();
        std::os::unix::fs::symlink(target.path().join("missing"), root.path().join("broken"))
            .unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(target.path().join("socket")).unwrap();
        std::os::unix::fs::symlink(target.path().join("socket"), root.path().join("special"))
            .unwrap();
        let result = super::browse_sync(
            super::BrowseParams {
                path: Some(root.path().to_string_lossy().into()),
                files: true,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.is_dir))
                .collect::<Vec<_>>(),
            vec![("directory-link", true), ("file-link", false)]
        );
        let entered = super::browse_sync(
            super::BrowseParams {
                path: Some(result.entries[0].path.clone()),
                files: false,
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            entered.path,
            target.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[tokio::test]
    async fn filesystem_work_keeps_capacity_until_canceled_blocking_job_exits() {
        use std::sync::{Arc, Condvar, Mutex};
        use std::time::Duration;
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started) = tokio::sync::oneshot::channel();
        let worker_gate = gate.clone();
        let worker_admission = admission.clone();
        let caller = tokio::spawn(async move {
            super::filesystem_work(worker_admission, Duration::from_secs(10), move |cancel| {
                let _ = started_tx.send(cancel.clone());
                let (lock, wake) = &*worker_gate;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = wake.wait(ready).unwrap();
                }
                Ok(())
            })
            .await
        });
        let cancel = started.await.unwrap();
        caller.abort();
        let _ = caller.await;
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(admission.available_permits(), 0);
        let busy = super::filesystem_work(admission.clone(), Duration::from_secs(1), |_| Ok(()))
            .await
            .unwrap_err();
        assert!(matches!(busy.0, otto_core::Error::Conflict(_)));
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_one();
        tokio::time::timeout(Duration::from_secs(3), async {
            while admission.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn filesystem_work_timeout_leaves_async_runtime_responsive() {
        use std::sync::Arc;
        use std::time::Duration;
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let result = super::filesystem_work(admission, Duration::from_millis(10), |_| {
            std::thread::sleep(Duration::from_millis(100));
            Ok(())
        })
        .await;
        assert!(matches!(
            result.unwrap_err().0,
            otto_core::Error::Upstream(_)
        ));
    }

    #[tokio::test]
    async fn os_permissions_http_requires_auth_and_preserves_regular_file_access() {
        use axum::{
            body::{to_bytes, Body},
            http::{Request, StatusCode},
            routing::get,
            Extension, Router,
        };
        use otto_core::auth::{AuthContext, AuthUser};
        use tower::ServiceExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".env");
        std::fs::write(&path, "synthetic fixture").unwrap();
        let read_uri = format!("/fs/read?path={}", path.display());
        let browse_uri = format!("/fs/browse?files=true&path={}", temp.path().display());
        let app = Router::new()
            .route("/fs/read", get(super::read_file))
            .route("/fs/browse", get(super::browse));
        for uri in [&read_uri, &browse_uri] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        for (root, managed) in [(false, false), (true, false), (true, true)] {
            let mut user = fixture_user();
            user.is_root = root;
            let context = AuthContext {
                real_user: user.clone(),
                effective_user: user.clone(),
                scope: None,
                mcp_only: false,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
                managed_session_id: managed.then(|| "fixture-session".into()),
            };
            let authenticated = app
                .clone()
                .layer(Extension(AuthUser(user)))
                .layer(Extension(context));
            for uri in [&read_uri, &browse_uri] {
                let response = authenticated
                    .clone()
                    .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "root={root}, managed={managed}"
                );
                let body: serde_json::Value = serde_json::from_slice(
                    &to_bytes(response.into_body(), 1024 * 1024).await.unwrap(),
                )
                .unwrap();
                if uri == &read_uri {
                    assert_eq!(body["content"], "synthetic fixture");
                } else {
                    assert_eq!(body["entries"][0]["name"], ".env");
                }
            }
        }
    }

    #[tokio::test]
    async fn filesystem_routes_retain_share_and_mcp_endpoint_scopes() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
            middleware,
            routing::get,
            Extension, Router,
        };
        use otto_core::auth::{AuthContext, AuthUser, SessionScope};
        use tower::ServiceExt;
        #[derive(Clone)]
        struct ScopeState;
        impl crate::feature_guard::HasGrants for ScopeState {
            fn grants(&self) -> otto_state::GrantsRepo {
                panic!("scoped credentials must be denied before feature grants");
            }
        }
        for mcp_only in [false, true] {
            let user = fixture_user();
            let context = AuthContext {
                real_user: user.clone(),
                effective_user: user.clone(),
                scope: (!mcp_only).then(|| SessionScope {
                    session_id: "shared-session".into(),
                    role: otto_core::domain::WorkspaceRole::Editor,
                    otp_pending: false,
                }),
                mcp_only,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
                managed_session_id: None,
            };
            let app = Router::new()
                .route("/api/v1/fs/read", get(super::read_file))
                .route("/api/v1/fs/browse", get(super::browse))
                .route_layer(middleware::from_fn_with_state(
                    ScopeState,
                    crate::feature_guard::feature_guard::<ScopeState>,
                ))
                .layer(Extension(AuthUser(user)))
                .layer(Extension(context));
            for uri in [
                "/api/v1/fs/read?path=/synthetic/not-opened",
                "/api/v1/fs/browse?path=/synthetic/not-opened",
            ] {
                let response = app
                    .clone()
                    .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
            }
        }
    }

    #[test]
    fn regular_file_reads_keep_caps_binary_detection_and_canonical_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.key");
        let cap = super::MAX_READ_BYTES as usize;
        for bytes in [vec![b'x'; cap + 9], vec![0xff; cap]] {
            std::fs::write(&path, bytes).unwrap();
            let read = super::read_file_sync(path.to_str().unwrap()).unwrap();
            assert!(read.truncated);
            assert!(read.content.len() <= cap);
        }
        std::fs::write(&path, b"binary\0fixture").unwrap();
        let read = super::read_file_sync(path.to_str().unwrap()).unwrap();
        assert!(read.truncated);
        assert!(read.content.is_empty());
        std::fs::write(&path, b"synthetic fixture").unwrap();
        #[cfg(unix)]
        {
            let link = temp.path().join("symlink");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            let read = super::read_file_sync(link.to_str().unwrap()).unwrap();
            assert_eq!(read.path, path.canonicalize().unwrap().to_string_lossy());
            assert_eq!(read.content, "synthetic fixture");
            let socket = temp.path().join("socket");
            let _socket = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            assert!(matches!(
                super::read_file_sync(socket.to_str().unwrap())
                    .unwrap_err()
                    .0,
                otto_core::Error::Invalid(_)
            ));
        }
        assert!(matches!(
            super::read_file_sync(temp.path().to_str().unwrap())
                .unwrap_err()
                .0,
            otto_core::Error::Invalid(_)
        ));
        assert!(matches!(
            super::read_file_sync(temp.path().join("missing").to_str().unwrap())
                .unwrap_err()
                .0,
            otto_core::Error::NotFound(_)
        ));
    }

    #[test]
    fn os_permission_errors_keep_the_attempted_path() {
        let path = std::path::Path::new("/synthetic/denied");
        let error = super::filesystem_error(
            "list directory",
            path,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        assert!(
            matches!(error.0, otto_core::Error::Forbidden(ref message) if message.contains("OS permission denied") && message.contains("/synthetic/denied"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn os_denied_temporary_files_and_directories_are_not_bypassed() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("denied");
        std::fs::create_dir(&folder).unwrap();
        let path = temp.path().join("denied.key");
        std::fs::write(&path, "synthetic fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o0)).unwrap();
        let read = super::read_file_sync(path.to_str().unwrap());
        let browse = super::browse_sync(
            super::BrowseParams {
                path: Some(folder.to_string_lossy().into()),
                files: true,
            },
            &std::sync::atomic::AtomicBool::new(false),
        );
        // Restore modes before assertions so failed tests never strand temp paths.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !rustix::process::geteuid().is_root() {
            assert!(matches!(
                read.unwrap_err().0,
                otto_core::Error::Forbidden(_)
            ));
            assert!(matches!(
                browse.unwrap_err().0,
                otto_core::Error::Forbidden(_)
            ));
        }
    }

    #[test]
    fn artifact_secret_checks_remain_separate() {
        assert!(super::sandbox::is_denied_dir(std::path::Path::new(
            "/etc/ssh"
        )));
        assert!(super::sandbox::is_denied_file(std::path::Path::new(
            "/fixture/id_ed25519"
        )));
    }
}

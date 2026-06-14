//! `GET /api/v1/fs/browse` — daemon-side filesystem browser for folder pickers.
//! `GET /api/v1/fs/read`   — read a file's contents (read-only, ~400KB cap).

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use otto_core::Error;

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
    if path.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}{}", &path[1..])
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
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
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
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dirs.extend(files);

    Ok(Json(FsBrowse {
        path: path_str,
        parent,
        entries: dirs,
    }))
}

/// Max file size we'll serve in full (~400 KB).
const MAX_READ_BYTES: u64 = 400 * 1024;
/// Number of bytes to probe for binary detection (NUL character check).
const BINARY_PROBE_BYTES: usize = 8 * 1024;

/// `GET /api/v1/fs/read?path=<abs-or-~-path>`
pub async fn read_file(
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
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

//! Run-scoped context files (PC-1, access-control-3-04):
//!
//! - `GET /api/v1/workflow-runs/{id}/context/browse?path=<rel>` → [`FsBrowse`]
//! - `GET /api/v1/workflow-runs/{id}/context/read?path=<rel>`   → [`FsRead`]
//!
//! A run's context dir is `<data_dir>/workflow-context/<run_id>/` — the
//! **Context files** tree and the **Final output** panel of the run view. It
//! sits inside Otto's data dir, which `/fs/*` refuses to every non-root caller
//! (and `/fs/*` needs Agents:Edit), so through `/fs/*` only root could ever see
//! a run's deliverable. These routes are gated like `get_run` instead: ws Viewer
//! on the run's workspace (root passes; share, MCP-only and agent-session
//! tokens are refused by the same middleware/guard as `get_run`). `/fs/*` and
//! its data-dir deny are unchanged.
//!
//! Paths are RELATIVE to the context dir (empty/absent = its root) and are
//! answered relative too. They are joined with
//! [`otto_core::paths::confine_join`] (absolute, `..` → 400) and the canonical
//! target must stay under the canonical root (a symlink escaping it → 400).
//! Errors: not a member 403; unknown run or missing file 404.

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, Query, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::Deserialize;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::fs::{FsBrowse, FsEntry, FsRead};
use crate::state::ServerCtx;

/// Same cap as `/fs/read` (~400 KiB).
const MAX_READ_BYTES: usize = 400 * 1024;
/// Bytes probed for a NUL (binary detection), as `/fs/read`.
const BINARY_PROBE_BYTES: usize = 8 * 1024;

#[derive(Deserialize)]
pub struct RelPathParams {
    /// Relative to the run's context dir; empty or absent = its root.
    #[serde(default)]
    path: Option<String>,
}

/// Gate on the run (404 unknown, ws Viewer on its workspace) and return the
/// run's context dir — not yet canonicalized, it may not exist.
async fn run_context_root(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &str,
) -> ApiResult<PathBuf> {
    let wid: Option<String> =
        sqlx::query_scalar("SELECT workspace_id FROM workflow_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(&ctx.pool)
            .await
            .map_err(|e| ApiError(Error::Internal(format!("workflow run lookup: {e}"))))?;
    let wid = wid.ok_or_else(|| ApiError(Error::NotFound("workflow run not found".into())))?;
    require_ws_role(ctx, user, &wid, WorkspaceRole::Viewer).await?;
    // Run ids are daemon-generated, but confine the join anyway (as
    // `with_context_dir` does) so a hostile id can never leave the tree.
    otto_core::paths::confine_join(&ctx.data_dir.join("workflow-context"), id)
        .ok_or_else(|| ApiError(Error::NotFound("workflow run not found".into())))
}

/// The canonical root and the canonical target of `rel` under it.
/// Absolute / `..` / escaping symlink → 400; a missing root or target → 404.
#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool
fn resolve(root: &FsPath, rel: &str) -> ApiResult<(PathBuf, PathBuf)> {
    let rel = rel.trim();
    let joined = if rel.is_empty() {
        root.to_path_buf()
    } else {
        otto_core::paths::confine_join(root, rel).ok_or_else(|| {
            ApiError(Error::Invalid(
                "path must be relative to the run's context dir (no absolute path or `..`)".into(),
            ))
        })?
    };
    let canonical_root = root
        .canonicalize()
        .map_err(|_| ApiError(Error::NotFound("this run has no context files".into())))?;
    let canonical = joined.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            ApiError(Error::NotFound(format!("{rel}: no such file in this run")))
        }
        _ => ApiError(Error::Invalid(format!("cannot resolve {rel}: {e}"))),
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ApiError(Error::Invalid(format!(
            "{rel} resolves outside the run's context dir"
        ))));
    }
    Ok((canonical_root, canonical))
}

/// `canonical` relative to `root` as a `/`-joined string ("" for the root).
fn rel_of(root: &FsPath, canonical: &FsPath) -> String {
    canonical
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("run context task failed: {e}"))))?
}

/// `GET /api/v1/workflow-runs/{id}/context/browse?path=<rel>`
pub async fn browse(
    Path(id): Path<Id>,
    Query(params): Query<RelPathParams>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FsBrowse>> {
    let root = run_context_root(&ctx, &user, &id).await?;
    let rel = params.path.unwrap_or_default();
    blocking(move || browse_sync(&root, &rel)).await.map(Json)
}

#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool
fn browse_sync(root: &FsPath, rel: &str) -> ApiResult<FsBrowse> {
    let (root, dir) = resolve(root, rel)?;
    if !dir.is_dir() {
        return Err(ApiError(Error::Invalid(format!("not a directory: {rel}"))));
    }
    let read_dir = std::fs::read_dir(&dir)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot list {rel}: {e}"))))?;
    let mut dirs: Vec<FsEntry> = Vec::new();
    let mut files: Vec<FsEntry> = Vec::new();
    for entry in read_dir.flatten() {
        // Follow links like the OS does on open, but only to targets that stay
        // inside the context dir; broken or escaping links are not listed.
        let Ok(target) = entry.path().canonicalize() else {
            continue;
        };
        if !target.starts_with(&root) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&target) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = rel_of(&root, &dir.join(&name));
        if meta.is_dir() {
            dirs.push(FsEntry {
                name,
                path,
                is_dir: true,
                is_git_repo: target.join(".git").is_dir(),
            });
        } else if meta.is_file() {
            files.push(FsEntry {
                name,
                path,
                is_dir: false,
                is_git_repo: false,
            });
        }
    }
    dirs.sort_by_cached_key(|e| e.name.to_lowercase());
    files.sort_by_cached_key(|e| e.name.to_lowercase());
    dirs.extend(files);
    let parent = (dir != root).then(|| dir.parent().map(|p| rel_of(&root, p)).unwrap_or_default());
    Ok(FsBrowse {
        path: rel_of(&root, &dir),
        parent,
        is_git_repo: dir.join(".git").is_dir(),
        entries: dirs,
    })
}

/// `GET /api/v1/workflow-runs/{id}/context/read?path=<rel>`
pub async fn read(
    Path(id): Path<Id>,
    Query(params): Query<RelPathParams>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FsRead>> {
    let root = run_context_root(&ctx, &user, &id).await?;
    let rel = params.path.unwrap_or_default();
    if rel.trim().is_empty() {
        return Err(ApiError(Error::Invalid("path is required".into())));
    }
    let restricted = !user.is_root;
    blocking(move || read_sync(&root, &rel, restricted))
        .await
        .map(Json)
}

#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool
fn read_sync(root: &FsPath, rel: &str, restricted: bool) -> ApiResult<FsRead> {
    let (root, file) = resolve(root, rel)?;
    // The same key-name deny `/fs/read` applies to non-root callers anywhere.
    if restricted && otto_core::secret_paths::is_denied_file(&file) {
        return Err(ApiError(Error::Forbidden(format!(
            "{rel} looks like a credential file; only the root user can open it"
        ))));
    }
    let bytes = read_at_most(&file, MAX_READ_BYTES + 1)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot read {rel}: {e}"))))?;
    let path = rel_of(&root, &file);
    let language = lang_from_path(&file);
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0u8) {
        return Ok(FsRead {
            path,
            content: String::new(),
            language,
            truncated: true,
        });
    }
    let mut content = String::from_utf8_lossy(&bytes).into_owned();
    let truncated = bytes.len() > MAX_READ_BYTES || content.len() > MAX_READ_BYTES;
    if content.len() > MAX_READ_BYTES {
        let mut end = MAX_READ_BYTES;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
    }
    Ok(FsRead {
        path,
        content,
        language,
        truncated,
    })
}

/// Extension → syntax-highlighter hint; the same table as `/fs/read`.
fn lang_from_path(path: &FsPath) -> String {
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
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "toml" => "ini",
        "sql" => "sql",
        _ => "",
    }
    .to_string()
}

/// Read at most `limit` bytes of a REGULAR file. `path` is canonical, so its
/// last component is never a symlink: `O_NOFOLLOW` refuses one swapped in since
/// the containment check, and `O_NONBLOCK` keeps a swapped-in FIFO from hanging.
fn read_at_most(path: &FsPath, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
        );
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
    use super::*;

    #[test]
    fn resolve_refuses_absolute_dotdot_and_escaping_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("run");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("final-output.md"), "# done").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "s").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("secret.txt"), root.join("escape")).unwrap();

        let (_, f) = resolve(&root, "final-output.md").unwrap();
        assert!(f.ends_with("final-output.md"));
        for bad in ["../secret.txt", "/etc/passwd", "sub/../../secret.txt"] {
            let e = resolve(&root, bad).unwrap_err();
            assert!(matches!(e.0, Error::Invalid(_)), "{bad}: {e:?}");
        }
        #[cfg(unix)]
        assert!(matches!(
            resolve(&root, "escape").unwrap_err().0,
            Error::Invalid(_)
        ));
        assert!(matches!(
            resolve(&root, "missing.md").unwrap_err().0,
            Error::NotFound(_)
        ));
        // No context dir at all → 404, not 400.
        assert!(matches!(
            resolve(&tmp.path().join("nope"), "").unwrap_err().0,
            Error::NotFound(_)
        ));
    }

    #[test]
    fn browse_answers_relative_paths_and_hides_escaping_links() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("run");
        std::fs::create_dir_all(root.join("nodes/a")).unwrap();
        std::fs::write(root.join("final-output.md"), "x").unwrap();
        std::fs::write(root.join("nodes/a/out.txt"), "y").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path(), root.join("up")).unwrap();

        let top = browse_sync(&root, "").unwrap();
        assert_eq!(top.path, "");
        assert_eq!(top.parent, None);
        let names: Vec<_> = top.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, vec!["nodes", "final-output.md"]);

        let nested = browse_sync(&root, "nodes/a").unwrap();
        assert_eq!(nested.path, "nodes/a");
        assert_eq!(nested.parent.as_deref(), Some("nodes"));
        assert_eq!(nested.entries[0].path, "nodes/a/out.txt");

        let read = read_sync(&root, "final-output.md", true).unwrap();
        assert_eq!(read.path, "final-output.md");
        assert_eq!(read.content, "x");
        assert_eq!(read.language, "markdown");
    }
}

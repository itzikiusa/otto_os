//! Session-scoped working-folder files (PC-1, access-control-4-03):
//!
//! - `GET /api/v1/sessions/{id}/files/browse?path=<rel>` → [`FsBrowse`]
//! - `GET /api/v1/sessions/{id}/files/read?path=<rel>`   → [`FsRead`]
//!
//! The Side panel's **Files** tab (and the editor it opens) inside a session
//! shows the session's `cwd`. Otto's own engines put many of those cwds in its
//! data dir — workflow worktrees, persona folders, goal-loop and scheduled-task
//! work dirs, swarm agent dirs (`otto_sandbox::AGENT_DATA_SUBDIRS`) — which
//! `/fs/*` refuses to every non-root caller, so a non-root owner got 403 on
//! their OWN session's files. These routes are gated per session instead: the
//! session owner, a ws Admin or root (`require_session_owner_or_admin`) plus
//! ws Viewer. Share, MCP-only and agent-session tokens are refused by the
//! feature guard (`is_host_files_route`), as on `/fs/*`. `/fs/*` and its global
//! data-dir deny are unchanged.
//!
//! Paths are RELATIVE to the canonical cwd (empty/absent = its root) and are
//! answered relative too. They are joined with
//! [`otto_core::paths::confine_join`] (absolute, `..` → 400) and the canonical
//! target must stay under the canonical cwd (an escaping symlink → 400). For a
//! non-root caller the key-file deny (`is_denied_file`), `otto.db*` /
//! `secrets.json`, and every protected dir still answer 403 unless the cwd
//! itself is one session work dir strictly inside an agent work area — a cwd
//! of `~` never opens `~/.ssh`, nor other users' work dirs in the data dir.
//! Errors: not allowed 403; unknown session or missing file 404.

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, Query, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::routes::fs::{
    lang_from_path, read_at_most, FsBrowse, FsEntry, FsRead, BINARY_PROBE_BYTES, MAX_READ_BYTES,
};
use crate::state::ServerCtx;

#[derive(Deserialize)]
pub struct RelPathParams {
    /// Relative to the session's cwd; empty or absent = its root.
    #[serde(default)]
    path: Option<String>,
}

/// What a non-root read/browse may not reach even inside its own cwd.
#[derive(Clone)]
struct Restrict {
    /// Canonical `<data_dir>/<sub>` for every agent work area.
    work_areas: Vec<PathBuf>,
}

impl Restrict {
    #[allow(clippy::disallowed_methods)] // sync: called on the blocking pool
    fn for_data_dir(data_dir: &FsPath) -> Self {
        let base = data_dir
            .canonicalize()
            .unwrap_or_else(|_| data_dir.to_path_buf());
        Self {
            work_areas: otto_sandbox::AGENT_DATA_SUBDIRS
                .iter()
                .map(|s| base.join(s))
                .collect(),
        }
    }

    /// True when canonical `p` lies in (or is) one of the agent work areas.
    pub(crate) fn in_work_area(&self, p: &FsPath) -> bool {
        self.work_areas.iter().any(|w| p.starts_with(w))
    }

    /// True when canonical `cwd` is ONE session's work dir: strictly inside an
    /// agent work area, never the area itself. Only such a cwd opens the data
    /// dir — a cwd of `~` (or of the data dir) must not reach every user's
    /// worktrees and persona folders through the work-area exception.
    pub(crate) fn is_session_work_dir(&self, cwd: &FsPath) -> bool {
        self.work_areas
            .iter()
            .any(|w| cwd.starts_with(w) && cwd != w.as_path())
    }

    /// `canonical` is the target, already confined under canonical `root`.
    fn check(&self, root: &FsPath, canonical: &FsPath, rel: &str) -> ApiResult<()> {
        use otto_core::secret_paths as sp;
        let name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let state_file = name.starts_with("otto.db") || name == "secrets.json";
        if state_file
            || sp::is_denied_file(canonical)
            || (sp::in_protected_dir(canonical) && !self.is_session_work_dir(root))
        {
            return Err(ApiError(Error::Forbidden(format!(
                "{} holds credentials or Otto's own state; only the root user can open it",
                if rel.is_empty() { "this folder" } else { rel }
            ))));
        }
        Ok(())
    }
}

/// Gate on the session (404 unknown; owner-or-admin + ws Viewer) and return
/// its cwd — not yet canonicalized, it may be gone.
async fn session_root(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
) -> ApiResult<PathBuf> {
    let session =
        crate::routes::transcript::session_gate(ctx, user, id, WorkspaceRole::Viewer).await?;
    if session.cwd.trim().is_empty() {
        return Err(ApiError(Error::NotFound(
            "this session has no working folder".into(),
        )));
    }
    Ok(PathBuf::from(session.cwd))
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
                "path must be relative to the session's working folder (no absolute path or `..`)"
                    .into(),
            ))
        })?
    };
    let canonical_root = root.canonicalize().map_err(|_| {
        ApiError(Error::NotFound(
            "the session's working folder no longer exists".into(),
        ))
    })?;
    let canonical = joined.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ApiError(Error::NotFound(format!(
            "{rel}: no such file in this session"
        ))),
        _ => ApiError(Error::Invalid(format!("cannot resolve {rel}: {e}"))),
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ApiError(Error::Invalid(format!(
            "{rel} resolves outside the session's working folder"
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
        .map_err(|e| ApiError(Error::Internal(format!("session files task failed: {e}"))))?
}

/// `GET /api/v1/sessions/{id}/files/browse?path=<rel>`
pub async fn browse(
    Path(id): Path<Id>,
    Query(params): Query<RelPathParams>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FsBrowse>> {
    let root = session_root(&ctx, &user, &id).await?;
    let rel = params.path.unwrap_or_default();
    let data_dir = ctx.data_dir.clone();
    let restricted = !user.is_root;
    blocking(move || {
        let restrict = restricted.then(|| Restrict::for_data_dir(&data_dir));
        browse_sync(&root, &rel, restrict.as_ref())
    })
    .await
    .map(Json)
}

#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool
fn browse_sync(root: &FsPath, rel: &str, restrict: Option<&Restrict>) -> ApiResult<FsBrowse> {
    let (root, dir) = resolve(root, rel)?;
    if let Some(r) = restrict {
        r.check(&root, &dir, rel)?;
    }
    if !dir.is_dir() {
        return Err(ApiError(Error::Invalid(format!("not a directory: {rel}"))));
    }
    let read_dir = std::fs::read_dir(&dir)
        .map_err(|e| ApiError(Error::Invalid(format!("cannot list {rel}: {e}"))))?;
    let mut dirs: Vec<FsEntry> = Vec::new();
    let mut files: Vec<FsEntry> = Vec::new();
    for entry in read_dir.flatten() {
        // Follow links like the OS does on open, but only to targets that stay
        // inside the cwd; broken or escaping links are not listed.
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

/// `GET /api/v1/sessions/{id}/files/read?path=<rel>`
pub async fn read(
    Path(id): Path<Id>,
    Query(params): Query<RelPathParams>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FsRead>> {
    let root = session_root(&ctx, &user, &id).await?;
    let rel = params.path.unwrap_or_default();
    if rel.trim().is_empty() {
        return Err(ApiError(Error::Invalid("path is required".into())));
    }
    let data_dir = ctx.data_dir.clone();
    let restricted = !user.is_root;
    blocking(move || {
        let restrict = restricted.then(|| Restrict::for_data_dir(&data_dir));
        read_sync(&root, &rel, restrict.as_ref())
    })
    .await
    .map(Json)
}

#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool
fn read_sync(root: &FsPath, rel: &str, restrict: Option<&Restrict>) -> ApiResult<FsRead> {
    let (root, file) = resolve(root, rel)?;
    if let Some(r) = restrict {
        r.check(&root, &file, rel)?;
    }
    let max = MAX_READ_BYTES as usize;
    let bytes = read_at_most(&file, max + 1)
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
    let truncated = bytes.len() > max || content.len() > max;
    if content.len() > max {
        let mut end = max;
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

/// `/ws/lsp` (PC-1): may a NON-root `user` root a language server at canonical
/// `root`, which the `/fs/*` deny refuses because it lies in Otto's data dir?
/// Yes only when `root` is inside an agent work area AND equals or lies under
/// the canonical cwd of a session the caller may open (owner, ws Admin) whose
/// cwd is itself one work dir strictly inside a work area.
pub(crate) async fn lsp_root_in_own_session(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    root: &FsPath,
) -> bool {
    let data_dir = ctx.data_dir.clone();
    let root_buf = root.to_path_buf();
    let restrict = match tokio::task::spawn_blocking(move || {
        let r = Restrict::for_data_dir(&data_dir);
        r.in_work_area(&root_buf).then_some(r)
    })
    .await
    {
        Ok(Some(r)) => r,
        _ => return false,
    };
    // Candidate sessions: those whose cwd is under the data dir at all (the
    // stored cwd may be non-canonical, so canonicalize before comparing).
    let rows: Vec<(String, String)> =
        match sqlx::query_as("SELECT id, cwd FROM sessions WHERE cwd <> ''")
            .fetch_all(&ctx.pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("lsp session-root lookup failed: {e}");
                return false;
            }
        };
    let root_buf = root.to_path_buf();
    #[allow(clippy::disallowed_methods)] // canonicalize on the blocking pool
    let matches: Vec<String> = tokio::task::spawn_blocking(move || {
        rows.into_iter()
            .filter(|(_, cwd)| {
                FsPath::new(cwd)
                    .canonicalize()
                    .is_ok_and(|c| restrict.is_session_work_dir(&c) && root_buf.starts_with(&c))
            })
            .map(|(id, _)| id)
            .collect()
    })
    .await
    .unwrap_or_default();
    for sid in matches {
        let Ok(session) = ctx.manager.get(&sid).await else {
            continue;
        };
        if crate::auth::require_session_owner_or_admin(ctx, user, &session)
            .await
            .is_ok()
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_refuses_absolute_dotdot_and_escaping_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cwd");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("otto.db"), "db").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("otto.db"), root.join("escape")).unwrap();

        let (_, f) = resolve(&root, "main.rs").unwrap();
        assert!(f.ends_with("main.rs"));
        for bad in [
            "../otto.db",
            "../../otto.db",
            "/etc/passwd",
            "src/../../otto.db",
        ] {
            let e = resolve(&root, bad).unwrap_err();
            assert!(matches!(e.0, Error::Invalid(_)), "{bad}: {e:?}");
        }
        #[cfg(unix)]
        assert!(matches!(
            resolve(&root, "escape").unwrap_err().0,
            Error::Invalid(_)
        ));
        assert!(matches!(
            resolve(&root, "missing.rs").unwrap_err().0,
            Error::NotFound(_)
        ));
    }

    #[test]
    fn restrict_opens_work_areas_but_not_state_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().canonicalize().unwrap();
        let r = Restrict::for_data_dir(&data);
        assert!(r.in_work_area(&data.join("personal/agent-1")));
        assert!(r.in_work_area(&data.join("workflow-runs/r/repo/src")));
        assert!(!r.in_work_area(&data.join("tls")));
        assert!(!r.in_work_area(&data));
        // Only a cwd strictly inside one work area is a session work dir.
        assert!(r.is_session_work_dir(&data.join("personal/a")));
        assert!(!r.is_session_work_dir(&data.join("personal")));
        assert!(!r.is_session_work_dir(&data));
        assert!(!r.is_session_work_dir(tmp.path().parent().unwrap()));
        // A state-file name is refused wherever it sits.
        let own = data.join("personal/a");
        let e = r
            .check(&own, &data.join("personal/a/otto.db"), "otto.db")
            .unwrap_err();
        assert!(matches!(e.0, Error::Forbidden(_)), "{e:?}");
        assert!(r
            .check(&own, &data.join("personal/a/notes.md"), "notes.md")
            .is_ok());
    }

    #[test]
    fn browse_answers_relative_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cwd");
        std::fs::create_dir_all(root.join("src/a")).unwrap();
        std::fs::write(root.join("README.md"), "x").unwrap();
        std::fs::write(root.join("src/a/lib.rs"), "y").unwrap();

        let top = browse_sync(&root, "", None).unwrap();
        assert_eq!(top.path, "");
        assert_eq!(top.parent, None);
        let names: Vec<_> = top.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);

        let nested = browse_sync(&root, "src/a", None).unwrap();
        assert_eq!(nested.parent.as_deref(), Some("src"));
        assert_eq!(nested.entries[0].path, "src/a/lib.rs");

        let read = read_sync(&root, "README.md", None).unwrap();
        assert_eq!(read.path, "README.md");
        assert_eq!(read.content, "x");
        assert_eq!(read.language, "markdown");
    }
}

//! Axum router for **Settings → Skills**: the bundled-skill catalogue plus the
//! manual, drift-aware install/update primitives. All paths are relative to the
//! `/api/v1` mount point.
//!
//! Reads (the catalogue) are open to any authenticated member; installs are
//! root-only. The backend NEVER overwrites an installed skill without first
//! copying the existing tree aside into `<library>/skills-backup/<name>-<ts>/`
//! — that is the "never override without asking" guarantee. The UI gets user
//! consent; the backend always keeps the safety copy.

use std::path::Path as FsPath;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_context::http::ContextCtx;
use otto_context::{user_skills, Library};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::Error;
use serde::{Deserialize, Serialize};

use crate::{install_into, install_state, list_bundled, InstallState};

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// Installs into the shared Otto library are root-only.
fn require_root(user: &AuthUser) -> Result<(), ApiErr> {
    if user.0.is_root {
        Ok(())
    } else {
        Err(Error::Forbidden("installing skills requires root".into()).into())
    }
}

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// One bundled skill plus its drift state against the installed copy.
#[derive(Debug, Clone, Serialize)]
pub struct BundledView {
    pub name: String,
    pub category: String,
    pub version: u32,
    pub description: String,
    /// The installed version, or `None` when not installed.
    pub installed_version: Option<u32>,
    /// `"not_installed" | "up_to_date" | "update_available" | "ahead"`.
    pub state: String,
    /// `true` iff the bundle is strictly newer than the installed copy
    /// (`bundled > installed`) — i.e. the `update_available` state. Lets the
    /// Settings UI show an "Update" button without re-deriving from `state`.
    /// A hand-edited copy that is `ahead` is NOT an update (this stays `false`).
    pub update_available: bool,
}

/// Result of installing a single bundled skill.
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub name: String,
    pub installed: bool,
    pub backed_up: bool,
    pub backup_path: Option<String>,
}

/// Result of a bulk install (all bundled skills, or one category).
#[derive(Debug, Clone, Serialize)]
pub struct InstallAllResult {
    pub installed: Vec<String>,
    pub backed_up: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InstallQuery {
    /// Back up an existing installed copy before overwriting (default `true`).
    #[serde(default = "default_true")]
    backup: bool,
}

#[derive(Debug, Deserialize)]
struct InstallAllQuery {
    /// Restrict the bulk install to a single category, e.g. `review`.
    category: Option<String>,
    #[serde(default = "default_true")]
    backup: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the bundled-skills router. Paths are relative to the `/api/v1` mount.
pub fn router<C: ContextCtx>() -> Router<C> {
    Router::new()
        .route("/library/bundled", get(list_bundled_skills::<C>))
        .route(
            "/library/bundled/{name}/install",
            post(install_bundled_skill::<C>),
        )
        .route("/library/bundled/install-all", post(install_all::<C>))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Map an [`InstallState`] to its wire `state` string + installed version.
fn state_view(st: Option<InstallState>) -> (String, Option<u32>) {
    match st {
        None | Some(InstallState::NotInstalled) => ("not_installed".to_string(), None),
        Some(InstallState::UpToDate) => ("up_to_date".to_string(), None),
        Some(InstallState::UpdateAvailable { installed, .. }) => {
            ("update_available".to_string(), Some(installed))
        }
        Some(InstallState::Ahead { installed, .. }) => ("ahead".to_string(), Some(installed)),
    }
}

async fn list_bundled_skills<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<BundledView>>> {
    let library = s.library();
    let views = list_bundled()
        .into_iter()
        .map(|b| {
            let st = install_state(library, &b.name);
            let (state, installed_version) = state_view(st);
            let update_available = matches!(st, Some(InstallState::UpdateAvailable { .. }));
            BundledView {
                name: b.name,
                category: b.category,
                version: b.version,
                description: b.description,
                installed_version,
                state,
                update_available,
            }
        })
        .collect();
    Ok(Json(views))
}

async fn install_bundled_skill<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
    Query(q): Query<InstallQuery>,
) -> ApiResult<Json<InstallResult>> {
    require_root(&user)?;
    let library = s.library();
    let result = install_one(library, &name, q.backup)?;
    if !result.installed {
        return Err(Error::NotFound(format!("bundled skill '{name}'")).into());
    }
    Ok(Json(result))
}

async fn install_all<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Query(q): Query<InstallAllQuery>,
) -> ApiResult<Json<InstallAllResult>> {
    require_root(&user)?;
    let library = s.library();

    let mut installed = Vec::new();
    let mut backed_up = Vec::new();
    for b in list_bundled() {
        if let Some(cat) = &q.category {
            if &b.category != cat {
                continue;
            }
        }
        let r = install_one(library, &b.name, q.backup)?;
        if r.installed {
            installed.push(r.name.clone());
        }
        if r.backed_up {
            backed_up.push(r.name);
        }
    }
    Ok(Json(InstallAllResult { installed, backed_up }))
}

// ---------------------------------------------------------------------------
// Install primitive (backup-then-install)
// ---------------------------------------------------------------------------

/// Back up an existing installed copy (when `backup` and present), then
/// install the bundled tree. Returns the per-skill outcome. `installed` is
/// `false` only when `name` is not a bundled skill.
fn install_one(library: &Library, name: &str, backup: bool) -> Result<InstallResult, ApiErr> {
    let installed_dir = library.root.join("skills").join(name);
    let mut backup_path: Option<String> = None;

    if backup && installed_dir.exists() {
        let dest = backup_dir(&library.root, name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("create backup dir: {e}")))?;
        }
        copy_tree(&installed_dir, &dest)
            .map_err(|e| Error::Internal(format!("back up skill '{name}': {e}")))?;
        backup_path = Some(dest.to_string_lossy().into_owned());
    }

    let installed = install_into(library, name)
        .map_err(|e| Error::Internal(format!("install skill '{name}': {e}")))?;

    // After the Library copy, mirror the freshly-installed skill tree into each
    // provider's user-level skills dir (~/.claude/skills, $CODEX_HOME/skills,
    // ~/.gemini/skills) so the CLIs discover it globally — not just inside Otto's
    // per-session bundle. Clean-overwrite + per-dir manifest, so this doubles as
    // the update path. Skipped when `name` isn't a bundled skill (installed=false).
    if installed {
        user_skills::install(name, &installed_dir)
            .map_err(|e| Error::Internal(format!("materialize user-level skill '{name}': {e}")))?;
    }

    Ok(InstallResult {
        name: name.to_string(),
        installed,
        backed_up: backup_path.is_some(),
        backup_path,
    })
}

/// `<library_root>/skills-backup/<name>-<unix_secs>/`.
fn backup_dir(root: &FsPath, name: &str) -> std::path::PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    root.join("skills-backup").join(format!("{name}-{secs}"))
}

/// Recursively copy a directory tree from `src` to `dest`.
fn copy_tree(src: &FsPath, dest: &FsPath) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

//! Axum router for **Skills Lab → Bundled**: the bundled-skill catalogue plus the
//! manual, drift-aware install/update primitives. All paths are relative to the
//! `/api/v1` mount point.
//!
//! Reads (the catalogue) are open to any authenticated member; installs are
//! root-only. The backend NEVER overwrites an installed skill without first
//! copying the existing tree aside into `<library>/skills-backup/<name>-<ts>/`
//! — that is the "never override without asking" guarantee. The UI gets user
//! consent; the backend always keeps the safety copy.

use std::path::Path as FsPath;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_context::http::ContextCtx;
use otto_context::library::{
    prune_skill_backups as prune_backups, reserve_skill_backup as backup_dir, SKILL_BACKUPS_KEPT,
};
use otto_context::{user_skills, Library};
use otto_core::api::{BundledSkillContent, Problem, SkillFileEntry};
use otto_core::auth::AuthUser;
use otto_core::Error;
use serde::{Deserialize, Serialize};

use crate::{
    bundled_body, bundled_file, bundled_files, install_into, install_state, list_bundled,
    InstallState,
};

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
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
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
    if otto_core::auth::root_authority(&user.0) {
        Ok(())
    } else if user.0.is_root {
        Err(otto_core::auth::root_refusal().into())
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
    /// `"not_installed" | "up_to_date" | "update_available" | "ahead" |
    /// "modified"` — `modified`: the installed tree is no longer what Otto
    /// installed (edited without a version bump); updating it asks first.
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
    /// Provider skill dirs (`~/.claude/skills`, …) left untouched because they
    /// already hold a user-owned skill of this name (S7-304).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_owned: Vec<String>,
}

/// Result of a bulk install (all bundled skills, or one category).
#[derive(Debug, Clone, Serialize)]
pub struct InstallAllResult {
    pub installed: Vec<String>,
    pub backed_up: Vec<String>,
    /// Already at the bundled version (or ahead of it without `force`) — left
    /// untouched in the Library, no backup made. Current Library copies still
    /// reconcile their provider mirrors so failed copies can be retried.
    pub skipped: Vec<String>,
    /// Installed copies the person edited since Otto installed them (their
    /// tree hash no longer matches): left untouched unless `force`.
    #[serde(default)]
    pub modified: Vec<String>,
    /// Skills whose install failed; the rest of the batch still ran.
    pub failed: Vec<InstallFailure>,
    /// `<provider skills dir>/<name>` entries left untouched because a
    /// user-owned skill of that name already lives there (S7-304).
    #[serde(default)]
    pub user_owned: Vec<String>,
}

/// One skill a bulk install could not install.
#[derive(Debug, Clone, Serialize)]
pub struct InstallFailure {
    pub name: String,
    pub error: String,
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
    /// Also reinstall skills whose installed copy is AHEAD of the bundled
    /// version (local edits); default `false` leaves them alone.
    #[serde(default)]
    force: bool,
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
        .route("/library/bundled/install-all", post(install_all::<C>))
        .route(
            "/library/bundled/{name}/install",
            post(install_bundled_skill::<C>),
        )
        .route("/library/bundled/{name}", get(get_bundled_skill::<C>))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Map an [`InstallState`] to its wire `state` string + installed version.
fn state_view(st: Option<InstallState>, on_disk: Option<u32>) -> (String, Option<u32>) {
    match st {
        None | Some(InstallState::NotInstalled) => ("not_installed".to_string(), None),
        // The bundle carries no installed version for this state: read it off
        // the installed SKILL.md so a badge can say "Installed v3".
        Some(InstallState::UpToDate) => ("up_to_date".to_string(), on_disk),
        Some(InstallState::UpdateAvailable { installed, .. }) => {
            ("update_available".to_string(), Some(installed))
        }
        Some(InstallState::Ahead { installed, .. }) => ("ahead".to_string(), Some(installed)),
        Some(InstallState::Modified { installed, .. }) => ("modified".to_string(), Some(installed)),
    }
}

async fn list_bundled_skills<C: ContextCtx>(
    State(s): State<C>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<BundledView>>> {
    // `install_state` hashes each installed tree: blocking fs work.
    let library = s.library().clone();
    let views = tokio::task::spawn_blocking(move || bundled_views(&library))
        .await
        .map_err(|e| Error::Internal(format!("list task: {e}")))?;
    Ok(Json(views))
}

fn bundled_views(library: &Library) -> Vec<BundledView> {
    list_bundled()
        .into_iter()
        .map(|b| {
            let st = install_state(library, &b.name);
            let on_disk = if matches!(st, Some(InstallState::UpToDate)) {
                crate::installed_version(library, &b.name)
            } else {
                None
            };
            let (state, installed_version) = state_view(st, on_disk);
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
        .collect()
}

/// Full content of one bundled skill (body + file list) so the Skills Lab can
/// view a skill that is not installed into the Library yet.
async fn get_bundled_skill<C: ContextCtx>(
    State(_s): State<C>,
    Extension(_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> ApiResult<Json<BundledSkillContent>> {
    let meta = list_bundled()
        .into_iter()
        .find(|b| b.name == name)
        .ok_or_else(|| Error::NotFound(format!("bundled skill '{name}'")))?;
    let body = bundled_body(&name).unwrap_or_default();
    let files = bundled_files(&name)
        .into_iter()
        .map(|path| {
            let size = bundled_file(&name, &path)
                .map(|c| c.len() as u64)
                .unwrap_or(0);
            SkillFileEntry {
                path,
                size,
                binary: false,
            }
        })
        .collect();
    Ok(Json(BundledSkillContent {
        name: meta.name,
        category: meta.category,
        version: meta.version,
        description: meta.description,
        body,
        files,
    }))
}

async fn install_bundled_skill<C: ContextCtx>(
    State(s): State<C>,
    Extension(user): Extension<AuthUser>,
    Path(name): Path<String>,
    Query(q): Query<InstallQuery>,
) -> ApiResult<Json<InstallResult>> {
    require_root(&user)?;
    let library = s.library().clone();
    // Recursive copies + the provider-dir mirror are blocking fs work: keep
    // them off the async workers.
    let result = tokio::task::spawn_blocking(move || install_one(&library, &name, q.backup))
        .await
        .map_err(|e| Error::Internal(format!("install task: {e}")))??;
    if !result.installed {
        let name = result.name;
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
    let library = s.library().clone();
    let result = tokio::task::spawn_blocking(move || {
        install_many(&library, q.category.as_deref(), q.backup, q.force)
    })
    .await
    .map_err(|e| Error::Internal(format!("install task: {e}")))?;
    Ok(Json(result))
}

/// The bulk install (12-mcp S1): skips skills already at the bundled version
/// (and, unless `force`, ones whose installed copy is ahead — local edits),
/// and keeps going past a failing skill, reporting it in `failed`.
fn install_many(
    library: &Library,
    category: Option<&str>,
    backup: bool,
    force: bool,
) -> InstallAllResult {
    install_many_into_dirs(
        library,
        category,
        backup,
        force,
        &user_skills::provider_skill_dirs(),
    )
}

fn install_many_into_dirs(
    library: &Library,
    category: Option<&str>,
    backup: bool,
    force: bool,
    provider_dirs: &[std::path::PathBuf],
) -> InstallAllResult {
    let mut out = InstallAllResult {
        installed: Vec::new(),
        backed_up: Vec::new(),
        skipped: Vec::new(),
        modified: Vec::new(),
        failed: Vec::new(),
        user_owned: Vec::new(),
    };
    for b in list_bundled() {
        if category.is_some_and(|c| b.category != c) {
            continue;
        }
        match install_state(library, &b.name) {
            Some(InstallState::UpToDate) => {
                match user_skills::install_into_dirs(
                    provider_dirs,
                    &b.name,
                    &library.root.join("skills").join(&b.name),
                ) {
                    Ok(user_owned) => {
                        out.user_owned.extend(
                            user_owned
                                .into_iter()
                                .map(|dir| dir.join(&b.name).to_string_lossy().into_owned()),
                        );
                        out.skipped.push(b.name);
                    }
                    Err(error) => out.failed.push(InstallFailure {
                        name: b.name,
                        error: format!("materialize user-level skill: {error}"),
                    }),
                }
                continue;
            }
            Some(InstallState::Ahead { .. }) if !force => {
                out.skipped.push(b.name);
                continue;
            }
            // The person's edits: never overwritten by a bulk install.
            Some(InstallState::Modified { .. }) if !force => {
                out.modified.push(b.name);
                continue;
            }
            _ => {}
        }
        match install_one_into_dirs(library, &b.name, backup, provider_dirs) {
            Ok(r) => {
                out.user_owned.extend(r.user_owned.iter().cloned());
                if r.installed {
                    out.installed.push(r.name.clone());
                }
                if r.backed_up {
                    out.backed_up.push(r.name);
                }
            }
            Err(ApiErr(e)) => out.failed.push(InstallFailure {
                name: b.name,
                error: e.to_string(),
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Install primitive (backup-then-install)
// ---------------------------------------------------------------------------

/// Back up an existing installed copy (when `backup` and present), then
/// install the bundled tree. Returns the per-skill outcome. `installed` is
/// `false` only when `name` is not a bundled skill.
fn install_one(library: &Library, name: &str, backup: bool) -> Result<InstallResult, ApiErr> {
    install_one_into_dirs(library, name, backup, &user_skills::provider_skill_dirs())
}

fn install_one_into_dirs(
    library: &Library,
    name: &str,
    backup: bool,
    provider_dirs: &[std::path::PathBuf],
) -> Result<InstallResult, ApiErr> {
    // `name` comes off the URL; a separator/`..` would aim the backup copy (and
    // the install) outside the Library — same not-a-bundled-skill outcome as an
    // unknown name, which the handler surfaces as 404.
    let Some(name) = otto_core::paths::safe_component(name) else {
        return Ok(InstallResult {
            name: name.to_string(),
            installed: false,
            backed_up: false,
            backup_path: None,
            user_owned: Vec::new(),
        });
    };
    let installed_dir = library.root.join("skills").join(name);
    let mut backup_path: Option<String> = None;
    // An edited copy is always backed up, whatever the caller asked.
    let backup = backup
        || matches!(
            install_state(library, name),
            Some(InstallState::Modified { .. })
        );

    if backup && installed_dir.exists() {
        let dest = backup_dir(&library.root, name)
            .map_err(|e| Error::Internal(format!("reserve backup directory: {e}")))?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("create backup dir: {e}")))?;
        }
        copy_tree(&installed_dir, &dest)
            .map_err(|e| Error::Internal(format!("back up skill '{name}': {e}")))?;
        backup_path = Some(dest.to_string_lossy().into_owned());
        prune_backups(&library.root, name, SKILL_BACKUPS_KEPT);
    }

    let installed = install_into(library, name)
        .map_err(|e| Error::Internal(format!("install skill '{name}': {e}")))?;

    // After the Library copy, mirror the freshly-installed skill tree into each
    // provider's user-level skills dir (~/.claude/skills, $CODEX_HOME/skills,
    // ~/.gemini/skills) so the CLIs discover it globally — not just inside Otto's
    // per-session bundle. Clean-overwrite + per-dir manifest, so this doubles as
    // the update path. Skipped when `name` isn't a bundled skill (installed=false).
    // A provider dir already holding a user-owned `<name>` is left alone.
    let mut user_owned = Vec::new();
    if installed {
        user_owned = user_skills::install_into_dirs(provider_dirs, name, &installed_dir)
            .map_err(|e| Error::Internal(format!("materialize user-level skill '{name}': {e}")))?
            .into_iter()
            .map(|d| d.join(name).to_string_lossy().into_owned())
            .collect();
    }

    Ok(InstallResult {
        name: name.to_string(),
        installed,
        backed_up: backup_path.is_some(),
        backup_path,
        user_owned,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_backups_never_reuse_the_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = backup_dir(dir.path(), "grill").unwrap();
        let second = backup_dir(dir.path(), "grill").unwrap();
        assert_ne!(
            first, second,
            "a later install must not overwrite the earlier safety copy"
        );
        std::fs::write(first.join("SKILL.md"), "original").unwrap();
        std::fs::write(second.join("SKILL.md"), "edited").unwrap();
        assert_eq!(
            std::fs::read_to_string(first.join("SKILL.md")).unwrap(),
            "original"
        );
        prune_backups(dir.path(), "grill", 1);
        assert_eq!(
            std::fs::read_dir(dir.path().join("skills-backup"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn prune_backups_keeps_newest_n_for_that_skill_only() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("skills-backup");
        for d in [
            "grill-1",
            "grill-5",
            "grill-3",
            "grill-9",
            "grill-x-2",
            "other-1",
        ] {
            std::fs::create_dir_all(b.join(d)).unwrap();
        }
        prune_backups(dir.path(), "grill", 2);
        let mut left: Vec<String> = std::fs::read_dir(&b)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        // `grill-x-2` isn't a `grill-<secs>` backup; `other-1` is another skill.
        assert_eq!(left, vec!["grill-5", "grill-9", "grill-x-2", "other-1"]);
    }

    #[test]
    fn bulk_retry_repairs_provider_mirrors_after_library_was_installed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path().join("library"));
        let skill = list_bundled().into_iter().next().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        let providers = vec![dir.path().join("provider-a"), blocked.join("skills")];
        let first = install_many_into_dirs(&lib, Some(&skill.category), true, false, &providers);
        assert!(first
            .failed
            .iter()
            .any(|failure| failure.name == skill.name));
        assert_eq!(
            install_state(&lib, &skill.name),
            Some(InstallState::UpToDate)
        );
        std::fs::remove_file(&blocked).unwrap();
        let retry = install_many_into_dirs(&lib, Some(&skill.category), true, false, &providers);
        assert!(retry.failed.is_empty(), "{:?}", retry.failed);
        for provider in &providers {
            assert_eq!(
                std::fs::read(provider.join(&skill.name).join("SKILL.md")).unwrap(),
                std::fs::read(lib.root.join("skills").join(&skill.name).join("SKILL.md")).unwrap()
            );
            assert!(std::fs::read_to_string(provider.join(".otto-managed.json"))
                .unwrap()
                .contains(&skill.name));
        }
        assert!(
            !lib.root.join("skills-backup").exists(),
            "retry must not rewrite current Library copies"
        );
    }

    #[test]
    fn bulk_reconciliation_preserves_ahead_and_user_owned_skills() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path().join("library"));
        let skill = list_bundled().into_iter().next().unwrap();
        let installed = lib.root.join("skills").join(&skill.name);
        std::fs::create_dir_all(&installed).unwrap();
        let ahead = format!("---\nversion: {}\n---\nlocal edits", skill.version + 1);
        std::fs::write(installed.join("SKILL.md"), &ahead).unwrap();
        let provider = dir.path().join("provider");
        let result = install_many_into_dirs(
            &lib,
            Some(&skill.category),
            true,
            false,
            std::slice::from_ref(&provider),
        );
        assert!(result.skipped.contains(&skill.name));
        assert_eq!(
            std::fs::read_to_string(installed.join("SKILL.md")).unwrap(),
            ahead
        );
        assert!(
            !provider.join(&skill.name).exists(),
            "ahead copies must remain opt-in"
        );

        assert!(install_into(&lib, &skill.name).unwrap());
        std::fs::create_dir_all(provider.join(&skill.name)).unwrap();
        std::fs::write(provider.join(&skill.name).join("SKILL.md"), "user copy").unwrap();
        let result = install_many_into_dirs(
            &lib,
            Some(&skill.category),
            true,
            false,
            std::slice::from_ref(&provider),
        );
        assert!(result.failed.is_empty());
        assert!(result
            .user_owned
            .contains(&provider.join(&skill.name).to_string_lossy().into_owned()));
        assert_eq!(
            std::fs::read_to_string(provider.join(&skill.name).join("SKILL.md")).unwrap(),
            "user copy"
        );
    }

    /// Install-all never overwrites a copy the person edited without bumping
    /// its version, and says so; a single update of it always backs it up.
    #[test]
    fn install_all_skips_modified_and_single_update_backs_it_up() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path().join("library"));
        let skill = list_bundled().into_iter().next().unwrap();
        let provider = dir.path().join("provider");
        assert!(install_into(&lib, &skill.name).unwrap());
        let md = lib.root.join("skills").join(&skill.name).join("SKILL.md");
        let edited = format!("{}\nlocal rule\n", std::fs::read_to_string(&md).unwrap());
        std::fs::write(&md, &edited).unwrap();

        let all = install_many_into_dirs(
            &lib,
            Some(&skill.category),
            false,
            false,
            std::slice::from_ref(&provider),
        );
        assert!(all.modified.contains(&skill.name), "{all:?}");
        assert!(!all.installed.contains(&skill.name));
        assert_eq!(std::fs::read_to_string(&md).unwrap(), edited);

        // backup=false is overridden for an edited copy.
        let one = install_one_into_dirs(&lib, &skill.name, false, std::slice::from_ref(&provider))
            .map_err(|ApiErr(e)| e)
            .unwrap();
        assert!(one.backed_up);
        let backup = std::path::PathBuf::from(one.backup_path.unwrap());
        assert_eq!(
            std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
            edited
        );
        assert_eq!(
            install_state(&lib, &skill.name),
            Some(InstallState::UpToDate)
        );
    }

    #[test]
    fn up_to_date_view_reports_the_installed_version() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let skill = list_bundled().into_iter().next().unwrap();
        assert!(install_into(&lib, &skill.name).unwrap());
        let view = bundled_views(&lib)
            .into_iter()
            .find(|v| v.name == skill.name)
            .unwrap();
        assert_eq!(view.state, "up_to_date");
        assert_eq!(view.installed_version, Some(skill.version));
    }

    #[test]
    fn install_many_skips_up_to_date_skills() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let first = list_bundled().into_iter().next().expect("a bundled skill");
        let cat = first.category.clone();
        // Seed every skill of the category into the Library only, so the bulk
        // install skips Library writes but reconciles an isolated provider mirror.
        for b in list_bundled().into_iter().filter(|b| b.category == cat) {
            assert!(install_into(&lib, &b.name).unwrap());
        }
        let r = install_many_into_dirs(
            &lib,
            Some(&cat),
            true,
            false,
            &[dir.path().join("provider")],
        );
        assert!(r.skipped.contains(&first.name), "{r:?}");
        assert!(r.installed.is_empty() && r.backed_up.is_empty() && r.failed.is_empty());
    }
}

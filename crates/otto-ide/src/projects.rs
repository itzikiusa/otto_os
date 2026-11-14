//! Named Multi-Repo IDE Projects store (`ide_named_projects` in SQLite) and
//! bulk child-repository scanner for multi-repo workspaces.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use sqlx::Row;

use crate::target::{canonicalize_local_root_dir, enforce_non_root_path};
use crate::types::{IdeNamedProject, IdeScanReposResp, IdeScannedRepo, IdeUpsertProjectReq};

#[derive(Clone, Debug)]
struct StoredProject {
    owner_user_id: String,
    project: IdeNamedProject,
}

#[derive(Default)]
struct ProjectCache {
    hydrated: bool,
    items: Vec<StoredProject>,
}

#[derive(Clone)]
pub struct ProjectStore {
    legacy_file_path: PathBuf,
    cache: Arc<RwLock<ProjectCache>>,
}

impl ProjectStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let legacy_file_path = data_dir.join("ide-projects.json");
        let initial = fs::read_to_string(&legacy_file_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<IdeNamedProject>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|project| StoredProject {
                owner_user_id: String::new(),
                project,
            })
            .collect();
        Self {
            legacy_file_path,
            cache: Arc::new(RwLock::new(ProjectCache {
                hydrated: false,
                items: initial,
            })),
        }
    }

    /// Ensure the in-memory cache is hydrated from `ide_named_projects`, migrating
    /// any pre-SQLite `<data_dir>/ide-projects.json` entries on first boot.
    pub async fn ensure_hydrated(&self, pool: &DbPool) -> Result<()> {
        if self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .hydrated
        {
            return Ok(());
        }
        self.reload_from_db(pool).await
    }

    pub async fn reload_from_db(&self, pool: &DbPool) -> Result<()> {
        let mut stored = load_projects_from_db(pool).await?;
        if stored.is_empty() && self.legacy_file_path.is_file() {
            let legacy_path = self.legacy_file_path.clone();
            let legacy_items: Vec<IdeNamedProject> = tokio::task::spawn_blocking(move || {
                fs::read_to_string(&legacy_path)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Vec<IdeNamedProject>>(&raw).ok())
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();

            for item in legacy_items {
                let folders_json =
                    serde_json::to_string(&item.folders).unwrap_or_else(|_| "[]".to_string());
                let _ = sqlx::query(
                    "INSERT OR IGNORE INTO ide_named_projects
                        (id, owner_user_id, name, primary_root, folders_json, pinned, updated_at)
                     VALUES (?1, '', ?2, ?3, ?4, ?5, ?6)",
                )
                .bind(&item.id)
                .bind(&item.name)
                .bind(&item.primary_root)
                .bind(&folders_json)
                .bind(if item.pinned { 1_i64 } else { 0_i64 })
                .bind(&item.updated_at)
                .execute(pool)
                .await;
            }
            let legacy_rm = self.legacy_file_path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = fs::remove_file(legacy_rm);
            })
            .await;
            stored = load_projects_from_db(pool).await?;
        }

        let mut guard = self.cache.write().unwrap_or_else(|e| e.into_inner());
        guard.items = stored;
        guard.hydrated = true;
        Ok(())
    }

    pub async fn list_for_user(&self, pool: &DbPool, user: &User) -> Result<Vec<IdeNamedProject>> {
        self.reload_from_db(pool).await?;
        let is_root = user.is_root;
        let user_id = user.id.clone();
        let snapshot: Vec<StoredProject> = self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .iter()
            .filter(|entry| can_access_project(entry, &user_id, is_root))
            .cloned()
            .collect();

        let validated = tokio::task::spawn_blocking(move || {
            let mut items = Vec::with_capacity(snapshot.len());
            for entry in snapshot {
                if let Some(proj) = revalidate_project_paths(entry.project, is_root) {
                    items.push(proj);
                }
            }
            sort_projects(&mut items);
            items
        })
        .await
        .map_err(|e| Error::Internal(format!("list projects worker failed: {e}")))?;

        Ok(validated)
    }

    /// Return all folders across Named Projects owned by `user` (from the hydrated cache).
    pub fn allowed_roots_for_user(&self, user: &User) -> Vec<String> {
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for entry in &guard.items {
            if !can_access_project(entry, &user.id, user.is_root) {
                continue;
            }
            if !entry.project.primary_root.trim().is_empty()
                && !out.contains(&entry.project.primary_root)
            {
                out.push(entry.project.primary_root.clone());
            }
            for f in &entry.project.folders {
                if !f.trim().is_empty() && !out.contains(f) {
                    out.push(f.clone());
                }
            }
        }
        out
    }

    /// Return attached sibling folders configured for any Named Project visible to
    /// `user` whose `primary_root` or any member entry in `folders` matches `root`.
    #[allow(clippy::disallowed_methods)]
    pub fn folders_for_root_for_user(&self, root: &str, user: &User) -> Vec<String> {
        let Some(lookup) = normalize_root_for_match(root) else {
            return Vec::new();
        };
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for entry in &guard.items {
            if !can_access_project(entry, &user.id, user.is_root) {
                continue;
            }
            if project_matches_root(&entry.project, &lookup) {
                return filter_folders_for_caller(&entry.project.folders, user.is_root);
            }
        }
        Vec::new()
    }

    /// Root-level lookup matching `primary_root` and every member entry in `folders`.
    #[allow(clippy::disallowed_methods)]
    pub fn folders_for_root(&self, root: &str) -> Vec<String> {
        let Some(lookup) = normalize_root_for_match(root) else {
            return Vec::new();
        };
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for entry in &guard.items {
            if project_matches_root(&entry.project, &lookup) {
                return entry.project.folders.clone();
            }
        }
        Vec::new()
    }

    pub async fn upsert_for_user(
        &self,
        pool: &DbPool,
        user: &User,
        req: IdeUpsertProjectReq,
    ) -> Result<IdeNamedProject> {
        self.ensure_hydrated(pool).await?;
        let is_root = user.is_root;
        let NormalizedUpsertProject {
            name,
            primary_root: primary_str,
            folders,
            pinned,
            explicit_id,
        } = tokio::task::spawn_blocking(move || normalize_upsert_req(req, is_root))
            .await
            .map_err(|e| Error::Internal(format!("upsert project worker failed: {e}")))??;

        let id = if let Some(req_id) = explicit_id {
            let existing_owner: Option<String> =
                sqlx::query_scalar("SELECT owner_user_id FROM ide_named_projects WHERE id = ?1")
                    .bind(&req_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| Error::Internal(format!("query ide_named_projects owner: {e}")))?;
            if let Some(owner) = &existing_owner {
                if !user.is_root && owner != &user.id {
                    return Err(Error::Forbidden(
                        "cannot overwrite a Named IDE Project owned by another user".into(),
                    ));
                }
            }
            req_id
        } else {
            let existing_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM ide_named_projects
                 WHERE owner_user_id = ?1 AND primary_root = ?2 AND name = ?3
                 LIMIT 1",
            )
            .bind(&user.id)
            .bind(&primary_str)
            .bind(&name)
            .fetch_optional(pool)
            .await
            .map_err(|e| Error::Internal(format!("lookup ide_named_projects id: {e}")))?;
            existing_id.unwrap_or_else(|| format!("ideproj_{}", new_id()))
        };

        let existing_owner: Option<String> =
            sqlx::query_scalar("SELECT owner_user_id FROM ide_named_projects WHERE id = ?1")
                .bind(&id)
                .fetch_optional(pool)
                .await
                .map_err(|e| Error::Internal(format!("query ide_named_projects owner: {e}")))?;

        let owner_to_store = match existing_owner {
            Some(owner) if !owner.is_empty() => owner,
            _ => user.id.clone(),
        };

        let now = Utc::now().to_rfc3339();
        let folders_json =
            serde_json::to_string(&folders).map_err(|e| Error::Internal(e.to_string()))?;

        sqlx::query(
            "INSERT INTO ide_named_projects
                (id, owner_user_id, name, primary_root, folders_json, pinned, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                owner_user_id = excluded.owner_user_id,
                name = excluded.name,
                primary_root = excluded.primary_root,
                folders_json = excluded.folders_json,
                pinned = excluded.pinned,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&owner_to_store)
        .bind(&name)
        .bind(&primary_str)
        .bind(&folders_json)
        .bind(if pinned { 1_i64 } else { 0_i64 })
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("persist ide_named_projects: {e}")))?;

        self.reload_from_db(pool).await?;

        Ok(IdeNamedProject {
            id,
            name,
            primary_root: primary_str,
            folders,
            pinned,
            updated_at: now,
        })
    }

    pub async fn delete_for_user(&self, pool: &DbPool, user: &User, id: &str) -> Result<bool> {
        self.ensure_hydrated(pool).await?;
        let target_id = id.trim();
        let existing_owner: Option<String> =
            sqlx::query_scalar("SELECT owner_user_id FROM ide_named_projects WHERE id = ?1")
                .bind(target_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| Error::Internal(format!("query ide_named_projects owner: {e}")))?;

        let Some(owner) = existing_owner else {
            return Ok(false);
        };
        if !user.is_root && owner != user.id {
            return Err(Error::Forbidden(
                "cannot delete a Named IDE Project owned by another user".into(),
            ));
        }

        let res = sqlx::query("DELETE FROM ide_named_projects WHERE id = ?1")
            .bind(target_id)
            .execute(pool)
            .await
            .map_err(|e| Error::Internal(format!("delete ide_named_projects: {e}")))?;

        self.reload_from_db(pool).await?;
        Ok(res.rows_affected() > 0)
    }
}

fn can_access_project(entry: &StoredProject, user_id: &str, is_root: bool) -> bool {
    is_root || (!entry.owner_user_id.is_empty() && entry.owner_user_id == user_id)
}

fn sort_projects(items: &mut [IdeNamedProject]) {
    items.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.name.cmp(&b.name))
    });
}

async fn load_projects_from_db(pool: &DbPool) -> Result<Vec<StoredProject>> {
    let rows = sqlx::query(
        "SELECT id, owner_user_id, name, primary_root, folders_json, pinned, updated_at
         FROM ide_named_projects",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Error::Internal(format!("load ide_named_projects: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let folders_json: String = row.try_get("folders_json").unwrap_or_default();
        let folders: Vec<String> = serde_json::from_str(&folders_json).unwrap_or_default();
        out.push(StoredProject {
            owner_user_id: row.try_get("owner_user_id").unwrap_or_default(),
            project: IdeNamedProject {
                id: row.try_get("id").unwrap_or_default(),
                name: row.try_get("name").unwrap_or_default(),
                primary_root: row.try_get("primary_root").unwrap_or_default(),
                folders,
                pinned: row.try_get::<i64, _>("pinned").unwrap_or(0) != 0,
                updated_at: row.try_get("updated_at").unwrap_or_default(),
            },
        });
    }
    Ok(out)
}

struct NormalizedUpsertProject {
    name: String,
    primary_root: String,
    folders: Vec<String>,
    pinned: bool,
    explicit_id: Option<String>,
}

#[allow(clippy::disallowed_methods)]
fn normalize_upsert_req(
    req: IdeUpsertProjectReq,
    is_root_user: bool,
) -> Result<NormalizedUpsertProject> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(Error::Invalid("project name must not be empty".into()));
    }
    let raw_primary = if !req.primary_root.trim().is_empty() {
        req.primary_root.trim()
    } else {
        req.folders
            .first()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Invalid("primary_root or folders[0] is required".into()))?
    };
    let primary_canon = canonicalize_local_root_dir(raw_primary, is_root_user)?;
    let primary_str = primary_canon.to_string_lossy().into_owned();

    let mut folders = vec![primary_str.clone()];
    for raw in &req.folders {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canon = canonicalize_local_root_dir(trimmed, is_root_user)?;
        let s = canon.to_string_lossy().into_owned();
        if !folders.contains(&s) {
            folders.push(s);
        }
    }

    let explicit_id = req
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(NormalizedUpsertProject {
        name,
        primary_root: primary_str,
        folders,
        pinned: req.pinned,
        explicit_id,
    })
}

#[allow(clippy::disallowed_methods)]
fn revalidate_project_paths(mut proj: IdeNamedProject, is_root: bool) -> Option<IdeNamedProject> {
    let primary_expanded = PathBuf::from(otto_core::paths::expand_tilde(proj.primary_root.trim()));
    let primary_canon = fs::canonicalize(&primary_expanded).unwrap_or(primary_expanded);
    if !is_root && enforce_non_root_path(&primary_canon).is_err() {
        return None;
    }
    proj.primary_root = primary_canon.to_string_lossy().into_owned();
    proj.folders = filter_folders_for_caller(&proj.folders, is_root);
    if !proj.folders.contains(&proj.primary_root) {
        proj.folders.insert(0, proj.primary_root.clone());
    }
    Some(proj)
}

#[allow(clippy::disallowed_methods)]
fn filter_folders_for_caller(folders: &[String], is_root: bool) -> Vec<String> {
    let mut out = Vec::with_capacity(folders.len());
    for raw in folders {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let expanded = PathBuf::from(otto_core::paths::expand_tilde(trimmed));
        let canon = fs::canonicalize(&expanded).unwrap_or(expanded);
        if !is_root && enforce_non_root_path(&canon).is_err() {
            continue;
        }
        let s = canon.to_string_lossy().into_owned();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[derive(Debug)]
struct NormalizedLookup {
    raw_trimmed: String,
    canonical: String,
}

#[allow(clippy::disallowed_methods)]
fn normalize_root_for_match(root: &str) -> Option<NormalizedLookup> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = PathBuf::from(otto_core::paths::expand_tilde(trimmed));
    let raw_str = trim_trailing_slash(&expanded.to_string_lossy());
    let canonical = fs::canonicalize(&expanded)
        .map(|p| trim_trailing_slash(&p.to_string_lossy()))
        .unwrap_or_else(|_| raw_str.clone());
    Some(NormalizedLookup {
        raw_trimmed: raw_str,
        canonical,
    })
}

fn trim_trailing_slash(s: &str) -> String {
    let stripped = s.trim().trim_end_matches('/');
    if stripped.is_empty() && s.trim().starts_with('/') {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

#[allow(clippy::disallowed_methods)]
fn path_matches_lookup(candidate: &str, lookup: &NormalizedLookup) -> bool {
    let Some(cand) = normalize_root_for_match(candidate) else {
        return false;
    };
    cand.canonical == lookup.canonical
        || cand.raw_trimmed == lookup.raw_trimmed
        || cand.canonical == lookup.raw_trimmed
        || cand.raw_trimmed == lookup.canonical
}

fn project_matches_root(proj: &IdeNamedProject, lookup: &NormalizedLookup) -> bool {
    path_matches_lookup(&proj.primary_root, lookup)
        || proj.folders.iter().any(|f| path_matches_lookup(f, lookup))
}

/// Fast 1-level scan of `parent_path` returning child repositories and folders
/// so users can bulk-select multi-repo workspaces into one Named IDE Project.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub fn scan_parent_repos(parent_path: &str, is_root_user: bool) -> Result<IdeScanReposResp> {
    let parent = canonicalize_local_root_dir(parent_path, is_root_user)?;
    let rd = fs::read_dir(&parent).map_err(|e| {
        Error::NotFound(format!("cannot read directory '{}': {e}", parent.display()))
    })?;

    let mut repos = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "Library") {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        let canon = fs::canonicalize(&path).unwrap_or(path);
        if !is_root_user && enforce_non_root_path(&canon).is_err() {
            continue;
        }
        let has_go_mod = canon.join("go.mod").is_file() || canon.join("go.work").is_file();
        let has_pom_or_gradle = canon.join("pom.xml").is_file()
            || canon.join("build.gradle").is_file()
            || canon.join("build.gradle.kts").is_file();
        let has_cargo = canon.join("Cargo.toml").is_file();
        let has_node = canon.join("package.json").is_file();
        let has_py =
            canon.join("pyproject.toml").is_file() || canon.join("requirements.txt").is_file();
        let has_git = canon.join(".git").exists();

        let kind = if has_go_mod {
            "go"
        } else if has_pom_or_gradle {
            "java"
        } else if has_cargo {
            "rust"
        } else if has_node {
            "node"
        } else if has_py {
            "python"
        } else if has_git {
            "git"
        } else {
            "folder"
        };

        repos.push(IdeScannedRepo {
            name,
            path: canon.to_string_lossy().into_owned(),
            kind: kind.to_string(),
            has_go_mod,
            has_pom_or_gradle,
            has_git,
        });
    }
    repos.sort_by(|a, b| {
        let a_is_repo = a.kind != "folder";
        let b_is_repo = b.kind != "folder";
        b_is_repo
            .cmp(&a_is_repo)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(IdeScanReposResp {
        parent_path: parent.to_string_lossy().into_owned(),
        repos,
    })
}

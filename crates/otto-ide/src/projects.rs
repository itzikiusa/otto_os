//! Named Multi-Repo IDE Projects store (`ide_named_projects` in SQLite) and
//! bulk child-repository scanner for multi-repo workspaces.

mod scanner;

pub use self::scanner::scan_parent_repos;
use self::scanner::{
    build_stored_project, legacy_project_authorized, load_projects_from_db,
    migrate_legacy_projects_atomic, normalize_lookup_fast, normalize_upsert_req,
    stored_project_primary_matches, NormalizedUpsertProject, StoredProject,
};

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use chrono::Utc;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use sqlx::Row;

use crate::types::{IdeNamedProject, IdeUpsertProjectReq};
use crate::IdeCtx;

#[derive(Default)]
struct ProjectCache {
    hydrated: bool,
    items: Vec<StoredProject>,
}

#[derive(Clone)]
pub struct ProjectStore {
    legacy_file_path: PathBuf,
    cache: Arc<RwLock<ProjectCache>>,
    generation: Arc<AtomicU64>,
}

impl ProjectStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let legacy_file_path = data_dir.join("ide-projects.json");
        let initial = fs::read_to_string(&legacy_file_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<IdeNamedProject>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|proj| build_stored_project(String::new(), proj))
            .collect();
        Self {
            legacy_file_path,
            cache: Arc::new(RwLock::new(ProjectCache {
                hydrated: false,
                items: initial,
            })),
            generation: Arc::new(AtomicU64::new(1)),
        }
    }

    pub(crate) fn cache_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Ensure the in-memory cache is hydrated from `ide_named_projects`, migrating
    /// any pre-SQLite `<data_dir>/ide-projects.json` entries atomically on first boot.
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
        let mut raw_rows = load_projects_from_db(pool).await?;
        if raw_rows.is_empty() && self.legacy_file_path.is_file() {
            migrate_legacy_projects_atomic(pool, &self.legacy_file_path).await?;
            raw_rows = load_projects_from_db(pool).await?;
        }

        let hydrated_items = tokio::task::spawn_blocking(move || {
            let mut out: Vec<StoredProject> = raw_rows
                .into_iter()
                .map(|(owner, proj)| build_stored_project(owner, proj))
                .collect();
            out.sort_by(|a, b| {
                b.project
                    .pinned
                    .cmp(&a.project.pinned)
                    .then_with(|| b.project.updated_at.cmp(&a.project.updated_at))
                    .then_with(|| a.project.name.cmp(&b.project.name))
            });
            out
        })
        .await
        .map_err(|e| Error::Internal(format!("hydrate projects worker failed: {e}")))?;

        let mut guard = self.cache.write().unwrap_or_else(|e| e.into_inner());
        guard.items = hydrated_items;
        guard.hydrated = true;
        drop(guard);
        self.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub async fn list_for_user(&self, pool: &DbPool, user: &User) -> Result<Vec<IdeNamedProject>> {
        self.ensure_hydrated(pool).await?;
        let has_root_auth = otto_core::auth::root_authority(user);
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let mut items = Vec::with_capacity(guard.items.len());
        for entry in &guard.items {
            if !can_access_owned_project(entry, &user.id, has_root_auth) {
                continue;
            }
            if let Some(proj) = project_for_caller(entry, has_root_auth) {
                items.push(proj);
            }
        }
        sort_projects(&mut items);
        Ok(items)
    }

    /// List Named Projects visible to `user`, including legacy migrated rows
    /// (`owner_user_id == ""`) whose roots are authorized via `ctx.authorize_local_target`.
    pub async fn list_for_user_with_ctx<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
    ) -> Result<Vec<IdeNamedProject>> {
        let pool = ctx.pool();
        self.ensure_hydrated(&pool).await?;
        let has_root_auth = otto_core::auth::root_authority(user);
        let snapshot: Vec<StoredProject> = self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .clone();

        let mut items = Vec::with_capacity(snapshot.len());
        for entry in snapshot {
            let Some(proj) = project_for_caller(&entry, has_root_auth) else {
                continue;
            };
            if can_access_owned_project(&entry, &user.id, has_root_auth)
                || (entry.owner_user_id.is_empty()
                    && legacy_project_authorized(ctx, user, &proj, false).await)
            {
                items.push(proj);
            }
        }
        sort_projects(&mut items);
        Ok(items)
    }

    /// Return all folders across Named Projects owned by `user` (from the hydrated cache).
    pub fn allowed_roots_for_user(&self, user: &User) -> Vec<String> {
        let has_root_auth = otto_core::auth::root_authority(user);
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for entry in &guard.items {
            if !can_access_owned_project(entry, &user.id, has_root_auth) {
                continue;
            }
            let Some(proj) = project_for_caller(entry, has_root_auth) else {
                continue;
            };
            if !proj.primary_root.trim().is_empty() && !out.contains(&proj.primary_root) {
                out.push(proj.primary_root.clone());
            }
            for f in &proj.folders {
                if !f.trim().is_empty() && !out.contains(f) {
                    out.push(f.clone());
                }
            }
        }
        out
    }

    /// Return attached sibling folders configured for any Named Project owned by `user`
    /// whose `primary_root` matches `root` (a member folder opened on its own stays one
    /// folder), using pure in-memory string comparisons (zero `realpath` syscalls on Tokio
    /// workers).
    pub fn folders_for_root_for_user(&self, root: &str, user: &User) -> Vec<String> {
        let Some(lookup) = normalize_lookup_fast(root) else {
            return Vec::new();
        };
        let has_root_auth = otto_core::auth::root_authority(user);
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for entry in &guard.items {
            if !can_access_owned_project(entry, &user.id, has_root_auth) {
                continue;
            }
            if stored_project_primary_matches(entry, &lookup) {
                if let Some(proj) = project_for_caller(entry, has_root_auth) {
                    return proj.folders;
                }
            }
        }
        Vec::new()
    }

    /// Return candidate attached folders from unowned legacy projects (`owner_user_id == ""`)
    /// matching `root`; callers MUST authorize each returned folder via `IdeCtx`.
    pub(crate) fn legacy_unowned_folders_for_root(&self, root: &str, is_root: bool) -> Vec<String> {
        let Some(lookup) = normalize_lookup_fast(root) else {
            return Vec::new();
        };
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for entry in &guard.items {
            if !entry.owner_user_id.is_empty() {
                continue;
            }
            if stored_project_primary_matches(entry, &lookup) {
                if let Some(proj) = project_for_caller(entry, is_root) {
                    return proj.folders;
                }
            }
        }
        Vec::new()
    }

    /// Root-level lookup matching a project's `primary_root` (not its member folders).
    pub fn folders_for_root(&self, root: &str) -> Vec<String> {
        let Some(lookup) = normalize_lookup_fast(root) else {
            return Vec::new();
        };
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for entry in &guard.items {
            if stored_project_primary_matches(entry, &lookup) {
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
        self.upsert_for_user_inner(pool, user, req, false).await
    }

    pub async fn upsert_for_user_claiming_legacy(
        &self,
        pool: &DbPool,
        user: &User,
        req: IdeUpsertProjectReq,
    ) -> Result<IdeNamedProject> {
        self.upsert_for_user_inner(pool, user, req, true).await
    }

    pub async fn upsert_for_user_with_ctx<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        req: IdeUpsertProjectReq,
    ) -> Result<IdeNamedProject> {
        let pool = ctx.pool();
        self.ensure_hydrated(&pool).await?;
        let mut allow_claim = otto_core::auth::root_authority(user);
        if !allow_claim {
            if let Some(req_id) = req.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((owner, proj)) = fetch_project_owner_row(&pool, req_id).await? {
                    allow_claim =
                        owner.is_empty() && legacy_project_authorized(ctx, user, &proj, true).await;
                }
            } else {
                allow_claim = true;
            }
        }
        self.upsert_for_user_inner(&pool, user, req, allow_claim)
            .await
    }

    async fn upsert_for_user_inner(
        &self,
        pool: &DbPool,
        user: &User,
        req: IdeUpsertProjectReq,
        allow_claim_unowned: bool,
    ) -> Result<IdeNamedProject> {
        self.ensure_hydrated(pool).await?;
        let has_root_auth = otto_core::auth::root_authority(user);
        let NormalizedUpsertProject {
            name,
            primary_root: primary_str,
            folders,
            pinned,
            explicit_id,
        } = tokio::task::spawn_blocking(move || normalize_upsert_req(req, has_root_auth))
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
                let unowned_claim_ok = owner.is_empty() && allow_claim_unowned;
                if !has_root_auth && owner != &user.id && !unowned_claim_ok {
                    return Err(Error::Forbidden(
                        "cannot overwrite a Named IDE Project owned by another user".into(),
                    ));
                }
            }
            req_id
        } else {
            let existing_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM ide_named_projects
                 WHERE (owner_user_id = ?1 OR (?4 = 1 AND owner_user_id = ''))
                   AND primary_root = ?2 AND name = ?3
                 LIMIT 1",
            )
            .bind(&user.id)
            .bind(&primary_str)
            .bind(&name)
            .bind(if has_root_auth || allow_claim_unowned {
                1_i64
            } else {
                0_i64
            })
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
        let Some((owner, _)) = fetch_project_owner_row(pool, target_id).await? else {
            return Ok(false);
        };
        if !otto_core::auth::root_authority(user) && owner != user.id {
            return Err(Error::Forbidden(
                "cannot delete a Named IDE Project owned by another user".into(),
            ));
        }
        self.execute_delete(pool, target_id).await
    }

    pub async fn delete_for_user_with_ctx<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        id: &str,
    ) -> Result<bool> {
        let pool = ctx.pool();
        self.ensure_hydrated(&pool).await?;
        let target_id = id.trim();
        let Some((owner, proj)) = fetch_project_owner_row(&pool, target_id).await? else {
            return Ok(false);
        };
        if !otto_core::auth::root_authority(user) && owner != user.id {
            let legacy_ok =
                owner.is_empty() && legacy_project_authorized(ctx, user, &proj, true).await;
            if !legacy_ok {
                return Err(Error::Forbidden(
                    "cannot delete a Named IDE Project owned by another user".into(),
                ));
            }
        }
        self.execute_delete(&pool, target_id).await
    }

    async fn execute_delete(&self, pool: &DbPool, target_id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM ide_named_projects WHERE id = ?1")
            .bind(target_id)
            .execute(pool)
            .await
            .map_err(|e| Error::Internal(format!("delete ide_named_projects: {e}")))?;
        self.reload_from_db(pool).await?;
        Ok(res.rows_affected() > 0)
    }
}

async fn fetch_project_owner_row(
    pool: &DbPool,
    target_id: &str,
) -> Result<Option<(String, IdeNamedProject)>> {
    let row = sqlx::query(
        "SELECT owner_user_id, primary_root, folders_json
         FROM ide_named_projects WHERE id = ?1",
    )
    .bind(target_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Internal(format!("query ide_named_projects owner: {e}")))?;

    let Some(row) = row else {
        return Ok(None);
    };
    let owner: String = row.try_get("owner_user_id").unwrap_or_default();
    let primary_root: String = row.try_get("primary_root").unwrap_or_default();
    let folders_json: String = row.try_get("folders_json").unwrap_or_default();
    let folders: Vec<String> = serde_json::from_str(&folders_json).unwrap_or_default();
    Ok(Some((
        owner,
        IdeNamedProject {
            id: target_id.to_string(),
            name: String::new(),
            primary_root,
            folders,
            pinned: false,
            updated_at: String::new(),
        },
    )))
}

fn can_access_owned_project(entry: &StoredProject, user_id: &str, is_root: bool) -> bool {
    is_root || (!entry.owner_user_id.is_empty() && entry.owner_user_id == user_id)
}

fn project_for_caller(entry: &StoredProject, is_root: bool) -> Option<IdeNamedProject> {
    if is_root {
        Some(entry.project.clone())
    } else {
        entry.non_root_project.clone()
    }
}

fn sort_projects(items: &mut [IdeNamedProject]) {
    items.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.name.cmp(&b.name))
    });
}

#[cfg(test)]
mod primary_root_tests {
    use super::*;

    fn write_legacy(dir: &std::path::Path, primary: &str, folders: &[&str]) {
        let proj = IdeNamedProject {
            id: "ideproj_1".into(),
            name: "Payments platform".into(),
            primary_root: primary.into(),
            folders: folders.iter().map(|s| s.to_string()).collect(),
            pinned: true,
            updated_at: String::new(),
        };
        fs::write(
            dir.join("ide-projects.json"),
            serde_json::to_string(&vec![proj]).expect("json"),
        )
        .expect("write legacy projects");
    }

    /// I3-02: opening ONE member repo of a multi-repo project opens that repo only;
    /// the project's folders come with its primary root (the project itself).
    #[test]
    fn member_root_does_not_inherit_the_projects_folders() {
        let data = tempfile::tempdir().expect("data dir");
        let ws = tempfile::tempdir().expect("ws dir");
        let core = ws.path().join("core");
        let api_a = ws.path().join("team-a/api");
        let api_b = ws.path().join("team-b/api");
        for d in [&core, &api_a, &api_b] {
            fs::create_dir_all(d).expect("mkdir repo");
        }
        let s = |p: &std::path::Path| p.to_string_lossy().to_string();
        write_legacy(data.path(), &s(&core), &[&s(&core), &s(&api_a), &s(&api_b)]);
        let store = ProjectStore::new(data.path().to_path_buf());

        assert!(
            store.folders_for_root(&s(&api_a)).is_empty(),
            "a member folder opened on its own stays one folder"
        );
        assert!(store.folders_for_root(&s(&api_b)).is_empty());
        let from_primary = store.folders_for_root(&s(&core));
        assert_eq!(
            from_primary.len(),
            3,
            "the project's own primary root still brings every folder: {from_primary:?}"
        );
        // A trailing slash is the same root.
        assert_eq!(store.folders_for_root(&format!("{}/", s(&core))).len(), 3);
    }

    /// The fix changes lookups only: the saved project keeps every folder.
    #[test]
    fn saved_project_folders_are_untouched() {
        let data = tempfile::tempdir().expect("data dir");
        let ws = tempfile::tempdir().expect("ws dir");
        let core = ws.path().join("core");
        let api = ws.path().join("api");
        fs::create_dir_all(&core).expect("mkdir");
        fs::create_dir_all(&api).expect("mkdir");
        let s = |p: &std::path::Path| p.to_string_lossy().to_string();
        write_legacy(data.path(), &s(&core), &[&s(&core), &s(&api)]);
        let store = ProjectStore::new(data.path().to_path_buf());
        let _ = store.folders_for_root(&s(&api));
        let guard = store.cache.read().unwrap_or_else(|e| e.into_inner());
        assert_eq!(guard.items.len(), 1);
        assert_eq!(guard.items[0].project.folders.len(), 2);
    }
}

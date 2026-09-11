//! Workspaces + membership repository.

use chrono::Utc;
use otto_core::domain::{User, Workspace, WorkspaceRole, SCRATCH_WORKSPACE_ID};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct WorkspacesRepo {
    pool: SqlitePool,
}

/// Membership row joined with user info.
pub struct Member {
    pub user_id: Id,
    pub username: String,
    pub display_name: String,
    pub role: WorkspaceRole,
}

fn row_to_workspace(r: &sqlx::sqlite::SqliteRow) -> Result<Workspace> {
    Ok(Workspace {
        id: r.get("id"),
        name: r.get("name"),
        root_path: r.get("root_path"),
        settings: json(&r.get::<String, _>("settings_json"))?,
        archived: r.get::<i64, _>("archived") != 0,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

/// A membership-joined row (`w.*` + `m.role AS my_role`).
fn row_to_workspace_with_role(r: &sqlx::sqlite::SqliteRow) -> Result<(Workspace, WorkspaceRole)> {
    let role = WorkspaceRole::parse(&r.get::<String, _>("my_role"))
        .ok_or_else(|| Error::Internal("bad role".into()))?;
    Ok((row_to_workspace(r)?, role))
}

impl WorkspacesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, name: &str, root_path: &str, creator: &Id) -> Result<Workspace> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, ?, ?, '{}', 0, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(root_path)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create workspace"))?;
        self.set_member(&id, creator, WorkspaceRole::Admin).await?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Workspace> {
        let r = sqlx::query("SELECT * FROM workspaces WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("workspace"))?;
        row_to_workspace(&r)
    }

    /// Make sure the system-owned scratch workspace (`SCRATCH_WORKSPACE_ID`)
    /// exists and is healthy: insert it when missing, then pin `root_path` to
    /// `home` and un-archive it (heals a renamed/archived row). Idempotent;
    /// called once per daemon boot. Writes no membership rows — every user is
    /// an implicit Editor there (see [`Self::role_of`]).
    pub async fn ensure_scratch(&self, home: &str) -> Result<Workspace> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT OR IGNORE INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'Scratch', ?, '{}', 0, ?)",
        )
        .bind(SCRATCH_WORKSPACE_ID)
        .bind(home)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("ensure scratch workspace"))?;
        sqlx::query("UPDATE workspaces SET root_path = ?, archived = 0 WHERE id = ?")
            .bind(home)
            .bind(SCRATCH_WORKSPACE_ID)
            .execute(&self.pool)
            .await
            .map_err(dberr("ensure scratch workspace"))?;
        self.get(&SCRATCH_WORKSPACE_ID.to_string()).await
    }

    /// All workspaces (root view), unarchived first. COMPLETE — includes the
    /// scratch row, so session restore / CLI-update restarts / mission-control
    /// backfill see scratch sessions too. User-facing lists (pickers, the
    /// sidebar, host selection) use [`Self::list_user_all`] instead.
    pub async fn list_all(&self) -> Result<Vec<Workspace>> {
        let rows = sqlx::query("SELECT * FROM workspaces ORDER BY archived, created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("workspaces"))?;
        rows.iter().map(row_to_workspace).collect()
    }

    /// [`Self::list_all`] minus the system-owned scratch workspace — the
    /// user-facing variant (`GET /workspaces` for root, vault/insights/
    /// self-improvement host selection).
    pub async fn list_user_all(&self) -> Result<Vec<Workspace>> {
        let rows =
            sqlx::query("SELECT * FROM workspaces WHERE id <> ? ORDER BY archived, created_at")
                .bind(SCRATCH_WORKSPACE_ID)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("workspaces"))?;
        rows.iter().map(row_to_workspace).collect()
    }

    /// Workspaces `user` is a member of, with the user's role. COMPLETE like
    /// [`Self::list_all`] (the scratch row has no members, so it only shows up
    /// here if someone wrote a membership row by hand); the user-facing
    /// variant is [`Self::list_user_for_user`].
    pub async fn list_for_user(&self, user_id: &Id) -> Result<Vec<(Workspace, WorkspaceRole)>> {
        let rows = sqlx::query(
            "SELECT w.*, m.role AS my_role FROM workspaces w
             JOIN workspace_members m ON m.workspace_id = w.id
             WHERE m.user_id = ? ORDER BY w.archived, w.created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("workspaces"))?;
        rows.iter().map(row_to_workspace_with_role).collect()
    }

    /// [`Self::list_for_user`] minus the system-owned scratch workspace.
    pub async fn list_user_for_user(
        &self,
        user_id: &Id,
    ) -> Result<Vec<(Workspace, WorkspaceRole)>> {
        let rows = sqlx::query(
            "SELECT w.*, m.role AS my_role FROM workspaces w
             JOIN workspace_members m ON m.workspace_id = w.id
             WHERE m.user_id = ? AND w.id <> ? ORDER BY w.archived, w.created_at",
        )
        .bind(user_id)
        .bind(SCRATCH_WORKSPACE_ID)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("workspaces"))?;
        rows.iter().map(row_to_workspace_with_role).collect()
    }

    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        root_path: Option<&str>,
        settings: Option<&serde_json::Value>,
        archived: Option<bool>,
    ) -> Result<Workspace> {
        if let Some(v) = name {
            sqlx::query("UPDATE workspaces SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workspace"))?;
        }
        if let Some(v) = root_path {
            sqlx::query("UPDATE workspaces SET root_path = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workspace"))?;
        }
        if let Some(v) = settings {
            sqlx::query("UPDATE workspaces SET settings_json = ? WHERE id = ?")
                .bind(v.to_string())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workspace"))?;
        }
        if let Some(v) = archived {
            sqlx::query("UPDATE workspaces SET archived = ? WHERE id = ?")
                .bind(v as i64)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update workspace"))?;
        }
        self.get(id).await
    }

    pub async fn set_member(&self, ws: &Id, user: &Id, role: WorkspaceRole) -> Result<()> {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)
             ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(ws)
        .bind(user)
        .bind(role.as_str())
        .execute(&self.pool)
        .await
        .map_err(dberr("set member"))?;
        Ok(())
    }

    pub async fn remove_member(&self, ws: &Id, user: &Id) -> Result<()> {
        sqlx::query("DELETE FROM workspace_members WHERE workspace_id = ? AND user_id = ?")
            .bind(ws)
            .bind(user)
            .execute(&self.pool)
            .await
            .map_err(dberr("remove member"))?;
        Ok(())
    }

    pub async fn members(&self, ws: &Id) -> Result<Vec<Member>> {
        let rows = sqlx::query(
            "SELECT m.user_id, m.role, u.username, u.display_name
             FROM workspace_members m JOIN users u ON u.id = m.user_id
             WHERE m.workspace_id = ? ORDER BY u.username",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("members"))?;
        rows.iter()
            .map(|r| {
                Ok(Member {
                    user_id: r.get("user_id"),
                    username: r.get("username"),
                    display_name: r.get("display_name"),
                    role: WorkspaceRole::parse(&r.get::<String, _>("role"))
                        .ok_or_else(|| Error::Internal("bad role".into()))?,
                })
            })
            .collect()
    }

    /// Role of `user` in `ws`; root users are admin everywhere, and every
    /// authenticated user is an Editor of the scratch workspace (no membership
    /// rows exist for it). Editor — not Admin — so the owner-or-admin gates on
    /// sessions keep a user's scratch sessions private to them.
    pub async fn role_of(&self, user: &User, ws: &Id) -> Result<Option<WorkspaceRole>> {
        if user.is_root {
            return Ok(Some(WorkspaceRole::Admin));
        }
        if ws == SCRATCH_WORKSPACE_ID {
            return Ok(Some(WorkspaceRole::Editor));
        }
        let row = sqlx::query(
            "SELECT role FROM workspace_members WHERE workspace_id = ? AND user_id = ?",
        )
        .bind(ws)
        .bind(&user.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("role"))?;
        Ok(row.and_then(|r| WorkspaceRole::parse(&r.get::<String, _>("role"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    async fn seed_user(pool: &SqlitePool, is_root: bool) -> User {
        let id = new_id();
        let now = Utc::now();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id).bind(&id).bind("x").bind("U").bind(is_root as i64).bind(fmt(now))
            .execute(pool).await.unwrap();
        User {
            id,
            username: "u".into(),
            display_name: "U".into(),
            is_root,
            disabled: false,
            created_at: now,
        }
    }

    async fn count_scratch_rows(pool: &SqlitePool) -> i64 {
        sqlx::query("SELECT COUNT(*) AS n FROM workspaces WHERE id = ?")
            .bind(SCRATCH_WORKSPACE_ID)
            .fetch_one(pool)
            .await
            .unwrap()
            .get("n")
    }

    #[tokio::test]
    async fn ensure_scratch_is_idempotent_and_heals() {
        let pool = mem_pool().await;
        let repo = WorkspacesRepo::new(pool.clone());
        let scratch_id = SCRATCH_WORKSPACE_ID.to_string();

        let ws = repo.ensure_scratch("/Users/me").await.unwrap();
        assert_eq!(ws.id, SCRATCH_WORKSPACE_ID);
        assert!(ws.is_system());
        assert_eq!(ws.name, "Scratch");
        assert_eq!(ws.root_path, "/Users/me");
        assert!(!ws.archived);
        assert!(repo.members(&scratch_id).await.unwrap().is_empty());

        // Second boot: still exactly one row.
        repo.ensure_scratch("/Users/me").await.unwrap();
        assert_eq!(count_scratch_rows(&pool).await, 1);

        // A tampered row (archived + moved) is healed on the next boot.
        repo.update(&scratch_id, None, Some("/elsewhere"), None, Some(true))
            .await
            .unwrap();
        let healed = repo.ensure_scratch("/Users/me").await.unwrap();
        assert!(!healed.archived);
        assert_eq!(healed.root_path, "/Users/me");
        assert_eq!(count_scratch_rows(&pool).await, 1);
    }

    #[tokio::test]
    async fn list_user_variants_omit_scratch_but_list_all_includes_it() {
        let pool = mem_pool().await;
        let repo = WorkspacesRepo::new(pool.clone());
        let user = seed_user(&pool, false).await;
        repo.ensure_scratch("/Users/me").await.unwrap();
        let mine = repo.create("Mine", "/tmp/mine", &user.id).await.unwrap();

        let all = repo.list_all().await.unwrap();
        assert!(all.iter().any(|w| w.is_system()));
        assert!(all.iter().any(|w| w.id == mine.id));

        let user_all = repo.list_user_all().await.unwrap();
        assert!(user_all.iter().all(|w| !w.is_system()));
        assert_eq!(user_all.len(), 1);
        assert_eq!(user_all[0].id, mine.id);

        // A hand-written membership row must not leak scratch into the
        // user-facing join either.
        repo.set_member(
            &SCRATCH_WORKSPACE_ID.to_string(),
            &user.id,
            WorkspaceRole::Viewer,
        )
        .await
        .unwrap();
        let joined = repo.list_for_user(&user.id).await.unwrap();
        assert!(joined.iter().any(|(w, _)| w.is_system()));
        let user_joined = repo.list_user_for_user(&user.id).await.unwrap();
        assert_eq!(user_joined.len(), 1);
        assert_eq!(user_joined[0].0.id, mine.id);
        assert_eq!(user_joined[0].1, WorkspaceRole::Admin);
    }

    #[tokio::test]
    async fn role_of_gives_editor_on_scratch_to_non_member_and_admin_to_root() {
        let pool = mem_pool().await;
        let repo = WorkspacesRepo::new(pool.clone());
        repo.ensure_scratch("/Users/me").await.unwrap();
        let scratch_id = SCRATCH_WORKSPACE_ID.to_string();
        let member = seed_user(&pool, false).await;
        let root = seed_user(&pool, true).await;

        assert_eq!(
            repo.role_of(&member, &scratch_id).await.unwrap(),
            Some(WorkspaceRole::Editor)
        );
        assert_eq!(
            repo.role_of(&root, &scratch_id).await.unwrap(),
            Some(WorkspaceRole::Admin)
        );
        // Membership semantics elsewhere are untouched: a non-member of an
        // ordinary workspace still has no role there.
        let other = repo.create("Other", "/tmp/other", &root.id).await.unwrap();
        assert_eq!(repo.role_of(&member, &other.id).await.unwrap(), None);
    }
}

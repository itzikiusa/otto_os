//! Workspaces + membership repository.

use chrono::Utc;
use otto_core::domain::{User, Workspace, WorkspaceRole};
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

    /// All workspaces (root view), unarchived first.
    pub async fn list_all(&self) -> Result<Vec<Workspace>> {
        let rows = sqlx::query("SELECT * FROM workspaces ORDER BY archived, created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("workspaces"))?;
        rows.iter().map(row_to_workspace).collect()
    }

    /// Workspaces `user` is a member of, with the user's role.
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
        rows.iter()
            .map(|r| {
                let role = WorkspaceRole::parse(&r.get::<String, _>("my_role"))
                    .ok_or_else(|| Error::Internal("bad role".into()))?;
                Ok((row_to_workspace(r)?, role))
            })
            .collect()
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

    /// Role of `user` in `ws`; root users are admin everywhere.
    pub async fn role_of(&self, user: &User, ws: &Id) -> Result<Option<WorkspaceRole>> {
        if user.is_root {
            return Ok(Some(WorkspaceRole::Admin));
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

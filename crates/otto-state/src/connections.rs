//! Connections repository.

use chrono::Utc;
use otto_core::domain::{Connection, ConnectionKind, Environment};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct ConnectionsRepo {
    pool: SqlitePool,
}

pub struct NewConnection {
    pub workspace_id: Option<Id>,
    pub name: String,
    pub kind: ConnectionKind,
    pub params: serde_json::Value,
    pub secret_ref: Option<String>,
    pub first_command: Option<String>,
    pub section_id: Option<Id>,
    /// Deployment environment (dev/staging/prod). Defaults to `Dev`.
    pub environment: Environment,
    /// Lock the profile against writes regardless of environment.
    pub read_only: bool,
    pub created_by: Id,
}

fn row_to_connection(r: &sqlx::sqlite::SqliteRow) -> Result<Connection> {
    Ok(Connection {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        kind: ConnectionKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad connection kind".into()))?,
        params: json(&r.get::<String, _>("params_json"))?,
        secret_ref: r.get("secret_ref"),
        first_command: r.get("first_command"),
        section_id: r.get("section_id"),
        environment: Environment::parse(&r.get::<String, _>("environment"))
            .ok_or_else(|| Error::Internal("bad connection environment".into()))?,
        read_only: r.get::<i64, _>("read_only") != 0,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl ConnectionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewConnection) -> Result<Connection> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO connections (id, workspace_id, name, kind, params_json, secret_ref,
                                      first_command, section_id, environment, read_only,
                                      created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&c.workspace_id)
        .bind(&c.name)
        .bind(c.kind.as_str())
        .bind(c.params.to_string())
        .bind(&c.secret_ref)
        .bind(&c.first_command)
        .bind(&c.section_id)
        .bind(c.environment.as_str())
        .bind(i64::from(c.read_only))
        .bind(&c.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create connection"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Connection> {
        let r = sqlx::query("SELECT * FROM connections WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("connection"))?;
        row_to_connection(&r)
    }

    /// Connections visible to a workspace: its own plus global (NULL workspace).
    pub async fn list_visible(&self, ws: &Id) -> Result<Vec<Connection>> {
        let rows = sqlx::query(
            "SELECT * FROM connections WHERE workspace_id = ? OR workspace_id IS NULL ORDER BY name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("connections"))?;
        rows.iter().map(row_to_connection).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: &Id,
        name: Option<&str>,
        params: Option<&serde_json::Value>,
        secret_ref: Option<Option<&str>>,
        first_command: Option<Option<&str>>,
        section_id: Option<Option<&str>>,
        environment: Option<Environment>,
        read_only: Option<bool>,
    ) -> Result<Connection> {
        if let Some(v) = name {
            sqlx::query("UPDATE connections SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = params {
            sqlx::query("UPDATE connections SET params_json = ? WHERE id = ?")
                .bind(v.to_string())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = secret_ref {
            sqlx::query("UPDATE connections SET secret_ref = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = first_command {
            sqlx::query("UPDATE connections SET first_command = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = section_id {
            sqlx::query("UPDATE connections SET section_id = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = environment {
            sqlx::query("UPDATE connections SET environment = ? WHERE id = ?")
                .bind(v.as_str())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        if let Some(v) = read_only {
            sqlx::query("UPDATE connections SET read_only = ? WHERE id = ?")
                .bind(i64::from(v))
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update connection"))?;
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM connections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete connection"))?;
        Ok(())
    }

    /// Like `list_visible` but filtered to connections owned by `caller_id`.
    /// Used when `connections.owner_private = true` for non-root users.
    pub async fn list_visible_for(&self, ws: &Id, caller_id: &Id) -> Result<Vec<Connection>> {
        let rows = sqlx::query(
            "SELECT * FROM connections \
             WHERE (workspace_id = ? OR workspace_id IS NULL) AND created_by = ? ORDER BY name",
        )
        .bind(ws)
        .bind(caller_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("connections for user"))?;
        rows.iter().map(row_to_connection).collect()
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

    async fn seed_user(pool: &SqlitePool) -> Id {
        seed_named_user(pool, "u").await
    }

    async fn seed_named_user(pool: &SqlitePool, name: &str) -> Id {
        let user = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&user)
        .bind(name)
        .bind("x")
        .bind(name)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        user
    }

    async fn seed_ws(pool: &SqlitePool) -> Id {
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&ws)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        ws
    }

    fn new_conn(user: &Id, environment: Environment, read_only: bool) -> NewConnection {
        new_conn_ws(user, None, environment, read_only)
    }

    fn new_conn_ws(
        user: &Id,
        ws: Option<Id>,
        environment: Environment,
        read_only: bool,
    ) -> NewConnection {
        NewConnection {
            workspace_id: ws,
            name: "c".into(),
            kind: ConnectionKind::Mysql,
            params: serde_json::json!({"host": "h"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment,
            read_only,
            created_by: user.clone(),
        }
    }

    #[tokio::test]
    async fn environment_and_read_only_round_trip() {
        let pool = mem_pool().await;
        let user = seed_user(&pool).await;
        let repo = ConnectionsRepo::new(pool.clone());

        // Defaults: dev / not read-only.
        let dev = repo
            .create(new_conn(&user, Environment::Dev, false))
            .await
            .unwrap();
        assert_eq!(dev.environment, Environment::Dev);
        assert!(!dev.read_only);
        assert!(!dev.is_write_guarded());

        // Created as prod + read-only.
        let prod = repo
            .create(new_conn(&user, Environment::Prod, true))
            .await
            .unwrap();
        assert_eq!(prod.environment, Environment::Prod);
        assert!(prod.read_only);
        assert!(prod.is_write_guarded());

        // Re-fetch confirms persistence.
        let fetched = repo.get(&prod.id).await.unwrap();
        assert_eq!(fetched.environment, Environment::Prod);
        assert!(fetched.read_only);

        // Update flips the fields independently.
        let updated = repo
            .update(
                &dev.id,
                None,
                None,
                None,
                None,
                None,
                Some(Environment::Staging),
                Some(true),
            )
            .await
            .unwrap();
        assert_eq!(updated.environment, Environment::Staging);
        assert!(updated.read_only);
        assert!(updated.is_write_guarded()); // read-only alone guards it
    }

    // --- list_visible_for (owner-private predicate) --------------------------

    #[tokio::test]
    async fn list_visible_for_excludes_others_connections() {
        let pool = mem_pool().await;
        let user_a = seed_named_user(&pool, "alice").await;
        let user_b = seed_named_user(&pool, "bob").await;
        let ws = seed_ws(&pool).await;
        let repo = ConnectionsRepo::new(pool.clone());

        // A creates a workspace-scoped connection.
        repo.create(new_conn_ws(&user_a, Some(ws.clone()), Environment::Dev, false))
            .await
            .unwrap();

        // B should see no connections when filtered to their own.
        let visible = repo.list_visible_for(&ws, &user_b).await.unwrap();
        assert!(
            visible.is_empty(),
            "user B should not see user A's connection"
        );
    }

    #[tokio::test]
    async fn list_visible_for_shows_own_connections() {
        let pool = mem_pool().await;
        let user_a = seed_named_user(&pool, "alice2").await;
        let user_b = seed_named_user(&pool, "bob2").await;
        let ws = seed_ws(&pool).await;
        let repo = ConnectionsRepo::new(pool.clone());

        repo.create(new_conn_ws(&user_a, Some(ws.clone()), Environment::Dev, false))
            .await
            .unwrap();
        repo.create(new_conn_ws(&user_b, Some(ws.clone()), Environment::Dev, false))
            .await
            .unwrap();

        // A only sees their own.
        let a_visible = repo.list_visible_for(&ws, &user_a).await.unwrap();
        assert_eq!(a_visible.len(), 1);
        assert_eq!(a_visible[0].created_by, user_a);

        // B only sees their own.
        let b_visible = repo.list_visible_for(&ws, &user_b).await.unwrap();
        assert_eq!(b_visible.len(), 1);
        assert_eq!(b_visible[0].created_by, user_b);
    }

    #[tokio::test]
    async fn list_visible_shows_all_setting_off() {
        // list_visible (unfiltered) still returns everything — default-off regression guard.
        let pool = mem_pool().await;
        let user_a = seed_named_user(&pool, "alice3").await;
        let user_b = seed_named_user(&pool, "bob3").await;
        let ws = seed_ws(&pool).await;
        let repo = ConnectionsRepo::new(pool.clone());

        repo.create(new_conn_ws(&user_a, Some(ws.clone()), Environment::Dev, false))
            .await
            .unwrap();
        repo.create(new_conn_ws(&user_b, Some(ws.clone()), Environment::Dev, false))
            .await
            .unwrap();

        let all = repo.list_visible(&ws).await.unwrap();
        assert_eq!(all.len(), 2, "list_visible (default path) must return all connections");
    }
}

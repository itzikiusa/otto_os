//! Users repository.

use chrono::Utc;
use otto_core::domain::User;
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct UsersRepo {
    pool: SqlitePool,
}

/// A user row including the password hash (kept out of `domain::User`).
pub struct UserRecord {
    pub user: User,
    pub password_hash: String,
}

fn row_to_user(r: &sqlx::sqlite::SqliteRow) -> Result<User> {
    Ok(User {
        id: r.get("id"),
        username: r.get("username"),
        display_name: r.get("display_name"),
        is_root: r.get::<i64, _>("is_root") != 0,
        disabled: r.get::<i64, _>("disabled") != 0,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl UsersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn count(&self) -> Result<i64> {
        let r = sqlx::query("SELECT COUNT(*) AS n FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("count users"))?;
        Ok(r.get("n"))
    }

    pub async fn create(
        &self,
        username: &str,
        password_hash: &str,
        display_name: &str,
        is_root: bool,
    ) -> Result<User> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(password_hash)
        .bind(display_name)
        .bind(is_root as i64)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.message().contains("UNIQUE") => {
                Error::Conflict(format!("username '{username}' already exists"))
            }
            other => Error::Internal(format!("create user: {other}")),
        })?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<User> {
        let r = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("user"))?;
        row_to_user(&r)
    }

    pub async fn get_by_username(&self, username: &str) -> Result<UserRecord> {
        let r = sqlx::query("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("user"))?;
        Ok(UserRecord {
            password_hash: r.get("password_hash"),
            user: row_to_user(&r)?,
        })
    }

    pub async fn list(&self) -> Result<Vec<User>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("users"))?;
        rows.iter().map(row_to_user).collect()
    }

    pub async fn update(
        &self,
        id: &Id,
        display_name: Option<&str>,
        password_hash: Option<&str>,
        disabled: Option<bool>,
    ) -> Result<User> {
        if let Some(d) = display_name {
            sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
                .bind(d)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update user"))?;
        }
        if let Some(p) = password_hash {
            sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
                .bind(p)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update user"))?;
        }
        if let Some(dis) = disabled {
            sqlx::query("UPDATE users SET disabled = ? WHERE id = ?")
                .bind(dis as i64)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update user"))?;
        }
        self.get(id).await
    }
}

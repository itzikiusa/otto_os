//! AWS accounts repository (`aws_accounts`, migration 0113).
//!
//! An "account" is how a user reaches AWS from Otto: either a named profile in
//! `~/.aws/config` (the `aws` CLI does SSO/assume-role/MFA itself) or static
//! access keys whose secret half lives in the Keychain under `secret_ref`.
//! Accounts are a global library (no workspace axis) — same posture as
//! `0023_global_connections`. Cached JSON blobs (`identity_json`,
//! `permissions_json`) are opaque here; `otto-aws` owns their shape.

use chrono::{DateTime, Utc};
use otto_core::domain::Environment;
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

/// One `aws_accounts` row. `params` carries the non-secret extras
/// (`access_key_id`, `role_arn`, `color`); secrets never enter this struct.
#[derive(Debug, Clone)]
pub struct AwsAccountRow {
    pub id: Id,
    pub name: String,
    /// `profile` | `access_keys`.
    pub auth_mode: String,
    pub profile: Option<String>,
    pub region: String,
    pub params: serde_json::Value,
    pub secret_ref: Option<String>,
    pub identity: Option<serde_json::Value>,
    pub permissions: Option<serde_json::Value>,
    pub permissions_checked_at: Option<DateTime<Utc>>,
    pub environment: Environment,
    pub created_by: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub struct NewAwsAccount {
    pub name: String,
    pub auth_mode: String,
    pub profile: Option<String>,
    pub region: String,
    pub params: serde_json::Value,
    pub secret_ref: Option<String>,
    pub environment: Environment,
    pub created_by: Option<Id>,
}

/// Partial update; `None` = keep. `secret_ref` uses the double-Option idiom
/// (`Some(None)` clears).
#[derive(Default)]
pub struct AwsAccountPatch {
    pub name: Option<String>,
    pub auth_mode: Option<String>,
    pub profile: Option<Option<String>>,
    pub region: Option<String>,
    pub params: Option<serde_json::Value>,
    pub secret_ref: Option<Option<String>>,
    pub environment: Option<Environment>,
}

fn opt_ts(r: &sqlx::sqlite::SqliteRow, col: &str) -> Result<Option<DateTime<Utc>>> {
    r.get::<Option<String>, _>(col)
        .map(|s| ts(&s))
        .transpose()
}

fn opt_json(r: &sqlx::sqlite::SqliteRow, col: &str) -> Result<Option<serde_json::Value>> {
    r.get::<Option<String>, _>(col)
        .map(|s| json(&s))
        .transpose()
}

fn row_to_account(r: &sqlx::sqlite::SqliteRow) -> Result<AwsAccountRow> {
    Ok(AwsAccountRow {
        id: r.get("id"),
        name: r.get("name"),
        auth_mode: r.get("auth_mode"),
        profile: r.get("profile"),
        region: r.get("region"),
        params: json(&r.get::<String, _>("params_json"))?,
        secret_ref: r.get("secret_ref"),
        identity: opt_json(r, "identity_json")?,
        permissions: opt_json(r, "permissions_json")?,
        permissions_checked_at: opt_ts(r, "permissions_checked_at")?,
        environment: Environment::parse(&r.get::<String, _>("environment"))
            .ok_or_else(|| Error::Internal("bad aws account environment".into()))?,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        last_used_at: opt_ts(r, "last_used_at")?,
    })
}

#[derive(Clone)]
pub struct AwsAccountsRepo {
    pool: SqlitePool,
}

impl AwsAccountsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, a: NewAwsAccount) -> Result<AwsAccountRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO aws_accounts (id, name, auth_mode, profile, region, params_json,
                                       secret_ref, environment, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.name)
        .bind(&a.auth_mode)
        .bind(&a.profile)
        .bind(&a.region)
        .bind(a.params.to_string())
        .bind(&a.secret_ref)
        .bind(a.environment.as_str())
        .bind(&a.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create aws account"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<AwsAccountRow> {
        let r = sqlx::query("SELECT * FROM aws_accounts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("aws account"))?;
        row_to_account(&r)
    }

    /// Every account, alphabetical. One unparseable row is skipped with a
    /// warning rather than blanking the list.
    pub async fn list(&self) -> Result<Vec<AwsAccountRow>> {
        let rows = sqlx::query("SELECT * FROM aws_accounts ORDER BY name, created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("aws accounts"))?;
        Ok(rows
            .iter()
            .filter_map(|r| match row_to_account(r) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!(id = %r.get::<String, _>("id"), "skipping unparseable aws account row: {e}");
                    None
                }
            })
            .collect())
    }

    pub async fn update(&self, id: &Id, p: AwsAccountPatch) -> Result<AwsAccountRow> {
        // Existence check first so a no-op patch on a missing id is still a 404.
        self.get(id).await?;
        let now = fmt(Utc::now());
        if let Some(v) = p.name {
            self.set_col(id, "name", v).await?;
        }
        if let Some(v) = p.auth_mode {
            self.set_col(id, "auth_mode", v).await?;
        }
        if let Some(v) = p.profile {
            sqlx::query("UPDATE aws_accounts SET profile = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update aws account"))?;
        }
        if let Some(v) = p.region {
            self.set_col(id, "region", v).await?;
        }
        if let Some(v) = p.params {
            self.set_col(id, "params_json", v.to_string()).await?;
        }
        if let Some(v) = p.secret_ref {
            sqlx::query("UPDATE aws_accounts SET secret_ref = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update aws account"))?;
        }
        if let Some(v) = p.environment {
            self.set_col(id, "environment", v.as_str().to_string()).await?;
        }
        sqlx::query("UPDATE aws_accounts SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update aws account"))?;
        self.get(id).await
    }

    /// Column-name whitelist lives in the callers above; never pass user input
    /// as `col`.
    async fn set_col(&self, id: &Id, col: &str, v: String) -> Result<()> {
        sqlx::query(&format!("UPDATE aws_accounts SET {col} = ? WHERE id = ?"))
            .bind(v)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update aws account"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        let res = sqlx::query("DELETE FROM aws_accounts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete aws account"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound("aws account".into()));
        }
        Ok(())
    }

    /// Cache the identity from a successful `sts get-caller-identity`.
    pub async fn set_identity(&self, id: &Id, identity: &serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE aws_accounts SET identity_json = ? WHERE id = ?")
            .bind(identity.to_string())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update aws account identity"))?;
        Ok(())
    }

    /// Cache a per-service permission probe result (stamped now).
    pub async fn set_permissions(&self, id: &Id, perms: &serde_json::Value) -> Result<()> {
        sqlx::query(
            "UPDATE aws_accounts SET permissions_json = ?, permissions_checked_at = ? WHERE id = ?",
        )
        .bind(perms.to_string())
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update aws account permissions"))?;
        Ok(())
    }

    /// Best-effort "used right now" stamp (errors ignored).
    pub async fn touch_used(&self, id: &Id) {
        let _ = sqlx::query("UPDATE aws_accounts SET last_used_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn crud_roundtrip() {
        let repo = AwsAccountsRepo::new(pool().await);
        let a = repo
            .create(NewAwsAccount {
                name: "prod".into(),
                auth_mode: "profile".into(),
                profile: Some("prod-sso".into()),
                region: "eu-west-1".into(),
                params: serde_json::json!({"color": "#f00"}),
                secret_ref: None,
                environment: Environment::Prod,
                created_by: None,
            })
            .await
            .unwrap();
        assert_eq!(a.auth_mode, "profile");
        assert_eq!(a.profile.as_deref(), Some("prod-sso"));
        assert_eq!(a.environment, Environment::Prod);
        assert!(a.identity.is_none());

        repo.set_identity(&a.id, &serde_json::json!({"account": "123456789012"}))
            .await
            .unwrap();
        repo.set_permissions(&a.id, &serde_json::json!({"s3": "allowed"}))
            .await
            .unwrap();
        let got = repo.get(&a.id).await.unwrap();
        assert_eq!(got.identity.unwrap()["account"], "123456789012");
        assert!(got.permissions_checked_at.is_some());

        let upd = repo
            .update(
                &a.id,
                AwsAccountPatch {
                    name: Some("prod2".into()),
                    region: Some("us-west-2".into()),
                    secret_ref: Some(Some("aws-x".into())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(upd.name, "prod2");
        assert_eq!(upd.region, "us-west-2");
        assert_eq!(upd.secret_ref.as_deref(), Some("aws-x"));
        assert!(upd.updated_at >= a.updated_at);

        assert_eq!(repo.list().await.unwrap().len(), 1);
        repo.delete(&a.id).await.unwrap();
        assert!(matches!(repo.get(&a.id).await, Err(Error::NotFound(_))));
        assert!(matches!(repo.delete(&a.id).await, Err(Error::NotFound(_))));
    }
}

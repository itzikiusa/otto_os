//! Owner-scoped subscription profile metadata; no secrets or arbitrary paths.
use crate::convert::{dberr, ts};
use chrono::Utc;
use otto_core::provider_accounts::ProviderAccount;
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct ProviderAccountsRepo {
    pool: SqlitePool,
}

impl ProviderAccountsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, owner: &Id) -> Result<Vec<ProviderAccount>> {
        let rows = sqlx::query("SELECT id, provider, label, created_at FROM provider_accounts WHERE user_id = ? ORDER BY provider, label")
            .bind(owner).fetch_all(&self.pool).await.map_err(dberr("list provider accounts"))?;
        rows.iter().map(account_row).collect()
    }

    pub async fn get(&self, owner: &Id, id: &Id) -> Result<ProviderAccount> {
        let row = sqlx::query("SELECT id, provider, label, created_at FROM provider_accounts WHERE user_id = ? AND id = ?")
            .bind(owner).bind(id).fetch_optional(&self.pool).await.map_err(dberr("get provider account"))?
            .ok_or_else(|| Error::NotFound("provider account is missing or belongs to another user; choose an account you own".into()))?;
        account_row(&row)
    }

    pub async fn create(&self, owner: &Id, provider: &str, label: &str) -> Result<ProviderAccount> {
        let label = label.trim();
        if !matches!(provider, "claude" | "codex") || label.is_empty() || label.chars().count() > 80
        {
            return Err(Error::Invalid(
                "choose Claude or Codex and an account label of 1–80 characters".into(),
            ));
        }
        let account = ProviderAccount {
            id: new_id(),
            provider: provider.into(),
            label: label.into(),
            created_at: Utc::now(),
        };
        sqlx::query("INSERT INTO provider_accounts (id, user_id, provider, label, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&account.id).bind(owner).bind(provider).bind(label).bind(account.created_at.to_rfc3339())
            .execute(&self.pool).await.map_err(|e| {
                if e.as_database_error().is_some_and(|e| e.is_unique_violation()) {
                    Error::Conflict("an account with this label already exists for that provider".into())
                } else { dberr("create provider account")(e) }
            })?;
        Ok(account)
    }
}

fn account_row(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderAccount> {
    Ok(ProviderAccount {
        id: row.get("id"),
        provider: row.get("provider"),
        label: row.get("label"),
        created_at: ts(&row.get::<String, _>("created_at"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn subscription_profiles_are_owner_scoped_and_do_not_store_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::open(&dir.path().join("state.db")).await.unwrap();
        let now = Utc::now().to_rfc3339();
        for owner in ["a", "b"] {
            sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, 'x', 'Test', 0, ?)")
                .bind(owner).bind(owner).bind(&now).execute(&pool).await.unwrap();
        }
        let repo = ProviderAccountsRepo::new(pool);
        let account = repo.create(&"a".into(), "codex", "Work").await.unwrap();
        assert_eq!(repo.list(&"a".into()).await.unwrap().len(), 1);
        assert!(repo.list(&"b".into()).await.unwrap().is_empty());
        assert!(repo.get(&"b".into(), &account.id).await.is_err());
        assert!(repo.create(&"a".into(), "codex", "Work").await.is_err());
        assert!(repo.create(&"a".into(), "shell", "Invalid").await.is_err());
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 4);
        assert!(json.get("token").is_none());
    }
}

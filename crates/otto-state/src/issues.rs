//! Issue-tracking accounts repository. Mirrors git.rs but for issue accounts.

use chrono::{DateTime, Utc};
use otto_core::domain::{IssueAccount, IssueProviderKind};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct IssuesRepo {
    pool: SqlitePool,
}

pub struct NewIssueAccount {
    pub user_id: Id,
    pub provider: IssueProviderKind,
    pub label: String,
    /// Stored in the `username` column.
    pub email: String,
    pub token_ref: String,
    /// Stored in the `api_base_url` column.
    pub base_url: String,
    /// When the token expires, if known (user-entered for Jira).
    pub token_expires_at: Option<DateTime<Utc>>,
}

fn row_to_account(r: &sqlx::sqlite::SqliteRow) -> Result<IssueAccount> {
    let api_base_url: Option<String> = r.get("api_base_url");
    Ok(IssueAccount {
        id: r.get("id"),
        user_id: r.get("user_id"),
        provider: IssueProviderKind::parse(&r.get::<String, _>("provider"))
            .ok_or_else(|| Error::Internal("bad issue provider".into()))?,
        label: r.get("label"),
        email: r.get("username"),
        token_ref: r.get("token_ref"),
        base_url: api_base_url.unwrap_or_default(),
        token_expires_at: r
            .get::<Option<String>, _>("token_expires_at")
            .map(|s| ts(&s))
            .transpose()?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl IssuesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_account(&self, a: NewIssueAccount) -> Result<IssueAccount> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO issue_accounts (id, user_id, provider, label, username, token_ref, api_base_url, token_expires_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.user_id)
        .bind(a.provider.as_str())
        .bind(&a.label)
        .bind(&a.email)
        .bind(&a.token_ref)
        .bind(&a.base_url)
        .bind(a.token_expires_at.map(fmt))
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create issue account"))?;
        self.get_account(&id).await
    }

    pub async fn get_account(&self, id: &Id) -> Result<IssueAccount> {
        let r = sqlx::query("SELECT * FROM issue_accounts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("issue account"))?;
        row_to_account(&r)
    }

    pub async fn list_accounts(&self, user_id: &Id) -> Result<Vec<IssueAccount>> {
        let rows =
            sqlx::query("SELECT * FROM issue_accounts WHERE user_id = ? ORDER BY created_at")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("issue accounts"))?;
        rows.iter().map(row_to_account).collect()
    }

    /// Every issue account across all users. Used by the credential monitor.
    pub async fn list_all_accounts(&self) -> Result<Vec<IssueAccount>> {
        let rows = sqlx::query("SELECT * FROM issue_accounts ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("issue accounts"))?;
        rows.iter().map(row_to_account).collect()
    }

    pub async fn delete_account(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM issue_accounts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete issue account"))?;
        Ok(())
    }

    /// Update mutable fields of an issue account.
    ///
    /// The caller supplies the *final* (merged) values for all columns.
    /// `email` maps to the `username` column; `base_url` maps to `api_base_url`.
    /// `token_expires_at` is the final value (None = NULL); callers wanting
    /// "absent keeps current" merge against the existing account first.
    pub async fn update_account(
        &self,
        id: &Id,
        label: &str,
        email: &str,
        token_ref: &str,
        base_url: &str,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<IssueAccount> {
        sqlx::query(
            "UPDATE issue_accounts SET label = ?, username = ?, token_ref = ?, api_base_url = ?, token_expires_at = ? WHERE id = ?",
        )
        .bind(label)
        .bind(email)
        .bind(token_ref)
        .bind(base_url)
        .bind(token_expires_at.map(fmt))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update issue account"))?;
        self.get_account(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, SubsecRound};

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
        let user = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&user)
        .bind("u")
        .bind("x")
        .bind("U")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        user
    }

    #[tokio::test]
    async fn issue_token_expiry_round_trips() {
        let pool = mem_pool().await;
        let repo = IssuesRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let exp = Utc::now().round_subsecs(0) + Duration::days(2);

        let a = repo
            .create_account(NewIssueAccount {
                user_id: user,
                provider: IssueProviderKind::Jira,
                label: "jira".into(),
                email: "me@example.com".into(),
                token_ref: "ref".into(),
                base_url: "https://x.atlassian.net".into(),
                token_expires_at: Some(exp),
            })
            .await
            .unwrap();
        assert_eq!(a.token_expires_at, Some(exp));

        // update with None clears it
        let updated = repo
            .update_account(&a.id, "jira", "me@example.com", "ref", "https://x.atlassian.net", None)
            .await
            .unwrap();
        assert_eq!(updated.token_expires_at, None);

        // and can be set again
        let exp2 = Utc::now().round_subsecs(0) + Duration::days(10);
        let updated2 = repo
            .update_account(
                &a.id,
                "jira",
                "me@example.com",
                "ref",
                "https://x.atlassian.net",
                Some(exp2),
            )
            .await
            .unwrap();
        assert_eq!(updated2.token_expires_at, Some(exp2));

        let all = repo.list_all_accounts().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].token_expires_at, Some(exp2));
    }
}

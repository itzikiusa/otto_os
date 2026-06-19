//! Git accounts + repos repositories.

use chrono::{DateTime, Utc};
use otto_core::domain::{GitAccount, GitProviderKind, Repo};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct GitStore {
    pool: SqlitePool,
}

pub struct NewGitAccount {
    pub user_id: Id,
    pub provider: GitProviderKind,
    pub label: String,
    pub username: String,
    pub token_ref: String,
    pub api_base_url: Option<String>,
    pub namespace: Option<String>,
    /// When the token expires, if known. Drives credential-expiry notices.
    pub token_expires_at: Option<DateTime<Utc>>,
}

pub struct NewRepo {
    pub workspace_id: Id,
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
    pub provider: Option<GitProviderKind>,
    pub git_account_id: Option<Id>,
}

fn row_to_account(r: &sqlx::sqlite::SqliteRow) -> Result<GitAccount> {
    Ok(GitAccount {
        id: r.get("id"),
        user_id: r.get("user_id"),
        provider: GitProviderKind::parse(&r.get::<String, _>("provider"))
            .ok_or_else(|| Error::Internal("bad provider".into()))?,
        label: r.get("label"),
        username: r.get("username"),
        token_ref: r.get("token_ref"),
        api_base_url: r.get("api_base_url"),
        namespace: r.get("namespace"),
        token_expires_at: r
            .get::<Option<String>, _>("token_expires_at")
            .map(|s| ts(&s))
            .transpose()?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_repo(r: &sqlx::sqlite::SqliteRow) -> Result<Repo> {
    let provider: Option<String> = r.get("provider");
    Ok(Repo {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        path: r.get("path"),
        remote_url: r.get("remote_url"),
        provider: provider.as_deref().and_then(GitProviderKind::parse),
        git_account_id: r.get("git_account_id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl GitStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- accounts ---------------------------------------------------------

    pub async fn create_account(&self, a: NewGitAccount) -> Result<GitAccount> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO git_accounts (id, user_id, provider, label, username, token_ref, api_base_url, namespace, token_expires_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.user_id)
        .bind(a.provider.as_str())
        .bind(&a.label)
        .bind(&a.username)
        .bind(&a.token_ref)
        .bind(&a.api_base_url)
        .bind(&a.namespace)
        .bind(a.token_expires_at.map(fmt))
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create git account"))?;
        self.get_account(&id).await
    }

    pub async fn get_account(&self, id: &Id) -> Result<GitAccount> {
        let r = sqlx::query("SELECT * FROM git_accounts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("git account"))?;
        row_to_account(&r)
    }

    pub async fn list_accounts(&self, user_id: &Id) -> Result<Vec<GitAccount>> {
        let rows = sqlx::query("SELECT * FROM git_accounts WHERE user_id = ? ORDER BY created_at")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("git accounts"))?;
        rows.iter().map(row_to_account).collect()
    }

    /// Every git account across all users. Used by the credential monitor.
    pub async fn list_all_accounts(&self) -> Result<Vec<GitAccount>> {
        let rows = sqlx::query("SELECT * FROM git_accounts ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("git accounts"))?;
        rows.iter().map(row_to_account).collect()
    }

    pub async fn delete_account(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM git_accounts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete git account"))?;
        Ok(())
    }

    /// Update mutable fields of a git account.
    ///
    /// The caller supplies the *final* (merged) values for all columns:
    /// - `label`, `username`, `token_ref` are always non-null.
    /// - `namespace` / `api_base_url` are `Option<String>` (None = clear to NULL).
    /// - `token_expires_at` is the final value (None = NULL); callers wanting
    ///   "absent keeps current" merge against the existing account first.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_account(
        &self,
        id: &Id,
        label: &str,
        username: &str,
        token_ref: &str,
        namespace: Option<&str>,
        api_base_url: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<GitAccount> {
        sqlx::query(
            "UPDATE git_accounts SET label = ?, username = ?, token_ref = ?, namespace = ?, api_base_url = ?, token_expires_at = ? WHERE id = ?",
        )
        .bind(label)
        .bind(username)
        .bind(token_ref)
        .bind(namespace)
        .bind(api_base_url)
        .bind(token_expires_at.map(fmt))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update git account"))?;
        self.get_account(id).await
    }

    /// Persist an auto-detected token expiry without touching other columns.
    /// Used by the credential monitor / account probes.
    pub async fn set_token_expiry(
        &self,
        id: &Id,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        sqlx::query("UPDATE git_accounts SET token_expires_at = ? WHERE id = ?")
            .bind(token_expires_at.map(fmt))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set git account token expiry"))?;
        Ok(())
    }

    // -- repos ------------------------------------------------------------

    pub async fn create_repo(&self, n: NewRepo) -> Result<Repo> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO repos (id, workspace_id, name, path, remote_url, provider, git_account_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.workspace_id)
        .bind(&n.name)
        .bind(&n.path)
        .bind(&n.remote_url)
        .bind(n.provider.map(|p| p.as_str()))
        .bind(&n.git_account_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create repo"))?;
        self.get_repo(&id).await
    }

    pub async fn get_repo(&self, id: &Id) -> Result<Repo> {
        let r = sqlx::query("SELECT * FROM repos WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("repo"))?;
        row_to_repo(&r)
    }

    pub async fn list_repos(&self, ws: &Id) -> Result<Vec<Repo>> {
        let rows = sqlx::query("SELECT * FROM repos WHERE workspace_id = ? ORDER BY name")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("repos"))?;
        rows.iter().map(row_to_repo).collect()
    }

    pub async fn delete_repo(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM repos WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete repo"))?;
        Ok(())
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
    async fn token_expiry_round_trips_through_create_update_and_set() {
        let pool = mem_pool().await;
        let store = GitStore::new(pool.clone());
        let user = seed_user(&pool).await;
        let exp = Utc::now().round_subsecs(0) + Duration::days(5);

        // create persists the expiry
        let a = store
            .create_account(NewGitAccount {
                user_id: user.clone(),
                provider: GitProviderKind::Github,
                label: "gh".into(),
                username: "octocat".into(),
                token_ref: "ref".into(),
                api_base_url: None,
                namespace: None,
                token_expires_at: Some(exp),
            })
            .await
            .unwrap();
        assert_eq!(a.token_expires_at, Some(exp));

        // update with None clears it (caller is responsible for merge)
        let updated = store
            .update_account(&a.id, "gh", "octocat", "ref", None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.token_expires_at, None);

        // set_token_expiry (auto-detect path) writes it back
        let exp2 = Utc::now().round_subsecs(0) + Duration::days(30);
        store.set_token_expiry(&a.id, Some(exp2)).await.unwrap();
        let reloaded = store.get_account(&a.id).await.unwrap();
        assert_eq!(reloaded.token_expires_at, Some(exp2));

        // list_all_accounts surfaces it across users
        let all = store.list_all_accounts().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].token_expires_at, Some(exp2));
    }

    #[tokio::test]
    async fn create_without_expiry_is_none() {
        let pool = mem_pool().await;
        let store = GitStore::new(pool.clone());
        let user = seed_user(&pool).await;
        let a = store
            .create_account(NewGitAccount {
                user_id: user,
                provider: GitProviderKind::Bitbucket,
                label: "bb".into(),
                username: "u".into(),
                token_ref: "ref".into(),
                api_base_url: None,
                namespace: None,
                token_expires_at: None,
            })
            .await
            .unwrap();
        assert_eq!(a.token_expires_at, None);
    }
}

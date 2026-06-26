//! Repo rules — durable lessons generalized from review findings, fed into the
//! Context Engine. DB is the single source of truth; the server renders all
//! enabled rules for a workspace into `WorkspaceContextConfig.repo_rules_md`,
//! which the Provisioner injects into future agent sessions' instruction files.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};
use otto_core::finding::RepoRule;
use otto_core::{new_id, Id, Result};

/// Input for creating a repo rule.
pub struct NewRepoRule<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub category: Option<&'a str>,
    pub severity: Option<&'a str>,
    pub glob: Option<&'a str>,
    pub source_finding_id: Option<&'a str>,
}

#[derive(Clone)]
pub struct RepoRulesRepo {
    pool: SqlitePool,
}

impl RepoRulesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, workspace_id: &str, by: &str, r: NewRepoRule<'_>) -> Result<RepoRule> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO repo_rules \
             (id, workspace_id, title, body, category, severity, glob, source_finding_id, \
              enabled, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(r.title)
        .bind(r.body)
        .bind(r.category)
        .bind(r.severity)
        .bind(r.glob)
        .bind(r.source_finding_id)
        .bind(by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create repo rule"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<RepoRule> {
        let row = sqlx::query("SELECT * FROM repo_rules WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get repo rule"))?;
        Self::row(&row)
    }

    /// All rules for a workspace, newest first.
    pub async fn list(&self, workspace_id: &str) -> Result<Vec<RepoRule>> {
        let rows =
            sqlx::query("SELECT * FROM repo_rules WHERE workspace_id = ? ORDER BY created_at DESC")
                .bind(workspace_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("list repo rules"))?;
        rows.iter().map(Self::row).collect()
    }

    /// Only enabled rules, oldest first (stable order for the rendered block).
    pub async fn list_enabled(&self, workspace_id: &str) -> Result<Vec<RepoRule>> {
        let rows = sqlx::query(
            "SELECT * FROM repo_rules WHERE workspace_id = ? AND enabled = 1 ORDER BY created_at",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list enabled repo rules"))?;
        rows.iter().map(Self::row).collect()
    }

    pub async fn set_enabled(&self, id: &Id, enabled: bool) -> Result<RepoRule> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE repo_rules SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(enabled))
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("toggle repo rule"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM repo_rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete repo rule"))?;
        Ok(())
    }

    fn row(r: &sqlx::sqlite::SqliteRow) -> Result<RepoRule> {
        let enabled: i64 = r.try_get("enabled").unwrap_or(1);
        Ok(RepoRule {
            id: r.get("id"),
            workspace_id: r.get("workspace_id"),
            title: r.get("title"),
            body: r.get("body"),
            category: r.try_get("category").ok().flatten(),
            severity: r.try_get("severity").ok().flatten(),
            glob: r.try_get("glob").ok().flatten(),
            source_finding_id: r.try_get("source_finding_id").ok().flatten(),
            enabled: enabled != 0,
            created_by: r.try_get("created_by").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or_default(),
            updated_at: r.try_get("updated_at").unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_list_toggle_delete() {
        let repo = RepoRulesRepo::new(mem_pool().await);
        let rule = repo
            .create(
                "ws1",
                "u1",
                NewRepoRule {
                    title: "Never build SQL with format!".into(),
                    body: "Use parameterized queries.".into(),
                    category: Some("security"),
                    severity: Some("high"),
                    glob: None,
                    source_finding_id: Some("f1"),
                },
            )
            .await
            .unwrap();
        assert!(rule.enabled);
        assert_eq!(repo.list_enabled("ws1").await.unwrap().len(), 1);

        repo.set_enabled(&rule.id, false).await.unwrap();
        assert_eq!(repo.list_enabled("ws1").await.unwrap().len(), 0);
        assert_eq!(repo.list("ws1").await.unwrap().len(), 1);

        repo.delete(&rule.id).await.unwrap();
        assert_eq!(repo.list("ws1").await.unwrap().len(), 0);
    }
}

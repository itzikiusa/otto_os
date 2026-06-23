//! Product story discovery run repository.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryRun {
    pub id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub swarm_id: Id,
    pub project_id: Id,
    pub status: String,
    pub brief_md: String,
    pub report_md: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewDiscoveryRun {
    pub story_id: Id,
    pub workspace_id: Id,
    pub swarm_id: Id,
    pub project_id: Id,
    pub brief_md: String,
    pub created_by: Id,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<DiscoveryRun> {
    Ok(DiscoveryRun {
        id: r.get("id"),
        story_id: r.get("story_id"),
        workspace_id: r.get("workspace_id"),
        swarm_id: r.get("swarm_id"),
        project_id: r.get("project_id"),
        status: r.get("status"),
        brief_md: r.get("brief_md"),
        report_md: r.get("report_md"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProductDiscoveryRepo {
    pool: SqlitePool,
}

impl ProductDiscoveryRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new discovery run; `status` defaults to `'running'`.
    pub async fn create(&self, r: NewDiscoveryRun) -> Result<DiscoveryRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_discovery_runs
             (id, story_id, workspace_id, swarm_id, project_id, status,
              brief_md, report_md, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'running', ?, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&r.story_id)
        .bind(&r.workspace_id)
        .bind(&r.swarm_id)
        .bind(&r.project_id)
        .bind(&r.brief_md)
        .bind(&r.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create discovery run"))?;
        self.get_required(&id).await
    }

    /// Fetch by id, returning `Err(NotFound)` when the row is absent.
    async fn get_required(&self, id: &Id) -> Result<DiscoveryRun> {
        self.get(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("discovery run {id}")))
    }

    pub async fn get(&self, id: &Id) -> Result<Option<DiscoveryRun>> {
        let row = sqlx::query("SELECT * FROM product_discovery_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get discovery run"))?;
        row.as_ref().map(row_to_run).transpose()
    }

    /// List runs for a story, newest first.
    pub async fn list_for_story(&self, story_id: &Id) -> Result<Vec<DiscoveryRun>> {
        let rows = sqlx::query(
            "SELECT * FROM product_discovery_runs WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list discovery runs for story"))?;
        rows.iter().map(row_to_run).collect()
    }

    pub async fn get_by_project(&self, project_id: &Id) -> Result<Option<DiscoveryRun>> {
        let row = sqlx::query(
            "SELECT * FROM product_discovery_runs WHERE project_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get discovery run by project"))?;
        row.as_ref().map(row_to_run).transpose()
    }

    pub async fn set_status(&self, id: &Id, status: &str) -> Result<()> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE product_discovery_runs SET status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set discovery run status"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("discovery run {id}")));
        }
        Ok(())
    }

    pub async fn set_report(&self, id: &Id, report_md: &str) -> Result<()> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE product_discovery_runs SET report_md = ?, updated_at = ? WHERE id = ?",
        )
        .bind(report_md)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set discovery run report"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("discovery run {id}")));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    #[tokio::test]
    async fn create_get_list_report_status_roundtrip() {
        let pool = mem_pool().await;
        let repo = ProductDiscoveryRepo::new(pool);

        let run = repo
            .create(NewDiscoveryRun {
                story_id: "s1".into(),
                workspace_id: "w1".into(),
                swarm_id: "sw1".into(),
                project_id: "p1".into(),
                brief_md: "## Discovery Brief".into(),
                created_by: "u1".into(),
            })
            .await
            .unwrap();

        assert_eq!(run.status, "running");

        // get_by_project returns it
        let by_proj = repo.get_by_project(&"p1".into()).await.unwrap().unwrap();
        assert_eq!(by_proj.id, run.id);

        // list_for_story newest first — add a second run
        let run2 = repo
            .create(NewDiscoveryRun {
                story_id: "s1".into(),
                workspace_id: "w1".into(),
                swarm_id: "sw1".into(),
                project_id: "p2".into(),
                brief_md: "## Discovery Brief 2".into(),
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        let list = repo.list_for_story(&"s1".into()).await.unwrap();
        assert_eq!(list.len(), 2);
        // newest first: run2 should be first (created later)
        assert_eq!(list[0].id, run2.id);

        // set_report then get shows the report
        repo.set_report(&run.id, "## Findings").await.unwrap();
        let fetched = repo.get(&run.id).await.unwrap().unwrap();
        assert_eq!(fetched.report_md.as_deref(), Some("## Findings"));

        // set_status("done") persists
        repo.set_status(&run.id, "done").await.unwrap();
        let fetched2 = repo.get(&run.id).await.unwrap().unwrap();
        assert_eq!(fetched2.status, "done");

        // set_status / set_report on a missing id returns NotFound, not a panic
        let missing: Id = "nosuchid".into();
        assert!(matches!(
            repo.set_status(&missing, "done").await,
            Err(otto_core::Error::NotFound(_))
        ));
        assert!(matches!(
            repo.set_report(&missing, "## nope").await,
            Err(otto_core::Error::NotFound(_))
        ));
    }
}

//! Work the main personal agent hands to another agent
//! (`otto.personal_agent_delegate`), tracked until its outcome is back in the
//! lead's chat (migration 0203). The row is what makes a hand-off
//! never-dropped: the report-back task settles it, and a daemon restart
//! settles every row it left open (`interrupted`) and still tells the lead.

use chrono::Utc;
use otto_core::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::convert::{dberr, fmt};
use crate::DbPool;

/// One delegation. `id` is the delegated (linked) thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonalAgentDelegation {
    pub id: String,
    pub workspace_id: String,
    pub lead_agent_id: String,
    pub lead_thread_id: String,
    pub member_agent_id: String,
    pub member_name: String,
    pub task: String,
    /// `running` | `done` | `failed` | `stopped` | `interrupted`.
    pub status: String,
    /// A one-line outcome (the report's first line, or why it failed).
    pub summary: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// When the outcome reached the lead's chat; `None` = still owed.
    pub delivered_at: Option<String>,
}

/// A delegation to record (`status` starts `running`).
#[derive(Debug, Clone)]
pub struct NewDelegation {
    pub id: String,
    pub workspace_id: String,
    pub lead_agent_id: String,
    pub lead_thread_id: String,
    pub member_agent_id: String,
    pub member_name: String,
    pub task: String,
}

/// Longest task text kept on the row (the full task is in the thread).
const TASK_CHARS: usize = 600;

#[derive(Clone)]
pub struct DelegationsRepo {
    pool: DbPool,
}

fn row_to(r: &sqlx::sqlite::SqliteRow) -> Result<PersonalAgentDelegation> {
    let s = |c: &str| -> Result<String> { r.try_get(c).map_err(dberr("delegation row")) };
    let o = |c: &str| -> Result<Option<String>> { r.try_get(c).map_err(dberr("delegation row")) };
    Ok(PersonalAgentDelegation {
        id: s("id")?,
        workspace_id: s("workspace_id")?,
        lead_agent_id: s("lead_agent_id")?,
        lead_thread_id: s("lead_thread_id")?,
        member_agent_id: s("member_agent_id")?,
        member_name: s("member_name")?,
        task: s("task")?,
        status: s("status")?,
        summary: s("summary")?,
        created_at: s("created_at")?,
        finished_at: o("finished_at")?,
        delivered_at: o("delivered_at")?,
    })
}

impl DelegationsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    pub async fn create(&self, d: NewDelegation) -> Result<PersonalAgentDelegation> {
        let task: String = d.task.trim().chars().take(TASK_CHARS).collect();
        sqlx::query(
            "INSERT INTO personal_agent_delegations (id, workspace_id, lead_agent_id, \
             lead_thread_id, member_agent_id, member_name, task, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'running', ?)",
        )
        .bind(&d.id)
        .bind(&d.workspace_id)
        .bind(&d.lead_agent_id)
        .bind(&d.lead_thread_id)
        .bind(&d.member_agent_id)
        .bind(&d.member_name)
        .bind(&task)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("record delegation"))?;
        self.get(&d.id).await
    }

    pub async fn get(&self, id: &str) -> Result<PersonalAgentDelegation> {
        let row = sqlx::query("SELECT * FROM personal_agent_delegations WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("delegation not found"))?;
        row_to(&row)
    }

    /// The delegation's outcome (only while it is still `running`, so a
    /// restart's `interrupted` never overwrites a late report and vice
    /// versa). `true` = this call settled it.
    pub async fn finish(&self, id: &str, status: &str, summary: &str) -> Result<bool> {
        let summary: String = summary.trim().chars().take(TASK_CHARS).collect();
        let done = sqlx::query(
            "UPDATE personal_agent_delegations SET status = ?, summary = ?, finished_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(status)
        .bind(&summary)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish delegation"))?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn mark_delivered(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE personal_agent_delegations SET delivered_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("mark delegation delivered"))?;
        Ok(())
    }

    /// Every delegation `agent_id` led or worked, newest first.
    pub async fn list_for_agent(
        &self,
        agent_id: &str,
        limit: i64,
    ) -> Result<Vec<PersonalAgentDelegation>> {
        let rows = sqlx::query(
            "SELECT * FROM personal_agent_delegations \
             WHERE lead_agent_id = ? OR member_agent_id = ? \
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(agent_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list delegations"))?;
        rows.iter().map(row_to).collect()
    }

    /// Delegations whose outcome is not yet in the lead's chat: still
    /// `running` ones a previous daemon life left behind, and settled ones it
    /// never delivered.
    pub async fn undelivered(&self) -> Result<Vec<PersonalAgentDelegation>> {
        let rows = sqlx::query(
            "SELECT * FROM personal_agent_delegations WHERE delivered_at IS NULL \
             ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("undelivered delegations"))?;
        rows.iter().map(row_to).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(id: &str) -> NewDelegation {
        NewDelegation {
            id: id.into(),
            workspace_id: "w".into(),
            lead_agent_id: "lead".into(),
            lead_thread_id: "lt".into(),
            member_agent_id: "m".into(),
            member_name: "Scout".into(),
            task: "check the build".into(),
        }
    }

    #[tokio::test]
    async fn a_delegation_settles_once_and_stays_owed_until_delivered() {
        let pool = crate::db::test_pool().await;
        let repo = DelegationsRepo::new(pool);
        let d = repo.create(new("t1")).await.unwrap();
        assert_eq!(d.status, "running");
        assert!(repo.finish("t1", "done", "# Build is green").await.unwrap());
        // A restart's `interrupted` never overwrites the real outcome.
        assert!(!repo.finish("t1", "interrupted", "restart").await.unwrap());
        assert_eq!(repo.get("t1").await.unwrap().status, "done");
        assert_eq!(repo.undelivered().await.unwrap().len(), 1);
        repo.mark_delivered("t1").await.unwrap();
        assert!(repo.undelivered().await.unwrap().is_empty());
        let mine = repo.list_for_agent("m", 10).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(repo.list_for_agent("lead", 10).await.unwrap()[0].id, "t1");
    }
}

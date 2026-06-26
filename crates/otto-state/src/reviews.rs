//! Persistence for PR review runs and their draft comments.

use chrono::Utc;
use otto_core::domain::{
    CommentSeverity, CommentState, Review, ReviewAgentState, ReviewComment, ReviewStatus,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct ReviewsRepo {
    pool: SqlitePool,
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn row_to_comment(r: &sqlx::sqlite::SqliteRow) -> Result<ReviewComment> {
    let line_raw: Option<i64> = r.get("line");
    let posted_raw: i64 = r.get("posted");
    Ok(ReviewComment {
        id: r.get("id"),
        review_id: r.get("review_id"),
        path: r.get("path"),
        line: line_raw.map(|v| v as u32),
        severity: CommentSeverity::parse(&r.get::<String, _>("severity"))
            .ok_or_else(|| Error::Internal("bad severity".into()))?,
        body: r.get("body"),
        state: CommentState::parse(&r.get::<String, _>("state"))
            .ok_or_else(|| Error::Internal("bad comment state".into()))?,
        posted: posted_raw != 0,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_review(r: &sqlx::sqlite::SqliteRow, comments: Vec<ReviewComment>) -> Result<Review> {
    let pr_number_raw: i64 = r.get("pr_number");
    let agents_raw: String = r.try_get("agents_json").unwrap_or_default();
    let agents: Vec<ReviewAgentState> = serde_json::from_str(&agents_raw).unwrap_or_default();
    Ok(Review {
        id: r.get("id"),
        repo_id: r.get("repo_id"),
        pr_number: pr_number_raw as u64,
        status: ReviewStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad review status".into()))?,
        error: r.get("error"),
        comments,
        agents,
        created_at: ts(&r.get::<String, _>("created_at"))?,
        verdict: None,
        blocker_count: None,
        summary_md: None,
    })
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

impl ReviewsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new review row in status "running".
    pub async fn create_review(&self, repo_id: &Id, pr_number: u64) -> Result<Review> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at)
             VALUES (?, ?, ?, 'running', ?)",
        )
        .bind(&id)
        .bind(repo_id)
        .bind(pr_number as i64)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create review"))?;
        self.get_review(&id).await
    }

    /// Update the status (and optionally the error field) of a review row.
    pub async fn set_status(
        &self,
        id: &Id,
        status: ReviewStatus,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE pr_reviews SET status = ?, error = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set review status"))?;
        Ok(())
    }

    /// Fail every review still marked `running`. Called on daemon startup: a
    /// review's background task dies with the process, so any row left
    /// `running` from a previous run is orphaned and would otherwise spin in
    /// the UI forever. Returns the number of rows updated.
    pub async fn fail_running(&self, error: &str) -> Result<u64> {
        let res = sqlx::query("UPDATE pr_reviews SET status = 'error', error = ? WHERE status = 'running'")
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(dberr("fail running reviews"))?;
        Ok(res.rows_affected())
    }

    /// Overwrite the agents_json column for a review row.
    pub async fn set_agents(&self, id: &Id, agents: &[ReviewAgentState]) -> Result<()> {
        let json = serde_json::to_string(agents)
            .map_err(|e| Error::Internal(format!("serialize agents: {e}")))?;
        sqlx::query("UPDATE pr_reviews SET agents_json = ? WHERE id = ?")
            .bind(&json)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set agents"))?;
        Ok(())
    }

    /// Atomically replace a single agent's row (element `index` of the
    /// `agents_json` array) without touching the others.
    ///
    /// Review agents run concurrently and each persists its own live state. If
    /// every writer rewrote the whole array (read-modify-write across an await),
    /// a stale snapshot could commit after a fresher one and revert another
    /// agent's row to "pending" — making live agents look stuck/capped in the
    /// UI. `json_replace` does the replace in a single SQL statement, which
    /// SQLite executes atomically under its write lock, so concurrent writers to
    /// *different* indices can never clobber each other. Requires the array
    /// element to already exist (the initial [`set_agents`] seeds every index).
    pub async fn set_agent_at(
        &self,
        id: &Id,
        index: usize,
        agent: &ReviewAgentState,
    ) -> Result<()> {
        let elem = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize agent: {e}")))?;
        let path = format!("$[{index}]");
        sqlx::query(
            "UPDATE pr_reviews SET agents_json = json_replace(agents_json, ?, json(?)) WHERE id = ?",
        )
        .bind(&path)
        .bind(&elem)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set agent"))?;
        Ok(())
    }

    /// Add a draft comment to a review.
    pub async fn add_comment(
        &self,
        review_id: &Id,
        path: Option<&str>,
        line: Option<u32>,
        severity: CommentSeverity,
        body: &str,
    ) -> Result<ReviewComment> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO pr_review_comments (id, review_id, path, line, severity, body, state, posted, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'draft', 0, ?)",
        )
        .bind(&id)
        .bind(review_id)
        .bind(path)
        .bind(line.map(|v| v as i64))
        .bind(severity.as_str())
        .bind(body)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add review comment"))?;
        self.get_comment(&id).await
    }

    /// Fetch a review by id, loading its comments.
    pub async fn get_review(&self, id: &Id) -> Result<Review> {
        let row = sqlx::query("SELECT * FROM pr_reviews WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("review"))?;
        let comments = self.comments_for_review(id).await?;
        row_to_review(&row, comments)
    }

    /// Fetch the most-recent review for a (repo, pr_number) pair, with its comments.
    pub async fn latest_for_pr(&self, repo_id: &Id, pr_number: u64) -> Result<Option<Review>> {
        let row = sqlx::query(
            "SELECT * FROM pr_reviews WHERE repo_id = ? AND pr_number = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(repo_id)
        .bind(pr_number as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("latest review"))?;

        match row {
            None => Ok(None),
            Some(r) => {
                let id: String = r.get("id");
                let comments = self.comments_for_review(&id).await?;
                row_to_review(&r, comments).map(Some)
            }
        }
    }

    /// Fetch a single comment by id.
    pub async fn get_comment(&self, id: &Id) -> Result<ReviewComment> {
        let row = sqlx::query("SELECT * FROM pr_review_comments WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("review comment"))?;
        row_to_comment(&row)
    }

    /// Update state and posted flag of a comment.
    pub async fn set_comment_state(
        &self,
        id: &Id,
        state: CommentState,
        posted: bool,
    ) -> Result<ReviewComment> {
        sqlx::query("UPDATE pr_review_comments SET state = ?, posted = ? WHERE id = ?")
            .bind(state.as_str())
            .bind(if posted { 1i64 } else { 0i64 })
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set comment state"))?;
        self.get_comment(id).await
    }

    /// Fetch ALL reviews for a (repo_id, pr_number) pair, newest-first, each
    /// fully loaded with comments and agents.
    pub async fn list_for_pr(&self, repo_id: &Id, pr_number: i64) -> Result<Vec<Review>> {
        let rows = sqlx::query(
            "SELECT * FROM pr_reviews WHERE repo_id = ? AND pr_number = ?
             ORDER BY created_at DESC",
        )
        .bind(repo_id)
        .bind(pr_number)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list reviews for pr"))?;

        let mut reviews = Vec::with_capacity(rows.len());
        for r in &rows {
            let id: String = r.get("id");
            let comments = self.comments_for_review(&id).await?;
            reviews.push(row_to_review(r, comments)?);
        }
        Ok(reviews)
    }

    // -- private helpers --------------------------------------------------

    async fn comments_for_review(&self, review_id: &Id) -> Result<Vec<ReviewComment>> {
        let rows =
            sqlx::query("SELECT * FROM pr_review_comments WHERE review_id = ? ORDER BY created_at")
                .bind(review_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("review comments"))?;
        rows.iter().map(row_to_comment).collect()
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

    fn agent(name: &str, status: &str) -> ReviewAgentState {
        ReviewAgentState {
            name: name.into(),
            provider: "claude".into(),
            model: String::new(),
            status: status.into(),
            note: String::new(),
            comment_count: 0,
            session_id: None,
            findings: Vec::new(),
        }
    }

    /// Regression for the "agents look capped/stuck at PENDING" bug: each agent
    /// must be able to persist its own row without another writer reverting it.
    #[tokio::test]
    async fn set_agent_at_updates_one_row_without_clobbering_siblings() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());

        let id = new_id();
        sqlx::query(
            "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at)
             VALUES (?, 'r', 1, 'running', '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();

        // Seed two pending agents (the initial whole-array write).
        repo.set_agents(&id, &[agent("a", "pending"), agent("b", "pending")])
            .await
            .unwrap();

        // Each agent flips to running with its own session id, one index at a
        // time — as the concurrent run does.
        let mut a0 = agent("a", "running");
        a0.session_id = Some("s0".into());
        repo.set_agent_at(&id, 0, &a0).await.unwrap();

        let mut a1 = agent("b", "running");
        a1.session_id = Some("s1".into());
        repo.set_agent_at(&id, 1, &a1).await.unwrap();

        // Re-writing index 0 must NOT revert index 1 back to pending.
        repo.set_agent_at(&id, 0, &a0).await.unwrap();

        let review = repo.get_review(&id).await.unwrap();
        assert_eq!(review.agents.len(), 2);
        assert_eq!(review.agents[0].status, "running");
        assert_eq!(review.agents[0].session_id.as_deref(), Some("s0"));
        assert_eq!(review.agents[1].status, "running");
        assert_eq!(review.agents[1].session_id.as_deref(), Some("s1"));
    }

    /// A cancelled review persists and reads back as `ReviewStatus::Cancelled`
    /// (the Cancel button's terminal state), distinct from done/error.
    #[tokio::test]
    async fn set_status_cancelled_round_trips() {
        use otto_core::domain::ReviewStatus;
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let review = repo.create_review(&"r".to_string(), 7).await.unwrap();
        assert_eq!(review.status, ReviewStatus::Running);

        repo.set_status(&review.id, ReviewStatus::Cancelled, None)
            .await
            .unwrap();

        let after = repo.get_review(&review.id).await.unwrap();
        assert_eq!(after.status, ReviewStatus::Cancelled);
    }
}

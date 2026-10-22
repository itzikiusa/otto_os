//! Persistence for PR review runs and their draft comments.

use crate::DbPool;
use chrono::Utc;
use otto_core::domain::{
    CommentSeverity, CommentState, Review, ReviewAgentState, ReviewComment, ReviewStatus,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, ts};

/// Where a review run read the code — see [`ReviewsRepo::set_run_context`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewRunContext {
    pub source_branch: Option<String>,
    pub cwd: Option<String>,
    pub head_sha: Option<String>,
}

#[derive(Clone)]
pub struct ReviewsRepo {
    pool: DbPool,
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
    // Derived, not stored: the deterministic-fallback marker rides on the
    // summarizer's agent row (agents_json), so the reviews table is untouched.
    let summary_fallback = agents.iter().any(|a| a.fallback).then_some(true);
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
        summary_fallback,
    })
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

impl ReviewsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
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
        // A late background completion must not undo cancellation. Only an
        // explicit new attempt (`running`) may reopen a cancelled review.
        sqlx::query("UPDATE pr_reviews SET status = ?, error = ? WHERE id = ? AND (status != 'cancelled' OR ? = 'running')")
            .bind(status.as_str())
            .bind(error)
            .bind(id)
            .bind(status.as_str())
            .execute(&self.pool)
            .await
            .map_err(dberr("set review status"))?;
        Ok(())
    }

    /// Atomically claim a finished review for a new attempt: flip it to
    /// `running` ONLY if it is not running already. `false` ⇒ another attempt
    /// won the race (two quick "Retry summarizer" clicks used to both pass a
    /// read-then-write check and each persist a full set of drafts).
    pub async fn try_begin_rerun(&self, id: &Id) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE pr_reviews SET status = 'running', error = NULL WHERE id = ? AND status != 'running' \
             AND NOT EXISTS (SELECT 1 FROM json_each(CASE WHEN json_valid(agents_json) THEN agents_json ELSE '[]' END) \
               WHERE CAST(key AS INTEGER) < json_array_length(agents_json) - 1 \
                 AND json_extract(value, '$.status') IN ('pending', 'running', 'waiting'))",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("claim review rerun"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Fail every review still marked `running`. Called on daemon startup: a
    /// review's background task dies with the process, so any row left
    /// `running` from a previous run is orphaned and would otherwise spin in
    /// the UI forever. Returns the number of rows updated.
    pub async fn fail_running(&self, error: &str) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE pr_reviews SET status = 'error', error = ? WHERE status = 'running'",
        )
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(dberr("fail running reviews"))?;
        Ok(res.rows_affected())
    }

    /// Settle every agent row a dead process left live. Called on daemon
    /// startup after [`Self::fail_running`]: a per-agent Retry runs on a
    /// SETTLED review, so its `pending` / `running` / `waiting` row is not
    /// covered by the review-level sweep, and while it stays live Retry, Stop
    /// and the summarizer re-run all refuse. Each such row becomes `error`
    /// with `note`, in reviews that are not `running`. Returns the number of
    /// agent rows rewritten.
    pub async fn fail_orphaned_agents(&self, note: &str) -> Result<u64> {
        let rows = sqlx::query(
            "SELECT id, agents_json FROM pr_reviews WHERE status != 'running' \
             AND EXISTS (SELECT 1 FROM json_each(CASE WHEN json_valid(agents_json) THEN agents_json ELSE '[]' END) \
               WHERE json_extract(value, '$.status') IN ('pending', 'running', 'waiting'))",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("orphaned review agents"))?;
        let mut settled = 0u64;
        for row in rows {
            let id: Id = row.get("id");
            let raw: String = row.try_get("agents_json").unwrap_or_default();
            let Ok(mut agents) = serde_json::from_str::<Vec<ReviewAgentState>>(&raw) else {
                continue;
            };
            for (index, agent) in agents.iter_mut().enumerate() {
                if matches!(agent.status.as_str(), "pending" | "running" | "waiting") {
                    agent.status = "error".into();
                    agent.note = note.to_string();
                    // Per element, so a malformed sibling never blocks the rest.
                    self.set_agent_at(&id, index, agent).await?;
                    settled += 1;
                }
            }
        }
        Ok(settled)
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

    /// [`Self::set_agent_at`] as a claim for a single-agent Retry: written only
    /// while the review is not running and that agent is still SETTLED
    /// (`done` / `error` / `skipped`). Two concurrent retries both passed the
    /// handler's snapshot check, both spawned an agent into the same index,
    /// and the replaced cancel flag left one of them unstoppable. `false` when
    /// another retry (or a live run) got there first.
    pub async fn claim_agent_retry(
        &self,
        id: &Id,
        index: usize,
        agent: &ReviewAgentState,
    ) -> Result<bool> {
        let elem = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize agent: {e}")))?;
        let path = format!("$[{index}]");
        let status_path = format!("$[{index}].status");
        let changed = sqlx::query(
            "UPDATE pr_reviews SET agents_json = json_replace(agents_json, ?, json(?)) \
             WHERE id = ? AND status != 'running' \
               AND json_extract(agents_json, ?) IN ('done', 'error', 'skipped')",
        )
        .bind(&path)
        .bind(&elem)
        .bind(id)
        .bind(&status_path)
        .execute(&self.pool)
        .await
        .map_err(dberr("claim agent retry"))?
        .rows_affected();
        Ok(changed == 1)
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

    /// Delete a review's unposted draft comments (a summarizer re-run replaces
    /// them). Approved/declined/posted comments are user decisions and stay.
    pub async fn delete_draft_comments(&self, review_id: &Id) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM pr_review_comments
             WHERE review_id = ? AND state = 'draft' AND posted = 0",
        )
        .bind(review_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("delete draft comments"))?;
        Ok(res.rows_affected())
    }

    /// Just a review's status — for pollers waiting on completion, which used
    /// to load the whole row (agents JSON) plus every comment each tick.
    pub async fn review_status(&self, id: &Id) -> Result<ReviewStatus> {
        let status: String = sqlx::query_scalar("SELECT status FROM pr_reviews WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("review status"))?;
        ReviewStatus::parse(&status).ok_or_else(|| Error::Internal("bad review status".into()))
    }

    /// A review's status plus its agent rows — what a waiter surfacing the
    /// reviewers' live sessions needs. One row, no comments: the workflow
    /// `review_run` node used to load the whole review AND decode every
    /// comment every 2 s for the review's lifetime.
    pub async fn review_agents(&self, id: &Id) -> Result<(ReviewStatus, Vec<ReviewAgentState>)> {
        let row = sqlx::query("SELECT status, agents_json FROM pr_reviews WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("review agents"))?;
        let status = ReviewStatus::parse(&row.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad review status".into()))?;
        let agents_raw: String = row.try_get("agents_json").unwrap_or_default();
        Ok((
            status,
            serde_json::from_str(&agents_raw).unwrap_or_default(),
        ))
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

    /// [`Self::get_review`] without the comments (`comments` is empty): for
    /// readers that use only the review row — the Mission Control projector
    /// runs this on every review event, and a re-run review keeps hundreds of
    /// decided comments it would decode for nothing.
    pub async fn review_head(&self, id: &Id) -> Result<Review> {
        let row = sqlx::query("SELECT * FROM pr_reviews WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("review"))?;
        row_to_review(&row, Vec::new())
    }

    /// The `limit` newest reviews of a workspace's repos, newest first, as
    /// heads ([`Self::review_head`], no comments) — one query for the
    /// projector's backfill instead of one `get_review` per review.
    pub async fn recent_heads_in_workspace(&self, ws: &Id, limit: i64) -> Result<Vec<Review>> {
        let rows = sqlx::query(
            "SELECT r.* FROM pr_reviews r JOIN repos gr ON gr.id = r.repo_id \
             WHERE gr.workspace_id = ? ORDER BY r.created_at DESC LIMIT ?",
        )
        .bind(ws)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("recent reviews"))?;
        rows.iter().map(|r| row_to_review(r, Vec::new())).collect()
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

    /// Person edits of a not-yet-posted comment: replace a DRAFT's body and/or
    /// move a DECLINED comment back to draft. Guarded in SQL so a comment that
    /// is posted (or changed state meanwhile) is never touched. `Ok(None)` ⇒
    /// nothing matched the guard (the caller reports a conflict).
    pub async fn edit_unposted_comment(
        &self,
        id: &Id,
        body: Option<&str>,
        restore_draft: bool,
    ) -> Result<Option<ReviewComment>> {
        let mut tx = self.pool.begin().await.map_err(dberr("edit comment"))?;
        if restore_draft {
            let res = sqlx::query(
                "UPDATE pr_review_comments SET state = 'draft' WHERE id = ? AND state = 'declined' AND posted = 0",
            )
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("restore comment"))?;
            if res.rows_affected() != 1 {
                return Ok(None);
            }
        }
        if let Some(body) = body {
            let res = sqlx::query(
                "UPDATE pr_review_comments SET body = ? WHERE id = ? AND state = 'draft' AND posted = 0",
            )
            .bind(body)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("edit comment body"))?;
            if res.rows_affected() != 1 {
                return Ok(None);
            }
        }
        tx.commit().await.map_err(dberr("edit comment"))?;
        self.get_comment(id).await.map(Some)
    }

    /// Atomically claim a comment for posting to the forge: flips `posted`
    /// 0→1 and returns `true` only for the ONE caller that flipped it. A double
    /// click / retried request sees `false` and must not post again (the
    /// unconditional state update used to let both requests post).
    pub async fn claim_comment_post(&self, id: &Id) -> Result<bool> {
        let res =
            sqlx::query("UPDATE pr_review_comments SET posted = 1 WHERE id = ? AND posted = 0")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("claim comment post"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Undo [`claim_comment_post`](Self::claim_comment_post) after the forge
    /// call failed, so a later approve can try again.
    pub async fn release_comment_post(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE pr_review_comments SET posted = 0 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("release comment post"))?;
        Ok(())
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

    // -- durable retry artifacts (migration 0100) -------------------------
    //
    // The per-agent prompt + the run's unified diff are persisted at dispatch
    // time so a per-agent Retry survives reboots / temp-dir sweeps / daemon
    // redeploys. The $TMPDIR copies remain the in-flight/legacy path; these
    // rows are the durable source a retry reads first.

    /// Upsert the fully-composed prompt for one review agent.
    pub async fn set_agent_prompt(
        &self,
        review_id: &Id,
        agent_index: usize,
        prompt: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO review_agent_prompts (review_id, agent_index, prompt)
             VALUES (?, ?, ?)
             ON CONFLICT (review_id, agent_index) DO UPDATE SET prompt = excluded.prompt",
        )
        .bind(review_id)
        .bind(agent_index as i64)
        .bind(prompt)
        .execute(&self.pool)
        .await
        .map_err(dberr("set agent prompt"))?;
        Ok(())
    }

    /// Read back one agent's persisted prompt (None for pre-0100 reviews).
    pub async fn get_agent_prompt(
        &self,
        review_id: &Id,
        agent_index: usize,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT prompt FROM review_agent_prompts WHERE review_id = ? AND agent_index = ?",
        )
        .bind(review_id)
        .bind(agent_index as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get agent prompt"))?;
        Ok(row.map(|r| r.get("prompt")))
    }

    /// Upsert the run's unified diff (one row per review).
    pub async fn set_diff(&self, review_id: &Id, diff: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO review_diffs (review_id, diff) VALUES (?, ?)
             ON CONFLICT (review_id) DO UPDATE SET diff = excluded.diff",
        )
        .bind(review_id)
        .bind(diff)
        .execute(&self.pool)
        .await
        .map_err(dberr("set review diff"))?;
        Ok(())
    }

    /// Read back the run's persisted diff (None for pre-0100 reviews).
    pub async fn get_diff(&self, review_id: &Id) -> Result<Option<String>> {
        let row = sqlx::query("SELECT diff FROM review_diffs WHERE review_id = ?")
            .bind(review_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get review diff"))?;
        Ok(row.map(|r| r.get("diff")))
    }

    /// Record where this run reviews the code (`0176_git2_review_run_context`):
    /// the source branch (scopes local finding resolution), the checkout path
    /// and its head sha (what a Retry re-enters). Upsert; `None`s are stored
    /// as NULL.
    pub async fn set_run_context(&self, review_id: &Id, ctx: &ReviewRunContext) -> Result<()> {
        sqlx::query(
            "INSERT INTO review_run_context (review_id, source_branch, cwd, head_sha)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (review_id) DO UPDATE SET source_branch = excluded.source_branch,
               cwd = excluded.cwd, head_sha = excluded.head_sha",
        )
        .bind(review_id)
        .bind(ctx.source_branch.as_deref())
        .bind(ctx.cwd.as_deref())
        .bind(ctx.head_sha.as_deref())
        .execute(&self.pool)
        .await
        .map_err(dberr("set review run context"))?;
        Ok(())
    }

    /// Read back [`Self::set_run_context`] (None for reviews that predate it).
    pub async fn get_run_context(&self, review_id: &Id) -> Result<Option<ReviewRunContext>> {
        let row = sqlx::query(
            "SELECT source_branch, cwd, head_sha FROM review_run_context WHERE review_id = ?",
        )
        .bind(review_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get review run context"))?;
        Ok(row.map(|r| ReviewRunContext {
            source_branch: r.get("source_branch"),
            cwd: r.get("cwd"),
            head_sha: r.get("head_sha"),
        }))
    }

    /// Drop every durable retry artifact for a review (prompts + diff) — used
    /// by the cancel cleanup, mirroring its temp-file sweep.
    pub async fn delete_run_artifacts(&self, review_id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM review_agent_prompts WHERE review_id = ?")
            .bind(review_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete agent prompts"))?;
        sqlx::query("DELETE FROM review_diffs WHERE review_id = ?")
            .bind(review_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete review diff"))?;
        Ok(())
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

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
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
            fallback: false,
            lens: String::new(),
            lens_slugs: Vec::new(),
        }
    }

    #[tokio::test]
    async fn run_context_round_trips_and_upserts() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let id = new_id();
        assert_eq!(repo.get_run_context(&id).await.unwrap(), None);
        let mut ctx = ReviewRunContext {
            source_branch: Some("feature/a".into()),
            cwd: Some("/tmp/wt".into()),
            head_sha: None,
        };
        repo.set_run_context(&id, &ctx).await.unwrap();
        assert_eq!(repo.get_run_context(&id).await.unwrap(), Some(ctx.clone()));
        ctx.head_sha = Some("abc1234".into());
        repo.set_run_context(&id, &ctx).await.unwrap();
        assert_eq!(repo.get_run_context(&id).await.unwrap(), Some(ctx));
    }

    /// backend-performance-4-04: the projector's head reads carry no comments,
    /// and the workspace backfill is the newest heads of that workspace only.
    #[tokio::test]
    async fn review_heads_skip_comments_and_scope_to_the_workspace() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        for (id, ws) in [("ra", "w1"), ("rb", "w2")] {
            sqlx::query(
                "INSERT INTO repos(id,workspace_id,name,path,created_at) \
                 VALUES(?,?,?,?, '2026-01-01T00:00:00Z')",
            )
            .bind(id)
            .bind(ws)
            .bind(id)
            .bind(format!("/tmp/{id}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        for (id, repo_id, at) in [
            ("old", "ra", "2026-01-01T00:00:00Z"),
            ("new", "ra", "2026-01-02T00:00:00Z"),
            ("other", "rb", "2026-01-03T00:00:00Z"),
        ] {
            sqlx::query(
                "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at)
                 VALUES (?, ?, 1, 'done', ?)",
            )
            .bind(id)
            .bind(repo_id)
            .bind(at)
            .execute(&pool)
            .await
            .unwrap();
        }
        let head = repo.review_head(&"new".to_string()).await.unwrap();
        assert_eq!((head.id.as_str(), head.comments.len()), ("new", 0));
        let ids = |v: Vec<Review>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
        let w1 = repo
            .recent_heads_in_workspace(&"w1".to_string(), 50)
            .await
            .unwrap();
        assert_eq!(ids(w1), vec!["new", "old"]);
        let one = repo
            .recent_heads_in_workspace(&"w1".to_string(), 1)
            .await
            .unwrap();
        assert_eq!(ids(one), vec!["new"]);
    }

    /// Two retries of one settled agent: only the first claims it; a live
    /// review or agent can't be claimed at all.
    #[tokio::test]
    async fn an_agent_retry_is_claimed_once() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let id = new_id();
        sqlx::query(
            "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at)
             VALUES (?, 'r', 1, 'done', '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        repo.set_agents(&id, &[agent("a", "error"), agent("b", "running")])
            .await
            .unwrap();
        let pending = agent("a", "pending");
        assert!(repo.claim_agent_retry(&id, 0, &pending).await.unwrap());
        assert!(
            !repo.claim_agent_retry(&id, 0, &pending).await.unwrap(),
            "the second retry loses"
        );
        assert!(
            !repo
                .claim_agent_retry(&id, 1, &agent("b", "pending"))
                .await
                .unwrap(),
            "a live agent is not retried"
        );
        assert_eq!(
            repo.get_review(&id).await.unwrap().agents[0].status,
            "pending"
        );
    }

    /// backend-correctness-3-01: the boot sweep settles live agent rows of
    /// settled reviews (an interrupted per-agent retry) and leaves a running
    /// review's rows and already-settled rows alone.
    #[tokio::test]
    async fn boot_sweep_settles_orphaned_agents_of_settled_reviews() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let settled = new_id();
        let running = new_id();
        for (id, status) in [(&settled, "done"), (&running, "running")] {
            sqlx::query(
                "INSERT INTO pr_reviews (id, repo_id, pr_number, status, created_at)
                 VALUES (?, 'r', 1, ?, '2026-01-01T00:00:00Z')",
            )
            .bind(id)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        }
        repo.set_agents(
            &settled,
            &[
                agent("a", "pending"),
                agent("b", "done"),
                agent("c", "waiting"),
                agent("summary", "done"),
            ],
        )
        .await
        .unwrap();
        repo.set_agents(&running, &[agent("a", "pending"), agent("s", "pending")])
            .await
            .unwrap();
        let note = "Interrupted by a daemon restart";
        assert_eq!(repo.fail_orphaned_agents(note).await.unwrap(), 2);
        let after = repo.get_review(&settled).await.unwrap().agents;
        assert_eq!(after[0].status, "error");
        assert_eq!(after[0].note, note);
        assert_eq!(after[1].status, "done", "settled rows untouched");
        assert_eq!(after[2].status, "error");
        let live = repo.get_review(&running).await.unwrap().agents;
        assert_eq!(live[0].status, "pending", "a running review is not swept");
        // Idempotent: nothing left to settle.
        assert_eq!(repo.fail_orphaned_agents(note).await.unwrap(), 0);
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

    /// Drafts are editable; declined comments restore to draft; posted or
    /// approved comments are never touched.
    #[tokio::test]
    async fn edit_unposted_comment_respects_state_guards() {
        use otto_core::domain::{CommentSeverity, CommentState};
        let repo = ReviewsRepo::new(mem_pool().await);
        let review = repo.create_review(&"r".to_string(), 7).await.unwrap();
        let c = repo
            .add_comment(
                &review.id,
                Some("a.rs"),
                Some(1),
                CommentSeverity::Warn,
                "orig",
            )
            .await
            .unwrap();
        let e = repo
            .edit_unposted_comment(&c.id, Some("edited"), false)
            .await
            .unwrap();
        assert_eq!(e.unwrap().body, "edited");
        // Declined: body edit refused until restored, restore works.
        repo.set_comment_state(&c.id, CommentState::Declined, false)
            .await
            .unwrap();
        assert!(repo
            .edit_unposted_comment(&c.id, Some("x"), false)
            .await
            .unwrap()
            .is_none());
        let r = repo
            .edit_unposted_comment(&c.id, Some("again"), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((r.state, r.body.as_str()), (CommentState::Draft, "again"));
        // Posted: untouchable.
        repo.set_comment_state(&c.id, CommentState::Approved, true)
            .await
            .unwrap();
        assert!(repo
            .edit_unposted_comment(&c.id, Some("y"), false)
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .edit_unposted_comment(&c.id, None, true)
            .await
            .unwrap()
            .is_none());
    }

    /// Double "Retry summarizer": only the first claim wins.
    #[tokio::test]
    async fn try_begin_rerun_is_exclusive() {
        use otto_core::domain::ReviewStatus;
        let repo = ReviewsRepo::new(mem_pool().await);
        let review = repo.create_review(&"r".to_string(), 7).await.unwrap();
        // Still running ⇒ no new attempt.
        assert!(!repo.try_begin_rerun(&review.id).await.unwrap());
        repo.set_status(&review.id, ReviewStatus::Error, Some("boom"))
            .await
            .unwrap();
        assert!(repo.try_begin_rerun(&review.id).await.unwrap());
        assert!(!repo.try_begin_rerun(&review.id).await.unwrap());
        let after = repo.get_review(&review.id).await.unwrap();
        assert_eq!(after.status, ReviewStatus::Running);
        assert!(after.error.is_none());
    }

    #[tokio::test]
    async fn reviewer_and_summarizer_retry_claims_exclude_each_other() {
        let repo = ReviewsRepo::new(mem_pool().await);
        let review = repo.create_review(&"r".into(), 7).await.unwrap();
        repo.set_agents(&review.id, &[agent("a", "done"), agent("summary", "done")])
            .await
            .unwrap();
        repo.set_status(&review.id, ReviewStatus::Done, None)
            .await
            .unwrap();
        assert!(repo
            .claim_agent_retry(&review.id, 0, &agent("a", "pending"))
            .await
            .unwrap());
        assert!(!repo.try_begin_rerun(&review.id).await.unwrap());
        repo.set_agent_at(&review.id, 0, &agent("a", "done"))
            .await
            .unwrap();
        // An interrupted summarizer row itself must remain retryable. Only
        // active REVIEWERS exclude the new summary attempt.
        repo.set_agent_at(&review.id, 1, &agent("summary", "running"))
            .await
            .unwrap();
        assert!(repo.try_begin_rerun(&review.id).await.unwrap());
        assert!(!repo
            .claim_agent_retry(&review.id, 0, &agent("a", "pending"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn comment_post_claim_is_exclusive_and_releasable() {
        let repo = ReviewsRepo::new(mem_pool().await);
        let review = repo.create_review(&"r".to_string(), 7).await.unwrap();
        let c = repo
            .add_comment(
                &review.id,
                Some("a.rs"),
                Some(3),
                CommentSeverity::Warn,
                "x",
            )
            .await
            .unwrap();
        assert!(repo.claim_comment_post(&c.id).await.unwrap());
        // A second (double-click / retried) approve must not post again.
        assert!(!repo.claim_comment_post(&c.id).await.unwrap());
        // A failed post releases the claim so a later approve can retry.
        repo.release_comment_post(&c.id).await.unwrap();
        assert!(repo.claim_comment_post(&c.id).await.unwrap());
    }

    #[tokio::test]
    async fn summarizer_completion_cannot_overwrite_cancelled_review() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool);
        let review = repo.create_review(&"r".to_string(), 7).await.unwrap();
        repo.set_status(&review.id, ReviewStatus::Cancelled, None)
            .await
            .unwrap();
        repo.set_status(&review.id, ReviewStatus::Done, None)
            .await
            .unwrap();
        assert_eq!(
            repo.get_review(&review.id).await.unwrap().status,
            ReviewStatus::Cancelled
        );
        repo.set_status(&review.id, ReviewStatus::Error, Some("late failure"))
            .await
            .unwrap();
        assert_eq!(
            repo.get_review(&review.id).await.unwrap().status,
            ReviewStatus::Cancelled
        );
        // Explicit retries may start a fresh run, and subsequently complete it.
        repo.set_status(&review.id, ReviewStatus::Running, None)
            .await
            .unwrap();
        repo.set_status(&review.id, ReviewStatus::Done, None)
            .await
            .unwrap();
        assert_eq!(
            repo.get_review(&review.id).await.unwrap().status,
            ReviewStatus::Done
        );
    }

    /// Durable-retry storage: prompt + diff rows round-trip, upsert in place,
    /// read back None when absent, and die together via delete_run_artifacts.
    #[tokio::test]
    async fn agent_prompts_and_diff_round_trip_and_cleanup() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let review = repo.create_review(&"r".to_string(), 1).await.unwrap();

        assert_eq!(repo.get_agent_prompt(&review.id, 0).await.unwrap(), None);
        assert_eq!(repo.get_diff(&review.id).await.unwrap(), None);

        repo.set_agent_prompt(&review.id, 0, "prompt-a")
            .await
            .unwrap();
        repo.set_agent_prompt(&review.id, 1, "prompt-b")
            .await
            .unwrap();
        repo.set_agent_prompt(&review.id, 0, "prompt-a2")
            .await
            .unwrap(); // upsert
        repo.set_diff(&review.id, "diff-1").await.unwrap();
        repo.set_diff(&review.id, "diff-2").await.unwrap(); // upsert

        assert_eq!(
            repo.get_agent_prompt(&review.id, 0)
                .await
                .unwrap()
                .as_deref(),
            Some("prompt-a2")
        );
        assert_eq!(
            repo.get_agent_prompt(&review.id, 1)
                .await
                .unwrap()
                .as_deref(),
            Some("prompt-b")
        );
        assert_eq!(
            repo.get_diff(&review.id).await.unwrap().as_deref(),
            Some("diff-2")
        );

        repo.delete_run_artifacts(&review.id).await.unwrap();
        assert_eq!(repo.get_agent_prompt(&review.id, 0).await.unwrap(), None);
        assert_eq!(repo.get_agent_prompt(&review.id, 1).await.unwrap(), None);
        assert_eq!(repo.get_diff(&review.id).await.unwrap(), None);
    }

    /// The deterministic-fallback marker rides in agents_json: a summarizer row
    /// with `fallback: true` surfaces as `Review.summary_fallback == Some(true)`
    /// and its absence keeps the field off the wire entirely (None).
    #[tokio::test]
    async fn summary_fallback_derives_from_agents_json() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let review = repo.create_review(&"r".to_string(), 2).await.unwrap();

        repo.set_agents(
            &review.id,
            &[agent("a", "done"), agent("Summarizer", "done")],
        )
        .await
        .unwrap();
        assert_eq!(
            repo.get_review(&review.id).await.unwrap().summary_fallback,
            None
        );

        let mut summarizer = agent("Summarizer", "done");
        summarizer.fallback = true;
        repo.set_agents(&review.id, &[agent("a", "done"), summarizer])
            .await
            .unwrap();
        assert_eq!(
            repo.get_review(&review.id).await.unwrap().summary_fallback,
            Some(true)
        );
    }

    /// The narrow waiter read returns the same status and agent rows as the
    /// full `get_review`, without the comments.
    #[tokio::test]
    async fn review_agents_matches_the_full_row() {
        let pool = mem_pool().await;
        let repo = ReviewsRepo::new(pool.clone());
        let review = repo.create_review(&"r".to_string(), 3).await.unwrap();
        let mut running = agent("a", "running");
        running.session_id = Some("s1".into());
        repo.set_agents(&review.id, &[running, agent("Summarizer", "pending")])
            .await
            .unwrap();
        let (status, agents) = repo.review_agents(&review.id).await.unwrap();
        let full = repo.get_review(&review.id).await.unwrap();
        assert_eq!(status, full.status);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].session_id.as_deref(), Some("s1"));
        assert!(repo.review_agents(&"missing".to_string()).await.is_err());
    }
}

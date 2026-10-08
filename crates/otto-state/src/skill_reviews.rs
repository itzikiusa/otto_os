//! Persistence for Skills Lab reviews (`skill_reviews`).
//!
//! One row per review of one skill package. The live per-agent state lives in
//! `agents_json` and is updated one array index at a time via [`SkillReviewsRepo::set_agent_at`]
//! (mirrors [`crate::reviews::ReviewsRepo::set_agent_at`]) so concurrent provider
//! agents never clobber each other's rows. The deterministic static report and
//! the summarizer's aggregate ride in `static_json` / `summary_json`.

use crate::DbPool;
use chrono::Utc;
use otto_core::domain::{SkillReview, SkillReviewAgent, SkillReviewSummary, SkillStaticReport};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct SkillReviewsRepo {
    pool: DbPool,
}

fn row_to_review(r: &sqlx::sqlite::SqliteRow) -> Result<SkillReview> {
    let agents_raw: String = r.try_get("agents_json").unwrap_or_default();
    let agents: Vec<SkillReviewAgent> = serde_json::from_str(&agents_raw).unwrap_or_default();
    let static_report: Option<SkillStaticReport> = r
        .try_get::<Option<String>, _>("static_json")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok());
    let summary: Option<SkillReviewSummary> = r
        .try_get::<Option<String>, _>("summary_json")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok());
    let fix_agent: Option<SkillReviewAgent> = r
        .try_get::<Option<String>, _>("fix_json")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok());
    Ok(SkillReview {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        skill_name: r.get("skill_name"),
        skill_source: r.get("skill_source"),
        status: r.get("status"),
        agent_mode: r.get("agent_mode"),
        instructions: r.try_get("instructions").unwrap_or_default(),
        agents,
        fix_agent,
        static_report,
        summary,
        error: r.get("error"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

impl SkillReviewsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// Recover work whose in-memory owner disappeared at daemon restart.
    pub async fn fail_running(&self) -> Result<u64> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin skill review recovery"))?;
        let rows = sqlx::query("SELECT * FROM skill_reviews WHERE status = 'running' OR EXISTS (SELECT 1 FROM json_each(agents_json) WHERE json_extract(value, '$.status') IN ('pending','running','waiting')) OR json_extract(fix_json, '$.status') IN ('pending','running','waiting')")
            .fetch_all(&mut *tx).await.map_err(dberr("read interrupted skill reviews"))?;
        let count = rows.len() as u64;
        for row in rows {
            let mut review = row_to_review(&row)?;
            let reason = "interrupted by daemon restart; inspect partial work before retrying";
            for agent in review.agents.iter_mut().chain(review.fix_agent.iter_mut()) {
                if matches!(agent.status.as_str(), "pending" | "running" | "waiting") {
                    agent.status = "error".into();
                    agent.note = reason.into();
                }
            }
            if review.status == "running" {
                review.status = "error".into();
                review.error = Some(reason.into());
            }
            let agents = serde_json::to_string(&review.agents)
                .map_err(|e| Error::Internal(e.to_string()))?;
            let fixer = review
                .fix_agent
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| Error::Internal(e.to_string()))?;
            sqlx::query("UPDATE skill_reviews SET status = ?, error = ?, agents_json = ?, fix_json = ?, updated_at = ? WHERE id = ?")
                .bind(&review.status).bind(&review.error).bind(agents).bind(fixer).bind(fmt(Utc::now())).bind(&review.id)
                .execute(&mut *tx).await.map_err(dberr("recover skill review"))?;
        }
        tx.commit()
            .await
            .map_err(dberr("commit skill review recovery"))?;
        Ok(count)
    }

    /// Claim a reviewer retry before spawning and invalidate the old aggregate.
    /// Parent status excludes another retry or Apply while results are changing.
    pub async fn claim_retry(
        &self,
        id: &Id,
        index: usize,
        agent: &SkillReviewAgent,
    ) -> Result<bool> {
        let agent = serde_json::to_string(agent).map_err(|e| Error::Internal(e.to_string()))?;
        let changed = sqlx::query("UPDATE skill_reviews SET status = 'running', error = NULL, summary_json = NULL, agents_json = json_replace(agents_json, ?, json(?)), updated_at = ? WHERE id = ? AND status IN ('done','error') AND (fix_json IS NULL OR json_extract(fix_json, '$.status') IN ('done','error','cancelled'))")
            .bind(format!("$[{index}]")).bind(agent).bind(fmt(Utc::now())).bind(id)
            .execute(&self.pool).await.map_err(dberr("claim skill review retry"))?.rows_affected();
        Ok(changed == 1)
    }

    /// Create a new review in status "running".
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        workspace_id: &Id,
        skill_name: &str,
        skill_source: &str,
        agent_mode: &str,
        instructions: &str,
        created_by: Option<&str>,
    ) -> Result<SkillReview> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO skill_reviews
               (id, workspace_id, skill_name, skill_source, status, agent_mode,
                instructions, agents_json, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, 'running', ?, ?, '[]', ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(skill_name)
        .bind(skill_source)
        .bind(agent_mode)
        .bind(instructions)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create skill review"))?;
        self.get(&id).await
    }

    /// Fetch one review by id.
    pub async fn get(&self, id: &Id) -> Result<SkillReview> {
        let row = sqlx::query("SELECT * FROM skill_reviews WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get skill review"))?
            .ok_or_else(|| Error::NotFound(format!("skill review '{id}'")))?;
        row_to_review(&row)
    }

    /// All reviews for a workspace, newest first.
    pub async fn list(&self, workspace_id: &Id) -> Result<Vec<SkillReview>> {
        let rows = sqlx::query(
            "SELECT * FROM skill_reviews WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list skill reviews"))?;
        rows.iter().map(row_to_review).collect()
    }

    /// Seed the whole agents array (one per provider + trailing summarizer).
    pub async fn set_agents(&self, id: &Id, agents: &[SkillReviewAgent]) -> Result<()> {
        let json = serde_json::to_string(agents)
            .map_err(|e| Error::Internal(format!("serialize agents: {e}")))?;
        self.touch_json(id, "agents_json", &json, false).await
    }

    /// Atomically replace a single agent's row (element `index`) — see the
    /// [`crate::reviews::ReviewsRepo::set_agent_at`] rationale.
    pub async fn set_agent_at(
        &self,
        id: &Id,
        index: usize,
        agent: &SkillReviewAgent,
    ) -> Result<()> {
        let elem = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize agent: {e}")))?;
        let path = format!("$[{index}]");
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE skill_reviews
               SET agents_json = json_replace(agents_json, ?, json(?)), updated_at = ?
             WHERE id = ?",
        )
        .bind(&path)
        .bind(&elem)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set skill review agent"))?;
        Ok(())
    }

    /// Store the deterministic static report.
    pub async fn set_static(&self, id: &Id, report: &SkillStaticReport) -> Result<()> {
        let json = serde_json::to_string(report)
            .map_err(|e| Error::Internal(format!("serialize static: {e}")))?;
        self.touch_json(id, "static_json", &json, false).await
    }

    /// Store the summarizer's aggregated report.
    pub async fn set_summary(&self, id: &Id, summary: &SkillReviewSummary) -> Result<()> {
        let json = serde_json::to_string(summary)
            .map_err(|e| Error::Internal(format!("serialize summary: {e}")))?;
        self.touch_json(id, "summary_json", &json, false).await
    }

    /// Store the apply-fixes agent row (whole-row replace; only one fixer runs
    /// at a time so there is no concurrent-index concern here).
    pub async fn set_fix(&self, id: &Id, agent: &SkillReviewAgent) -> Result<()> {
        let json = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize fix agent: {e}")))?;
        self.touch_json(id, "fix_json", &json, false).await
    }

    /// Admit one fixer. This is separate from progress writes after admission.
    pub async fn claim_fix(&self, id: &Id, agent: &SkillReviewAgent) -> Result<bool> {
        self.claim_fix_checked(id, agent, None).await
    }

    async fn claim_fix_checked(
        &self,
        id: &Id,
        agent: &SkillReviewAgent,
        expected_updated_at: Option<String>,
    ) -> Result<bool> {
        let json = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize fix agent: {e}")))?;
        let result = sqlx::query(
            "UPDATE skill_reviews SET fix_json = ?, updated_at = ? WHERE id = ? AND status = 'done' \
             AND (fix_json IS NULL OR CASE WHEN json_valid(fix_json) \
             THEN json_extract(fix_json, '$.status') IN ('done', 'error', 'cancelled') ELSE 0 END) \
             AND (? IS NULL OR updated_at = ?)",
        )
        .bind(json).bind(fmt(Utc::now())).bind(id)
        .bind(&expected_updated_at).bind(&expected_updated_at)
        .execute(&self.pool).await.map_err(dberr("claim skill fixer"))?;
        Ok(result.rows_affected() == 1)
    }

    /// Admit Apply only for the exact review snapshot used to build its prompt.
    pub async fn claim_fix_snapshot(
        &self,
        review: &SkillReview,
        agent: &SkillReviewAgent,
    ) -> Result<bool> {
        self.claim_fix_checked(&review.id, agent, Some(fmt(review.updated_at)))
            .await
    }

    /// Set the terminal status (+ optional error message).
    pub async fn set_status(&self, id: &Id, status: &str, error: Option<&str>) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE skill_reviews SET status = ?, error = ?, updated_at = ? WHERE id = ? AND (status != 'cancelled' OR ? = 'cancelled')")
            .bind(status)
            .bind(error)
            .bind(&now)
            .bind(id)
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(dberr("set skill review status"))?;
        Ok(())
    }

    /// Delete a review row.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM skill_reviews WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete skill review"))?;
        Ok(())
    }

    /// Write a JSON column (bumping `updated_at`). `_raw` is reserved for future
    /// use; the value is always a serialized JSON string here.
    async fn touch_json(&self, id: &Id, column: &str, json: &str, _raw: bool) -> Result<()> {
        let now = fmt(Utc::now());
        // `column` is a fixed internal literal, never user input.
        let sql = format!("UPDATE skill_reviews SET {column} = ?, updated_at = ? WHERE id = ?");
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(json)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update skill review json"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{SkillFinding, SkillScoreRow};

    async fn pool() -> DbPool {
        let pool = DbPool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn recovery_terminalizes_orphan_review_and_fixer_without_losing_results() {
        let repo = SkillReviewsRepo::new(pool().await);
        let running = repo
            .create(&"ws1".into(), "fixture", "library", "agents", "", None)
            .await
            .unwrap();
        let agent = SkillReviewAgent {
            name: "reviewer".into(),
            provider: "fixture".into(),
            model: String::new(),
            status: "running".into(),
            note: String::new(),
            session_id: Some("old-session".into()),
            findings: vec![],
        };
        repo.set_agents(&running.id, std::slice::from_ref(&agent))
            .await
            .unwrap();
        let done = repo
            .create(&"ws1".into(), "finished", "library", "static", "", None)
            .await
            .unwrap();
        repo.set_status(&done.id, "done", None).await.unwrap();
        repo.set_fix(&done.id, &agent).await.unwrap();
        assert_eq!(repo.fail_running().await.unwrap(), 2);
        let recovered = repo.get(&running.id).await.unwrap();
        assert_eq!(recovered.status, "error");
        assert_eq!(recovered.agents[0].status, "error");
        assert_eq!(
            recovered.agents[0].session_id.as_deref(),
            Some("old-session")
        );
        assert!(recovered.error.unwrap().contains("restart"));
        let finished = repo.get(&done.id).await.unwrap();
        assert_eq!(finished.status, "done");
        assert_eq!(finished.fix_agent.unwrap().status, "error");
        assert!(repo.claim_fix(&done.id, &agent).await.unwrap());
    }

    #[tokio::test]
    async fn fixer_admission_is_atomic_and_rejects_cancelled_or_missing_reviews() {
        let repo = SkillReviewsRepo::new(pool().await);
        let rev = repo
            .create(&"ws1".into(), "fixture", "library", "static", "", None)
            .await
            .unwrap();
        let pending = SkillReviewAgent {
            name: "fixer".into(),
            provider: "fixture".into(),
            model: String::new(),
            status: "pending".into(),
            note: String::new(),
            session_id: None,
            findings: vec![],
        };
        assert!(
            !repo.claim_fix(&rev.id, &pending).await.unwrap(),
            "running review cannot admit a fixer"
        );
        repo.set_status(&rev.id, "done", None).await.unwrap();
        let (a, b) = tokio::join!(
            repo.claim_fix(&rev.id, &pending),
            repo.claim_fix(&rev.id, &pending)
        );
        assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
        let mut done = pending.clone();
        done.status = "done".into();
        repo.set_fix(&rev.id, &done).await.unwrap();
        assert!(
            repo.claim_fix(&rev.id, &pending).await.unwrap(),
            "completed fixer may be retried"
        );
        repo.set_status(&rev.id, "cancelled", None).await.unwrap();
        repo.set_fix(&rev.id, &done).await.unwrap();
        assert!(!repo.claim_fix(&rev.id, &pending).await.unwrap());
        repo.delete(&rev.id).await.unwrap();
        assert!(!repo.claim_fix(&rev.id, &pending).await.unwrap());
    }

    #[tokio::test]
    async fn retry_admission_invalidates_summary_and_excludes_apply_until_publication() {
        let repo = SkillReviewsRepo::new(pool().await);
        let review = repo
            .create(&"ws1".into(), "fixture", "library", "agents", "", None)
            .await
            .unwrap();
        let pending = SkillReviewAgent {
            name: "reviewer".into(),
            provider: "fixture".into(),
            model: String::new(),
            status: "pending".into(),
            note: String::new(),
            session_id: None,
            findings: vec![],
        };
        repo.set_agents(&review.id, std::slice::from_ref(&pending))
            .await
            .unwrap();
        let summary = SkillReviewSummary {
            verdict: "Ready".into(),
            average_score: 5.0,
            scorecard: vec![],
            findings: vec![],
            patch_plan: vec!["old instruction".into()],
        };
        repo.set_summary(&review.id, &summary).await.unwrap();
        repo.set_status(&review.id, "done", None).await.unwrap();
        let (a, b) = tokio::join!(
            repo.claim_retry(&review.id, 0, &pending),
            repo.claim_retry(&review.id, 0, &pending)
        );
        assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
        let claimed = repo.get(&review.id).await.unwrap();
        assert_eq!(claimed.status, "running");
        assert!(claimed.summary.is_none());
        assert!(!repo.claim_fix(&review.id, &pending).await.unwrap());
        repo.set_status(&review.id, "done", None).await.unwrap();
        assert!(repo.claim_fix(&review.id, &pending).await.unwrap());
        assert!(!repo.claim_retry(&review.id, 0, &pending).await.unwrap());
        repo.set_status(&review.id, "cancelled", None)
            .await
            .unwrap();
        repo.set_status(&review.id, "done", None).await.unwrap();
        assert_eq!(repo.get(&review.id).await.unwrap().status, "cancelled");
        assert!(!repo.claim_retry(&review.id, 0, &pending).await.unwrap());
    }

    #[tokio::test]
    async fn stale_apply_snapshot_cannot_claim_after_retry_publishes_new_summary() {
        let repo = SkillReviewsRepo::new(pool().await);
        let review = repo
            .create(&"ws1".into(), "fixture", "library", "agents", "", None)
            .await
            .unwrap();
        let pending = SkillReviewAgent {
            name: "reviewer".into(),
            provider: "fixture".into(),
            model: String::new(),
            status: "pending".into(),
            note: String::new(),
            session_id: None,
            findings: vec![],
        };
        repo.set_agents(&review.id, std::slice::from_ref(&pending))
            .await
            .unwrap();
        let mut summary = SkillReviewSummary {
            verdict: "Ready".into(),
            average_score: 5.0,
            scorecard: vec![],
            findings: vec![],
            patch_plan: vec!["obsolete instruction".into()],
        };
        repo.set_summary(&review.id, &summary).await.unwrap();
        repo.set_status(&review.id, "done", None).await.unwrap();
        let apply_snapshot = repo.get(&review.id).await.unwrap();
        assert!(repo.claim_retry(&review.id, 0, &pending).await.unwrap());
        summary.patch_plan = vec!["new instruction".into()];
        repo.set_summary(&review.id, &summary).await.unwrap();
        repo.set_status(&review.id, "done", None).await.unwrap();
        assert!(!repo
            .claim_fix_snapshot(&apply_snapshot, &pending)
            .await
            .unwrap());
        let current = repo.get(&review.id).await.unwrap();
        assert!(
            current.fix_agent.is_none(),
            "stale admission must not leave a pending fixer"
        );
        assert_eq!(
            current.summary.as_ref().unwrap().patch_plan,
            vec!["new instruction"]
        );
        assert!(repo.claim_fix_snapshot(&current, &pending).await.unwrap());
    }

    #[tokio::test]
    async fn round_trip_static_agents_summary() {
        let repo = SkillReviewsRepo::new(pool().await);
        let ws: Id = "ws1".into();
        let rev = repo
            .create(
                &ws,
                "grill",
                "bundled",
                "agents",
                "focus on trigger precision",
                Some("root"),
            )
            .await
            .unwrap();
        assert_eq!(rev.status, "running");
        assert_eq!(rev.skill_source, "bundled");
        assert_eq!(rev.instructions, "focus on trigger precision");
        assert!(rev.fix_agent.is_none());

        // Seed two agent rows + summarizer.
        let agents = vec![
            SkillReviewAgent {
                name: "claude".into(),
                provider: "claude".into(),
                model: "".into(),
                status: "pending".into(),
                note: "".into(),
                session_id: None,
                findings: vec![],
            },
            SkillReviewAgent {
                name: "summarizer".into(),
                provider: "claude".into(),
                model: "".into(),
                status: "pending".into(),
                note: "".into(),
                session_id: None,
                findings: vec![],
            },
        ];
        repo.set_agents(&rev.id, &agents).await.unwrap();
        // Update index 0 atomically.
        let mut a0 = agents[0].clone();
        a0.status = "done".into();
        a0.session_id = Some("sess-1".into());
        a0.findings = vec![SkillFinding {
            severity: "High".into(),
            code: "NO_EXAMPLES".into(),
            title: "no examples".into(),
            evidence: "SKILL.md".into(),
            why: "w".into(),
            fix: "f".into(),
        }];
        repo.set_agent_at(&rev.id, 0, &a0).await.unwrap();

        let stat = SkillStaticReport {
            verdict: "Ready with fixes".into(),
            average_score: 4.2,
            scorecard: vec![SkillScoreRow {
                area: "examples".into(),
                score: 3,
                notes: "n".into(),
            }],
            findings: vec![],
        };
        repo.set_static(&rev.id, &stat).await.unwrap();
        let sum = SkillReviewSummary {
            verdict: "Ready with fixes".into(),
            average_score: 4.2,
            scorecard: vec![],
            findings: vec![],
            patch_plan: vec!["add examples".into()],
        };
        repo.set_summary(&rev.id, &sum).await.unwrap();
        repo.set_status(&rev.id, "done", None).await.unwrap();

        // Apply-fixes agent round-trip.
        let fixer = SkillReviewAgent {
            name: "fixer".into(),
            provider: "claude".into(),
            model: "".into(),
            status: "running".into(),
            note: "".into(),
            session_id: Some("sess-fix".into()),
            findings: vec![],
        };
        repo.set_fix(&rev.id, &fixer).await.unwrap();

        let got = repo.get(&rev.id).await.unwrap();
        assert_eq!(got.status, "done");
        assert_eq!(got.instructions, "focus on trigger precision");
        let fx = got.fix_agent.as_ref().unwrap();
        assert_eq!(fx.status, "running");
        assert_eq!(fx.session_id.as_deref(), Some("sess-fix"));
        assert_eq!(got.agents.len(), 2);
        assert_eq!(got.agents[0].status, "done");
        assert_eq!(got.agents[0].session_id.as_deref(), Some("sess-1"));
        assert_eq!(got.agents[0].findings.len(), 1);
        assert_eq!(got.agents[1].status, "pending"); // untouched
        assert_eq!(got.static_report.unwrap().verdict, "Ready with fixes");
        assert_eq!(
            got.summary.unwrap().patch_plan,
            vec!["add examples".to_string()]
        );

        let list = repo.list(&ws).await.unwrap();
        assert_eq!(list.len(), 1);

        repo.delete(&rev.id).await.unwrap();
        assert!(repo.get(&rev.id).await.is_err());
    }
}

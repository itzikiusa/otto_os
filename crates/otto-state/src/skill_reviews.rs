//! Persistence for Skills Lab reviews (`skill_reviews`).
//!
//! One row per review of one skill package. The live per-agent state lives in
//! `agents_json` and is updated one array index at a time via [`SkillReviewsRepo::set_agent_at`]
//! (mirrors [`crate::reviews::ReviewsRepo::set_agent_at`]) so concurrent provider
//! agents never clobber each other's rows. The deterministic static report and
//! the summarizer's aggregate ride in `static_json` / `summary_json`.

use chrono::Utc;
use otto_core::domain::{SkillReview, SkillReviewAgent, SkillReviewSummary, SkillStaticReport};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct SkillReviewsRepo {
    pool: SqlitePool,
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
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
    pub async fn set_agent_at(&self, id: &Id, index: usize, agent: &SkillReviewAgent) -> Result<()> {
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

    /// Set the terminal status (+ optional error message).
    pub async fn set_status(&self, id: &Id, status: &str, error: Option<&str>) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE skill_reviews SET status = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(&now)
            .bind(id)
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
        sqlx::query(&sql)
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

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn round_trip_static_agents_summary() {
        let repo = SkillReviewsRepo::new(pool().await);
        let ws: Id = "ws1".into();
        let rev = repo
            .create(&ws, "grill", "bundled", "agents", "focus on trigger precision", Some("root"))
            .await
            .unwrap();
        assert_eq!(rev.status, "running");
        assert_eq!(rev.skill_source, "bundled");
        assert_eq!(rev.instructions, "focus on trigger precision");
        assert!(rev.fix_agent.is_none());

        // Seed two agent rows + summarizer.
        let agents = vec![
            SkillReviewAgent { name: "claude".into(), provider: "claude".into(), model: "".into(), status: "pending".into(), note: "".into(), session_id: None, findings: vec![] },
            SkillReviewAgent { name: "summarizer".into(), provider: "claude".into(), model: "".into(), status: "pending".into(), note: "".into(), session_id: None, findings: vec![] },
        ];
        repo.set_agents(&rev.id, &agents).await.unwrap();
        // Update index 0 atomically.
        let mut a0 = agents[0].clone();
        a0.status = "done".into();
        a0.session_id = Some("sess-1".into());
        a0.findings = vec![SkillFinding { severity: "High".into(), code: "NO_EXAMPLES".into(), title: "no examples".into(), evidence: "SKILL.md".into(), why: "w".into(), fix: "f".into() }];
        repo.set_agent_at(&rev.id, 0, &a0).await.unwrap();

        let stat = SkillStaticReport {
            verdict: "Ready with fixes".into(),
            average_score: 4.2,
            scorecard: vec![SkillScoreRow { area: "examples".into(), score: 3, notes: "n".into() }],
            findings: vec![],
        };
        repo.set_static(&rev.id, &stat).await.unwrap();
        let sum = SkillReviewSummary { verdict: "Ready with fixes".into(), average_score: 4.2, scorecard: vec![], findings: vec![], patch_plan: vec!["add examples".into()] };
        repo.set_summary(&rev.id, &sum).await.unwrap();
        repo.set_status(&rev.id, "done", None).await.unwrap();

        // Apply-fixes agent round-trip.
        let fixer = SkillReviewAgent { name: "fixer".into(), provider: "claude".into(), model: "".into(), status: "running".into(), note: "".into(), session_id: Some("sess-fix".into()), findings: vec![] };
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
        assert_eq!(got.summary.unwrap().patch_plan, vec!["add examples".to_string()]);

        let list = repo.list(&ws).await.unwrap();
        assert_eq!(list.len(), 1);

        repo.delete(&rev.id).await.unwrap();
        assert!(repo.get(&rev.id).await.is_err());
    }
}

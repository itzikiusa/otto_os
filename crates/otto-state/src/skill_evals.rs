//! Persistence for Skills Evaluator runs and their iterations.
//!
//! A run (`skill_evals`) owns one or more iterations (`skill_eval_iterations`).
//! Each iteration's per-validation live agent state lives in its `agents_json`
//! column and is updated one array index at a time via `set_iter_agent_at`
//! (mirrors [`crate::reviews::ReviewsRepo::set_agent_at`]) so concurrent
//! validators never clobber each other's rows.

use chrono::Utc;
use otto_core::domain::{
    EvalIteration, EvalScore, EvalValidationState, SkillEval, SkillEvalStatus,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct SkillEvalsRepo {
    pool: SqlitePool,
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn row_to_iteration(r: &sqlx::sqlite::SqliteRow) -> Result<EvalIteration> {
    let agents_raw: String = r.try_get("agents_json").unwrap_or_default();
    let agents: Vec<EvalValidationState> = serde_json::from_str(&agents_raw).unwrap_or_default();
    let base_iter: Option<i64> = r.get("base_iter");
    let scoring: Option<EvalScore> = r
        .try_get::<Option<String>, _>("scoring_json")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok());
    let human_rating: Option<u8> = r
        .try_get::<Option<i64>, _>("human_rating")
        .ok()
        .flatten()
        .map(|v| v.clamp(0, 5) as u8);
    Ok(EvalIteration {
        id: r.get("id"),
        eval_id: r.get("eval_id"),
        iter: r.get::<i64, _>("iter") as u32,
        base_iter: base_iter.map(|v| v as u32),
        skill_name: r.get("skill_name"),
        skill_before: r.get("skill_before"),
        skill_after: r.get("skill_after"),
        impl_provider: r.get("impl_provider"),
        impl_session_id: r.get("impl_session_id"),
        impl_summary: r.get("impl_summary"),
        worktree_path: r.get("worktree_path"),
        status: r.get("status"),
        note: r.get("note"),
        score: r.get("score"),
        agents,
        improvement_summary: r.get("improvement_summary"),
        skill_diff: r.get("skill_diff"),
        scoring,
        proof_pack_id: r.try_get("proof_pack_id").ok().flatten(),
        human_rating,
        human_note: r.try_get("human_note").unwrap_or_default(),
        human_rater: r.try_get("human_rater").unwrap_or_default(),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_eval(r: &sqlx::sqlite::SqliteRow, iterations: Vec<EvalIteration>) -> Result<SkillEval> {
    let best_iter: Option<i64> = r.get("best_iteration");
    let config_raw: String = r.try_get("config_json").unwrap_or_default();
    let config: serde_json::Value = serde_json::from_str(&config_raw).unwrap_or(serde_json::Value::Null);
    Ok(SkillEval {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        source_skill: r.get("source_skill"),
        task: r.get("task"),
        impl_cli: r.get("impl_cli"),
        target_iterations: r.get::<i64, _>("target_iterations") as u32,
        status: SkillEvalStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad skill eval status".into()))?,
        error: r.get("error"),
        summary: r.get("summary"),
        best_iteration: best_iter.map(|v| v as u32),
        best_score: r.get("best_score"),
        iterations,
        config,
        mode: r.try_get("mode").unwrap_or_else(|_| "generate".into()),
        golden_task_id: r.try_get("golden_task_id").ok().flatten(),
        matrix_id: r.try_get("matrix_id").ok().flatten(),
        dim_provider: r.try_get("dim_provider").ok().flatten(),
        dim_skill: r.try_get("dim_skill").ok().flatten(),
        dim_prompt: r.try_get("dim_prompt").ok().flatten(),
        composite_score: r.try_get("composite_score").ok().flatten(),
        promoted: r.try_get::<i64, _>("promoted").map(|v| v != 0).unwrap_or(false),
        promoted_at: r.try_get("promoted_at").ok().flatten(),
        promoted_by: r.try_get("promoted_by").ok().flatten(),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

impl SkillEvalsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new generate-mode run in status "running".
    #[allow(clippy::too_many_arguments)]
    pub async fn create_eval(
        &self,
        workspace_id: &Id,
        source_skill: &str,
        task: &str,
        impl_cli: &str,
        target_iterations: u32,
        config: &serde_json::Value,
    ) -> Result<SkillEval> {
        self.create_eval_ex(
            workspace_id,
            source_skill,
            task,
            impl_cli,
            target_iterations,
            config,
            "generate",
            None,
            None,
            None,
        )
        .await
    }

    /// Create a run with full eval-lab metadata (mode, golden link, matrix cell
    /// dimensions). `dims` is `(provider, skill, prompt)` for a matrix cell.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_eval_ex(
        &self,
        workspace_id: &Id,
        source_skill: &str,
        task: &str,
        impl_cli: &str,
        target_iterations: u32,
        config: &serde_json::Value,
        mode: &str,
        golden_task_id: Option<&str>,
        matrix_id: Option<&str>,
        dims: Option<(&str, &str, &str)>,
    ) -> Result<SkillEval> {
        let id = new_id();
        let now = fmt(Utc::now());
        let config_json = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string());
        let (dp, ds, dpr) = match dims {
            Some((p, s, pr)) => (Some(p), Some(s), Some(pr)),
            None => (None, None, None),
        };
        sqlx::query(
            "INSERT INTO skill_evals
                (id, workspace_id, source_skill, task, impl_cli, target_iterations, status,
                 config_json, mode, golden_task_id, matrix_id, dim_provider, dim_skill, dim_prompt, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'running', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(source_skill)
        .bind(task)
        .bind(impl_cli)
        .bind(target_iterations as i64)
        .bind(&config_json)
        .bind(mode)
        .bind(golden_task_id)
        .bind(matrix_id)
        .bind(dp)
        .bind(ds)
        .bind(dpr)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create skill eval"))?;
        self.get_eval(&id).await
    }

    /// Delete a run and its iterations (used by the UI's "delete run").
    pub async fn delete(&self, id: &Id) -> Result<()> {
        // Explicit child delete first (independent of the foreign_keys pragma).
        sqlx::query("DELETE FROM skill_eval_iterations WHERE eval_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete skill eval iterations"))?;
        sqlx::query("DELETE FROM skill_evals WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete skill eval"))?;
        Ok(())
    }

    /// Update the run's status (and optional error).
    pub async fn set_status(
        &self,
        id: &Id,
        status: SkillEvalStatus,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE skill_evals SET status = ?, error = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set skill eval status"))?;
        Ok(())
    }

    /// Record the final summary and the winning iteration.
    pub async fn set_summary(
        &self,
        id: &Id,
        summary: &str,
        best_iteration: Option<u32>,
        best_score: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE skill_evals SET summary = ?, best_iteration = ?, best_score = ? WHERE id = ?",
        )
        .bind(summary)
        .bind(best_iteration.map(|v| v as i64))
        .bind(best_score)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set skill eval summary"))?;
        Ok(())
    }

    /// Fail every run still marked `running` (daemon-startup recovery): the
    /// background task dies with the process, so an orphaned row would poll
    /// forever in the UI. Returns the number of rows updated.
    pub async fn fail_running(&self, error: &str) -> Result<u64> {
        let res =
            sqlx::query("UPDATE skill_evals SET status = 'error', error = ? WHERE status = 'running'")
                .bind(error)
                .execute(&self.pool)
                .await
                .map_err(dberr("fail running skill evals"))?;
        Ok(res.rows_affected())
    }

    /// Insert a new pending iteration with the given seeded validation agents.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_iteration(
        &self,
        eval_id: &Id,
        iter: u32,
        base_iter: Option<u32>,
        skill_name: &str,
        skill_before: &str,
        impl_provider: &str,
        agents: &[EvalValidationState],
    ) -> Result<EvalIteration> {
        let id = new_id();
        let now = fmt(Utc::now());
        let agents_json = serde_json::to_string(agents)
            .map_err(|e| Error::Internal(format!("serialize eval agents: {e}")))?;
        sqlx::query(
            "INSERT INTO skill_eval_iterations
                (id, eval_id, iter, base_iter, skill_name, skill_before, impl_provider,
                 status, agents_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(eval_id)
        .bind(iter as i64)
        .bind(base_iter.map(|v| v as i64))
        .bind(skill_name)
        .bind(skill_before)
        .bind(impl_provider)
        .bind(&agents_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add skill eval iteration"))?;
        self.get_iteration(&id).await
    }

    /// Overwrite the whole validation-agents array for an iteration.
    pub async fn set_iter_agents(
        &self,
        iter_id: &Id,
        agents: &[EvalValidationState],
    ) -> Result<()> {
        let json = serde_json::to_string(agents)
            .map_err(|e| Error::Internal(format!("serialize eval agents: {e}")))?;
        sqlx::query("UPDATE skill_eval_iterations SET agents_json = ? WHERE id = ?")
            .bind(&json)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set eval agents"))?;
        Ok(())
    }

    /// Atomically replace a single validation agent (element `index`) without
    /// touching the others (see [`crate::reviews::ReviewsRepo::set_agent_at`]).
    pub async fn set_iter_agent_at(
        &self,
        iter_id: &Id,
        index: usize,
        agent: &EvalValidationState,
    ) -> Result<()> {
        let elem = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize eval agent: {e}")))?;
        let path = format!("$[{index}]");
        sqlx::query(
            "UPDATE skill_eval_iterations SET agents_json = json_replace(agents_json, ?, json(?)) WHERE id = ?",
        )
        .bind(&path)
        .bind(&elem)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval agent"))?;
        Ok(())
    }

    /// Update an iteration's status + note.
    pub async fn set_iter_status(&self, iter_id: &Id, status: &str, note: &str) -> Result<()> {
        sqlx::query("UPDATE skill_eval_iterations SET status = ?, note = ? WHERE id = ?")
            .bind(status)
            .bind(note)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set eval iteration status"))?;
        Ok(())
    }

    /// Record the implementation outcome for an iteration.
    pub async fn set_iter_impl(
        &self,
        iter_id: &Id,
        impl_session_id: Option<&str>,
        impl_summary: &str,
        worktree_path: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE skill_eval_iterations
                SET impl_session_id = ?, impl_summary = ?, worktree_path = ? WHERE id = ?",
        )
        .bind(impl_session_id)
        .bind(impl_summary)
        .bind(worktree_path)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval iteration impl"))?;
        Ok(())
    }

    /// Record an iteration's aggregate score.
    pub async fn set_iter_score(&self, iter_id: &Id, score: f64) -> Result<()> {
        sqlx::query("UPDATE skill_eval_iterations SET score = ? WHERE id = ?")
            .bind(score)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set eval iteration score"))?;
        Ok(())
    }

    /// Record the improvement the improver produced (seeds the next iteration).
    pub async fn set_iter_improvement(
        &self,
        iter_id: &Id,
        skill_after: Option<&str>,
        improvement_summary: &str,
        skill_diff: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE skill_eval_iterations
                SET skill_after = ?, improvement_summary = ?, skill_diff = ? WHERE id = ?",
        )
        .bind(skill_after)
        .bind(improvement_summary)
        .bind(skill_diff)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval iteration improvement"))?;
        Ok(())
    }

    /// Persist an iteration's multi-signal score + its proof pack id.
    pub async fn set_iter_scoring(
        &self,
        iter_id: &Id,
        scoring: &EvalScore,
        proof_pack_id: Option<&str>,
    ) -> Result<()> {
        let json = serde_json::to_string(scoring)
            .map_err(|e| Error::Internal(format!("serialize eval scoring: {e}")))?;
        sqlx::query(
            "UPDATE skill_eval_iterations SET scoring_json = ?, proof_pack_id = ? WHERE id = ?",
        )
        .bind(&json)
        .bind(proof_pack_id)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval iteration scoring"))?;
        Ok(())
    }

    /// Record a human rating (0–5) for an iteration.
    pub async fn set_iter_human(
        &self,
        iter_id: &Id,
        rating: u8,
        note: &str,
        rater: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE skill_eval_iterations SET human_rating = ?, human_note = ?, human_rater = ? WHERE id = ?",
        )
        .bind(rating.min(5) as i64)
        .bind(note)
        .bind(rater)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval iteration human rating"))?;
        Ok(())
    }

    /// Record the run's headline composite score (best iteration).
    pub async fn set_eval_composite(&self, eval_id: &Id, composite: f64) -> Result<()> {
        sqlx::query("UPDATE skill_evals SET composite_score = ? WHERE id = ?")
            .bind(composite)
            .bind(eval_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set eval composite"))?;
        Ok(())
    }

    /// Mark a run as promoted to the library.
    pub async fn set_promoted(&self, eval_id: &Id, by: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE skill_evals SET promoted = 1, promoted_at = ?, promoted_by = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(by)
        .bind(eval_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set eval promoted"))?;
        Ok(())
    }

    /// All cell runs of a matrix (by matrix_id), oldest first.
    pub async fn list_for_matrix(&self, matrix_id: &str) -> Result<Vec<SkillEval>> {
        let rows = sqlx::query(
            "SELECT * FROM skill_evals WHERE matrix_id = ? ORDER BY created_at",
        )
        .bind(matrix_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list matrix evals"))?;
        let mut evals = Vec::with_capacity(rows.len());
        for r in &rows {
            let id: String = r.get("id");
            let iterations = self.iterations_for_eval(&id).await?;
            evals.push(row_to_eval(r, iterations)?);
        }
        Ok(evals)
    }

    /// Fetch a single iteration.
    pub async fn get_iteration(&self, iter_id: &Id) -> Result<EvalIteration> {
        let row = sqlx::query("SELECT * FROM skill_eval_iterations WHERE id = ?")
            .bind(iter_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("skill eval iteration"))?;
        row_to_iteration(&row)
    }

    /// Fetch a run by id with all its iterations (oldest first).
    pub async fn get_eval(&self, id: &Id) -> Result<SkillEval> {
        let row = sqlx::query("SELECT * FROM skill_evals WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("skill eval"))?;
        let iterations = self.iterations_for_eval(id).await?;
        row_to_eval(&row, iterations)
    }

    /// All runs for a workspace, newest first, each with its iterations.
    pub async fn list_for_workspace(&self, workspace_id: &Id) -> Result<Vec<SkillEval>> {
        let rows = sqlx::query(
            "SELECT * FROM skill_evals WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list skill evals"))?;

        let mut evals = Vec::with_capacity(rows.len());
        for r in &rows {
            let id: String = r.get("id");
            let iterations = self.iterations_for_eval(&id).await?;
            evals.push(row_to_eval(r, iterations)?);
        }
        Ok(evals)
    }

    // -- private helpers ----------------------------------------------------

    async fn iterations_for_eval(&self, eval_id: &Id) -> Result<Vec<EvalIteration>> {
        let rows = sqlx::query(
            "SELECT * FROM skill_eval_iterations WHERE eval_id = ? ORDER BY iter",
        )
        .bind(eval_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("skill eval iterations"))?;
        rows.iter().map(row_to_iteration).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::EvalFinding;

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

    fn agent(validation: &str, status: &str) -> EvalValidationState {
        EvalValidationState {
            validation: validation.into(),
            name: validation.into(),
            provider: "claude".into(),
            model: String::new(),
            status: status.into(),
            note: String::new(),
            passed: false,
            score: 0.0,
            session_id: None,
            findings: Vec::new(),
        }
    }

    #[tokio::test]
    async fn run_iteration_round_trip_and_atomic_agent_update() {
        let pool = mem_pool().await;
        let repo = SkillEvalsRepo::new(pool.clone());

        let eval = repo
            .create_eval(&"ws1".into(), "golang-feature", "add X", "claude", 2, &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(eval.status, SkillEvalStatus::Running);
        assert!(eval.iterations.is_empty());

        let it = repo
            .add_iteration(
                &eval.id,
                1,
                None,
                "golang-feature-run-ab-iter1",
                "---\nname: x\n---\nbody",
                "claude",
                &[agent("logs", "pending"), agent("docs", "pending")],
            )
            .await
            .unwrap();
        assert_eq!(it.iter, 1);
        assert_eq!(it.agents.len(), 2);

        // Each validation flips independently; one update must not clobber the other.
        let mut a0 = agent("logs", "done");
        a0.session_id = Some("s0".into());
        a0.passed = true;
        a0.score = 90.0;
        a0.findings = vec![EvalFinding {
            severity: "warn".into(),
            issue: "missing context in log".into(),
            suggestion: "use logger.InfoF(ctx, ...)".into(),
            location: Some("main.go:10".into()),
        }];
        repo.set_iter_agent_at(&it.id, 0, &a0).await.unwrap();

        let mut a1 = agent("docs", "running");
        a1.session_id = Some("s1".into());
        repo.set_iter_agent_at(&it.id, 1, &a1).await.unwrap();
        // Re-write index 0; index 1 must survive.
        repo.set_iter_agent_at(&it.id, 0, &a0).await.unwrap();

        repo.set_iter_score(&it.id, 85.5).await.unwrap();
        repo.set_iter_status(&it.id, "done", "scored 85.5").await.unwrap();
        repo.set_summary(&eval.id, "best is iter 1", Some(1), Some(85.5))
            .await
            .unwrap();

        let loaded = repo.get_eval(&eval.id).await.unwrap();
        assert_eq!(loaded.iterations.len(), 1);
        let it = &loaded.iterations[0];
        assert_eq!(it.agents.len(), 2);
        assert_eq!(it.agents[0].status, "done");
        assert_eq!(it.agents[0].session_id.as_deref(), Some("s0"));
        assert_eq!(it.agents[0].findings.len(), 1);
        assert_eq!(it.agents[0].findings[0].suggestion, "use logger.InfoF(ctx, ...)");
        assert_eq!(it.agents[1].status, "running");
        assert_eq!(it.score, 85.5);
        assert_eq!(loaded.best_iteration, Some(1));

        let list = repo.list_for_workspace(&"ws1".into()).await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn scoring_human_and_matrix_dims_round_trip() {
        let pool = mem_pool().await;
        let repo = SkillEvalsRepo::new(pool.clone());

        let eval = repo
            .create_eval_ex(
                &"ws1".into(),
                "golang-feature",
                "add X",
                "claude",
                1,
                &serde_json::json!({}),
                "score_only",
                Some("gt1"),
                Some("mx1"),
                Some(("claude", "golang-feature", "task-A")),
            )
            .await
            .unwrap();
        assert_eq!(eval.mode, "score_only");
        assert_eq!(eval.golden_task_id.as_deref(), Some("gt1"));
        assert_eq!(eval.matrix_id.as_deref(), Some("mx1"));
        assert_eq!(eval.dim_provider.as_deref(), Some("claude"));

        let it = repo
            .add_iteration(&eval.id, 1, None, "x", "body", "claude", &[])
            .await
            .unwrap();

        let mut sc = EvalScore::default();
        sc.tests.ran = true;
        sc.tests.score = 100.0;
        sc.composite = 92.5;
        sc.proof_status = "passed".into();
        sc.done_score = 88;
        repo.set_iter_scoring(&it.id, &sc, Some("pp1")).await.unwrap();
        repo.set_iter_human(&it.id, 4, "looks good", "root").await.unwrap();
        repo.set_eval_composite(&eval.id, 92.5).await.unwrap();
        repo.set_promoted(&eval.id, "root").await.unwrap();

        let loaded = repo.get_eval(&eval.id).await.unwrap();
        assert_eq!(loaded.composite_score, Some(92.5));
        assert!(loaded.promoted);
        assert_eq!(loaded.promoted_by.as_deref(), Some("root"));
        let it = &loaded.iterations[0];
        let scoring = it.scoring.as_ref().expect("scoring persisted");
        assert_eq!(scoring.proof_status, "passed");
        assert_eq!(scoring.done_score, 88);
        assert!((scoring.composite - 92.5).abs() < 1e-9);
        assert_eq!(it.proof_pack_id.as_deref(), Some("pp1"));
        assert_eq!(it.human_rating, Some(4));
        assert_eq!(it.human_note, "looks good");

        // Matrix listing finds the cell.
        let cells = repo.list_for_matrix("mx1").await.unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].id, eval.id);
    }
}

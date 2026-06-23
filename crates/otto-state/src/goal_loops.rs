//! Persistence for Goal Loops — runs and their iterations.
//!
//! Mirrors the [`ReviewsRepo`](crate::reviews::ReviewsRepo) patterns: JSON-blob
//! columns for the structured config/state, `json_replace` for atomic per-agent
//! live-state updates, and a `fail_running` orphan sweep on boot (but ours
//! RETURNs the rows so the daemon can also remove the loops' worktrees/sessions).

use chrono::{DateTime, Utc};
use otto_core::domain::{
    GoalLoop, GoalLoopAgentCfg, GoalLoopConfig, GoalLoopDefinition, GoalLoopDetail,
    GoalLoopEvaluation, GoalLoopIteration, GoalLoopLimits, GoalLoopPhase, GoalLoopStatus,
    LoopAgentState,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct GoalLoopsRepo {
    pool: SqlitePool,
}

/// Fields needed to create a new loop (status starts `draft`).
pub struct NewGoalLoop {
    pub workspace_id: Id,
    pub name: String,
    pub repo_path: String,
    pub definition: GoalLoopDefinition,
    pub limits: GoalLoopLimits,
    pub config: GoalLoopConfig,
    pub created_by: Id,
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    match s {
        Some(v) if !v.is_empty() => ts(&v).map(Some),
        _ => Ok(None),
    }
}

fn row_to_loop(r: &sqlx::sqlite::SqliteRow) -> Result<GoalLoop> {
    let def: GoalLoopDefinition = serde_json::from_str(&r.get::<String, _>("definition_json"))
        .map_err(|e| Error::Internal(format!("bad definition_json: {e}")))?;
    let limits: GoalLoopLimits = serde_json::from_str(&r.get::<String, _>("limits_json"))
        .map_err(|e| Error::Internal(format!("bad limits_json: {e}")))?;
    let config: GoalLoopConfig = serde_json::from_str(&r.get::<String, _>("config_json"))
        .map_err(|e| Error::Internal(format!("bad config_json: {e}")))?;
    Ok(GoalLoop {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        repo_path: r.get("repo_path"),
        definition: def,
        limits,
        config,
        status: GoalLoopStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad goal-loop status".into()))?,
        phase: parse_phase(&r.get::<String, _>("phase")),
        iterations_started: r.get::<i64, _>("iterations_started") as u32,
        current_iteration: r.get::<i64, _>("current_iteration") as u32,
        progress_pct: r.get::<i64, _>("progress_pct") as u32,
        context_digest: r.get("context_digest"),
        branch: r.get("branch"),
        worktree_path: r.get("worktree_path"),
        base_commit: r.get("base_commit"),
        summary: r.get("summary"),
        error: r.get("error"),
        run_started_at: opt_ts(r.get("run_started_at"))?,
        elapsed_secs: r.get::<i64, _>("elapsed_secs") as u64,
        cost_usd: r.get("cost_usd"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        finished_at: opt_ts(r.get("finished_at"))?,
    })
}

fn parse_phase(s: &str) -> GoalLoopPhase {
    match s {
        "planning" => GoalLoopPhase::Planning,
        "executing" => GoalLoopPhase::Executing,
        "evaluating" => GoalLoopPhase::Evaluating,
        "digesting" => GoalLoopPhase::Digesting,
        "waiting" => GoalLoopPhase::Waiting,
        _ => GoalLoopPhase::Done,
    }
}

fn row_to_iter(r: &sqlx::sqlite::SqliteRow) -> Result<GoalLoopIteration> {
    let agents_raw: String = r.try_get("agents_json").unwrap_or_default();
    let agents: Vec<LoopAgentState> = serde_json::from_str(&agents_raw).unwrap_or_default();
    let eval_raw: Option<String> = r.get("evaluation_json");
    let evaluation: Option<GoalLoopEvaluation> = eval_raw
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(&s).ok());
    Ok(GoalLoopIteration {
        id: r.get("id"),
        loop_id: r.get("loop_id"),
        workspace_id: r.get("workspace_id"),
        idx: r.get::<i64, _>("idx") as u32,
        status: r.get("status"),
        plan: r.get("plan"),
        agents,
        evaluation,
        context_in: r.get("context_in"),
        context_out: r.get("context_out"),
        tokens_input: r.get::<i64, _>("tokens_input") as u64,
        tokens_output: r.get::<i64, _>("tokens_output") as u64,
        cost_usd: r.get("cost_usd"),
        started_at: ts(&r.get::<String, _>("started_at"))?,
        finished_at: opt_ts(r.get("finished_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

impl GoalLoopsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, n: NewGoalLoop) -> Result<GoalLoop> {
        let id = new_id();
        let now = fmt(Utc::now());
        let def = serde_json::to_string(&n.definition)
            .map_err(|e| Error::Internal(format!("serialize definition: {e}")))?;
        let limits = serde_json::to_string(&n.limits)
            .map_err(|e| Error::Internal(format!("serialize limits: {e}")))?;
        let config = serde_json::to_string(&n.config)
            .map_err(|e| Error::Internal(format!("serialize config: {e}")))?;
        sqlx::query(
            "INSERT INTO goal_loops
             (id, workspace_id, name, repo_path, definition_json, limits_json, config_json,
              status, phase, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'draft', 'done', ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.workspace_id)
        .bind(&n.name)
        .bind(&n.repo_path)
        .bind(&def)
        .bind(&limits)
        .bind(&config)
        .bind(&n.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create goal loop"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<GoalLoop> {
        let row = sqlx::query("SELECT * FROM goal_loops WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("goal loop"))?;
        row_to_loop(&row)
    }

    pub async fn get_detail(&self, id: &Id) -> Result<GoalLoopDetail> {
        let loop_ = self.get(id).await?;
        let iterations = self.iterations_for(id).await?;
        Ok(GoalLoopDetail { loop_, iterations })
    }

    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<GoalLoop>> {
        let rows = sqlx::query(
            "SELECT * FROM goal_loops WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list goal loops"))?;
        rows.iter().map(row_to_loop).collect()
    }

    /// Loops the controller may currently own (running/paused/blocked) — used by
    /// the boot sweep.
    pub async fn list_running(&self) -> Result<Vec<GoalLoop>> {
        let rows = sqlx::query(
            "SELECT * FROM goal_loops WHERE status IN ('running','paused','blocked')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list running goal loops"))?;
        rows.iter().map(row_to_loop).collect()
    }

    async fn iterations_for(&self, loop_id: &Id) -> Result<Vec<GoalLoopIteration>> {
        let rows = sqlx::query(
            "SELECT * FROM goal_loop_iterations WHERE loop_id = ? ORDER BY idx",
        )
        .bind(loop_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("goal loop iterations"))?;
        rows.iter().map(row_to_iter).collect()
    }

    pub async fn get_iteration(&self, loop_id: &Id, idx: u32) -> Result<GoalLoopIteration> {
        let row = sqlx::query(
            "SELECT * FROM goal_loop_iterations WHERE loop_id = ? AND idx = ?",
        )
        .bind(loop_id)
        .bind(idx as i64)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("goal loop iteration"))?;
        row_to_iter(&row)
    }

    // -- runtime writers (controller-only) --------------------------------

    fn touch(&self) -> String {
        fmt(Utc::now())
    }

    /// Set status + phase + the display pointers in one statement (the common
    /// per-transition write that precedes an emit).
    pub async fn update_runtime(
        &self,
        id: &Id,
        status: GoalLoopStatus,
        phase: GoalLoopPhase,
        current_iteration: u32,
        progress_pct: u32,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE goal_loops SET status = ?, phase = ?, current_iteration = ?,
             progress_pct = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(phase.as_str())
        .bind(current_iteration as i64)
        .bind(progress_pct as i64)
        .bind(self.touch())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update goal-loop runtime"))?;
        Ok(())
    }

    /// Just the phase (cheap, for phase-only transitions within an iteration).
    pub async fn set_phase(&self, id: &Id, phase: GoalLoopPhase) -> Result<()> {
        sqlx::query("UPDATE goal_loops SET phase = ?, updated_at = ? WHERE id = ?")
            .bind(phase.as_str())
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set goal-loop phase"))?;
        Ok(())
    }

    /// Atomically increment and return the immutable iteration counter.
    pub async fn bump_iterations_started(&self, id: &Id) -> Result<u32> {
        sqlx::query(
            "UPDATE goal_loops SET iterations_started = iterations_started + 1,
             updated_at = ? WHERE id = ?",
        )
        .bind(self.touch())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("bump iterations_started"))?;
        let row = sqlx::query("SELECT iterations_started FROM goal_loops WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("read iterations_started"))?;
        Ok(row.get::<i64, _>("iterations_started") as u32)
    }

    pub async fn set_context_digest(&self, id: &Id, digest: &str) -> Result<()> {
        sqlx::query("UPDATE goal_loops SET context_digest = ?, updated_at = ? WHERE id = ?")
            .bind(digest)
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set context digest"))?;
        Ok(())
    }

    pub async fn set_branch(
        &self,
        id: &Id,
        branch: &str,
        worktree_path: &str,
        base_commit: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE goal_loops SET branch = ?, worktree_path = ?, base_commit = ?,
             updated_at = ? WHERE id = ?",
        )
        .bind(branch)
        .bind(worktree_path)
        .bind(base_commit)
        .bind(self.touch())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set goal-loop branch"))?;
        Ok(())
    }

    pub async fn set_run_started_at(&self, id: &Id, at: Option<DateTime<Utc>>) -> Result<()> {
        sqlx::query("UPDATE goal_loops SET run_started_at = ?, updated_at = ? WHERE id = ?")
            .bind(at.map(fmt))
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run_started_at"))?;
        Ok(())
    }

    pub async fn add_elapsed(&self, id: &Id, secs: u64) -> Result<()> {
        sqlx::query(
            "UPDATE goal_loops SET elapsed_secs = elapsed_secs + ?, updated_at = ? WHERE id = ?",
        )
        .bind(secs as i64)
        .bind(self.touch())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("add elapsed"))?;
        Ok(())
    }

    pub async fn add_cost(&self, id: &Id, usd: f64) -> Result<()> {
        sqlx::query(
            "UPDATE goal_loops SET cost_usd = cost_usd + ?, updated_at = ? WHERE id = ?",
        )
        .bind(usd)
        .bind(self.touch())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("add cost"))?;
        Ok(())
    }

    pub async fn set_name(&self, id: &Id, name: &str) -> Result<()> {
        sqlx::query("UPDATE goal_loops SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set goal-loop name"))?;
        Ok(())
    }

    pub async fn set_limits(&self, id: &Id, limits: &GoalLoopLimits) -> Result<()> {
        let json = serde_json::to_string(limits)
            .map_err(|e| Error::Internal(format!("serialize limits: {e}")))?;
        sqlx::query("UPDATE goal_loops SET limits_json = ?, updated_at = ? WHERE id = ?")
            .bind(&json)
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set goal-loop limits"))?;
        Ok(())
    }

    pub async fn set_config(&self, id: &Id, config: &GoalLoopConfig) -> Result<()> {
        let json = serde_json::to_string(config)
            .map_err(|e| Error::Internal(format!("serialize config: {e}")))?;
        sqlx::query("UPDATE goal_loops SET config_json = ?, updated_at = ? WHERE id = ?")
            .bind(&json)
            .bind(self.touch())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set goal-loop config"))?;
        Ok(())
    }

    /// Move a loop to a terminal/blocked state with a final summary/error.
    /// Clears the wall-clock anchor and sets `finished_at`. The controller adds
    /// the final active window via [`add_elapsed`] before calling this.
    pub async fn finalize(
        &self,
        id: &Id,
        status: GoalLoopStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = self.touch();
        sqlx::query(
            "UPDATE goal_loops SET status = ?, phase = 'done', summary = ?, error = ?,
             run_started_at = NULL, finished_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(summary)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finalize goal loop"))?;
        Ok(())
    }

    /// Move a loop into the Running state (start/resume): re-anchor the
    /// wall-clock window and clear any prior finish/error so a resumed loop
    /// isn't shown as finished. The controller takes over from here.
    pub async fn mark_running(&self, id: &Id, run_started_at: DateTime<Utc>) -> Result<()> {
        let now = self.touch();
        sqlx::query(
            "UPDATE goal_loops SET status = 'running', run_started_at = ?, finished_at = NULL,
             error = NULL, updated_at = ? WHERE id = ?",
        )
        .bind(fmt(run_started_at))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("mark goal loop running"))?;
        Ok(())
    }

    // -- iterations -------------------------------------------------------

    /// Create the iteration row, seeding `agents_json` with one `pending`
    /// placeholder per executor so later `set_iter_agent_at` calls (which use
    /// `json_replace`) have an existing index to target.
    pub async fn add_iteration(
        &self,
        loop_id: &Id,
        workspace_id: &Id,
        idx: u32,
        context_in: &str,
        executors: &[GoalLoopAgentCfg],
    ) -> Result<GoalLoopIteration> {
        let id = new_id();
        let now = fmt(Utc::now());
        let placeholders: Vec<LoopAgentState> = executors
            .iter()
            .map(|e| LoopAgentState {
                name: e.name.clone(),
                provider: e.provider.clone(),
                model: e.model.clone(),
                status: "pending".to_string(),
                note: String::new(),
                session_id: None,
                output_summary: None,
            })
            .collect();
        let agents_json = serde_json::to_string(&placeholders)
            .map_err(|e| Error::Internal(format!("serialize agents: {e}")))?;
        sqlx::query(
            "INSERT INTO goal_loop_iterations
             (id, loop_id, workspace_id, idx, status, agents_json, context_in, started_at)
             VALUES (?, ?, ?, ?, 'planning', ?, ?, ?)",
        )
        .bind(&id)
        .bind(loop_id)
        .bind(workspace_id)
        .bind(idx as i64)
        .bind(&agents_json)
        .bind(context_in)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add goal-loop iteration"))?;
        self.get_iteration_by_id(&id).await
    }

    async fn get_iteration_by_id(&self, id: &Id) -> Result<GoalLoopIteration> {
        let row = sqlx::query("SELECT * FROM goal_loop_iterations WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("goal loop iteration"))?;
        row_to_iter(&row)
    }

    pub async fn update_iteration_status(
        &self,
        iter_id: &Id,
        status: &str,
        finished: bool,
    ) -> Result<()> {
        let fin = if finished { Some(fmt(Utc::now())) } else { None };
        sqlx::query(
            "UPDATE goal_loop_iterations SET status = ?, finished_at = COALESCE(?, finished_at)
             WHERE id = ?",
        )
        .bind(status)
        .bind(fin)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update iteration status"))?;
        Ok(())
    }

    pub async fn set_iter_plan(&self, iter_id: &Id, plan: &str) -> Result<()> {
        sqlx::query("UPDATE goal_loop_iterations SET plan = ? WHERE id = ?")
            .bind(plan)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set iteration plan"))?;
        Ok(())
    }

    /// Atomically replace one executor's live state (element `index` of
    /// `agents_json`) without clobbering siblings — see [`ReviewsRepo::set_agent_at`].
    /// Requires the index to already exist (seeded by [`add_iteration`]).
    pub async fn set_iter_agent_at(
        &self,
        iter_id: &Id,
        index: usize,
        agent: &LoopAgentState,
    ) -> Result<()> {
        let elem = serde_json::to_string(agent)
            .map_err(|e| Error::Internal(format!("serialize agent: {e}")))?;
        let path = format!("$[{index}]");
        sqlx::query(
            "UPDATE goal_loop_iterations SET agents_json = json_replace(agents_json, ?, json(?))
             WHERE id = ?",
        )
        .bind(&path)
        .bind(&elem)
        .bind(iter_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set iteration agent"))?;
        Ok(())
    }

    pub async fn set_iter_evaluation(
        &self,
        iter_id: &Id,
        eval: &GoalLoopEvaluation,
    ) -> Result<()> {
        let json = serde_json::to_string(eval)
            .map_err(|e| Error::Internal(format!("serialize evaluation: {e}")))?;
        sqlx::query("UPDATE goal_loop_iterations SET evaluation_json = ? WHERE id = ?")
            .bind(&json)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set iteration evaluation"))?;
        Ok(())
    }

    pub async fn set_iter_context_out(&self, iter_id: &Id, ctx: &str) -> Result<()> {
        sqlx::query("UPDATE goal_loop_iterations SET context_out = ? WHERE id = ?")
            .bind(ctx)
            .bind(iter_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set iteration context_out"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM goal_loop_iterations WHERE loop_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete goal-loop iterations"))?;
        sqlx::query("DELETE FROM goal_loops WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete goal loop"))?;
        Ok(())
    }

    /// Boot sweep: a loop's controller dies with the daemon, so any row left in
    /// `running`/`paused`/`blocked` is orphaned. Flip them (and their
    /// non-terminal iterations) to failed/error and RETURN the rows so the caller
    /// can also remove worktrees + kill executor sessions.
    pub async fn fail_running(&self, error: &str) -> Result<Vec<GoalLoop>> {
        let loops = self.list_running().await?;
        if loops.is_empty() {
            return Ok(loops);
        }
        let now = fmt(Utc::now());
        for l in &loops {
            sqlx::query(
                "UPDATE goal_loops SET status = 'failed', error = ?, phase = 'done',
                 run_started_at = NULL, finished_at = ?, updated_at = ? WHERE id = ?",
            )
            .bind(error)
            .bind(&now)
            .bind(&now)
            .bind(&l.id)
            .execute(&self.pool)
            .await
            .map_err(dberr("fail running goal loop"))?;
            sqlx::query(
                "UPDATE goal_loop_iterations SET status = 'error', finished_at = COALESCE(finished_at, ?)
                 WHERE loop_id = ? AND status NOT IN ('done','error')",
            )
            .bind(&now)
            .bind(&l.id)
            .execute(&self.pool)
            .await
            .map_err(dberr("fail running goal-loop iterations"))?;
        }
        Ok(loops)
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

    fn new_loop() -> NewGoalLoop {
        NewGoalLoop {
            workspace_id: "ws1".into(),
            name: "Test goal".into(),
            repo_path: "/tmp/repo".into(),
            definition: GoalLoopDefinition {
                title: "Make it stream".into(),
                summary: String::new(),
                objectives: vec![],
                acceptance_criteria: vec![],
                constraints: vec![],
                out_of_scope: vec![],
                success_signal: String::new(),
            },
            limits: GoalLoopLimits::default(),
            config: GoalLoopConfig::default(),
            created_by: "u1".into(),
        }
    }

    #[tokio::test]
    async fn create_get_and_iterations_roundtrip() {
        let pool = mem_pool().await;
        let repo = GoalLoopsRepo::new(pool.clone());

        let l = repo.create(new_loop()).await.unwrap();
        assert_eq!(l.status, GoalLoopStatus::Draft);
        assert_eq!(l.iterations_started, 0);

        // Gate counter increments immutably.
        let n = repo.bump_iterations_started(&l.id).await.unwrap();
        assert_eq!(n, 1);

        // Seed an iteration with one executor placeholder, then flip it live.
        let execs = l.config.executors.clone();
        let it = repo
            .add_iteration(&l.id, &l.workspace_id, 1, "", &execs)
            .await
            .unwrap();
        assert_eq!(it.agents.len(), 1);
        assert_eq!(it.agents[0].status, "pending");

        let mut a0 = it.agents[0].clone();
        a0.status = "running".into();
        a0.session_id = Some("s0".into());
        repo.set_iter_agent_at(&it.id, 0, &a0).await.unwrap();

        let detail = repo.get_detail(&l.id).await.unwrap();
        assert_eq!(detail.iterations.len(), 1);
        assert_eq!(detail.iterations[0].agents[0].status, "running");
        assert_eq!(
            detail.iterations[0].agents[0].session_id.as_deref(),
            Some("s0")
        );
    }

    #[tokio::test]
    async fn fail_running_returns_and_flips_rows() {
        let pool = mem_pool().await;
        let repo = GoalLoopsRepo::new(pool.clone());
        let l = repo.create(new_loop()).await.unwrap();
        repo.update_runtime(&l.id, GoalLoopStatus::Running, GoalLoopPhase::Planning, 1, 0)
            .await
            .unwrap();

        let failed = repo.fail_running("interrupted").await.unwrap();
        assert_eq!(failed.len(), 1);
        let after = repo.get(&l.id).await.unwrap();
        assert_eq!(after.status, GoalLoopStatus::Failed);
        assert_eq!(after.error.as_deref(), Some("interrupted"));
    }
}

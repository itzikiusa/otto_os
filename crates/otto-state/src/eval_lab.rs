//! Persistence for the eval lab: per-repo **golden tasks** (the reusable
//! evaluation corpus + regression cases) and **matrices** (provider × skill ×
//! prompt comparison runs). Migration `0088_eval_lab.sql`.
//!
//! Matrix *cells* are `skill_evals` rows (see [`crate::SkillEvalsRepo`]); this
//! module owns only the matrix header row.

use chrono::Utc;
use otto_core::domain::{EvalMatrix, GoldenTask, MatrixPrompt};
use otto_core::{new_id, Error, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

// ===========================================================================
// Golden tasks
// ===========================================================================

#[derive(Clone)]
pub struct GoldenTasksRepo {
    pool: SqlitePool,
}

fn row_to_golden(r: &sqlx::sqlite::SqliteRow) -> Result<GoldenTask> {
    let tags_raw: String = r.try_get("tags_json").unwrap_or_else(|_| "[]".into());
    let tags: Vec<String> = serde_json::from_str(&tags_raw).unwrap_or_default();
    Ok(GoldenTask {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        repo_key: r.get("repo_key"),
        name: r.get("name"),
        prompt: r.get("prompt"),
        skill: r.get("skill"),
        test_cmd: r.get("test_cmd"),
        lint_cmd: r.get("lint_cmd"),
        build_cmd: r.get("build_cmd"),
        rubric: r.get("rubric"),
        tags,
        origin: r.get("origin"),
        source_eval_id: r.get("source_eval_id"),
        source_iter_id: r.get("source_iter_id"),
        enabled: r.get::<i64, _>("enabled") != 0,
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

/// Fields a caller supplies to create/update a golden task.
#[derive(Debug, Clone, Default)]
pub struct GoldenTaskInput {
    pub name: String,
    pub prompt: String,
    pub skill: String,
    pub test_cmd: String,
    pub lint_cmd: String,
    pub build_cmd: String,
    pub rubric: String,
    pub tags: Vec<String>,
    pub enabled: bool,
}

impl GoldenTasksRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        workspace_id: &str,
        repo_key: &str,
        input: &GoldenTaskInput,
        origin: &str,
        source_eval_id: Option<&str>,
        source_iter_id: Option<&str>,
        created_by: &str,
    ) -> Result<GoldenTask> {
        let id = new_id();
        let now = fmt(Utc::now());
        let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO eval_golden_tasks
                (id, workspace_id, repo_key, name, prompt, skill, test_cmd, lint_cmd, build_cmd,
                 rubric, tags_json, origin, source_eval_id, source_iter_id, enabled,
                 created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(repo_key)
        .bind(&input.name)
        .bind(&input.prompt)
        .bind(&input.skill)
        .bind(&input.test_cmd)
        .bind(&input.lint_cmd)
        .bind(&input.build_cmd)
        .bind(&input.rubric)
        .bind(&tags_json)
        .bind(origin)
        .bind(source_eval_id)
        .bind(source_iter_id)
        .bind(input.enabled as i64)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create golden task"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<GoldenTask> {
        let row = sqlx::query("SELECT * FROM eval_golden_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("golden task"))?;
        row_to_golden(&row)
    }

    /// List golden tasks for a workspace, optionally narrowed to one repo, newest
    /// first.
    pub async fn list(&self, workspace_id: &str, repo_key: Option<&str>) -> Result<Vec<GoldenTask>> {
        let rows = if let Some(rk) = repo_key {
            sqlx::query(
                "SELECT * FROM eval_golden_tasks WHERE workspace_id = ? AND repo_key = ? ORDER BY created_at DESC",
            )
            .bind(workspace_id)
            .bind(rk)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT * FROM eval_golden_tasks WHERE workspace_id = ? ORDER BY created_at DESC",
            )
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(dberr("list golden tasks"))?;
        rows.iter().map(row_to_golden).collect()
    }

    pub async fn update(&self, id: &str, input: &GoldenTaskInput) -> Result<GoldenTask> {
        let now = fmt(Utc::now());
        let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "UPDATE eval_golden_tasks SET name = ?, prompt = ?, skill = ?, test_cmd = ?,
                 lint_cmd = ?, build_cmd = ?, rubric = ?, tags_json = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&input.name)
        .bind(&input.prompt)
        .bind(&input.skill)
        .bind(&input.test_cmd)
        .bind(&input.lint_cmd)
        .bind(&input.build_cmd)
        .bind(&input.rubric)
        .bind(&tags_json)
        .bind(input.enabled as i64)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update golden task"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM eval_golden_tasks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete golden task"))?;
        Ok(())
    }

    /// The existing regression task captured from a given iteration, if any
    /// (used to dedupe failed-eval → regression).
    pub async fn find_by_source_iter(&self, iter_id: &str) -> Result<Option<GoldenTask>> {
        let row = sqlx::query("SELECT * FROM eval_golden_tasks WHERE source_iter_id = ? LIMIT 1")
            .bind(iter_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("find golden by source iter"))?;
        row.as_ref().map(row_to_golden).transpose()
    }
}

// ===========================================================================
// Matrices
// ===========================================================================

#[derive(Clone)]
pub struct EvalMatricesRepo {
    pool: SqlitePool,
}

fn row_to_matrix(r: &sqlx::sqlite::SqliteRow) -> Result<EvalMatrix> {
    let providers: Vec<String> =
        serde_json::from_str(&r.try_get::<String, _>("providers_json").unwrap_or_else(|_| "[]".into()))
            .unwrap_or_default();
    let skills: Vec<String> =
        serde_json::from_str(&r.try_get::<String, _>("skills_json").unwrap_or_else(|_| "[]".into()))
            .unwrap_or_default();
    let prompts: Vec<MatrixPrompt> =
        serde_json::from_str(&r.try_get::<String, _>("prompts_json").unwrap_or_else(|_| "[]".into()))
            .unwrap_or_default();
    Ok(EvalMatrix {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        status: r.get("status"),
        repo_key: r.get("repo_key"),
        mode: r.get("mode"),
        providers,
        skills,
        prompts,
        cells: Vec::new(), // populated by the server from skill_evals
        created_at: r.get("created_at"),
    })
}

impl EvalMatricesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        workspace_id: &str,
        name: &str,
        mode: &str,
        repo_key: &str,
        providers: &[String],
        skills: &[String],
        prompts: &[MatrixPrompt],
        created_by: &str,
    ) -> Result<EvalMatrix> {
        let id = new_id();
        let now = fmt(Utc::now());
        let providers_json = serde_json::to_string(providers).unwrap_or_else(|_| "[]".into());
        let skills_json = serde_json::to_string(skills).unwrap_or_else(|_| "[]".into());
        let prompts_json = serde_json::to_string(prompts)
            .map_err(|e| Error::Internal(format!("serialize matrix prompts: {e}")))?;
        sqlx::query(
            "INSERT INTO eval_matrices
                (id, workspace_id, name, status, repo_key, mode, providers_json, skills_json,
                 prompts_json, created_by, created_at)
             VALUES (?, ?, ?, 'running', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(name)
        .bind(repo_key)
        .bind(mode)
        .bind(&providers_json)
        .bind(&skills_json)
        .bind(&prompts_json)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create matrix"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<EvalMatrix> {
        let row = sqlx::query("SELECT * FROM eval_matrices WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("matrix"))?;
        row_to_matrix(&row)
    }

    pub async fn list(&self, workspace_id: &str) -> Result<Vec<EvalMatrix>> {
        let rows = sqlx::query(
            "SELECT * FROM eval_matrices WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list matrices"))?;
        rows.iter().map(row_to_matrix).collect()
    }

    pub async fn set_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE eval_matrices SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set matrix status"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::Id as CoreId;

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

    fn input(name: &str) -> GoldenTaskInput {
        GoldenTaskInput {
            name: name.into(),
            prompt: "do X".into(),
            test_cmd: "cargo test".into(),
            tags: vec!["go".into()],
            enabled: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn golden_crud_and_repo_filter() {
        let pool = mem_pool().await;
        let repo = GoldenTasksRepo::new(pool);

        let a = repo
            .create("ws1", "repoA", &input("task A"), "manual", None, None, "root")
            .await
            .unwrap();
        let _b = repo
            .create("ws1", "repoB", &input("task B"), "manual", None, None, "root")
            .await
            .unwrap();
        assert_eq!(a.origin, "manual");
        assert_eq!(a.tags, vec!["go".to_string()]);

        // list filters by repo_key
        assert_eq!(repo.list("ws1", Some("repoA")).await.unwrap().len(), 1);
        assert_eq!(repo.list("ws1", None).await.unwrap().len(), 2);

        // update
        let mut up = input("task A renamed");
        up.test_cmd = "go test ./...".into();
        let a2 = repo.update(&a.id, &up).await.unwrap();
        assert_eq!(a2.name, "task A renamed");
        assert_eq!(a2.test_cmd, "go test ./...");

        // delete
        repo.delete(&a.id).await.unwrap();
        assert_eq!(repo.list("ws1", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn regression_dedupe_by_source_iter() {
        let pool = mem_pool().await;
        let repo = GoldenTasksRepo::new(pool);
        assert!(repo.find_by_source_iter("iterX").await.unwrap().is_none());
        let g = repo
            .create("ws1", "repoA", &input("reg"), "regression", Some("ev1"), Some("iterX"), "root")
            .await
            .unwrap();
        assert_eq!(g.origin, "regression");
        let found = repo.find_by_source_iter("iterX").await.unwrap();
        assert_eq!(found.map(|t| t.id), Some(g.id));
        // The unique partial index rejects a second regression from the same iter.
        let dup = repo
            .create("ws1", "repoA", &input("reg2"), "regression", Some("ev1"), Some("iterX"), "root")
            .await;
        assert!(dup.is_err());
    }

    #[tokio::test]
    async fn matrix_create_get_list() {
        let pool = mem_pool().await;
        let repo = EvalMatricesRepo::new(pool);
        let prompts = vec![MatrixPrompt {
            label: "P1".into(),
            task: "do X".into(),
            golden_task_id: None,
        }];
        let m = repo
            .create(
                "ws1",
                "claude-vs-codex",
                "score_only",
                "repoA",
                &["claude".to_string(), "codex".to_string()],
                &["golang-feature".to_string()],
                &prompts,
                "root",
            )
            .await
            .unwrap();
        assert_eq!(m.status, "running");
        assert_eq!(m.providers.len(), 2);
        assert_eq!(m.prompts.len(), 1);

        repo.set_status(&m.id, "done").await.unwrap();
        assert_eq!(repo.get(&m.id).await.unwrap().status, "done");
        assert_eq!(repo.list("ws1").await.unwrap().len(), 1);

        let _ = CoreId::from("x"); // keep the import used
    }
}

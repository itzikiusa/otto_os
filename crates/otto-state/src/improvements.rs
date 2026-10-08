//! Improvement runs + edits (the self-improvement version log).

use crate::DbPool;
use chrono::Utc;
use otto_core::domain::{
    ImprovementEdit, ImprovementEditKind, ImprovementEditStatus, ImprovementRisk, ImprovementRun,
    ImprovementRunStatus, ImprovementTarget, ImprovementTrigger,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct ImprovementsRepo {
    pool: DbPool,
}

/// A durable source claim. The token fences completion from expired owners.
pub struct EvidenceClaim {
    pub token: String,
    pub checkpoint: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct LearningTrail {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub summary: String,
}

/// Insert payload for a new edit row.
pub struct NewEdit {
    pub run_id: Id,
    pub workspace_id: Id,
    pub target: ImprovementTarget,
    pub target_ref: String,
    pub target_path: String,
    pub kind: ImprovementEditKind,
    pub risk: ImprovementRisk,
    pub status: ImprovementEditStatus,
    pub rationale: String,
    pub evidence: Vec<String>,
    pub before_content: Option<String>,
    pub after_content: String,
    pub actor: Option<String>,
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<ImprovementRun> {
    Ok(ImprovementRun {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        trigger: ImprovementTrigger::parse(&r.get::<String, _>("trigger"))
            .ok_or_else(|| Error::Internal("bad trigger".into()))?,
        status: ImprovementRunStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad run status".into()))?,
        summary: r.get("summary"),
        sessions_reviewed: r.get("sessions_reviewed"),
        applied: r.get("applied"),
        pending: r.get("pending"),
        error: r.get("error"),
        started_at: ts(&r.get::<String, _>("started_at"))?,
        finished_at: match r.get::<Option<String>, _>("finished_at") {
            Some(s) => Some(ts(&s)?),
            None => None,
        },
    })
}

fn row_to_edit(r: &sqlx::sqlite::SqliteRow) -> Result<ImprovementEdit> {
    Ok(ImprovementEdit {
        id: r.get("id"),
        run_id: r.get("run_id"),
        workspace_id: r.get("workspace_id"),
        target: ImprovementTarget::parse(&r.get::<String, _>("target"))
            .ok_or_else(|| Error::Internal("bad target".into()))?,
        target_ref: r.get("target_ref"),
        target_path: r.get("target_path"),
        kind: ImprovementEditKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad kind".into()))?,
        risk: ImprovementRisk::parse(&r.get::<String, _>("risk"))
            .ok_or_else(|| Error::Internal("bad risk".into()))?,
        status: ImprovementEditStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad edit status".into()))?,
        rationale: r.get("rationale"),
        evidence: json(&r.get::<String, _>("evidence_json"))
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        before_content: r.get("before_content"),
        after_content: r.get("after_content"),
        applied_at: match r.get::<Option<String>, _>("applied_at") {
            Some(s) => Some(ts(&s)?),
            None => None,
        },
        actor: r.get("actor"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl ImprovementsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// Claim one evidence stream for at most 30 minutes. A single SQL statement
    /// prevents live/channel/narrative triggers from concurrently consuming it.
    pub async fn claim_evidence(&self, source: &str) -> Result<Option<EvidenceClaim>> {
        let token = new_id();
        let now = Utc::now();
        let row = sqlx::query(
            "INSERT INTO learning_checkpoints(source,lease_token,lease_until,updated_at)
             VALUES(?,?,?,?) ON CONFLICT(source) DO UPDATE SET
             lease_token=excluded.lease_token, lease_until=excluded.lease_until,
             updated_at=excluded.updated_at
             WHERE learning_checkpoints.lease_until IS NULL OR learning_checkpoints.lease_until < ?
             RETURNING checkpoint_json",
        )
        .bind(source)
        .bind(&token)
        .bind(fmt(now + chrono::Duration::minutes(30)))
        .bind(fmt(now))
        .bind(fmt(now))
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("learning checkpoint"))?;
        Ok(row.map(|r| EvidenceClaim {
            token,
            checkpoint: serde_json::from_str(&r.get::<String, _>("checkpoint_json"))
                .unwrap_or_default(),
        }))
    }

    /// `None` releases a failed/skipped attempt without consuming its evidence.
    pub async fn finish_evidence(
        &self,
        source: &str,
        token: &str,
        checkpoint: Option<&serde_json::Value>,
    ) -> Result<()> {
        sqlx::query("UPDATE learning_checkpoints SET checkpoint_json=COALESCE(?,checkpoint_json),
                     lease_token=NULL,lease_until=NULL,updated_at=? WHERE source=? AND lease_token=?")
            .bind(checkpoint.map(serde_json::Value::to_string)).bind(fmt(Utc::now()))
            .bind(source).bind(token).execute(&self.pool).await.map_err(dberr("learning checkpoint"))?;
        Ok(())
    }

    /// Only intentional user observations and skill names, never raw terminal
    /// output/tool payloads. Filter before LIMIT so tool-heavy sessions retain
    /// their latest corrections. Existing trail ownership remains unchanged.
    pub async fn learning_trail(
        &self,
        session: &Id,
        after: Option<&str>,
        since: &str,
    ) -> Result<Vec<LearningTrail>> {
        let rows = sqlx::query(
            "SELECT id,kind,source,summary FROM agent_trail
            WHERE session_id=? AND id>COALESCE(?,'') AND ts>=?
            AND (kind='skill' OR (source='user' AND kind IN ('prompt','note')))
            ORDER BY id DESC LIMIT 100",
        )
        .bind(session)
        .bind(after)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("learning trail"))?;
        let mut out: Vec<_> = rows
            .iter()
            .map(|r| LearningTrail {
                id: r.get("id"),
                kind: r.get("kind"),
                source: r.get("source"),
                summary: r.get("summary"),
            })
            .collect();
        out.reverse();
        Ok(out)
    }

    // ---- runs ----

    /// Serialize every trigger against the same workspace admission boundary.
    pub async fn create_run(&self, ws: &Id, trigger: ImprovementTrigger) -> Result<ImprovementRun> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("admit improvement"))?;
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM improvement_runs WHERE workspace_id = ? AND status = 'running')")
            .bind(ws).fetch_one(&mut *tx).await.map_err(dberr("check improvement admission"))?;
        if busy {
            return Err(Error::Conflict(
                "an improvement run is already in progress".into(),
            ));
        }
        let id = new_id();
        sqlx::query("INSERT INTO improvement_runs (id, workspace_id, trigger, status, started_at) VALUES (?, ?, ?, 'running', ?)")
            .bind(&id).bind(ws).bind(trigger.as_str()).bind(fmt(Utc::now()))
            .execute(&mut *tx).await.map_err(dberr("create run"))?;
        let row = sqlx::query("SELECT * FROM improvement_runs WHERE id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("created run"))?;
        let run = row_to_run(&row)?;
        tx.commit()
            .await
            .map_err(dberr("commit improvement admission"))?;
        Ok(run)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_run(
        &self,
        id: &Id,
        status: ImprovementRunStatus,
        summary: &str,
        sessions_reviewed: i64,
        applied: i64,
        pending: i64,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE improvement_runs SET status = ?, summary = ?, sessions_reviewed = ?, \
             applied = ?, pending = ?, error = ?, finished_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(summary)
        .bind(sessions_reviewed)
        .bind(applied)
        .bind(pending)
        .bind(error)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish run"))?;
        Ok(())
    }

    pub async fn get_run(&self, id: &Id) -> Result<ImprovementRun> {
        let r = sqlx::query("SELECT * FROM improvement_runs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("run"))?;
        row_to_run(&r)
    }

    pub async fn list_runs(&self, ws: &Id, limit: i64) -> Result<Vec<ImprovementRun>> {
        let rows = sqlx::query(
            "SELECT * FROM improvement_runs WHERE workspace_id = ? \
             ORDER BY started_at DESC LIMIT ?",
        )
        .bind(ws)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// Boot-time recovery: runs execute in-process, so any row still
    /// `running` when the daemon starts lost its worker (crash, kill, update).
    /// Left alone it blocked every later scheduled/live run in its workspace
    /// forever (`has_running`). Marks them `failed`; returns how many.
    pub async fn fail_orphaned_runs(&self) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE improvement_runs SET status = 'failed', \
             error = COALESCE(error, 'interrupted: the daemon restarted while this run was in progress'), \
             finished_at = ? WHERE status = 'running'",
        )
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("fail orphaned runs"))?;
        Ok(r.rows_affected())
    }

    /// True if the workspace currently has a run in `status = 'running'`.
    pub async fn has_running(&self, ws: &Id) -> Result<bool> {
        let r = sqlx::query(
            "SELECT COUNT(*) AS n FROM improvement_runs WHERE workspace_id = ? AND status = 'running'",
        )
        .bind(ws)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("running count"))?;
        Ok(r.get::<i64, _>("n") > 0)
    }

    // ---- edits ----

    pub async fn create_edit(&self, e: NewEdit) -> Result<ImprovementEdit> {
        let id = new_id();
        let now = fmt(Utc::now());
        let applied_at = if e.status == ImprovementEditStatus::Applied {
            Some(now.clone())
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO improvement_edits (id, run_id, workspace_id, target, target_ref, \
             target_path, kind, risk, status, rationale, evidence_json, before_content, \
             after_content, applied_at, actor, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&e.run_id)
        .bind(&e.workspace_id)
        .bind(e.target.as_str())
        .bind(&e.target_ref)
        .bind(&e.target_path)
        .bind(e.kind.as_str())
        .bind(e.risk.as_str())
        .bind(e.status.as_str())
        .bind(&e.rationale)
        .bind(serde_json::to_string(&e.evidence).unwrap_or_else(|_| "[]".into()))
        .bind(&e.before_content)
        .bind(&e.after_content)
        .bind(&applied_at)
        .bind(&e.actor)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create edit"))?;
        self.get_edit(&id).await
    }

    pub async fn get_edit(&self, id: &Id) -> Result<ImprovementEdit> {
        let r = sqlx::query("SELECT * FROM improvement_edits WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("edit"))?;
        row_to_edit(&r)
    }

    pub async fn list_edits_by_run(&self, run_id: &Id) -> Result<Vec<ImprovementEdit>> {
        let rows =
            sqlx::query("SELECT * FROM improvement_edits WHERE run_id = ? ORDER BY created_at")
                .bind(run_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("edits"))?;
        rows.iter().map(row_to_edit).collect()
    }

    pub async fn list_edits_by_status(
        &self,
        ws: &Id,
        status: ImprovementEditStatus,
    ) -> Result<Vec<ImprovementEdit>> {
        let rows = sqlx::query(
            "SELECT * FROM improvement_edits WHERE workspace_id = ? AND status = ? \
             ORDER BY created_at DESC",
        )
        .bind(ws)
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("edits"))?;
        rows.iter().map(row_to_edit).collect()
    }

    /// Set status (+ stamp applied_at/actor when transitioning to applied).
    pub async fn set_edit_status(
        &self,
        id: &Id,
        status: ImprovementEditStatus,
        actor: Option<&str>,
    ) -> Result<ImprovementEdit> {
        let applied_at = if status == ImprovementEditStatus::Applied {
            Some(fmt(Utc::now()))
        } else {
            None
        };
        sqlx::query(
            "UPDATE improvement_edits SET status = ?, actor = COALESCE(?, actor), \
             applied_at = COALESCE(?, applied_at) WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(actor)
        .bind(&applied_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set edit status"))?;
        self.get_edit(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovery_improvement_admission_serializes_all_workspace_triggers() {
        let pool = crate::db::test_pool().await;
        sqlx::query("INSERT INTO workspaces (id,name,root_path,created_at) VALUES ('admission-ws','w','/tmp','2026-10-08')")
            .execute(&pool).await.unwrap();
        let repo = ImprovementsRepo::new(pool.clone());
        let ws = "admission-ws".to_string();
        let (manual, live) = tokio::join!(
            repo.create_run(&ws, ImprovementTrigger::Manual),
            repo.create_run(&ws, ImprovementTrigger::Live)
        );
        assert_eq!(
            usize::from(manual.is_ok()) + usize::from(live.is_ok()),
            1,
            "only one workspace producer may be admitted"
        );
        let conflict = if manual.is_err() {
            manual.as_ref().err()
        } else {
            live.as_ref().err()
        };
        assert!(matches!(conflict, Some(Error::Conflict(_))));
        let winner = manual.or(live).unwrap();
        repo.finish_run(
            &winner.id,
            ImprovementRunStatus::Done,
            "finished",
            0,
            0,
            0,
            None,
        )
        .await
        .unwrap();
        assert!(repo.create_run(&ws, ImprovementTrigger::Live).await.is_ok());
    }

    #[tokio::test]
    async fn orphaned_running_runs_fail_at_boot() {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        let repo = ImprovementsRepo::new(pool);
        let ws: Id = "w1".into();
        let done = repo
            .create_run(&ws, ImprovementTrigger::Manual)
            .await
            .unwrap();
        repo.finish_run(&done.id, ImprovementRunStatus::Done, "ok", 1, 0, 0, None)
            .await
            .unwrap();
        let stuck = repo
            .create_run(&ws, ImprovementTrigger::Manual)
            .await
            .unwrap();
        assert!(repo.has_running(&ws).await.unwrap());
        assert_eq!(repo.fail_orphaned_runs().await.unwrap(), 1);
        assert!(!repo.has_running(&ws).await.unwrap());
        let s = repo.get_run(&stuck.id).await.unwrap();
        assert_eq!(s.status, ImprovementRunStatus::Failed);
        assert!(s.error.unwrap_or_default().contains("interrupted"));
        let d = repo.get_run(&done.id).await.unwrap();
        assert_eq!(
            d.status,
            ImprovementRunStatus::Done,
            "finished runs untouched"
        );
        assert_eq!(repo.fail_orphaned_runs().await.unwrap(), 0);
    }
}

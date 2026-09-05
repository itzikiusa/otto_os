//! Durable reviewed-change state machine. Every transition is compare-and-swap;
//! claims and target locks commit before an adapter may send SQL.
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeTarget {
    pub connection_id: Id,
    pub node: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSnapshot {
    pub target: ChangeTarget,
    pub environment: String,
    pub policy_revision: i64,
    pub connection_fingerprint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub script: String,
    pub targets: Vec<ChangeTarget>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseChange {
    pub id: Id,
    pub author_id: Id,
    pub real_author_id: Id,
    pub title: String,
    pub description: String,
    pub script: String,
    pub targets: Vec<ChangeTarget>,
    pub revision: i64,
    pub status: String,
    pub content_hash: String,
    pub executor_id: Option<Id>,
    pub validation: Value,
    pub approved_by: Option<Id>,
    pub approved_real_by: Option<Id>,
    pub approval_hash: Option<String>,
    pub cancellation_requested: bool,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeAttempt {
    pub id: Id,
    pub change_id: Id,
    pub connection_id: Id,
    pub node: Option<String>,
    pub state: String,
    pub executor_id: Id,
    pub ordinal: i64,
    pub summary: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub id: Id,
    pub revision: i64,
    pub action: String,
    pub actor_id: Id,
    pub real_actor_id: Id,
    pub data: Value,
    pub created_at: String,
}
#[derive(Clone)]
pub struct DatabaseChangesRepo {
    pool: SqlitePool,
}
fn db(e: sqlx::Error) -> Error {
    Error::Internal(format!("database changes: {e}"))
}
fn encode(v: &impl Serialize) -> Result<String> {
    serde_json::to_string(v).map_err(|e| Error::Internal(e.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| Error::Internal(format!("invalid stored change: {e}")))
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn conflict() -> Error {
    Error::Conflict("change was modified or is not in the required state; refresh it".into())
}
fn from_row(r: sqlx::sqlite::SqliteRow) -> Result<DatabaseChange> {
    Ok(DatabaseChange {
        id: r.get("id"),
        author_id: r.get("author_id"),
        real_author_id: r.get("real_author_id"),
        title: r.get("title"),
        description: r.get("description"),
        script: r.get("script"),
        targets: decode(r.get("targets_json"))?,
        revision: r.get("revision"),
        status: r.get("status"),
        content_hash: r.get("content_hash"),
        executor_id: r.get("executor_id"),
        validation: decode(r.get("validation_json"))?,
        approved_by: r.get("approved_by"),
        approved_real_by: r.get("approved_real_by"),
        approval_hash: r.get("approval_hash"),
        cancellation_requested: r.get("cancellation_requested"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}
async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    revision: i64,
    action: &str,
    actor: &str,
    real: &str,
    data: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO database_change_events(id,change_id,revision,action,actor_id,real_actor_id,data_json,created_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(otto_core::new_id()).bind(id).bind(revision).bind(action).bind(actor).bind(real).bind(encode(&data)?).bind(now()).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}
/// Includes executor and required approval policy, alongside exact normalized targets.
pub fn artifact_hash(
    change: &DatabaseChange,
    executor: &str,
    snapshots: &[TargetSnapshot],
) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(encode(
            &json!({"revision":change.revision,"script":change.script,"targets":snapshots,"executor":executor,"approval_policy":"independent-review-v1"})
        )?)
    ))
}
impl DatabaseChangesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn get(&self, id: &str) -> Result<DatabaseChange> {
        from_row(
            sqlx::query("SELECT * FROM database_changes WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?
                .ok_or_else(|| Error::NotFound("database change".into()))?,
        )
    }
    pub async fn list(&self, before: Option<&str>) -> Result<Vec<DatabaseChange>> {
        let rows=sqlx::query("SELECT * FROM database_changes WHERE (? IS NULL OR created_at<?) ORDER BY created_at DESC,id DESC LIMIT 100")
            .bind(before).bind(before).fetch_all(&self.pool).await.map_err(db)?;
        rows.into_iter().map(from_row).collect()
    }
    pub async fn create(
        &self,
        input: &ChangeInput,
        actor: &str,
        real: &str,
    ) -> Result<DatabaseChange> {
        let id = otto_core::new_id();
        let time = now();
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO database_changes(id,author_id,real_author_id,title,description,script,targets_json,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'draft',?,?)")
            .bind(&id).bind(actor).bind(real).bind(&input.title).bind(&input.description).bind(&input.script).bind(encode(&input.targets)?).bind(&time).bind(&time).execute(&mut *tx).await.map_err(db)?;
        event(&mut tx, &id, 1, "created", actor, real, json!(input)).await?;
        tx.commit().await.map_err(db)?;
        self.get(&id).await
    }
    pub async fn revise(
        &self,
        old: &DatabaseChange,
        input: &ChangeInput,
        actor: &str,
        real: &str,
    ) -> Result<DatabaseChange> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET title=?,description=?,script=?,targets_json=?,revision=revision+1,status='draft',content_hash='',executor_id=NULL,validation_json='{}',approved_by=NULL,approved_real_by=NULL,approval_hash=NULL,updated_at=? WHERE id=? AND revision=? AND status IN ('draft','validated','rejected','approved','awaiting_review')")
            .bind(&input.title).bind(&input.description).bind(&input.script).bind(encode(&input.targets)?).bind(now()).bind(&old.id).bind(old.revision).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        event(
            &mut tx,
            &old.id,
            old.revision + 1,
            "revised",
            actor,
            real,
            json!(input),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.get(&old.id).await
    }
    pub async fn validate(
        &self,
        old: &DatabaseChange,
        executor: &str,
        snapshots: &[TargetSnapshot],
        actor: &str,
        real: &str,
    ) -> Result<DatabaseChange> {
        let hash = artifact_hash(old, executor, snapshots)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET status='validated',content_hash=?,executor_id=?,validation_json=?,approved_by=NULL,approved_real_by=NULL,approval_hash=NULL,updated_at=? WHERE id=? AND revision=? AND status IN ('draft','validated','rejected')")
            .bind(&hash).bind(executor).bind(encode(&json!({"targets":snapshots,"policy":"independent-review-v1","note":"Preflight is not an execution guarantee"}))?).bind(now()).bind(&old.id).bind(old.revision).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        event(
            &mut tx,
            &old.id,
            old.revision,
            "validated",
            actor,
            real,
            json!({"hash":hash,"executor_id":executor}),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.get(&old.id).await
    }
    pub async fn transition(
        &self,
        old: &DatabaseChange,
        to: &str,
        actor: &str,
        real: &str,
        note: &str,
    ) -> Result<DatabaseChange> {
        let valid = matches!(
            (old.status.as_str(), to),
            ("validated", "awaiting_review")
                | ("awaiting_review", "approved")
                | ("awaiting_review", "rejected")
                | (
                    "draft" | "validated" | "awaiting_review" | "approved" | "rejected",
                    "cancelled"
                )
        );
        if !valid {
            return Err(conflict());
        }
        if to == "approved"
            && (actor == old.author_id
                || real == old.real_author_id
                || actor == old.real_author_id
                || real == old.author_id)
        {
            return Err(Error::Forbidden("independent approval required; authors cannot approve their own changes, including through impersonation".into()));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET status=?,approved_by=?,approved_real_by=?,approval_hash=?,updated_at=? WHERE id=? AND revision=? AND status=? AND content_hash=?")
            .bind(to).bind((to=="approved").then_some(actor)).bind((to=="approved").then_some(real)).bind((to=="approved").then_some(old.content_hash.as_str())).bind(now()).bind(&old.id).bind(old.revision).bind(&old.status).bind(&old.content_hash).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        event(
            &mut tx,
            &old.id,
            old.revision,
            to,
            actor,
            real,
            json!({"note":note,"content_hash":old.content_hash}),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.get(&old.id).await
    }
    /// Atomic claim plus one durable lock per target. Conflicting claims roll back
    /// completely; a second click cannot create another execution attempt.
    pub async fn claim(
        &self,
        old: &DatabaseChange,
        snapshots: &[TargetSnapshot],
        actor: &str,
        real: &str,
    ) -> Result<Vec<ChangeAttempt>> {
        if old.executor_id.as_deref() != Some(actor)
            || old.approval_hash.as_deref() != Some(&old.content_hash)
            || artifact_hash(old, actor, snapshots)? != old.content_hash
        {
            return Err(conflict());
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET status='running',updated_at=? WHERE id=? AND revision=? AND status='approved' AND content_hash=? AND approval_hash=content_hash AND executor_id=?")
            .bind(now()).bind(&old.id).bind(old.revision).bind(&old.content_hash).bind(actor).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        for (i, snapshot) in snapshots.iter().enumerate() {
            let inserted=sqlx::query("INSERT INTO database_change_attempts(id,change_id,connection_id,node,script,state,executor_id,content_hash,policy_revision,connection_fingerprint,ordinal) VALUES(?,?,?,?,?,'queued',?,?,?,?,?)")
                .bind(otto_core::new_id()).bind(&old.id).bind(&snapshot.target.connection_id).bind(&snapshot.target.node).bind(&old.script).bind(actor).bind(&old.content_hash).bind(snapshot.policy_revision).bind(&snapshot.connection_fingerprint).bind(i as i64).execute(&mut *tx).await;
            match inserted { Ok(_)=>{},Err(e) if e.as_database_error().is_some_and(|e|e.is_unique_violation())=>return Err(Error::Conflict("a target has an active or unresolved change; reconcile it before another execution".into())),Err(e)=>return Err(db(e)) }
        }
        event(
            &mut tx,
            &old.id,
            old.revision,
            "claimed",
            actor,
            real,
            json!({"hash":old.content_hash}),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.attempts(&old.id).await
    }
    pub async fn attempts(&self, id: &str) -> Result<Vec<ChangeAttempt>> {
        Ok(
            sqlx::query(
                "SELECT * FROM database_change_attempts WHERE change_id=? ORDER BY ordinal",
            )
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?
            .into_iter()
            .map(|r| ChangeAttempt {
                id: r.get("id"),
                change_id: r.get("change_id"),
                connection_id: r.get("connection_id"),
                node: r.get("node"),
                state: r.get("state"),
                executor_id: r.get("executor_id"),
                ordinal: r.get("ordinal"),
                summary: r.get("summary"),
                started_at: r.get("started_at"),
                finished_at: r.get("finished_at"),
            })
            .collect(),
        )
    }
    pub async fn start_attempt(&self, id: &str) -> Result<()> {
        let n=sqlx::query("UPDATE database_change_attempts SET state='running',started_at=? WHERE id=? AND state='queued' AND EXISTS(SELECT 1 FROM database_changes c WHERE c.id=change_id AND c.status='running' AND c.cancellation_requested=0)")
            .bind(now()).bind(id).execute(&self.pool).await.map_err(db)?.rows_affected();
        if n == 1 {
            Ok(())
        } else {
            Err(conflict())
        }
    }
    /// Errors after adapter invocation are unknown, even if they look like a
    /// timeout or syntax failure: a preceding DDL statement may have committed.
    pub async fn finish_attempt(&self, id: &str, succeeded: bool) -> Result<()> {
        self.finish_attempt_progress(id, succeeded, None).await
    }
    /// Persist progress counts, never SQL results or driver errors.
    pub async fn finish_attempt_progress(
        &self,
        id: &str,
        succeeded: bool,
        completed: Option<usize>,
    ) -> Result<()> {
        let summary=match (succeeded,completed) {
            (true,Some(n))=>format!("Execution completed: {n} statements confirmed."),
            (true,None)=>"Execution completed".into(),
            (false,Some(n))=>format!("{n} statements confirmed before the batch stopped. Inspect the database and reconcile; no automatic retry."),
            (false,None)=>"Execution did not confirm completion. Inspect the database and reconcile before retrying.".into(),
        };
        let n=sqlx::query("UPDATE database_change_attempts SET state=?,summary=?,finished_at=? WHERE id=? AND state='running'")
            .bind(if succeeded{"succeeded"}else{"outcome_unknown"}).bind(summary).bind(now()).bind(id).execute(&self.pool).await.map_err(db)?.rows_affected();
        if n == 1 {
            Ok(())
        } else {
            Err(conflict())
        }
    }
    pub async fn finish(&self, id: &str, actor: &str, real: &str) -> Result<DatabaseChange> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("UPDATE database_change_attempts SET state='cancelled',finished_at=? WHERE change_id=? AND state='queued'").bind(now()).bind(id).execute(&mut *tx).await.map_err(db)?;
        let status:String=sqlx::query_scalar("SELECT CASE WHEN SUM(state='outcome_unknown')>0 THEN 'outcome_unknown' WHEN SUM(state='succeeded')=COUNT(*) THEN 'succeeded' WHEN SUM(state IN ('succeeded','partially_applied'))>0 THEN 'partially_applied' WHEN (SELECT cancellation_requested FROM database_changes WHERE id=change_id)=1 THEN 'cancelled' ELSE 'failed' END FROM database_change_attempts WHERE change_id=?").bind(id).fetch_one(&mut *tx).await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET status=?,updated_at=? WHERE id=? AND status IN ('running','outcome_unknown')").bind(&status).bind(now()).bind(id).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM database_changes WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        event(&mut tx, id, revision, &status, actor, real, json!({})).await?;
        tx.commit().await.map_err(db)?;
        self.get(id).await
    }
    pub async fn request_cancel(
        &self,
        old: &DatabaseChange,
        actor: &str,
        real: &str,
    ) -> Result<DatabaseChange> {
        if old.status != "running" {
            return self.transition(old, "cancelled", actor, real, "").await;
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Invalidates the execution adapter's running claim; its polling loop
        // attempts native cancellation, but cannot promise to undo committed SQL.
        let n=sqlx::query("UPDATE database_changes SET cancellation_requested=1,status='outcome_unknown',updated_at=? WHERE id=? AND status='running'").bind(now()).bind(&old.id).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        sqlx::query("UPDATE database_change_attempts SET state='outcome_unknown',summary='Cancellation requested; inspect target before reconciliation' WHERE change_id=? AND state='running'").bind(&old.id).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE database_change_attempts SET state='cancelled',finished_at=? WHERE change_id=? AND state='queued'").bind(now()).bind(&old.id).execute(&mut *tx).await.map_err(db)?;
        event(
            &mut tx,
            &old.id,
            old.revision,
            "cancellation_requested",
            actor,
            real,
            json!({}),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.get(&old.id).await
    }
    pub async fn reconcile(
        &self,
        old: &DatabaseChange,
        attempt: &str,
        outcome: &str,
        note: &str,
        actor: &str,
        real: &str,
    ) -> Result<DatabaseChange> {
        if old.status != "outcome_unknown"
            || !matches!(outcome, "succeeded" | "failed" | "partially_applied")
            || note.trim().len() < 10
        {
            return Err(Error::Invalid("reconciliation requires an unknown attempt, succeeded/failed/partially_applied outcome, and evidence note of at least 10 characters".into()));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        let n=sqlx::query("UPDATE database_change_attempts SET state=?,summary=?,finished_at=? WHERE id=? AND change_id=? AND state='outcome_unknown'").bind(outcome).bind(note).bind(now()).bind(attempt).bind(&old.id).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if n != 1 {
            return Err(conflict());
        }
        event(
            &mut tx,
            &old.id,
            old.revision,
            "reconciled",
            actor,
            real,
            json!({"attempt_id":attempt,"outcome":outcome,"evidence":note}),
        )
        .await?;
        tx.commit().await.map_err(db)?;
        self.finish(&old.id, actor, real).await
    }
    /// Call once during daemon boot before accepting requests. Never resume SQL.
    pub async fn recover_interrupted(&self) -> Result<u64> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let interrupted=sqlx::query("SELECT id,revision FROM database_changes WHERE status='running' OR (status='outcome_unknown' AND cancellation_requested=1 AND NOT EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='outcome_unknown'))").fetch_all(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE database_change_attempts SET state='outcome_unknown',summary='Daemon restarted during execution; inspect target before reconciliation' WHERE state='running'").execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE database_change_attempts SET state='cancelled',finished_at=? WHERE state='queued'").bind(now()).execute(&mut *tx).await.map_err(db)?;
        let n=sqlx::query("UPDATE database_changes SET status=CASE WHEN EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='outcome_unknown') THEN 'outcome_unknown' WHEN EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='partially_applied') THEN 'partially_applied' WHEN EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='succeeded') AND EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state<>'succeeded') THEN 'partially_applied' WHEN EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='succeeded') THEN 'succeeded' WHEN cancellation_requested=1 THEN 'cancelled' ELSE 'failed' END,updated_at=? WHERE status='running' OR (status='outcome_unknown' AND cancellation_requested=1 AND NOT EXISTS(SELECT 1 FROM database_change_attempts a WHERE a.change_id=database_changes.id AND a.state='outcome_unknown'))").bind(now()).execute(&mut *tx).await.map_err(db)?.rows_affected();
        for change in interrupted {
            let id: String = change.get("id");
            let revision: i64 = change.get("revision");
            event(&mut tx,&id,revision,"recovered","system","system",json!({"note":"Daemon recovery: unsent work cancelled; potentially sent SQL requires reconciliation"})).await?;
        }
        tx.commit().await.map_err(db)?;
        Ok(n)
    }
    pub async fn history(&self, id: &str) -> Result<Vec<ChangeEvent>> {
        sqlx::query("SELECT * FROM database_change_events WHERE change_id=? ORDER BY created_at,id LIMIT 500").bind(id).fetch_all(&self.pool).await.map_err(db)?.into_iter().map(|r|Ok(ChangeEvent{id:r.get("id"),revision:r.get("revision"),action:r.get("action"),actor_id:r.get("actor_id"),real_actor_id:r.get("real_actor_id"),data:decode(r.get("data_json"))?,created_at:r.get("created_at")})).collect()
    }
}
#[cfg(test)]
#[path = "database_changes_tests.rs"]
mod tests;

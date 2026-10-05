//! Repository for workflow triggers (schedule / webhook / event / chat kinds).
//!
//! Mirrors the pattern used by [`crate::workflows::WorkflowsRepo`]: thin data
//! layer, all SQL inline, types re-exported from `otto_core`.

use crate::DbPool;
use chrono::Utc;
use otto_core::{new_id, Error, Id, Result};
use serde_json::Value;
use sqlx::Row;

use crate::convert::{dberr, ts};

/// A persisted workflow trigger row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowTrigger {
    pub id: Id,
    pub workflow_id: Id,
    /// "schedule" | "webhook" | "event" | "chat" — a `chat` spec is
    /// `{channel, chat, thread?, mention_only?}` and starts a run from a
    /// message posted in the bound channel/chat(/thread).
    pub kind: String,
    /// Kind-specific configuration (JSON object).
    pub spec: Value,
    pub enabled: bool,
    /// Internal eligibility epoch captured by scheduler scans, not a wire field.
    #[serde(skip)]
    pub admission_generation: i64,
    pub created_at: chrono::DateTime<Utc>,
    /// When the schedule was last (re)armed — created, re-enabled after a
    /// pause, or its cadence/timezone/expression changed. The scheduler's
    /// due check never looks before it (a resumed trigger doesn't fire the
    /// run it missed while off). `None` on rows predating migration 0151.
    #[serde(default)]
    pub armed_at: Option<chrono::DateTime<Utc>>,
}

/// Fields required to create a trigger.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewWorkflowTrigger {
    pub workflow_id: Id,
    pub kind: String,
    pub spec: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn row_to_trigger(r: &sqlx::sqlite::SqliteRow) -> Result<WorkflowTrigger> {
    let spec: Value = serde_json::from_str(&r.get::<String, _>("spec_json"))
        .unwrap_or(Value::Object(Default::default()));
    Ok(WorkflowTrigger {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        kind: r.get("kind"),
        spec,
        enabled: r.get::<i64, _>("enabled") != 0,
        admission_generation: r.get("admission_generation"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        armed_at: r
            .get::<Option<String>, _>("armed_at")
            .and_then(|s| ts(&s).ok()),
    })
}

/// The spec keys that define WHEN a schedule trigger fires. A change to any of
/// them re-arms it; prompt / destination edits don't.
const SCHEDULE_KEYS: [&str; 7] = [
    "cadence",
    "every_min",
    "at",
    "weekday",
    "expr",
    "timezone",
    "run_at",
];

/// Whether an update re-arms the trigger: resumed after a pause, or one of
/// its [`SCHEDULE_KEYS`] changed.
fn rearms(was_enabled: bool, enabled: bool, old_spec: &Value, new_spec: &Value) -> bool {
    let resumed = !was_enabled && enabled;
    let respec = SCHEDULE_KEYS
        .iter()
        .any(|k| old_spec.get(k) != new_spec.get(k));
    resumed || respec
}

#[derive(Clone)]
pub struct TriggersRepo {
    pool: DbPool,
}

impl TriggersRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// List all triggers for a workflow, ordered oldest-first.
    pub async fn list(&self, workflow_id: &Id) -> Result<Vec<WorkflowTrigger>> {
        let rows = sqlx::query(
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at, armed_at, admission_generation
             FROM workflow_triggers
             WHERE workflow_id = ?
             ORDER BY created_at",
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list triggers"))?;

        rows.iter().map(row_to_trigger).collect()
    }

    /// List all enabled triggers of a specific kind across ALL workflows.
    /// Used by the scheduler to find `schedule` triggers that are due.
    pub async fn list_enabled_by_kind(&self, kind: &str) -> Result<Vec<WorkflowTrigger>> {
        let rows = sqlx::query(
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at, armed_at, admission_generation
             FROM workflow_triggers
             WHERE kind = ? AND enabled = 1
             ORDER BY created_at",
        )
        .bind(kind)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list triggers by kind"))?;

        rows.iter().map(row_to_trigger).collect()
    }

    /// Fetch a single trigger by id.
    pub async fn get(&self, id: &Id) -> Result<WorkflowTrigger> {
        let row = sqlx::query(
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at, armed_at, admission_generation
             FROM workflow_triggers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get trigger"))?
        .ok_or_else(|| Error::NotFound("trigger".into()))?;

        row_to_trigger(&row)
    }

    /// Look up a webhook trigger by its token (stored in spec_json.token).
    /// Returns the first match (tokens should be globally unique).
    pub async fn find_webhook(&self, workflow_id: &Id, token: &str) -> Result<WorkflowTrigger> {
        let rows = self.list(workflow_id).await?;
        rows.into_iter()
            .find(|t| {
                t.kind == "webhook"
                    && t.enabled
                    && t.spec.get("token").and_then(Value::as_str) == Some(token)
            })
            .ok_or_else(|| Error::NotFound("webhook trigger".into()))
    }

    /// Create a new trigger. Returns the created row.
    pub async fn create(&self, new: NewWorkflowTrigger) -> Result<WorkflowTrigger> {
        let id = new_id();
        let spec_json = serde_json::to_string(&new.spec)
            .map_err(|e| Error::Internal(format!("spec serialize: {e}")))?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workflow_triggers
                 (id, workflow_id, kind, spec_json, enabled, created_at, armed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.workflow_id)
        .bind(&new.kind)
        .bind(&spec_json)
        .bind(new.enabled as i64)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create trigger"))?;

        self.get(&id).await
    }

    /// Update the spec and/or enabled flag of a trigger.
    pub async fn update(
        &self,
        id: &Id,
        spec: Option<Value>,
        enabled: Option<bool>,
    ) -> Result<WorkflowTrigger> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin trigger edit"))?;
        let row = sqlx::query("SELECT * FROM workflow_triggers WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("get trigger for edit"))?;
        let current = row_to_trigger(&row)?;
        let mut new_spec = spec.unwrap_or_else(|| current.spec.clone());
        let new_enabled = enabled.unwrap_or(current.enabled);
        let changed = SCHEDULE_KEYS
            .iter()
            .any(|k| current.spec.get(k) != new_spec.get(k));
        let reset_once = current.kind == "schedule"
            && new_spec.get("cadence").and_then(Value::as_str) == Some("once")
            && changed;
        let armed_at = rearms(current.enabled, new_enabled, &current.spec, &new_spec)
            .then(|| Utc::now().to_rfc3339());
        // The write lock protects both schedule comparison and cursor ownership.
        // Ordinary edits/resume retain the stored fired flag, never a client one;
        // a distinct one-shot occurrence (including timezone changes) clears it.
        if let Some(object) = new_spec.as_object_mut() {
            object.remove("last_run");
            if !reset_once {
                if let Some(cursor) = current.spec.get("last_run") {
                    object.insert("last_run".into(), cursor.clone());
                }
            }
        }
        let eligibility_changed = changed || current.enabled != new_enabled;
        let row=sqlx::query("UPDATE workflow_triggers SET spec_json=?,enabled=?,armed_at=COALESCE(?,armed_at),admission_generation=admission_generation+? WHERE id=? RETURNING *")
            .bind(new_spec.to_string()).bind(new_enabled as i64).bind(armed_at).bind(i64::from(eligibility_changed)).bind(id)
            .fetch_one(&mut *tx).await.map_err(dberr("update trigger"))?;
        let updated = row_to_trigger(&row)?;
        tx.commit().await.map_err(dberr("commit trigger edit"))?;
        Ok(updated)
    }

    /// Admit one captured schedule tick. Cursor claim and queued run (including
    /// its immutable workflow version) commit together. Re-check active runs
    /// inside the write transaction, since the scheduler's early check may age.
    pub async fn claim_scheduled_run(
        &self,
        captured: &WorkflowTrigger,
        workspace_id: &Id,
        input: &Value,
        now: &str,
    ) -> Result<Option<otto_core::workflows::WorkflowRun>> {
        if !captured.enabled || captured.kind != "schedule" {
            return Ok(None);
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled workflow admission"))?;
        let claimed = sqlx::query(
            "UPDATE workflow_triggers SET spec_json=json_set(spec_json,'$.last_run',?)
             WHERE id=? AND workflow_id=? AND kind='schedule' AND enabled=1
             AND admission_generation=? AND json_extract(spec_json,'$.last_run') IS ?
             AND EXISTS(SELECT 1 FROM workflows WHERE id=? AND workspace_id=?)
             AND NOT EXISTS(SELECT 1 FROM workflow_runs WHERE workflow_id=? AND status IN ('pending','running'))",
        )
        .bind(now).bind(&captured.id).bind(&captured.workflow_id)
        .bind(captured.admission_generation)
        .bind(captured.spec.get("last_run").and_then(Value::as_str))
        .bind(&captured.workflow_id).bind(workspace_id).bind(&captured.workflow_id)
        .execute(&mut *tx).await.map_err(dberr("claim scheduled workflow occurrence"))?
        .rows_affected();
        if claimed == 0 {
            tx.commit()
                .await
                .map_err(dberr("finish stale workflow admission"))?;
            return Ok(None);
        }
        // An insertion/projection failure drops the transaction and rolls back
        // the cursor too; the occurrence remains available to a later tick.
        let run = crate::workflows::WorkflowsRepo::insert_run(
            &mut tx,
            &captured.workflow_id,
            workspace_id,
            input,
            None,
        )
        .await?;
        tx.commit()
            .await
            .map_err(dberr("commit scheduled workflow admission"))?;
        Ok(Some(run))
    }

    /// Fixture-only cursor seeding. Production uses the atomic run claim.
    #[cfg(test)]
    pub async fn set_last_run(&self, id: &Id, last_run: &str) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_triggers SET spec_json = json_set(spec_json, '$.last_run', ?)
             WHERE id = ?",
        )
        .bind(last_run)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("advance trigger cursor"))?;
        Ok(())
    }

    /// Delete a trigger by id.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM workflow_triggers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete trigger"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::WorkflowsRepo;
    use otto_core::workflows::WorkflowGraph;

    /// Verify that `default_true` returns `true` so newly-created triggers are
    /// enabled by default when the field is absent from the request JSON.
    #[test]
    fn default_enabled_is_true() {
        assert!(default_true());
    }

    /// A trigger `spec` that is not valid JSON falls back to an empty object
    /// rather than surfacing an error (defensive parse in `row_to_trigger`).
    #[test]
    fn bad_spec_falls_back_to_empty_object() {
        // Simulate the fallback path used in row_to_trigger for corrupt rows.
        let parsed: Value =
            serde_json::from_str("not-json").unwrap_or(Value::Object(Default::default()));
        assert!(parsed.as_object().is_some());
    }

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

    #[tokio::test]
    async fn review4_retimed_once_rearms_without_trusting_client_cursor() {
        let pool = mem_pool().await;
        let workflows = WorkflowsRepo::new(pool.clone());
        let repo = TriggersRepo::new(pool);
        let wf = workflows
            .create(
                &"ws".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u".into(),
            )
            .await
            .unwrap();
        for old in [
            serde_json::json!({"cadence":"once","run_at":"2026-10-05T10:00:00","timezone":"UTC"}),
            serde_json::json!({"cadence":"interval","every_min":60}),
            serde_json::json!({"cadence":"once","run_at":"2026-10-05T11:00:00","timezone":"Europe/London"}),
        ] {
            let trigger = repo
                .create(NewWorkflowTrigger {
                    workflow_id: wf.id.clone(),
                    kind: "schedule".into(),
                    spec: old,
                    enabled: true,
                })
                .await
                .unwrap();
            repo.set_last_run(&trigger.id, "2026-10-05T10:00:00Z")
                .await
                .unwrap();
            let next = serde_json::json!({"cadence":"once","run_at":"2026-10-05T11:00:00","timezone":"UTC","last_run":"1900-01-01T00:00:00Z"});
            let retimed = repo
                .update(&trigger.id, Some(next.clone()), None)
                .await
                .unwrap();
            assert!(
                retimed.spec.get("last_run").is_none(),
                "new one-shot occurrence retains its old fired flag: {}",
                retimed.spec
            );
            repo.set_last_run(&trigger.id, "2026-10-05T11:00:00Z")
                .await
                .unwrap();
            let mut ordinary = next;
            ordinary["prompt"] = serde_json::json!("new prompt");
            let resaved = repo
                .update(&trigger.id, Some(ordinary), Some(false))
                .await
                .unwrap();
            assert_eq!(resaved.spec["last_run"], "2026-10-05T11:00:00Z");
            let resumed = repo.update(&trigger.id, None, Some(true)).await.unwrap();
            assert_eq!(
                resumed.spec["last_run"], "2026-10-05T11:00:00Z",
                "resuming the same occurrence must not duplicate it"
            );
        }
    }

    /// Perf W12 budget: the workflow-trigger scheduler's per-minute scan is
    /// ONE statement for any number of schedule triggers (a tick with nothing
    /// due does nothing else).
    #[tokio::test]
    async fn schedule_scan_is_one_query_for_any_n() {
        let pool = mem_pool().await;
        let workflows = WorkflowsRepo::new(pool.clone());
        let repo = TriggersRepo::new(pool.clone());
        let wf = workflows
            .create(
                &"ws1".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        for _ in 0..8 {
            repo.create(NewWorkflowTrigger {
                workflow_id: wf.id.clone(),
                kind: "schedule".into(),
                spec: serde_json::json!({"cron": "0 9 * * *"}),
                enabled: true,
            })
            .await
            .unwrap();
        }
        let probe = pool.statement_probe();
        probe.reset();
        assert_eq!(
            repo.list_enabled_by_kind("schedule").await.unwrap().len(),
            8
        );
        assert_eq!(probe.take().len(), 1);
    }

    /// Regression for the `chat` trigger kind: it was added to route
    /// validation (`create_trigger` in otto-server) and documented, but the
    /// original 0058 `workflow_triggers.kind` CHECK only allowed
    /// `schedule|webhook|event`, so a `chat` INSERT failed at the DB layer.
    /// Covers every kind (not just `chat`) so a regression in the widened
    /// CHECK for any of them is caught here too, and round-trips the chat
    /// binding spec shape (`{channel, chat, mention_only}`).
    #[tokio::test]
    async fn create_accepts_every_trigger_kind_including_chat() {
        let pool = mem_pool().await;
        let workflows = WorkflowsRepo::new(pool.clone());
        let repo = TriggersRepo::new(pool);

        let graph = WorkflowGraph::default();
        let wf = workflows
            .create(&"ws1".into(), "WF", "desc", "", &graph, &"u1".into())
            .await
            .unwrap();

        let chat_spec = serde_json::json!({
            "channel": "slack",
            "chat": "C1",
            "mention_only": false,
        });

        for kind in ["schedule", "webhook", "event", "chat"] {
            let spec = if kind == "chat" {
                chat_spec.clone()
            } else {
                serde_json::json!({})
            };
            let created = repo
                .create(NewWorkflowTrigger {
                    workflow_id: wf.id.clone(),
                    kind: kind.to_string(),
                    spec: spec.clone(),
                    enabled: true,
                })
                .await
                .unwrap_or_else(|e| panic!("create trigger kind={kind} failed: {e}"));
            assert_eq!(created.kind, kind);
            assert_eq!(created.spec, spec, "spec round-trips for kind={kind}");
        }
    }

    /// F7: a config edit never rolls back (or wipes) the server-owned
    /// `last_run` cursor, and advancing the cursor never clobbers the config.
    #[tokio::test]
    async fn trigger_cursor_is_server_owned() {
        let pool = mem_pool().await;
        let workflows = WorkflowsRepo::new(pool.clone());
        let repo = TriggersRepo::new(pool);
        let wf = workflows
            .create(
                &"ws1".into(),
                "WF",
                "desc",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let t = repo
            .create(NewWorkflowTrigger {
                workflow_id: wf.id.clone(),
                kind: "schedule".into(),
                spec: serde_json::json!({"cadence": "daily", "at": "09:00"}),
                enabled: true,
            })
            .await
            .unwrap();
        // A client can't plant a cursor the server never set.
        let t2 = repo
            .update(
                &t.id,
                Some(serde_json::json!({"cadence": "daily", "at": "09:00", "last_run": "2020-01-01T00:00:00+00:00"})),
                None,
            )
            .await
            .unwrap();
        assert!(t2.spec.get("last_run").is_none());
        // The scheduler fires: only the cursor moves.
        repo.set_last_run(&t.id, "2026-10-25T06:00:00+00:00")
            .await
            .unwrap();
        // A stale edit (yesterday's cursor, new time) keeps today's cursor.
        let t3 = repo
            .update(
                &t.id,
                Some(serde_json::json!({"cadence": "daily", "at": "10:00", "last_run": "2026-10-24T06:00:00+00:00"})),
                None,
            )
            .await
            .unwrap();
        assert_eq!(t3.spec["at"], "10:00");
        assert_eq!(t3.spec["last_run"], "2026-10-25T06:00:00+00:00");
        // An edit without a cursor doesn't wipe it either.
        let t4 = repo
            .update(
                &t.id,
                Some(serde_json::json!({"cadence": "daily", "at": "11:00"})),
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(t4.spec["last_run"], "2026-10-25T06:00:00+00:00");
        assert!(!t4.enabled);
        // Advancing the cursor leaves that edit in place.
        repo.set_last_run(&t.id, "2026-10-26T06:00:00+00:00")
            .await
            .unwrap();
        let t5 = repo.get(&t.id).await.unwrap();
        assert_eq!(t5.spec["at"], "11:00");
        assert_eq!(t5.spec["last_run"], "2026-10-26T06:00:00+00:00");
    }

    /// W1: a trigger is armed at creation and re-armed only when it's resumed
    /// or its schedule really changes — not on a prompt edit or a re-save.
    #[tokio::test]
    async fn triggers_are_armed_at_creation_and_on_resume_or_reschedule() {
        let pool = mem_pool().await;
        let workflows = WorkflowsRepo::new(pool.clone());
        let repo = TriggersRepo::new(pool.clone());
        let wf = workflows
            .create(
                &"ws1".into(),
                "WF",
                "desc",
                "",
                &WorkflowGraph::default(),
                &"u1".into(),
            )
            .await
            .unwrap();
        let spec = serde_json::json!({"cadence": "daily", "at": "09:00"});
        let t = repo
            .create(NewWorkflowTrigger {
                workflow_id: wf.id.clone(),
                kind: "schedule".into(),
                spec: spec.clone(),
                enabled: true,
            })
            .await
            .unwrap();
        assert_eq!(t.armed_at, Some(t.created_at));

        let old = "2020-01-01T00:00:00+00:00";
        let set_armed = |v: &'static str| {
            let pool = pool.clone();
            let id = t.id.clone();
            async move {
                sqlx::query("UPDATE workflow_triggers SET armed_at = ? WHERE id = ?")
                    .bind(v)
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };
        set_armed(old).await;
        let old_ts = ts(old).unwrap();

        // A prompt edit with the same cadence keeps the arm instant.
        let t2 = repo
            .update(
                &t.id,
                Some(serde_json::json!({"cadence": "daily", "at": "09:00", "prompt": "hi"})),
                None,
            )
            .await
            .unwrap();
        assert_eq!(t2.armed_at, Some(old_ts));
        // Pausing doesn't re-arm.
        let t3 = repo.update(&t.id, None, Some(false)).await.unwrap();
        assert_eq!(t3.armed_at, Some(old_ts));
        // Resuming does.
        let t4 = repo.update(&t.id, None, Some(true)).await.unwrap();
        assert!(t4.armed_at.unwrap() > old_ts);
        // A new time does.
        set_armed(old).await;
        let t5 = repo
            .update(
                &t.id,
                Some(serde_json::json!({"cadence": "daily", "at": "10:00", "prompt": "hi"})),
                None,
            )
            .await
            .unwrap();
        assert!(t5.armed_at.unwrap() > old_ts);
    }

    #[test]
    fn rearms_only_on_resume_or_schedule_key_change() {
        let a = serde_json::json!({"cadence": "interval", "every_min": 30, "prompt": "x"});
        let b = serde_json::json!({"cadence": "interval", "every_min": 30, "prompt": "y"});
        let c = serde_json::json!({"cadence": "interval", "every_min": 60});
        assert!(!rearms(true, true, &a, &b));
        assert!(!rearms(true, false, &a, &a));
        assert!(rearms(false, true, &a, &a));
        assert!(rearms(true, true, &a, &c));
    }
}

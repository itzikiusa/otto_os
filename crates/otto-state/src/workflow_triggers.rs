//! Repository for workflow triggers (schedule / webhook / event kinds).
//!
//! Mirrors the pattern used by [`crate::workflows::WorkflowsRepo`]: thin data
//! layer, all SQL inline, types re-exported from `otto_core`.

use chrono::Utc;
use otto_core::{new_id, Error, Id, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, ts};

/// A persisted workflow trigger row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowTrigger {
    pub id: Id,
    pub workflow_id: Id,
    /// "schedule" | "webhook" | "event"
    pub kind: String,
    /// Kind-specific configuration (JSON object).
    pub spec: Value,
    pub enabled: bool,
    pub created_at: chrono::DateTime<Utc>,
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
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

#[derive(Clone)]
pub struct TriggersRepo {
    pool: SqlitePool,
}

impl TriggersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// List all triggers for a workflow, ordered oldest-first.
    pub async fn list(&self, workflow_id: &Id) -> Result<Vec<WorkflowTrigger>> {
        let rows = sqlx::query(
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at
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
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at
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
            "SELECT id, workflow_id, kind, spec_json, enabled, created_at
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
            "INSERT INTO workflow_triggers (id, workflow_id, kind, spec_json, enabled, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.workflow_id)
        .bind(&new.kind)
        .bind(&spec_json)
        .bind(new.enabled as i64)
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
        let current = self.get(id).await?;
        let new_spec = spec.unwrap_or(current.spec.clone());
        let new_enabled = enabled.unwrap_or(current.enabled);
        let spec_json = serde_json::to_string(&new_spec)
            .map_err(|e| Error::Internal(format!("spec serialize: {e}")))?;
        sqlx::query(
            "UPDATE workflow_triggers SET spec_json = ?, enabled = ? WHERE id = ?",
        )
        .bind(&spec_json)
        .bind(new_enabled as i64)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update trigger"))?;

        self.get(id).await
    }

    /// Update only the spec (used by the scheduler to advance `last_run`).
    pub async fn set_spec(&self, id: &Id, spec: Value) -> Result<()> {
        let spec_json = serde_json::to_string(&spec)
            .map_err(|e| Error::Internal(format!("spec serialize: {e}")))?;
        sqlx::query("UPDATE workflow_triggers SET spec_json = ? WHERE id = ?")
            .bind(&spec_json)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set_spec trigger"))?;
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
}

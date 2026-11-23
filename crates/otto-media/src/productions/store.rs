//! SQLite persistence of productions (`media_productions`, migrations 0223, 0227-0230).

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use sqlx::Row;

use super::{Production, ProductionSummary};
use crate::store::MediaStore;

fn db(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("production timestamp '{s}': {e}")))
}

fn row_production(r: &sqlx::sqlite::SqliteRow) -> Result<Production> {
    let json = |col: &str| -> Result<serde_json::Value> {
        serde_json::from_str(&r.get::<String, _>(col))
            .map_err(|e| Error::Internal(format!("production {col}: {e}")))
    };
    let id: String = r.get("id");
    Ok(Production {
        project_id: r
            .get::<Option<String>, _>("project_id")
            .unwrap_or(id.clone()),
        project_name: None,
        project_pieces: Vec::new(),
        id,
        workspace_id: r.get("workspace_id"),
        description: r.get("description"),
        use_for: r.get("use_for"),
        status: r.get("status"),
        message: r.get("message"),
        eta_secs: None,
        pictures_eta_secs: None,
        scene_eta_secs: None,
        eta_unreused_secs: None,
        plan: serde_json::from_value(json("plan_json")?).unwrap_or_default(),
        scene_artifact_id: r.get("scene_artifact_id"),
        revisions: serde_json::from_value(json("revisions_json")?).unwrap_or_default(),
        provider: r.get("provider"),
        model: r.get("model"),
        effort: r.get("effort"),
        queued_at: r
            .get::<Option<String>, _>("queued_at")
            .map(|s| ts(&s))
            .transpose()?,
        agent: None,
        queue_position: None,
        scene_ready: false,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

impl MediaStore {
    /// Insert or replace the whole production row.
    pub async fn save_production(&self, p: &Production) -> Result<()> {
        let plan = serde_json::to_string(&p.plan)?;
        let revisions = serde_json::to_string(&p.revisions)?;
        sqlx::query(
            "INSERT OR REPLACE INTO media_productions
             (id, workspace_id, description, use_for, status, message, plan_json,
              revisions_json, scene_artifact_id, provider, model, effort, queued_at,
              created_by, created_at, updated_at, project_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.workspace_id)
        .bind(&p.description)
        .bind(&p.use_for)
        .bind(&p.status)
        .bind(&p.message)
        .bind(plan)
        .bind(revisions)
        .bind(&p.scene_artifact_id)
        .bind(&p.provider)
        .bind(&p.model)
        .bind(&p.effort)
        .bind(p.queued_at.map(|t| t.to_rfc3339()))
        .bind(&p.created_by)
        .bind(p.created_at.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(&p.project_id)
        .execute(self.pool())
        .await
        .map_err(db("media.productions.save"))?;
        Ok(())
    }

    pub async fn require_production(&self, id: &str) -> Result<Production> {
        let row = sqlx::query("SELECT * FROM media_productions WHERE id = ?")
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(db("media.productions.get"))?
            .ok_or_else(|| Error::NotFound(format!("production {id}")))?;
        row_production(&row)
    }

    /// The productions of these workspaces, newest first, at most `limit`,
    /// without plans (the title and item counts are read out of the plan).
    pub async fn list_productions(
        &self,
        workspace_ids: &[String],
        limit: i64,
    ) -> Result<Vec<ProductionSummary>> {
        if workspace_ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; workspace_ids.len()].join(", ");
        let sql = format!(
            "SELECT id, workspace_id, description, use_for, status, message, plan_json,
                    scene_artifact_id, created_at, updated_at, project_id
             FROM media_productions WHERE workspace_id IN ({marks})
             ORDER BY created_at DESC, id DESC LIMIT ?"
        );
        // Only `?` placeholders are formatted in; every value is bound.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));
        for id in workspace_ids {
            q = q.bind(id);
        }
        let rows = q
            .bind(limit)
            .fetch_all(self.pool())
            .await
            .map_err(db("media.productions.list"))?;
        rows.iter()
            .map(|r| {
                let plan: super::Plan =
                    serde_json::from_str(&r.get::<String, _>("plan_json")).unwrap_or_default();
                let enabled = plan.items.iter().filter(|i| i.enabled);
                let scene: Option<String> = r.get("scene_artifact_id");
                let results = plan
                    .items
                    .iter()
                    .flat_map(|i| [&i.result, &i.previous_result]);
                let made = results.flatten().flat_map(|r| [&r.artifact_id, &r.clip_id]);
                let pieces = scene.iter().chain(made.flatten()).cloned().collect();
                let id: String = r.get("id");
                Ok(ProductionSummary {
                    project_id: r
                        .get::<Option<String>, _>("project_id")
                        .unwrap_or(id.clone()),
                    project_name: None,
                    id,
                    workspace_id: r.get("workspace_id"),
                    description: r.get("description"),
                    use_for: r.get("use_for"),
                    status: r.get("status"),
                    message: r.get("message"),
                    items_done: enabled.clone().filter(|i| i.status == "done").count() as i64,
                    items_total: enabled.count() as i64,
                    title: plan.title.clone(),
                    scene_artifact_id: scene,
                    pieces,
                    workspace_name: None,
                    queue_position: None,
                    created_at: ts(&r.get::<String, _>("created_at"))?,
                    updated_at: ts(&r.get::<String, _>("updated_at"))?,
                })
            })
            .collect()
    }

    /// The creations of a project (its first one and every one started in
    /// it), newest first.
    pub async fn project_productions(&self, project_id: &str) -> Result<Vec<Production>> {
        let rows = sqlx::query(
            "SELECT * FROM media_productions WHERE id = ?1 OR project_id = ?1
             ORDER BY created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(self.pool())
        .await
        .map_err(db("media.productions.project"))?;
        rows.iter().map(row_production).collect()
    }

    /// Productions a previous daemon life left planning or running.
    pub async fn interrupted_productions(&self) -> Result<Vec<Production>> {
        let rows =
            sqlx::query("SELECT * FROM media_productions WHERE status IN ('planning', 'running')")
                .fetch_all(self.pool())
                .await
                .map_err(db("media.productions.interrupted"))?;
        rows.iter().map(row_production).collect()
    }
}

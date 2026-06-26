//! Persistence for **Proof Packs** (migration `0077_proof_packs.sql`).
//!
//! Stores packs and their evidence artifacts. The derived `status`/`risk_score`
//! are written by the server engine (`otto_server::proof`) after each mutation;
//! this repo is pure storage. `ProofArtifact.metadata` is the `metadata_json`
//! TEXT column; timestamps are RFC3339 strings.

use otto_core::proof::{
    ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack, ProofStatus, WorkItemKind,
};
use otto_core::{new_id, Error, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json};
use chrono::Utc;

#[derive(Clone)]
pub struct ProofRepo {
    pool: SqlitePool,
}

// --- Row mapping -----------------------------------------------------------

fn row_to_pack(r: &sqlx::sqlite::SqliteRow) -> Result<ProofPack> {
    let kind_raw: String = r.get("work_item_kind");
    let status_raw: String = r.get("status");
    Ok(ProofPack {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        work_item_kind: WorkItemKind::parse(&kind_raw)
            .ok_or_else(|| Error::Internal(format!("bad work_item_kind '{kind_raw}'")))?,
        work_item_id: r.get("work_item_id"),
        title: r.get("title"),
        status: ProofStatus::parse(&status_raw)
            .ok_or_else(|| Error::Internal(format!("bad proof status '{status_raw}'")))?,
        summary: r.get("summary"),
        risk_score: r.get::<i64, _>("risk_score").clamp(0, 100) as u8,
        parent_pack_id: r.get("parent_pack_id"),
        waived_by: r.get("waived_by"),
        waived_reason: r.get("waived_reason"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

fn row_to_artifact(r: &sqlx::sqlite::SqliteRow) -> Result<ProofArtifact> {
    let kind_raw: String = r.get("kind");
    let status_raw: String = r.get("status");
    let meta_raw: String = r.get("metadata_json");
    Ok(ProofArtifact {
        id: r.get("id"),
        proof_pack_id: r.get("proof_pack_id"),
        workspace_id: r.get("workspace_id"),
        kind: ProofArtifactKind::parse(&kind_raw)
            .ok_or_else(|| Error::Internal(format!("bad artifact kind '{kind_raw}'")))?,
        title: r.get("title"),
        content_ref: r.get("content_ref"),
        status: ProofArtifactStatus::parse(&status_raw)
            .ok_or_else(|| Error::Internal(format!("bad artifact status '{status_raw}'")))?,
        metadata: json(&meta_raw).unwrap_or(Value::Null),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

impl ProofRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Packs ---------------------------------------------------------------

    /// Create a pack. Fails if one already exists for the work item.
    pub async fn create_pack(
        &self,
        workspace_id: &str,
        kind: WorkItemKind,
        work_item_id: &str,
        title: &str,
        created_by: &str,
        parent: Option<&str>,
    ) -> Result<ProofPack> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO proof_packs (id, workspace_id, work_item_kind, work_item_id, title, \
             status, summary, risk_score, parent_pack_id, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'missing', '', 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(kind.as_str())
        .bind(work_item_id)
        .bind(title)
        .bind(parent)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create proof pack"))?;
        self.get_pack(&id).await
    }

    /// Ensure a pack exists for the work item, creating one if absent (the
    /// idempotent gate entry point). Returns the existing or new pack.
    pub async fn ensure_pack(
        &self,
        workspace_id: &str,
        kind: WorkItemKind,
        work_item_id: &str,
        title: &str,
        created_by: &str,
    ) -> Result<ProofPack> {
        if let Some(p) = self.find_by_work_item(kind, work_item_id).await? {
            return Ok(p);
        }
        match self
            .create_pack(workspace_id, kind, work_item_id, title, created_by, None)
            .await
        {
            Ok(p) => Ok(p),
            // Lost a race against a concurrent create — return the winner.
            Err(_) => self
                .find_by_work_item(kind, work_item_id)
                .await?
                .ok_or_else(|| Error::Internal("ensure_pack: pack vanished".into())),
        }
    }

    pub async fn get_pack(&self, id: &str) -> Result<ProofPack> {
        let row = sqlx::query("SELECT * FROM proof_packs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get proof pack"))?;
        row_to_pack(&row)
    }

    pub async fn find_by_work_item(
        &self,
        kind: WorkItemKind,
        work_item_id: &str,
    ) -> Result<Option<ProofPack>> {
        let row = sqlx::query(
            "SELECT * FROM proof_packs WHERE work_item_kind = ? AND work_item_id = ? LIMIT 1",
        )
        .bind(kind.as_str())
        .bind(work_item_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find proof pack by work item"))?;
        row.as_ref().map(row_to_pack).transpose()
    }

    /// List packs in a workspace, optionally filtered by status / kind / work item.
    pub async fn list_packs(
        &self,
        workspace_id: &str,
        status: Option<&str>,
        kind: Option<&str>,
        work_item_id: Option<&str>,
    ) -> Result<Vec<ProofPack>> {
        let mut sql = String::from("SELECT * FROM proof_packs WHERE workspace_id = ?");
        if status.is_some() {
            sql.push_str(" AND status = ?");
        }
        if kind.is_some() {
            sql.push_str(" AND work_item_kind = ?");
        }
        if work_item_id.is_some() {
            sql.push_str(" AND work_item_id = ?");
        }
        sql.push_str(" ORDER BY updated_at DESC");
        let mut q = sqlx::query(&sql).bind(workspace_id);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(k) = kind {
            q = q.bind(k);
        }
        if let Some(w) = work_item_id {
            q = q.bind(w);
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list proof packs"))?;
        rows.iter().map(row_to_pack).collect()
    }

    pub async fn list_children(&self, parent_id: &str) -> Result<Vec<ProofPack>> {
        let rows = sqlx::query(
            "SELECT * FROM proof_packs WHERE parent_pack_id = ? ORDER BY updated_at DESC",
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list child proof packs"))?;
        rows.iter().map(row_to_pack).collect()
    }

    pub async fn update_meta(
        &self,
        id: &str,
        title: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET \
               title = COALESCE(?, title), \
               summary = COALESCE(?, summary), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(title)
        .bind(summary)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update proof pack meta"))?;
        Ok(())
    }

    /// Persist the derived status + risk (the engine computes them).
    pub async fn set_status_risk(&self, id: &str, status: ProofStatus, risk: u8) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET status = ?, risk_score = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(risk as i64)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set proof pack status/risk"))?;
        Ok(())
    }

    /// Waive a pack (human override). Sets status=waived + records who/why.
    pub async fn waive(&self, id: &str, by: &str, reason: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET status = 'waived', waived_by = ?, waived_reason = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(by)
        .bind(reason)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("waive proof pack"))?;
        Ok(())
    }

    pub async fn delete_pack(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM proof_packs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete proof pack"))?;
        Ok(())
    }

    // -- Artifacts -----------------------------------------------------------

    pub async fn list_artifacts(&self, pack_id: &str) -> Result<Vec<ProofArtifact>> {
        let rows = sqlx::query(
            "SELECT * FROM proof_artifacts WHERE proof_pack_id = ? ORDER BY created_at ASC",
        )
        .bind(pack_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list proof artifacts"))?;
        rows.iter().map(row_to_artifact).collect()
    }

    pub async fn get_artifact(&self, id: &str) -> Result<ProofArtifact> {
        let row = sqlx::query("SELECT * FROM proof_artifacts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get proof artifact"))?;
        row_to_artifact(&row)
    }

    /// Insert an artifact (always a new row — used for manual / distinct-title
    /// evidence).
    #[allow(clippy::too_many_arguments)]
    pub async fn add_artifact(
        &self,
        pack_id: &str,
        workspace_id: &str,
        kind: ProofArtifactKind,
        title: &str,
        content_ref: Option<&str>,
        status: ProofArtifactStatus,
        metadata: &Value,
        created_by: &str,
    ) -> Result<ProofArtifact> {
        let id = new_id();
        let now = fmt(Utc::now());
        let meta_str = serde_json::to_string(metadata)
            .map_err(|e| Error::Internal(format!("serialize artifact metadata: {e}")))?;
        sqlx::query(
            "INSERT INTO proof_artifacts (id, proof_pack_id, workspace_id, kind, title, \
             content_ref, status, metadata_json, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(pack_id)
        .bind(workspace_id)
        .bind(kind.as_str())
        .bind(title)
        .bind(content_ref)
        .bind(status.as_str())
        .bind(&meta_str)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add proof artifact"))?;
        self.get_artifact(&id).await
    }

    /// Upsert an artifact keyed by `(pack, kind, title)` — auto-assembly uses this
    /// so a re-run REPLACES the prior artifact instead of duplicating it (D8: no
    /// stuck-`failed`, no accumulation).
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_artifact_by_title(
        &self,
        pack_id: &str,
        workspace_id: &str,
        kind: ProofArtifactKind,
        title: &str,
        content_ref: Option<&str>,
        status: ProofArtifactStatus,
        metadata: &Value,
        created_by: &str,
    ) -> Result<ProofArtifact> {
        let now = fmt(Utc::now());
        let meta_str = serde_json::to_string(metadata)
            .map_err(|e| Error::Internal(format!("serialize artifact metadata: {e}")))?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM proof_artifacts WHERE proof_pack_id = ? AND kind = ? AND title = ? LIMIT 1",
        )
        .bind(pack_id)
        .bind(kind.as_str())
        .bind(title)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("lookup proof artifact"))?;
        if let Some(id) = existing {
            sqlx::query(
                "UPDATE proof_artifacts SET content_ref = ?, status = ?, metadata_json = ?, \
                 created_by = ?, updated_at = ? WHERE id = ?",
            )
            .bind(content_ref)
            .bind(status.as_str())
            .bind(&meta_str)
            .bind(created_by)
            .bind(&now)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update proof artifact"))?;
            self.get_artifact(&id).await
        } else {
            self.add_artifact(
                pack_id,
                workspace_id,
                kind,
                title,
                content_ref,
                status,
                metadata,
                created_by,
            )
            .await
        }
    }

    pub async fn delete_artifact(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM proof_artifacts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete proof artifact"))?;
        Ok(())
    }
}

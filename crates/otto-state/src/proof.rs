//! Persistence for **Proof Packs** (migration `0077_proof_packs.sql`).
//!
//! Stores packs and their evidence artifacts. The derived `status`/`risk_score`
//! are written by the server engine (`otto_server::proof`) after each mutation;
//! this repo is pure storage. `ProofArtifact.metadata` is the `metadata_json`
//! TEXT column; timestamps are RFC3339 strings.

use crate::DbPool;
use otto_core::proof::{
    ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack, ProofStatus, WorkItemKind,
};
use otto_core::{new_id, Error, Result};
use serde_json::Value;
use sqlx::Row;

use crate::convert::{dberr, fmt, json};
use chrono::Utc;

#[derive(Clone)]
pub struct ProofRepo {
    pool: DbPool,
    /// Content-addressed media file store (`<dir>/<sha[..2]>/<sha>`). `None`
    /// keeps media inline in `proof_blobs.data` (tests / legacy callers).
    media_dir: Option<std::sync::Arc<std::path::PathBuf>>,
}

/// Columns of `proof_artifacts` with `content_ref` replaced — the shared
/// projection for metadata-only reads (badges, recompute, gates).
const ART_META_COLS: &str = "id, proof_pack_id, workspace_id, kind, title, \
     NULL AS content_ref, status, metadata_json, content_sha256, created_by, created_at, updated_at";

/// Snapshot columns WITHOUT the frozen bundle/report bodies (each can be
/// megabytes); listings only need the meta.
const SNAPSHOT_META_COLS: &str = "id, proof_pack_id, workspace_id, seq, sha256, status, \
     done_score, risk_score, '' AS bundle_json, '' AS report_md, '' AS report_html, note, \
     created_by, created_at";

/// Keyset cursor for [`ProofRepo::list_packs_page`]: the last row's
/// `(updated_at, id)`.
pub type PackCursor = (String, String);

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
        done_score: r.get::<i64, _>("done_score").clamp(0, 100) as u8,
        parent_pack_id: r.get("parent_pack_id"),
        repo_id: r.get("repo_id"),
        pr_number: r.get("pr_number"),
        waived_by: r.get("waived_by"),
        waived_reason: r.get("waived_reason"),
        waived_at: r.get("waived_at"),
        archived_at: r.try_get("archived_at").ok().flatten(),
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
        content_sha256: r.get("content_sha256"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

/// Compute the integrity hash to store for an artifact: the SHA-256 of its
/// inline content. URL / blob / empty refs get `None` (nothing inline to hash).
fn artifact_sha(content_ref: Option<&str>, metadata: &Value) -> Option<String> {
    let ref_kind = metadata
        .get("ref_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("inline");
    match (content_ref, ref_kind) {
        (Some(c), "inline") => Some(otto_core::proof::content_sha256(c)),
        _ => None,
    }
}

/// A persisted immutable snapshot row (the server maps it to the API DTO).
#[derive(Debug, Clone)]
pub struct ProofSnapshotRow {
    pub id: String,
    pub proof_pack_id: String,
    pub workspace_id: String,
    pub seq: i64,
    pub sha256: String,
    pub status: String,
    pub done_score: u8,
    pub risk_score: u8,
    pub bundle_json: String,
    pub report_md: String,
    pub report_html: String,
    pub note: String,
    pub created_by: String,
    pub created_at: String,
}

fn row_to_snapshot(r: &sqlx::sqlite::SqliteRow) -> ProofSnapshotRow {
    ProofSnapshotRow {
        id: r.get("id"),
        proof_pack_id: r.get("proof_pack_id"),
        workspace_id: r.get("workspace_id"),
        seq: r.get("seq"),
        sha256: r.get("sha256"),
        status: r.get("status"),
        done_score: r.get::<i64, _>("done_score").clamp(0, 100) as u8,
        risk_score: r.get::<i64, _>("risk_score").clamp(0, 100) as u8,
        bundle_json: r.get("bundle_json"),
        report_md: r.get("report_md"),
        report_html: r.get("report_html"),
        note: r.get("note"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
    }
}

/// A persisted media blob.
#[derive(Debug, Clone)]
pub struct ProofBlob {
    pub id: String,
    pub artifact_id: String,
    pub workspace_id: String,
    pub sha256: String,
    pub mime: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    pub created_at: String,
}

impl ProofRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self {
            pool,
            media_dir: None,
        }
    }

    /// Store new media as files under `dir` (content-addressed, deduped)
    /// instead of BLOBs in the state DB.
    pub fn with_media_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.media_dir = Some(std::sync::Arc::new(dir.into()));
        self
    }

    /// The configured media file store directory, if any.
    pub fn media_dir(&self) -> Option<&std::path::Path> {
        self.media_dir.as_deref().map(|p| p.as_path())
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
        Ok(self
            .list_packs_page(workspace_id, status, kind, work_item_id, None, None, false)
            .await?
            .0)
    }

    /// One keyset page of [`Self::list_packs`], newest first, walking
    /// `idx_proof_packs_ws_updated`. `limit = None` returns every row. The
    /// returned cursor is `Some` only when more rows may follow.
    #[allow(clippy::too_many_arguments)] // filter knobs, all optional
    pub async fn list_packs_page(
        &self,
        workspace_id: &str,
        status: Option<&str>,
        kind: Option<&str>,
        work_item_id: Option<&str>,
        limit: Option<u32>,
        after: Option<&PackCursor>,
        include_archived: bool,
    ) -> Result<(Vec<ProofPack>, Option<PackCursor>)> {
        let mut sql = String::from("SELECT * FROM proof_packs WHERE workspace_id = ?");
        if !include_archived {
            sql.push_str(" AND archived_at IS NULL");
        }
        if status.is_some() {
            sql.push_str(" AND status = ?");
        }
        if kind.is_some() {
            sql.push_str(" AND work_item_kind = ?");
        }
        if work_item_id.is_some() {
            sql.push_str(" AND work_item_id = ?");
        }
        if after.is_some() {
            sql.push_str(" AND (updated_at < ? OR (updated_at = ? AND id < ?))");
        }
        sql.push_str(" ORDER BY updated_at DESC, id DESC");
        if limit.is_some() {
            sql.push_str(" LIMIT ?");
        }
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(workspace_id);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(k) = kind {
            q = q.bind(k);
        }
        if let Some(w) = work_item_id {
            q = q.bind(w);
        }
        if let Some((u, id)) = after {
            q = q.bind(u).bind(u).bind(id);
        }
        if let Some(l) = limit {
            q = q.bind(l.max(1) as i64);
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list proof packs"))?;
        let packs = rows.iter().map(row_to_pack).collect::<Result<Vec<_>>>()?;
        let next = match limit {
            Some(l) if packs.len() as u32 >= l.max(1) => {
                packs.last().map(|p| (p.updated_at.clone(), p.id.clone()))
            }
            _ => None,
        };
        Ok((packs, next))
    }

    /// The packs of exactly these work items (`(kind, work_item_id)` pairs) in
    /// `workspace_id` — the scoped proof summary (R3). One query per distinct
    /// kind, each driving from `json_each` into the unique
    /// `idx_proof_packs_workitem` index (`CROSS JOIN` pins the loop order and
    /// `+workspace_id` keeps the planner off `idx_proof_packs_ws_updated`, which
    /// it otherwise picks — a scan of the whole workspace), so the rows read
    /// match the filter however many packs the workspace holds. Order:
    /// unspecified.
    pub async fn list_packs_for_work_items(
        &self,
        workspace_id: &str,
        items: &[(String, String)],
    ) -> Result<Vec<ProofPack>> {
        // Deduped per kind: a repeated id never yields a repeated row.
        let mut by_kind: std::collections::BTreeMap<&str, std::collections::BTreeSet<&str>> =
            std::collections::BTreeMap::new();
        for (k, id) in items {
            by_kind.entry(k.as_str()).or_default().insert(id.as_str());
        }
        let mut out = Vec::new();
        for (kind, ids) in by_kind {
            let ids_json = serde_json::to_string(&ids)
                .map_err(|e| Error::Internal(format!("proof work items: {e}")))?;
            let rows = sqlx::query(
                "SELECT p.* FROM json_each(?2) AS j \
                 CROSS JOIN proof_packs p \
                   ON p.work_item_kind = ?1 AND p.work_item_id = j.value \
                 WHERE +p.workspace_id = ?3 AND p.archived_at IS NULL",
            )
            .bind(kind)
            .bind(&ids_json)
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list proof packs for work items"))?;
            for r in &rows {
                out.push(row_to_pack(r)?);
            }
        }
        Ok(out)
    }

    /// Opt-in archive of stale session packs (R3): `session` packs in
    /// `workspace_id` with NO evidence artifacts, not waived, not already
    /// archived, and last updated before `cutoff` (RFC3339). Returns how many
    /// match; with `apply` they are stamped `archived_at` (never deleted —
    /// any later change or new artifact clears the stamp, migration 0191
    /// triggers). `updated_at` is left alone so the list order is unchanged.
    pub async fn archive_stale_session_packs(
        &self,
        workspace_id: &str,
        cutoff: &str,
        apply: bool,
    ) -> Result<u64> {
        const MATCH: &str = "workspace_id = ? AND work_item_kind = 'session' \
             AND archived_at IS NULL AND status != 'waived' AND updated_at < ? \
             AND NOT EXISTS (SELECT 1 FROM proof_artifacts a \
                             WHERE a.proof_pack_id = proof_packs.id)";
        if !apply {
            let sql = format!("SELECT COUNT(*) FROM proof_packs WHERE {MATCH}");
            let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(workspace_id)
                .bind(cutoff)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("count stale session packs"))?;
            return Ok(n.max(0) as u64);
        }
        let sql = format!("UPDATE proof_packs SET archived_at = ? WHERE {MATCH}");
        let r = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(fmt(Utc::now()))
            .bind(workspace_id)
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(dberr("archive stale session packs"))?;
        Ok(r.rows_affected())
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

    /// Persist derived status + risk + done-contract score (the engine computes
    /// all three on recompute).
    pub async fn set_status_risk_done(
        &self,
        id: &str,
        status: ProofStatus,
        risk: u8,
        done: u8,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET status = ?, risk_score = ?, done_score = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(risk as i64)
        .bind(done as i64)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set proof pack status/risk/done"))?;
        Ok(())
    }

    /// Link a pack to a registered repo and (optionally) a PR number. Only writes
    /// the columns provided so re-linking the same repo while learning the PR
    /// number later is idempotent.
    pub async fn set_repo_link(
        &self,
        id: &str,
        repo_id: Option<&str>,
        pr_number: Option<i64>,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET \
               repo_id   = COALESCE(?, repo_id), \
               pr_number = COALESCE(?, pr_number), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(repo_id)
        .bind(pr_number)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set proof pack repo link"))?;
        Ok(())
    }

    /// Waive a pack (human override). Sets status=waived + records who/why/when.
    pub async fn waive(&self, id: &str, by: &str, reason: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET status = 'waived', waived_by = ?, waived_reason = ?, \
             waived_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(by)
        .bind(reason)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("waive proof pack"))?;
        Ok(())
    }

    /// Link a pack to a parent (rollup). No-op if already set to the same parent.
    pub async fn set_parent(&self, id: &str, parent_id: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE proof_packs SET parent_pack_id = ?, updated_at = ? \
             WHERE id = ? AND (parent_pack_id IS NULL OR parent_pack_id <> ?)",
        )
        .bind(parent_id)
        .bind(&now)
        .bind(id)
        .bind(parent_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set proof pack parent"))?;
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

    /// A pack's artifacts WITHOUT `content_ref` (it comes back `None`) — what
    /// recompute, badges and the gates read. Status/risk/done/badges derive
    /// from kind, status, title and metadata only, so loading every artifact's
    /// inline content (up to 2 MiB each) per recompute was pure waste.
    pub async fn list_artifacts_meta(&self, pack_id: &str) -> Result<Vec<ProofArtifact>> {
        let sql = format!(
            "SELECT {ART_META_COLS} FROM proof_artifacts WHERE proof_pack_id = ? \
             ORDER BY created_at ASC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(pack_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list proof artifact meta"))?;
        rows.iter().map(row_to_artifact).collect()
    }

    /// A pack's artifacts with `content_ref` cut to its first `cap_chars`
    /// characters IN SQL, plus each artifact's full content length in bytes.
    /// The detail view only shows a capped preview; the full body is fetched
    /// on demand from `/proof-artifacts/{id}/content`.
    pub async fn list_artifacts_preview(
        &self,
        pack_id: &str,
        cap_chars: usize,
    ) -> Result<Vec<(ProofArtifact, i64)>> {
        let rows = sqlx::query(
            "SELECT id, proof_pack_id, workspace_id, kind, title, \
                    substr(content_ref, 1, ?) AS content_ref, \
                    COALESCE(length(CAST(content_ref AS BLOB)), 0) AS content_len, \
                    status, metadata_json, content_sha256, created_by, created_at, updated_at \
             FROM proof_artifacts WHERE proof_pack_id = ? ORDER BY created_at ASC",
        )
        .bind(cap_chars as i64)
        .bind(pack_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list proof artifact previews"))?;
        rows.iter()
            .map(|r| Ok((row_to_artifact(r)?, r.get::<i64, _>("content_len"))))
            .collect()
    }

    /// Metadata-only artifacts for many packs at once (child rollups), grouped
    /// by pack — one `IN (…)` query instead of one full read per child.
    pub async fn artifacts_meta_for_packs(
        &self,
        pack_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ProofArtifact>>> {
        let mut out: std::collections::HashMap<String, Vec<ProofArtifact>> =
            std::collections::HashMap::new();
        for chunk in pack_ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT {ART_META_COLS} FROM proof_artifacts WHERE proof_pack_id IN ({marks}) \
                 ORDER BY proof_pack_id, created_at ASC"
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));
            for id in chunk {
                q = q.bind(id);
            }
            let rows = q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("proof artifact meta for packs"))?;
            for r in &rows {
                let a = row_to_artifact(r)?;
                out.entry(a.proof_pack_id.clone()).or_default().push(a);
            }
        }
        Ok(out)
    }

    /// Every artifact of every pack in `workspace_id`, grouped by pack, for
    /// BADGES and counts only: `content_ref` is not read (it comes back
    /// `None`). One query instead of one per pack — the proof summary/list
    /// used to issue N queries and read each artifact's inline content (up to
    /// 2 MiB, typically a whole diff) just to compute badges from kind,
    /// status, title and metadata (r3-07-01). Walks `idx_proof_packs_ws` then
    /// `idx_proof_artifacts_pack`; order within a pack matches
    /// [`Self::list_artifacts`].
    pub async fn badge_artifacts(
        &self,
        workspace_id: &str,
    ) -> Result<std::collections::HashMap<String, Vec<ProofArtifact>>> {
        let rows = sqlx::query(
            "SELECT a.id, a.proof_pack_id, a.workspace_id, a.kind, a.title, \
                    NULL AS content_ref, a.status, a.metadata_json, a.content_sha256, \
                    a.created_by, a.created_at, a.updated_at \
             FROM proof_packs p JOIN proof_artifacts a ON a.proof_pack_id = p.id \
             WHERE p.workspace_id = ? \
             ORDER BY a.proof_pack_id, a.created_at ASC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("proof badge artifacts"))?;
        let mut out: std::collections::HashMap<String, Vec<ProofArtifact>> =
            std::collections::HashMap::new();
        for r in &rows {
            let a = row_to_artifact(r)?;
            out.entry(a.proof_pack_id.clone()).or_default().push(a);
        }
        Ok(out)
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
        let sha = artifact_sha(content_ref, metadata);
        sqlx::query(
            "INSERT INTO proof_artifacts (id, proof_pack_id, workspace_id, kind, title, \
             content_ref, status, metadata_json, content_sha256, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(pack_id)
        .bind(workspace_id)
        .bind(kind.as_str())
        .bind(title)
        .bind(content_ref)
        .bind(status.as_str())
        .bind(&meta_str)
        .bind(&sha)
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
            let sha = artifact_sha(content_ref, metadata);
            sqlx::query(
                "UPDATE proof_artifacts SET content_ref = ?, status = ?, metadata_json = ?, \
                 content_sha256 = ?, created_by = ?, updated_at = ? WHERE id = ?",
            )
            .bind(content_ref)
            .bind(status.as_str())
            .bind(&meta_str)
            .bind(&sha)
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

    /// Point an artifact's `content_ref` at a stored blob (used after creating a
    /// media artifact + its blob). Does not touch `content_sha256` (the blob's
    /// own sha lives in `proof_blobs`).
    pub async fn set_artifact_ref(&self, id: &str, content_ref: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE proof_artifacts SET content_ref = ?, updated_at = ? WHERE id = ?")
            .bind(content_ref)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set proof artifact ref"))?;
        Ok(())
    }

    // -- Snapshots (immutable; append-only) ----------------------------------

    /// The next `seq` for a pack's snapshots (1-based, monotonic).
    pub async fn next_snapshot_seq(&self, pack_id: &str) -> Result<i64> {
        let max: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM proof_snapshots WHERE proof_pack_id = ?")
                .bind(pack_id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("next snapshot seq"))?;
        Ok(max.unwrap_or(0) + 1)
    }

    /// Persist an immutable snapshot. Never updated or deleted afterward.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_snapshot(
        &self,
        pack_id: &str,
        workspace_id: &str,
        sha256: &str,
        status: &str,
        done_score: u8,
        risk_score: u8,
        bundle_json: &str,
        report_md: &str,
        report_html: &str,
        note: &str,
        created_by: &str,
    ) -> Result<ProofSnapshotRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        let seq = self.next_snapshot_seq(pack_id).await?;
        sqlx::query(
            "INSERT INTO proof_snapshots (id, proof_pack_id, workspace_id, seq, sha256, status, \
             done_score, risk_score, bundle_json, report_md, report_html, note, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(pack_id)
        .bind(workspace_id)
        .bind(seq)
        .bind(sha256)
        .bind(status)
        .bind(done_score as i64)
        .bind(risk_score as i64)
        .bind(bundle_json)
        .bind(report_md)
        .bind(report_html)
        .bind(note)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create proof snapshot"))?;
        self.get_snapshot(&id).await
    }

    pub async fn get_snapshot(&self, id: &str) -> Result<ProofSnapshotRow> {
        let row = sqlx::query("SELECT * FROM proof_snapshots WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get proof snapshot"))?;
        Ok(row_to_snapshot(&row))
    }

    /// Snapshots for a pack, newest first — META ONLY: `bundle_json`,
    /// `report_md` and `report_html` come back empty (fetch one snapshot with
    /// [`Self::get_snapshot`] for the bodies).
    pub async fn list_snapshots(&self, pack_id: &str) -> Result<Vec<ProofSnapshotRow>> {
        let sql = format!(
            "SELECT {SNAPSHOT_META_COLS} FROM proof_snapshots WHERE proof_pack_id = ? \
             ORDER BY seq DESC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(pack_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list proof snapshots"))?;
        Ok(rows.iter().map(row_to_snapshot).collect())
    }

    // -- Media blobs ---------------------------------------------------------

    /// Persist a media blob. With a media dir the bytes go to the
    /// content-addressed file store (deduped by sha — an identical screenshot
    /// is written once) and the row keeps an empty placeholder; otherwise they
    /// stay inline. `sha256` must be the hex SHA-256 of `data`.
    pub async fn add_blob(
        &self,
        artifact_id: &str,
        workspace_id: &str,
        sha256: &str,
        mime: &str,
        data: &[u8],
    ) -> Result<String> {
        let id = new_id();
        let now = fmt(Utc::now());
        let stored = match self.media_dir.clone() {
            Some(dir) => {
                let (sha, bytes) = (sha256.to_string(), data.to_vec());
                tokio::task::spawn_blocking(move || media_put(&dir, &sha, &bytes))
                    .await
                    .map_err(|e| Error::Internal(format!("media store join: {e}")))??;
                true
            }
            None => false,
        };
        let inline: &[u8] = if stored { &[] } else { data };
        sqlx::query(
            "INSERT INTO proof_blobs (id, artifact_id, workspace_id, sha256, mime, size_bytes, \
             data, created_at, stored) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(artifact_id)
        .bind(workspace_id)
        .bind(sha256)
        .bind(mime)
        .bind(data.len() as i64)
        .bind(inline)
        .bind(&now)
        .bind(stored as i64)
        .execute(&self.pool)
        .await
        .map_err(dberr("add proof blob"))?;
        Ok(id)
    }

    /// Fetch the blob for an artifact (the most recent if more than one).
    /// File-stored bytes are read off the runtime; a row whose bytes were
    /// never moved falls back to the inline BLOB.
    pub async fn blob_for_artifact(&self, artifact_id: &str) -> Result<Option<ProofBlob>> {
        let row = sqlx::query(
            "SELECT * FROM proof_blobs WHERE artifact_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(artifact_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get proof blob"))?;
        let Some(r) = row else { return Ok(None) };
        let mut blob = ProofBlob {
            id: r.get("id"),
            artifact_id: r.get("artifact_id"),
            workspace_id: r.get("workspace_id"),
            sha256: r.get("sha256"),
            mime: r.get("mime"),
            size_bytes: r.get("size_bytes"),
            data: r.get("data"),
            created_at: r.get("created_at"),
        };
        if r.get::<i64, _>("stored") != 0 {
            let dir = self.media_dir.clone().ok_or_else(|| {
                Error::Internal("proof media is file-stored but no media dir is set".into())
            })?;
            let sha = blob.sha256.clone();
            blob.data = tokio::task::spawn_blocking(move || std::fs::read(media_path(&dir, &sha)))
                .await
                .map_err(|e| Error::Internal(format!("media read join: {e}")))?
                .map_err(|e| Error::NotFound(format!("proof media file: {e}")))?;
        }
        Ok(Some(blob))
    }

    /// Move up to `limit` legacy inline BLOBs into the file store. Each row's
    /// bytes are re-hashed, written + fsynced, READ BACK and verified before
    /// the row flips to `stored = 1` and drops its inline copy — a crash at
    /// any point leaves the row readable (worst case an orphan file the GC
    /// removes). A row whose bytes don't match its recorded sha is left
    /// inline untouched. Walks rows in id order after `after`; returns how
    /// many rows moved and the cursor for the next batch (`None` = done).
    /// No-op without a media dir.
    pub async fn migrate_media_batch(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<(usize, Option<String>)> {
        let Some(dir) = self.media_dir.clone() else {
            return Ok((0, None));
        };
        let rows = sqlx::query(
            "SELECT id, sha256, data FROM proof_blobs \
             WHERE stored = 0 AND length(data) > 0 AND id > ? ORDER BY id LIMIT ?",
        )
        .bind(after.unwrap_or(""))
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list inline proof media"))?;
        let mut moved = 0;
        let full = rows.len() as u32 >= limit.max(1);
        let mut last = None;
        for r in rows {
            let id: String = r.get("id");
            last = Some(id.clone());
            let sha: String = r.get("sha256");
            let data: Vec<u8> = r.get("data");
            let d = dir.clone();
            let s = sha.clone();
            let ok = tokio::task::spawn_blocking(move || -> Result<bool> {
                if otto_core::proof::bytes_sha256(&data) != s {
                    return Ok(false);
                }
                media_put(&d, &s, &data)?;
                let back = std::fs::read(media_path(&d, &s))
                    .map_err(|e| Error::Internal(format!("media read-back: {e}")))?;
                Ok(otto_core::proof::bytes_sha256(&back) == s)
            })
            .await
            .map_err(|e| Error::Internal(format!("media migrate join: {e}")))??;
            if !ok {
                tracing::warn!(blob = %id, "proof media sha mismatch; left inline");
                continue;
            }
            sqlx::query(
                "UPDATE proof_blobs SET stored = 1, data = X'' WHERE id = ? AND stored = 0",
            )
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(dberr("mark proof media stored"))?;
            moved += 1;
        }
        Ok((moved, if full { last } else { None }))
    }

    /// Delete media files no `stored = 1` row references any more (their
    /// artifacts were deleted — rows cascade, files don't). Files younger
    /// than `min_age` are kept so an upload whose file is written but whose
    /// row is not yet inserted is never collected. Returns files removed.
    pub async fn gc_media_files(&self, min_age: std::time::Duration) -> Result<usize> {
        let Some(dir) = self.media_dir.clone() else {
            return Ok(0);
        };
        let d = dir.clone();
        let candidates = tokio::task::spawn_blocking(move || media_list(&d, min_age))
            .await
            .map_err(|e| Error::Internal(format!("media list join: {e}")))?;
        let mut removed = 0;
        for sha in candidates {
            let used: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM proof_blobs WHERE sha256 = ? AND stored = 1 LIMIT 1",
            )
            .bind(&sha)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("proof media ref check"))?;
            if used.is_none() && std::fs::remove_file(media_path(&dir, &sha)).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// `(files, bytes)` in the media store — the storage-size gauge.
    pub async fn media_store_size(&self) -> (u64, u64) {
        let Some(dir) = self.media_dir.clone() else {
            return (0, 0);
        };
        tokio::task::spawn_blocking(move || {
            let mut n = 0u64;
            let mut bytes = 0u64;
            for sub in std::fs::read_dir(&*dir).into_iter().flatten().flatten() {
                for f in std::fs::read_dir(sub.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if let Ok(m) = f.metadata() {
                        if m.is_file() {
                            n += 1;
                            bytes += m.len();
                        }
                    }
                }
            }
            (n, bytes)
        })
        .await
        .unwrap_or((0, 0))
    }
}

// --- Content-addressed media file store -------------------------------------

fn valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn media_path(dir: &std::path::Path, sha: &str) -> std::path::PathBuf {
    dir.join(&sha[..2.min(sha.len())]).join(sha)
}

/// Write `data` at its sha path atomically (tmp + fsync + rename + dir
/// fsync). An existing file of the right size is reused (dedupe).
fn media_put(dir: &std::path::Path, sha: &str, data: &[u8]) -> Result<()> {
    use std::io::Write;
    if !valid_sha(sha) {
        return Err(Error::Invalid(format!("bad media sha '{sha}'")));
    }
    let path = media_path(dir, sha);
    if let Ok(m) = std::fs::metadata(&path) {
        if m.len() == data.len() as u64 {
            return Ok(());
        }
    }
    let parent = path.parent().expect("sha path has a parent");
    let io = |e: std::io::Error| Error::Internal(format!("proof media write: {e}"));
    std::fs::create_dir_all(parent).map_err(io)?;
    let tmp = parent.join(format!(".{sha}.{}.tmp", new_id()));
    let res = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(io)
}

/// Every sha-named file older than `min_age` (stale tmp files included under
/// their own names are skipped — they never match a 64-hex name, and are
/// removed here directly once old).
fn media_list(dir: &std::path::Path, min_age: std::time::Duration) -> Vec<String> {
    let now = std::time::SystemTime::now();
    let mut out = Vec::new();
    for sub in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(sub.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let old = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= min_age);
            if !old {
                continue;
            }
            let name = f.file_name().to_string_lossy().into_owned();
            if valid_sha(&name) {
                out.push(name);
            } else if name.ends_with(".tmp") {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
    out
}

#[cfg(test)]
mod proof_perf_tests {
    use super::*;
    use otto_core::proof::bytes_sha256;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    async fn pack_with_artifact(repo: &ProofRepo, wid: &str, content: &str) -> (ProofPack, String) {
        let p = repo
            .create_pack("ws", WorkItemKind::Manual, wid, "t", "u", None)
            .await
            .unwrap();
        let a = repo
            .add_artifact(
                &p.id,
                "ws",
                ProofArtifactKind::Diff,
                "diff",
                Some(content),
                ProofArtifactStatus::Info,
                &serde_json::json!({}),
                "u",
            )
            .await
            .unwrap();
        (p, a.id)
    }

    #[tokio::test]
    async fn proof_list_packs_pages_by_keyset() {
        let repo = ProofRepo::new(mem_pool().await);
        for i in 0..7 {
            repo.create_pack("ws", WorkItemKind::Manual, &format!("w{i}"), "t", "u", None)
                .await
                .unwrap();
        }
        let mut seen = Vec::new();
        let mut cur: Option<PackCursor> = None;
        loop {
            let (page, next) = repo
                .list_packs_page("ws", None, None, None, Some(3), cur.as_ref(), false)
                .await
                .unwrap();
            seen.extend(page.into_iter().map(|p| p.id));
            match next {
                Some(c) => cur = Some(c),
                None => break,
            }
        }
        let all = repo.list_packs("ws", None, None, None).await.unwrap();
        assert_eq!(seen.len(), 7);
        assert_eq!(seen, all.into_iter().map(|p| p.id).collect::<Vec<_>>());
    }

    /// R4 budget: at 5 000 packs (with an artifact each) the first keyset
    /// page and the work-item-scoped summary read stay well under 50 ms and
    /// read exactly the rows asked for — not the whole workspace.
    #[tokio::test]
    async fn proof_5k_packs_page_and_scoped_summary_within_budget() {
        let pool = mem_pool().await;
        sqlx::query(
            "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 4999)
             INSERT INTO proof_packs (id, workspace_id, work_item_kind, work_item_id, title,
                                      created_by, created_at, updated_at)
             SELECT printf('p%05d', i), 'ws', 'session', printf('s%05d', i), 't', 'u',
                    strftime('%Y-%m-%dT%H:%M:%SZ', 1767225600 + i, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%SZ', 1767225600 + i, 'unixepoch')
             FROM n",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO proof_artifacts (id, proof_pack_id, workspace_id, kind, title,
                                          content_ref, status, created_by, created_at, updated_at)
             SELECT 'a' || id, id, 'ws', 'diff', 'diff', 'x', 'info', 'u', created_at, created_at
             FROM proof_packs",
        )
        .execute(&pool)
        .await
        .unwrap();
        // The scoped read is an index probe on the unique work-item index,
        // never a scan of the workspace's packs.
        let detail: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT p.* FROM json_each(?2) AS j \
             CROSS JOIN proof_packs p ON p.work_item_kind = ?1 AND p.work_item_id = j.value \
             WHERE +p.workspace_id = ?3",
        )
        .bind("session")
        .bind("[\"s00001\"]")
        .bind("ws")
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(
            detail
                .iter()
                .any(|r| r.3.contains("idx_proof_packs_workitem")),
            "{detail:?}"
        );
        let repo = ProofRepo::new(pool);
        // Warm the statement cache / page cache once, then measure.
        repo.list_packs_page("ws", None, None, None, Some(100), None, false)
            .await
            .unwrap();
        let t = std::time::Instant::now();
        let (page, next) = repo
            .list_packs_page("ws", None, None, None, Some(100), None, false)
            .await
            .unwrap();
        let page_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(page.len(), 100);
        assert!(next.is_some());

        let items: Vec<(String, String)> = (0..200)
            .map(|i| ("session".to_string(), format!("s{:05}", i * 25)))
            .chain(std::iter::once(("session".into(), "not-a-session".into())))
            .collect();
        let t = std::time::Instant::now();
        let packs = repo.list_packs_for_work_items("ws", &items).await.unwrap();
        let ids: Vec<String> = packs.iter().map(|p| p.id.clone()).collect();
        let arts = repo.artifacts_meta_for_packs(&ids).await.unwrap();
        let summary_ms = t.elapsed().as_secs_f64() * 1e3;
        // Rows read match the filter: 200 known work items, the unknown one
        // matches nothing, and another workspace's pack never leaks in.
        assert_eq!(packs.len(), 200);
        assert_eq!(arts.values().map(Vec::len).sum::<usize>(), 200);
        assert!(repo
            .list_packs_for_work_items("other-ws", &items)
            .await
            .unwrap()
            .is_empty());
        eprintln!("proof 5k: first page {page_ms:.2} ms, scoped summary (200) {summary_ms:.2} ms");
        assert!(page_ms < 50.0, "first page took {page_ms:.2} ms");
        assert!(summary_ms < 50.0, "scoped summary took {summary_ms:.2} ms");
    }

    /// R3 opt-in archive: a dry run changes nothing; apply hides only stale,
    /// evidence-less, un-waived session packs from the list + summary (never
    /// deletes); any later update or new artifact un-archives the pack.
    #[tokio::test]
    async fn proof_archive_stale_session_packs_is_opt_in_and_self_heals() {
        let pool = mem_pool().await;
        let repo = ProofRepo::new(pool.clone());
        let mk = |wid: &'static str, kind: WorkItemKind| {
            let repo = repo.clone();
            async move {
                repo.create_pack("ws", kind, wid, "t", "u", None)
                    .await
                    .unwrap()
            }
        };
        let stale = mk("s-stale", WorkItemKind::Session).await;
        let with_ev = mk("s-evidence", WorkItemKind::Session).await;
        let manual = mk("m-stale", WorkItemKind::Manual).await;
        let fresh = mk("s-fresh", WorkItemKind::Session).await;
        repo.add_artifact(
            &with_ev.id,
            "ws",
            ProofArtifactKind::Diff,
            "diff",
            Some("x"),
            ProofArtifactStatus::Info,
            &serde_json::json!({}),
            "u",
        )
        .await
        .unwrap();
        // Age everything but `fresh` past the cutoff.
        sqlx::query("UPDATE proof_packs SET updated_at = '2020-01-01T00:00:00Z' WHERE id != ?")
            .bind(&fresh.id)
            .execute(&pool)
            .await
            .unwrap();
        let cutoff = "2025-01-01T00:00:00Z";

        // Dry run: counts only the stale evidence-less session pack, changes nothing.
        assert_eq!(
            repo.archive_stale_session_packs("ws", cutoff, false)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            repo.list_packs("ws", None, None, None).await.unwrap().len(),
            4
        );

        // Apply: hidden from the default list + the scoped summary, still stored.
        assert_eq!(
            repo.archive_stale_session_packs("ws", cutoff, true)
                .await
                .unwrap(),
            1
        );
        let live = repo.list_packs("ws", None, None, None).await.unwrap();
        assert_eq!(live.len(), 3);
        assert!(live.iter().all(|p| p.id != stale.id));
        let (all, _) = repo
            .list_packs_page("ws", None, None, None, None, None, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 4);
        let items = vec![
            ("session".to_string(), "s-stale".to_string()),
            ("session".to_string(), "s-fresh".to_string()),
        ];
        let scoped = repo.list_packs_for_work_items("ws", &items).await.unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, fresh.id);
        assert!(repo
            .get_pack(&stale.id)
            .await
            .unwrap()
            .archived_at
            .is_some());
        assert!(repo
            .get_pack(&manual.id)
            .await
            .unwrap()
            .archived_at
            .is_none());
        // Idempotent: nothing left to archive.
        assert_eq!(
            repo.archive_stale_session_packs("ws", cutoff, true)
                .await
                .unwrap(),
            0
        );

        // Any later update un-archives it (recompute path) …
        repo.set_status_risk(&stale.id, ProofStatus::Missing, 0)
            .await
            .unwrap();
        assert!(repo
            .get_pack(&stale.id)
            .await
            .unwrap()
            .archived_at
            .is_none());
        assert_eq!(
            repo.list_packs("ws", None, None, None).await.unwrap().len(),
            4
        );

        // … and so does new evidence.
        sqlx::query("UPDATE proof_packs SET updated_at = '2020-01-01T00:00:00Z' WHERE id = ?")
            .bind(&stale.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            repo.archive_stale_session_packs("ws", cutoff, true)
                .await
                .unwrap(),
            1
        );
        repo.add_artifact(
            &stale.id,
            "ws",
            ProofArtifactKind::Log,
            "log",
            Some("y"),
            ProofArtifactStatus::Info,
            &serde_json::json!({}),
            "u",
        )
        .await
        .unwrap();
        assert!(repo
            .get_pack(&stale.id)
            .await
            .unwrap()
            .archived_at
            .is_none());
    }

    #[tokio::test]
    async fn proof_meta_and_preview_projections_trim_content() {
        let repo = ProofRepo::new(mem_pool().await);
        let big = "é".repeat(10_000); // 20 000 bytes
        let (p, _) = pack_with_artifact(&repo, "w", &big).await;
        let meta = repo.list_artifacts_meta(&p.id).await.unwrap();
        assert_eq!(meta.len(), 1);
        assert!(meta[0].content_ref.is_none());
        let prev = repo.list_artifacts_preview(&p.id, 100).await.unwrap();
        assert_eq!(
            prev[0].0.content_ref.as_deref().unwrap().chars().count(),
            100
        );
        assert_eq!(prev[0].1, 20_000);
        let grouped = repo
            .artifacts_meta_for_packs(std::slice::from_ref(&p.id))
            .await
            .unwrap();
        assert_eq!(grouped[&p.id].len(), 1);
    }

    #[tokio::test]
    async fn proof_snapshot_listing_is_meta_only() {
        let repo = ProofRepo::new(mem_pool().await);
        let (p, _) = pack_with_artifact(&repo, "w", "x").await;
        let s = repo
            .create_snapshot(
                &p.id,
                "ws",
                "sha",
                "passed",
                1,
                2,
                "{\"big\":1}",
                "# md",
                "<p>",
                "n",
                "u",
            )
            .await
            .unwrap();
        assert_eq!(s.bundle_json, "{\"big\":1}");
        let list = repo.list_snapshots(&p.id).await.unwrap();
        assert_eq!(list[0].id, s.id);
        assert!(list[0].bundle_json.is_empty() && list[0].report_html.is_empty());
        assert_eq!(list[0].note, "n");
    }

    #[tokio::test]
    async fn proof_media_file_store_dedupes_and_gcs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ProofRepo::new(mem_pool().await).with_media_dir(dir.path().join("media"));
        let (p, a1) = pack_with_artifact(&repo, "w", "x").await;
        let a2 = repo
            .add_artifact(
                &p.id,
                "ws",
                ProofArtifactKind::Screenshot,
                "s2",
                None,
                ProofArtifactStatus::Info,
                &serde_json::json!({}),
                "u",
            )
            .await
            .unwrap()
            .id;
        let bytes = b"\x89PNG fake image".to_vec();
        let sha = bytes_sha256(&bytes);
        repo.add_blob(&a1, "ws", &sha, "image/png", &bytes)
            .await
            .unwrap();
        repo.add_blob(&a2, "ws", &sha, "image/png", &bytes)
            .await
            .unwrap();
        // One file for two rows; nothing inline in SQLite.
        assert_eq!(repo.media_store_size().await, (1, bytes.len() as u64));
        let inline: i64 = sqlx::query_scalar("SELECT SUM(length(data)) FROM proof_blobs")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(inline, 0);
        assert_eq!(
            repo.blob_for_artifact(&a2).await.unwrap().unwrap().data,
            bytes
        );
        // Still referenced by a2 → kept.
        repo.delete_artifact(&a1).await.unwrap();
        assert_eq!(
            repo.gc_media_files(std::time::Duration::ZERO)
                .await
                .unwrap(),
            0
        );
        // Unreferenced → collected (but not while younger than min_age).
        repo.delete_artifact(&a2).await.unwrap();
        assert_eq!(
            repo.gc_media_files(std::time::Duration::from_secs(3600))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            repo.gc_media_files(std::time::Duration::ZERO)
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.media_store_size().await, (0, 0));
    }

    #[tokio::test]
    async fn proof_legacy_inline_media_migrates_safely() {
        let dir = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        // Legacy writer: no media dir → bytes inline.
        let legacy = ProofRepo::new(pool.clone());
        let (_, a1) = pack_with_artifact(&legacy, "w", "x").await;
        let (_, a2) = pack_with_artifact(&legacy, "w2", "x").await;
        let good = b"good bytes".to_vec();
        legacy
            .add_blob(&a1, "ws", &bytes_sha256(&good), "image/png", &good)
            .await
            .unwrap();
        // A corrupt row (sha doesn't match its bytes) must stay inline.
        legacy
            .add_blob(&a2, "ws", &"0".repeat(64), "image/png", b"other")
            .await
            .unwrap();
        let repo = ProofRepo::new(pool).with_media_dir(dir.path());
        // Reads fall back to the BLOB before the move.
        assert_eq!(
            repo.blob_for_artifact(&a1).await.unwrap().unwrap().data,
            good
        );
        // Batches of one walk past the corrupt row instead of stalling on it.
        let (m1, c1) = repo.migrate_media_batch(None, 1).await.unwrap();
        let (m2, c2) = repo.migrate_media_batch(c1.as_deref(), 1).await.unwrap();
        let (m3, _) = repo.migrate_media_batch(c2.as_deref(), 1).await.unwrap();
        assert_eq!(m1 + m2 + m3, 1);
        assert_eq!(repo.migrate_media_batch(None, 10).await.unwrap(), (0, None));
        assert_eq!(
            repo.blob_for_artifact(&a1).await.unwrap().unwrap().data,
            good
        );
        assert_eq!(
            repo.blob_for_artifact(&a2).await.unwrap().unwrap().data,
            b"other"
        );
        let stored: i64 = sqlx::query_scalar("SELECT SUM(stored) FROM proof_blobs")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(stored, 1);
    }
}

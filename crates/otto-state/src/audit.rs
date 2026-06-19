//! Audit-log repository: an append-only ledger of security-relevant actions.
//!
//! Rows mirror [`otto_core::domain::AuditEntry`]. There is intentionally NO
//! update or delete path — the only mutations are [`AuditRepo::insert`] (append)
//! and [`AuditRepo::list`] / [`AuditRepo::count`] (read). Callers write
//! best-effort: an insert failure must never fail the request being audited (see
//! `ServerCtx::audit`), so errors here are logged and swallowed upstream.

use chrono::Utc;
use otto_core::api::AuditLogQuery;
use otto_core::domain::AuditEntry;
use otto_core::{new_id, Error, Id, Result};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::convert::{dberr, fmt, ts};

/// Input for [`AuditRepo::insert`]. `id`/`ts` are owned by the repository.
pub struct NewAuditEntry {
    /// Acting user; `None` for an unauthenticated actor or daemon-internal call.
    pub user_id: Option<Id>,
    /// Stable snake_case verb, e.g. `"login.success"`.
    pub action: String,
    /// Optional subject of the action.
    pub target: Option<String>,
    /// Optional action-specific context (stored as JSON TEXT).
    pub detail: Option<serde_json::Value>,
    /// Optional client IP (the real socket peer).
    pub ip: Option<String>,
}

#[derive(Clone)]
pub struct AuditRepo {
    pool: SqlitePool,
}

fn row_to_entry(r: &sqlx::sqlite::SqliteRow) -> Result<AuditEntry> {
    let detail = match r.get::<Option<String>, _>("detail") {
        Some(s) => Some(
            serde_json::from_str::<serde_json::Value>(&s)
                .map_err(|e| Error::Internal(format!("bad audit detail: {e}")))?,
        ),
        None => None,
    };
    Ok(AuditEntry {
        id: r.get("id"),
        ts: ts(&r.get::<String, _>("ts"))?,
        user_id: r.get("user_id"),
        action: r.get("action"),
        target: r.get("target"),
        detail,
        ip: r.get("ip"),
    })
}

impl AuditRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append one audit entry. Returns the persisted row. Callers treat this as
    /// best-effort (see the module docs): a failure must not abort the audited
    /// request.
    pub async fn insert(&self, e: NewAuditEntry) -> Result<AuditEntry> {
        let id = new_id();
        let now = fmt(Utc::now());
        let detail_json = match &e.detail {
            Some(v) => Some(
                serde_json::to_string(v)
                    .map_err(|err| Error::Internal(format!("encode audit detail: {err}")))?,
            ),
            None => None,
        };
        sqlx::query(
            "INSERT INTO audit_log (id, ts, user_id, action, target, detail, ip)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&now)
        .bind(&e.user_id)
        .bind(&e.action)
        .bind(&e.target)
        .bind(&detail_json)
        .bind(&e.ip)
        .execute(&self.pool)
        .await
        .map_err(dberr("insert audit entry"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<AuditEntry> {
        let r = sqlx::query("SELECT * FROM audit_log WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("audit entry"))?;
        row_to_entry(&r)
    }

    /// Append the shared `WHERE` predicates of [`AuditLogQuery`] to `qb`. Shared
    /// by [`Self::list`] and [`Self::count`] so they always filter identically.
    fn push_filters<'a>(qb: &mut QueryBuilder<'a, Sqlite>, q: &'a AuditLogQuery, from: &'a str, to: &'a str) {
        qb.push(" WHERE 1=1");
        if q.from.is_some() {
            qb.push(" AND ts >= ").push_bind(from);
        }
        if q.to.is_some() {
            qb.push(" AND ts <= ").push_bind(to);
        }
        if let Some(action) = &q.action {
            qb.push(" AND action = ").push_bind(action);
        }
        if let Some(uid) = &q.user_id {
            qb.push(" AND user_id = ").push_bind(uid);
        }
    }

    /// A page of entries matching the filters, newest first. `limit` is clamped
    /// to `[1, 500]`; `offset` defaults to 0.
    pub async fn list(&self, q: &AuditLogQuery) -> Result<Vec<AuditEntry>> {
        let limit = q.limit.unwrap_or(100).clamp(1, 500);
        let offset = q.offset.unwrap_or(0).max(0);
        // RFC3339 strings sort lexicographically in the same order as the
        // timestamps they encode (fixed offset), so bound `ts` as TEXT.
        let from = q.from.map(fmt).unwrap_or_default();
        let to = q.to.map(fmt).unwrap_or_default();

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM audit_log");
        Self::push_filters(&mut qb, q, &from, &to);
        qb.push(" ORDER BY ts DESC, id DESC LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list audit log"))?;
        rows.iter().map(row_to_entry).collect()
    }

    /// Total rows matching the filters (ignores `limit`/`offset`), for paging.
    pub async fn count(&self, q: &AuditLogQuery) -> Result<i64> {
        let from = q.from.map(fmt).unwrap_or_default();
        let to = q.to.map(fmt).unwrap_or_default();

        let mut qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS n FROM audit_log");
        Self::push_filters(&mut qb, q, &from, &to);

        let r = qb
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("count audit log"))?;
        Ok(r.get::<i64, _>("n"))
    }
}

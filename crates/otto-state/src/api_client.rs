//! API client ("Postman" section) repository.
//!
//! Workspace-scoped persistence for the API client: collections, saved
//! requests, environments and execution history. Mirrors the
//! `otto_core::domain` `Api*` structs. Free-form JSON fields (headers / query /
//! auth / variables / request / response) are stored as TEXT in `*_json`
//! columns. Timestamps are RFC3339 TEXT. At most one environment per workspace
//! is active; [`ApiClientRepo::set_active`] enforces this.

use chrono::Utc;
use otto_core::domain::{
    ApiAutomation, ApiCollection, ApiEnvironment, ApiHistoryEntry, ApiHistorySourceSummary,
    ApiHistorySummary, ApiRequest,
};
use otto_core::{new_id, Id, Result};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct ApiClientRepo {
    pool: SqlitePool,
}

// --- inputs -----------------------------------------------------------------

/// Input for [`ApiClientRepo::create_collection`].
pub struct NewApiCollection {
    pub workspace_id: Id,
    pub name: String,
    pub parent_id: Option<Id>,
    pub position: i64,
}

/// Input for [`ApiClientRepo::create_request`] / `update_request`.
pub struct NewApiRequest {
    /// Pre-generated id for create (None = repo generates one). The route
    /// layer pre-generates ids so Keychain secret refs (`otto.api.request.<id>`)
    /// can be written BEFORE the row exists — a plaintext secret never
    /// transits SQLite, not even transiently. Ignored by `update_request`.
    pub id: Option<Id>,
    pub workspace_id: Id,
    pub collection_id: Option<Id>,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: serde_json::Value,
    pub query: serde_json::Value,
    pub body_mode: String,
    pub body: String,
    pub auth: serde_json::Value,
    /// Optional `ssh`-kind connection id to tunnel executions through.
    pub ssh_connection_id: Option<Id>,
    /// Versioned extension object (scripts/docs/settings/…); None = unset.
    pub extras: Option<serde_json::Value>,
    pub position: i64,
}

/// Input for [`ApiClientRepo::create_environment`].
pub struct NewApiEnvironment {
    pub workspace_id: Id,
    pub name: String,
    pub variables: serde_json::Value,
    /// Names of variables whose values live in the Keychain (empty = none).
    pub secret_keys: Vec<String>,
}

/// Input for [`ApiClientRepo::create_automation`] / `update_automation`.
pub struct NewApiAutomation {
    pub workspace_id: Id,
    pub name: String,
    pub steps: serde_json::Value,
}

/// Input for [`ApiClientRepo::insert_history`].
pub struct NewApiHistory {
    pub workspace_id: Id,
    pub method: String,
    pub url: String,
    pub status: Option<i64>,
    pub duration_ms: Option<i64>,
    pub request: serde_json::Value,
    pub response: serde_json::Value,
}

/// Filters for an API-client history listing.
pub struct ApiHistoryQuery {
    pub limit: i64,
    pub q: Option<String>,
    pub status: Option<i64>,
    pub request_id: Option<String>,
    pub source: Option<String>,
}

// --- row mappers ------------------------------------------------------------

fn row_to_collection(r: &sqlx::sqlite::SqliteRow) -> Result<ApiCollection> {
    Ok(ApiCollection {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        parent_id: r.get("parent_id"),
        position: r.get("position"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_request(r: &sqlx::sqlite::SqliteRow) -> Result<ApiRequest> {
    Ok(ApiRequest {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        collection_id: r.get("collection_id"),
        name: r.get("name"),
        method: r.get("method"),
        url: r.get("url"),
        headers: json(&r.get::<String, _>("headers_json"))?,
        query: json(&r.get::<String, _>("query_json"))?,
        body_mode: r.get("body_mode"),
        body: r.get("body"),
        auth: json(&r.get::<String, _>("auth_json"))?,
        ssh_connection_id: r.get("ssh_connection_id"),
        extras: match r.get::<Option<String>, _>("extras_json") {
            Some(s) => Some(json(&s)?),
            None => None,
        },
        position: r.get("position"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_environment(r: &sqlx::sqlite::SqliteRow) -> Result<ApiEnvironment> {
    Ok(ApiEnvironment {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        variables: json(&r.get::<String, _>("variables_json"))?,
        secret_keys: match r.get::<Option<String>, _>("secret_keys_json") {
            Some(s) => serde_json::from_str(&s).unwrap_or_default(),
            None => Vec::new(),
        },
        is_active: r.get::<i64, _>("is_active") != 0,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_history(r: &sqlx::sqlite::SqliteRow) -> Result<ApiHistoryEntry> {
    Ok(ApiHistoryEntry {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        method: r.get("method"),
        url: r.get("url"),
        status: r.get("status"),
        duration_ms: r.get("duration_ms"),
        request: json(&r.get::<String, _>("request_json"))?,
        response: json(&r.get::<String, _>("response_json"))?,
        executed_at: ts(&r.get::<String, _>("executed_at"))?,
    })
}

fn row_to_automation(r: &sqlx::sqlite::SqliteRow) -> Result<ApiAutomation> {
    Ok(ApiAutomation {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        steps: json(&r.get::<String, _>("steps_json"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl ApiClientRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // --- collections --------------------------------------------------------

    pub async fn list_collections(&self, ws: &Id) -> Result<Vec<ApiCollection>> {
        let rows = sqlx::query(
            "SELECT * FROM api_collections WHERE workspace_id = ? ORDER BY position, name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("api collections"))?;
        rows.iter().map(row_to_collection).collect()
    }

    pub async fn get_collection(&self, id: &Id) -> Result<ApiCollection> {
        let r = sqlx::query("SELECT * FROM api_collections WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api collection"))?;
        row_to_collection(&r)
    }

    pub async fn create_collection(&self, c: NewApiCollection) -> Result<ApiCollection> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_collections (id, workspace_id, name, parent_id, position, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&c.workspace_id)
        .bind(&c.name)
        .bind(&c.parent_id)
        .bind(c.position)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create api collection"))?;
        self.get_collection(&id).await
    }

    pub async fn update_collection(
        &self,
        id: &Id,
        name: Option<&str>,
        parent_id: Option<Option<&str>>,
    ) -> Result<ApiCollection> {
        if let Some(v) = name {
            sqlx::query("UPDATE api_collections SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api collection"))?;
        }
        if let Some(v) = parent_id {
            sqlx::query("UPDATE api_collections SET parent_id = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api collection"))?;
        }
        self.get_collection(id).await
    }

    pub async fn delete_collection(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM api_collections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete api collection"))?;
        Ok(())
    }

    // --- requests -----------------------------------------------------------

    pub async fn list_requests(
        &self,
        ws: &Id,
        collection_id: Option<&Id>,
    ) -> Result<Vec<ApiRequest>> {
        let rows = match collection_id {
            Some(cid) => sqlx::query(
                "SELECT * FROM api_requests
                  WHERE workspace_id = ? AND collection_id = ?
                  ORDER BY position, name",
            )
            .bind(ws)
            .bind(cid)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("api requests"))?,
            None => sqlx::query(
                "SELECT * FROM api_requests WHERE workspace_id = ? ORDER BY position, name",
            )
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("api requests"))?,
        };
        rows.iter().map(row_to_request).collect()
    }

    pub async fn get_request(&self, id: &Id) -> Result<ApiRequest> {
        let r = sqlx::query("SELECT * FROM api_requests WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api request"))?;
        row_to_request(&r)
    }

    pub async fn create_request(&self, q: NewApiRequest) -> Result<ApiRequest> {
        let id = q.id.clone().unwrap_or_else(new_id);
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_requests
                (id, workspace_id, collection_id, name, method, url, headers_json, query_json,
                 body_mode, body, auth_json, ssh_connection_id, extras_json, position,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&q.workspace_id)
        .bind(&q.collection_id)
        .bind(&q.name)
        .bind(&q.method)
        .bind(&q.url)
        .bind(q.headers.to_string())
        .bind(q.query.to_string())
        .bind(&q.body_mode)
        .bind(&q.body)
        .bind(q.auth.to_string())
        .bind(&q.ssh_connection_id)
        .bind(q.extras.as_ref().map(|v| v.to_string()))
        .bind(q.position)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create api request"))?;
        self.get_request(&id).await
    }

    /// Full overwrite of the mutable fields of a request (the route handler
    /// builds the new state from the existing row + the patch body).
    pub async fn update_request(&self, id: &Id, q: NewApiRequest) -> Result<ApiRequest> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE api_requests SET
                collection_id = ?, name = ?, method = ?, url = ?, headers_json = ?,
                query_json = ?, body_mode = ?, body = ?, auth_json = ?, ssh_connection_id = ?,
                extras_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&q.collection_id)
        .bind(&q.name)
        .bind(&q.method)
        .bind(&q.url)
        .bind(q.headers.to_string())
        .bind(q.query.to_string())
        .bind(&q.body_mode)
        .bind(&q.body)
        .bind(q.auth.to_string())
        .bind(&q.ssh_connection_id)
        .bind(q.extras.as_ref().map(|v| v.to_string()))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update api request"))?;
        self.get_request(id).await
    }

    pub async fn delete_request(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM api_requests WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete api request"))?;
        Ok(())
    }

    // --- environments -------------------------------------------------------

    pub async fn list_environments(&self, ws: &Id) -> Result<Vec<ApiEnvironment>> {
        let rows =
            sqlx::query("SELECT * FROM api_environments WHERE workspace_id = ? ORDER BY name")
                .bind(ws)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("api environments"))?;
        rows.iter().map(row_to_environment).collect()
    }

    pub async fn get_environment(&self, id: &Id) -> Result<ApiEnvironment> {
        let r = sqlx::query("SELECT * FROM api_environments WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api environment"))?;
        row_to_environment(&r)
    }

    /// The workspace's active environment, if any.
    pub async fn active_environment(&self, ws: &Id) -> Result<Option<ApiEnvironment>> {
        let row = sqlx::query(
            "SELECT * FROM api_environments WHERE workspace_id = ? AND is_active = 1 LIMIT 1",
        )
        .bind(ws)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("active api environment"))?;
        match row {
            Some(r) => Ok(Some(row_to_environment(&r)?)),
            None => Ok(None),
        }
    }

    pub async fn create_environment(&self, e: NewApiEnvironment) -> Result<ApiEnvironment> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_environments (id, workspace_id, name, variables_json, secret_keys_json, is_active, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(&e.workspace_id)
        .bind(&e.name)
        .bind(e.variables.to_string())
        .bind(serde_json::to_string(&e.secret_keys).unwrap_or_else(|_| "[]".into()))
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create api environment"))?;
        self.get_environment(&id).await
    }

    pub async fn update_environment(
        &self,
        id: &Id,
        name: Option<&str>,
        variables: Option<&serde_json::Value>,
        secret_keys: Option<&[String]>,
    ) -> Result<ApiEnvironment> {
        if let Some(v) = name {
            sqlx::query("UPDATE api_environments SET name = ? WHERE id = ?")
                .bind(v)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api environment"))?;
        }
        if let Some(v) = variables {
            sqlx::query("UPDATE api_environments SET variables_json = ? WHERE id = ?")
                .bind(v.to_string())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api environment"))?;
        }
        if let Some(v) = secret_keys {
            sqlx::query("UPDATE api_environments SET secret_keys_json = ? WHERE id = ?")
                .bind(serde_json::to_string(v).unwrap_or_else(|_| "[]".into()))
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api environment"))?;
        }
        self.get_environment(id).await
    }

    pub async fn delete_environment(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM api_environments WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete api environment"))?;
        Ok(())
    }

    /// Mark `id` the workspace's single active environment, deactivating any
    /// others. `id` must belong to `ws`.
    pub async fn set_active(&self, ws: &Id, id: &Id) -> Result<ApiEnvironment> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("activate api environment"))?;
        sqlx::query("UPDATE api_environments SET is_active = 0 WHERE workspace_id = ?")
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("deactivate api environments"))?;
        sqlx::query("UPDATE api_environments SET is_active = 1 WHERE id = ? AND workspace_id = ?")
            .bind(id)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("activate api environment"))?;
        tx.commit()
            .await
            .map_err(dberr("activate api environment"))?;
        self.get_environment(id).await
    }

    // --- automations --------------------------------------------------------

    pub async fn list_automations(&self, ws: &Id) -> Result<Vec<ApiAutomation>> {
        let rows = sqlx::query(
            "SELECT * FROM api_automations WHERE workspace_id = ? ORDER BY created_at, name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("api automations"))?;
        rows.iter().map(row_to_automation).collect()
    }

    pub async fn get_automation(&self, id: &Id) -> Result<ApiAutomation> {
        let r = sqlx::query("SELECT * FROM api_automations WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api automation"))?;
        row_to_automation(&r)
    }

    pub async fn create_automation(&self, a: NewApiAutomation) -> Result<ApiAutomation> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_automations (id, workspace_id, name, steps_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.workspace_id)
        .bind(&a.name)
        .bind(a.steps.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create api automation"))?;
        self.get_automation(&id).await
    }

    /// Update the mutable fields (name and/or steps) of an automation. Each
    /// argument is applied only when present, mirroring the collection updater.
    pub async fn update_automation(
        &self,
        id: &Id,
        name: Option<&str>,
        steps: Option<&serde_json::Value>,
    ) -> Result<ApiAutomation> {
        let now = fmt(Utc::now());
        if let Some(v) = name {
            sqlx::query("UPDATE api_automations SET name = ?, updated_at = ? WHERE id = ?")
                .bind(v)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api automation"))?;
        }
        if let Some(v) = steps {
            sqlx::query("UPDATE api_automations SET steps_json = ?, updated_at = ? WHERE id = ?")
                .bind(v.to_string())
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update api automation"))?;
        }
        self.get_automation(id).await
    }

    pub async fn delete_automation(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM api_automations WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete api automation"))?;
        Ok(())
    }

    // --- history ------------------------------------------------------------

    pub async fn list_history(&self, ws: &Id, limit: i64) -> Result<Vec<ApiHistoryEntry>> {
        self.list_history_filtered(
            ws,
            &ApiHistoryQuery {
                limit,
                q: None,
                status: None,
                request_id: None,
                source: None,
            },
        )
        .await
    }

    /// List history newest-first with optional request/status/source filters.
    pub async fn list_history_filtered(
        &self,
        ws: &Id,
        f: &ApiHistoryQuery,
    ) -> Result<Vec<ApiHistoryEntry>> {
        let mut qb = QueryBuilder::<Sqlite>::new("SELECT * FROM api_history WHERE workspace_id = ");
        qb.push_bind(ws);
        if let Some(q) = &f.q {
            // Escape the LIKE metacharacters so `a_b` matches `a_b`, not `a-b`.
            let pattern = format!(
                "%{}%",
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            qb.push(" AND (url LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR method LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\')");
        }
        if let Some(status) = f.status {
            qb.push(" AND status = ").push_bind(status);
        }
        if let Some(request_id) = &f.request_id {
            qb.push(" AND json_extract(request_json,'$.request_id') = ")
                .push_bind(request_id);
        }
        if let Some(source) = &f.source {
            if source == "human" {
                qb.push(" AND COALESCE(json_extract(request_json,'$.source.kind'),'human') = ");
            } else {
                qb.push(" AND json_extract(request_json,'$.source.kind') = ");
            }
            qb.push_bind(source);
        }
        qb.push(" ORDER BY executed_at DESC, id DESC LIMIT ")
            .push_bind(f.limit);
        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("api history"))?;
        rows.iter().map(row_to_history).collect()
    }

    /// Explicit scalar projection: never reads retained request/response bodies.
    pub async fn list_history_summaries(
        &self,
        ws: &Id,
        f: &ApiHistoryQuery,
    ) -> Result<Vec<ApiHistorySummary>> {
        let mut qb = QueryBuilder::<Sqlite>::new("SELECT id,workspace_id,method,url,status,duration_ms,executed_at,request_id,CAST(source_kind AS TEXT) AS source_kind,source_session_id,source_via FROM api_history WHERE workspace_id = ");
        qb.push_bind(ws);
        if let Some(q) = &f.q {
            // Escape the LIKE metacharacters so `a_b` matches `a_b`, not `a-b`.
            let pattern = format!(
                "%{}%",
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            qb.push(" AND (url LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR method LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\')");
        }
        if let Some(status) = f.status {
            qb.push(" AND status = ").push_bind(status);
        }
        if let Some(request_id) = &f.request_id {
            qb.push(" AND request_id = ").push_bind(request_id);
        }
        if let Some(source) = &f.source {
            qb.push(" AND source_kind = ").push_bind(source);
        }
        qb.push(" ORDER BY executed_at DESC, id DESC LIMIT ")
            .push_bind(f.limit);
        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("api history"))?;
        rows.iter()
            .map(|r| {
                Ok(ApiHistorySummary {
                    id: r.get("id"),
                    workspace_id: r.get("workspace_id"),
                    method: r.get("method"),
                    url: r.get("url"),
                    status: r.get("status"),
                    duration_ms: r.get("duration_ms"),
                    executed_at: ts(&r.get::<String, _>("executed_at"))?,
                    request_id: r.get("request_id"),
                    source: ApiHistorySourceSummary {
                        kind: r.get("source_kind"),
                        session_id: r.get("source_session_id"),
                        via: r.get("source_via"),
                    },
                })
            })
            .collect()
    }

    /// Fetch one history entry by id.
    pub async fn get_history(&self, id: &Id) -> Result<ApiHistoryEntry> {
        let r = sqlx::query("SELECT * FROM api_history WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api history"))?;
        row_to_history(&r)
    }

    pub async fn insert_history(&self, h: NewApiHistory) -> Result<ApiHistoryEntry> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_history
                (id, workspace_id, method, url, status, duration_ms, request_json,
                 response_json, executed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&h.workspace_id)
        .bind(&h.method)
        .bind(&h.url)
        .bind(h.status)
        .bind(h.duration_ms)
        .bind(h.request.to_string())
        .bind(h.response.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("insert api history"))?;
        let r = sqlx::query("SELECT * FROM api_history WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("api history"))?;
        row_to_history(&r)
    }

    /// Retention for `ws`'s request history: drop rows older than
    /// `max_age_days` (0 = no age limit), then keep only the newest `max_rows`
    /// (0 = no row cap). Runs after each insert — a runtime cap, no migration;
    /// existing rows beyond the cap are trimmed on the workspace's next run.
    /// Returns the number of rows deleted.
    pub async fn prune_history(&self, ws: &Id, max_rows: i64, max_age_days: i64) -> Result<u64> {
        let mut deleted = 0u64;
        if max_age_days > 0 {
            let cutoff = fmt(Utc::now() - chrono::Duration::days(max_age_days));
            deleted += sqlx::query("DELETE FROM api_history WHERE workspace_id = ? AND executed_at < ?")
                .bind(ws)
                .bind(&cutoff)
                .execute(&self.pool)
                .await
                .map_err(dberr("prune api history"))?
                .rows_affected();
        }
        if max_rows > 0 {
            deleted += sqlx::query(
                "DELETE FROM api_history WHERE workspace_id = ? AND id IN (
                    SELECT id FROM api_history WHERE workspace_id = ?
                    ORDER BY executed_at DESC, id DESC LIMIT -1 OFFSET ?)",
            )
            .bind(ws)
            .bind(ws)
            .bind(max_rows)
            .execute(&self.pool)
            .await
            .map_err(dberr("prune api history"))?
            .rows_affected();
        }
        Ok(deleted)
    }

    pub async fn clear_history(&self, ws: &Id) -> Result<()> {
        sqlx::query("DELETE FROM api_history WHERE workspace_id = ?")
            .bind(ws)
            .execute(&self.pool)
            .await
            .map_err(dberr("clear api history"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::new_id;
    use serde_json::json as jval;

    async fn setup() -> (SqlitePool, Id) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        // A workspace row is required for the FK.
        let ws = new_id();
        let user = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, '', ?)",
        )
        .bind(&user)
        .bind(format!("u-{user}"))
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
        )
        .bind(&ws)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        (pool, ws)
    }

    #[tokio::test]
    async fn collection_and_request_crud() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let col = repo
            .create_collection(NewApiCollection {
                workspace_id: ws.clone(),
                name: "My API".into(),
                parent_id: None,
                position: 0,
            })
            .await
            .unwrap();
        assert_eq!(col.name, "My API");
        assert_eq!(repo.list_collections(&ws).await.unwrap().len(), 1);

        let col = repo
            .update_collection(&col.id, Some("Renamed"), None)
            .await
            .unwrap();
        assert_eq!(col.name, "Renamed");

        let req = repo
            .create_request(NewApiRequest {
                id: None,
                workspace_id: ws.clone(),
                collection_id: Some(col.id.clone()),
                name: "list users".into(),
                method: "GET".into(),
                url: "https://api.test/users".into(),
                headers: jval!([{"key":"Accept","value":"application/json","enabled":true}]),
                query: jval!([]),
                body_mode: "none".into(),
                body: String::new(),
                auth: jval!({"type":"none"}),
                ssh_connection_id: None,
                extras: None,
                position: 0,
            })
            .await
            .unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.headers[0]["key"], "Accept");

        // filter by collection
        let in_col = repo.list_requests(&ws, Some(&col.id)).await.unwrap();
        assert_eq!(in_col.len(), 1);

        // update
        let updated = repo
            .update_request(
                &req.id,
                NewApiRequest {
                    id: None,
                    workspace_id: ws.clone(),
                    collection_id: Some(col.id.clone()),
                    name: "list users".into(),
                    method: "POST".into(),
                    url: "https://api.test/users".into(),
                    headers: jval!([]),
                    query: jval!([]),
                    body_mode: "json".into(),
                    body: "{}".into(),
                    auth: jval!({"type":"none"}),
                    ssh_connection_id: None,
                    extras: None,
                    position: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.method, "POST");
        assert_eq!(updated.body_mode, "json");

        // deleting the collection nulls the request's collection_id (SET NULL)
        repo.delete_collection(&col.id).await.unwrap();
        let after = repo.get_request(&req.id).await.unwrap();
        assert!(after.collection_id.is_none());

        repo.delete_request(&req.id).await.unwrap();
        assert!(repo.list_requests(&ws, None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn environment_single_active() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let a = repo
            .create_environment(NewApiEnvironment {
                workspace_id: ws.clone(),
                name: "dev".into(),
                variables: jval!({"base":"http://localhost"}),
                secret_keys: Vec::new(),
            })
            .await
            .unwrap();
        let b = repo
            .create_environment(NewApiEnvironment {
                workspace_id: ws.clone(),
                name: "prod".into(),
                variables: jval!({"base":"https://api.test"}),
                secret_keys: Vec::new(),
            })
            .await
            .unwrap();
        assert!(!a.is_active && !b.is_active);
        assert!(repo.active_environment(&ws).await.unwrap().is_none());

        repo.set_active(&ws, &a.id).await.unwrap();
        assert_eq!(
            repo.active_environment(&ws).await.unwrap().unwrap().id,
            a.id
        );

        // activating b deactivates a — only one active per workspace
        repo.set_active(&ws, &b.id).await.unwrap();
        let active = repo.active_environment(&ws).await.unwrap().unwrap();
        assert_eq!(active.id, b.id);
        assert!(!repo.get_environment(&a.id).await.unwrap().is_active);

        let updated = repo
            .update_environment(&b.id, Some("production"), Some(&jval!({"k":"v"})), None)
            .await
            .unwrap();
        assert_eq!(updated.name, "production");
        assert_eq!(updated.variables["k"], "v");

        repo.delete_environment(&a.id).await.unwrap();
        assert_eq!(repo.list_environments(&ws).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn request_extras_round_trip() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let extras = jval!({
            "v": 1,
            "transport": "http",
            "graphql_variables": "{\"id\": 1}",
            "docs_md": "# Users\nList them.",
            "scripts": { "pre": "pm.environment.set('a','1')", "post": "pm.test('ok', () => {})" },
            "settings": { "timeout_ms": 30000, "follow_redirects": false, "tls_verify": true },
        });
        let base = NewApiRequest {
            id: None,
            workspace_id: ws.clone(),
            collection_id: None,
            name: "with extras".into(),
            method: "GET".into(),
            url: "https://api.test/x".into(),
            headers: jval!([]),
            query: jval!([]),
            body_mode: "none".into(),
            body: String::new(),
            auth: jval!({"type":"none"}),
            ssh_connection_id: None,
            extras: Some(extras.clone()),
            position: 0,
        };
        let req = repo.create_request(base).await.unwrap();
        assert_eq!(req.extras.as_ref().unwrap(), &extras);

        // list round-trips too
        let listed = repo.list_requests(&ws, None).await.unwrap();
        assert_eq!(
            listed[0].extras.as_ref().unwrap()["docs_md"],
            "# Users\nList them."
        );

        // update to None clears the column
        let cleared = repo
            .update_request(
                &req.id,
                NewApiRequest {
                    id: None,
                    workspace_id: ws.clone(),
                    collection_id: None,
                    name: "with extras".into(),
                    method: "GET".into(),
                    url: "https://api.test/x".into(),
                    headers: jval!([]),
                    query: jval!([]),
                    body_mode: "none".into(),
                    body: String::new(),
                    auth: jval!({"type":"none"}),
                    ssh_connection_id: None,
                    extras: None,
                    position: 0,
                },
            )
            .await
            .unwrap();
        assert!(cleared.extras.is_none());
    }

    #[tokio::test]
    async fn environment_secret_keys_round_trip() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let env = repo
            .create_environment(NewApiEnvironment {
                workspace_id: ws.clone(),
                name: "prod".into(),
                variables: jval!({"base":"https://api.test"}),
                secret_keys: vec!["api_token".into()],
            })
            .await
            .unwrap();
        assert_eq!(env.secret_keys, vec!["api_token".to_string()]);
        // Secret values never live in the row.
        assert!(env.variables.get("api_token").is_none());

        let updated = repo
            .update_environment(
                &env.id,
                None,
                None,
                Some(&["api_token".to_string(), "db_pass".to_string()]),
            )
            .await
            .unwrap();
        assert_eq!(updated.secret_keys.len(), 2);

        // name/variables untouched by a secret-keys-only update
        assert_eq!(updated.name, "prod");
        assert_eq!(updated.variables["base"], "https://api.test");
    }

    #[tokio::test]
    async fn history_insert_list_clear() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        for i in 0..3 {
            repo.insert_history(NewApiHistory {
                workspace_id: ws.clone(),
                method: "GET".into(),
                url: format!("https://api.test/{i}"),
                status: Some(200),
                duration_ms: Some(12),
                request: jval!({"method":"GET"}),
                response: jval!({"status":200}),
            })
            .await
            .unwrap();
        }
        let all = repo.list_history(&ws, 10).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].status, Some(200));

        let limited = repo.list_history(&ws, 2).await.unwrap();
        assert_eq!(limited.len(), 2);

        repo.clear_history(&ws).await.unwrap();
        assert!(repo.list_history(&ws, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn prune_history_caps_rows_and_age_per_workspace() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool.clone());
        let row = |url: &str| NewApiHistory {
            workspace_id: ws.clone(),
            method: "GET".into(),
            url: url.into(),
            status: Some(200),
            duration_ms: Some(1),
            request: jval!({}),
            response: jval!({"status": 200}),
        };
        for i in 0..5 {
            repo.insert_history(row(&format!("https://h.test/{i}")))
                .await
                .unwrap();
        }
        // Age out one row by back-dating it well past the limit.
        let old = fmt(Utc::now() - chrono::Duration::days(400));
        sqlx::query("UPDATE api_history SET executed_at = ? WHERE url = 'https://h.test/0'")
            .bind(&old)
            .execute(&pool)
            .await
            .unwrap();

        // Limits of 0 disable pruning entirely.
        assert_eq!(repo.prune_history(&ws, 0, 0).await.unwrap(), 0);
        assert_eq!(repo.list_history(&ws, 10).await.unwrap().len(), 5);

        // Age: the back-dated row goes; then the row cap keeps the newest 2.
        let deleted = repo.prune_history(&ws, 2, 90).await.unwrap();
        assert_eq!(deleted, 3);
        let left = repo.list_history(&ws, 10).await.unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|h| h.url != "https://h.test/0"));
    }

    #[tokio::test]
    async fn history_filter_by_q_status_source_and_request_id() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let agent = repo
            .insert_history(NewApiHistory {
                workspace_id: ws.clone(),
                method: "POST".into(),
                url: "https://agent.test/login".into(),
                status: Some(201),
                duration_ms: Some(20),
                request: jval!({
                    "request_id": "req-agent",
                    "source": {"kind": "agent", "session_id": "s1"},
                }),
                response: jval!({"status": 201}),
            })
            .await
            .unwrap();
        repo.insert_history(NewApiHistory {
            workspace_id: ws.clone(),
            method: "GET".into(),
            url: "https://human.test/users".into(),
            status: Some(200),
            duration_ms: Some(10),
            request: jval!({
                "request_id": "req-human",
                "source": {"kind": "human", "session_id": null},
            }),
            response: jval!({"status": 200}),
        })
        .await
        .unwrap();
        repo.insert_history(NewApiHistory {
            workspace_id: ws.clone(),
            method: "DELETE".into(),
            url: "https://legacy.test/users/1".into(),
            status: Some(204),
            duration_ms: Some(11),
            request: jval!({"method": "DELETE"}),
            response: jval!({"status": 204}),
        })
        .await
        .unwrap();

        let filter = |q, status, request_id, source| ApiHistoryQuery {
            limit: 10,
            q,
            status,
            request_id,
            source,
        };
        let by_q = repo
            .list_history_filtered(&ws, &filter(Some("LOGIN".into()), None, None, None))
            .await
            .unwrap();
        assert_eq!(by_q.len(), 1);
        assert_eq!(by_q[0].id, agent.id);

        let by_status = repo
            .list_history_filtered(&ws, &filter(None, Some(201), None, None))
            .await
            .unwrap();
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].id, agent.id);

        let by_request = repo
            .list_history_filtered(&ws, &filter(None, None, Some("req-agent".into()), None))
            .await
            .unwrap();
        assert_eq!(by_request.len(), 1);
        assert_eq!(by_request[0].id, agent.id);

        let by_agent = repo
            .list_history_filtered(&ws, &filter(None, None, None, Some("agent".into())))
            .await
            .unwrap();
        assert_eq!(by_agent.len(), 1);
        assert_eq!(by_agent[0].id, agent.id);

        let by_human = repo
            .list_history_filtered(&ws, &filter(None, None, None, Some("human".into())))
            .await
            .unwrap();
        assert_eq!(by_human.len(), 2, "explicit human + legacy row");
    }

    #[tokio::test]
    async fn get_history_round_trips() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);
        let inserted = repo
            .insert_history(NewApiHistory {
                workspace_id: ws,
                method: "PATCH".into(),
                url: "https://api.test/items/1".into(),
                status: Some(202),
                duration_ms: Some(42),
                request: jval!({"name": "update item"}),
                response: jval!({"status": 202, "body": "ok"}),
            })
            .await
            .unwrap();

        let fetched = repo.get_history(&inserted.id).await.unwrap();
        assert_eq!(fetched.id, inserted.id);
        assert_eq!(fetched.method, "PATCH");
        assert_eq!(fetched.response["body"], "ok");
    }

    #[tokio::test]
    async fn automation_crud() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);

        let steps = jval!([
            {
                "request_id": "r1",
                "assertions": [{"kind": "status", "op": "eq", "value": 200}],
                "extract": [{"path": "token", "var": "auth"}],
            }
        ]);
        let auto = repo
            .create_automation(NewApiAutomation {
                workspace_id: ws.clone(),
                name: "login flow".into(),
                steps: steps.clone(),
            })
            .await
            .unwrap();
        assert_eq!(auto.name, "login flow");
        assert_eq!(auto.steps[0]["request_id"], "r1");
        assert_eq!(repo.list_automations(&ws).await.unwrap().len(), 1);

        let renamed = repo
            .update_automation(&auto.id, Some("auth flow"), None)
            .await
            .unwrap();
        assert_eq!(renamed.name, "auth flow");
        // steps preserved when only name changes
        assert_eq!(renamed.steps[0]["request_id"], "r1");

        let new_steps = jval!([{"request_id": "r2", "assertions": [], "extract": []}]);
        let restepped = repo
            .update_automation(&auto.id, None, Some(&new_steps))
            .await
            .unwrap();
        assert_eq!(restepped.name, "auth flow");
        assert_eq!(restepped.steps[0]["request_id"], "r2");

        repo.delete_automation(&auto.id).await.unwrap();
        assert!(repo.list_automations(&ws).await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn history_summaries_do_not_transfer_bodies_and_preserve_detail() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool);
        let body = format!("BODY_SENTINEL{}", "x".repeat(512 * 1024));
        let request = jval!({"request_id":"request-1","body":"REQUEST_BODY_SENTINEL","source":{"kind":"agent","session_id":"session-1","via":"mcp"}});
        let mut last_id = String::new();
        for _ in 0..100 {
            last_id = repo
                .insert_history(NewApiHistory {
                    workspace_id: ws.clone(),
                    method: "POST".into(),
                    url: "https://fixture.invalid/a_b".into(),
                    status: Some(201),
                    duration_ms: Some(3),
                    request: request.clone(),
                    response: jval!({"body":body}),
                })
                .await
                .unwrap()
                .id;
        }
        let filter = ApiHistoryQuery {
            limit: 100,
            q: Some("a_b".into()),
            status: Some(201),
            request_id: Some("request-1".into()),
            source: Some("agent".into()),
        };
        let rows = repo.list_history_summaries(&ws, &filter).await.unwrap();
        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0].source.session_id.as_deref(), Some("session-1"));
        let payload = serde_json::to_string(&rows).unwrap();
        assert!(!payload.contains("BODY_SENTINEL"));
        assert!(!payload.contains("REQUEST_BODY_SENTINEL"));
        assert!(payload.len() < 100 * 1024);
        let detail = repo.get_history(&last_id).await.unwrap();
        assert_eq!(detail.request, request);
        assert_eq!(detail.response["body"], body);
        assert_eq!(repo.list_history(&ws, 100).await.unwrap().len(), 100);
        assert!(repo
            .list_history_summaries(&"other-workspace".into(), &filter)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn history_summary_metadata_tracks_direct_legacy_writes_and_malformed_json() {
        let (pool, ws) = setup().await;
        let repo = ApiClientRepo::new(pool.clone());
        let samples = [
            ("legacy", r#"{"source":"automation_run"}"#, "human"),
            ("null", r#"{"source":{"kind":null}}"#, "human"),
            (
                "agent",
                r#"{"request_id":"req","source":{"kind":"agent","session_id":"s"}}"#,
                "agent",
            ),
            ("custom", r#"{"source":{"kind":"custom"}}"#, "custom"),
            ("malformed", "{broken", "human"),
        ];
        for (id, request, _) in &samples {
            sqlx::query("INSERT INTO api_history(id,workspace_id,method,url,request_json,response_json,executed_at) VALUES(?,?,'GET','https://fixture.invalid',?,'{}','2026-09-13T10:00:00Z')").bind(id).bind(&ws).bind(request).execute(&pool).await.unwrap();
        }
        let filter = ApiHistoryQuery {
            limit: 100,
            q: None,
            status: None,
            request_id: None,
            source: None,
        };
        let rows = repo.list_history_summaries(&ws, &filter).await.unwrap();
        assert_eq!(rows.len(), samples.len());
        for (id, request, kind) in samples {
            assert_eq!(rows.iter().find(|r| r.id == id).unwrap().source.kind, kind);
            let preserved: String =
                sqlx::query_scalar("SELECT request_json FROM api_history WHERE id=?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(preserved, request);
        }
        sqlx::query("UPDATE api_history SET request_json=? WHERE id='legacy'")
            .bind(r#"{"request_id":"updated","source":{"kind":"agent","via":"fixture"}}"#)
            .execute(&pool)
            .await
            .unwrap();
        let changed = repo
            .list_history_summaries(
                &ws,
                &ApiHistoryQuery {
                    request_id: Some("updated".into()),
                    source: Some("agent".into()),
                    ..filter
                },
            )
            .await
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].source.via.as_deref(), Some("fixture"));
    }

    #[tokio::test]
    async fn history_summary_backfill_preserves_original_bytes_and_filter_types() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql("CREATE TABLE api_history(id TEXT PRIMARY KEY,workspace_id TEXT,executed_at TEXT,request_json TEXT,response_json TEXT)").execute(&pool).await.unwrap();
        let samples = [
            (
                "a",
                r#"{"source":{"kind":"agent","session_id":"s","via":"mcp"},"request_id":"r","body":"retain"}"#,
            ),
            ("b", "{malformed"),
            ("c", r#"{"source":{"kind":1}}"#),
            ("d", r#"{"source":{"kind":"1"}}"#),
        ];
        for (id, request) in samples {
            sqlx::query("INSERT INTO api_history VALUES(?,'workspace','2026-09-13',?,'  {\"body\":\"retain\"}  ')").bind(id).bind(request).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(include_str!("../migrations/0132_api_history_summaries.sql"))
            .execute(&pool)
            .await
            .unwrap();
        for (id, request) in samples {
            let row = sqlx::query("SELECT request_json,response_json FROM api_history WHERE id=?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(row.get::<String, _>("request_json"), request);
            assert_eq!(
                row.get::<String, _>("response_json"),
                "  {\"body\":\"retain\"}  "
            );
        }
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM api_history WHERE source_kind=?")
            .bind("1")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ids, vec!["d"]);
        let source: String = sqlx::query_scalar("SELECT source_kind FROM api_history WHERE id='b'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(source, "human");
        for query in ["EXPLAIN QUERY PLAN SELECT id FROM api_history WHERE workspace_id='workspace' AND source_kind='agent' ORDER BY executed_at DESC,id DESC LIMIT 100","EXPLAIN QUERY PLAN SELECT id FROM api_history WHERE workspace_id='workspace' AND request_id='r' ORDER BY executed_at DESC,id DESC LIMIT 100"] {
            let rows=sqlx::query(query).fetch_all(&pool).await.unwrap();
            assert!(rows.iter().any(|r| r.get::<String,_>("detail").contains("USING COVERING INDEX")));
            assert!(!rows.iter().any(|r| r.get::<String,_>("detail").contains("TEMP B-TREE")));
        }
    }
}

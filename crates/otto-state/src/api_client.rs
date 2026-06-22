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
    ApiAutomation, ApiCollection, ApiEnvironment, ApiHistoryEntry, ApiRequest,
};
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

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
    pub position: i64,
}

/// Input for [`ApiClientRepo::create_environment`].
pub struct NewApiEnvironment {
    pub workspace_id: Id,
    pub name: String,
    pub variables: serde_json::Value,
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
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO api_requests
                (id, workspace_id, collection_id, name, method, url, headers_json, query_json,
                 body_mode, body, auth_json, ssh_connection_id, position, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                updated_at = ?
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
        let rows = sqlx::query(
            "SELECT * FROM api_environments WHERE workspace_id = ? ORDER BY name",
        )
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
            "INSERT INTO api_environments (id, workspace_id, name, variables_json, is_active, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(&e.workspace_id)
        .bind(&e.name)
        .bind(e.variables.to_string())
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
        sqlx::query(
            "UPDATE api_environments SET is_active = 1 WHERE id = ? AND workspace_id = ?",
        )
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
        let rows = sqlx::query(
            "SELECT * FROM api_history WHERE workspace_id = ?
              ORDER BY executed_at DESC, id DESC LIMIT ?",
        )
        .bind(ws)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("api history"))?;
        rows.iter().map(row_to_history).collect()
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
        sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, '', ?)")
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
                position: 0,
            })
            .await
            .unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.headers[0]["key"], "Accept");

        // filter by collection
        let in_col = repo
            .list_requests(&ws, Some(&col.id))
            .await
            .unwrap();
        assert_eq!(in_col.len(), 1);

        // update
        let updated = repo
            .update_request(
                &req.id,
                NewApiRequest {
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
            })
            .await
            .unwrap();
        let b = repo
            .create_environment(NewApiEnvironment {
                workspace_id: ws.clone(),
                name: "prod".into(),
                variables: jval!({"base":"https://api.test"}),
            })
            .await
            .unwrap();
        assert!(!a.is_active && !b.is_active);
        assert!(repo.active_environment(&ws).await.unwrap().is_none());

        repo.set_active(&ws, &a.id).await.unwrap();
        assert_eq!(repo.active_environment(&ws).await.unwrap().unwrap().id, a.id);

        // activating b deactivates a — only one active per workspace
        repo.set_active(&ws, &b.id).await.unwrap();
        let active = repo.active_environment(&ws).await.unwrap().unwrap();
        assert_eq!(active.id, b.id);
        assert!(!repo.get_environment(&a.id).await.unwrap().is_active);

        let updated = repo
            .update_environment(&b.id, Some("production"), Some(&jval!({"k":"v"})))
            .await
            .unwrap();
        assert_eq!(updated.name, "production");
        assert_eq!(updated.variables["k"], "v");

        repo.delete_environment(&a.id).await.unwrap();
        assert_eq!(repo.list_environments(&ws).await.unwrap().len(), 1);
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
}

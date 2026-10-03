//! Kubernetes pod HTTP actions (K-3) — saved per-workload HTTP requests, see
//! migration `0158_k8s_pod_actions.sql`. The row is the wire `PodAction` DTO
//! (serde-derived). Template variables (`{{logger}}`, `{{level}}`) stay raw:
//! the UI fills them at run time.

use std::collections::BTreeMap;

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::convert::{dberr, fmt};
use crate::DbPool;

/// Wire + row shape of a saved pod action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodAction {
    pub id: Id,
    pub cluster_id: Id,
    pub namespace: String,
    pub workload_kind: String,
    pub workload: String,
    pub name: String,
    pub method: String,
    pub port: u16,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body_template: Option<String>,
    pub created_by: Option<Id>,
    pub updated_at: String,
}

/// Upsert payload (`id: None` ⇒ a new row). Validation is the caller's job.
#[derive(Debug, Clone)]
pub struct UpsertPodAction {
    pub id: Option<Id>,
    pub cluster_id: Id,
    pub namespace: String,
    pub workload_kind: String,
    pub workload: String,
    pub name: String,
    pub method: String,
    pub port: u16,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body_template: Option<String>,
    pub created_by: Option<Id>,
}

/// Optional list filters (all `None` ⇒ every action of the cluster).
#[derive(Debug, Clone, Default)]
pub struct PodActionFilter {
    pub namespace: Option<String>,
    pub workload_kind: Option<String>,
    pub workload: Option<String>,
}

fn row_to_action(r: &sqlx::sqlite::SqliteRow) -> PodAction {
    let headers: BTreeMap<String, String> =
        serde_json::from_str(&r.get::<String, _>("headers_json")).unwrap_or_default();
    PodAction {
        id: r.get("id"),
        cluster_id: r.get("cluster_id"),
        namespace: r.get("namespace"),
        workload_kind: r.get("workload_kind"),
        workload: r.get("workload"),
        name: r.get("name"),
        method: r.get("method"),
        port: u16::try_from(r.get::<i64, _>("port")).unwrap_or(0),
        path: r.get("path"),
        headers,
        body_template: r.get("body_template"),
        created_by: r.get("created_by"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Clone)]
pub struct K8sPodActionsRepo {
    pool: DbPool,
}

impl K8sPodActionsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    pub async fn list(&self, cluster_id: &Id, f: &PodActionFilter) -> Result<Vec<PodAction>> {
        let rows = sqlx::query(
            "SELECT * FROM k8s_pod_actions
             WHERE cluster_id = ?
               AND (? IS NULL OR namespace = ?)
               AND (? IS NULL OR workload_kind = ?)
               AND (? IS NULL OR workload = ?)
             ORDER BY workload, name COLLATE NOCASE, id",
        )
        .bind(cluster_id)
        .bind(&f.namespace)
        .bind(&f.namespace)
        .bind(&f.workload_kind)
        .bind(&f.workload_kind)
        .bind(&f.workload)
        .bind(&f.workload)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list k8s pod actions"))?;
        Ok(rows.iter().map(row_to_action).collect())
    }

    pub async fn get(&self, cluster_id: &Id, id: &Id) -> Result<PodAction> {
        let r = sqlx::query("SELECT * FROM k8s_pod_actions WHERE cluster_id = ? AND id = ?")
            .bind(cluster_id)
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("k8s pod action"))?;
        Ok(row_to_action(&r))
    }

    /// Insert, or replace the row with the same `(cluster_id, id)`.
    pub async fn upsert(&self, a: UpsertPodAction) -> Result<PodAction> {
        let id = a.id.clone().unwrap_or_else(new_id);
        let headers = serde_json::to_string(&a.headers).unwrap_or_else(|_| "{}".into());
        sqlx::query(
            "INSERT INTO k8s_pod_actions (id, cluster_id, namespace, workload_kind, workload,
                                          name, method, port, path, headers_json,
                                          body_template, created_by, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 namespace = excluded.namespace, workload_kind = excluded.workload_kind,
                 workload = excluded.workload, name = excluded.name,
                 method = excluded.method, port = excluded.port, path = excluded.path,
                 headers_json = excluded.headers_json,
                 body_template = excluded.body_template, updated_at = excluded.updated_at
             WHERE k8s_pod_actions.cluster_id = excluded.cluster_id",
        )
        .bind(&id)
        .bind(&a.cluster_id)
        .bind(&a.namespace)
        .bind(&a.workload_kind)
        .bind(&a.workload)
        .bind(&a.name)
        .bind(&a.method)
        .bind(i64::from(a.port))
        .bind(&a.path)
        .bind(headers)
        .bind(&a.body_template)
        .bind(&a.created_by)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("save k8s pod action"))?;
        self.get(&a.cluster_id, &id).await
    }

    /// `true` when a row was removed.
    pub async fn delete(&self, cluster_id: &Id, id: &Id) -> Result<bool> {
        let r = sqlx::query("DELETE FROM k8s_pod_actions WHERE cluster_id = ? AND id = ?")
            .bind(cluster_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete k8s pod action"))?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{K8sClusterSource, K8sClustersRepo, NewK8sCluster};
    use otto_core::domain::Environment;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool.into()
    }

    fn action(id: Option<&str>, name: &str) -> UpsertPodAction {
        UpsertPodAction {
            id: id.map(Into::into),
            cluster_id: "c1".into(),
            namespace: "shop".into(),
            workload_kind: "deployment".into(),
            workload: "api".into(),
            name: name.into(),
            method: "POST".into(),
            port: 8081,
            path: "/actuator/loggers/{{logger}}".into(),
            headers: BTreeMap::from([("Content-Type".into(), "application/json".into())]),
            body_template: Some(r#"{"configuredLevel":"{{level}}"}"#.into()),
            created_by: None,
        }
    }

    #[tokio::test]
    async fn upsert_list_delete_roundtrip_and_cascade() {
        let pool = pool().await;
        K8sClustersRepo::new(pool.clone())
            .create(NewK8sCluster {
                id: "c1".into(),
                name: "c1".into(),
                source: K8sClusterSource::Kubeconfig,
                kubeconfig_path: None,
                context_name: "ctx".into(),
                default_namespace: None,
                aws_account_id: None,
                environment: Environment::Dev,
                color: None,
                params: serde_json::json!({}),
                created_by: None,
            })
            .await
            .unwrap();
        let repo = K8sPodActionsRepo::new(pool.clone());
        let a = repo.upsert(action(None, "Set level")).await.unwrap();
        assert_eq!(a.port, 8081);
        assert_eq!(a.headers["Content-Type"], "application/json");
        assert!(a.body_template.as_deref().unwrap().contains("{{level}}"));

        let renamed = repo
            .upsert(action(Some(a.id.as_str()), "Renamed"))
            .await
            .unwrap();
        assert_eq!(renamed.id, a.id);
        assert_eq!(renamed.name, "Renamed");

        let all = repo
            .list(&"c1".into(), &PodActionFilter::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        let none = repo
            .list(
                &"c1".into(),
                &PodActionFilter {
                    workload: Some("other".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(none.is_empty());

        assert!(repo.delete(&"c1".into(), &a.id).await.unwrap());
        assert!(!repo.delete(&"c1".into(), &a.id).await.unwrap());

        repo.upsert(action(None, "again")).await.unwrap();
        K8sClustersRepo::new(pool.clone())
            .delete(&"c1".into())
            .await
            .unwrap();
        assert!(repo
            .list(&"c1".into(), &PodActionFilter::default())
            .await
            .unwrap()
            .is_empty());
    }
}

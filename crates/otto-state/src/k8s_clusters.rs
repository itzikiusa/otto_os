//! Kubernetes console — cluster (kubeconfig context) registry repository.
//!
//! One row = one `(kubeconfig file, context name)` pair plus UI defaults; see
//! migration `0114_k8s_clusters.sql` for the three `source`s (`kubeconfig`,
//! `imported`, `eks`) and `docs/design/aws-k8s-consoles.md` §3.1 for the wire
//! shape. The row IS the wire `K8sCluster` DTO (serde-derived) so the HTTP
//! layer never re-maps it. Secrets never live here — an imported kubeconfig is
//! a 0600 file under `<data_dir>/kube/`, and EKS token refresh goes through the
//! linked `aws_accounts` row.

use chrono::{DateTime, Utc};
use otto_core::domain::Environment;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

/// Where a cluster row came from — drives whether Otto owns the kubeconfig file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum K8sClusterSource {
    /// An existing context in `~/.kube/config` / `$KUBECONFIG` (user-owned file).
    Kubeconfig,
    /// YAML pasted in the UI → `<data_dir>/kube/<id>.yaml` (Otto-owned).
    Imported,
    /// Written by `aws eks update-kubeconfig` into `<data_dir>/kube/<id>.yaml`
    /// (Otto-owned, linked to an AWS account).
    Eks,
}

impl K8sClusterSource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "kubeconfig" => Some(Self::Kubeconfig),
            "imported" => Some(Self::Imported),
            "eks" => Some(Self::Eks),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Kubeconfig => "kubeconfig",
            Self::Imported => "imported",
            Self::Eks => "eks",
        }
    }

    /// `true` when the kubeconfig file belongs to Otto and is removed with the row.
    pub fn otto_owned_kubeconfig(&self) -> bool {
        matches!(self, Self::Imported | Self::Eks)
    }
}

/// Wire + row shape of a registered cluster (contract §3.1 `K8sCluster`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sCluster {
    pub id: Id,
    pub name: String,
    pub source: K8sClusterSource,
    /// `None` ⇒ kubectl's own default resolution (`~/.kube/config`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kubeconfig_path: Option<String>,
    pub context_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_account_id: Option<Id>,
    pub environment: Environment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Non-secret extras (`eks_region`, `eks_cluster`, …) — `color` is lifted
    /// out of here into its own field for the UI.
    pub params: Value,
    /// Cached capability probe (`K8sCapabilities`), `None` until first probed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    pub created_by: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Insert payload. The caller picks the `id` up front because imported / EKS
/// kubeconfigs are written to `<data_dir>/kube/<id>.yaml` BEFORE the row exists
/// (the file must validate first).
pub struct NewK8sCluster {
    pub id: Id,
    pub name: String,
    pub source: K8sClusterSource,
    pub kubeconfig_path: Option<String>,
    pub context_name: String,
    pub default_namespace: Option<String>,
    pub aws_account_id: Option<Id>,
    pub environment: Environment,
    pub color: Option<String>,
    pub params: Value,
    pub created_by: Option<Id>,
}

/// Partial update — every `None` keeps the stored value. `Some(None)` on the
/// double-optional fields clears them.
#[derive(Debug, Default)]
pub struct K8sClusterPatch {
    pub name: Option<String>,
    pub kubeconfig_path: Option<Option<String>>,
    pub context_name: Option<String>,
    pub default_namespace: Option<Option<String>>,
    pub environment: Option<Environment>,
    pub color: Option<Option<String>>,
}

fn row_to_cluster(r: &sqlx::sqlite::SqliteRow) -> Result<K8sCluster> {
    let params: Value = json(&r.get::<String, _>("params_json"))?;
    let color = params
        .get("color")
        .and_then(Value::as_str)
        .map(str::to_string);
    let capabilities = r
        .get::<Option<String>, _>("capabilities_json")
        .as_deref()
        .map(json)
        .transpose()?;
    Ok(K8sCluster {
        id: r.get("id"),
        name: r.get("name"),
        source: K8sClusterSource::parse(&r.get::<String, _>("source"))
            .ok_or_else(|| Error::Internal("bad k8s cluster source".into()))?,
        kubeconfig_path: r.get("kubeconfig_path"),
        context_name: r.get("context_name"),
        default_namespace: r.get("default_namespace"),
        aws_account_id: r.get("aws_account_id"),
        environment: Environment::parse(&r.get::<String, _>("environment"))
            .ok_or_else(|| Error::Internal("bad k8s cluster environment".into()))?,
        color,
        params,
        capabilities,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        last_used_at: r
            .get::<Option<String>, _>("last_used_at")
            .as_deref()
            .map(ts)
            .transpose()?,
    })
}

/// Fold `color` into the stored `params_json` (it is a UI extra, not a column).
fn params_with_color(mut params: Value, color: Option<&str>) -> Value {
    if !params.is_object() {
        params = Value::Object(Default::default());
    }
    let obj = params.as_object_mut().expect("object");
    match color.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => {
            obj.insert("color".into(), Value::String(c.to_string()));
        }
        None => {
            obj.remove("color");
        }
    }
    params
}

#[derive(Clone)]
pub struct K8sClustersRepo {
    pool: SqlitePool,
}

impl K8sClustersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewK8sCluster) -> Result<K8sCluster> {
        let now = fmt(Utc::now());
        let params = params_with_color(c.params, c.color.as_deref());
        sqlx::query(
            "INSERT INTO k8s_clusters (id, name, source, kubeconfig_path, context_name,
                                       default_namespace, aws_account_id, params_json,
                                       environment, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(c.name.trim())
        .bind(c.source.as_str())
        .bind(&c.kubeconfig_path)
        .bind(&c.context_name)
        .bind(&c.default_namespace)
        .bind(&c.aws_account_id)
        .bind(params.to_string())
        .bind(c.environment.as_str())
        .bind(&c.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create k8s cluster"))?;
        self.get(&c.id).await
    }

    pub async fn get(&self, id: &Id) -> Result<K8sCluster> {
        let r = sqlx::query("SELECT * FROM k8s_clusters WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("k8s cluster"))?;
        row_to_cluster(&r)
    }

    /// Every cluster (the registry is a global library), name-sorted.
    pub async fn list(&self) -> Result<Vec<K8sCluster>> {
        let rows = sqlx::query("SELECT * FROM k8s_clusters ORDER BY name COLLATE NOCASE, id")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list k8s clusters"))?;
        rows.iter()
            .filter_map(|r| match row_to_cluster(r) {
                Ok(c) => Some(Ok(c)),
                Err(e) => {
                    tracing::warn!(id = %r.get::<String, _>("id"), "skipping unparseable k8s cluster row: {e}");
                    None
                }
            })
            .collect()
    }

    pub async fn update(&self, id: &Id, p: K8sClusterPatch) -> Result<K8sCluster> {
        let cur = self.get(id).await?;
        let name = p.name.map(|n| n.trim().to_string()).unwrap_or(cur.name);
        let kubeconfig_path = p.kubeconfig_path.unwrap_or(cur.kubeconfig_path);
        let context_name = p.context_name.unwrap_or(cur.context_name);
        let default_namespace = p.default_namespace.unwrap_or(cur.default_namespace);
        let environment = p.environment.unwrap_or(cur.environment);
        let color = match p.color {
            Some(c) => c,
            None => cur.color,
        };
        let params = params_with_color(cur.params, color.as_deref());
        sqlx::query(
            "UPDATE k8s_clusters SET name = ?, kubeconfig_path = ?, context_name = ?,
                    default_namespace = ?, environment = ?, params_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&kubeconfig_path)
        .bind(&context_name)
        .bind(&default_namespace)
        .bind(environment.as_str())
        .bind(params.to_string())
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update k8s cluster"))?;
        self.get(id).await
    }

    /// Replace the cached capability probe.
    pub async fn set_capabilities(&self, id: &Id, caps: &Value) -> Result<()> {
        sqlx::query("UPDATE k8s_clusters SET capabilities_json = ? WHERE id = ?")
            .bind(caps.to_string())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set k8s capabilities"))?;
        Ok(())
    }

    /// Bump `last_used_at` (best-effort recency for the cluster cards).
    pub async fn touch(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE k8s_clusters SET last_used_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch k8s cluster"))?;
        Ok(())
    }

    /// Delete the row. Returns the deleted cluster so the caller can remove an
    /// Otto-owned kubeconfig file (`source.otto_owned_kubeconfig()`).
    pub async fn delete(&self, id: &Id) -> Result<K8sCluster> {
        let cur = self.get(id).await?;
        sqlx::query("DELETE FROM k8s_clusters WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete k8s cluster"))?;
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn new(id: &str, source: K8sClusterSource) -> NewK8sCluster {
        NewK8sCluster {
            id: id.into(),
            name: format!("cluster-{id}"),
            source,
            kubeconfig_path: Some("/tmp/kube.yaml".into()),
            context_name: "ctx".into(),
            default_namespace: Some("default".into()),
            aws_account_id: None,
            environment: Environment::Staging,
            color: Some("#f00".into()),
            params: serde_json::json!({"eks_region": "eu-west-1"}),
            created_by: None,
        }
    }

    #[tokio::test]
    async fn crud_roundtrip_keeps_color_inside_params() {
        let repo = K8sClustersRepo::new(pool().await);
        let c = repo.create(new("a", K8sClusterSource::Imported)).await.unwrap();
        assert_eq!(c.color.as_deref(), Some("#f00"));
        assert_eq!(c.params["eks_region"], "eu-west-1");
        assert_eq!(c.params["color"], "#f00");
        assert_eq!(c.environment, Environment::Staging);
        assert!(c.capabilities.is_none());
        assert!(c.source.otto_owned_kubeconfig());

        let up = repo
            .update(
                &c.id,
                K8sClusterPatch {
                    name: Some(" renamed ".into()),
                    color: Some(None),
                    default_namespace: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(up.name, "renamed");
        assert_eq!(up.color, None);
        assert!(up.params.get("color").is_none());
        assert_eq!(up.params["eks_region"], "eu-west-1");
        assert_eq!(up.default_namespace, None);
        assert_eq!(up.context_name, "ctx");

        repo.set_capabilities(&c.id, &serde_json::json!({"metrics_server": true}))
            .await
            .unwrap();
        repo.touch(&c.id).await.unwrap();
        let got = repo.get(&c.id).await.unwrap();
        assert_eq!(got.capabilities.unwrap()["metrics_server"], true);
        assert!(got.last_used_at.is_some());

        repo.create(new("b", K8sClusterSource::Kubeconfig)).await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 2);

        let deleted = repo.delete(&c.id).await.unwrap();
        assert_eq!(deleted.id, "a");
        assert!(matches!(repo.get(&c.id).await, Err(Error::NotFound(_))));
        assert_eq!(repo.list().await.unwrap().len(), 1);
    }

    #[test]
    fn source_parse_roundtrip() {
        for s in ["kubeconfig", "imported", "eks"] {
            assert_eq!(K8sClusterSource::parse(s).unwrap().as_str(), s);
        }
        assert!(K8sClusterSource::parse("nope").is_none());
        assert!(!K8sClusterSource::Kubeconfig.otto_owned_kubeconfig());
    }

    #[test]
    fn wire_shape_omits_empty_optionals() {
        let c = K8sCluster {
            id: "x".into(),
            name: "n".into(),
            source: K8sClusterSource::Kubeconfig,
            kubeconfig_path: None,
            context_name: "c".into(),
            default_namespace: None,
            aws_account_id: None,
            environment: Environment::Dev,
            color: None,
            params: serde_json::json!({}),
            capabilities: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["source"], "kubeconfig");
        assert_eq!(v["environment"], "dev");
        assert!(v.get("kubeconfig_path").is_none());
        assert!(v.get("capabilities").is_none());
        assert!(v["created_by"].is_null());
    }
}

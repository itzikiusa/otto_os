//! EKS — list/describe clusters + nodegroups (View), import-kubeconfig
//! (Edit + `kubernetes:Admin`) (§2.6). The import writes an Otto-owned
//! kubeconfig to `<data_dir>/kube/<new_id>.yaml` (0600) via
//! `aws eks update-kubeconfig` and inserts a `k8s_clusters` row
//! (`source='eks'`, `aws_account_id`) so the Kubernetes console can drive it
//! with the linked account's credentials.

use std::os::unix::fs::PermissionsExt;

use chrono::{DateTime, Utc};
use otto_core::domain::Environment;
use otto_core::{new_id, Error, Id, Result};
use otto_state::{AwsAccountRow, SqlitePool};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

/// `list-clusters` → describe fan-out cap (§2.6).
const DESCRIBE_CAP: usize = 20;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EksCluster {
    pub name: String,
    pub status: String,
    pub version: Option<String>,
    pub endpoint: Option<String>,
    pub arn: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClustersResp {
    pub clusters: Vec<EksCluster>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Nodegroup {
    pub name: String,
    pub status: String,
    pub desired: Option<u64>,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub instance_types: Vec<String>,
    pub ami_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterDetail {
    pub cluster: Value,
    pub nodegroups: Vec<Nodegroup>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImportReq {
    pub cluster_name_override: Option<String>,
    pub default_namespace: Option<String>,
}

/// `K8sCluster` DTO (§3.1) as returned by the import. Mirrors the shape the
/// Kubernetes console serves from `k8s_clusters`.
#[derive(Debug, Clone, Serialize)]
pub struct K8sCluster {
    pub id: Id,
    pub name: String,
    pub source: String,
    pub kubeconfig_path: Option<String>,
    pub context_name: String,
    pub default_namespace: Option<String>,
    pub aws_account_id: Option<Id>,
    pub environment: Environment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    pub created_by: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

pub fn cluster_names(v: &Value) -> Vec<String> {
    v.get("clusters")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `describe-cluster` → summary row.
pub fn normalize_cluster(v: &Value) -> Option<EksCluster> {
    let c = v.get("cluster")?;
    Some(EksCluster {
        name: s(c, "name")?,
        status: s(c, "status").unwrap_or_else(|| "UNKNOWN".into()),
        version: s(c, "version"),
        endpoint: s(c, "endpoint"),
        arn: s(c, "arn"),
        created_at: s(c, "createdAt"),
    })
}

/// `describe-nodegroup` → row.
pub fn normalize_nodegroup(v: &Value) -> Option<Nodegroup> {
    let n = v.get("nodegroup")?;
    let sc = n.get("scalingConfig").cloned().unwrap_or(Value::Null);
    Some(Nodegroup {
        name: s(n, "nodegroupName")?,
        status: s(n, "status").unwrap_or_else(|| "UNKNOWN".into()),
        desired: sc.get("desiredSize").and_then(|x| x.as_u64()),
        min: sc.get("minSize").and_then(|x| x.as_u64()),
        max: sc.get("maxSize").and_then(|x| x.as_u64()),
        instance_types: n
            .get("instanceTypes")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        ami_type: s(n, "amiType"),
    })
}

pub fn validate_cluster_name(n: &str) -> Result<()> {
    if n.is_empty()
        || n.len() > 100
        || !n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(Error::Invalid(format!("invalid EKS cluster name '{n}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn list_clusters(
    svc: &AwsService,
    a: &AwsAccountRow,
    region: Option<&str>,
) -> Result<ClustersResp> {
    let v = svc.run_json(a, region, &["eks", "list-clusters"]).await?;
    let mut clusters = Vec::new();
    for name in cluster_names(&v).into_iter().take(DESCRIBE_CAP) {
        match svc
            .run_json(a, region, &["eks", "describe-cluster", "--name", &name])
            .await
        {
            Ok(d) => {
                if let Some(c) = normalize_cluster(&d) {
                    clusters.push(c);
                }
            }
            Err(e) => {
                tracing::warn!(cluster = %name, "eks describe-cluster failed: {e}");
                clusters.push(EksCluster {
                    name,
                    status: "UNKNOWN".into(),
                    version: None,
                    endpoint: None,
                    arn: None,
                    created_at: None,
                });
            }
        }
    }
    Ok(ClustersResp { clusters })
}

pub async fn describe_cluster(
    svc: &AwsService,
    a: &AwsAccountRow,
    name: &str,
    region: Option<&str>,
) -> Result<ClusterDetail> {
    validate_cluster_name(name)?;
    let d = svc
        .run_json(a, region, &["eks", "describe-cluster", "--name", name])
        .await?;
    let cluster = d
        .get("cluster")
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("eks cluster {name}")))?;
    let ng = svc
        .run_json(
            a,
            region,
            &["eks", "list-nodegroups", "--cluster-name", name],
        )
        .await?;
    let names: Vec<String> = ng
        .get("nodegroups")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut nodegroups = Vec::new();
    for g in names.iter().take(DESCRIBE_CAP) {
        if let Ok(v) = svc
            .run_json(
                a,
                region,
                &[
                    "eks",
                    "describe-nodegroup",
                    "--cluster-name",
                    name,
                    "--nodegroup-name",
                    g,
                ],
            )
            .await
        {
            if let Some(n) = normalize_nodegroup(&v) {
                nodegroups.push(n);
            }
        }
    }
    Ok(ClusterDetail {
        cluster,
        nodegroups,
    })
}

/// `aws eks update-kubeconfig` into an Otto-owned file + a `k8s_clusters`
/// row. `pool` is the daemon DB (the row insert is a small local statement
/// matching migration 0114).
// TODO(k8s): switch the insert to `otto_state::K8sClustersRepo` once it lands
// (it is being built in parallel by the Kubernetes console work).
pub async fn import_kubeconfig(
    svc: &AwsService,
    pool: &SqlitePool,
    a: &AwsAccountRow,
    name: &str,
    req: &ImportReq,
    region: Option<&str>,
    creator: &Id,
) -> Result<K8sCluster> {
    validate_cluster_name(name)?;
    let region_str = crate::accounts::resolve_region(a, region)?.to_string();
    let alias = req
        .cluster_name_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
        .to_string();
    if alias.len() > 200 || alias.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(Error::Invalid(
            "cluster_name_override must be a single token".into(),
        ));
    }
    let default_namespace = req
        .default_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let id = new_id();
    let kube_dir = svc.data_dir.join("kube");
    std::fs::create_dir_all(&kube_dir)
        .map_err(|e| Error::Internal(format!("create {}: {e}", kube_dir.display())))?;
    let _ = std::fs::set_permissions(&kube_dir, std::fs::Permissions::from_mode(0o700));
    let path = kube_dir.join(format!("{id}.yaml"));
    let path_s = path.to_string_lossy().into_owned();

    let res = svc
        .run(
            a,
            Some(&region_str),
            &[
                "eks",
                "update-kubeconfig",
                "--name",
                name,
                "--kubeconfig",
                &path_s,
                "--alias",
                &alias,
                "--region",
                &region_str,
            ],
        )
        .await;
    if let Err(e) = res {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    if !path.is_file() {
        return Err(Error::Upstream(
            "aws eks update-kubeconfig produced no kubeconfig file".into(),
        ));
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| Error::Internal(format!("chmod kubeconfig: {e}")))?;

    let now = Utc::now();
    let params = serde_json::json!({ "eks_region": region_str, "eks_cluster": name });
    let env = a.environment;
    sqlx::query(
        "INSERT INTO k8s_clusters (id, name, source, kubeconfig_path, context_name, default_namespace,
                                   aws_account_id, params_json, environment, created_by, created_at, updated_at)
         VALUES (?, ?, 'eks', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&alias)
    .bind(&path_s)
    .bind(&alias)
    .bind(&default_namespace)
    .bind(&a.id)
    .bind(params.to_string())
    .bind(env.as_str())
    .bind(creator)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| {
        let _ = std::fs::remove_file(&path);
        Error::Internal(format!("insert k8s cluster: {e}"))
    })?;

    Ok(K8sCluster {
        id,
        name: alias.clone(),
        source: "eks".into(),
        kubeconfig_path: Some(path_s),
        context_name: alias,
        default_namespace,
        aws_account_id: Some(a.id.clone()),
        environment: env,
        color: None,
        capabilities: None,
        created_by: Some(creator.clone()),
        created_at: now,
        updated_at: now,
        last_used_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_and_nodegroup_normalize() {
        let l: Value = serde_json::json!({"clusters": ["prod-eu", "staging"]});
        assert_eq!(cluster_names(&l), vec!["prod-eu", "staging"]);
        let d: Value = serde_json::from_str(
            r#"{"cluster": {"name": "prod-eu", "arn": "arn:aws:eks:eu-west-1:123456789012:cluster/prod-eu", "createdAt": "2023-02-01T10:00:00+00:00", "version": "1.29", "endpoint": "https://ABC.gr7.eu-west-1.eks.amazonaws.com", "status": "ACTIVE", "certificateAuthority": {"data": "LS0t"}}}"#,
        )
        .unwrap();
        let c = normalize_cluster(&d).unwrap();
        assert_eq!(c.name, "prod-eu");
        assert_eq!(c.version.as_deref(), Some("1.29"));
        assert_eq!(c.status, "ACTIVE");
        let n: Value = serde_json::from_str(
            r#"{"nodegroup": {"nodegroupName": "workers", "status": "ACTIVE", "scalingConfig": {"minSize": 1, "maxSize": 5, "desiredSize": 3}, "instanceTypes": ["m5.large"], "amiType": "AL2_x86_64"}}"#,
        )
        .unwrap();
        let g = normalize_nodegroup(&n).unwrap();
        assert_eq!(g.name, "workers");
        assert_eq!((g.min, g.max, g.desired), (Some(1), Some(5), Some(3)));
        assert_eq!(g.instance_types, vec!["m5.large"]);
        assert_eq!(g.ami_type.as_deref(), Some("AL2_x86_64"));
    }

    #[test]
    fn cluster_name_validation() {
        assert!(validate_cluster_name("prod-eu_1").is_ok());
        assert!(validate_cluster_name("bad name").is_err());
        assert!(validate_cluster_name("").is_err());
    }
}

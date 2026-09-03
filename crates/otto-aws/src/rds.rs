//! RDS — list/describe DB instances (View). Read-only by design: the console
//! has no start/stop/reboot for databases; CloudWatch metrics live in
//! `metrics.rs` (`AWS/RDS` · `DBInstanceIdentifier`).

use std::collections::BTreeMap;

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

/// `RdsInstance` DTO.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RdsInstance {
    pub identifier: String,
    pub engine: Option<String>,
    pub engine_version: Option<String>,
    pub class: Option<String>,
    pub status: String,
    pub az: Option<String>,
    pub multi_az: bool,
    pub storage_gb: Option<u64>,
    pub storage_type: Option<String>,
    pub endpoint: Option<String>,
    pub port: Option<u16>,
    pub db_name: Option<String>,
    pub master_username: Option<String>,
    pub publicly_accessible: bool,
    pub created: Option<String>,
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstancesResp {
    pub instances: Vec<RdsInstance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceDetail {
    #[serde(flatten)]
    pub instance: RdsInstance,
    pub raw: Value,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InstancesQuery {
    pub region: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

pub fn normalize_instance(i: &Value) -> Option<RdsInstance> {
    let tags: BTreeMap<String, String> = i
        .get("TagList")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| Some((s(t, "Key")?, s(t, "Value").unwrap_or_default())))
                .collect()
        })
        .unwrap_or_default();
    let ep = i.get("Endpoint");
    Some(RdsInstance {
        identifier: s(i, "DBInstanceIdentifier")?,
        engine: s(i, "Engine"),
        engine_version: s(i, "EngineVersion"),
        class: s(i, "DBInstanceClass"),
        status: s(i, "DBInstanceStatus").unwrap_or_else(|| "unknown".into()),
        az: s(i, "AvailabilityZone"),
        multi_az: i.get("MultiAZ").and_then(|m| m.as_bool()).unwrap_or(false),
        storage_gb: i.get("AllocatedStorage").and_then(|a| a.as_u64()),
        storage_type: s(i, "StorageType"),
        endpoint: ep.and_then(|e| s(e, "Address")),
        port: ep
            .and_then(|e| e.get("Port"))
            .and_then(|p| p.as_u64())
            .and_then(|p| u16::try_from(p).ok()),
        db_name: s(i, "DBName"),
        master_username: s(i, "MasterUsername"),
        publicly_accessible: i
            .get("PubliclyAccessible")
            .and_then(|p| p.as_bool())
            .unwrap_or(false),
        created: s(i, "InstanceCreateTime"),
        tags,
    })
}

pub fn normalize_instances(v: &Value) -> Vec<RdsInstance> {
    v.get("DBInstances")
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().filter_map(normalize_instance).collect())
        .unwrap_or_default()
}

/// Client-side free-text filter over identifier / engine / class / endpoint.
pub fn matches_query(i: &RdsInstance, q: &str) -> bool {
    let q = q.to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    [
        Some(&i.identifier),
        i.engine.as_ref(),
        i.class.as_ref(),
        i.endpoint.as_ref(),
        i.db_name.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|f| f.to_ascii_lowercase().contains(&q))
}

/// RDS identifiers: 1–63 chars, letters / digits / hyphens, starting with a
/// letter (the console lower-cases them).
pub fn validate_identifier(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 63
        && id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !ok {
        return Err(Error::Invalid(format!(
            "invalid DB instance identifier '{id}'"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn list_instances(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &InstancesQuery,
) -> Result<InstancesResp> {
    let v = svc
        .run_json(a, q.region.as_deref(), &["rds", "describe-db-instances"])
        .await?;
    let mut instances = normalize_instances(&v);
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        instances.retain(|i| matches_query(i, text));
    }
    Ok(InstancesResp { instances })
}

pub async fn describe_instance(
    svc: &AwsService,
    a: &AwsAccountRow,
    id: &str,
    region: Option<&str>,
) -> Result<InstanceDetail> {
    validate_identifier(id)?;
    let v = svc
        .run_json(
            a,
            region,
            &[
                "rds",
                "describe-db-instances",
                "--db-instance-identifier",
                id,
            ],
        )
        .await?;
    let raw = v
        .get("DBInstances")
        .and_then(|d| d.as_array())
        .and_then(|d| d.first())
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("db instance {id}")))?;
    let instance = normalize_instance(&raw)
        .ok_or_else(|| Error::Upstream("describe-db-instances: malformed instance".into()))?;
    Ok(InstanceDetail { instance, raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIBE: &str = r#"{
      "DBInstances": [
        {"DBInstanceIdentifier": "orders-db", "DBInstanceClass": "db.r6g.large", "Engine": "postgres",
         "DBInstanceStatus": "available", "MasterUsername": "app", "DBName": "orders",
         "Endpoint": {"Address": "orders-db.abc.eu-west-1.rds.amazonaws.com", "Port": 5432, "HostedZoneId": "Z1"},
         "AllocatedStorage": 100, "InstanceCreateTime": "2024-01-10T08:00:00+00:00",
         "AvailabilityZone": "eu-west-1a", "MultiAZ": true, "EngineVersion": "15.4",
         "PubliclyAccessible": false, "StorageType": "gp3",
         "TagList": [{"Key": "env", "Value": "prod"}, {"Key": "team", "Value": "payments"}]},
        {"DBInstanceIdentifier": "scratch", "Engine": "mysql", "DBInstanceStatus": "stopped"}
      ]
    }"#;

    #[test]
    fn instances_normalize() {
        let v: Value = serde_json::from_str(DESCRIBE).unwrap();
        let list = normalize_instances(&v);
        assert_eq!(list.len(), 2);
        let a = &list[0];
        assert_eq!(a.identifier, "orders-db");
        assert_eq!(a.engine.as_deref(), Some("postgres"));
        assert_eq!(a.engine_version.as_deref(), Some("15.4"));
        assert_eq!(a.class.as_deref(), Some("db.r6g.large"));
        assert_eq!(a.status, "available");
        assert_eq!(a.az.as_deref(), Some("eu-west-1a"));
        assert!(a.multi_az);
        assert_eq!(a.storage_gb, Some(100));
        assert_eq!(a.storage_type.as_deref(), Some("gp3"));
        assert_eq!(
            a.endpoint.as_deref(),
            Some("orders-db.abc.eu-west-1.rds.amazonaws.com")
        );
        assert_eq!(a.port, Some(5432));
        assert_eq!(a.db_name.as_deref(), Some("orders"));
        assert!(!a.publicly_accessible);
        assert_eq!(a.tags["team"], "payments");
        let b = &list[1];
        assert_eq!(b.status, "stopped");
        assert!(b.endpoint.is_none() && b.port.is_none() && b.storage_gb.is_none());
        assert!(!b.multi_az);
        assert!(normalize_instances(&Value::Null).is_empty());
    }

    #[test]
    fn query_filter_and_identifier_validation() {
        let v: Value = serde_json::from_str(DESCRIBE).unwrap();
        let list = normalize_instances(&v);
        assert!(matches_query(&list[0], "POSTGRES"));
        assert!(matches_query(&list[0], "r6g"));
        assert!(matches_query(&list[0], "rds.amazonaws"));
        assert!(!matches_query(&list[1], "orders"));
        assert!(validate_identifier("orders-db").is_ok());
        assert!(validate_identifier("a").is_ok());
        assert!(validate_identifier("1abc").is_err());
        assert!(validate_identifier("bad_name").is_err());
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier(&"x".repeat(64)).is_err());
    }
}

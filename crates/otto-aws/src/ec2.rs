//! EC2 — list/describe (View), start/stop/reboot (Edit) (§2.4).

use std::collections::BTreeMap;

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

/// `Ec2Instance` DTO.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Ec2Instance {
    pub instance_id: String,
    pub name: Option<String>,
    pub state: String,
    #[serde(rename = "type")]
    pub instance_type: Option<String>,
    pub az: Option<String>,
    pub private_ip: Option<String>,
    pub public_ip: Option<String>,
    pub launch_time: Option<String>,
    pub platform: Option<String>,
    pub vpc_id: Option<String>,
    pub subnet_id: Option<String>,
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstancesResp {
    pub instances: Vec<Ec2Instance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceDetail {
    #[serde(flatten)]
    pub instance: Ec2Instance,
    pub raw: Value,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InstancesQuery {
    pub region: Option<String>,
    pub state: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfirmReq {
    pub confirm_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StateChangeResp {
    pub previous_state: String,
    pub current_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Start,
    Stop,
    Reboot,
}

impl PowerAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "reboot" => Some(Self::Reboot),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Reboot => "reboot",
        }
    }
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

pub fn normalize_instance(i: &Value) -> Option<Ec2Instance> {
    let tags: BTreeMap<String, String> = i
        .get("Tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| Some((s(t, "Key")?, s(t, "Value").unwrap_or_default())))
                .collect()
        })
        .unwrap_or_default();
    Some(Ec2Instance {
        instance_id: s(i, "InstanceId")?,
        name: tags.get("Name").cloned(),
        state: i
            .get("State")
            .and_then(|st| st.get("Name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string(),
        instance_type: s(i, "InstanceType"),
        az: i
            .get("Placement")
            .and_then(|p| p.get("AvailabilityZone"))
            .and_then(|z| z.as_str())
            .map(str::to_string),
        private_ip: s(i, "PrivateIpAddress"),
        public_ip: s(i, "PublicIpAddress"),
        launch_time: s(i, "LaunchTime"),
        platform: s(i, "PlatformDetails").or_else(|| s(i, "Platform")),
        vpc_id: s(i, "VpcId"),
        subnet_id: s(i, "SubnetId"),
        tags,
    })
}

/// Flatten `Reservations[].Instances[]`.
pub fn normalize_instances(v: &Value) -> Vec<Ec2Instance> {
    v.get("Reservations")
        .and_then(|r| r.as_array())
        .map(|res| {
            res.iter()
                .filter_map(|r| r.get("Instances").and_then(|i| i.as_array()))
                .flatten()
                .filter_map(normalize_instance)
                .collect()
        })
        .unwrap_or_default()
}

/// Client-side free-text filter over id / name / ips / type.
pub fn matches_query(i: &Ec2Instance, q: &str) -> bool {
    let q = q.to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    [
        Some(&i.instance_id),
        i.name.as_ref(),
        i.private_ip.as_ref(),
        i.public_ip.as_ref(),
        i.instance_type.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|f| f.to_ascii_lowercase().contains(&q))
}

/// `start-instances` / `stop-instances` → `{previous_state, current_state}`.
pub fn normalize_state_change(v: &Value, key: &str) -> Option<StateChangeResp> {
    let first = v.get(key)?.as_array()?.first()?;
    let name = |k: &str| first.get(k)?.get("Name")?.as_str().map(str::to_string);
    Some(StateChangeResp {
        previous_state: name("PreviousState")?,
        current_state: name("CurrentState")?,
    })
}

pub fn validate_instance_id(id: &str) -> Result<()> {
    let ok = id.starts_with("i-")
        && id.len() >= 10
        && id.len() <= 19
        && id[2..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        return Err(Error::Invalid(format!("invalid instance id '{id}'")));
    }
    Ok(())
}

const STATES: &[&str] = &[
    "pending",
    "running",
    "shutting-down",
    "terminated",
    "stopping",
    "stopped",
];

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn list_instances(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &InstancesQuery,
) -> Result<InstancesResp> {
    let mut args = vec!["ec2", "describe-instances"];
    let filter;
    if let Some(st) = q.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !STATES.contains(&st) {
            return Err(Error::Invalid(format!("unknown instance state '{st}'")));
        }
        filter = format!("Name=instance-state-name,Values={st}");
        args.extend(["--filters", filter.as_str()]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
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
    validate_instance_id(id)?;
    let v = svc
        .run_json(
            a,
            region,
            &["ec2", "describe-instances", "--instance-ids", id],
        )
        .await?;
    let raw = v
        .get("Reservations")
        .and_then(|r| r.as_array())
        .and_then(|r| r.first())
        .and_then(|r| r.get("Instances"))
        .and_then(|i| i.as_array())
        .and_then(|i| i.first())
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("instance {id}")))?;
    let instance = normalize_instance(&raw)
        .ok_or_else(|| Error::Upstream("describe-instances: malformed instance".into()))?;
    Ok(InstanceDetail { instance, raw })
}

pub async fn power(
    svc: &AwsService,
    a: &AwsAccountRow,
    id: &str,
    action: PowerAction,
    confirm_id: Option<&str>,
    region: Option<&str>,
) -> Result<StateChangeResp> {
    validate_instance_id(id)?;
    if matches!(action, PowerAction::Stop | PowerAction::Reboot) && confirm_id != Some(id) {
        return Err(Error::Invalid(format!(
            "confirm_id must equal the instance id '{id}'"
        )));
    }
    match action {
        PowerAction::Start => {
            let v = svc
                .run_json(a, region, &["ec2", "start-instances", "--instance-ids", id])
                .await?;
            normalize_state_change(&v, "StartingInstances")
                .ok_or_else(|| Error::Upstream("start-instances returned no state change".into()))
        }
        PowerAction::Stop => {
            let v = svc
                .run_json(a, region, &["ec2", "stop-instances", "--instance-ids", id])
                .await?;
            normalize_state_change(&v, "StoppingInstances")
                .ok_or_else(|| Error::Upstream("stop-instances returned no state change".into()))
        }
        PowerAction::Reboot => {
            // reboot-instances returns nothing; report the observed state.
            let before = describe_instance(svc, a, id, region).await?.instance.state;
            svc.run(
                a,
                region,
                &["ec2", "reboot-instances", "--instance-ids", id],
            )
            .await?;
            Ok(StateChangeResp {
                previous_state: before.clone(),
                current_state: before,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIBE: &str = r#"{
      "Reservations": [
        {"ReservationId": "r-1", "Instances": [
          {"InstanceId": "i-0abc123456789def0", "InstanceType": "t3.medium",
           "State": {"Code": 16, "Name": "running"},
           "Placement": {"AvailabilityZone": "eu-west-1a"},
           "PrivateIpAddress": "10.0.1.5", "PublicIpAddress": "54.1.2.3",
           "LaunchTime": "2024-06-01T10:00:00+00:00", "PlatformDetails": "Linux/UNIX",
           "VpcId": "vpc-1", "SubnetId": "subnet-1",
           "Tags": [{"Key": "Name", "Value": "api-1"}, {"Key": "env", "Value": "prod"}]}
        ]},
        {"ReservationId": "r-2", "Instances": [
          {"InstanceId": "i-0fedcba9876543210", "InstanceType": "m5.large",
           "State": {"Code": 80, "Name": "stopped"}, "Placement": {"AvailabilityZone": "eu-west-1b"},
           "PrivateIpAddress": "10.0.2.9", "LaunchTime": "2024-01-01T00:00:00+00:00", "Platform": "windows"}
        ]}
      ]
    }"#;

    #[test]
    fn instances_normalize() {
        let v: Value = serde_json::from_str(DESCRIBE).unwrap();
        let list = normalize_instances(&v);
        assert_eq!(list.len(), 2);
        let a = &list[0];
        assert_eq!(a.instance_id, "i-0abc123456789def0");
        assert_eq!(a.name.as_deref(), Some("api-1"));
        assert_eq!(a.state, "running");
        assert_eq!(a.instance_type.as_deref(), Some("t3.medium"));
        assert_eq!(a.az.as_deref(), Some("eu-west-1a"));
        assert_eq!(a.public_ip.as_deref(), Some("54.1.2.3"));
        assert_eq!(a.platform.as_deref(), Some("Linux/UNIX"));
        assert_eq!(a.tags["env"], "prod");
        let b = &list[1];
        assert!(b.name.is_none() && b.public_ip.is_none());
        assert_eq!(b.platform.as_deref(), Some("windows"));
        // Serialized key is `type` per the contract.
        let j = serde_json::to_value(a).unwrap();
        assert_eq!(j["type"], "t3.medium");
        assert!(j.get("instance_type").is_none());
    }

    #[test]
    fn query_filter() {
        let v: Value = serde_json::from_str(DESCRIBE).unwrap();
        let list = normalize_instances(&v);
        assert!(matches_query(&list[0], "API"));
        assert!(matches_query(&list[0], "10.0.1"));
        assert!(matches_query(&list[1], "m5"));
        assert!(!matches_query(&list[1], "api"));
    }

    #[test]
    fn state_change_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"StoppingInstances": [{"CurrentState": {"Code": 64, "Name": "stopping"}, "InstanceId": "i-0abc123456789def0", "PreviousState": {"Code": 16, "Name": "running"}}]}"#,
        )
        .unwrap();
        let sc = normalize_state_change(&v, "StoppingInstances").unwrap();
        assert_eq!(sc.previous_state, "running");
        assert_eq!(sc.current_state, "stopping");
        assert!(normalize_state_change(&v, "StartingInstances").is_none());
    }

    #[test]
    fn instance_id_validation() {
        assert!(validate_instance_id("i-0abc123456789def0").is_ok());
        assert!(validate_instance_id("i-12345678").is_ok());
        assert!(validate_instance_id("vol-123").is_err());
        assert!(validate_instance_id("i-zzzz").is_err());
    }
}

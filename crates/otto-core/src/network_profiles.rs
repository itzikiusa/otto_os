//! Session-scoped SSH forwarding profiles and their non-secret endpoint contract.
use crate::{Error, Id, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkEndpoint {
    pub name: String,
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(default)]
    pub host_env: Option<String>,
    #[serde(default)]
    pub port_env: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProfileInput {
    pub name: String,
    pub ssh_connection_id: Id,
    pub endpoints: Vec<NetworkEndpoint>,
    #[serde(default)]
    pub archived: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkProfile {
    pub id: Id,
    pub workspace_id: Id,
    #[serde(flatten)]
    pub input: NetworkProfileInput,
    pub version: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkLocalEndpoint {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub host_env: Option<String>,
    pub port_env: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNetworkStatus {
    pub profile_id: Option<Id>,
    pub profile_name: Option<String>,
    pub profile_version: Option<i64>,
    /// connected, error, stopped, or disabled; never a claim of DB reachability.
    pub status: String,
    pub error: Option<String>,
    pub endpoints: Vec<NetworkLocalEndpoint>,
}
impl Default for SessionNetworkStatus {
    fn default() -> Self {
        Self {
            profile_id: None,
            profile_name: None,
            profile_version: None,
            status: "disabled".into(),
            error: None,
            endpoints: vec![],
        }
    }
}

pub fn valid_host(host: &str) -> bool {
    if let Some(ip) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return ip.parse::<std::net::Ipv6Addr>().is_ok();
    }
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn mapping(name: &str, host: bool) -> bool {
    let suffix = if host { "_HOST" } else { "_PORT" };
    let pg = if host { "PGHOST" } else { "PGPORT" };
    name.len() <= 64
        && (name.ends_with(suffix) || name == pg)
        && !name.starts_with("OTTO_")
        && !name.starts_with("LD_")
        && !name.starts_with("DYLD_")
        && name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
impl NetworkProfileInput {
    pub fn validate(&self) -> Result<()> {
        let bad = |message: &str| Err(Error::Invalid(message.into()));
        if self.name.trim().is_empty() || self.name.len() > 200 {
            return bad("network profile name must be 1–200 bytes");
        }
        if self.ssh_connection_id.trim().is_empty() {
            return bad("select an SSH connection");
        }
        if self.endpoints.is_empty() || self.endpoints.len() > 8 {
            return bad("a network profile needs 1–8 endpoints");
        }
        let mut names = HashSet::new();
        let mut envs = HashSet::new();
        for endpoint in &self.endpoints {
            if !identifier(&endpoint.name) || !names.insert(endpoint.name.to_ascii_uppercase()) {
                return bad("endpoint names must be unique identifiers of 1–32 characters");
            }
            if !valid_host(&endpoint.remote_host) || endpoint.remote_port == 0 {
                return bad("endpoint needs a valid DNS/IP host and nonzero port");
            }
            for (name, host) in [(&endpoint.host_env, true), (&endpoint.port_env, false)] {
                if let Some(name) = name {
                    if !mapping(name, host) || !envs.insert(name.clone()) {
                        return bad("environment mappings must be unique *_HOST/*_PORT names (or PGHOST/PGPORT), outside reserved prefixes");
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> NetworkProfileInput {
        serde_json::from_value(serde_json::json!({"name":"Office", "ssh_connection_id":"ssh", "endpoints":[
            {"name":"database","remote_host":"db.internal","remote_port":5432,"host_env":"PGHOST","port_env":"PGPORT"}
        ]})).unwrap()
    }
    #[test]
    fn accepts_explicit_tcp_endpoints_and_rejects_unsafe_mapping() {
        let mut profile = input();
        assert!(profile.validate().is_ok());
        profile.endpoints[0].host_env = Some("PATH".into());
        assert!(profile.validate().is_err());
        profile.endpoints[0].host_env = Some("DB_HOST".into());
        profile.endpoints[0].remote_host = "db:5432:other".into();
        assert!(profile.validate().is_err());
        profile.endpoints[0].remote_host = "[2001:db8::1]".into();
        assert!(profile.validate().is_ok());
    }
    #[test]
    fn rejects_duplicate_names_envs_and_zero_ports() {
        let mut profile = input();
        profile.endpoints.push(profile.endpoints[0].clone());
        profile.endpoints[1].name = "DATABASE".into();
        assert!(profile.validate().is_err());
        profile.endpoints[1].name = "other".into();
        assert!(profile.validate().is_err());
        profile.endpoints.pop();
        profile.endpoints[0].remote_port = 0;
        assert!(profile.validate().is_err());
    }
}

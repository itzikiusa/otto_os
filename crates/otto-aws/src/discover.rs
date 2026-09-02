//! Profile discovery: parse `~/.aws/config` + `~/.aws/credentials` for
//! profile **names and metadata only**. Key values (`aws_access_key_id`,
//! `aws_secret_access_key`, `aws_session_token`) are never read into the
//! result. Hand-rolled ini parsing — the format is small and stable, and this
//! crate must not pull a parser dependency for it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// `DiscoveredProfile` DTO (§2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sso_start_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sso_session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    /// `config` | `credentials`.
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoverResp {
    pub profiles: Vec<DiscoveredProfile>,
}

/// Parsed ini: section name → (key → value). Keys are lower-cased; nested
/// `key =\n  sub = v` blocks (CLI v2 `s3 =` style) are flattened as
/// `key.sub`. Comments (`#`, `;`) and blank lines are skipped.
pub fn parse_ini(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section: Option<String> = None;
    let mut nested_key: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let name = trimmed[1..trimmed.len() - 1].trim().to_string();
            out.entry(name.clone()).or_default();
            section = Some(name);
            nested_key = None;
            continue;
        }
        let Some(sec) = section.as_ref() else {
            continue;
        };
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().to_string();
        let entry = out.entry(sec.clone()).or_default();
        if indented {
            if let Some(parent) = &nested_key {
                entry.insert(format!("{parent}.{k}"), v);
                continue;
            }
        }
        if v.is_empty() {
            nested_key = Some(k.clone());
        } else {
            nested_key = None;
        }
        entry.insert(k, v);
    }
    out
}

/// Section name → profile name for `~/.aws/config` (`[profile x]` → `x`,
/// `[default]` → `default`; `[sso-session x]` / `[services x]` are not
/// profiles ⇒ `None`).
fn config_profile_name(section: &str) -> Option<String> {
    if section == "default" {
        return Some("default".into());
    }
    section
        .strip_prefix("profile ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Merge the two files. Config wins on metadata; credentials-only profiles
/// are added with `source: "credentials"`. Never copies key material.
pub fn discover_from(config: Option<&str>, credentials: Option<&str>) -> Vec<DiscoveredProfile> {
    let mut by_name: BTreeMap<String, DiscoveredProfile> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    if let Some(cfg) = config {
        let ini = parse_ini(cfg);
        // sso-session blocks carry the start URL for v2-style SSO profiles.
        let sso_sessions: BTreeMap<String, String> = ini
            .iter()
            .filter_map(|(sec, kv)| {
                let name = sec.strip_prefix("sso-session ")?.trim().to_string();
                let url = kv.get("sso_start_url")?.clone();
                Some((name, url))
            })
            .collect();
        for (sec, kv) in &ini {
            let Some(name) = config_profile_name(sec) else {
                continue;
            };
            let sso_session = kv.get("sso_session").cloned();
            let sso_start_url = kv.get("sso_start_url").cloned().or_else(|| {
                sso_session
                    .as_ref()
                    .and_then(|s| sso_sessions.get(s).cloned())
            });
            let p = DiscoveredProfile {
                name: name.clone(),
                region: kv.get("region").cloned(),
                sso_start_url,
                sso_session,
                role_arn: kv.get("role_arn").cloned(),
                source: "config",
            };
            if by_name.insert(name.clone(), p).is_none() {
                order.push(name);
            }
        }
    }
    if let Some(creds) = credentials {
        let ini = parse_ini(creds);
        for (sec, kv) in &ini {
            let name = sec.trim().to_string();
            if name.is_empty() {
                continue;
            }
            if let Some(existing) = by_name.get_mut(&name) {
                if existing.region.is_none() {
                    existing.region = kv.get("region").cloned();
                }
                continue;
            }
            by_name.insert(
                name.clone(),
                DiscoveredProfile {
                    name: name.clone(),
                    region: kv.get("region").cloned(),
                    sso_start_url: None,
                    sso_session: None,
                    role_arn: None,
                    source: "credentials",
                },
            );
            order.push(name);
        }
    }
    order
        .into_iter()
        .filter_map(|n| by_name.remove(&n))
        .collect()
}

/// `$AWS_CONFIG_FILE` / `$AWS_SHARED_CREDENTIALS_FILE` or the `~/.aws` defaults.
pub fn default_paths() -> (PathBuf, PathBuf) {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cfg = std::env::var("AWS_CONFIG_FILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".aws/config"));
    let creds = std::env::var("AWS_SHARED_CREDENTIALS_FILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".aws/credentials"));
    (cfg, creds)
}

/// Read-only discovery from disk. Missing files are simply empty.
pub fn discover_files(config: &Path, credentials: &Path) -> Vec<DiscoveredProfile> {
    let cfg = std::fs::read_to_string(config).ok();
    let creds = std::fs::read_to_string(credentials).ok();
    discover_from(cfg.as_deref(), creds.as_deref())
}

pub fn discover() -> Vec<DiscoveredProfile> {
    let (c, k) = default_paths();
    discover_files(&c, &k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
# comment
[default]
region = us-east-1
output = json

[profile dev-sso]
sso_session = corp
sso_account_id = 111122223333
sso_role_name = Developer
region = eu-west-1

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = us-east-1
sso_registration_scopes = sso:account:access

[profile legacy-sso]
sso_start_url = https://legacy.awsapps.com/start
sso_region = us-east-1

[profile admin]
role_arn = arn:aws:iam::123456789012:role/Admin
source_profile = default
s3 =
  max_concurrent_requests = 20
  addressing_style = path
region = ap-southeast-2

[services my-services]
s3 =
  endpoint_url = http://localhost:4566
"#;

    const CREDENTIALS: &str = r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY

[keys-only]
aws_access_key_id = AKIAI44QH8DHBEXAMPLE
aws_secret_access_key = je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY
aws_session_token = FQoGZXIvYXdzEBYaDDDDDDDDDDDDDDDD
region = us-west-2
"#;

    #[test]
    fn parses_sections_keys_and_nested_blocks() {
        let ini = parse_ini(CONFIG);
        assert_eq!(ini["default"]["region"], "us-east-1");
        assert_eq!(ini["profile admin"]["s3.max_concurrent_requests"], "20");
        assert_eq!(ini["profile admin"]["region"], "ap-southeast-2");
        assert_eq!(
            ini["sso-session corp"]["sso_start_url"],
            "https://corp.awsapps.com/start"
        );
    }

    #[test]
    fn discovers_profiles_with_metadata_and_never_secrets() {
        let profiles = discover_from(Some(CONFIG), Some(CREDENTIALS));
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["default", "admin", "dev-sso", "legacy-sso", "keys-only"]
        );

        let dev = profiles.iter().find(|p| p.name == "dev-sso").unwrap();
        assert_eq!(dev.sso_session.as_deref(), Some("corp"));
        // Resolved through the sso-session block.
        assert_eq!(
            dev.sso_start_url.as_deref(),
            Some("https://corp.awsapps.com/start")
        );
        assert_eq!(dev.region.as_deref(), Some("eu-west-1"));
        assert_eq!(dev.source, "config");

        let legacy = profiles.iter().find(|p| p.name == "legacy-sso").unwrap();
        assert_eq!(
            legacy.sso_start_url.as_deref(),
            Some("https://legacy.awsapps.com/start")
        );
        assert!(legacy.sso_session.is_none());

        let admin = profiles.iter().find(|p| p.name == "admin").unwrap();
        assert_eq!(
            admin.role_arn.as_deref(),
            Some("arn:aws:iam::123456789012:role/Admin")
        );

        let keys = profiles.iter().find(|p| p.name == "keys-only").unwrap();
        assert_eq!(keys.source, "credentials");
        assert_eq!(keys.region.as_deref(), Some("us-west-2"));

        // `[services …]` and `[sso-session …]` are not profiles.
        assert!(!names.contains(&"my-services") && !names.contains(&"corp"));

        // Nothing secret leaks through serialization.
        let json = serde_json::to_string(&profiles).unwrap();
        assert!(!json.contains("AKIA"));
        assert!(!json.contains("wJalrXUtnFEMI"));
        assert!(!json.contains("FQoGZXIvYXdz"));
    }

    #[test]
    fn missing_files_yield_empty() {
        assert!(discover_from(None, None).is_empty());
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_files(&tmp.path().join("nope"), &tmp.path().join("nope2")).is_empty());
    }
}

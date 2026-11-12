//! The AlterConfigs request for a topic config edit. rdkafka 0.39 exposes only
//! the legacy full-replace `alter_configs` (every override not in the request
//! is reset to the broker default), so the request carries the topic's current
//! overrides plus the edited keys. Building it refuses — instead of sending a
//! request that would silently wipe overrides — whenever the current overrides
//! cannot be read in full: the describe failed for the topic (the old code
//! skipped a per-resource error and sent the edited key alone), or an override
//! is sensitive and its value is withheld by the broker.

use crate::types::ConfigKv;
use otto_core::{Error, Result};
use rdkafka::admin::{ConfigResourceResult, ConfigSource};
use std::collections::BTreeMap;

/// Every key the full-replace request must carry: the topic's dynamic
/// overrides with `edits` applied on top. Sorted by key (deterministic).
pub(super) fn merged_topic_overrides(
    topic: &str,
    current: Vec<ConfigResourceResult>,
    edits: &[ConfigKv],
) -> Result<Vec<(String, String)>> {
    let edited = |name: &str| edits.iter().any(|kv| kv.name == name);
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    let mut described = false;
    for res in current {
        let cr = res.map_err(|code| {
            Error::Upstream(format!(
                "could not read the current config of {topic} ({}); nothing was changed",
                super::kafka_code_text(code)
            ))
        })?;
        described = true;
        for e in &cr.entries {
            if e.source != ConfigSource::DynamicTopic {
                continue;
            }
            match &e.value {
                Some(v) => {
                    merged.insert(e.name.clone(), v.clone());
                }
                None if edited(&e.name) => {}
                None => {
                    return Err(Error::Invalid(format!(
                        "{topic} has a sensitive override `{}` whose value the broker does not return; \
                         changing another key would reset it, so nothing was changed",
                        e.name
                    )))
                }
            }
        }
    }
    if !described {
        return Err(Error::Upstream(format!(
            "the broker returned no config for {topic}; nothing was changed"
        )));
    }
    for kv in edits {
        merged.insert(kv.name.clone(), kv.value.clone());
    }
    Ok(merged.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdkafka::admin::{AlterConfig, ConfigEntry, ConfigResource, OwnedResourceSpecifier};
    use rdkafka::error::RDKafkaErrorCode;

    fn entry(name: &str, value: Option<&str>, source: ConfigSource) -> ConfigEntry {
        ConfigEntry {
            name: name.into(),
            value: value.map(Into::into),
            source,
            is_read_only: false,
            is_default: false,
            is_sensitive: value.is_none(),
        }
    }

    fn described(entries: Vec<ConfigEntry>) -> Vec<ConfigResourceResult> {
        vec![Ok(ConfigResource {
            specifier: OwnedResourceSpecifier::Topic("orders".into()),
            entries,
        })]
    }

    fn kv(name: &str, value: &str) -> ConfigKv {
        ConfigKv {
            name: name.into(),
            value: value.into(),
        }
    }

    #[test]
    fn editing_one_key_carries_every_other_override() {
        let current = described(vec![
            entry("retention.ms", Some("86400000"), ConfigSource::DynamicTopic),
            entry(
                "cleanup.policy",
                Some("compact"),
                ConfigSource::DynamicTopic,
            ),
            // Broker/default values are NOT pinned onto the topic.
            entry("segment.bytes", Some("1073741824"), ConfigSource::Default),
            entry(
                "max.message.bytes",
                Some("2000000"),
                ConfigSource::StaticBroker,
            ),
        ]);
        let got =
            merged_topic_overrides("orders", current, &[kv("retention.ms", "3600000")]).unwrap();
        assert_eq!(
            got,
            vec![
                ("cleanup.policy".to_string(), "compact".to_string()),
                ("retention.ms".to_string(), "3600000".to_string()),
            ]
        );
        // The request actually sent: exactly those keys.
        let mut req = AlterConfig::new(rdkafka::admin::ResourceSpecifier::Topic("orders"));
        for (k, v) in &got {
            req = req.set(k, v);
        }
        assert_eq!(req.entries.len(), 2);
        assert_eq!(req.entries.get("cleanup.policy"), Some(&"compact"));
        assert_eq!(req.entries.get("retention.ms"), Some(&"3600000"));
    }

    #[test]
    fn failed_describe_refuses_instead_of_wiping_overrides() {
        let current = vec![Err(RDKafkaErrorCode::TopicAuthorizationFailed)];
        let err = merged_topic_overrides("orders", current, &[kv("retention.ms", "1")])
            .expect_err("a failed describe must not send the edited key alone");
        assert!(err.to_string().contains("nothing was changed"), "{err}");
        let err = merged_topic_overrides("orders", Vec::new(), &[kv("retention.ms", "1")])
            .expect_err("an empty describe must not send the edited key alone");
        assert!(err.to_string().contains("nothing was changed"), "{err}");
    }

    #[test]
    fn unreadable_sensitive_override_refuses_unless_it_is_the_edited_key() {
        let current = || {
            described(vec![
                entry("retention.ms", Some("1000"), ConfigSource::DynamicTopic),
                entry("secret.thing", None, ConfigSource::DynamicTopic),
            ])
        };
        assert!(merged_topic_overrides("orders", current(), &[kv("retention.ms", "5")]).is_err());
        let got =
            merged_topic_overrides("orders", current(), &[kv("secret.thing", "new")]).unwrap();
        assert_eq!(
            got,
            vec![
                ("retention.ms".to_string(), "1000".to_string()),
                ("secret.thing".to_string(), "new".to_string()),
            ]
        );
    }
}

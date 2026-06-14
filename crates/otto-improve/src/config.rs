//! Read `SelfImprovementConfig` from a workspace and compute run scheduling.

use chrono::{DateTime, Duration, Utc};
use otto_core::api::SelfImprovementConfig;
use serde_json::Value;

/// Parse the `self_improvement` block out of `Workspace.settings`, falling
/// back to defaults when absent or malformed.
pub fn effective_config(settings: &Value) -> SelfImprovementConfig {
    settings
        .get("self_improvement")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Merge scheduler-managed timestamps back into a settings JSON value, leaving
/// every other settings key untouched. Returns the new settings object.
pub fn write_config(settings: &Value, cfg: &SelfImprovementConfig) -> Value {
    let mut obj = settings.as_object().cloned().unwrap_or_default();
    obj.insert(
        "self_improvement".to_string(),
        serde_json::to_value(cfg).unwrap_or(Value::Null),
    );
    Value::Object(obj)
}

/// Whether a scheduled run is due at `now`. Due when enabled and either no
/// `next_run_at` has been set yet (never run) or it is in the past.
pub fn is_due(cfg: &SelfImprovementConfig, now: DateTime<Utc>) -> bool {
    if !cfg.enabled {
        return false;
    }
    match cfg.next_run_at {
        None => true,
        Some(t) => t <= now,
    }
}

/// The next run timestamp = `now + cadence_minutes`.
pub fn next_run(cfg: &SelfImprovementConfig, now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::minutes(cfg.cadence_minutes.max(1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 13, h, 0, 0).unwrap()
    }

    #[test]
    fn missing_block_yields_defaults() {
        let cfg = effective_config(&json!({"other": 1}));
        assert!(!cfg.enabled);
        assert_eq!(cfg.cadence_minutes, 60);
        assert_eq!(cfg.lookback_hours, 24);
    }

    #[test]
    fn disabled_is_never_due() {
        let cfg = SelfImprovementConfig::default();
        assert!(!is_due(&cfg, t(12)));
    }

    #[test]
    fn enabled_without_next_run_is_due() {
        let cfg = SelfImprovementConfig { enabled: true, ..Default::default() };
        assert!(is_due(&cfg, t(12)));
    }

    #[test]
    fn enabled_with_future_next_run_is_not_due() {
        let cfg = SelfImprovementConfig {
            enabled: true,
            next_run_at: Some(t(13)),
            ..Default::default()
        };
        assert!(!is_due(&cfg, t(12)));
        assert!(is_due(&cfg, t(14)));
    }

    #[test]
    fn write_config_preserves_other_keys() {
        let settings = json!({"keep": "me"});
        let cfg = SelfImprovementConfig { enabled: true, ..Default::default() };
        let out = write_config(&settings, &cfg);
        assert_eq!(out.get("keep").unwrap(), "me");
        assert!(out.get("self_improvement").unwrap().get("enabled").unwrap().as_bool().unwrap());
    }
}

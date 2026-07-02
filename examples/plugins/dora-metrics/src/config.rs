//! Plugin configuration — persisted as `config.json` under
//! `OTTO_PLUGIN_DATA_DIR`. Signals are configurable so the plugin adapts to a
//! team's conventions (deploy-tag naming, branch prefixes, scan depth).

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Branch-name prefixes used to classify merge commits. Matching is
/// case-insensitive substring on the merge subject; hotfix wins over release
/// wins over feature when several match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BranchPrefixes {
    pub feature: Vec<String>,
    pub release: Vec<String>,
    pub hotfix: Vec<String>,
}

impl Default for BranchPrefixes {
    fn default() -> Self {
        Self {
            feature: vec!["feature/".into()],
            release: vec!["release/".into()],
            hotfix: vec!["hotfix/".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Case-insensitive substring a tag must contain to count as a deploy.
    pub deploy_tag_pattern: String,
    pub branch_prefixes: BranchPrefixes,
    /// How many commits `git log --all` scans per repo.
    pub scan_depth: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            deploy_tag_pattern: "deploy".into(),
            branch_prefixes: BranchPrefixes::default(),
            scan_depth: 5000,
        }
    }
}

impl Config {
    /// Load from `<dir>/config.json`; any missing/corrupt file falls back to
    /// defaults (a plugin must never fail to boot over bad state).
    pub fn load(dir: &Path) -> Config {
        let path = dir.join("config.json");
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    /// Atomic save (tmp + rename) to `<dir>/config.json`.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let tmp = dir.join("config.json.tmp");
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, dir.join("config.json")).map_err(|e| format!("rename: {e}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.deploy_tag_pattern.trim().is_empty() {
            return Err("deploy_tag_pattern must not be empty".into());
        }
        if !(100..=50_000).contains(&self.scan_depth) {
            return Err("scan_depth must be between 100 and 50000".into());
        }
        let p = &self.branch_prefixes;
        for list in [&p.feature, &p.release, &p.hotfix] {
            if list.is_empty() || list.iter().any(|s| s.trim().is_empty()) {
                return Err("branch prefixes must be non-empty".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_defaults_on_missing_and_corrupt() {
        let dir = std::env::temp_dir().join(format!("dora-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Config::load(&dir), Config::default());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), b"{not json").unwrap();
        assert_eq!(Config::load(&dir), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("dora-cfg-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = Config {
            deploy_tag_pattern: "prod-".into(),
            scan_depth: 1234,
            ..Config::default()
        };
        cfg.save(&dir).unwrap();
        assert_eq!(Config::load(&dir), cfg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut cfg = Config {
            deploy_tag_pattern: "  ".into(),
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
        cfg.deploy_tag_pattern = "deploy".into();
        cfg.scan_depth = 0;
        assert!(cfg.validate().is_err());
        cfg.scan_depth = 99_999;
        assert!(cfg.validate().is_err());
        cfg.scan_depth = 5000;
        assert!(cfg.validate().is_ok());
        cfg.branch_prefixes.hotfix = vec![];
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn partial_json_fills_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"deploy_tag_pattern":"rel"}"#).unwrap();
        assert_eq!(cfg.deploy_tag_pattern, "rel");
        assert_eq!(cfg.scan_depth, 5000);
        assert_eq!(cfg.branch_prefixes, BranchPrefixes::default());
    }
}

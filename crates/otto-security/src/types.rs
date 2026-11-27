use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Vulnerability,
    Secret,
    Misconfiguration,
    License,
    Sast,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanSettings {
    pub checks: Vec<Check>,
    pub excludes: Vec<String>,
    pub parallelism: u8,
    pub memory_mib: u32,
    pub timeout_secs: u32,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            checks: vec![
                Check::Vulnerability,
                Check::Secret,
                Check::Misconfiguration,
                Check::Sast,
            ],
            excludes: Vec::new(),
            parallelism: 1,
            memory_mib: 1024,
            timeout_secs: 180,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub fingerprint: String,
    pub engine: String,
    pub check: Check,
    pub severity: Severity,
    pub root: String,
    pub path: String,
    pub line: Option<u64>,
    pub rule: String,
    pub title: String,
    pub description: String,
    pub fix: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanMetrics {
    pub wall_ms: u64,
    pub peak_cpu_percent: f64,
    pub peak_rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineVersion {
    pub version: String,
    pub updated_at: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    Cached,
    Incremental,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineRun {
    pub name: String,
    pub version: EngineVersion,
    pub metrics: ScanMetrics,
    pub mode: ScanMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub id: String,
    pub status: ScanStatus,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub engine: Option<EngineVersion>,
    pub metrics: Option<ScanMetrics>,
    pub error: Option<String>,
    pub count: u32,
    pub engines: Vec<EngineRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDetail {
    pub scan: ScanSummary,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectScans {
    pub settings: ScanSettings,
    pub engine: Option<EngineVersion>,
    pub scans: Vec<ScanSummary>,
    pub opengrep: Option<EngineVersion>,
}

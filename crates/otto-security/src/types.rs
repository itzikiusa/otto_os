use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Vulnerability,
    Secret,
    Misconfiguration,
    License,
    Sast,
    Dast,
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

/// What started a scan: a person (`manual`) or the project schedule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanTrigger {
    #[default]
    Manual,
    Scheduled,
}

/// An engine that did not contribute to an otherwise completed scan:
/// refused before it started (`failed: false`) or failed while running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineNote {
    pub engine: String,
    pub reason: String,
    pub failed: bool,
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
    #[serde(default)]
    pub trigger: ScanTrigger,
    #[serde(default)]
    pub skipped: Vec<EngineNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDetail {
    pub scan: ScanSummary,
    pub findings: Vec<Finding>,
    pub annotations: FindingAnnotations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectScans {
    pub settings: ScanSettings,
    pub engine: Option<EngineVersion>,
    pub scans: Vec<ScanSummary>,
    pub opengrep: Option<EngineVersion>,
    pub nuclei: Option<EngineVersion>,
    pub schedule: Option<Schedule>,
    pub baseline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleInput {
    pub cron: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub cron: String,
    pub timezone: String,
    pub next_runs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FindingAnnotations {
    pub existing: Vec<String>,
    pub baseline: Vec<String>,
    pub suppressions: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManageScan {
    PreviewSchedule {
        schedule: ScheduleInput,
    },
    SetSchedule {
        schedule: Option<ScheduleInput>,
    },
    SetSuppression {
        fingerprint: String,
        reason: Option<String>,
    },
    SetBaseline {
        scan_id: Option<String>,
    },
    ImportSarif {
        sarif: serde_json::Value,
    },
}

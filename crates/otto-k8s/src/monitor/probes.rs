//! Monitor configuration model: probes (what to fetch from every pod and how
//! to parse it), exclusions (which pods to skip), transport, cadence and
//! retention — plus validation (spec limits), the `*`/`?` glob matcher used by
//! exclusions and prometheus include/exclude lists, a `kubectl -l`-style
//! label-selector matcher, and the wizard presets.

use std::collections::BTreeMap;

use otto_core::{Error, Result};
use otto_state::K8sMonitorConfigRow;
use serde::{Deserialize, Serialize};

pub const MAX_PROBES: usize = 10;
pub const MAX_MAPPINGS: usize = 200;
pub const MIN_INTERVAL: u32 = 15;
pub const MAX_INTERVAL: u32 = 3600;
pub const MAX_CONCURRENCY: u32 = 32;
pub const MAX_RETENTION_DAYS: u32 = 90;
pub const DEFAULT_TIMEOUT_MS: u64 = 3000;
pub const MIN_SERIES_CAP: u32 = 100;
pub const MAX_SERIES_CAP: u32 = 10_000;
pub const DEFAULT_SERIES_CAP: u32 = 1500;

fn default_series_cap() -> u32 {
    DEFAULT_SERIES_CAP
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeFormat {
    Prometheus,
    Json,
    Health,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Number,
    Bytes,
    BytesHuman,
    DurationHuman,
    Percent,
}

/// One JSON-probe field mapping: `metric` emits a sample, `label` attaches a
/// pod-level string label (e.g. build version) to every sample of the cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mapping {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub unit: Unit,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    pub name: String,
    /// `None` = the container's first declared port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub path: String,
    pub format: ProbeFormat,
    #[serde(default)]
    pub mappings: Vec<Mapping>,
    /// Prometheus series-name globs; empty = everything.
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Exclusion {
    Namespace {
        #[serde(rename = "match")]
        pattern: String,
    },
    Pod {
        #[serde(rename = "match")]
        pattern: String,
    },
    Label {
        selector: String,
    },
    /// Matched against `"<kind>:<workload>"` (kind lowercased), e.g. `cronjob:*`.
    Workload {
        #[serde(rename = "match")]
        pattern: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Auto,
    Proxy,
    PortForward,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Auto => "auto",
            Transport::Proxy => "proxy",
            Transport::PortForward => "port_forward",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "proxy" => Transport::Proxy,
            "port_forward" => Transport::PortForward,
            _ => Transport::Auto,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorConfig {
    pub enabled: bool,
    pub interval_secs: u32,
    /// Empty = the cluster's default namespace.
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub probes: Vec<Probe>,
    #[serde(default)]
    pub exclusions: Vec<Exclusion>,
    #[serde(default)]
    pub transport: Transport,
    pub concurrency: u32,
    pub retention_days: u32,
    /// Prometheus series kept per pod per cycle; `_bucket` series are dropped
    /// first when a body overflows it.
    #[serde(default = "default_series_cap")]
    pub series_cap: u32,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 60,
            namespaces: vec![],
            probes: vec![],
            exclusions: vec![],
            transport: Transport::Auto,
            concurrency: 8,
            retention_days: 14,
            series_cap: DEFAULT_SERIES_CAP,
        }
    }
}

impl MonitorConfig {
    /// Spec limits. `cluster_default_ns` is the cluster row's default
    /// namespace — when absent the config must name namespaces explicitly.
    pub fn validate(&self, cluster_default_ns: Option<&str>) -> Result<()> {
        let inv = |m: String| Error::Invalid(m);
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&self.interval_secs) {
            return Err(inv(format!("interval_secs must be {MIN_INTERVAL}..{MAX_INTERVAL}")));
        }
        if !(1..=MAX_CONCURRENCY).contains(&self.concurrency) {
            return Err(inv(format!("concurrency must be 1..{MAX_CONCURRENCY}")));
        }
        if !(1..=MAX_RETENTION_DAYS).contains(&self.retention_days) {
            return Err(inv(format!("retention_days must be 1..{MAX_RETENTION_DAYS}")));
        }
        if !(MIN_SERIES_CAP..=MAX_SERIES_CAP).contains(&self.series_cap) {
            return Err(inv(format!("series_cap must be {MIN_SERIES_CAP}..{MAX_SERIES_CAP}")));
        }
        if self.probes.len() > MAX_PROBES {
            return Err(inv(format!("at most {MAX_PROBES} probes")));
        }
        let mappings: usize = self.probes.iter().map(|p| p.mappings.len()).sum();
        if mappings > MAX_MAPPINGS {
            return Err(inv(format!("at most {MAX_MAPPINGS} mappings in total")));
        }
        let mut names = std::collections::HashSet::new();
        for p in &self.probes {
            let name = p.name.trim();
            if name.is_empty() {
                return Err(inv("probe name is required".into()));
            }
            if !names.insert(name.to_string()) {
                return Err(inv(format!("duplicate probe name '{name}'")));
            }
            if !p.path.starts_with('/') {
                return Err(inv(format!("probe '{name}': path must start with '/'")));
            }
            if p.port == Some(0) {
                return Err(inv(format!("probe '{name}': port must be 1..65535")));
            }
            if !(100..=30_000).contains(&p.timeout_ms) {
                return Err(inv(format!("probe '{name}': timeout_ms must be 100..30000")));
            }
            for m in &p.mappings {
                if m.field.trim().is_empty() {
                    return Err(inv(format!("probe '{name}': mapping field is required")));
                }
                if m.metric.is_none() && m.label.is_none() {
                    return Err(inv(format!(
                        "probe '{name}': mapping '{}' needs a metric or a label",
                        m.field
                    )));
                }
                if let Some(mn) = &m.metric {
                    if !metric_name_ok(mn) {
                        return Err(inv(format!("probe '{name}': bad metric name '{mn}'")));
                    }
                }
            }
        }
        for e in &self.exclusions {
            match e {
                Exclusion::Namespace { pattern }
                | Exclusion::Pod { pattern }
                | Exclusion::Workload { pattern } => {
                    if pattern.trim().is_empty() {
                        return Err(inv("exclusion pattern is required".into()));
                    }
                }
                Exclusion::Label { selector } => {
                    if selector.trim().is_empty() {
                        return Err(inv("exclusion selector is required".into()));
                    }
                }
            }
        }
        if self.effective_namespaces(cluster_default_ns).is_empty() {
            return Err(inv(
                "namespaces are required: the cluster has no default namespace".into(),
            ));
        }
        Ok(())
    }

    /// Explicit list, else the cluster default (trimmed, deduped, no blanks).
    pub fn effective_namespaces(&self, cluster_default_ns: Option<&str>) -> Vec<String> {
        let mut out: Vec<String> = self
            .namespaces
            .iter()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        out.dedup();
        if out.is_empty() {
            if let Some(d) = cluster_default_ns.map(str::trim).filter(|d| !d.is_empty()) {
                out.push(d.to_string());
            }
        }
        out
    }
}

/// Metric names are prometheus-style identifiers (they end up as ClickHouse
/// literals and query filters — see `queries::ident_ok`).
pub fn metric_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.'))
}

/// `*` (any run) / `?` (one char) glob, case-sensitive, no escapes.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// `k=v,k2!=v2,k3` (bare key = exists). Whitespace around terms is ignored;
/// an empty selector matches everything.
pub fn label_selector_matches(selector: &str, labels: &BTreeMap<String, String>) -> bool {
    selector
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .all(|term| {
            if let Some((k, v)) = term.split_once("!=") {
                labels.get(k.trim()).map(String::as_str) != Some(v.trim())
            } else if let Some((k, v)) = term.split_once('=') {
                labels.get(k.trim()).map(String::as_str) == Some(v.trim())
            } else if let Some(k) = term.strip_prefix('!') {
                !labels.contains_key(k.trim())
            } else {
                labels.contains_key(term)
            }
        })
}

/// The pod facts exclusions look at.
pub struct PodRef<'a> {
    pub namespace: &'a str,
    pub name: &'a str,
    pub workload_kind: &'a str,
    pub workload: &'a str,
    pub labels: &'a BTreeMap<String, String>,
}

pub fn is_excluded(ex: &[Exclusion], p: &PodRef<'_>) -> bool {
    ex.iter().any(|e| match e {
        Exclusion::Namespace { pattern } => glob_match(pattern, p.namespace),
        Exclusion::Pod { pattern } => glob_match(pattern, p.name),
        Exclusion::Label { selector } => label_selector_matches(selector, p.labels),
        Exclusion::Workload { pattern } => glob_match(
            pattern,
            &format!("{}:{}", p.workload_kind.to_ascii_lowercase(), p.workload),
        ),
    })
}

fn mapping(field: &str, metric: Option<&str>, label: Option<&str>, unit: Unit) -> Mapping {
    Mapping {
        field: field.into(),
        metric: metric.map(String::from),
        label: label.map(String::from),
        unit,
    }
}

fn probe(name: &str, port: Option<u16>, path: &str, format: ProbeFormat) -> Probe {
    Probe {
        name: name.into(),
        port,
        path: path.into(),
        format,
        mappings: vec![],
        include: vec![],
        exclude: vec![],
        timeout_ms: DEFAULT_TIMEOUT_MS,
    }
}

/// Wizard presets `(id, title, probes)`. They only fill the form; nothing
/// here is hard-wired into the collector.
pub fn presets() -> Vec<(String, String, Vec<Probe>)> {
    let go_info = Probe {
        mappings: vec![
            mapping("memory_stats.sys", Some("mem_sys_bytes"), None, Unit::BytesHuman),
            mapping("memory_stats.alloc", Some("mem_alloc_bytes"), None, Unit::BytesHuman),
            mapping("go_routines_num", Some("goroutines"), None, Unit::Number),
            mapping("build_info.version", None, Some("version"), Unit::Number),
            mapping("build_info.commit", None, Some("commit"), Unit::Number),
        ],
        ..probe("info", Some(9000), "/actuator/info", ProbeFormat::Json)
    };
    let go_prom = Probe {
        include: vec!["http_*".into()],
        ..probe("prom", Some(9000), "/actuator/prometheus", ProbeFormat::Prometheus)
    };
    let go_health = probe("health", Some(9000), "/actuator/health", ProbeFormat::Health);

    let spring_prom = Probe {
        include: vec![
            "process_cpu_usage".into(),
            "jvm_memory_used_bytes".into(),
            "jvm_memory_max_bytes".into(),
            "http_server_requests_seconds_*".into(),
        ],
        ..probe("prom", None, "/actuator/prometheus", ProbeFormat::Prometheus)
    };
    let spring_health = probe("health", None, "/actuator/health", ProbeFormat::Health);

    vec![
        (
            "go_actuator".into(),
            "Go actuator (Otto services)".into(),
            vec![go_info, go_prom, go_health],
        ),
        (
            "spring_actuator".into(),
            "Spring Boot actuator".into(),
            vec![spring_prom, spring_health],
        ),
        (
            "plain_metrics".into(),
            "Plain /metrics".into(),
            vec![probe("metrics", None, "/metrics", ProbeFormat::Prometheus)],
        ),
    ]
}

/// SQLite row → typed config. Tolerant: a JSON column that fails to parse
/// yields an empty list rather than an error (the UI shows the raw row).
pub fn from_row(row: &K8sMonitorConfigRow) -> MonitorConfig {
    fn list<T: for<'de> Deserialize<'de>>(v: &serde_json::Value) -> Vec<T> {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
    MonitorConfig {
        enabled: row.enabled,
        interval_secs: row.interval_secs.clamp(0, i64::from(u32::MAX)) as u32,
        namespaces: list(&row.namespaces),
        probes: list(&row.probes),
        exclusions: list(&row.exclusions),
        transport: Transport::parse(&row.transport),
        concurrency: row.concurrency.clamp(0, i64::from(u32::MAX)) as u32,
        retention_days: row.retention_days.clamp(0, i64::from(u32::MAX)) as u32,
        series_cap: row.series_cap.clamp(0, i64::from(u32::MAX)) as u32,
    }
}

pub fn to_row(cluster_id: &str, c: &MonitorConfig) -> K8sMonitorConfigRow {
    K8sMonitorConfigRow {
        cluster_id: cluster_id.to_string(),
        enabled: c.enabled,
        interval_secs: i64::from(c.interval_secs),
        namespaces: serde_json::to_value(&c.namespaces).unwrap_or_default(),
        probes: serde_json::to_value(&c.probes).unwrap_or_default(),
        exclusions: serde_json::to_value(&c.exclusions).unwrap_or_default(),
        transport: c.transport.as_str().into(),
        concurrency: i64::from(c.concurrency),
        retention_days: i64::from(c.retention_days),
        series_cap: i64::from(c.series_cap),
        updated_at: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, path: &str) -> Probe {
        probe(name, Some(9000), path, ProbeFormat::Json)
    }
    fn cfg() -> MonitorConfig {
        MonitorConfig {
            enabled: true,
            probes: vec![p("info", "/actuator/info")],
            ..MonitorConfig::default()
        }
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("*-confsrv-*", "gowithdrawal-confsrv-sh4sn"));
        assert!(glob_match("kube-*", "kube-system"));
        assert!(!glob_match("kube-*", "mscasino"));
        assert!(glob_match("frb-?????", "frb-12345"));
        assert!(!glob_match("frb-?????", "frb-1234"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(glob_match("exact", "exact"));
    }

    #[test]
    fn selector_eq_ne_exists() {
        let mut l = BTreeMap::new();
        l.insert("app".to_string(), "frb".to_string());
        l.insert("tier".to_string(), "job".to_string());
        assert!(label_selector_matches("app=frb,tier!=web", &l));
        assert!(label_selector_matches("app", &l));
        assert!(label_selector_matches("!missing", &l));
        assert!(!label_selector_matches("app=web", &l));
        assert!(!label_selector_matches("missing", &l));
        assert!(label_selector_matches("", &l));
    }

    #[test]
    fn exclusion_kinds() {
        let l = BTreeMap::new();
        let pr = PodRef {
            namespace: "mscasino",
            name: "gowithdrawal-confsrv-sh4sn",
            workload_kind: "CronJob",
            workload: "gowithdrawal-confsrv",
            labels: &l,
        };
        assert!(is_excluded(&[Exclusion::Pod { pattern: "*-confsrv-*".into() }], &pr));
        assert!(is_excluded(&[Exclusion::Workload { pattern: "cronjob:*".into() }], &pr));
        assert!(!is_excluded(&[Exclusion::Namespace { pattern: "kube-*".into() }], &pr));
        assert!(!is_excluded(&[], &pr));
    }

    #[test]
    fn validate_limits() {
        assert!(cfg().validate(Some("mscasino")).is_ok());
        let mut c = cfg();
        c.interval_secs = 5;
        assert!(c.validate(Some("ns")).is_err());
        let mut c = cfg();
        c.probes[0].path = "actuator".into();
        assert!(c.validate(Some("ns")).is_err());
        let mut c = cfg();
        c.probes = (0..11).map(|i| p(&format!("p{i}"), "/m")).collect();
        assert!(c.validate(Some("ns")).is_err());
        let mut c = cfg();
        c.probes = vec![p("a", "/m"), p("a", "/n")];
        assert!(c.validate(Some("ns")).is_err(), "duplicate names");
        let mut c = cfg();
        c.retention_days = 91;
        assert!(c.validate(Some("ns")).is_err());
        let mut c = cfg();
        c.concurrency = 0;
        assert!(c.validate(Some("ns")).is_err());
        let mut c = cfg();
        c.probes[0].mappings = vec![mapping("x", None, None, Unit::Number)];
        assert!(c.validate(Some("ns")).is_err(), "mapping needs metric or label");
        let mut c = cfg();
        c.probes[0].mappings = vec![mapping("x", Some("bad name"), None, Unit::Number)];
        assert!(c.validate(Some("ns")).is_err(), "metric name charset");
        // Groove STG case: no default namespace + empty list → invalid
        assert!(cfg().validate(None).is_err());
        let mut c = cfg();
        c.namespaces = vec!["groove".into()];
        assert!(c.validate(None).is_ok());
        assert_eq!(c.effective_namespaces(None), vec!["groove".to_string()]);
        assert_eq!(cfg().effective_namespaces(Some("mscasino")), vec!["mscasino".to_string()]);
    }

    #[test]
    fn presets_parse_and_validate() {
        for (id, _, probes) in presets() {
            let mut c = cfg();
            c.probes = probes;
            assert!(c.validate(Some("ns")).is_ok(), "{id}");
        }
        assert!(presets()
            .iter()
            .any(|(id, _, p)| id == "go_actuator" && p.iter().any(|x| x.path == "/actuator/prometheus")));
    }

    #[test]
    fn row_roundtrip_tolerates_garbage() {
        let mut row = K8sMonitorConfigRow::default_for("c1");
        row.probes = serde_json::json!("not-an-array");
        assert!(from_row(&row).probes.is_empty());
        let r2 = to_row("c1", &cfg());
        assert_eq!(from_row(&r2), cfg());
        assert_eq!(r2.transport, "auto");
    }

    #[test]
    fn exclusion_json_shape() {
        let e: Exclusion = serde_json::from_str(r#"{"kind":"label","selector":"tier=job"}"#).unwrap();
        assert_eq!(e, Exclusion::Label { selector: "tier=job".into() });
        let e: Exclusion = serde_json::from_str(r#"{"kind":"pod","match":"*-x"}"#).unwrap();
        assert_eq!(e, Exclusion::Pod { pattern: "*-x".into() });
    }
}

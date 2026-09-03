//! CloudWatch metrics for one SQS queue / EC2 instance / RDS instance (View).
//!
//! One `aws cloudwatch get-metric-data` call per request: the whole per-
//! namespace catalog goes into a single `--metric-data-queries file://<tmp>`
//! JSON (written under `<data_dir>/tmp/<ulid>.json`, removed afterwards — the
//! dimension value only ever travels inside that JSON, never as a path). The
//! period follows the range (1h→60 s … 30d→3600 s) and honours CloudWatch's
//! retention rule (data older than 15 days needs ≥300 s, older than 63 days
//! ≥3600 s). Results are cached 30 s per (account, region, namespace,
//! dimension, range) so the UI's auto-refresh stays cheap.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use otto_core::domain::Feature;
use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::accounts::AwsService;

/// Cache TTL for one (account, namespace, dimension, range) answer.
pub const CACHE_TTL: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// Which CloudWatch namespace a request targets — each has exactly one
/// dimension key and one feature grant that gates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Sqs,
    Ec2,
    Rds,
}

impl Namespace {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "AWS/SQS" => Some(Self::Sqs),
            "AWS/EC2" => Some(Self::Ec2),
            "AWS/RDS" => Some(Self::Rds),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sqs => "AWS/SQS",
            Self::Ec2 => "AWS/EC2",
            Self::Rds => "AWS/RDS",
        }
    }
    /// The one dimension name CloudWatch keys this namespace's per-resource
    /// metrics on.
    pub fn dimension(&self) -> &'static str {
        match self {
            Self::Sqs => "QueueName",
            Self::Ec2 => "InstanceId",
            Self::Rds => "DBInstanceIdentifier",
        }
    }
    /// Feature whose `View` grant is required (checked in the handler — the
    /// policy table only sees the path, not the namespace).
    pub fn feature(&self) -> Feature {
        match self {
            Self::Sqs => Feature::AwsSqs,
            Self::Ec2 => Feature::AwsEc2,
            Self::Rds => Feature::AwsRds,
        }
    }
}

/// Human unit of a series — the UI picks its axis formatter from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Count,
    Bytes,
    Percent,
    Seconds,
    Ms,
    CountPerSec,
    BytesPerSec,
}

/// One catalog entry: the query id (also the response `id`, must match
/// CloudWatch's `^[a-z][a-zA-Z0-9_]*$`), the metric, its statistic and unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDef {
    pub id: &'static str,
    pub metric: &'static str,
    pub stat: &'static str,
    pub unit: Unit,
    pub label: &'static str,
}

const fn m(
    id: &'static str,
    metric: &'static str,
    stat: &'static str,
    unit: Unit,
    label: &'static str,
) -> MetricDef {
    MetricDef {
        id,
        metric,
        stat,
        unit,
        label,
    }
}

pub const SQS_CATALOG: &[MetricDef] = &[
    m(
        "messages_sent",
        "NumberOfMessagesSent",
        "Sum",
        Unit::Count,
        "Sent",
    ),
    m(
        "messages_received",
        "NumberOfMessagesReceived",
        "Sum",
        Unit::Count,
        "Received",
    ),
    m(
        "messages_deleted",
        "NumberOfMessagesDeleted",
        "Sum",
        Unit::Count,
        "Deleted",
    ),
    m(
        "empty_receives",
        "NumberOfEmptyReceives",
        "Sum",
        Unit::Count,
        "Empty receives",
    ),
    m(
        "sent_message_size",
        "SentMessageSize",
        "Average",
        Unit::Bytes,
        "Avg message size",
    ),
    m(
        "bytes_in",
        "SentMessageSize",
        "Sum",
        Unit::Bytes,
        "Bytes in",
    ),
    m(
        "oldest_message_age",
        "ApproximateAgeOfOldestMessage",
        "Maximum",
        Unit::Seconds,
        "Age of oldest message",
    ),
    m(
        "messages_visible",
        "ApproximateNumberOfMessagesVisible",
        "Average",
        Unit::Count,
        "Visible",
    ),
    m(
        "messages_not_visible",
        "ApproximateNumberOfMessagesNotVisible",
        "Average",
        Unit::Count,
        "In flight",
    ),
    m(
        "messages_delayed",
        "ApproximateNumberOfMessagesDelayed",
        "Average",
        Unit::Count,
        "Delayed",
    ),
];

pub const EC2_CATALOG: &[MetricDef] = &[
    m("cpu", "CPUUtilization", "Average", Unit::Percent, "CPU"),
    m("network_in", "NetworkIn", "Sum", Unit::Bytes, "Network in"),
    m(
        "network_out",
        "NetworkOut",
        "Sum",
        Unit::Bytes,
        "Network out",
    ),
    m(
        "packets_in",
        "NetworkPacketsIn",
        "Sum",
        Unit::Count,
        "Packets in",
    ),
    m(
        "packets_out",
        "NetworkPacketsOut",
        "Sum",
        Unit::Count,
        "Packets out",
    ),
    m(
        "disk_read_bytes",
        "DiskReadBytes",
        "Sum",
        Unit::Bytes,
        "Disk read",
    ),
    m(
        "disk_write_bytes",
        "DiskWriteBytes",
        "Sum",
        Unit::Bytes,
        "Disk write",
    ),
    m(
        "disk_read_ops",
        "DiskReadOps",
        "Sum",
        Unit::Count,
        "Disk read ops",
    ),
    m(
        "disk_write_ops",
        "DiskWriteOps",
        "Sum",
        Unit::Count,
        "Disk write ops",
    ),
    m(
        "status_check_failed",
        "StatusCheckFailed",
        "Maximum",
        Unit::Count,
        "Status check failed",
    ),
    m(
        "cpu_credit_balance",
        "CPUCreditBalance",
        "Average",
        Unit::Count,
        "CPU credit balance",
    ),
    m(
        "cpu_credit_usage",
        "CPUCreditUsage",
        "Sum",
        Unit::Count,
        "CPU credit usage",
    ),
];

pub const RDS_CATALOG: &[MetricDef] = &[
    m("cpu", "CPUUtilization", "Average", Unit::Percent, "CPU"),
    m(
        "connections",
        "DatabaseConnections",
        "Average",
        Unit::Count,
        "Connections",
    ),
    m(
        "freeable_memory",
        "FreeableMemory",
        "Average",
        Unit::Bytes,
        "Freeable memory",
    ),
    m(
        "free_storage",
        "FreeStorageSpace",
        "Average",
        Unit::Bytes,
        "Free storage",
    ),
    m(
        "read_iops",
        "ReadIOPS",
        "Average",
        Unit::CountPerSec,
        "Read IOPS",
    ),
    m(
        "write_iops",
        "WriteIOPS",
        "Average",
        Unit::CountPerSec,
        "Write IOPS",
    ),
    m(
        "read_latency",
        "ReadLatency",
        "Average",
        Unit::Seconds,
        "Read latency",
    ),
    m(
        "write_latency",
        "WriteLatency",
        "Average",
        Unit::Seconds,
        "Write latency",
    ),
    m(
        "read_throughput",
        "ReadThroughput",
        "Average",
        Unit::BytesPerSec,
        "Read throughput",
    ),
    m(
        "write_throughput",
        "WriteThroughput",
        "Average",
        Unit::BytesPerSec,
        "Write throughput",
    ),
    m(
        "network_rx",
        "NetworkReceiveThroughput",
        "Average",
        Unit::BytesPerSec,
        "Network receive",
    ),
    m(
        "network_tx",
        "NetworkTransmitThroughput",
        "Average",
        Unit::BytesPerSec,
        "Network transmit",
    ),
    m(
        "swap_usage",
        "SwapUsage",
        "Average",
        Unit::Bytes,
        "Swap usage",
    ),
    m(
        "disk_queue_depth",
        "DiskQueueDepth",
        "Average",
        Unit::Count,
        "Disk queue depth",
    ),
    m(
        "burst_balance",
        "BurstBalance",
        "Average",
        Unit::Percent,
        "Burst balance",
    ),
];

/// The CPU-credit metrics only exist on burstable families (t2/t3/t3a/t4g);
/// for anything else they're dropped from the query so the UI never shows
/// two permanently empty cards.
pub fn is_burstable(instance_type: Option<&str>) -> bool {
    instance_type
        .map(|t| {
            let fam = t.split('.').next().unwrap_or("");
            matches!(fam, "t2" | "t3" | "t3a" | "t4g")
        })
        .unwrap_or(false)
}

/// Catalog for a namespace, filtered by what the resource can report.
pub fn catalog(ns: Namespace, instance_type: Option<&str>) -> Vec<MetricDef> {
    match ns {
        Namespace::Sqs => SQS_CATALOG.to_vec(),
        Namespace::Rds => RDS_CATALOG.to_vec(),
        Namespace::Ec2 => {
            let burst = instance_type.is_none() || is_burstable(instance_type);
            EC2_CATALOG
                .iter()
                .copied()
                .filter(|d| burst || !d.metric.starts_with("CPUCredit"))
                .collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Range / period
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Range {
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
    #[serde(rename = "30d")]
    D30,
}

impl Range {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::H1),
            "6h" => Some(Self::H6),
            "24h" => Some(Self::H24),
            "7d" => Some(Self::D7),
            "30d" => Some(Self::D30),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::H1 => "1h",
            Self::H6 => "6h",
            Self::H24 => "24h",
            Self::D7 => "7d",
            Self::D30 => "30d",
        }
    }
    pub fn duration(&self) -> chrono::Duration {
        match self {
            Self::H1 => chrono::Duration::hours(1),
            Self::H6 => chrono::Duration::hours(6),
            Self::H24 => chrono::Duration::hours(24),
            Self::D7 => chrono::Duration::days(7),
            Self::D30 => chrono::Duration::days(30),
        }
    }
    /// The product choice per range (before the retention floor).
    fn base_period(&self) -> u32 {
        match self {
            Self::H1 => 60,
            Self::H6 | Self::H24 => 300,
            Self::D7 | Self::D30 => 3600,
        }
    }
}

/// CloudWatch's retention rule: points older than 15 days are only kept at
/// ≥5-minute resolution, older than 63 days at ≥1-hour.
pub fn min_period_for_age(age: chrono::Duration) -> u32 {
    if age > chrono::Duration::days(63) {
        3600
    } else if age > chrono::Duration::days(15) {
        300
    } else {
        60
    }
}

/// Period for a range whose window ends `now` and starts `range` earlier.
pub fn period_for(range: Range) -> u32 {
    range
        .base_period()
        .max(min_period_for_age(range.duration()))
}

/// Floor a timestamp to a period boundary (CloudWatch aligns datapoints to
/// multiples of the period since the epoch).
fn align(t: DateTime<Utc>, period: u32) -> DateTime<Utc> {
    let p = i64::from(period);
    let secs = t.timestamp() - t.timestamp().rem_euclid(p);
    Utc.timestamp_opt(secs, 0).single().unwrap_or(t)
}

// ---------------------------------------------------------------------------
// Query JSON
// ---------------------------------------------------------------------------

/// The `--metric-data-queries` document for a catalog + one dimension.
pub fn build_queries(ns: Namespace, defs: &[MetricDef], dim_value: &str, period: u32) -> Value {
    Value::Array(
        defs.iter()
            .map(|d| {
                json!({
                    "Id": d.id,
                    "Label": d.label,
                    "ReturnData": true,
                    "MetricStat": {
                        "Metric": {
                            "Namespace": ns.as_str(),
                            "MetricName": d.metric,
                            "Dimensions": [{ "Name": ns.dimension(), "Value": dim_value }]
                        },
                        "Period": period,
                        "Stat": d.stat
                    }
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub t: DateTime<Utc>,
    pub v: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Series {
    pub id: String,
    pub metric: String,
    pub stat: String,
    pub unit: Unit,
    pub label: String,
    pub points: Vec<Point>,
    /// Latest non-null value.
    pub current: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sum: Option<f64>,
    pub avg: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsResp {
    pub namespace: String,
    pub dim_name: String,
    pub dim_value: String,
    pub range: Range,
    pub period_seconds: u32,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub series: Vec<Series>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MetricsQuery {
    pub namespace: String,
    pub dim_name: Option<String>,
    pub dim_value: String,
    pub range: Option<String>,
    pub region: Option<String>,
    /// EC2 only: lets the daemon drop the CPU-credit metrics for
    /// non-burstable families.
    pub instance_type: Option<String>,
}

/// Dimension values are queue names / instance ids / DB identifiers: a bounded
/// alnum + `-_.` token starting with an alnum (so it can never read as a CLI
/// flag). Anything else is refused before it reaches the CLI.
pub fn validate_dim_value(v: &str) -> Result<()> {
    let ok = !v.is_empty()
        && v.len() <= 256
        && v.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !ok {
        return Err(Error::Invalid(format!("invalid dimension value '{v}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Summary of the non-null values of one series (all `None` when empty).
#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    current: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    sum: Option<f64>,
    avg: Option<f64>,
}

fn stats(values: &[Option<f64>]) -> Stats {
    let present: Vec<f64> = values.iter().flatten().copied().collect();
    if present.is_empty() {
        return Stats::default();
    }
    let sum: f64 = present.iter().sum();
    Stats {
        current: values.iter().rev().flatten().next().copied(),
        min: Some(present.iter().copied().fold(f64::INFINITY, f64::min)),
        max: Some(present.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        sum: Some(sum),
        avg: Some(sum / present.len() as f64),
    }
}

/// Parse `get-metric-data` output into one [`Series`] per catalog entry, on
/// the aligned `[start, end]` grid: every grid slot the API did not return is
/// a `null` point (a gap), and any off-grid timestamp the API did return is
/// kept in place so nothing is silently dropped. Catalog entries missing
/// from the output still get a (fully-null) series.
pub fn parse_results(
    v: &Value,
    defs: &[MetricDef],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    period: u32,
) -> Vec<Series> {
    let mut by_id: HashMap<&str, BTreeMap<DateTime<Utc>, f64>> = HashMap::new();
    if let Some(results) = v.get("MetricDataResults").and_then(|r| r.as_array()) {
        for r in results {
            let Some(id) = r.get("Id").and_then(|i| i.as_str()) else {
                continue;
            };
            let ts = r.get("Timestamps").and_then(|t| t.as_array());
            let vals = r.get("Values").and_then(|t| t.as_array());
            let entry = by_id.entry(id).or_default();
            if let (Some(ts), Some(vals)) = (ts, vals) {
                for (t, val) in ts.iter().zip(vals.iter()) {
                    let Some(t) = t
                        .as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&Utc))
                    else {
                        continue;
                    };
                    if let Some(val) = val.as_f64() {
                        // Duplicate timestamps (paginated overlap) keep the
                        // first value.
                        entry.entry(t).or_insert(val);
                    }
                }
            }
        }
    }
    // Grid: aligned start .. end (inclusive of the last complete bucket).
    let aligned = align(start, period);
    let step = chrono::Duration::seconds(i64::from(period));
    let mut grid: Vec<DateTime<Utc>> = Vec::new();
    let mut t = aligned;
    while t <= end {
        grid.push(t);
        t += step;
    }
    defs.iter()
        .map(|d| {
            let data = by_id.get(d.id);
            let mut slots: BTreeMap<DateTime<Utc>, Option<f64>> =
                grid.iter().map(|t| (*t, None)).collect();
            if let Some(data) = data {
                for (t, v) in data {
                    slots.insert(*t, Some(*v));
                }
            }
            let points: Vec<Point> = slots.into_iter().map(|(t, v)| Point { t, v }).collect();
            let values: Vec<Option<f64>> = points.iter().map(|p| p.v).collect();
            let Stats {
                current,
                min,
                max,
                sum,
                avg,
            } = stats(&values);
            Series {
                id: d.id.to_string(),
                metric: d.metric.to_string(),
                stat: d.stat.to_string(),
                unit: d.unit,
                label: d.label.to_string(),
                points,
                current,
                min,
                max,
                sum,
                avg,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

type Cache = Mutex<HashMap<String, (Instant, MetricsResp)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(account_id: &str, region: Option<&str>, q: &MetricsQuery, range: Range) -> String {
    format!(
        "{account_id}|{}|{}|{}|{}|{}",
        region.unwrap_or(""),
        q.namespace,
        q.dim_value,
        range.as_str(),
        q.instance_type.as_deref().unwrap_or("")
    )
}

fn cache_get(key: &str) -> Option<MetricsResp> {
    let c = cache().lock().ok()?;
    let (at, resp) = c.get(key)?;
    (at.elapsed() < CACHE_TTL).then(|| resp.clone())
}

fn cache_put(key: String, resp: MetricsResp) {
    if let Ok(mut c) = cache().lock() {
        c.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        c.insert(key, (Instant::now(), resp));
    }
}

/// Test hook: drop every cached answer.
pub fn cache_clear() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

// ---------------------------------------------------------------------------
// Call
// ---------------------------------------------------------------------------

/// Resolve + validate the query shape (namespace, dimension, range) without
/// touching the CLI. Returns `(namespace, range)`.
pub fn resolve(q: &MetricsQuery) -> Result<(Namespace, Range)> {
    let ns = Namespace::parse(&q.namespace).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown namespace '{}' (AWS/SQS, AWS/EC2 or AWS/RDS)",
            q.namespace
        ))
    })?;
    if let Some(d) = q.dim_name.as_deref().filter(|s| !s.is_empty()) {
        if d != ns.dimension() {
            return Err(Error::Invalid(format!(
                "dim_name for {} must be {}",
                ns.as_str(),
                ns.dimension()
            )));
        }
    }
    validate_dim_value(&q.dim_value)?;
    let range = match q.range.as_deref().filter(|s| !s.is_empty()) {
        None => Range::H1,
        Some(r) => Range::parse(r)
            .ok_or_else(|| Error::Invalid(format!("unknown range '{r}' (1h|6h|24h|7d|30d)")))?,
    };
    Ok((ns, range))
}

pub async fn get_metrics(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &MetricsQuery,
) -> Result<MetricsResp> {
    let (ns, range) = resolve(q)?;
    let region = q.region.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let key = cache_key(&a.id, region, q, range);
    if let Some(hit) = cache_get(&key) {
        return Ok(hit);
    }
    let period = period_for(range);
    let end = align(Utc::now(), period);
    let start = end - range.duration();
    let defs = catalog(ns, q.instance_type.as_deref());
    let queries = build_queries(ns, &defs, &q.dim_value, period);

    // Scratch file at an Otto-owned location: <data_dir>/tmp/<fresh ULID>.json.
    let tmp_dir = crate::paths::owned_dir(&svc.data_dir, "tmp")?;
    let tmp = crate::paths::owned_file(&tmp_dir, &otto_core::new_id(), "json")?;
    std::fs::write(&tmp, serde_json::to_vec(&queries)?)
        .map_err(|e| Error::Internal(format!("write metric queries: {e}")))?;
    let file_arg = format!("file://{}", tmp.to_string_lossy());
    let start_s = start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_s = end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let res = svc
        .run_json(
            a,
            region,
            &[
                "cloudwatch",
                "get-metric-data",
                "--metric-data-queries",
                &file_arg,
                "--start-time",
                &start_s,
                "--end-time",
                &end_s,
                "--scan-by",
                "TimestampAscending",
            ],
        )
        .await;
    let _ = std::fs::remove_file(&tmp);
    let v = res?;
    let resp = MetricsResp {
        namespace: ns.as_str().to_string(),
        dim_name: ns.dimension().to_string(),
        dim_value: q.dim_value.clone(),
        range,
        period_seconds: period,
        start,
        end,
        series: parse_results(&v, &defs, start, end, period),
    };
    cache_put(key, resp.clone());
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../testdata/get-metric-data.json");

    #[test]
    fn catalog_ids_are_cloudwatch_safe_and_unique() {
        for defs in [SQS_CATALOG, EC2_CATALOG, RDS_CATALOG] {
            let mut ids: Vec<&str> = defs.iter().map(|d| d.id).collect();
            for id in &ids {
                let mut ch = id.chars();
                assert!(ch.next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
                assert!(
                    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "{id}"
                );
            }
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), defs.len(), "duplicate ids");
        }
    }

    #[test]
    fn query_json_shape() {
        let defs = catalog(Namespace::Sqs, None);
        let q = build_queries(Namespace::Sqs, &defs, "orders.fifo", 300);
        let arr = q.as_array().unwrap();
        assert_eq!(arr.len(), SQS_CATALOG.len());
        let first = &arr[0];
        assert_eq!(first["Id"], "messages_sent");
        assert_eq!(first["ReturnData"], true);
        assert_eq!(first["MetricStat"]["Period"], 300);
        assert_eq!(first["MetricStat"]["Stat"], "Sum");
        assert_eq!(first["MetricStat"]["Metric"]["Namespace"], "AWS/SQS");
        assert_eq!(
            first["MetricStat"]["Metric"]["MetricName"],
            "NumberOfMessagesSent"
        );
        assert_eq!(
            first["MetricStat"]["Metric"]["Dimensions"][0]["Name"],
            "QueueName"
        );
        assert_eq!(
            first["MetricStat"]["Metric"]["Dimensions"][0]["Value"],
            "orders.fifo"
        );
        // Two stats over the same metric get distinct ids.
        let sizes: Vec<&Value> = arr
            .iter()
            .filter(|e| e["MetricStat"]["Metric"]["MetricName"] == "SentMessageSize")
            .collect();
        assert_eq!(sizes.len(), 2);
        assert_ne!(sizes[0]["Id"], sizes[1]["Id"]);
        // EC2 / RDS dimension keys.
        let e = build_queries(Namespace::Ec2, &catalog(Namespace::Ec2, None), "i-0abc", 60);
        assert_eq!(
            e[0]["MetricStat"]["Metric"]["Dimensions"][0]["Name"],
            "InstanceId"
        );
        let r = build_queries(Namespace::Rds, &catalog(Namespace::Rds, None), "db-1", 60);
        assert_eq!(
            r[0]["MetricStat"]["Metric"]["Dimensions"][0]["Name"],
            "DBInstanceIdentifier"
        );
        assert_eq!(r[0]["MetricStat"]["Metric"]["Namespace"], "AWS/RDS");
    }

    #[test]
    fn ec2_credit_metrics_only_for_burstable() {
        assert!(is_burstable(Some("t3.medium")));
        assert!(is_burstable(Some("t4g.nano")));
        assert!(!is_burstable(Some("m5.large")));
        assert!(!is_burstable(None));
        let all = catalog(Namespace::Ec2, None);
        assert!(all.iter().any(|d| d.id == "cpu_credit_balance"));
        let t3 = catalog(Namespace::Ec2, Some("t3a.small"));
        assert!(t3.iter().any(|d| d.id == "cpu_credit_balance"));
        let m5 = catalog(Namespace::Ec2, Some("m5.large"));
        assert!(!m5.iter().any(|d| d.metric.starts_with("CPUCredit")));
        assert_eq!(m5.len(), all.len() - 2);
    }

    #[test]
    fn period_from_range_rules() {
        assert_eq!(period_for(Range::H1), 60);
        assert_eq!(period_for(Range::H6), 300);
        assert_eq!(period_for(Range::H24), 300);
        assert_eq!(period_for(Range::D7), 3600);
        assert_eq!(period_for(Range::D30), 3600);
        // Retention floors.
        assert_eq!(min_period_for_age(chrono::Duration::days(1)), 60);
        assert_eq!(min_period_for_age(chrono::Duration::days(16)), 300);
        assert_eq!(min_period_for_age(chrono::Duration::days(64)), 3600);
        // Range parsing round-trips and rejects junk.
        for r in ["1h", "6h", "24h", "7d", "30d"] {
            assert_eq!(Range::parse(r).unwrap().as_str(), r);
        }
        assert!(Range::parse("2h").is_none());
        assert_eq!(serde_json::to_value(Range::D7).unwrap(), "7d");
    }

    #[test]
    fn resolve_validates_shape() {
        let ok = MetricsQuery {
            namespace: "AWS/SQS".into(),
            dim_name: Some("QueueName".into()),
            dim_value: "orders".into(),
            range: Some("6h".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&ok).unwrap(), (Namespace::Sqs, Range::H6));
        let default_range = MetricsQuery {
            namespace: "AWS/EC2".into(),
            dim_value: "i-0abc123456789def0".into(),
            ..Default::default()
        };
        assert_eq!(
            resolve(&default_range).unwrap(),
            (Namespace::Ec2, Range::H1)
        );
        let bad_ns = MetricsQuery {
            namespace: "AWS/Lambda".into(),
            dim_value: "fn".into(),
            ..Default::default()
        };
        assert!(resolve(&bad_ns).is_err());
        let bad_dim = MetricsQuery {
            namespace: "AWS/RDS".into(),
            dim_name: Some("InstanceId".into()),
            dim_value: "db".into(),
            ..Default::default()
        };
        assert!(resolve(&bad_dim).is_err());
        for bad in ["", "a b", "x;rm", "../etc", "--profile"] {
            assert!(validate_dim_value(bad).is_err(), "{bad}");
        }
        assert!(validate_dim_value("orders-dlq.fifo").is_ok());
        assert!(validate_dim_value("i-0abc123456789def0").is_ok());
    }

    #[test]
    fn parses_sample_into_sorted_series_with_stats() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let defs = catalog(Namespace::Sqs, None);
        let start = DateTime::parse_from_rfc3339("2024-06-01T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2024-06-01T10:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let series = parse_results(&v, &defs, start, end, 60);
        assert_eq!(series.len(), defs.len());
        let sent = series.iter().find(|s| s.id == "messages_sent").unwrap();
        assert_eq!(sent.metric, "NumberOfMessagesSent");
        assert_eq!(sent.stat, "Sum");
        assert_eq!(sent.unit, Unit::Count);
        // Grid: 10:00 .. 10:05 inclusive = 6 slots; the sample returns 10:00,
        // 10:02, 10:01 (unsorted) and 10:04 — 10:03 and 10:05 are gaps.
        assert_eq!(sent.points.len(), 6);
        let ts: Vec<String> = sent.points.iter().map(|p| p.t.to_rfc3339()).collect();
        assert!(ts.windows(2).all(|w| w[0] < w[1]), "sorted: {ts:?}");
        let vals: Vec<Option<f64>> = sent.points.iter().map(|p| p.v).collect();
        assert_eq!(
            vals,
            vec![Some(12.0), Some(3.0), Some(7.0), None, Some(20.0), None]
        );
        assert_eq!(sent.current, Some(20.0));
        assert_eq!(sent.min, Some(3.0));
        assert_eq!(sent.max, Some(20.0));
        assert_eq!(sent.sum, Some(42.0));
        assert_eq!(sent.avg, Some(10.5));
        // A metric present in the output with zero datapoints → all-null series.
        let age = series
            .iter()
            .find(|s| s.id == "oldest_message_age")
            .unwrap();
        assert_eq!(age.points.len(), 6);
        assert!(age.points.iter().all(|p| p.v.is_none()));
        assert_eq!(age.current, None);
        assert_eq!(age.sum, None);
        // A metric missing from the output entirely is still returned.
        let delayed = series.iter().find(|s| s.id == "messages_delayed").unwrap();
        assert!(delayed.points.iter().all(|p| p.v.is_none()));
        // Off-grid timestamps are kept, not dropped.
        let recv = series.iter().find(|s| s.id == "messages_received").unwrap();
        assert!(recv
            .points
            .iter()
            .any(|p| p.t.to_rfc3339().starts_with("2024-06-01T10:02:30") && p.v == Some(5.0)));
        // Wire shape.
        let j = serde_json::to_value(sent).unwrap();
        assert_eq!(j["unit"], "count");
        assert!(j["points"][3]["v"].is_null());
        assert!(j["points"][0]["t"]
            .as_str()
            .unwrap()
            .starts_with("2024-06-01T10:00:00"));
    }

    #[test]
    fn cache_round_trip_and_ttl_key() {
        cache_clear();
        let q = MetricsQuery {
            namespace: "AWS/EC2".into(),
            dim_value: "i-0abc123456789def0".into(),
            range: Some("1h".into()),
            ..Default::default()
        };
        let key = cache_key("acct", Some("eu-west-1"), &q, Range::H1);
        assert!(cache_get(&key).is_none());
        let resp = MetricsResp {
            namespace: "AWS/EC2".into(),
            dim_name: "InstanceId".into(),
            dim_value: q.dim_value.clone(),
            range: Range::H1,
            period_seconds: 60,
            start: Utc::now(),
            end: Utc::now(),
            series: vec![],
        };
        cache_put(key.clone(), resp.clone());
        assert_eq!(cache_get(&key), Some(resp));
        // Different range / region → different key.
        assert_ne!(key, cache_key("acct", Some("eu-west-1"), &q, Range::D7));
        assert_ne!(key, cache_key("acct", Some("us-east-1"), &q, Range::H1));
        cache_clear();
        assert!(cache_get(&key).is_none());
    }
}

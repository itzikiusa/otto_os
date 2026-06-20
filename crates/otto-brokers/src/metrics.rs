//! Cluster metrics: derived throughput (sampled from watermark deltas) and
//! per-broker CPU/RAM scraped from an optional Prometheus exposition endpoint
//! (Redpanda `:9644`, or a JMX exporter in front of Apache Kafka).

use crate::types::{BrokerResourceMetrics, ClusterMetrics, NamedMetric, ThroughputPoint};
use chrono::Utc;
use otto_core::{Error, Result};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

const MAX_POINTS: usize = 120;
/// Minimum interval between full watermark sweeps. The UI polls every 4 s;
/// sweeps are expensive over a tunnel, so we reuse the last value within this
/// window. The sweep runs on the FIRST call and then every WATERMARK_TTL_MS.
const WATERMARK_TTL_MS: i64 = 8_000;

/// One parsed Prometheus line: `name{labels} value`.
#[derive(Debug, Clone)]
pub struct Sample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

/// Counter metric names (seconds) we treat as CPU; we rate them between scrapes.
const CPU_COUNTERS: &[&str] = &[
    "redpanda_cpu_busy_seconds_total",
    "process_cpu_seconds_total",
    "kafka_cpu_seconds_total",
];
/// Gauge metric names treated as "memory used".
const MEM_USED: &[&str] = &[
    "redpanda_memory_allocated_memory",
    "jvm_memory_bytes_used",
    "process_resident_memory_bytes",
];
/// Gauge metric names treated as "memory available" (free).
const MEM_FREE: &[&str] = &["redpanda_memory_available_memory"];
/// Gauge metric names treated as "memory total/limit".
const MEM_TOTAL: &[&str] = &["jvm_memory_bytes_max", "machine_memory_bytes"];

/// Parse a Prometheus text-exposition document into samples (comments skipped).
pub fn parse_prometheus(text: &str) -> Vec<Sample> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, rest)) = split_metric(line) else {
            continue;
        };
        // value is the first whitespace-delimited token of `rest` (ignore an
        // optional trailing timestamp).
        let Some(tok) = rest.split_whitespace().next() else {
            continue;
        };
        let value = match tok {
            "NaN" | "+Inf" | "-Inf" => continue,
            _ => match tok.parse::<f64>() {
                Ok(v) => v,
                Err(_) => continue,
            },
        };
        let (name, labels) = head;
        out.push(Sample {
            name,
            labels,
            value,
        });
    }
    out
}

/// A metric name plus its parsed labels.
type MetricHead = (String, BTreeMap<String, String>);

/// Split `name{labels}` (or bare `name`) from the value portion.
/// Returns `((name, labels), value_str)`.
fn split_metric(line: &str) -> Option<(MetricHead, &str)> {
    if let Some(open) = line.find('{') {
        let name = line[..open].trim().to_string();
        let close = line[open + 1..].find('}')? + open + 1;
        let labels = parse_labels(&line[open + 1..close]);
        let rest = line[close + 1..].trim();
        Some(((name, labels), rest))
    } else {
        let mut it = line.splitn(2, char::is_whitespace);
        let name = it.next()?.to_string();
        let rest = it.next()?.trim();
        Some(((name, BTreeMap::new()), rest))
    }
}

fn parse_labels(s: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    // Simple key="value" splitter; values may contain commas/escapes rarely —
    // good enough for the curated metrics we read.
    for part in s.split("\",") {
        let part = part.trim().trim_end_matches('"');
        if let Some((k, v)) = part.split_once("=\"") {
            map.insert(k.trim().to_string(), v.to_string());
        }
    }
    map
}

fn matches_any(name: &str, set: &[&str]) -> bool {
    set.contains(&name)
}

fn instance_of(s: &Sample) -> String {
    for key in ["instance", "node", "pod", "node_id"] {
        if let Some(v) = s.labels.get(key) {
            return v.clone();
        }
    }
    "broker".to_string()
}

/// Rolling per-cluster metric state. Throughput is sampled on demand (the UI
/// polls `/metrics`); CPU counters are rated between consecutive scrapes. The
/// watermark sweep total is cached for `WATERMARK_TTL_MS` so a 4 s UI poll
/// doesn't issue a full per-partition scan on every request.
#[derive(Default)]
pub struct ClusterMetricState {
    throughput: VecDeque<ThroughputPoint>,
    last_total: Option<(i64, i64)>,
    /// Cached watermark total (ts_ms, total) from the last sweep.
    cached_watermark: Option<(i64, i64)>,
    /// instance → (ts_ms, summed cpu-seconds counter) from the previous scrape.
    last_cpu: HashMap<String, (i64, f64)>,
    /// topic → (ts_ms, high-watermark message count) from the previous
    /// `topics/stats` call, used to derive a per-topic msg/s production rate.
    topic_watermarks: HashMap<String, (i64, i64)>,
}

impl ClusterMetricState {
    /// Check if a new watermark sweep is due (first call or older than TTL).
    pub fn needs_sweep(&self) -> bool {
        match self.cached_watermark {
            None => true,
            Some((ts, _)) => chrono::Utc::now().timestamp_millis() - ts >= WATERMARK_TTL_MS,
        }
    }

    /// Store a freshly-fetched watermark total and return it.
    pub fn store_watermark(&mut self, total: i64) -> i64 {
        self.cached_watermark = Some((chrono::Utc::now().timestamp_millis(), total));
        total
    }

    /// Return the cached total without fetching.
    pub fn cached_total(&self) -> Option<i64> {
        self.cached_watermark.map(|(_, t)| t)
    }

    /// Record the current high-watermark `count` for topic `name` and return its
    /// production rate (msg/s) vs the previous sample, if one exists. Returns
    /// `None` on the first sample (baseline established), on a count error
    /// (`count < 0`), or when called again within 500 ms (too soon to measure —
    /// the baseline is kept). The rate is clamped to ≥ 0 since high watermarks
    /// are monotonic (a decrease means the topic was recreated, not negative
    /// throughput).
    pub fn topic_rate(&mut self, name: &str, count: i64) -> Option<f64> {
        if count < 0 {
            return None;
        }
        let now = chrono::Utc::now().timestamp_millis();
        match self.topic_watermarks.get(name).copied() {
            Some((prev_ts, prev_count)) if now - prev_ts >= 500 => {
                let dt = (now - prev_ts) as f64 / 1000.0;
                let rate = ((count - prev_count).max(0) as f64 / dt).max(0.0);
                self.topic_watermarks.insert(name.to_string(), (now, count));
                Some(rate)
            }
            // Too soon since the last sample — keep the baseline, no rate yet.
            Some(_) => None,
            // First sample for this topic — establish the baseline.
            None => {
                self.topic_watermarks.insert(name.to_string(), (now, count));
                None
            }
        }
    }
}

impl ClusterMetricState {
    /// Record the current cluster-wide message total and return msg/s.
    fn record_throughput(&mut self, now_ms: i64, total: i64) -> f64 {
        let rate = match self.last_total {
            Some((prev_ts, prev_total)) if now_ms > prev_ts => {
                let dt = (now_ms - prev_ts) as f64 / 1000.0;
                ((total - prev_total) as f64 / dt).max(0.0)
            }
            _ => 0.0,
        };
        self.last_total = Some((now_ms, total));
        self.throughput.push_back(ThroughputPoint {
            ts_ms: now_ms,
            total_messages: total,
            messages_per_sec: rate,
        });
        while self.throughput.len() > MAX_POINTS {
            self.throughput.pop_front();
        }
        rate
    }

    /// Assemble the metrics response, folding in an optional Prometheus scrape.
    pub fn build(&mut self, total_messages: i64, prometheus: Option<&str>) -> ClusterMetrics {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let rate = self.record_throughput(now_ms, total_messages);

        let (brokers, prometheus_available) = match prometheus {
            Some(text) => (self.brokers_from_prometheus(now_ms, text), true),
            None => (Vec::new(), false),
        };

        ClusterMetrics {
            throughput: self.throughput.iter().cloned().collect(),
            messages_per_sec: rate,
            total_messages,
            brokers,
            prometheus_available,
            sampled_at: now,
        }
    }

    fn brokers_from_prometheus(&mut self, now_ms: i64, text: &str) -> Vec<BrokerResourceMetrics> {
        let samples = parse_prometheus(text);

        // Aggregate per instance.
        let mut cpu_counter: HashMap<String, f64> = HashMap::new();
        let mut mem_used: HashMap<String, f64> = HashMap::new();
        let mut mem_free: HashMap<String, f64> = HashMap::new();
        let mut mem_total: HashMap<String, f64> = HashMap::new();
        let mut instances: BTreeMap<String, ()> = BTreeMap::new();
        let mut extra: HashMap<String, Vec<NamedMetric>> = HashMap::new();

        for s in &samples {
            let inst = instance_of(s);
            instances.insert(inst.clone(), ());
            if matches_any(&s.name, CPU_COUNTERS) {
                *cpu_counter.entry(inst).or_default() += s.value;
            } else if matches_any(&s.name, MEM_USED) {
                *mem_used.entry(inst).or_default() += s.value;
            } else if matches_any(&s.name, MEM_FREE) {
                *mem_free.entry(inst).or_default() += s.value;
            } else if matches_any(&s.name, MEM_TOTAL) {
                let e = mem_total.entry(inst).or_default();
                *e = e.max(s.value);
            }
        }

        // A few notable extra gauges for display (connection/partition counts).
        for s in &samples {
            if matches!(
                s.name.as_str(),
                "redpanda_kafka_partitions"
                    | "redpanda_storage_disk_free_bytes"
                    | "redpanda_storage_disk_total_bytes"
                    | "kafka_server_replicamanager_partitioncount"
            ) {
                let inst = instance_of(s);
                extra.entry(inst).or_default().push(NamedMetric {
                    name: s.name.clone(),
                    value: s.value,
                });
            }
        }

        let mut out = Vec::new();
        for (inst, _) in instances {
            // CPU percent from the counter delta since the last scrape.
            let cpu_percent = cpu_counter.get(&inst).and_then(|&now_val| {
                let prev = self.last_cpu.insert(inst.clone(), (now_ms, now_val));
                match prev {
                    Some((prev_ts, prev_val)) if now_ms > prev_ts => {
                        let dt = (now_ms - prev_ts) as f64 / 1000.0;
                        Some(((now_val - prev_val) / dt * 100.0).clamp(0.0, 100_000.0))
                    }
                    _ => None,
                }
            });
            let used = mem_used.get(&inst).copied();
            let total = mem_total.get(&inst).copied().or_else(|| {
                match (used, mem_free.get(&inst).copied()) {
                    (Some(u), Some(f)) => Some(u + f),
                    _ => None,
                }
            });
            out.push(BrokerResourceMetrics {
                instance: inst.clone(),
                cpu_percent,
                memory_used_bytes: used,
                memory_total_bytes: total,
                extra: extra.remove(&inst).unwrap_or_default(),
            });
        }
        out
    }
}

/// Scrape a Prometheus exposition endpoint, returning the raw text. When
/// `socks_proxy` is set the request rides an SSH SOCKS tunnel (the metrics
/// endpoint is private, behind the user's bastion).
pub async fn scrape(url: &str, skip_tls_verify: bool, socks_proxy: Option<&str>) -> Result<String> {
    // SSRF guard (audit S1): the metrics URL is a user-supplied cluster-profile
    // field. Resolve + classify the host before connecting, and re-validate every
    // redirect hop, so it can't reach loopback / RFC1918 / link-local
    // (169.254.169.254 cloud-metadata) / CGNAT addresses the loopback-bound
    // daemon could otherwise be steered into. Skipped when tunnelled: the
    // private target is the intent and the bastion creds are the authority.
    if socks_proxy.is_none() {
        otto_netguard::check_url(url)
            .await
            .map_err(|m| Error::Forbidden(format!("metrics endpoint blocked: {m}")))?;
    }
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_certs(skip_tls_verify)
        .redirect(otto_netguard::redirect_policy())
        .timeout(Duration::from_secs(8));
    if let Some(proxy) = socks_proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|e| Error::Internal(format!("metrics socks proxy: {e}")))?,
        );
    }
    let client = builder
        .build()
        .map_err(|e| Error::Internal(format!("metrics client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("metrics endpoint: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Upstream(format!(
            "metrics endpoint returned {}",
            resp.status()
        )));
    }
    resp.text()
        .await
        .map_err(|e| Error::Upstream(format!("metrics endpoint: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_exposition() {
        let text = "# HELP x\n# TYPE x counter\n\
                    redpanda_memory_allocated_memory{shard=\"0\"} 1048576\n\
                    redpanda_memory_allocated_memory{shard=\"1\"} 2097152\n\
                    redpanda_memory_available_memory{shard=\"0\"} 1000\n\
                    redpanda_cpu_busy_seconds_total{shard=\"0\"} 5.0\n\
                    redpanda_cpu_busy_seconds_total{shard=\"1\"} 3.0\n";
        let samples = parse_prometheus(text);
        assert_eq!(samples.len(), 5);
        let mem: f64 = samples
            .iter()
            .filter(|s| s.name == "redpanda_memory_allocated_memory")
            .map(|s| s.value)
            .sum();
        assert_eq!(mem, 3_145_728.0);
    }

    #[test]
    fn build_metrics_and_cpu_rate() {
        let mut st = ClusterMetricState::default();
        // First build: no prior scrape → no cpu rate yet, mem summed.
        let text = "redpanda_cpu_busy_seconds_total{shard=\"0\"} 0\n\
                    redpanda_memory_allocated_memory{shard=\"0\"} 1000\n\
                    redpanda_memory_available_memory{shard=\"0\"} 3000\n";
        let m = st.build(0, Some(text));
        assert!(m.prometheus_available);
        assert_eq!(m.brokers.len(), 1);
        assert_eq!(m.brokers[0].memory_used_bytes, Some(1000.0));
        assert_eq!(m.brokers[0].memory_total_bytes, Some(4000.0));
        assert!(m.brokers[0].cpu_percent.is_none());

        // Manually advance the prior-scrape timestamp by 1s and add 0.5 cpu-sec
        // → 50% utilization.
        st.last_cpu
            .insert("broker".into(), (st.last_total.unwrap().0 - 1000, 0.0));
        let text2 = "redpanda_cpu_busy_seconds_total{shard=\"0\"} 0.5\n";
        let m2 = st.build(100, Some(text2));
        assert!(m2.brokers[0].cpu_percent.unwrap() > 49.0);
    }

    #[test]
    fn throughput_rate() {
        let mut st = ClusterMetricState::default();
        let _ = st.build(0, None);
        // Force a 1s gap.
        st.last_total = Some((st.last_total.unwrap().0 - 1000, 0));
        let m = st.build(100, None);
        assert!(!m.prometheus_available);
        assert!(m.messages_per_sec > 99.0 && m.messages_per_sec < 101.0);
    }
}

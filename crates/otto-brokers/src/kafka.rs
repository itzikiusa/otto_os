//! Kafka driver over `rdkafka` (librdkafka). Wraps an `AdminClient`, a base
//! `BaseConsumer` (metadata / watermarks / groups) and a `FutureProducer`.
//!
//! Consumer-only operations are **synchronous** librdkafka C calls and are meant
//! to be run on a blocking thread by the service (`spawn_blocking`). Admin and
//! producer operations are **async** (driven by librdkafka's background poll
//! threads) and are awaited directly. Consuming returns RAW bytes — decoding
//! (incl. async schema-registry Avro) is the service's job.

use crate::types::{
    BrokerNode, ClusterOverview, ConfigKv, ConsumeReq, CreateTopicReq, GroupDetail, GroupMember,
    GroupOffset, GroupSummary, OffsetResetMode, PartitionInfo, PartitionRange, ProduceReq,
    ProduceResp, SaslMechanism, SecurityProtocol, StartPosition, TestClusterResp, TopicConfigEntry,
    TopicPartition, TopicSummary,
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::{Error, Result};
use rdkafka::admin::{
    AdminClient, AdminOptions, AlterConfig, ConfigSource, NewTopic, ResourceSpecifier,
    TopicReplication,
};
use rdkafka::client::{ClientContext, DefaultClientContext};
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer, ConsumerContext};
use rdkafka::error::{KafkaError, RDKafkaErrorCode};
use rdkafka::message::{Header, Headers, Message, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::topic_partition_list::{Offset, TopicPartitionList};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const META_TIMEOUT: Duration = Duration::from_secs(15);
const WATERMARK_TIMEOUT: Duration = Duration::from_secs(8);
const GROUP_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum raw messages scanned when a key_filter is active (prevents infinite
/// loops on sparse matches; the caller's `limit` still caps *matching* results).
const MAX_SCAN_WITH_FILTER: usize = 50_000;
/// Parallelism for the cluster-wide watermark scan (throughput total). librdkafka
/// is thread-safe, so we fan the per-partition ListOffsets queries across worker
/// threads — a few hundred partitions over a slow/tunnelled link complete in a
/// couple of round-trip batches instead of one-by-one (minutes).
pub const WATERMARK_WORKERS: usize = 16;
/// Raw key+value byte budget for one viewer peek (the response is ~1.3–2×
/// this once decoded/escaped). Hitting it ends the peek early with
/// `truncated = true`. Replay reads are unbounded (they must copy everything).
pub const MAX_CONSUME_BYTES: usize = 16 * 1024 * 1024;

fn kerr(e: KafkaError) -> Error {
    Error::Upstream(format!("kafka: {e}"))
}

/// Returned as `Error::Forbidden` when the broker's ACLs deny this principal
/// consumer-group access. The service negative-caches on it (so it stops
/// probing) and the UI shows a "grant DescribeGroup" banner instead of an error.
pub const GROUP_ACL_DENIED: &str = "consumer-group access denied by the broker's ACLs — grant DescribeGroup to view consumer lag and connected consumers";

/// True when a librdkafka error is a consumer-group authorization denial. The
/// `Result` of `fetch_group_list` carries this code when `FindCoordinator` is
/// refused, so we can map it to a clear, cacheable `Forbidden` rather than a
/// generic upstream error.
fn group_auth_denied(e: &KafkaError) -> bool {
    e.rdkafka_error_code() == Some(RDKafkaErrorCode::GroupAuthorizationFailed)
}

/// Map a group-listing error: a group-authorization denial → a distinguishable
/// `Forbidden`, anything else → the generic upstream mapping.
fn group_err(e: KafkaError) -> Error {
    if group_auth_denied(&e) {
        Error::Forbidden(GROUP_ACL_DENIED.to_string())
    } else {
        kerr(e)
    }
}

/// True for internal/system topics (hidden by default, like Conduktor).
pub fn is_internal(name: &str) -> bool {
    name.starts_with("__") || name == "_schemas" || name.starts_with("_redpanda")
}

/// Connection parameters resolved from a cluster profile + Keychain secrets.
#[derive(Clone)]
pub struct KafkaConnSpec {
    pub bootstrap_servers: String,
    pub security_protocol: SecurityProtocol,
    pub sasl_mechanism: Option<SaslMechanism>,
    pub sasl_username: Option<String>,
    pub sasl_password: Option<String>,
    pub tls_skip_verify: bool,
}

/// A raw consumed record (bytes not yet decoded).
pub struct RawMessage {
    pub partition: i32,
    pub offset: i64,
    pub timestamp_ms: Option<i64>,
    pub key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
    pub headers: Vec<(String, Vec<u8>)>,
    pub size: usize,
}

pub struct RawConsume {
    pub messages: Vec<RawMessage>,
    pub partitions: Vec<PartitionRange>,
    pub truncated: bool,
}

pub struct KafkaClient {
    admin: AdminClient<DefaultClientContext>,
    consumer: BaseConsumer<QuietContext>,
    producer: FutureProducer,
    base_config: ClientConfig,
    /// Idle pooled peek consumer (manual assignment, never commits). A peek
    /// takes it (or builds a fresh one when another peek holds it) and puts it
    /// back unassigned — one client per cluster instead of a new
    /// bootstrap/TLS/SASL handshake on every peek or live-tail tick.
    peek_pool: std::sync::Mutex<Option<BaseConsumer<QuietContext>>>,
}

/// A leased peek consumer; unassigns and returns it to the pool on drop.
struct PeekLease<'a> {
    pool: &'a std::sync::Mutex<Option<BaseConsumer<QuietContext>>>,
    consumer: Option<BaseConsumer<QuietContext>>,
}

impl PeekLease<'_> {
    fn consumer(&self) -> &BaseConsumer<QuietContext> {
        self.consumer
            .as_ref()
            .expect("lease holds a consumer until drop")
    }
}

impl Drop for PeekLease<'_> {
    fn drop(&mut self) {
        let Some(c) = self.consumer.take() else {
            return;
        };
        // Only a cleanly unassigned consumer goes back; otherwise drop it.
        if c.unassign().is_ok() {
            let mut slot = self.pool.lock().unwrap_or_else(|p| p.into_inner());
            if slot.is_none() {
                *slot = Some(c);
            }
        }
    }
}

/// Split a "Latest N" peek over partitions (`(partition, available)`), water-
/// filling so partitions with fewer messages than their share hand the rest to
/// the others. The takes sum to `min(n, Σ available)`.
fn latest_split(parts: &[(i32, i64)], n: i64) -> HashMap<i32, i64> {
    let mut order: Vec<(i32, i64)> = parts.to_vec();
    order.sort_by_key(|&(_, avail)| avail);
    let mut remaining = n.max(0);
    let mut out = HashMap::with_capacity(order.len());
    let len = order.len() as i64;
    for (i, (p, avail)) in order.into_iter().enumerate() {
        let left = len - i as i64;
        let share = (remaining + left - 1) / left; // ceil
        let take = avail.min(share).max(0);
        remaining -= take;
        out.insert(p, take);
    }
    out
}

fn build_config(spec: &KafkaConnSpec) -> ClientConfig {
    let mut c = ClientConfig::new();
    c.set("bootstrap.servers", &spec.bootstrap_servers);
    c.set("security.protocol", spec.security_protocol.librdkafka());
    c.set("client.id", "otto-brokers");
    c.set("socket.timeout.ms", "10000");
    c.set("broker.address.family", "v4");
    if spec.security_protocol.uses_tls() && spec.tls_skip_verify {
        c.set("enable.ssl.certificate.verification", "false");
    }
    if spec.security_protocol.uses_sasl() {
        c.set(
            "sasl.mechanism",
            spec.sasl_mechanism.unwrap_or_default().librdkafka(),
        );
        if let Some(u) = &spec.sasl_username {
            c.set("sasl.username", u);
        }
        if let Some(p) = &spec.sasl_password {
            c.set("sasl.password", p);
        }
    }
    c
}

/// Consumer context that downgrades two noisy-but-expected librdkafka global
/// errors so they don't spam the daemon log at ERROR. `PartitionEOF` is the
/// normal "reached end of partition" signal (emitted when
/// `enable.partition.eof=true`; peek uses it to know when to stop) — logged at
/// trace. `GroupAuthorizationFailed` means the broker's ACLs deny this principal
/// consumer-group access; browsing/peeking still works (manual partition
/// assignment needs no group), so it is incidental noise on the peek path
/// (logged at debug), while the group-only features surface the failure in the
/// UI. All other client errors keep their normal ERROR level.
#[derive(Clone)]
struct QuietContext;

impl ClientContext for QuietContext {
    fn error(&self, error: KafkaError, reason: &str) {
        match error.rdkafka_error_code() {
            Some(RDKafkaErrorCode::PartitionEOF) => {
                tracing::trace!("kafka: end of partition ({reason})");
            }
            Some(RDKafkaErrorCode::GroupAuthorizationFailed) => {
                tracing::debug!("kafka: consumer-group operation denied by broker ACLs ({reason})");
            }
            _ if matches!(error, KafkaError::PartitionEOF(_)) => {
                tracing::trace!("kafka: end of partition ({reason})");
            }
            _ => tracing::error!("kafka client error: {error} ({reason})"),
        }
    }
}

impl ConsumerContext for QuietContext {}

impl KafkaClient {
    pub fn connect(spec: &KafkaConnSpec) -> Result<Self> {
        let base = build_config(spec);
        let admin: AdminClient<DefaultClientContext> = base.create().map_err(kerr)?;
        let producer: FutureProducer = base.create().map_err(kerr)?;
        let mut cc = base.clone();
        cc.set("group.id", "otto-brokers-meta");
        cc.set("enable.auto.commit", "false");
        let consumer: BaseConsumer<QuietContext> =
            cc.create_with_context(QuietContext).map_err(kerr)?;
        Ok(Self {
            admin,
            consumer,
            producer,
            base_config: base,
            peek_pool: std::sync::Mutex::new(None),
        })
    }

    /// Lease the pooled peek consumer, or build a fresh one if it's in use.
    fn peek_lease(&self) -> Result<PeekLease<'_>> {
        let pooled = self
            .peek_pool
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let consumer = match pooled {
            Some(c) => c,
            None => {
                let mut cfg = self.base_config.clone();
                cfg.set("group.id", format!("otto-peek-{}", peek_suffix("pool")));
                cfg.set("enable.auto.commit", "false");
                cfg.set("enable.partition.eof", "true");
                // Bound librdkafka's prefetch: a peek returns ≤ MAX_CONSUME_BYTES,
                // but the defaults queue up to 64 MB per partition.
                cfg.set("queued.max.messages.kbytes", "8192");
                cfg.set("queued.min.messages", "1000");
                cfg.set("fetch.max.bytes", "8388608");
                cfg.set("max.partition.fetch.bytes", "1048576");
                cfg.create_with_context(QuietContext).map_err(kerr)?
            }
        };
        Ok(PeekLease {
            pool: &self.peek_pool,
            consumer: Some(consumer),
        })
    }

    // ---- sync (consumer) ops — run via spawn_blocking ---------------------

    pub fn test(&self) -> Result<TestClusterResp> {
        let start = Instant::now();
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let n = md.brokers().len();
        Ok(TestClusterResp {
            ok: true,
            latency_ms: start.elapsed().as_millis() as u64,
            message: format!("connected — {n} broker(s)"),
            broker_count: n,
        })
    }

    pub fn overview(&self) -> Result<ClusterOverview> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let cluster_id = self.consumer.client().fetch_cluster_id(META_TIMEOUT);

        let mut leaders: HashMap<i32, usize> = HashMap::new();
        let (mut topic_count, mut internal, mut partition_count) = (0usize, 0usize, 0usize);
        let mut under_replicated = 0usize;
        for t in md.topics() {
            topic_count += 1;
            if is_internal(t.name()) {
                internal += 1;
            }
            for p in t.partitions() {
                partition_count += 1;
                *leaders.entry(p.leader()).or_default() += 1;
                if p.isr().len() < p.replicas().len() {
                    under_replicated += 1;
                }
            }
        }
        let mut brokers: Vec<BrokerNode> = md
            .brokers()
            .iter()
            .map(|b| BrokerNode {
                id: b.id(),
                host: b.host().to_string(),
                port: b.port(),
                rack: None,
                is_controller: false,
                partition_leaders: leaders.get(&b.id()).copied().unwrap_or(0),
            })
            .collect();
        brokers.sort_by_key(|b| b.id);

        // Leadership-imbalance: coefficient of variation of leader counts.
        let leadership_imbalance = if brokers.len() >= 2 {
            let counts: Vec<f64> = brokers.iter().map(|b| b.partition_leaders as f64).collect();
            let mean = counts.iter().sum::<f64>() / counts.len() as f64;
            if mean > 0.0 {
                let variance =
                    counts.iter().map(|&c| (c - mean).powi(2)).sum::<f64>() / counts.len() as f64;
                Some((variance.sqrt() / mean * 100.0).round() / 100.0)
            } else {
                None
            }
        } else {
            None
        };

        let groups = self
            .consumer
            .fetch_group_list(None, GROUP_TIMEOUT)
            .map(|g| g.groups().len())
            .unwrap_or(0);

        Ok(ClusterOverview {
            cluster_id,
            controller_id: -1,
            brokers,
            topic_count,
            internal_topic_count: internal,
            partition_count,
            consumer_group_count: groups,
            under_replicated_partitions: Some(under_replicated),
            leadership_imbalance,
        })
    }

    /// List topics from a single metadata pass. **No per-partition watermark
    /// fetch** — on a large cluster (or over an SSH tunnel) that is hundreds of
    /// blocking round-trips and makes the topic list take minutes. `message_count`
    /// is therefore `-1` ("not computed"); the exact per-partition counts are
    /// loaded lazily by `topic_partitions` when a topic is opened.
    pub fn list_topics(&self) -> Result<Vec<TopicSummary>> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let mut out = Vec::with_capacity(md.topics().len());
        for t in md.topics() {
            let rf = t
                .partitions()
                .iter()
                .map(|p| p.replicas().len())
                .max()
                .unwrap_or(0);
            out.push(TopicSummary {
                name: t.name().to_string(),
                partitions: t.partitions().len(),
                replication_factor: rf,
                message_count: -1, // lazy: computed on topic open
                cleanup_policy: None,
                internal: is_internal(t.name()),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// `(low, high)` watermarks for many partitions in TWO batched ListOffsets
    /// passes (`offsets_for_times` with the special timestamps `-2` earliest /
    /// `-1` latest — the same queries `fetch_watermarks` sends, but grouped per
    /// leader broker by librdkafka). A 12k-partition cluster costs ~2 × brokers
    /// requests instead of ~24k serial round-trips. Partitions the batch could
    /// not resolve (leaderless, per-partition error, or the whole batch failing)
    /// fall back to the per-partition fan-out. Missing entries = unreachable.
    pub fn batch_watermarks(&self, parts: &[(&str, i32)]) -> HashMap<(String, i32), (i64, i64)> {
        let mut out: HashMap<(String, i32), (i64, i64)> = HashMap::with_capacity(parts.len());
        if parts.is_empty() {
            return out;
        }
        let query = |which: Offset| -> HashMap<(String, i32), i64> {
            let mut tpl = TopicPartitionList::with_capacity(parts.len());
            for &(t, p) in parts {
                if tpl.add_partition_offset(t, p, which).is_err() {
                    return HashMap::new();
                }
            }
            match self.consumer.offsets_for_times(tpl, WATERMARK_TIMEOUT) {
                Ok(res) => res
                    .elements()
                    .iter()
                    .filter(|e| e.error().is_ok())
                    .filter_map(|e| match e.offset() {
                        Offset::Offset(o) if o >= 0 => {
                            Some(((e.topic().to_string(), e.partition()), o))
                        }
                        _ => None,
                    })
                    .collect(),
                Err(_) => HashMap::new(),
            }
        };
        // Both passes in parallel (each is already batched per leader).
        let (lows, highs) = std::thread::scope(|s| {
            let lo = s.spawn(|| query(Offset::Beginning));
            let hi = query(Offset::End);
            (lo.join().unwrap_or_default(), hi)
        });
        let mut missing: Vec<(&str, i32)> = Vec::new();
        for &(t, p) in parts {
            let k = (t.to_string(), p);
            match (lows.get(&k), highs.get(&k)) {
                (Some(&lo), Some(&hi)) => {
                    out.insert(k, (lo, hi));
                }
                _ => missing.push((t, p)),
            }
        }
        if !missing.is_empty() {
            for (k, v) in self.fanout_watermarks(&missing) {
                out.insert(k, v);
            }
        }
        out
    }

    /// Per-partition `fetch_watermarks` fanned across `WATERMARK_WORKERS`
    /// threads — the fallback for partitions the batched pass didn't resolve.
    fn fanout_watermarks(&self, parts: &[(&str, i32)]) -> Vec<((String, i32), (i64, i64))> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let results = std::sync::Mutex::new(Vec::with_capacity(parts.len()));
        let next = AtomicUsize::new(0);
        let workers = WATERMARK_WORKERS.min(parts.len()).max(1);
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(topic, partition)) = parts.get(i) else {
                        break;
                    };
                    if let Ok(w) =
                        self.consumer
                            .fetch_watermarks(topic, partition, WATERMARK_TIMEOUT)
                    {
                        results
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(((topic.to_string(), partition), w));
                    }
                });
            }
        });
        results.into_inner().unwrap_or_else(|p| p.into_inner())
    }

    /// Total message count across all non-internal partitions (drives the
    /// throughput sampler). One metadata pass, then ONE batched watermark pass
    /// (see [`Self::batch_watermarks`]).
    pub fn total_messages(&self) -> Result<i64> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let targets: Vec<(&str, i32)> = md
            .topics()
            .iter()
            .filter(|t| !is_internal(t.name()))
            .flat_map(|t| t.partitions().iter().map(move |p| (t.name(), p.id())))
            .collect();
        if targets.is_empty() {
            return Ok(0);
        }
        Ok(self
            .batch_watermarks(&targets)
            .values()
            .map(|&(low, high)| (high - low).max(0))
            .sum())
    }

    /// Message counts for many topics: one metadata pass plus one batched
    /// watermark pass for all their partitions (the Topics tab's 50-row page).
    /// A topic missing from metadata, or with no resolvable partition, maps to
    /// `-1` ("count unavailable").
    pub fn topics_message_counts(&self, topics: &[String]) -> Result<HashMap<String, i64>> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let wanted: HashSet<&str> = topics.iter().map(String::as_str).collect();
        let targets: Vec<(&str, i32)> = md
            .topics()
            .iter()
            .filter(|t| wanted.contains(t.name()))
            .flat_map(|t| t.partitions().iter().map(move |p| (t.name(), p.id())))
            .collect();
        let wm = self.batch_watermarks(&targets);
        let mut out: HashMap<String, i64> = topics.iter().map(|t| (t.clone(), -1)).collect();
        for ((t, _), (low, high)) in wm {
            let e = out.entry(t).or_insert(-1);
            *e = (*e).max(0) + (high - low).max(0);
        }
        Ok(out)
    }

    /// Cheap message count for a single topic (sum of `high-low` watermarks
    /// across its partitions) — drives the lazily-filled "Count" column.
    pub fn topic_message_count(&self, topic: &str) -> Result<i64> {
        let md = self
            .consumer
            .fetch_metadata(Some(topic), META_TIMEOUT)
            .map_err(kerr)?;
        let mt = md
            .topics()
            .iter()
            .find(|t| t.name() == topic)
            .ok_or_else(|| Error::NotFound(format!("topic {topic}")))?;
        let targets: Vec<(&str, i32)> = mt.partitions().iter().map(|p| (topic, p.id())).collect();
        Ok(self
            .batch_watermarks(&targets)
            .values()
            .map(|&(low, high)| (high - low).max(0))
            .sum())
    }

    /// Partitions + watermarks for a topic (the async `topic_configs` is fetched
    /// separately and merged by the service).
    pub fn topic_partitions(&self, topic: &str) -> Result<(Vec<PartitionInfo>, i64)> {
        let md = self
            .consumer
            .fetch_metadata(Some(topic), META_TIMEOUT)
            .map_err(kerr)?;
        let mt = md
            .topics()
            .iter()
            .find(|t| t.name() == topic)
            .ok_or_else(|| Error::NotFound(format!("topic {topic}")))?;
        if mt.partitions().is_empty() {
            return Err(Error::NotFound(format!("topic {topic}")));
        }
        let targets: Vec<(&str, i32)> = mt.partitions().iter().map(|p| (topic, p.id())).collect();
        let wm = self.batch_watermarks(&targets);
        let mut partitions = Vec::new();
        let mut total = 0i64;
        for p in mt.partitions() {
            let (low, high) = wm
                .get(&(topic.to_string(), p.id()))
                .copied()
                .unwrap_or((0, 0));
            let count = (high - low).max(0);
            total += count;
            partitions.push(PartitionInfo {
                id: p.id(),
                leader: p.leader(),
                replicas: p.replicas().to_vec(),
                isr: p.isr().to_vec(),
                low,
                high,
                message_count: count,
            });
        }
        partitions.sort_by_key(|p| p.id);
        Ok((partitions, total))
    }

    /// Peek raw messages with the pooled, manually-assigned consumer; never
    /// commits. Unbounded bytes (replay produces everything it reads).
    pub fn consume_raw(&self, topic: &str, req: &ConsumeReq) -> Result<RawConsume> {
        self.consume_raw_from(topic, req, None, None)
    }

    /// [`Self::consume_raw`] with viewer options: `starts` consumes ONLY those
    /// partitions, each from its own offset (the live tail's single request),
    /// and `byte_budget` ends the peek early (`truncated`) once the raw
    /// key+value bytes reach it.
    pub fn consume_raw_from(
        &self,
        topic: &str,
        req: &ConsumeReq,
        starts: Option<&HashMap<i32, i64>>,
        byte_budget: Option<usize>,
    ) -> Result<RawConsume> {
        let md = self
            .consumer
            .fetch_metadata(Some(topic), META_TIMEOUT)
            .map_err(kerr)?;
        let mt = md
            .topics()
            .iter()
            .find(|t| t.name() == topic)
            .ok_or_else(|| Error::NotFound(format!("topic {topic}")))?;
        let all: Vec<i32> = mt.partitions().iter().map(|p| p.id()).collect();
        let parts: Vec<i32> = match (starts, req.partition) {
            (Some(st), _) => {
                let mut v: Vec<i32> = all.iter().copied().filter(|p| st.contains_key(p)).collect();
                v.sort_unstable();
                v
            }
            (None, Some(p)) if all.contains(&p) => vec![p],
            (None, Some(p)) => return Err(Error::Invalid(format!("partition {p} not in topic"))),
            (None, None) => all,
        };
        if parts.is_empty() {
            return Ok(RawConsume {
                messages: Vec::new(),
                partitions: Vec::new(),
                truncated: false,
            });
        }
        let limit = req.limit.clamp(1, 5000);

        // Pooled peek consumer (one bootstrap/TLS/SASL handshake per cluster,
        // not per peek); returned to the pool on every exit path by the guard.
        let lease = self.peek_lease()?;
        let consumer = lease.consumer();

        // Watermarks per partition — one batched pass on the metadata consumer.
        let targets: Vec<(&str, i32)> = parts.iter().map(|&p| (topic, p)).collect();
        let wm = self.batch_watermarks(&targets);
        let mut ranges = Vec::new();
        let mut high_of: HashMap<i32, i64> = HashMap::new();
        let mut low_of: HashMap<i32, i64> = HashMap::new();
        for &p in &parts {
            let (low, high) = wm.get(&(topic.to_string(), p)).copied().unwrap_or((0, 0));
            ranges.push(PartitionRange {
                partition: p,
                low,
                high,
            });
            high_of.insert(p, high);
            low_of.insert(p, low);
        }

        // Resolve timestamp starts up front.
        let ts_starts: HashMap<i32, i64> = match req.start {
            StartPosition::Timestamp { timestamp_ms } => {
                let mut q = TopicPartitionList::new();
                for &p in &parts {
                    q.add_partition_offset(topic, p, Offset::Offset(timestamp_ms))
                        .map_err(kerr)?;
                }
                let resolved = consumer
                    .offsets_for_times(q, WATERMARK_TIMEOUT)
                    .map_err(kerr)?;
                resolved
                    .elements()
                    .iter()
                    .map(|e| {
                        let off = match e.offset() {
                            Offset::Offset(o) => o,
                            Offset::End => *high_of.get(&e.partition()).unwrap_or(&0),
                            _ => *low_of.get(&e.partition()).unwrap_or(&0),
                        };
                        (e.partition(), off)
                    })
                    .collect()
            }
            _ => HashMap::new(),
        };

        // When find_from_beginning is requested alongside a key_filter, scan from
        // the earliest offset so the filter can match older messages regardless of
        // the caller's `start` position.
        let effective_start = if req.find_from_beginning && req.key_filter.is_some() {
            StartPosition::Beginning
        } else {
            req.start
        };

        // "Latest N" across partitions: split N over the partitions (water-fill,
        // so short partitions hand their unused share to busy ones). Starting
        // every partition at `high - N` assigned up to N × partitions messages
        // (prefetching ~64 MB+ to return 50) and returned whichever arrived
        // first, not the latest.
        let latest_take = latest_split(
            &parts
                .iter()
                .map(|&p| (p, (high_of[&p] - low_of[&p]).max(0)))
                .collect::<Vec<_>>(),
            limit as i64,
        );

        // Build the assignment with per-partition start offsets.
        let mut tpl = TopicPartitionList::new();
        let mut expected = 0i64;
        for &p in &parts {
            let low = low_of[&p];
            let high = high_of[&p];
            let explicit = starts
                .and_then(|st| st.get(&p))
                .map(|&o| o.clamp(low, high));
            let start = match (explicit, effective_start) {
                (Some(o), _) => o,
                (None, StartPosition::Beginning) => low,
                (None, StartPosition::Latest) => {
                    (high - latest_take.get(&p).copied().unwrap_or(0)).max(low)
                }
                (None, StartPosition::Offset { offset }) => offset.clamp(low, high),
                (None, StartPosition::Timestamp { .. }) => {
                    ts_starts.get(&p).copied().unwrap_or(low)
                }
            };
            expected += (high - start).max(0);
            tpl.add_partition_offset(topic, p, Offset::Offset(start))
                .map_err(kerr)?;
        }
        consumer.assign(&tpl).map_err(kerr)?;

        let mut messages = Vec::new();
        if expected == 0 {
            return Ok(RawConsume {
                messages,
                partitions: ranges,
                truncated: false,
            });
        }

        // Pre-compile the key filter to lowercase once (raw bytes are interpreted
        // as UTF-8 best-effort; non-UTF-8 keys never match the filter).
        let key_filter_lower: Option<String> = req.key_filter.as_ref().map(|f| f.to_lowercase());

        // When a key filter is active the want quota counts only matching messages
        // so we may scan more than `limit` raw messages from the broker. Use a
        // large enough inner cap so we don't spin forever on sparse matches.
        let raw_cap = if key_filter_lower.is_some() {
            expected.min(MAX_SCAN_WITH_FILTER as i64) as usize
        } else {
            (limit as i64).min(expected) as usize
        };
        let want = (limit as i64).min(expected) as usize;
        let deadline = Instant::now() + Duration::from_millis(req.max_wait_ms.unwrap_or(5000));
        let mut done: HashSet<i32> = HashSet::new();
        let mut scanned = 0usize;
        // Response byte budget: stop (and flag `truncated`) once the raw
        // key+value bytes reach it — 5000 × MB-sized payloads is GBs of JSON.
        let mut bytes = 0usize;
        let mut over_budget = false;
        let start_of: HashMap<i32, i64> = tpl
            .elements()
            .iter()
            .filter_map(|e| match e.offset() {
                Offset::Offset(o) => Some((e.partition(), o)),
                _ => None,
            })
            .collect();
        while messages.len() < want
            && scanned < raw_cap
            && Instant::now() < deadline
            && done.len() < parts.len()
        {
            match consumer.poll(Duration::from_millis(250)) {
                Some(Ok(m)) => {
                    let part = m.partition();
                    let offset = m.offset();
                    // Defensive with a pooled consumer: ignore anything that
                    // isn't from this assignment (stale prefetch).
                    if m.topic() != topic || start_of.get(&part).is_none_or(|&s| offset < s) {
                        continue;
                    }
                    scanned += 1;

                    // Server-side key filter: evaluate against raw bytes before
                    // paying the cost of allocation / pushing to results.
                    if let Some(ref filter) = key_filter_lower {
                        let matches = m.key().is_some_and(|k| {
                            String::from_utf8_lossy(k)
                                .to_lowercase()
                                .contains(filter.as_str())
                        });
                        if !matches {
                            if offset + 1 >= high_of.get(&part).copied().unwrap_or(i64::MAX) {
                                done.insert(part);
                            }
                            continue;
                        }
                    }

                    let mut headers = Vec::new();
                    if let Some(hs) = m.headers() {
                        for i in 0..hs.count() {
                            let h = hs.get(i);
                            headers.push((
                                h.key.to_string(),
                                h.value.map(|v| v.to_vec()).unwrap_or_default(),
                            ));
                        }
                    }
                    let key = m.key().map(|k| k.to_vec());
                    let value = m.payload().map(|v| v.to_vec());
                    let size = m.key_len() + m.payload_len();
                    bytes += size;
                    messages.push(RawMessage {
                        partition: part,
                        offset,
                        timestamp_ms: m.timestamp().to_millis(),
                        key,
                        value,
                        headers,
                        size,
                    });
                    if offset + 1 >= high_of.get(&part).copied().unwrap_or(i64::MAX) {
                        done.insert(part);
                    }
                    if byte_budget.is_some_and(|b| bytes >= b) {
                        over_budget = true;
                        break;
                    }
                }
                Some(Err(KafkaError::PartitionEOF(p))) => {
                    done.insert(p);
                }
                Some(Err(_)) => {}
                None => {
                    if done.len() == parts.len() {
                        break;
                    }
                }
            }
        }
        let truncated =
            over_budget || ((messages.len() as i64) < expected && messages.len() >= want);
        messages.sort_by(|a, b| a.partition.cmp(&b.partition).then(a.offset.cmp(&b.offset)));
        Ok(RawConsume {
            messages,
            partitions: ranges,
            truncated,
        })
    }

    pub fn list_groups(&self) -> Result<Vec<GroupSummary>> {
        let list = self
            .consumer
            .fetch_group_list(None, GROUP_TIMEOUT)
            .map_err(group_err)?;
        let mut out: Vec<GroupSummary> = list
            .groups()
            .iter()
            .map(|g| GroupSummary {
                group_id: g.name().to_string(),
                state: g.state().to_string(),
                protocol_type: g.protocol_type().to_string(),
                members: g.members().len(),
            })
            .collect();
        out.sort_by(|a, b| a.group_id.cmp(&b.group_id));
        Ok(out)
    }

    pub fn describe_group(&self, group: &str) -> Result<GroupDetail> {
        let list = self
            .consumer
            .fetch_group_list(Some(group), GROUP_TIMEOUT)
            .map_err(group_err)?;
        let info = list
            .groups()
            .iter()
            .find(|g| g.name() == group)
            .ok_or_else(|| Error::NotFound(format!("group {group}")))?;

        let members: Vec<GroupMember> = info
            .members()
            .iter()
            .map(|m| GroupMember {
                member_id: m.id().to_string(),
                client_id: m.client_id().to_string(),
                host: m.client_host().to_string(),
                assignments: m
                    .assignment()
                    .map(parse_member_assignment)
                    .unwrap_or_default(),
            })
            .collect();

        // Committed offsets for this group across all non-internal partitions.
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        let mut tpl = TopicPartitionList::new();
        for t in md.topics() {
            if is_internal(t.name()) {
                continue;
            }
            for p in t.partitions() {
                let _ = tpl.add_partition(t.name(), p.id());
            }
        }

        let mut grp_cfg = self.base_config.clone();
        grp_cfg.set("group.id", group);
        grp_cfg.set("enable.auto.commit", "false");
        let grp_consumer: BaseConsumer<QuietContext> =
            grp_cfg.create_with_context(QuietContext).map_err(kerr)?;
        let committed = grp_consumer
            .committed_offsets(tpl, GROUP_TIMEOUT)
            .map_err(kerr)?;

        // High watermarks for every committed partition in one batched pass
        // (was one serial ListOffsets pair per partition — ~60 s for a
        // 600-partition group over a tunnel).
        let committed_elems = committed.elements();
        let committed_parts: Vec<(&str, i32)> = committed_elems
            .iter()
            .filter(|e| matches!(e.offset(), Offset::Offset(o) if o >= 0))
            .map(|e| (e.topic(), e.partition()))
            .collect();
        let wm = self.batch_watermarks(&committed_parts);
        let mut offsets = Vec::new();
        let mut total_lag = 0i64;
        for e in committed.elements() {
            let current = match e.offset() {
                Offset::Offset(o) if o >= 0 => o,
                _ => continue,
            };
            let high = wm
                .get(&(e.topic().to_string(), e.partition()))
                .map(|&(_, h)| h)
                .unwrap_or(current);
            let lag = (high - current).max(0);
            total_lag += lag;
            offsets.push(GroupOffset {
                topic: e.topic().to_string(),
                partition: e.partition(),
                current_offset: current,
                high_watermark: high,
                lag,
            });
        }
        offsets.sort_by(|a, b| a.topic.cmp(&b.topic).then(a.partition.cmp(&b.partition)));

        Ok(GroupDetail {
            group_id: group.to_string(),
            state: info.state().to_string(),
            protocol_type: info.protocol_type().to_string(),
            protocol: info.protocol().to_string(),
            members,
            offsets,
            total_lag,
        })
    }

    /// Reset (commit) consumer-group offsets. `positions` maps each topic-partition
    /// to the desired offset (already resolved by the caller from
    /// earliest/latest/explicit/timestamp). The group must exist. This commits
    /// via a fresh consumer client so the existing metadata consumer is untouched.
    ///
    /// # Safety
    /// This is a destructive write: a consumer group that is actively consuming will
    /// have its committed offset overwritten. The caller is responsible for requiring
    /// `guard()` + a typed UI confirm before invoking this.
    pub fn reset_offsets(&self, group: &str, positions: HashMap<(String, i32), i64>) -> Result<()> {
        if positions.is_empty() {
            return Ok(());
        }
        let mut grp_cfg = self.base_config.clone();
        grp_cfg.set("group.id", group);
        grp_cfg.set("enable.auto.commit", "false");
        let consumer: BaseConsumer<QuietContext> =
            grp_cfg.create_with_context(QuietContext).map_err(kerr)?;

        let mut tpl = TopicPartitionList::new();
        for ((topic, partition), offset) in &positions {
            tpl.add_partition_offset(topic, *partition, Offset::Offset(*offset))
                .map_err(kerr)?;
        }
        consumer
            .commit(&tpl, rdkafka::consumer::CommitMode::Sync)
            .map_err(kerr)?;
        Ok(())
    }

    /// Resolve the target offset for a single topic-partition given a reset mode.
    /// Used by `reset_group_offsets` to fan across all group assignments.
    pub fn resolve_offset(
        &self,
        topic: &str,
        partition: i32,
        mode: &OffsetResetMode,
        timestamp_ms: Option<i64>,
    ) -> Result<i64> {
        let (low, high) = self
            .consumer
            .fetch_watermarks(topic, partition, WATERMARK_TIMEOUT)
            .map_err(kerr)?;
        match mode {
            OffsetResetMode::Earliest => Ok(low),
            OffsetResetMode::Latest => Ok(high),
            OffsetResetMode::Offset(o) => Ok((*o).clamp(low, high)),
            OffsetResetMode::Timestamp => {
                let ts = timestamp_ms.unwrap_or(0);
                let mut q = TopicPartitionList::new();
                q.add_partition_offset(topic, partition, Offset::Offset(ts))
                    .map_err(kerr)?;
                let resolved = self
                    .consumer
                    .offsets_for_times(q, WATERMARK_TIMEOUT)
                    .map_err(kerr)?;
                let off = resolved
                    .elements()
                    .first()
                    .map(|e| match e.offset() {
                        Offset::Offset(o) => o,
                        Offset::End => high,
                        _ => low,
                    })
                    .unwrap_or(low);
                Ok(off)
            }
        }
    }

    // ---- async (admin / producer) ops — awaited directly ------------------

    pub async fn create_topic(&self, req: &CreateTopicReq) -> Result<()> {
        let mut nt = NewTopic::new(
            &req.name,
            req.partitions.max(1),
            TopicReplication::Fixed(req.replication_factor.max(1)),
        );
        for kv in &req.configs {
            nt = nt.set(&kv.name, &kv.value);
        }
        let opts = AdminOptions::new().operation_timeout(Some(Duration::from_secs(20)));
        let res = self.admin.create_topics([&nt], &opts).await.map_err(kerr)?;
        for r in res {
            r.map_err(|(name, code)| topic_op_err("create", &name, code))?;
        }
        Ok(())
    }

    pub async fn delete_topic(&self, topic: &str) -> Result<()> {
        let opts = AdminOptions::new().operation_timeout(Some(Duration::from_secs(20)));
        let res = self
            .admin
            .delete_topics(&[topic], &opts)
            .await
            .map_err(kerr)?;
        for r in res {
            r.map_err(|(name, code)| topic_op_err("delete", &name, code))?;
        }
        Ok(())
    }

    pub async fn topic_configs(&self, topic: &str) -> Result<Vec<TopicConfigEntry>> {
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(15)));
        let spec = ResourceSpecifier::Topic(topic);
        let res = self
            .admin
            .describe_configs([&spec], &opts)
            .await
            .map_err(kerr)?;
        let mut out = Vec::new();
        for r in res {
            let cr = r.map_err(|e| Error::Upstream(format!("describe configs: {e:?}")))?;
            for e in &cr.entries {
                out.push(TopicConfigEntry {
                    name: e.name.clone(),
                    value: e.value.clone(),
                    source: format!("{:?}", e.source),
                    is_default: e.is_default,
                    is_sensitive: e.is_sensitive,
                    is_read_only: e.is_read_only,
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// `cleanup.policy` for many topics in ONE DescribeConfigs request (was one
    /// admin round-trip per topic, serially). Topics the broker refused
    /// (e.g. no DESCRIBE_CONFIGS) are simply absent; a whole-request failure is
    /// an empty map — the policy column is best-effort.
    pub async fn topics_cleanup_policy(
        &self,
        topics: &[String],
    ) -> HashMap<String, Option<String>> {
        if topics.is_empty() {
            return HashMap::new();
        }
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(15)));
        let specs: Vec<ResourceSpecifier<'_>> = topics
            .iter()
            .map(|t| ResourceSpecifier::Topic(t.as_str()))
            .collect();
        let Ok(res) = self.admin.describe_configs(specs.iter(), &opts).await else {
            return HashMap::new();
        };
        let mut out = HashMap::with_capacity(topics.len());
        for cr in res.into_iter().flatten() {
            if let rdkafka::admin::OwnedResourceSpecifier::Topic(name) = &cr.specifier {
                let policy = cr
                    .entries
                    .iter()
                    .find(|e| e.name == "cleanup.policy")
                    .and_then(|e| e.value.clone());
                out.insert(name.clone(), policy);
            }
        }
        out
    }

    /// Safe topic config update: merge the requested keys over the existing
    /// dynamic (non-default) overrides so other overrides aren't reset. (0.39's
    /// `alter_configs` is a full replace; there is no incremental variant.)
    pub async fn alter_topic_configs(&self, topic: &str, kvs: &[ConfigKv]) -> Result<()> {
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(15)));
        let spec = ResourceSpecifier::Topic(topic);
        let current = self
            .admin
            .describe_configs([&spec], &opts)
            .await
            .map_err(kerr)?;

        let mut merged: HashMap<String, String> = HashMap::new();
        for cr in current.into_iter().flatten() {
            for e in &cr.entries {
                if matches!(e.source, ConfigSource::DynamicTopic) {
                    if let Some(v) = &e.value {
                        merged.insert(e.name.clone(), v.clone());
                    }
                }
            }
        }
        for kv in kvs {
            merged.insert(kv.name.clone(), kv.value.clone());
        }

        let mut alter = AlterConfig::new(ResourceSpecifier::Topic(topic));
        for (k, v) in &merged {
            alter = alter.set(k, v);
        }
        let res = self
            .admin
            .alter_configs([&alter], &opts)
            .await
            .map_err(kerr)?;
        for r in res {
            r.map_err(|(_, code)| Error::Upstream(format!("alter config: {code:?}")))?;
        }
        Ok(())
    }

    pub async fn produce(&self, topic: &str, req: &ProduceReq) -> Result<ProduceResp> {
        let value: Vec<u8> = if req.value_base64 {
            B64.decode(req.value.as_bytes())
                .map_err(|e| Error::Invalid(format!("value base64: {e}")))?
        } else {
            req.value.clone().into_bytes()
        };
        let key: Option<Vec<u8>> = match &req.key {
            Some(k) if req.key_base64 => Some(
                B64.decode(k.as_bytes())
                    .map_err(|e| Error::Invalid(format!("key base64: {e}")))?,
            ),
            Some(k) => Some(k.clone().into_bytes()),
            None => None,
        };
        let mut owned = OwnedHeaders::new();
        for h in &req.headers {
            owned = owned.insert(Header {
                key: &h.key,
                value: Some(h.value.as_bytes()),
            });
        }

        let mut record: FutureRecord<'_, Vec<u8>, Vec<u8>> = FutureRecord::to(topic);
        record = record.payload(&value);
        if let Some(k) = &key {
            record = record.key(k);
        }
        if let Some(p) = req.partition {
            record = record.partition(p);
        }
        if !req.headers.is_empty() {
            record = record.headers(owned);
        }
        match self.producer.send(record, Duration::from_secs(15)).await {
            Ok(d) => Ok(ProduceResp {
                partition: d.partition,
                offset: d.offset,
            }),
            Err((e, _)) => Err(kerr(e)),
        }
    }
}

fn topic_op_err(op: &str, name: &str, code: rdkafka::error::RDKafkaErrorCode) -> Error {
    use rdkafka::error::RDKafkaErrorCode as C;
    match code {
        C::TopicAlreadyExists => Error::Conflict(format!("topic {name} already exists")),
        C::UnknownTopicOrPartition | C::UnknownTopic => {
            Error::NotFound(format!("topic {name} not found"))
        }
        other => Error::Upstream(format!("{op} topic {name}: {other:?}")),
    }
}

/// Unique suffix for the throwaway peek group id (process-wide counter).
fn peek_suffix(topic: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", topic.len(), SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Parse a Kafka consumer-protocol member Assignment blob into topic-partitions.
/// Best-effort: returns empty on any malformed input.
fn parse_member_assignment(bytes: &[u8]) -> Vec<TopicPartition> {
    let mut out = Vec::new();
    let mut r = ByteReader::new(bytes);
    let _version = match r.i16() {
        Some(v) => v,
        None => return out,
    };
    let topic_count = match r.i32() {
        Some(n) if n >= 0 => n,
        _ => return out,
    };
    for _ in 0..topic_count {
        let Some(topic) = r.kafka_string() else {
            return out;
        };
        let Some(pcount) = r.i32() else {
            return out;
        };
        for _ in 0..pcount.max(0) {
            let Some(p) = r.i32() else {
                return out;
            };
            out.push(TopicPartition {
                topic: topic.clone(),
                partition: p,
            });
        }
    }
    out
}

struct ByteReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }
    fn i16(&mut self) -> Option<i16> {
        self.take(2).map(|b| i16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.take(4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn kafka_string(&mut self) -> Option<String> {
        let len = self.i16()?;
        if len < 0 {
            return Some(String::new());
        }
        let bytes = self.take(len as usize)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_split_water_fills() {
        // Even split when every partition has plenty.
        let m = latest_split(&[(0, 100), (1, 100), (2, 100)], 30);
        assert_eq!((m[&0], m[&1], m[&2]), (10, 10, 10));
        // A short partition hands its unused share to the busy ones.
        let m = latest_split(&[(0, 2), (1, 100), (2, 100)], 30);
        assert_eq!(m[&0], 2);
        assert_eq!(m[&1] + m[&2], 28);
        // Never more than available; the total never exceeds N.
        let m = latest_split(&[(0, 3), (1, 0)], 50);
        assert_eq!((m[&0], m[&1]), (3, 0));
        let m = latest_split(
            &[(0, 1000); 1]
                .iter()
                .copied()
                .chain((1..100).map(|p| (p, 1000)))
                .collect::<Vec<_>>(),
            50,
        );
        assert_eq!(m.values().sum::<i64>(), 50);
    }

    #[test]
    fn internal_topic_detection() {
        assert!(is_internal("__consumer_offsets"));
        assert!(is_internal("_schemas"));
        assert!(!is_internal("orders"));
    }

    #[test]
    fn parse_assignment_blob() {
        // version=0; 1 topic "t" with partitions [0,1]
        let mut b = Vec::new();
        b.extend_from_slice(&0i16.to_be_bytes()); // version
        b.extend_from_slice(&1i32.to_be_bytes()); // topic count
        b.extend_from_slice(&1i16.to_be_bytes()); // topic name len
        b.extend_from_slice(b"t");
        b.extend_from_slice(&2i32.to_be_bytes()); // partition count
        b.extend_from_slice(&0i32.to_be_bytes());
        b.extend_from_slice(&1i32.to_be_bytes());
        let tps = parse_member_assignment(&b);
        assert_eq!(tps.len(), 2);
        assert_eq!(tps[0].topic, "t");
        assert_eq!(tps[1].partition, 1);
    }

    #[test]
    fn parse_assignment_malformed_is_empty() {
        assert!(parse_member_assignment(&[0x00]).is_empty());
    }
}

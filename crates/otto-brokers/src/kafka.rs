//! Kafka driver over `rdkafka` (librdkafka). Wraps an `AdminClient`, a base
//! `BaseConsumer` (metadata / watermarks / groups) and a `FutureProducer` (the
//! admin client and producer are created on first use).
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
    // `rdkafka_error_code()` is None for admin operations, which carry their
    // code directly — and are exactly the topic/config toasts this is for.
    let code = match &e {
        KafkaError::AdminOp(code) => Some(*code),
        other => other.rdkafka_error_code(),
    };
    match code.and_then(kafka_headline) {
        Some(headline) => Error::Upstream(format!("kafka: {headline}")),
        None => Error::Upstream(format!("kafka: {e}")),
    }
}

/// A short, actionable sentence for the librdkafka codes people actually hit
/// (unreachable brokers, ACL/auth denials, timeouts, rejected topic changes) —
/// what a toast shows instead of Rust `Debug` output such as
/// `KafkaError(AdminOp(BrokerTransportFailure))`. `None` = no fixed headline;
/// the caller keeps librdkafka's own description.
fn kafka_headline(code: RDKafkaErrorCode) -> Option<&'static str> {
    use RDKafkaErrorCode as C;
    Some(match code {
        C::BrokerTransportFailure | C::AllBrokersDown | C::Resolve => {
            "can't reach the brokers — check the bootstrap servers, and that the SSH tunnel for this profile is open"
        }
        C::OperationTimedOut | C::RequestTimedOut => {
            "the brokers didn't answer in time — check the network or tunnel, then retry"
        }
        C::Authentication | C::SaslAuthenticationFailed => {
            "sign-in to the brokers failed — check the profile's SASL user, password and mechanism"
        }
        C::TopicAuthorizationFailed => {
            "the broker's ACLs deny this principal access to the topic"
        }
        C::ClusterAuthorizationFailed => {
            "the broker's ACLs deny this principal the cluster operation"
        }
        C::PolicyViolation => "the broker's topic policy rejected this change",
        C::InvalidPartitions => "invalid partition count for this topic",
        C::InvalidReplicationFactor => {
            "invalid replication factor — it can't exceed the number of brokers"
        }
        C::InvalidConfig => "the broker rejected a config value as invalid",
        _ => return None,
    })
}

/// [`kafka_headline`] or librdkafka's own description (`Display`, never `Debug`).
/// librdkafka's `Display` is `InvalidMessage (Broker: Invalid message)` — keep
/// only the human description in the parentheses.
fn kafka_code_text(code: RDKafkaErrorCode) -> String {
    if let Some(h) = kafka_headline(code) {
        return h.to_string();
    }
    let text = code.to_string();
    match text.find(" (") {
        Some(i) if text.ends_with(')') => text[i + 2..text.len() - 1].to_string(),
        _ => text,
    }
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
    pub headers: Vec<(String, Option<Vec<u8>>)>,
    pub size: usize,
}

pub struct RawConsume {
    pub messages: Vec<RawMessage>,
    pub partitions: Vec<PartitionRange>,
    pub truncated: bool,
}

pub struct KafkaClient {
    /// Created on the first admin op / produce (see [`lazy_client`]): each
    /// librdkafka handle owns its own broker threads and connections, and a
    /// browse-only session never needs either.
    admin: std::sync::OnceLock<AdminClient<DefaultClientContext>>,
    consumer: BaseConsumer<QuietContext>,
    producer: std::sync::OnceLock<FutureProducer>,
    /// Serializes the lazy creation above (so two first calls racing never
    /// build — and then drop — a second handle).
    lazy_init: std::sync::Mutex<()>,
    base_config: ClientConfig,
    /// Idle pooled peek consumer (manual assignment, never commits). A peek
    /// takes it (or builds a fresh one when another peek holds it) and puts it
    /// back unassigned — one client per cluster instead of a new
    /// bootstrap/TLS/SASL handshake on every peek or live-tail tick.
    peek_pool: std::sync::Mutex<Option<BaseConsumer<QuietContext>>>,
    /// Idle consumers carrying a `group.id` (committed-offset reads and
    /// offset resets need one), most recently used last, at most
    /// [`GROUP_POOL_MAX`]. Clicking through a group's describe → dry-run →
    /// reset → re-describe reuses one client instead of a fresh
    /// bootstrap/TLS/SASL handshake per call (SC-10). Never subscribed, so
    /// holding one never joins (or rebalances) the real group.
    group_pool: std::sync::Mutex<Vec<(String, BaseConsumer<QuietContext>)>>,
    /// `topic → partition ids`, so the Topics tab's 5 s count refresh maps its
    /// ~50 visible names to partitions without an all-topics metadata pass
    /// (that response is the whole cluster — MBs on a 10k-partition cluster).
    partition_cache: std::sync::Mutex<PartitionCache>,
}

/// Idle group-scoped consumers kept per cluster (each is a librdkafka client
/// with its own broker threads, so the pool stays small).
const GROUP_POOL_MAX: usize = 4;

/// How long a cached `topic → partition ids` entry is trusted. Partitions only
/// change by an explicit admin op (create/delete here, or an `kafka-topics
/// --alter` elsewhere), so a minute of staleness costs at most a missing new
/// partition in a count — never a wrong offset (watermarks are always live).
const PARTITION_CACHE_TTL: Duration = Duration::from_secs(60);

/// Above this many uncached topics one all-topics metadata pass (which also
/// refills the cache) beats that many serial single-topic round-trips.
const PARTITION_MISS_FANOUT_MAX: usize = 4;

/// Per-cluster `topic → partition ids` cache (see [`PARTITION_CACHE_TTL`]).
/// A topic absent from metadata is cached as an empty list, so a deleted
/// topic still on screen doesn't force a refetch on every refresh.
#[derive(Default)]
struct PartitionCache {
    entries: HashMap<String, (Instant, Vec<i32>)>,
}

impl PartitionCache {
    fn put(&mut self, topic: &str, ids: Vec<i32>, now: Instant) {
        self.entries.insert(topic.to_string(), (now, ids));
    }

    fn invalidate(&mut self, topic: &str) {
        self.entries.remove(topic);
    }

    /// Split `topics` into watermark targets served from fresh entries and the
    /// topics that need a metadata lookup (stale entries are evicted).
    fn plan(&mut self, topics: &[String], now: Instant) -> (Vec<(String, i32)>, Vec<String>) {
        self.entries
            .retain(|_, (at, _)| now.saturating_duration_since(*at) < PARTITION_CACHE_TTL);
        let (mut targets, mut misses) = (Vec::new(), Vec::new());
        let mut seen = HashSet::new();
        for t in topics.iter().filter(|t| seen.insert(t.as_str())) {
            match self.entries.get(t) {
                Some((_, ids)) => targets.extend(ids.iter().map(|&p| (t.clone(), p))),
                None => misses.push(t.clone()),
            }
        }
        (targets, misses)
    }
}

/// Topic → partition metadata lookups, abstracted so the cache's fetch plan
/// is testable without a broker.
trait TopicMetaSource {
    /// Every topic's partition ids (one all-topics metadata request).
    fn all_partitions(&self) -> Result<Vec<(String, Vec<i32>)>>;
    /// One topic's partition ids; empty when the topic doesn't exist.
    fn topic_partition_ids(&self, topic: &str) -> Result<Vec<i32>>;
}

impl TopicMetaSource for BaseConsumer<QuietContext> {
    fn all_partitions(&self) -> Result<Vec<(String, Vec<i32>)>> {
        let md = self.fetch_metadata(None, META_TIMEOUT).map_err(kerr)?;
        Ok(md
            .topics()
            .iter()
            .map(|t| {
                let ids = t.partitions().iter().map(|p| p.id()).collect();
                (t.name().to_string(), ids)
            })
            .collect())
    }

    fn topic_partition_ids(&self, topic: &str) -> Result<Vec<i32>> {
        let md = self
            .fetch_metadata(Some(topic), META_TIMEOUT)
            .map_err(kerr)?;
        Ok(md
            .topics()
            .iter()
            .find(|t| t.name() == topic)
            .map(|t| t.partitions().iter().map(|p| p.id()).collect())
            .unwrap_or_default())
    }
}

/// Watermark targets for `topics`: fresh cache entries first, then metadata
/// for the misses only — per topic for a handful, one all-topics pass (which
/// refills the whole cache) for more. A per-topic lookup error stops the
/// lookups (an unreachable cluster must not cost N × [`META_TIMEOUT`]); it is
/// returned only when nothing at all resolved, else those topics read `-1`.
fn partition_targets_with<S: TopicMetaSource>(
    src: &S,
    cache: &std::sync::Mutex<PartitionCache>,
    topics: &[String],
) -> Result<Vec<(String, i32)>> {
    let lock = || cache.lock().unwrap_or_else(|p| p.into_inner());
    let (mut targets, misses) = lock().plan(topics, Instant::now());
    if misses.is_empty() {
        return Ok(targets);
    }
    if misses.len() > PARTITION_MISS_FANOUT_MAX {
        let all = src.all_partitions()?;
        let now = Instant::now();
        let mut found: HashMap<String, Vec<i32>> = all.into_iter().collect();
        let mut c = lock();
        for (t, ids) in &found {
            c.put(t, ids.clone(), now);
        }
        for t in misses {
            let ids = found.remove(&t).unwrap_or_default();
            if !c.entries.contains_key(&t) {
                c.put(&t, Vec::new(), now);
            }
            targets.extend(ids.into_iter().map(|p| (t.clone(), p)));
        }
        return Ok(targets);
    }
    for t in misses {
        match src.topic_partition_ids(&t) {
            Ok(ids) => {
                lock().put(&t, ids.clone(), Instant::now());
                targets.extend(ids.into_iter().map(|p| (t.clone(), p)));
            }
            Err(e) if targets.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    Ok(targets)
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

/// The two consumer calls the batched watermark path needs — a seam so the
/// batching/fallback logic runs against a mocked client in unit tests.
trait WatermarkSource: Sync {
    /// One `offsets_for_times` pass (ListOffsets, batched per leader).
    fn offsets_for(
        &self,
        tpl: TopicPartitionList,
    ) -> std::result::Result<TopicPartitionList, KafkaError>;
    /// One partition's `(low, high)` (the per-partition fallback).
    fn watermarks(
        &self,
        topic: &str,
        partition: i32,
    ) -> std::result::Result<(i64, i64), KafkaError>;
}

impl WatermarkSource for BaseConsumer<QuietContext> {
    fn offsets_for(
        &self,
        tpl: TopicPartitionList,
    ) -> std::result::Result<TopicPartitionList, KafkaError> {
        self.offsets_for_times(tpl, WATERMARK_TIMEOUT)
    }
    fn watermarks(
        &self,
        topic: &str,
        partition: i32,
    ) -> std::result::Result<(i64, i64), KafkaError> {
        self.fetch_watermarks(topic, partition, WATERMARK_TIMEOUT)
    }
}

/// See [`KafkaClient::batch_watermarks`].
fn batch_watermarks_with<S: WatermarkSource>(
    src: &S,
    parts: &[(&str, i32)],
) -> HashMap<(String, i32), (i64, i64)> {
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
        match src.offsets_for(tpl) {
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
        for (k, v) in fanout_watermarks_with(src, &missing) {
            out.insert(k, v);
        }
    }
    out
}

/// Per-partition watermarks fanned across `WATERMARK_WORKERS` threads — the
/// fallback for partitions the batched pass didn't resolve.
fn fanout_watermarks_with<S: WatermarkSource>(
    src: &S,
    parts: &[(&str, i32)],
) -> Vec<((String, i32), (i64, i64))> {
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
                if let Ok(w) = src.watermarks(topic, partition) {
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

/// `cell`'s client, built from `cfg` on first use (double-checked under
/// `lock`, so concurrent first calls create exactly one).
fn lazy_client<'a, T: rdkafka::config::FromClientConfig>(
    cell: &'a std::sync::OnceLock<T>,
    lock: &std::sync::Mutex<()>,
    cfg: &ClientConfig,
) -> Result<&'a T> {
    if let Some(c) = cell.get() {
        return Ok(c);
    }
    let _g = lock.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(c) = cell.get() {
        return Ok(c);
    }
    let client: T = cfg.create().map_err(kerr)?;
    Ok(cell.get_or_init(|| client))
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
        let mut cc = base.clone();
        cc.set("group.id", "otto-brokers-meta");
        cc.set("enable.auto.commit", "false");
        let consumer: BaseConsumer<QuietContext> =
            cc.create_with_context(QuietContext).map_err(kerr)?;
        Ok(Self {
            admin: std::sync::OnceLock::new(),
            consumer,
            producer: std::sync::OnceLock::new(),
            lazy_init: std::sync::Mutex::new(()),
            base_config: base,
            peek_pool: std::sync::Mutex::new(None),
            group_pool: std::sync::Mutex::new(Vec::new()),
            partition_cache: std::sync::Mutex::new(PartitionCache::default()),
        })
    }

    /// The admin client, created on first use.
    fn admin(&self) -> Result<&AdminClient<DefaultClientContext>> {
        lazy_client(&self.admin, &self.lazy_init, &self.base_config)
    }

    /// The producer, created on the first produce.
    fn producer(&self) -> Result<&FutureProducer> {
        lazy_client(&self.producer, &self.lazy_init, &self.base_config)
    }

    /// Run `f` with a consumer whose `group.id` is `group`: the pooled one when
    /// idle, else a fresh one; afterwards it goes back to the pool (LRU, at most
    /// [`GROUP_POOL_MAX`]; the evicted client is dropped outside the lock).
    fn with_group_consumer<T>(
        &self,
        group: &str,
        f: impl FnOnce(&BaseConsumer<QuietContext>) -> Result<T>,
    ) -> Result<T> {
        let pooled = {
            let mut pool = self.group_pool.lock().unwrap_or_else(|p| p.into_inner());
            pool.iter()
                .position(|(g, _)| g == group)
                .map(|i| pool.remove(i).1)
        };
        let consumer = match pooled {
            Some(c) => c,
            None => {
                let mut cfg = self.base_config.clone();
                cfg.set("group.id", group);
                cfg.set("enable.auto.commit", "false");
                cfg.create_with_context(QuietContext).map_err(kerr)?
            }
        };
        let out = f(&consumer);
        let evicted = {
            let mut pool = self.group_pool.lock().unwrap_or_else(|p| p.into_inner());
            if pool.iter().any(|(g, _)| g == group) {
                // A concurrent call for the same group returned first.
                Some(consumer)
            } else {
                pool.push((group.to_string(), consumer));
                (pool.len() > GROUP_POOL_MAX).then(|| pool.remove(0).1)
            }
        };
        drop(evicted);
        out
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
                // but the defaults queue up to 64 MB per partition. Both limits
                // are PER PARTITION, so an assignment over a 100-partition topic
                // could buffer 100 × the value — 8 MB / 1000 msgs each meant up
                // to ~800 MB resident for one peek. 1 MB / 100 msgs per partition
                // still keeps a fetch in flight ahead of the reader.
                cfg.set("queued.max.messages.kbytes", "1024");
                cfg.set("queued.min.messages", "100");
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

    /// Refill the partition cache from a metadata response already in hand.
    fn remember_partitions(&self, md: &rdkafka::metadata::Metadata) {
        let now = Instant::now();
        let mut c = self
            .partition_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for t in md.topics() {
            c.put(
                t.name(),
                t.partitions().iter().map(|p| p.id()).collect(),
                now,
            );
        }
    }

    fn forget_partitions(&self, topic: &str) {
        self.partition_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .invalidate(topic);
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
        self.remember_partitions(&md);
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
        batch_watermarks_with(&self.consumer, parts)
    }

    /// Total message count across all non-internal partitions (drives the
    /// throughput sampler). One metadata pass, then ONE batched watermark pass
    /// (see [`Self::batch_watermarks`]).
    pub fn total_messages(&self) -> Result<i64> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        self.remember_partitions(&md);
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

    /// Message counts for many topics: partition ids from the per-cluster cache
    /// (metadata only for misses — see [`partition_targets_with`]) plus one
    /// batched watermark pass for all their partitions (the Topics tab's 50-row
    /// page, refreshed every 5 s). A topic missing from metadata, or with no
    /// resolvable partition, maps to `-1` ("count unavailable").
    pub fn topics_message_counts(&self, topics: &[String]) -> Result<HashMap<String, i64>> {
        let owned = partition_targets_with(&self.consumer, &self.partition_cache, topics)?;
        let targets: Vec<(&str, i32)> = owned.iter().map(|(t, p)| (t.as_str(), *p)).collect();
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
        self.remember_partitions(&md);
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
        self.remember_partitions(&md);
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
        // Partition ids from the per-cluster cache (60 s TTL, invalidated on
        // create/delete): a live-tail tick every 3 s no longer pays a metadata
        // round trip before its ListOffsets batch.
        let all: Vec<i32> =
            partition_targets_with(&self.consumer, &self.partition_cache, &[topic.to_string()])?
                .into_iter()
                .map(|(_, p)| p)
                .collect();
        if all.is_empty() {
            return Err(Error::NotFound(format!("topic {topic}")));
        }
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
                            headers.push((h.key.to_string(), h.value.map(|v| v.to_vec())));
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

        let committed = self.with_group_consumer(group, |c| {
            c.committed_offsets(tpl, GROUP_TIMEOUT).map_err(kerr)
        })?;

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
    /// via the pooled group-scoped client (never the shared metadata consumer).
    ///
    /// # Safety
    /// This is a destructive write: a consumer group that is actively consuming will
    /// have its committed offset overwritten. The caller is responsible for requiring
    /// `guard()` + a typed UI confirm before invoking this.
    pub fn reset_offsets(&self, group: &str, positions: HashMap<(String, i32), i64>) -> Result<()> {
        if positions.is_empty() {
            return Ok(());
        }
        let mut tpl = TopicPartitionList::new();
        for ((topic, partition), offset) in &positions {
            tpl.add_partition_offset(topic, *partition, Offset::Offset(*offset))
                .map_err(kerr)?;
        }
        // The same pooled group client the describe/dry-run just used (a
        // manual commit never subscribes, so it never joins the group).
        self.with_group_consumer(group, |c| {
            c.commit(&tpl, rdkafka::consumer::CommitMode::Sync)
                .map_err(kerr)
        })
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
        let res = self.admin()?.create_topics([&nt], &opts).await;
        // After the op (success or not): a refresh racing it may have cached
        // the topic as absent.
        self.forget_partitions(&req.name);
        let res = res.map_err(kerr)?;
        for r in res {
            r.map_err(|(name, code)| topic_op_err("create", &name, code))?;
        }
        Ok(())
    }

    pub async fn delete_topic(&self, topic: &str) -> Result<()> {
        let opts = AdminOptions::new().operation_timeout(Some(Duration::from_secs(20)));
        let res = self.admin()?.delete_topics(&[topic], &opts).await;
        self.forget_partitions(topic);
        let res = res.map_err(kerr)?;
        for r in res {
            r.map_err(|(name, code)| topic_op_err("delete", &name, code))?;
        }
        Ok(())
    }

    pub async fn topic_configs(&self, topic: &str) -> Result<Vec<TopicConfigEntry>> {
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(15)));
        let spec = ResourceSpecifier::Topic(topic);
        let res = self
            .admin()?
            .describe_configs([&spec], &opts)
            .await
            .map_err(kerr)?;
        let mut out = Vec::new();
        for r in res {
            let cr = r.map_err(|e| {
                Error::Upstream(format!("describe configs: {}", kafka_code_text(e)))
            })?;
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
        let Ok(admin) = self.admin() else {
            return HashMap::new();
        };
        let Ok(res) = admin.describe_configs(specs.iter(), &opts).await else {
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
            .admin()?
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
            .admin()?
            .alter_configs([&alter], &opts)
            .await
            .map_err(kerr)?;
        for r in res {
            r.map_err(|(_, code)| {
                Error::Upstream(format!("alter config: {}", kafka_code_text(code)))
            })?;
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
        let headers: Vec<_> = req
            .headers
            .iter()
            .map(|h| (h.key.clone(), Some(h.value.as_bytes().to_vec())))
            .collect();
        // The public text API spells a tombstone as empty text. Base64 is the
        // escape hatch for a present, zero-byte payload; never collapse these.
        let payload = if !req.value_base64 && value.is_empty() {
            None
        } else {
            Some(value.as_slice())
        };
        self.produce_raw(topic, req.partition, key.as_deref(), payload, &headers)
            .await
    }

    /// Internal replay adapter: no text decoding or nullable-payload coercion.
    pub(crate) async fn produce_raw(
        &self,
        topic: &str,
        partition: Option<i32>,
        key: Option<&[u8]>,
        value: Option<&[u8]>,
        headers: &[(String, Option<Vec<u8>>)],
    ) -> Result<ProduceResp> {
        let mut record: FutureRecord<'_, [u8], [u8]> = FutureRecord::to(topic);
        if let Some(value) = value {
            record = record.payload(value);
        }
        if let Some(key) = key {
            record = record.key(key);
        }
        if let Some(partition) = partition {
            record = record.partition(partition);
        }
        if !headers.is_empty() {
            let mut owned = OwnedHeaders::new();
            for (key, value) in headers {
                owned = owned.insert(Header {
                    key,
                    value: value.as_deref(),
                });
            }
            record = record.headers(owned);
        }
        match self.producer()?.send(record, Duration::from_secs(15)).await {
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
        other => Error::Upstream(format!("{op} topic {name}: {}", kafka_code_text(other))),
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
    #[test]
    fn kafka_errors_read_as_sentences_not_debug() {
        use rdkafka::error::{KafkaError, RDKafkaErrorCode as C};
        let e = super::kerr(KafkaError::AdminOp(C::BrokerTransportFailure));
        let text = e.to_string();
        assert!(text.contains("can't reach the brokers"), "{text}");
        assert!(!text.contains("AdminOp") && !text.contains("BrokerTransportFailure"));
        let e = super::topic_op_err("create", "orders", C::PolicyViolation);
        assert_eq!(
            e.to_string(),
            "upstream: create topic orders: the broker's topic policy rejected this change"
        );
        // No fixed headline → librdkafka's description, still not Debug.
        assert_eq!(
            super::kafka_code_text(C::InvalidMessage),
            "Broker: Invalid message"
        );
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    type Key = (String, i32);

    /// A scripted consumer for the batched watermark path: per-pass offsets
    /// (a partition missing from a map comes back unresolved, as a leaderless
    /// or errored partition does), an optional whole-batch failure, and the
    /// per-partition fallback — with call counters.
    #[derive(Default)]
    struct MockSource {
        lows: HashMap<Key, i64>,
        highs: HashMap<Key, i64>,
        fail_batch: bool,
        fallback: HashMap<Key, (i64, i64)>,
        batch_calls: AtomicUsize,
        fallback_calls: Mutex<Vec<Key>>,
    }

    impl WatermarkSource for MockSource {
        fn offsets_for(
            &self,
            tpl: TopicPartitionList,
        ) -> std::result::Result<TopicPartitionList, KafkaError> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_batch {
                return Err(KafkaError::MetadataFetch(RDKafkaErrorCode::RequestTimedOut));
            }
            let elems = tpl.elements();
            let src = match elems.first().map(|e| e.offset()) {
                Some(Offset::Beginning) => &self.lows,
                Some(Offset::End) => &self.highs,
                other => panic!("unexpected query offset {other:?}"),
            };
            let mut out = TopicPartitionList::new();
            for e in elems {
                let off = src
                    .get(&(e.topic().to_string(), e.partition()))
                    .map_or(Offset::Invalid, |&o| Offset::from_raw(o));
                out.add_partition_offset(e.topic(), e.partition(), off)
                    .unwrap();
            }
            Ok(out)
        }

        fn watermarks(
            &self,
            topic: &str,
            partition: i32,
        ) -> std::result::Result<(i64, i64), KafkaError> {
            let k = (topic.to_string(), partition);
            self.fallback_calls.lock().unwrap().push(k.clone());
            self.fallback
                .get(&k)
                .copied()
                .ok_or(KafkaError::MetadataFetch(
                    RDKafkaErrorCode::LeaderNotAvailable,
                ))
        }
    }

    fn k(t: &str, p: i32) -> Key {
        (t.to_string(), p)
    }

    #[test]
    fn batch_watermarks_resolves_everything_in_two_passes() {
        let mut m = MockSource::default();
        let parts: Vec<(&str, i32)> = (0..600)
            .map(|p| (if p % 2 == 0 { "a" } else { "b" }, p))
            .collect();
        for &(t, p) in &parts {
            m.lows.insert(k(t, p), i64::from(p));
            m.highs.insert(k(t, p), i64::from(p) * 10 + 5);
        }
        let wm = batch_watermarks_with(&m, &parts);
        assert_eq!(wm.len(), 600);
        assert_eq!(wm[&k("a", 4)], (4, 45));
        assert_eq!(wm[&k("b", 599)], (599, 5995));
        // ONE earliest + ONE latest pass for 600 partitions, no per-partition fallback.
        assert_eq!(m.batch_calls.load(Ordering::SeqCst), 2);
        assert!(m.fallback_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn batch_watermarks_falls_back_only_for_unresolved_partitions() {
        let mut m = MockSource::default();
        for p in 0..4 {
            m.lows.insert(k("t", p), 0);
            m.highs.insert(k("t", p), 100);
        }
        // P4: latest unresolved in the batch; P5: earliest unresolved; P6: both
        // unresolved AND unreachable in the fallback (left out = "unreachable").
        m.lows.insert(k("t", 4), 1);
        m.highs.insert(k("t", 5), 9);
        m.fallback.insert(k("t", 4), (1, 44));
        m.fallback.insert(k("t", 5), (2, 55));
        let parts: Vec<(&str, i32)> = (0..7).map(|p| ("t", p)).collect();
        let wm = batch_watermarks_with(&m, &parts);
        assert_eq!(wm[&k("t", 0)], (0, 100));
        assert_eq!(
            wm[&k("t", 4)],
            (1, 44),
            "a half-resolved partition takes the fallback pair"
        );
        assert_eq!(wm[&k("t", 5)], (2, 55));
        assert!(
            !wm.contains_key(&k("t", 6)),
            "unreachable partitions are left out"
        );
        let mut asked = m.fallback_calls.lock().unwrap().clone();
        asked.sort();
        assert_eq!(asked, vec![k("t", 4), k("t", 5), k("t", 6)]);
    }

    #[test]
    fn batch_watermarks_whole_batch_failure_uses_the_fanout() {
        let mut m = MockSource {
            fail_batch: true,
            ..MockSource::default()
        };
        let parts: Vec<(&str, i32)> = (0..40).map(|p| ("t", p)).collect();
        for p in 0..40 {
            m.fallback.insert(k("t", p), (0, i64::from(p)));
        }
        let wm = batch_watermarks_with(&m, &parts);
        assert_eq!(wm.len(), 40);
        assert_eq!(wm[&k("t", 39)], (0, 39));
        assert_eq!(m.fallback_calls.lock().unwrap().len(), 40);
        // Negative "offsets" (the -1/-2 sentinels echoed back) never count as resolved.
        let mut m = MockSource::default();
        m.lows.insert(k("t", 0), -2);
        m.highs.insert(k("t", 0), -1);
        let wm = batch_watermarks_with(&m, &[("t", 0)]);
        assert!(wm.is_empty());
        assert_eq!(m.fallback_calls.lock().unwrap().len(), 1);
        // Nothing asked, nothing sent.
        let m = MockSource::default();
        assert!(batch_watermarks_with(&m, &[]).is_empty());
        assert_eq!(m.batch_calls.load(Ordering::SeqCst), 0);
    }

    fn offline_client() -> KafkaClient {
        KafkaClient::connect(&KafkaConnSpec {
            bootstrap_servers: "127.0.0.1:1".into(),
            security_protocol: SecurityProtocol::Plaintext,
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
            tls_skip_verify: false,
        })
        .expect("librdkafka clients are created lazily (no broker needed)")
    }

    /// N4: connecting builds only the metadata consumer; the admin client and
    /// producer appear on first use, once.
    #[test]
    fn admin_and_producer_are_created_on_first_use() {
        let c = offline_client();
        assert!(c.admin.get().is_none() && c.producer.get().is_none());
        let p1 = c.producer().unwrap() as *const FutureProducer;
        assert!(
            c.admin.get().is_none(),
            "producing must not build the admin"
        );
        assert!(std::ptr::eq(p1, c.producer().unwrap()));
        let a1 = c.admin().unwrap() as *const AdminClient<DefaultClientContext>;
        assert!(std::ptr::eq(a1, c.admin().unwrap()));
    }

    /// Counts metadata requests by kind; `topics` is the cluster.
    #[derive(Default)]
    struct MetaMock {
        topics: Vec<(String, Vec<i32>)>,
        all_calls: AtomicUsize,
        one_calls: AtomicUsize,
        fail: bool,
    }

    impl TopicMetaSource for MetaMock {
        fn all_partitions(&self) -> Result<Vec<(String, Vec<i32>)>> {
            self.all_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.topics.clone())
        }
        fn topic_partition_ids(&self, topic: &str) -> Result<Vec<i32>> {
            self.one_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(Error::Upstream("broker down".into()));
            }
            Ok(self
                .topics
                .iter()
                .find(|(t, _)| t == topic)
                .map(|(_, ids)| ids.clone())
                .unwrap_or_default())
        }
    }

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("t{i}")).collect()
    }

    #[test]
    fn warm_partition_cache_skips_all_topics_metadata() {
        let src = MetaMock {
            topics: (0..60).map(|i| (format!("t{i}"), vec![0, 1, 2])).collect(),
            ..Default::default()
        };
        let cache = std::sync::Mutex::new(PartitionCache::default());
        // Warm it the way `list_topics` does (from metadata already in hand).
        {
            let mut c = cache.lock().unwrap();
            for (t, ids) in &src.topics {
                c.put(t, ids.clone(), Instant::now());
            }
        }
        let page = names(50);
        for _ in 0..3 {
            let targets = partition_targets_with(&src, &cache, &page).unwrap();
            assert_eq!(targets.len(), 150);
        }
        assert_eq!(
            src.all_calls.load(Ordering::SeqCst),
            0,
            "no all-topics fetch"
        );
        assert_eq!(src.one_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn partition_cache_fetches_only_misses() {
        let src = MetaMock {
            topics: vec![("a".into(), vec![0]), ("b".into(), vec![0, 1])],
            ..Default::default()
        };
        let cache = std::sync::Mutex::new(PartitionCache::default());
        cache.lock().unwrap().put("a", vec![0], Instant::now());
        // A few misses → per-topic lookups; a gone topic is negatively cached.
        let want = vec!["a".to_string(), "b".into(), "gone".into()];
        let mut t = partition_targets_with(&src, &cache, &want).unwrap();
        t.sort();
        assert_eq!(t, vec![("a".into(), 0), ("b".into(), 0), ("b".into(), 1)]);
        assert_eq!(src.one_calls.load(Ordering::SeqCst), 2);
        partition_targets_with(&src, &cache, &want).unwrap();
        assert_eq!(src.one_calls.load(Ordering::SeqCst), 2, "now all cached");
        assert_eq!(src.all_calls.load(Ordering::SeqCst), 0);
        // Invalidation (create/delete topic) makes it a miss again.
        cache.lock().unwrap().invalidate("b");
        partition_targets_with(&src, &cache, &want).unwrap();
        assert_eq!(src.one_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn partition_cache_cold_page_uses_one_all_topics_pass_and_expires() {
        let src = MetaMock {
            topics: (0..60).map(|i| (format!("t{i}"), vec![0])).collect(),
            ..Default::default()
        };
        let cache = std::sync::Mutex::new(PartitionCache::default());
        let page = names(50);
        assert_eq!(
            partition_targets_with(&src, &cache, &page).unwrap().len(),
            50
        );
        assert_eq!(src.all_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            src.one_calls.load(Ordering::SeqCst),
            0,
            "no N serial lookups"
        );
        // Past the TTL every entry is stale → planned as a miss again.
        let later = Instant::now() + PARTITION_CACHE_TTL + Duration::from_secs(1);
        let (targets, misses) = cache.lock().unwrap().plan(&page, later);
        assert!(targets.is_empty());
        assert_eq!(misses.len(), 50);
    }

    #[test]
    fn partition_lookup_failure_stops_after_first_error() {
        let src = MetaMock {
            fail: true,
            ..Default::default()
        };
        let cache = std::sync::Mutex::new(PartitionCache::default());
        let few = names(3);
        assert!(partition_targets_with(&src, &cache, &few).is_err());
        assert_eq!(src.one_calls.load(Ordering::SeqCst), 1, "not 3 × timeout");
        // With something resolved from cache, the rest just read "-1".
        cache.lock().unwrap().put("t0", vec![0], Instant::now());
        let t = partition_targets_with(&src, &cache, &few).unwrap();
        assert_eq!(t, vec![("t0".to_string(), 0)]);
    }

    #[test]
    fn pooled_peek_consumer_config_is_accepted() {
        // The lowered per-partition prefetch bounds must still build a client.
        let client = offline_client();
        assert!(client.peek_lease().is_ok());
    }

    #[test]
    fn group_describe_reuses_one_consumer_per_group() {
        let client = offline_client();
        let ptr = |c: &BaseConsumer<QuietContext>| Ok(c.client().native_ptr() as usize);
        let first = client.with_group_consumer("orders", ptr).unwrap();
        let again = client.with_group_consumer("orders", ptr).unwrap();
        assert_eq!(first, again, "describe → reset → describe reuse one client");
        // An error inside still returns the client to the pool.
        let err: Result<()> =
            client.with_group_consumer("orders", |_| Err(Error::Invalid("boom".into())));
        assert!(err.is_err());
        assert_eq!(client.with_group_consumer("orders", ptr).unwrap(), first);
        // Other groups get their own; the pool stays bounded (LRU).
        for g in ["a", "b", "c", "d"] {
            client.with_group_consumer(g, ptr).unwrap();
        }
        let pool = client.group_pool.lock().unwrap();
        assert_eq!(pool.len(), GROUP_POOL_MAX);
        assert!(
            !pool.iter().any(|(g, _)| g == "orders"),
            "least recently used is evicted"
        );
    }

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

    /// An in-process librdkafka mock cluster with request tracking, so a
    /// test can count the Kafka protocol requests a call really sends.
    /// (`rdkafka::mocking::MockCluster` hides its handle, and request
    /// tracking is only reachable through the raw API.)
    struct TrackedMock {
        mock: *mut rdkafka::bindings::rd_kafka_mock_cluster_t,
        /// The handle the mock cluster lives on; dropped after it.
        _owner: rdkafka::producer::BaseProducer,
        bootstrap: String,
    }

    /// Kafka protocol API keys.
    const API_LIST_OFFSETS: i16 = 2;
    const API_METADATA: i16 = 3;

    impl TrackedMock {
        fn new(topics: &[(&str, i32)]) -> Self {
            use rdkafka::bindings as rd;
            use rdkafka::producer::Producer;
            let owner: rdkafka::producer::BaseProducer = ClientConfig::new().create().unwrap();
            // SAFETY: `owner` outlives the mock (destroyed first in `Drop`).
            let mock = unsafe { rd::rd_kafka_mock_cluster_new(owner.client().native_ptr(), 1) };
            assert!(!mock.is_null(), "mock cluster");
            for &(t, parts) in topics {
                let name = std::ffi::CString::new(t).unwrap();
                // SAFETY: valid mock handle and NUL-terminated name.
                let err = unsafe { rd::rd_kafka_mock_topic_create(mock, name.as_ptr(), parts, 1) };
                assert_eq!(err, rd::rd_kafka_resp_err_t::RD_KAFKA_RESP_ERR_NO_ERROR);
            }
            // SAFETY: the returned string is owned by the mock cluster.
            let bootstrap = unsafe {
                std::ffi::CStr::from_ptr(rd::rd_kafka_mock_cluster_bootstraps(mock))
                    .to_string_lossy()
                    .into_owned()
            };
            // SAFETY: valid mock handle.
            unsafe { rd::rd_kafka_mock_start_request_tracking(mock) };
            Self {
                mock,
                _owner: owner,
                bootstrap,
            }
        }

        fn clear(&self) {
            // SAFETY: valid mock handle.
            unsafe { rdkafka::bindings::rd_kafka_mock_clear_requests(self.mock) };
        }

        /// Wait until the clients on this mock stop sending Metadata in the
        /// background, then `clear`. librdkafka fires an async Metadata
        /// request whenever a broker connection comes up (`connect_up` →
        /// `metadata_refresh_known_topics`), and the cold pass / first tail
        /// tick open new connections (bootstrap → the learned broker id, the
        /// fresh peek-pool consumer) — that refresh can land a few ms into
        /// the NEXT measured window. It is not part of the call under test,
        /// so the budget is measured only once the cluster is quiet: a
        /// 300 ms window with no Metadata request (5 s ceiling, after which
        /// the measurement runs anyway and the budget assertion decides).
        fn settle_then_clear(&self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                self.clear();
                std::thread::sleep(Duration::from_millis(300));
                if self.requests(API_METADATA) == 0 || Instant::now() >= deadline {
                    break;
                }
            }
            self.clear();
        }

        /// Requests seen since the last `clear`, by API key.
        fn requests(&self, api_key: i16) -> usize {
            use rdkafka::bindings as rd;
            let mut n = 0usize;
            // SAFETY: the array and its `n` elements are ours to read, then
            // free with `destroy_array`.
            unsafe {
                let arr = rd::rd_kafka_mock_get_requests(self.mock, &mut n);
                if arr.is_null() {
                    return 0;
                }
                let hits = (0..n)
                    .filter(|&i| rd::rd_kafka_mock_request_api_key(*arr.add(i)) == api_key)
                    .count();
                rd::rd_kafka_mock_request_destroy_array(arr, n);
                hits
            }
        }

        fn client(&self) -> KafkaClient {
            KafkaClient::connect(&KafkaConnSpec {
                bootstrap_servers: self.bootstrap.clone(),
                security_protocol: SecurityProtocol::Plaintext,
                sasl_mechanism: None,
                sasl_username: None,
                sasl_password: None,
                tls_skip_verify: false,
            })
            .unwrap()
        }

        fn produce(&self, topic: &str, n: usize) {
            use rdkafka::producer::{BaseRecord, Producer};
            let p: rdkafka::producer::BaseProducer = ClientConfig::new()
                .set("bootstrap.servers", &self.bootstrap)
                .create()
                .unwrap();
            for i in 0..n {
                let key = format!("k{i}");
                p.send(
                    BaseRecord::to(topic)
                        .key(&key)
                        .payload("v")
                        .partition((i % 3) as i32),
                )
                .map_err(|(e, _)| e)
                .unwrap();
            }
            p.flush(Duration::from_secs(10)).unwrap();
        }
    }

    impl Drop for TrackedMock {
        fn drop(&mut self) {
            // SAFETY: created in `new`, destroyed exactly once, before `_owner`.
            unsafe { rdkafka::bindings::rd_kafka_mock_cluster_destroy(self.mock) };
        }
    }

    /// N5 budget, against a real (in-process) Kafka protocol peer: the
    /// Topics tab's warm count refresh sends NO metadata request and one
    /// batched ListOffsets per offset kind (never one per partition), within
    /// a generous wall-clock ceiling; a live-tail tick over a 3-partition
    /// topic is the same single watermark batch and returns the new messages.
    #[test]
    fn mock_cluster_counts_and_tail_tick_request_budget() {
        let topics: Vec<(String, i32)> = (0..6).map(|i| (format!("t{i}"), 3)).collect();
        let refs: Vec<(&str, i32)> = topics.iter().map(|(t, p)| (t.as_str(), *p)).collect();
        let mock = TrackedMock::new(&refs);
        mock.produce("t0", 30);
        let client = mock.client();
        let names: Vec<String> = topics.iter().map(|(t, _)| t.clone()).collect();

        // Cold: one all-topics pass fills the partition cache.
        let cold = client.topics_message_counts(&names).unwrap();
        assert_eq!(cold["t0"], 30, "{cold:?}");

        // Warm: partitions from the cache, watermarks in one batch per kind.
        mock.settle_then_clear();
        let started = Instant::now();
        let warm = client.topics_message_counts(&names).unwrap();
        let took = started.elapsed();
        assert_eq!(warm, cold);
        assert_eq!(
            mock.requests(API_METADATA),
            0,
            "a warm count refresh must not fetch metadata"
        );
        let list_offsets = mock.requests(API_LIST_OFFSETS);
        assert!(
            (1..=2).contains(&list_offsets),
            "18 partitions must cost ≤ 2 batched ListOffsets, saw {list_offsets}"
        );
        assert!(took < Duration::from_secs(2), "warm counts took {took:?}");

        // Live tail: first tick reads the latest page and warms the peek pool.
        let req: ConsumeReq = serde_json::from_value(serde_json::json!({
            "limit": 50, "max_wait_ms": 3000
        }))
        .unwrap();
        let first = client.consume_raw_from("t0", &req, None, None).unwrap();
        assert_eq!(first.messages.len(), 30);
        let starts: HashMap<i32, i64> = first
            .partitions
            .iter()
            .map(|r| (r.partition, r.high))
            .collect();
        mock.produce("t0", 30);

        // Next tick: everything after the previous highs, in one batch.
        mock.settle_then_clear();
        let started = Instant::now();
        let tick = client
            .consume_raw_from("t0", &req, Some(&starts), Some(MAX_CONSUME_BYTES))
            .unwrap();
        let took = started.elapsed();
        assert_eq!(tick.messages.len(), 30, "the tick returns the new messages");
        let list_offsets = mock.requests(API_LIST_OFFSETS);
        assert!(
            (1..=2).contains(&list_offsets),
            "a tail tick is one batched watermark pass, saw {list_offsets} ListOffsets"
        );
        assert!(took < Duration::from_secs(2), "tail tick took {took:?}");
        // The partition list comes from the warm cache, not a metadata request.
        assert_eq!(
            mock.requests(API_METADATA),
            0,
            "a warm tail tick must not fetch topic metadata"
        );
    }
}

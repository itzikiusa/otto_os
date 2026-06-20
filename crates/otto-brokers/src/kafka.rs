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
    ProduceResp, SaslMechanism, SecurityProtocol, StartPosition, TestClusterResp,
    TopicConfigEntry, TopicPartition, TopicSummary,
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
                let variance = counts.iter().map(|&c| (c - mean).powi(2)).sum::<f64>()
                    / counts.len() as f64;
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

    /// Total message count across all non-internal partitions (drives the
    /// throughput sampler). One metadata pass, then the per-partition watermark
    /// queries are fanned across worker threads (sharing the thread-safe
    /// consumer) so a large or slow/tunnelled cluster completes in a couple of
    /// parallel batches rather than hundreds of sequential round-trips.
    pub fn total_messages(&self) -> Result<i64> {
        use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
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
        let total = AtomicI64::new(0);
        let next = AtomicUsize::new(0);
        let workers = WATERMARK_WORKERS.min(targets.len());
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(topic, partition)) = targets.get(i) else {
                        break;
                    };
                    if let Ok((low, high)) =
                        self.consumer
                            .fetch_watermarks(topic, partition, WATERMARK_TIMEOUT)
                    {
                        total.fetch_add((high - low).max(0), Ordering::Relaxed);
                    }
                });
            }
        });
        Ok(total.load(Ordering::Relaxed))
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
        let mut total = 0i64;
        for p in mt.partitions() {
            if let Ok((low, high)) =
                self.consumer
                    .fetch_watermarks(topic, p.id(), WATERMARK_TIMEOUT)
            {
                total += (high - low).max(0);
            }
        }
        Ok(total)
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
        let mut partitions = Vec::new();
        let mut total = 0i64;
        for p in mt.partitions() {
            let (low, high) = self
                .consumer
                .fetch_watermarks(topic, p.id(), WATERMARK_TIMEOUT)
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

    /// Peek raw messages. Creates a throwaway assigned consumer for clean offset
    /// control; never commits.
    pub fn consume_raw(&self, topic: &str, req: &ConsumeReq) -> Result<RawConsume> {
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
        let parts: Vec<i32> = match req.partition {
            Some(p) if all.contains(&p) => vec![p],
            Some(p) => return Err(Error::Invalid(format!("partition {p} not in topic"))),
            None => all,
        };
        let limit = req.limit.clamp(1, 5000);

        // Fresh consumer for an isolated assignment.
        let mut cfg = self.base_config.clone();
        cfg.set("group.id", format!("otto-peek-{}", peek_suffix(topic)));
        cfg.set("enable.auto.commit", "false");
        cfg.set("enable.partition.eof", "true");
        let consumer: BaseConsumer<QuietContext> =
            cfg.create_with_context(QuietContext).map_err(kerr)?;

        // Watermarks per partition.
        let mut ranges = Vec::new();
        let mut high_of: HashMap<i32, i64> = HashMap::new();
        let mut low_of: HashMap<i32, i64> = HashMap::new();
        for &p in &parts {
            let (low, high) = consumer
                .fetch_watermarks(topic, p, WATERMARK_TIMEOUT)
                .unwrap_or((0, 0));
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

        // Build the assignment with per-partition start offsets.
        let mut tpl = TopicPartitionList::new();
        let mut expected = 0i64;
        for &p in &parts {
            let low = low_of[&p];
            let high = high_of[&p];
            let start = match effective_start {
                StartPosition::Beginning => low,
                StartPosition::Latest => (high - limit as i64).max(low),
                StartPosition::Offset { offset } => offset.clamp(low, high),
                StartPosition::Timestamp { .. } => ts_starts.get(&p).copied().unwrap_or(low),
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
        let key_filter_lower: Option<String> =
            req.key_filter.as_ref().map(|f| f.to_lowercase());

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
        while messages.len() < want
            && scanned < raw_cap
            && Instant::now() < deadline
            && done.len() < parts.len()
        {
            match consumer.poll(Duration::from_millis(250)) {
                Some(Ok(m)) => {
                    let part = m.partition();
                    let offset = m.offset();
                    scanned += 1;

                    // Server-side key filter: evaluate against raw bytes before
                    // paying the cost of allocation / pushing to results.
                    if let Some(ref filter) = key_filter_lower {
                        let matches = m.key().is_some_and(|k| {
                            String::from_utf8_lossy(k).to_lowercase().contains(filter.as_str())
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
        let truncated = (messages.len() as i64) < expected && messages.len() >= want;
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

        let mut offsets = Vec::new();
        let mut total_lag = 0i64;
        for e in committed.elements() {
            let current = match e.offset() {
                Offset::Offset(o) if o >= 0 => o,
                _ => continue,
            };
            let high = self
                .consumer
                .fetch_watermarks(e.topic(), e.partition(), WATERMARK_TIMEOUT)
                .map(|(_, h)| h)
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
    pub fn reset_offsets(
        &self,
        group: &str,
        positions: HashMap<(String, i32), i64>,
    ) -> Result<()> {
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

//! Domain + API DTOs for the Message Brokers (Kafka) feature.
//!
//! Wire JSON is snake_case; secrets are NEVER serialized (the cluster row keeps
//! only Keychain refs, exposed as `has_*_password` booleans). Mirrored in
//! `ui/src/lib/api/types.ts`.

use chrono::{DateTime, Utc};
use otto_core::domain::Environment;
use otto_core::id::Id;
use serde::{Deserialize, Deserializer, Serialize};

/// SSH tunnel config (shared with the DB Explorer). When present on a cluster,
/// the brokers service reaches it through a bastion (see `proxy` module).
pub use otto_ssh::SshTunnelConfig;

// ---------------------------------------------------------------------------
// Cluster connection profile
// ---------------------------------------------------------------------------

/// Transport + auth scheme used to reach the cluster (maps to librdkafka
/// `security.protocol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityProtocol {
    #[default]
    Plaintext,
    Ssl,
    SaslPlaintext,
    SaslSsl,
}

impl SecurityProtocol {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plaintext" => Some(Self::Plaintext),
            "ssl" => Some(Self::Ssl),
            "sasl_plaintext" => Some(Self::SaslPlaintext),
            "sasl_ssl" => Some(Self::SaslSsl),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plaintext => "plaintext",
            Self::Ssl => "ssl",
            Self::SaslPlaintext => "sasl_plaintext",
            Self::SaslSsl => "sasl_ssl",
        }
    }

    /// The value librdkafka expects for `security.protocol`.
    pub fn librdkafka(&self) -> &'static str {
        match self {
            Self::Plaintext => "PLAINTEXT",
            Self::Ssl => "SSL",
            Self::SaslPlaintext => "SASL_PLAINTEXT",
            Self::SaslSsl => "SASL_SSL",
        }
    }

    pub fn uses_sasl(&self) -> bool {
        matches!(self, Self::SaslPlaintext | Self::SaslSsl)
    }

    pub fn uses_tls(&self) -> bool {
        matches!(self, Self::Ssl | Self::SaslSsl)
    }
}

/// SASL mechanism (only username/password mechanisms — no Kerberos in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaslMechanism {
    #[default]
    Plain,
    // Explicit renames: serde's snake_case would emit `scram_sha256` (no
    // underscore before digits), but the DB strings, manual parse/as_str, and
    // the TS types all use `scram_sha_256`/`scram_sha_512`. Keep them aligned so
    // the JSON extractor accepts what the UI sends (else 422 on SCRAM).
    #[serde(rename = "scram_sha_256")]
    ScramSha256,
    #[serde(rename = "scram_sha_512")]
    ScramSha512,
}

impl SaslMechanism {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plain" => Some(Self::Plain),
            "scram_sha_256" => Some(Self::ScramSha256),
            "scram_sha_512" => Some(Self::ScramSha512),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::ScramSha256 => "scram_sha_256",
            Self::ScramSha512 => "scram_sha_512",
        }
    }

    /// The value librdkafka expects for `sasl.mechanism`.
    pub fn librdkafka(&self) -> &'static str {
        match self {
            Self::Plain => "PLAIN",
            Self::ScramSha256 => "SCRAM-SHA-256",
            Self::ScramSha512 => "SCRAM-SHA-512",
        }
    }
}

/// A saved Kafka cluster profile. Secrets live only in the Keychain
/// (`broker-{id}` / `broker-sr-{id}`); this never carries them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerCluster {
    pub id: Id,
    /// None = global profile (root-managed), visible to all workspaces.
    pub workspace_id: Option<Id>,
    pub name: String,
    /// Comma-separated `host:port` list.
    pub bootstrap_servers: String,
    pub security_protocol: SecurityProtocol,
    pub sasl_mechanism: Option<SaslMechanism>,
    pub sasl_username: Option<String>,
    /// True when a SASL password is stored in the Keychain.
    pub has_sasl_password: bool,
    /// Skip TLS certificate verification (self-signed brokers).
    #[serde(default)]
    pub tls_skip_verify: bool,
    pub schema_registry_url: Option<String>,
    pub schema_registry_username: Option<String>,
    pub has_sr_password: bool,
    /// Optional Prometheus exposition endpoint for broker CPU/RAM metrics.
    pub metrics_url: Option<String>,
    /// Optional UI accent color (e.g. `#ff5f57`).
    pub color: Option<String>,
    /// Optional SSH tunnel (bastion) used to reach a private cluster (e.g. MSK).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshTunnelConfig>,
    /// Section the cluster is filed under in the sidebar (None = ungrouped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<Id>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub read_only: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

impl BrokerCluster {
    /// Guarded clusters reject produce/delete/alter without an explicit confirm.
    pub fn is_write_guarded(&self) -> bool {
        self.read_only || self.environment.is_production()
    }
}

/// Create/update payload. PATCH semantics: an absent `*_password` keeps the
/// stored secret; absent `environment`/`read_only` preserve the write-guard.
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertClusterReq {
    pub name: String,
    pub bootstrap_servers: String,
    #[serde(default)]
    pub security_protocol: SecurityProtocol,
    #[serde(default)]
    pub sasl_mechanism: Option<SaslMechanism>,
    #[serde(default)]
    pub sasl_username: Option<String>,
    /// Write-only.
    #[serde(default)]
    pub sasl_password: Option<String>,
    #[serde(default)]
    pub tls_skip_verify: Option<bool>,
    #[serde(default)]
    pub schema_registry_url: Option<String>,
    #[serde(default)]
    pub schema_registry_username: Option<String>,
    /// Write-only.
    #[serde(default)]
    pub schema_registry_password: Option<String>,
    #[serde(default)]
    pub metrics_url: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// SSH tunnel. Double-option so the handler can tell apart: absent = keep
    /// the stored tunnel; explicit `null` = clear it; object = set it.
    #[serde(default, deserialize_with = "double_option")]
    pub ssh: Option<Option<SshTunnelConfig>>,
    /// Section assignment, same PATCH semantics: absent = keep; `null` =
    /// ungroup; id = file under that section.
    #[serde(default, deserialize_with = "double_option")]
    pub section_id: Option<Option<Id>>,
    #[serde(default)]
    pub environment: Option<Environment>,
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// Deserialize so a present-but-null field becomes `Some(None)` while an absent
/// field stays `None` — the PATCH "keep vs clear" distinction.
fn double_option<'de, T, D>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Serialize)]
pub struct TestClusterResp {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
    pub broker_count: usize,
}

// ---------------------------------------------------------------------------
// Cluster sections (sidebar grouping)
// ---------------------------------------------------------------------------

/// A user-defined section (folder) that groups cluster profiles in the sidebar.
/// Nests into a tree via `parent_id` (None = top-level).
#[derive(Debug, Clone, Serialize)]
pub struct BrokerClusterSection {
    pub id: Id,
    pub workspace_id: Id,
    pub parent_id: Option<Id>,
    pub name: String,
    pub position: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

/// Create (`parent_id` nests; absent = top-level) or rename (rename ignores
/// `parent_id`) a section.
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertSectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// Reparent a section (None = top-level).
#[derive(Debug, Clone, Deserialize)]
pub struct MoveSectionReq {
    #[serde(default)]
    pub parent_id: Option<Id>,
}

// ---------------------------------------------------------------------------
// Cluster overview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ClusterOverview {
    pub cluster_id: Option<String>,
    pub controller_id: i32,
    pub brokers: Vec<BrokerNode>,
    pub topic_count: usize,
    pub internal_topic_count: usize,
    pub partition_count: usize,
    pub consumer_group_count: usize,
    /// Number of partitions with ISR count < replica count (under-replicated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub under_replicated_partitions: Option<usize>,
    /// Leadership-imbalance ratio: std-dev of leader counts / mean leader count
    /// (0.0 = perfectly balanced, higher = more skewed). None when < 2 brokers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leadership_imbalance: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrokerNode {
    pub id: i32,
    pub host: String,
    pub port: i32,
    pub rack: Option<String>,
    pub is_controller: bool,
    /// Number of partitions this broker leads.
    pub partition_leaders: usize,
}

// ---------------------------------------------------------------------------
// Topics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TopicSummary {
    pub name: String,
    pub partitions: usize,
    pub replication_factor: usize,
    /// Sum of (high − low) watermarks across partitions.
    pub message_count: i64,
    pub cleanup_policy: Option<String>,
    pub internal: bool,
}

/// Lazily-loaded per-topic stats for the topics table (the list itself is a
/// single metadata pass; these are fetched per topic in the background).
#[derive(Debug, Clone, Serialize)]
pub struct TopicStats {
    pub message_count: i64,
    pub cleanup_policy: Option<String>,
}

/// Request body for the batch stats endpoint `POST /topics/stats`.
/// Returns a map of topic name → stats, fetching counts in parallel using
/// the shared `WATERMARK_WORKERS` pool (avoids N×1 HTTP round-trips from the UI).
#[derive(Debug, Clone, Deserialize)]
pub struct BatchStatsReq {
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicDetail {
    pub name: String,
    pub internal: bool,
    pub partitions: Vec<PartitionInfo>,
    pub configs: Vec<TopicConfigEntry>,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartitionInfo {
    pub id: i32,
    pub leader: i32,
    pub replicas: Vec<i32>,
    pub isr: Vec<i32>,
    pub low: i64,
    pub high: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicConfigEntry {
    pub name: String,
    pub value: Option<String>,
    pub source: String,
    pub is_default: bool,
    pub is_sensitive: bool,
    pub is_read_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTopicReq {
    pub name: String,
    #[serde(default = "one")]
    pub partitions: i32,
    #[serde(default = "one")]
    pub replication_factor: i32,
    #[serde(default)]
    pub configs: Vec<ConfigKv>,
    /// Explicit confirm for guarded (prod / read-only) clusters.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlterConfigsReq {
    pub configs: Vec<ConfigKv>,
    /// Explicit confirm for guarded (prod / read-only) clusters.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigKv {
    pub name: String,
    pub value: String,
}

fn one() -> i32 {
    1
}

// ---------------------------------------------------------------------------
// Messages: consume / produce
// ---------------------------------------------------------------------------

/// How to interpret a message value when decoding for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueFormat {
    /// JSON → string → schemaless protobuf → hex, in that order. Confluent-framed
    /// Avro is decoded automatically when a schema registry is configured.
    #[default]
    Auto,
    Json,
    Utf8,
    Hex,
    Base64,
    Protobuf,
    Avro,
}

/// Where to start reading from.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum StartPosition {
    /// Oldest available messages, forward.
    Beginning,
    /// The most recent `limit` messages (default).
    #[default]
    Latest,
    /// From an explicit offset (requires a single `partition`).
    Offset { offset: i64 },
    /// First message at or after the given epoch-millis timestamp.
    Timestamp { timestamp_ms: i64 },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsumeReq {
    /// None = every partition.
    #[serde(default)]
    pub partition: Option<i32>,
    #[serde(default)]
    pub start: StartPosition,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub max_wait_ms: Option<u64>,
    /// Case-insensitive substring filter on the decoded key.
    #[serde(default)]
    pub key_filter: Option<String>,
    /// Case-insensitive substring filter on the decoded value.
    #[serde(default)]
    pub value_filter: Option<String>,
    #[serde(default)]
    pub decode: ValueFormat,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsumeResp {
    pub messages: Vec<KafkaMessage>,
    pub partitions: Vec<PartitionRange>,
    /// True when the limit/timeout was hit before draining the range.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartitionRange {
    pub partition: i32,
    pub low: i64,
    pub high: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KafkaMessage {
    pub partition: i32,
    pub offset: i64,
    pub timestamp_ms: Option<i64>,
    pub key: Option<DecodedPayload>,
    pub value: Option<DecodedPayload>,
    pub headers: Vec<MessageHeader>,
    pub size_bytes: usize,
}

/// A best-effort human rendering of a key/value plus the raw bytes (base64) so
/// the UI can re-render as hex/base64 on demand.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedPayload {
    /// `json` | `string` | `hex` | `base64` | `protobuf` | `avro` | `null`.
    pub format: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageHeader {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProduceReq {
    #[serde(default)]
    pub partition: Option<i32>,
    #[serde(default)]
    pub key: Option<String>,
    pub value: String,
    #[serde(default)]
    pub headers: Vec<MessageHeader>,
    /// Interpret `key` as base64-encoded bytes.
    #[serde(default)]
    pub key_base64: bool,
    /// Interpret `value` as base64-encoded bytes.
    #[serde(default)]
    pub value_base64: bool,
    /// Explicit confirm for guarded (prod / read-only) clusters.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProduceResp {
    pub partition: i32,
    pub offset: i64,
}

// ---------------------------------------------------------------------------
// Consumer groups
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct GroupSummary {
    pub group_id: String,
    pub state: String,
    pub protocol_type: String,
    pub members: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupDetail {
    pub group_id: String,
    pub state: String,
    pub protocol_type: String,
    pub protocol: String,
    pub members: Vec<GroupMember>,
    pub offsets: Vec<GroupOffset>,
    pub total_lag: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupMember {
    pub member_id: String,
    pub client_id: String,
    pub host: String,
    pub assignments: Vec<TopicPartition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicPartition {
    pub topic: String,
    pub partition: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupOffset {
    pub topic: String,
    pub partition: i32,
    pub current_offset: i64,
    pub high_watermark: i64,
    pub lag: i64,
}

// ---------------------------------------------------------------------------
// Group offset reset
// ---------------------------------------------------------------------------

/// How to interpret the target position when resetting group offsets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum OffsetResetMode {
    /// Reset to the earliest available offset (beginning) for each partition.
    Earliest,
    /// Reset to the latest offset (end) for each partition.
    Latest,
    /// Reset to an explicit absolute offset (requires `offset` field).
    Offset(i64),
    /// Reset to the first offset at or after `timestamp_ms`.
    Timestamp,
}

/// Request body for `POST /brokers/clusters/{id}/groups/{group}/reset`.
#[derive(Debug, Clone, Deserialize)]
pub struct GroupResetReq {
    #[serde(flatten)]
    pub mode: OffsetResetMode,
    /// Required when `mode = "timestamp"`.
    #[serde(default)]
    pub timestamp_ms: Option<i64>,
    /// Scope to a single topic (all its partitions committed by this group).
    /// None = all topics the group has offsets for.
    #[serde(default)]
    pub topic: Option<String>,
    /// Explicit confirm for guarded (prod / read-only) clusters.
    #[serde(default)]
    pub confirm: bool,
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ClusterMetrics {
    /// Sampled throughput series (oldest → newest).
    pub throughput: Vec<ThroughputPoint>,
    pub messages_per_sec: f64,
    pub total_messages: i64,
    pub brokers: Vec<BrokerResourceMetrics>,
    pub prometheus_available: bool,
    pub sampled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThroughputPoint {
    pub ts_ms: i64,
    pub total_messages: i64,
    pub messages_per_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrokerResourceMetrics {
    /// Prometheus `instance`/`node`/shard label, or `broker`.
    pub instance: String,
    pub cpu_percent: Option<f64>,
    pub memory_used_bytes: Option<f64>,
    pub memory_total_bytes: Option<f64>,
    /// A handful of additional notable gauges for display.
    pub extra: Vec<NamedMetric>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedMetric {
    pub name: String,
    pub value: f64,
}

// ---------------------------------------------------------------------------
// Schema registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SchemaSubject {
    pub subject: String,
    pub version: i32,
    pub id: i32,
    pub schema_type: String,
    pub schema: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_protocol_roundtrip() {
        for p in [
            SecurityProtocol::Plaintext,
            SecurityProtocol::Ssl,
            SecurityProtocol::SaslPlaintext,
            SecurityProtocol::SaslSsl,
        ] {
            assert_eq!(SecurityProtocol::parse(p.as_str()), Some(p));
        }
        assert_eq!(SecurityProtocol::SaslSsl.librdkafka(), "SASL_SSL");
        assert!(SecurityProtocol::SaslSsl.uses_sasl());
        assert!(SecurityProtocol::SaslSsl.uses_tls());
    }

    #[test]
    fn sasl_mechanism_roundtrip() {
        for m in [
            SaslMechanism::Plain,
            SaslMechanism::ScramSha256,
            SaslMechanism::ScramSha512,
        ] {
            assert_eq!(SaslMechanism::parse(m.as_str()), Some(m));
        }
        assert_eq!(SaslMechanism::ScramSha256.librdkafka(), "SCRAM-SHA-256");
    }

    #[test]
    fn sasl_mechanism_serde_matches_db_strings() {
        // serde JSON repr must equal the as_str()/DB/TS strings (regression: the
        // default snake_case would emit `scram_sha512`, breaking the 9096 path).
        for m in [
            SaslMechanism::Plain,
            SaslMechanism::ScramSha256,
            SaslMechanism::ScramSha512,
        ] {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, format!("\"{}\"", m.as_str()));
            let back: SaslMechanism = serde_json::from_str(&json).unwrap();
            assert_eq!(back, m);
        }
        // Exactly what the UI sends.
        let m: SaslMechanism = serde_json::from_str("\"scram_sha_512\"").unwrap();
        assert_eq!(m, SaslMechanism::ScramSha512);
    }

    #[test]
    fn upsert_ssh_three_state() {
        let base = r#""name":"c","bootstrap_servers":"b:9092""#;
        // Absent → keep (None).
        let req: UpsertClusterReq = serde_json::from_str(&format!("{{{base}}}")).unwrap();
        assert!(req.ssh.is_none());
        // Explicit null → clear (Some(None)).
        let req: UpsertClusterReq =
            serde_json::from_str(&format!("{{{base},\"ssh\":null}}")).unwrap();
        assert_eq!(req.ssh, Some(None));
        // Object → set (Some(Some(..))).
        let req: UpsertClusterReq = serde_json::from_str(&format!(
            "{{{base},\"ssh\":{{\"host\":\"bastion\",\"user\":\"ec2-user\"}}}}"
        ))
        .unwrap();
        let cfg = req.ssh.unwrap().unwrap();
        assert_eq!(cfg.host, "bastion");
        assert_eq!(cfg.port, 22);
        assert_eq!(cfg.user, "ec2-user");
    }

    #[test]
    fn consume_req_defaults() {
        let req: ConsumeReq = serde_json::from_str("{}").unwrap();
        assert_eq!(req.limit, 50);
        assert_eq!(req.decode, ValueFormat::Auto);
        assert_eq!(req.start, StartPosition::Latest);
        assert!(req.partition.is_none());
    }

    #[test]
    fn start_position_tagged() {
        let s: StartPosition = serde_json::from_str(r#"{"type":"offset","offset":42}"#).unwrap();
        assert_eq!(s, StartPosition::Offset { offset: 42 });
        let s: StartPosition = serde_json::from_str(r#"{"type":"beginning"}"#).unwrap();
        assert_eq!(s, StartPosition::Beginning);
    }
}

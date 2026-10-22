// Contract mirror: brokers. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Environment,
} from './connections';
import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Message Brokers (Kafka) — mirror of crates/otto-brokers/src/types.rs
// ---------------------------------------------------------------------------

export type SecurityProtocol = 'plaintext' | 'ssl' | 'sasl_plaintext' | 'sasl_ssl';
export type SaslMechanism = 'plain' | 'scram_sha_256' | 'scram_sha_512';

/** SSH tunnel (bastion) config — shared with the DB Explorer connection form.
 * Auth is key-file/agent only (no password). Mirrors otto_ssh::SshTunnelConfig. */
export interface SshTunnelConfig {
  host: string;
  port?: number;
  /** Optional — the server falls back to ~/.ssh/config, then $USER. */
  user?: string;
  identity_file?: string | null;
}

export interface BrokerCluster {
  id: Id;
  workspace_id: Id | null;
  name: string;
  bootstrap_servers: string;
  security_protocol: SecurityProtocol;
  sasl_mechanism: SaslMechanism | null;
  sasl_username: string | null;
  has_sasl_password: boolean;
  tls_skip_verify: boolean;
  schema_registry_url: string | null;
  schema_registry_username: string | null;
  has_sr_password: boolean;
  metrics_url: string | null;
  color: string | null;
  /** SSH tunnel to reach a private cluster (e.g. MSK) through a bastion. */
  ssh?: SshTunnelConfig | null;
  /** Section the cluster is filed under in the sidebar (null = ungrouped). */
  section_id?: Id | null;
  environment: Environment;
  read_only: boolean;
  created_by: Id;
  created_at: string;
}

export interface UpsertClusterReq {
  name: string;
  bootstrap_servers: string;
  security_protocol: SecurityProtocol;
  sasl_mechanism?: SaslMechanism | null;
  sasl_username?: string | null;
  sasl_password?: string | null;
  tls_skip_verify?: boolean | null;
  schema_registry_url?: string | null;
  schema_registry_username?: string | null;
  schema_registry_password?: string | null;
  metrics_url?: string | null;
  color?: string | null;
  /** SSH tunnel. Omit to keep the stored value; `null` to clear; object to set. */
  ssh?: SshTunnelConfig | null;
  /** Section assignment. Omit = keep; `null` = ungroup; id = file under section. */
  section_id?: Id | null;
  environment?: Environment | null;
  read_only?: boolean | null;
}

/** Lazily-loaded per-topic stats (message count + cleanup policy). */
export interface TopicStats {
  message_count: number;
  cleanup_policy: string | null;
  /** Approx production rate (msg/s) from the high-watermark delta between two
   *  consecutive `topics/stats` calls; null on the first sample or a count error. */
  msg_per_sec?: number | null;
}

/** Request body for POST /brokers/clusters/{id}/topics/stats (batch). */
export interface BatchStatsReq {
  names: string[];
}

export interface TestClusterResp {
  ok: boolean;
  latency_ms: number;
  message: string;
  broker_count: number;
}

export interface ClusterOverview {
  cluster_id: string | null;
  controller_id: number;
  brokers: BrokerNode[];
  topic_count: number;
  internal_topic_count: number;
  partition_count: number;
  consumer_group_count: number;
  /** Number of partitions with ISR count < replica count. */
  under_replicated_partitions?: number | null;
  /** Leadership imbalance: coefficient of variation of leader counts (0 = balanced). */
  leadership_imbalance?: number | null;
}

export interface BrokerNode {
  id: number;
  host: string;
  port: number;
  rack: string | null;
  is_controller: boolean;
  partition_leaders: number;
}

export interface TopicSummary {
  name: string;
  partitions: number;
  replication_factor: number;
  message_count: number;
  cleanup_policy: string | null;
  internal: boolean;
}

export interface TopicDetail {
  name: string;
  internal: boolean;
  partitions: PartitionInfo[];
  configs: TopicConfigEntry[];
  message_count: number;
}

export interface PartitionInfo {
  id: number;
  leader: number;
  replicas: number[];
  isr: number[];
  low: number;
  high: number;
  message_count: number;
}

export interface TopicConfigEntry {
  name: string;
  value: string | null;
  source: string;
  is_default: boolean;
  is_sensitive: boolean;
  is_read_only: boolean;
}

export interface ConfigKv {
  name: string;
  value: string;
}

export interface CreateTopicReq {
  name: string;
  partitions: number;
  replication_factor: number;
  configs?: ConfigKv[];
  confirm?: boolean;
}

export interface AlterConfigsReq {
  configs: ConfigKv[];
  confirm?: boolean;
}

export type ValueFormat = 'auto' | 'json' | 'utf8' | 'hex' | 'base64' | 'protobuf' | 'avro';

export type StartPosition =
  | { type: 'beginning' }
  | { type: 'latest' }
  | { type: 'offset'; offset: number }
  | { type: 'timestamp'; timestamp_ms: number };

export interface ConsumeReq {
  partition?: number | null;
  start?: StartPosition;
  limit?: number;
  max_wait_ms?: number | null;
  /** Applied server-side during consume (raw bytes). Limits the result to
   * messages whose key contains this substring (case-insensitive). */
  key_filter?: string | null;
  /** When true and key_filter is set, scan from the earliest offset so older
   * matching messages are found regardless of the `start` position. */
  find_from_beginning?: boolean;
  /** Applied post-decode in the service layer. */
  value_filter?: string | null;
  decode?: ValueFormat;
  /** When true, the server runs message key/value/headers through
   * `otto_core::redact` before returning. Raw payloads never leave the server
   * when this flag is set. The response `masked` field confirms it. */
  mask?: boolean;
  /** List mode: values over 2 KiB come back cut to a 2 KiB preview with
   * `raw_base64` dropped and `truncated: true`; fetch the one message
   * (`partition` + `start: offset` + `limit: 1`) for the full value. */
  preview?: boolean;
  /** Live tail: consume ONLY these partitions, each from its own offset, in
   * one request. Overrides `start` / `partition`. */
  start_offsets?: { partition: number; offset: number }[] | null;
}

export interface DecodedPayload {
  format: string;
  text: string;
  schema_id?: number;
  raw_base64?: string;
  /** Set on a `preview: true` peek when `text` was cut to 2 KiB. */
  truncated?: boolean;
}

export interface MessageHeader {
  key: string;
  value: string;
}

export interface KafkaMessage {
  partition: number;
  offset: number;
  timestamp_ms: number | null;
  key: DecodedPayload | null;
  value: DecodedPayload | null;
  headers: MessageHeader[];
  size_bytes: number;
}

export interface PartitionRange {
  partition: number;
  low: number;
  high: number;
}

export interface ConsumeResp {
  messages: KafkaMessage[];
  partitions: PartitionRange[];
  truncated: boolean;
  /** True when message payloads were run through `otto_core::redact` server-side
   * (ConsumeReq.mask=true). The UI surfaces this as a badge. */
  masked?: boolean;
}

export interface ProduceReq {
  partition?: number | null;
  key?: string | null;
  value: string;
  headers?: MessageHeader[];
  key_base64?: boolean;
  value_base64?: boolean;
  confirm?: boolean;
}

export interface ProduceResp {
  partition: number;
  offset: number;
}

export interface GroupSummary {
  group_id: string;
  state: string;
  protocol_type: string;
  members: number;
}

export interface TopicPartition {
  topic: string;
  partition: number;
}

export interface GroupMember {
  member_id: string;
  client_id: string;
  host: string;
  assignments: TopicPartition[];
}

export interface GroupOffset {
  topic: string;
  partition: number;
  current_offset: number;
  high_watermark: number;
  lag: number;
}

export interface GroupDetail {
  group_id: string;
  state: string;
  protocol_type: string;
  protocol: string;
  members: GroupMember[];
  offsets: GroupOffset[];
  total_lag: number;
}

export type OffsetResetMode = 'earliest' | 'latest' | 'offset' | 'timestamp';

/** Request body for POST /brokers/clusters/{id}/groups/{group}/reset */
export interface GroupResetReq {
  /** Reset mode (earliest | latest | offset | timestamp). */
  mode: OffsetResetMode;
  /** Required when mode = 'offset'. */
  offset?: number;
  /** Required when mode = 'timestamp'. Epoch millis. */
  timestamp_ms?: number;
  /** Scope to a single topic (omit = all topics the group has offsets for). */
  topic?: string;
  /** Explicit confirm for guarded (prod / read-only) clusters. */
  confirm?: boolean;
}

export interface ThroughputPoint {
  ts_ms: number;
  total_messages: number;
  messages_per_sec: number;
}

export interface NamedMetric {
  name: string;
  value: number;
}

export interface BrokerResourceMetrics {
  instance: string;
  cpu_percent: number | null;
  memory_used_bytes: number | null;
  memory_total_bytes: number | null;
  extra: NamedMetric[];
}

export interface ClusterMetrics {
  throughput: ThroughputPoint[];
  messages_per_sec: number;
  total_messages: number;
  brokers: BrokerResourceMetrics[];
  prometheus_available: boolean;
  sampled_at: string;
}

export interface SchemaSubject {
  subject: string;
  version: number;
  id: number;
  schema_type: string;
  schema: string;
}

/** Broker schema history returns identifiers only, oldest first. */
/** Lifecycle of a stored replay record (PC-5, migration 0193): `running` while
 *  it publishes, then `done` / `failed`; `interrupted` when the daemon stopped
 *  mid-run. Rows from before the column read `done`. */
export type BrokerReplayStatus = 'running' | 'done' | 'failed' | 'interrupted';

/** A replay can stop after acknowledged writes. Inspect error before claiming success. */
export interface BrokerReplayResp {
  replay_id: string;
  /** The stored record's status (PC-5). */
  status?: BrokerReplayStatus;
  source_topic: string;
  target_topic: string;
  count: number;
  evidence: { partition: number; offset: number; key_preview: string | null; target_partition: number; target_offset: number }[];
  error?: string;
  evidence_saved: boolean;
}

export interface BrokerSchemaVersion { version: number; }
/** Immutable numeric version body (the detail endpoint also accepts latest). */
export interface BrokerSchemaVersionDetail {
  subject: string;
  version: number;
  id: number;
  schema_type: string;
  schema: string;
}

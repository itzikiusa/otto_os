// Contract mirror: aws. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Environment,
} from './connections';
import type {
  Id,
} from './core';
import type {
  QueryResult,
} from './database';

// ---------------------------------------------------------------------------
// AWS console — mirrors docs/design/aws-k8s-consoles.md §2 (crates/otto-aws).
// Accounts never carry secrets; the daemon shells out to the `aws` CLI.
// ---------------------------------------------------------------------------

export type AwsAuthMode = 'profile' | 'access_keys';
export type AwsService = 's3' | 'sqs' | 'ec2' | 'athena' | 'eks' | 'rds';
export type InstallTool = 'aws' | 'kubectl' | 'k9s';
export type InstallJobState = 'idle' | 'running' | 'done' | 'failed';

/** Background CLI installer state (one per tool), polled via `/aws/status`. */
export interface InstallJob {
  tool: InstallTool;
  state: InstallJobState;
  log_tail: string;
  started_at?: string | null;
  finished_at?: string | null;
  error?: string | null;
}

/** `GET /aws/status`. */
export interface AwsStatus {
  installed: boolean;
  version: string | null;
  path: string | null;
  install: InstallJob;
  /** Live counters of the daemon-wide `aws` CLI child cap. */
  cli?: AwsCliStats;
}

/** `GET /aws/status` → `cli`: the daemon-wide `aws` child cap, live. */
export interface AwsCliStats {
  running: number;
  queued: number;
  spawned_total: number;
  max_concurrent: number;
  /** Of `max_concurrent`, the share background probes / fan-outs may hold. */
  background_max?: number;
  /** Nearest-rank percentiles (ms) over the last `samples` (≤ 256) calls. */
  wait_ms_p50?: number;
  wait_ms_p95?: number;
  call_ms_p50?: number;
  call_ms_p95?: number;
  samples?: number;
  /** Calls signed in-process (logs tail, query status, EC2 lists) — no child. */
  native_total?: number;
}

/** One profile parsed from `~/.aws/config` / `~/.aws/credentials` — never key values. */
export interface DiscoveredProfile {
  name: string;
  region?: string | null;
  sso_start_url?: string | null;
  sso_session?: string | null;
  role_arn?: string | null;
  source: 'config' | 'credentials';
}

export interface AwsRegion {
  code: string;
  name: string;
}

/** `sts get-caller-identity`. */
export interface AwsIdentity {
  account: string;
  arn: string;
  user_id: string;
}

export type AwsPermissionState = 'allowed' | 'denied' | 'unknown';

/** `GET /aws/accounts/{id}/permissions` — the per-service View probe. */
export interface AwsPermissions {
  checked_at: string;
  identity?: AwsIdentity | null;
  services: Record<AwsService, AwsPermissionState>;
  login_required: boolean;
}

export interface AwsAccount {
  id: Id;
  name: string;
  auth_mode: AwsAuthMode;
  profile?: string | null;
  region: string;
  access_key_id?: string | null;
  role_arn?: string | null;
  /** Custom service endpoint (LocalStack, VPC endpoints, S3-compatible stores);
   *  injected as `AWS_ENDPOINT_URL` into every CLI subprocess when set. */
  endpoint_url?: string | null;
  environment: Environment;
  color?: string | null;
  identity?: AwsIdentity | null;
  permissions?: AwsPermissions | null;
  /** When the sign-in ends (profile accounts only) — warn before it lapses. */
  session?: AwsSessionInfo | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
  last_used_at?: string | null;
}

/** `AwsAccount.session` — the SSO token / exported temporary credentials. */
export interface AwsSessionInfo {
  expires_at: string;
  /** `sso` = the IAM Identity Center token; `credentials` = exported role creds. */
  source: 'sso' | 'credentials';
  /** The CLI renews it silently (refresh token / re-export) — no warning needed. */
  refreshable: boolean;
}

/** `POST /aws/accounts` body; every field optional on `PATCH` (secrets omitted = keep). */
export interface UpsertAwsAccountReq {
  name: string;
  auth_mode: AwsAuthMode;
  profile?: string | null;
  region: string;
  access_key_id?: string | null;
  secret_access_key?: string | null;
  session_token?: string | null;
  role_arn?: string | null;
  /** `http(s)://…` — plain `http` only for loopback hosts (400 otherwise).
   *  On `PATCH`, `""` clears it; omitted keeps it. */
  endpoint_url?: string | null;
  environment?: Environment;
  color?: string | null;
}

/** `POST /aws/accounts/{id}/test`. */
export interface AwsTestResp {
  ok: boolean;
  latency_ms: number;
  message: string;
  identity?: AwsIdentity | null;
  login_required: boolean;
}

// --- S3 (read-only) ---

export interface S3Bucket {
  name: string;
  creation_date: string;
  region?: string | null;
}

export interface S3Object {
  key: string;
  size: number;
  last_modified: string;
  storage_class?: string | null;
  etag?: string | null;
}

/** `GET …/s3/buckets/{bucket}/objects` (one delimiter level). */
export interface S3ListObjectsResp {
  prefixes: string[];
  objects: S3Object[];
  next_token?: string | null;
  is_truncated: boolean;
}

export interface S3ObjectHead {
  key: string;
  size: number;
  content_type?: string | null;
  last_modified: string;
  etag?: string | null;
  metadata: Record<string, string>;
  storage_class?: string | null;
}

/** `GET …/s3/buckets/{bucket}/preview` — `binary:true` when the type is not previewable. */
export interface S3PreviewResp {
  text?: string | null;
  truncated?: boolean;
  content_type?: string | null;
  binary?: boolean;
  /** `text` → `text`; `image`/`pdf` → fetch `download?inline=true` (≤ 25 MB); `binary` → download only. */
  kind?: 'text' | 'image' | 'pdf' | 'binary';
  size?: number | null;
}

/** `POST …/s3/buckets/{bucket}/presign`. */
export interface S3PresignResp {
  url: string;
  expires_at: string;
  /** Temporary credentials end before `expires_at` — the link dies with them. */
  warning?: string | null;
}

/** `PUT …/s3/buckets/{bucket}/object?key=` (raw body) → 201. */
export interface S3UploadResp {
  key: string;
  size: number;
}

/** One region that failed in an `?region=all` fan-out (EC2 / EKS / RDS lists). */
export interface AwsRegionError {
  region: string;
  message: string;
}

// --- CloudWatch Logs (`/aws/accounts/{id}/logs/*`) ---
export interface AwsLogGroup {
  name: string;
  arn?: string | null;
  created_ms?: number | null;
  retention_days?: number | null;
  stored_bytes?: number | null;
  class?: string | null;
}
export interface AwsLogStream {
  name: string;
  created_ms?: number | null;
  first_event_ms?: number | null;
  last_event_ms?: number | null;
  stored_bytes?: number | null;
}
export interface AwsLogEvent {
  /** Unique per event — tail polls dedupe on it. */
  id: string;
  stream: string;
  timestamp: number;
  ingestion_time?: number | null;
  message: string;
}
export interface AwsLogEventsQuery {
  group: string;
  streams?: string[];
  pattern?: string;
  /** Epoch ms. */
  start?: number;
  end?: number;
  token?: string | null;
  max?: number;
  region?: string;
}
export interface AwsLogsInsightsReq {
  groups: string[];
  query: string;
  /** Epoch ms. */
  start: number;
  end: number;
  limit?: number;
}
/** `GET …/logs/insights/{qid}` — `result` is the DB Explorer `QueryResult` shape. */
export interface AwsLogsInsightsResults {
  status: 'Scheduled' | 'Running' | 'Complete' | 'Failed' | 'Cancelled' | 'Timeout' | 'Unknown'; // AWS wire values; ui-guards: allow
  done: boolean;
  result: QueryResult;
  records_matched?: number | null;
  records_scanned?: number | null;
  bytes_scanned?: number | null;
}

/** `POST …/s3/buckets/{bucket}/download-to` / `GET …/s3/download-jobs/{job}` —
 *  a daemon-side download of a large object into a local directory. */
export interface S3DownloadJob {
  id: string;
  bucket: string;
  key: string;
  /** Final file path (present once `state` is `completed`). */
  local_path: string;
  state: 'running' | 'completed' | 'failed' | 'cancelled';
  bytes: number;
  total: number;
  error?: string | null;
}

// --- SQS ---

export interface SqsQueue {
  url: string;
  name: string;
  fifo: boolean;
}

export interface SqsQueueAttributesResp {
  attributes: Record<string, string>;
  approx_messages: number;
  approx_not_visible: number;
  approx_delayed: number;
  dlq_target_arn?: string | null;
}

export interface SqsMessage {
  message_id: string;
  receipt_handle: string;
  body: string;
  attributes: Record<string, string>;
  message_attributes: Record<string, unknown>;
  md5?: string | null;
}

export interface SqsPeekReq {
  url: string;
  max?: number;
  /** Ignored: the daemon always peeks with visibility timeout 0. */
  visibility_timeout?: number;
}

export interface SqsSendReq {
  url: string;
  body: string;
  delay_seconds?: number;
  group_id?: string;
  dedup_id?: string;
  message_attributes?: Record<string, { DataType: string; StringValue: string }>;
}

export interface SqsRedriveReq {
  source_arn: string;
  destination_arn?: string;
}

// --- EC2 ---

export type Ec2State =
  | 'pending'
  | 'running'
  | 'shutting-down'
  | 'terminated'
  | 'stopping'
  | 'stopped'
  | string;

export interface Ec2Instance {
  instance_id: string;
  /** Set on `?region=all` rows. */
  region?: string;
  name?: string | null;
  state: Ec2State;
  type: string;
  az?: string | null;
  private_ip?: string | null;
  public_ip?: string | null;
  launch_time?: string | null;
  platform?: string | null;
  vpc_id?: string | null;
  subnet_id?: string | null;
  tags: Record<string, string>;
}

export interface Ec2InstanceDetail extends Ec2Instance {
  raw: unknown;
}

export type Ec2Action = 'start' | 'stop' | 'reboot';

export interface Ec2ActionResp {
  previous_state: string;
  current_state: string;
}

// --- Athena ---

export interface AthenaWorkgroup {
  name: string;
  state: string;
  output_location?: string | null;
}

export interface AthenaColumn {
  name: string;
  type: string;
}

export interface AthenaTable {
  name: string;
  type?: string | null;
  columns: AthenaColumn[];
}

export interface AthenaExecution {
  id: string;
  query: string;
  state: AthenaQueryState;
  submitted_at: string;
  completed_at?: string | null;
  data_scanned_bytes?: number | null;
  execution_ms?: number | null;
}

export interface AthenaQueryReq {
  sql: string;
  database?: string;
  workgroup?: string;
  output_location?: string;
  /** Required on a prod account for any statement that is not a plain read
   *  (DDL/DML) — set only after the person confirms; else 400 `confirm_required`. */
  confirm?: boolean;
}

export type AthenaQueryState = 'QUEUED' | 'RUNNING' | 'SUCCEEDED' | 'FAILED' | 'CANCELLED';

/** `GET …/athena/query/{qid}` — `result` is the DB Explorer `QueryResult` shape
 *  so `ResultsGrid` renders it as-is. */
export interface AthenaQueryStatus {
  state: AthenaQueryState;
  reason?: string | null;
  stats: { data_scanned_bytes: number; execution_ms: number };
  result?: QueryResult | null;
  next_token?: string | null;
}

// --- EKS ---

export interface EksClusterSummary {
  name: string;
  /** Set on `?region=all` rows. */
  region?: string;
  status: string;
  version?: string | null;
  endpoint?: string | null;
  arn?: string | null;
  created_at?: string | null;
}

export interface EksNodegroup {
  name: string;
  status: string;
  desired?: number | null;
  min?: number | null;
  max?: number | null;
  instance_types: string[];
  ami_type?: string | null;
}

export interface EksClusterDetail {
  cluster: unknown;
  nodegroups: EksNodegroup[];
}

export interface EksImportReq {
  cluster_name_override?: string;
  default_namespace?: string;
}

/** The `K8sCluster` row created by `import-kubeconfig` — only the fields the AWS
 *  module needs to navigate (`#/kubernetes/<id>`); the full shape lives in the
 *  Kubernetes console section. */
export interface EksImportResp {
  id: Id;
  name: string;
  [extra: string]: unknown;
}

// --- RDS (read-only) ---

export interface RdsInstance {
  identifier: string;
  /** Set on `?region=all` rows. */
  region?: string;
  engine?: string | null;
  engine_version?: string | null;
  class?: string | null;
  status: string;
  az?: string | null;
  multi_az: boolean;
  storage_gb?: number | null;
  storage_type?: string | null;
  /** Endpoint host (port is separate). */
  endpoint?: string | null;
  port?: number | null;
  db_name?: string | null;
  master_username?: string | null;
  publicly_accessible: boolean;
  created?: string | null;
  tags: Record<string, string>;
}

export interface RdsInstanceDetail extends RdsInstance {
  raw: unknown;
}

// --- CloudWatch metrics (`GET …/metrics`) ---

export type MetricsNamespace = 'AWS/SQS' | 'AWS/EC2' | 'AWS/RDS';
export type MetricsDimension = 'QueueName' | 'InstanceId' | 'DBInstanceIdentifier';
export type MetricsRange = '1h' | '6h' | '24h' | '7d' | '30d';
export type MetricUnit =
  | 'count'
  | 'bytes'
  | 'percent'
  | 'seconds'
  | 'ms'
  | 'count_per_sec'
  | 'bytes_per_sec';

export interface MetricPoint {
  /** RFC 3339. */
  t: string;
  /** `null` = CloudWatch returned nothing for this period slot (a gap). */
  v: number | null;
}

export interface MetricSeries {
  /** Catalog id (`messages_sent`, `cpu`, …) — stable across calls. */
  id: string;
  metric: string;
  stat: 'Sum' | 'Average' | 'Maximum' | string;
  unit: MetricUnit;
  label: string;
  /** Every period slot of `[start, end]`, ascending; gaps are `null`. */
  points: MetricPoint[];
  /** Latest non-null value; `null` when the series has no data. */
  current: number | null;
  min: number | null;
  max: number | null;
  sum: number | null;
  avg: number | null;
}

export interface MetricsResp {
  namespace: MetricsNamespace;
  dim_name: MetricsDimension;
  dim_value: string;
  range: MetricsRange;
  period_seconds: number;
  start: string;
  end: string;
  series: MetricSeries[];
}

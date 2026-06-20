// Module-local types for the B6 broker operator workflow features.
// Do NOT import from or add to ui/src/lib/api/types.ts.

// ---- Schema registry version history & compat --------------------------------

export interface SchemaVersion {
  version: number;
  id: number;
  schema_type: string;
  schema: string;
}

export interface SchemaVersionDetail {
  subject: string;
  version: number;
  id: number;
  schema_type: string;
  schema: string;
}

export interface CompatCheckResp {
  compatible: boolean;
  messages: string[];
}

// ---- DLQ / Replay ------------------------------------------------------------

export type ReplaySelector =
  | { type: 'latest'; count: number }
  | { type: 'offset_range'; partition: number; from: number; to: number }
  | { type: 'timestamp'; timestamp_ms: number; limit: number };

export interface ReplayEvidence {
  partition: number;
  offset: number;
  key_preview: string | null;
  target_partition: number;
  target_offset: number;
}

export interface ReplayResp {
  replay_id: string;
  source_topic: string;
  target_topic: string;
  count: number;
  evidence: ReplayEvidence[];
}

// ---- Offset-reset dry-run preview -------------------------------------------

export interface DryRunPartition {
  topic: string;
  partition: number;
  current_offset: number;
  target_offset: number;
  /** Positive = lag decreases; negative = lag increases (rewinding). */
  lag_delta: number;
}

export interface DryRunResp {
  group: string;
  partitions: DryRunPartition[];
  total_lag_before: number;
  total_lag_after: number;
}

// ---- Lag alerts --------------------------------------------------------------

export interface LagAlert {
  id: string;
  cluster_id: string;
  topic: string;
  group_name: string;
  threshold: number;
  enabled: boolean;
  created_at: string;
  /** Lag at last evaluation if the alert is breached; absent when not breached. */
  breach_lag?: number;
}

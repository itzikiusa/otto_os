// Module-local types for the B6 broker operator workflow features.
// Shared wire contracts are re-exported from lib/api/types.ts.

// ---- Schema registry version history & compat --------------------------------

export type {
  BrokerSchemaVersion as SchemaVersion,
  BrokerSchemaVersionDetail as SchemaVersionDetail,
} from '../../lib/api/types';

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

export type { BrokerReplayResp as ReplayResp } from '../../lib/api/types';

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

/** The per-cluster views, in tab order (BrokersPage + the embedded ClusterViewer). */
export type ClusterView = 'overview' | 'topics' | 'groups' | 'schema' | 'replay' | 'alerts';
export const CLUSTER_VIEWS: { id: ClusterView; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'topics', label: 'Topics' },
  { id: 'groups', label: 'Consumer groups' },
  { id: 'schema', label: 'Schema Registry' },
  { id: 'replay', label: 'Replay' },
  { id: 'alerts', label: 'Lag alerts' },
];

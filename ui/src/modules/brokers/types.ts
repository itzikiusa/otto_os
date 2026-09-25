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

/** The per-cluster views, in tab order (BrokersPage + the embedded ClusterViewer). */
export type ClusterView = 'overview' | 'topics' | 'groups' | 'schema' | 'replay' | 'alerts';
export const CLUSTER_VIEWS: { id: ClusterView; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'topics', label: 'Topics' },
  { id: 'groups', label: 'Consumer Groups' },
  { id: 'schema', label: 'Schema Registry' },
  { id: 'replay', label: 'Replay' },
  { id: 'alerts', label: 'Lag Alerts' },
];

/** ←/→/Home/End across a view tablist: returns the next view (and moves focus
 *  to its tab), or null when the key isn't a tablist key. */
export function clusterViewKey(e: KeyboardEvent, current: ClusterView): ClusterView | null {
  const i = CLUSTER_VIEWS.findIndex((v) => v.id === current);
  let j = -1;
  if (e.key === 'ArrowRight') j = (i + 1) % CLUSTER_VIEWS.length;
  else if (e.key === 'ArrowLeft') j = (i - 1 + CLUSTER_VIEWS.length) % CLUSTER_VIEWS.length;
  else if (e.key === 'Home') j = 0;
  else if (e.key === 'End') j = CLUSTER_VIEWS.length - 1;
  if (j < 0) return null;
  e.preventDefault();
  const list = e.currentTarget as HTMLElement | null;
  queueMicrotask(() => list?.querySelectorAll<HTMLElement>('[role=tab]')[j]?.focus());
  return CLUSTER_VIEWS[j].id;
}

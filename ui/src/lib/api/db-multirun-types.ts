// Multi-target / parameterised DB runs ("Run on…") — mirrors
// `crates/otto-dbviewer/src/multirun/mod.rs` (contract: docs/contracts/api.md
// § "DB Explorer — multi-target runs"). Re-exported from `types.ts`.

import type { DbEngine, Environment, Id, QueryResult } from './types';

/** Per-target ClickHouse cluster choice (ignored for other engines). */
export type DbClusterMode = 'auto' | 'off' | 'custom';

/** One target: a connection, optionally scoped to a database / schema /
 *  Redis keyspace (`node`, same as `RunQueryReq.node`). */
export interface DbMultiRunTarget {
  connection_id: Id;
  node?: string | null;
  cluster_mode?: DbClusterMode;
  /** Required when `cluster_mode` is `custom`. */
  cluster_name?: string | null;
}

/** How a placeholder value is rendered — the editor's VarType. */
export type DbParamType = 'string' | 'number' | 'raw';

/** A placeholder (`:name` / `{name}` / `{{name}}`) and its values. One value =
 *  fixed; more = one run per value (cartesian product across parameters, and
 *  with every target). */
export interface DbMultiRunParam {
  name: string;
  values: string[];
  type?: DbParamType;
  /** Escape quotes/backslashes inside a `string` value (default true). */
  escape?: boolean;
}

/** Body of `POST /db/multi-run/plan`, and the core of a start request. */
export interface DbMultiRunSpec {
  statement: string;
  targets: DbMultiRunTarget[];
  params?: DbMultiRunParam[];
  /** Per-run row cap (default 500, max 10 000). */
  max_rows?: number | null;
  timeout_ms?: number | null;
  mask?: boolean | null;
  /** Refuse any write/DDL; run in the engine's native read-only mode. */
  read_only?: boolean;
}

/** Body of `POST /db/multi-runs`. */
export interface DbStartMultiRunReq extends DbMultiRunSpec {
  /** Parallel runs (1 = sequential, the default; max 8). */
  concurrency?: number;
  /** Stop dispatching after the first failure (default true). */
  stop_on_error?: boolean;
  /** Acknowledges the guarded (prod / read-only) writes — set only after the
   *  typed confirmation listing every target + final statement. */
  confirm_write?: boolean;
  /** The approved preview's `plan_hash`; a changed plan is refused (409 `plan_changed:`). */
  plan_hash?: string | null;
}

export type DbClusterSource =
  | 'macro'
  | 'system_clusters'
  | 'replicated_database'
  | 'ambiguous'
  | 'not_cluster'
  | 'probe_failed'
  | 'not_needed';

export interface DbTargetCluster {
  mode: DbClusterMode;
  detected: string | null;
  source: DbClusterSource;
  candidates: string[];
  database_engine: string | null;
  /** The cluster actually injected (`null` = no ON CLUSTER). */
  applied: string | null;
  note: string | null;
}

export interface DbPlannedTarget {
  index: number;
  connection_id: Id;
  connection_name: string;
  environment: Environment;
  read_only: boolean;
  guarded: boolean;
  node: string | null;
  label: string;
  cluster?: DbTargetCluster;
}

export interface DbParamValue {
  name: string;
  value: string;
}

export interface DbPlannedRun {
  index: number;
  target: number;
  values: DbParamValue[];
  label: string;
  /** The FINAL statement sent (placeholders substituted, ON CLUSTER injected). */
  statement: string;
  on_cluster?: string[];
  cluster_skipped?: string[];
  is_write: boolean;
  needs_confirm: boolean;
}

/** Response of `POST /db/multi-run/plan`. */
export interface DbMultiRunPlan {
  engine: DbEngine;
  placeholders: string[];
  targets: DbPlannedTarget[];
  runs: DbPlannedRun[];
  write_count: number;
  needs_confirm: boolean;
  warnings: string[];
  plan_hash: string;
}

export type DbRunStatus = 'pending' | 'running' | 'ok' | 'failed' | 'skipped' | 'cancelled';
export type DbMultiRunStatus = 'running' | 'done' | 'cancelled';

export interface DbMultiRunSummary {
  total: number;
  ok: number;
  failed: number;
  running: number;
  pending: number;
  skipped: number;
  cancelled: number;
}

export interface DbMultiRunItem {
  index: number;
  target: number;
  label: string;
  values: DbParamValue[];
  statement_preview: string;
  on_cluster?: string[];
  is_write: boolean;
  status: DbRunStatus;
  duration_ms?: number;
  row_count?: number;
  rows_affected?: number;
  message?: string;
  error?: string;
  has_result: boolean;
  result_dropped?: boolean;
}

/** `GET /db/multi-runs/{rid}` and the `POST /db/multi-runs` (202) response. */
export interface DbMultiRunJob {
  id: string;
  status: DbMultiRunStatus;
  engine: DbEngine;
  created_at: string;
  finished_at?: string;
  concurrency: number;
  stop_on_error: boolean;
  read_only: boolean;
  statement_preview: string;
  summary: DbMultiRunSummary;
  targets: DbPlannedTarget[];
  items: DbMultiRunItem[];
}

/** `GET /db/multi-runs` entries. */
export interface DbMultiRunBrief {
  id: string;
  status: DbMultiRunStatus;
  engine: DbEngine;
  created_at: string;
  finished_at?: string;
  statement_preview: string;
  target_count: number;
  summary: DbMultiRunSummary;
}

/** `GET /db/multi-runs/{rid}/items/{index}`. */
export interface DbMultiRunItemDetail {
  item: DbMultiRunItem;
  connection_id: Id;
  node: string | null;
  statement: string;
  result?: QueryResult;
}

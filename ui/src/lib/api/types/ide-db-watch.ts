// TS mirror of crates/otto-ide/src/db_watch.rs (POST /api/v1/ide/db-watch/detect
// and POST /api/v1/ide/db-watch/snapshot): live database access detection,
// saved + ephemeral Testcontainers target matching, and before/after row diffs.

export type IdeDbEngine = 'mysql' | 'postgres' | 'mongodb' | 'redis' | 'clickhouse';

export interface IdeDetectedDbAccess {
  engine: IdeDbEngine;
  object_name: string;
  kind: 'table' | 'collection' | 'key_prefix';
  operations: string[];
  database_hint?: string | null;
  source_file: string;
  line: number;
}

export interface IdeSavedDbTarget {
  target_kind: 'saved';
  connection_id: string;
  name: string;
  engine: IdeDbEngine;
  host: string;
  port: number;
  database?: string | null;
  environment?: string | null;
}

export interface IdeEphemeralDbTarget {
  target_kind: 'ephemeral';
  ephemeral_id: string;
  name: string;
  engine: IdeDbEngine;
  host: string;
  port: number;
  database?: string | null;
  source: 'testcontainers' | 'process';
  container_image?: string | null;
}

export type IdeDbWatchTarget = IdeSavedDbTarget | IdeEphemeralDbTarget;

export interface IdeDbWatchDetectReq {
  root: string;
  connection_id?: string | null;
  active_file?: string | null;
  additional_roots?: string[];
}

export interface IdeDbWatchDetectResp {
  detected_objects: IdeDetectedDbAccess[];
  saved_connections: IdeSavedDbTarget[];
  ephemeral_targets: IdeEphemeralDbTarget[];
}

export interface IdeDbWatchSnapshotItem {
  columns: string[];
  rows: unknown[][];
}

export interface IdeDbWatchSnapshotReq {
  root: string;
  target_kind: 'saved' | 'ephemeral';
  connection_id?: string | null;
  ephemeral_id?: string | null;
  engine: IdeDbEngine;
  database?: string | null;
  object_name: string;
  custom_query?: string | null;
  limit?: number;
  previous?: IdeDbWatchSnapshotItem | null;
}

export interface IdeDbFieldChange {
  column: string;
  before: unknown;
  after: unknown;
}

export interface IdeDbRowDiff {
  key: string;
  kind: 'added' | 'modified' | 'removed';
  row: unknown[];
  previous_row?: unknown[] | null;
  changes: IdeDbFieldChange[];
}

export interface IdeDbWatchDiffSummary {
  added_count: number;
  modified_count: number;
  removed_count: number;
  unchanged_count: number;
  diffs: IdeDbRowDiff[];
}

export interface IdeDbWatchSnapshotResp {
  engine: IdeDbEngine;
  target_label: string;
  object_name: string;
  executed_query: string;
  captured_at_ms: number;
  duration_ms: number;
  snapshot: IdeDbWatchSnapshotItem;
  diff?: IdeDbWatchDiffSummary | null;
}

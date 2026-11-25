// TS mirror of crates/otto-ide/src/db_watch (POST /api/v1/ide/db-watch/detect,
// and the watch engine): live database access detection,
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
  /** Only when the detect request named a target: is it a real object there? */
  verified?: boolean | null;
  /** Rank input: 100 paused frame's file, 60 other stack file, 40 the paused file's folder, 30 active file, 10 elsewhere. */
  score: number;
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
  /** The real endpoint is on this Mac: a loopback host with NO SSH tunnel (an
   *  SSH-tunnelled 127.0.0.1 is remote). Never auto-pick a saved target anyway. */
  local: boolean;
}

export interface IdeEphemeralDbTarget {
  target_kind: 'ephemeral';
  ephemeral_id: string;
  name: string;
  engine: IdeDbEngine;
  host: string;
  port: number;
  database?: string | null;
  /** `container`: a DB container without the Testcontainers label (maybe another project's). */
  source: 'testcontainers' | 'process' | 'container';
  container_image?: string | null;
}

export type IdeDbWatchTarget = IdeSavedDbTarget | IdeEphemeralDbTarget;

export interface IdeDbWatchDetectReq {
  root: string;
  connection_id?: string | null;
  active_file?: string | null;
  additional_roots?: string[];
  /** With a target, each detected object gets `verified` against its real objects. */
  target?: IdeDbWatchTargetRef | null;
  database?: string | null;
  /** Rank by the paused frame's file and the other stack files of this session. */
  debug_session_id?: string | null;
  /** A person picked / confirmed `target`: required for a saved target that is
   *  not on this Mac (listing it dials it); else 403. */
  confirm_target?: boolean;
}

export interface IdeDbWatchDetectResp {
  /** Ordered best first. `verified` is set only when the request named a target:
   *  `false` = the name is not an object of that target (hide it; count it in
   *  the "N names in code not found" footer). Redis prefixes are never `false`. */
  detected_objects: IdeDetectedDbAccess[];
  saved_connections: IdeSavedDbTarget[];
  ephemeral_targets: IdeEphemeralDbTarget[];
}

export interface IdeDbWatchSnapshotItem {
  columns: string[];
  rows: unknown[][];
}

export interface IdeDbFieldChange {
  column: string;
  before: unknown;
  after: unknown;
  /** The raw values differ but the wire carries masked ones (agent reads): show "changed · hidden". */
  masked?: boolean;
}

export interface IdeDbRowDiff {
  key: string;
  kind: 'added' | 'modified' | 'removed';
  row: unknown[];
  previous_row?: unknown[] | null;
  changes: IdeDbFieldChange[];
  /** At least one cell of `row` / `previous_row` is masked. */
  masked?: boolean;
}

export interface IdeDbWatchDiffSummary {
  added_count: number;
  modified_count: number;
  removed_count: number;
  unchanged_count: number;
  diffs: IdeDbRowDiff[];
}


// ── Watch engine (daemon-owned; /api/v1/ide/db-watch/watches) ─────────────
// The daemon keeps checkpoint 0 (the baseline), the previous and the latest
// checkpoint per item, diffs RAW values, and masks only what goes to an agent
// (`for_agent`, or any agent credential). A bound watch checkpoints itself on
// every debugger stop (`stop_seq` change) and when a run leaves `running`;
// the UI refetches on `ide_db_watch_checkpoint`.

/** `auto` = the newest live local Testcontainers / miniredis / local container of
 *  `engine` (never a saved connection); re-resolved when that container is gone. */
export type IdeDbWatchTargetRef =
  | { kind: 'saved'; connection_id: string }
  | { kind: 'ephemeral'; ephemeral_id: string }
  | { kind: 'auto'; engine: IdeDbEngine };

/** `column = value` (SQL) / an equality match (Mongo). `value`: a number, true/false,
 *  null (IS NULL) or text of letters, digits and _ . : @ - only (≤ 128) — anything else is
 *  a 400. Redis items have no filter: their `object` IS the key prefix or glob pattern. */
export interface IdeDbWatchFilter {
  column: string;
  value: string | number | boolean | null;
}

export interface IdeDbWatchItemSpec {
  /** Table / collection / Redis key prefix or pattern (`session:*`). */
  object: string;
  filter?: IdeDbWatchFilter | null;
}

export type IdeDbWatchBind = { debug_session_id: string } | { run_id: string };

export type IdeDbWatchCompare = 'previous_stop' | 'start';

/** POST /ide/db-watch/watches — one watch per (user, root): a new one replaces it. */
export interface IdeDbWatchStartReq {
  root: string;
  target: IdeDbWatchTargetRef;
  database?: string | null;
  /** 1–8 items. */
  items: IdeDbWatchItemSpec[];
  bind?: IdeDbWatchBind | null;
  /** Required (from a person, never an agent credential) for a prod-tagged target; never remembered. */
  confirm_production?: boolean;
  /** Required (from a person) for a saved target that is not on this Mac: starting a watch dials it. */
  confirm_target?: boolean;
}

/** PATCH /ide/db-watch/watches/{id} — every field optional. */
export interface IdeDbWatchUpdateReq {
  /** Replaces the item list; items kept by `object` keep their checkpoints. */
  items?: IdeDbWatchItemSpec[] | null;
  /** Rebind to a new debug session / run (the old binding is dropped). */
  bind?: IdeDbWatchBind | null;
  confirm_production?: boolean;
}

/** POST /ide/db-watch/watches/{id}/snapshot */
export interface IdeDbWatchSnapshotNowReq {
  label?: string | null;
  compare?: IdeDbWatchCompare;
  for_agent?: boolean;
}

export interface IdeDbWatchTargetInfo {
  kind: 'saved' | 'ephemeral';
  connection_id?: string | null;
  ephemeral_id?: string | null;
  engine: IdeDbEngine;
  label: string;
  /** Saved targets: the connection's environment (`prod` shows in --danger). */
  environment?: string | null;
  /** Ephemeral targets, or a saved loopback endpoint without an SSH tunnel. */
  local: boolean;
}

export interface IdeDbWatchItemInfo {
  object: string;
  kind: 'table' | 'collection' | 'key_prefix';
  filter?: IdeDbWatchFilter | null;
  /** The snapshot orders by these DESC; [] = no stable order (the window note says so). */
  primary_key: string[];
  row_estimate?: number | null;
  /** Describe failed (e.g. "not found in pr_bo_1"); checkpoints still retry it. */
  error?: string | null;
}

export type IdeDbWatchCheckpointReason = 'start' | 'stop' | 'run_end' | 'session_end' | 'manual';

export interface IdeDbWatchCheckpointInfo {
  /** 0 = the baseline. */
  seq: number;
  reason: IdeDbWatchCheckpointReason;
  label?: string | null;
  /** Where the debuggee stopped (`reason: 'stop'`). */
  file?: string | null;
  line?: number | null;
  captured_at_ms: number;
  duration_ms: number;
  /** Against the previous checkpoint, over all items. */
  changed_rows: number;
  changed_items: number;
  failed_items: number;
}

export interface IdeDbWatch {
  id: string;
  root: string;
  target: IdeDbWatchTargetInfo;
  database?: string | null;
  items: IdeDbWatchItemInfo[];
  bind?: IdeDbWatchBind | null;
  /** Set when the bound session / run ended; the watch is removed 10 min later. */
  bound_ended_at_ms?: number | null;
  checkpoint_count: number;
  latest?: IdeDbWatchCheckpointInfo | null;
  /** A checkpoint is being taken now (another one requested meanwhile is coalesced into one). */
  busy: boolean;
  created_at_ms: number;
}

export interface IdeDbWatchItemDiff {
  object: string;
  engine: IdeDbEngine;
  executed_query: string;
  /** Honest window: "all 37 rows", "newest 150 by id of ~1.2k — older rows not compared",
   *  "150 rows, no primary key — order not stable". */
  window_note: string;
  /** The read hit the 150-row cap. */
  window_full: boolean;
  columns: string[];
  added_count: number;
  modified_count: number;
  removed_count: number;
  unchanged_count: number;
  /** ≤ 150; a modified row's `changes` lists the changed cells only. */
  diffs: IdeDbRowDiff[];
  more_count: number;
  /** The latest checkpoint's rows (`?rows=1` / with a single checkpoint: "rows at end of run"). */
  current?: IdeDbWatchSnapshotItem | null;
  error?: string | null;
}

/** GET /ide/db-watch/watches/{id}?compare=&rows=&for_agent= and POST …/snapshot */
export interface IdeDbWatchReadResp {
  watch: IdeDbWatch;
  compare: IdeDbWatchCompare;
  /** null while only one checkpoint exists (nothing to compare). */
  from_seq: number | null;
  to_seq: number | null;
  items: IdeDbWatchItemDiff[];
}

/** GET /ide/db-watch/watches?root= */
export interface IdeDbWatchListResp {
  watches: IdeDbWatch[];
}

/** POST /ide/db-watch/objects — the target's real watchable objects ("Add table…"). */
export interface IdeDbWatchObjectsReq {
  root: string;
  target: IdeDbWatchTargetRef;
  database?: string | null;
  /** As on detect: required for a saved target that is not on this Mac. */
  confirm_target?: boolean;
}

export interface IdeDbWatchObject {
  name: string;
  kind: 'table' | 'collection' | 'key_prefix';
}

export interface IdeDbWatchObjectsResp {
  target: IdeDbWatchTargetInfo;
  /** The database / schema the names belong to (Postgres: the schema). */
  database?: string | null;
  objects: IdeDbWatchObject[];
  /** More than 500 objects; the rest are not listed. */
  truncated: boolean;
}

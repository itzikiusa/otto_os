// Contract mirror: database. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';
import type {
  WorkflowGraph,
} from './workflows';

// ---------------------------------------------------------------------------
// Database Explorer (DB module) — mirrors crates/otto-* DB endpoints.
// ---------------------------------------------------------------------------

/** Database engines the explorer can talk to (subset of ConnectionKind). */
export type DbEngine = 'mysql' | 'postgres' | 'redis' | 'mongodb' | 'clickhouse';

/** Schema-tree node taxonomy across SQL / Redis / Mongo. */
export type DbNodeKind =
  | 'database'
  | 'schema'
  | 'table'
  | 'view'
  | 'procedure'
  | 'function'
  | 'trigger'
  | 'column'
  | 'index'
  | 'collection'
  | 'field'
  | 'keyspace'
  | 'key_namespace'
  | 'key'
  | 'folder';

/** One lazy node in the schema tree (databases → tables → columns, etc.). */
export interface SchemaNode {
  id: string;
  label: string;
  kind: DbNodeKind;
  /** Dimmed suffix — a type, or an opt-in row ESTIMATE like `1.3K`. */
  detail?: string;
  has_children: boolean;
}

/** POST /connections/{id}/db/search-objects */
export interface ObjectSearchReq {
  q: string;
  schema?: string;
  /** `schema` (default, cheap) | `all` (sweeps everything reachable). */
  scope?: 'schema' | 'all';
  /** Reserved for column search; empty = browsable objects. */
  kinds?: string[];
  limit?: number;
}

export interface ObjectHit {
  schema: string;
  name: string;
  kind: DbNodeKind;
  /** Node-path id — revealing a hit uses the same navigation as a browsed node. */
  path: string;
}

export interface ObjectSearchResult {
  hits: ObjectHit[];
  truncated: boolean;
  /** Schemas actually inspected — the real cost of an `all` sweep on MongoDB. */
  scanned: number;
  /** `false` when the engine has no object namespace (Redis). */
  supported: boolean;
}

export interface DbColumnDef {
  name: string;
  data_type: string;
  nullable: boolean;
  default?: string | null;
  key?: string | null;
  extra?: string | null;
  comment?: string | null;
  /** Per-column collation when the engine reports one (MySQL string columns);
   *  the Table Designer re-states it on CHANGE COLUMN. */
  collation?: string | null;
}

export interface DbIndexDef {
  name: string;
  columns: string[];
  unique: boolean;
  method?: string | null;
  /**
   * Full engine-native definition when available: Mongo = the raw listIndexes
   * document (partialFilterExpression, collation, TTL…), Postgres = the
   * pg_get_indexdef DDL string.
   */
  definition?: unknown;
}

export interface DbForeignKey {
  name: string;
  columns: string[];
  ref_table: string;
  ref_columns: string[];
  ref_schema?: string | null;
}

/** Full detail for a selected object (table / view / collection / key). */
export interface ObjectDetail {
  name: string;
  kind: DbNodeKind;
  columns: DbColumnDef[];
  primary_key: string[];
  indexes: DbIndexDef[];
  foreign_keys: DbForeignKey[];
  ddl?: string | null;
  row_count?: number | null;
  /** Enforced Mongo collection browsing retains only sampled_fields, sampled_paths
   * and sampled_path_types (schema names/types), never sampled document values. */
  extra?: unknown;
}

/** One column on an ERD diagram card (trimmed ColumnDef + PK/FK flags). */
export interface DbGraphColumn {
  name: string;
  data_type: string;
  nullable: boolean;
  primary_key: boolean;
  foreign_key: boolean;
}

/** A table/view/collection node in the schema graph (an ERD card). */
export interface DbGraphTable {
  /** Opaque NodePath id (e.g. `db:shop/table:orders`) for opening elsewhere. */
  id: string;
  schema: string;
  name: string;
  kind: DbNodeKind;
  columns: DbGraphColumn[];
}

/** A foreign-key relationship between two graph tables (an ERD edge). */
export interface DbGraphEdge {
  name: string;
  from_table: string;
  from_columns: string[];
  to_schema: string;
  to_table: string;
  to_columns: string[];
}

/** The relationship graph (ERD) for one schema: tables + FK edges. Engines
 *  without FK metadata (Redis/Mongo) return `relationships: false` + no edges. */
export interface DbSchemaGraph {
  schema: string;
  tables: DbGraphTable[];
  edges: DbGraphEdge[];
  relationships: boolean;
  truncated: boolean;
}

/** A column in a query result set. */
export interface DbColumn {
  name: string;
  type_hint?: string | null;
}

export interface QueryStats {
  duration_ms: number;
  row_count: number;
  bytes_read?: number | null;
}

/** Response of `POST /connections/{id}/db/cancel` — mirrors `CancelOutcome` /
 *  `CancelStatus` in `crates/otto-dbviewer/src/service.rs`. Only `cancelled`
 *  means the database stopped the work; `aborted` means Otto dropped its side
 *  (mongosh killed, no further statement sent) while a statement already on
 *  the server may still complete; `not_stoppable` means it keeps running. */
export type DbCancelStatus = 'cancelled' | 'aborted' | 'not_running' | 'not_stoppable';
export interface DbCancelOutcome {
  status: DbCancelStatus;
}

/** Body of `POST /connections/{id}/db/query` — mirrors `QueryRequest` in
 *  `crates/otto-dbviewer/src/types.rs` (contract §"Database Explorer"). Every
 *  field but `statement` is `#[serde(default)]` on the wire, so the client only
 *  sends the ones that apply to the run. */
export interface RunQueryReq {
  statement: string;
  /** Soft cap on returned rows (the auto-limiter's `n`). */
  max_rows?: number | null;
  /** Node context to scope execution — the active database / Redis keyspace. */
  node?: string | null;
  /** Explicit acknowledgement of a write/DDL on a guarded (production /
   *  read-only) connection; set only after the typed confirmation. */
  confirm_write?: boolean;
  /** Client-supplied id so `…/db/cancel` can issue engine-native cancellation. */
  query_id?: string | null;
  /** Per-statement wall-clock timeout in ms (0 / absent = no limit). */
  timeout_ms?: number | null;
  /** Redact cell values server-side before they leave the daemon. */
  mask?: boolean | null;
  /** Zero-based row offset — applied only to an auto-limited single statement. */
  offset?: number | null;
  /** Keyset cursor: the previous page's {@link QueryResult.next_cursor} echoed
   *  back verbatim. Applied only to a keyset-eligible MongoDB `find` (where it
   *  replaces `skip`); ignored everywhere else, so it may always ride along
   *  with `offset`. Opaque Extended-JSON — never inspected client-side. */
  cursor?: unknown;
  /** Return the query plan instead of running it (Mongo: server `explain`). */
  explain?: boolean;
  /** Engine-specific positional/named params (unused by the Explorer UI). */
  params?: unknown;
  /** Run read-only whatever the connection's write-guard: a write/DDL is
   *  refused (403, `Problem.message` prefixed `read_only: `) before the driver
   *  is touched, and the rest runs in the engine's native read-only mode —
   *  without the MCP path's forced mask / 200-row cap. Set on agent-driven
   *  runs; a refusal is the cue to ask the human, then re-run without it.
   *  Wins over `confirm_write`. */
  read_only?: boolean;
}

/** Result of running a statement: tabular rows + stats.
 *
 *  For a **multi-statement batch** the top-level fields describe the FIRST
 *  statement; each later statement is one entry in `more_results` (in order),
 *  and every batch entry carries a `statement` preview label. A single-statement
 *  run omits all of these (back-compat: byte-identical to before). */
export interface QueryResult {
  columns: DbColumn[];
  rows: unknown[][];
  rows_affected?: number | null;
  stats: QueryStats;
  message?: string | null;
  truncated: boolean;
  /** Why the result was clipped when it was NOT the row cap: `bytes` = the
   *  response reached the daemon's 32 MB budget (estimated JSON) first. Absent
   *  for an unclipped result or a row-cap clip. */
  truncated_reason?: 'bytes' | null;
  /** True when the server ran cell values through `otto_core::redact` (QueryRequest.mask=true). */
  masked?: boolean;
  /** Stored cell values were shortened for display; row edits/copy-as-insert must be disabled. */
  cells_truncated?: boolean;
  /** Later result sets from a multi-statement batch, in execution order (the
   *  top-level fields are the first statement's result). Absent/empty for a
   *  single statement. */
  more_results?: QueryResult[];
  /** A ≤80-char single-line preview of the statement that produced THIS result —
   *  set only for the entries of a multi-statement batch (labels the switcher). */
  statement?: string | null;
  /** True when this batch entry is the statement that FAILED: execution stopped
   *  here and `message` holds the engine error. A single-statement failure is an
   *  HTTP error instead, never this flag. */
  errored?: boolean;
  /** The `LIMIT` the server auto-injected for an unconstrained single SELECT
   *  (Mongo: an unconstrained `find`). Present ⇔ the result was auto-paginated,
   *  so the UI shows its pager exactly then; absent for explicit user
   *  LIMIT/OFFSET, non-paginatable statements, and batches. */
  auto_limited?: number | null;
  /** Keyset-pagination cursor for the NEXT page (MongoDB only): the Extended-JSON
   *  `_id` of the last row returned (`{"$oid": …}`, a number, a string…).
   *  Present only when the page was keyset-eligible — an unconstrained `find`
   *  with no `_id` in the filter and no sort or `{_id: 1}` — AND the auto-limit
   *  truncated it. Echo it back verbatim as `cursor` on the next-page request;
   *  absent ⇒ page by `offset` as before. */
  next_cursor?: unknown;
}

/** One node in a normalized query plan (`POST …/db/query-plan`). `warnings`
 *  flags costly access patterns the UI badges in red (full scans, filesort,
 *  temporary tables). */
export interface DbPlanNode {
  /** The operation (`Seq Scan`, `table`, `IXSCAN`, `ReadFromMergeTree`, …). */
  op: string;
  /** The object it touches (table / collection / index), when known. */
  object?: string | null;
  /** A short human detail line (access type, filter, condition), when known. */
  detail?: string | null;
  /** Estimated rows for this node, when the engine reports one. */
  est_rows?: number | null;
  /** Costly-pattern flags (e.g. "full table scan", "Using filesort"). */
  warnings?: string[];
  /** Child plan nodes. */
  children?: DbPlanNode[];
}

/** A normalized query plan distilled from an engine's native EXPLAIN
 *  (`POST /connections/{id}/db/query-plan`). `raw` is the engine's untouched
 *  EXPLAIN JSON (for a "raw" toggle). Consumed by the plan-tree UI (Task 10). */
export interface DbQueryPlan {
  /** `mysql` | `postgres` | `clickhouse` | `mongodb`. */
  engine: string;
  root: DbPlanNode;
  raw: unknown;
}

/** `GET /db/mongosh` — whether the `mongosh` CLI (used to execute pasted
 *  mongosh SCRIPTS and by Mongo terminal sessions) is on the daemon's PATH.
 *  The query editor probes this when it detects a script, so a missing binary
 *  surfaces as an inline install hint before the run. Mirrors `MongoshInfo`
 *  in `crates/otto-dbviewer/src/types.rs`. */
export interface MongoshInfo {
  available: boolean;
  version?: string;
}

/**
 * Body for the read-only MCP query endpoint
 * (`POST /api/v1/connections/{id}/db/mcp-query`) — the agent-facing query path used
 * by `ottod mcp-tools`. Writes/DDL are refused server-side (`mcp_read_only:`); rows
 * are hard-capped (200) and PII-masked. Mirrors `McpQueryReq` in
 * `crates/otto-dbviewer/src/http.rs`.
 */
export interface McpDbQueryReq {
  statement: string;
  max_rows?: number | null;
  node?: string | null;
}

/** Selectable output format for the streaming local-file export. */
export type DbExportFormat =
  | 'csv'
  | 'csv_with_names'
  | 'tsv'
  | 'tsv_with_names'
  | 'json'
  | 'ndjson';

/**
 * `POST /connections/{id}/db/export-to-path` — stream an uncapped result to a
 * local file on the daemon host (selectable format, configurable path). The
 * daemon streams the result row/chunk-by-chunk so its memory stays bounded
 * regardless of size.
 */
export interface ExportToPathReq {
  statement: string;
  /** Active database to scope unqualified names (same as a query's `node`). */
  node?: string | null;
  /** Output format; default `csv`. */
  format?: DbExportFormat;
  /**
   * Destination on the daemon host. A leading `~` expands to the daemon user's
   * home. An existing directory → `<dir>/export.<ext>`; otherwise a full file
   * path whose parent dir is created.
   */
  local_path: string;
  /** Optional row cap; blank/absent = all rows. */
  max_rows?: number | null;
}

/** `POST /connections/{id}/db/export-to-path` response. */
export interface ExportToPathResp {
  /** The absolute file path actually written. */
  local_path: string;
  /** Rows written. */
  rows: number;
  /** Bytes written to the file. */
  bytes: number;
  /** Wall-clock duration of the export, in milliseconds. */
  duration_ms: number;
}

/** Supported import file formats (the mirror of the export). Delimited formats
 *  take the first row as the header; JSON/NDJSON carry keys per object. */
export type ImportFormat = 'csv' | 'tsv' | 'ndjson' | 'json';

/**
 * `POST /connections/{id}/db/import` — import a local file (on the daemon host)
 * into an existing table/collection. Guarded like a query, so a Prod/read-only
 * connection refuses the import unless `confirm_write` is set (the typed-
 * confirmation flow re-sends it). MySQL/ClickHouse import as batched `INSERT`s;
 * **MongoDB** imports as `insertMany` batches (`table` = the collection; CSV/TSV
 * string cells are coerced to numbers/bools/null). Redis has no bulk-load surface.
 */
export interface ImportReq {
  /** Source file on the daemon host (a leading `~` expands to the daemon home). */
  local_path: string;
  /** File format. */
  format: ImportFormat;
  /** Destination table (must already exist). */
  table: string;
  /** Rows per INSERT; clamped 1..=5000 server-side (default 500). */
  batch_size?: number;
  /** Typed-confirmation acknowledgement for a guarded (Prod/read-only) connection. */
  confirm_write?: boolean;
}

/** One streamed NDJSON line from `POST …/db/import`. The final line carries
 *  `done` (with `rows`/`batches`) or `error` (text starting `write_blocked:`
 *  means a guarded connection needs the typed confirmation). */
export interface ImportResult {
  done?: boolean;
  rows?: number;
  batches?: number;
  error?: string;
}

/**
 * `POST /connections/{id}/db/nl-to-sql` — draft a read query from plain English,
 * validated with `EXPLAIN` against the live schema before it's returned. Never
 * emits a write/DDL. A 400 starting "NL-to-SQL is not configured" means no
 * drafter is wired; a 400 starting "could not produce a valid read query" means
 * the bounded retry loop was exhausted (its message carries the last engine error).
 */
export interface NlToSqlReq {
  /** The user's plain-English question. */
  question: string;
  /** Optional active-database node (same semantics as a query's `node`). */
  node?: string;
  /** Draft/validate retries; clamped 1..=4 server-side (default 3). */
  max_attempts?: number;
}

/** A validated read query: the SQL, its plan text (from EXPLAIN), how many
 *  drafting attempts it took, and any non-fatal notes. */
export interface NlToSqlOutcome {
  sql: string;
  plan: string;
  attempts: number;
  warnings: string[];
}

/** Entry mode for the embedded DB Assistant panel:
 *  - `nl`          — "Ask in English": produce a runnable query for the question.
 *  - `ask`         — "Ask AI": free-form question about the data/schema.
 *  - `investigate` — examine a result/object (seeded with the statement + a small
 *                    sample of the result columns/rows via `result_context`). */
export type DbAssistMode = 'nl' | 'ask' | 'investigate';

/**
 * `POST /connections/{id}/db/assist` — run ONE turn of the file-backed, embedded
 * DB Assistant. The agent runs as a managed Otto session in an ephemeral, trusted
 * working dir seeded with the full schema + a read-only `q` tool; it never touches
 * the DB directly. Returns the assist id (resume key), the live agent session id
 * (the panel mounts a `<Terminal>` on it), and the agent's proposed SQL + note.
 */
export interface DbAssistReq {
  /** The user's question / instruction for this turn. */
  question: string;
  /** Entry mode (defaults to `ask` server-side). */
  mode?: DbAssistMode;
  /** Active-database node to scope the schema/queries (same as a query's `node`). */
  node?: string;
  /** Agent CLI to run (claude/codex/…); defaults to the workspace/global default. */
  provider?: string;
  /** investigate-mode seed: the current statement + a small result sample. */
  result_context?: string;
  /** Resume an existing assist (its session) instead of starting a new one. */
  assist_id?: string;
}

/** The result of one DB Assistant turn. */
export interface DbAssistResp {
  /** Stable id for this assist (resume key; also the DELETE/summary path segment). */
  assist_id: Id;
  /** The live agent session id — the panel binds `<Terminal>` to `/ws/term/{id}`. */
  session_id: Id;
  /** The agent's current proposed SQL (its ANSWER.sql), or empty. */
  sql: string;
  /** A one-line explanation/note from the agent, or empty. */
  note: string;
}

/** `POST /connections/{id}/db/assist/{aid}/summary` — the rendered investigation
 *  summary (the panel downloads it as a `.md` file). */
export interface DbAssistSummaryResp {
  markdown: string;
}

/** `POST /connections/{id}/db/close` — tear down a connection's live server-side
 *  resources when its workbench tab closes: drops the SSH tunnel, closes pooled
 *  engine connections, and cancels any in-flight queries. Idempotent — closing a
 *  connection with nothing open is a no-op (`closed: false`). */
export interface DbCloseResp {
  /** True when a live pool/tunnel existed and was torn down. */
  closed: boolean;
  /** How many in-flight queries the server cancelled as part of the close. */
  cancelled_queries?: number;
}

export type DbCompletionKind =
  | 'keyword'
  | 'function'
  | 'table'
  | 'view'
  | 'column'
  | 'database'
  | 'collection'
  | 'field'
  | 'command'
  | 'operator';

export interface DbCompletionItem {
  label: string;
  kind: DbCompletionKind;
  detail?: string | null;
  insert_text?: string | null;
  /**
   * Ranking hint mapped to CodeMirror's `boost`. Higher sorts earlier among
   * equally-matching options — index columns/fields out-rank plain ones, and
   * tables out-rank keywords in a slot where they're expected.
   */
  score?: number | null;
}

/** What a given engine supports — drives the UI affordances. */
export interface DbCapabilities {
  engine: DbEngine;
  sql: boolean;
  joins: boolean;
  /** Session-pinned transactions (BEGIN…COMMIT across calls). False for the
   *  pooled engines — each run acquires an independent connection. */
  transactions: boolean;
  multi_statement: boolean;
  /** Server-side cancel of an in-flight query: MySQL/ClickHouse/Postgres/MongoDB
   *  (Mongo: the run is tagged `comment: "otto:<query_id>"`, cancel resolves it
   *  via `$currentOp` + `killOp`; silently a no-op on a server that denies
   *  `inprog`/`killop`). When false (Redis) the Stop button is client-side only. */
  cancel?: boolean;
  /** The engine can produce a query plan (drives the Explain button; false for
   *  Redis, which has no plan surface). */
  explain?: boolean;
  default_port: number;
  schema_levels: string[];
  query_language: 'sql' | 'redis' | 'mongo';
}

export interface DbTestResult {
  ok: boolean;
  latency_ms?: number | null;
  message: string;
  server_version?: string | null;
}

export interface DbSavedQuery {
  id: string;
  workspace_id: string;
  connection_id?: string | null;
  name: string;
  statement: string;
  created_by: string;
  created_at: string;
}

export interface DbHistoryEntry {
  id: string;
  connection_id: string;
  statement: string;
  ok: boolean;
  duration_ms: number;
  row_count: number;
  error?: string | null;
  created_at: string;
  /** Present when a history LIST clipped `statement` to a 16 KiB preview: the
   *  full length (chars). Fetch `GET …/db/history/{id}` for the whole text. */
  statement_len?: number;
}

/** `POST …/db/completion` reply. `truncated` (omitted when false) means the
 *  daemon capped the list for the typed word — re-ask on the next keystroke. */
export interface DbCompletionResponse {
  items: DbCompletionItem[];
  truncated?: boolean;
}

/** Supported widget visualizations. */
export type DbViz = 'table' | 'line' | 'bar' | 'area' | 'pie' | 'number';

/** Maps result columns onto a chart's axes/series. */
export interface DbWidgetMapping {
  x?: string;
  y?: string[];
  category?: string;
  value?: string;
}

export interface DbWidget {
  id: string;
  workspace_id: string;
  dashboard_id?: string | null;
  connection_id: string;
  title: string;
  statement: string;
  viz: DbViz;
  mapping: DbWidgetMapping;
  options: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

/** One tile placement on a dashboard grid. */
export interface DbLayoutItem {
  widget_id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface DbDashboard {
  id: string;
  workspace_id: string;
  name: string;
  layout: DbLayoutItem[];
  refresh_secs?: number | null;
  created_at: string;
  updated_at: string;
}

/** A ready-made example workflow (e.g. a game pipeline: agent design + engine). */
export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  instructions: string;
  icon: string;
  graph: WorkflowGraph;
}

// ── DB Explorer: re-attach probe (POST /connections/{id}/db/query-status) ──
/** `QueryStatus` (otto-dbviewer types.rs). `elapsed_ms` only while running. */
export interface DbQueryStatus {
  status: 'running' | 'done' | 'unknown';
  result?: QueryResult;
  error?: string;
  elapsed_ms?: number;
}

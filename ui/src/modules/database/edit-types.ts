// Engine adapters for the inline-edit flow. `EditFlow` decides WHEN a statement
// is built (pending cells, selection, duplicate, doc save); the adapter for the
// active engine decides WHAT it looks like (UPDATE vs `$set`, `ALTER TABLE …
// DELETE` vs `deleteMany`, …). Adapters are pure: every input arrives in the
// ctx, nothing reads the store.
import type { DbEngine } from '../../lib/api/types';
import { sqlAdapter } from './edit-sql';
import { mongoAdapter } from './edit-mongo';
import { redisAdapter } from './edit-redis';

/** Typed values from the nested-field editor (Vertical view): `raw` is the
 *  editor text, validated per `kind` (24-hex ObjectId, ISO date, digit-only
 *  long, …); the Mongo adapter turns it into the matching EJSON sentinel. */
export type TypedKind = 'string' | 'number' | 'bool' | 'null' | 'objectId' | 'date' | 'long' | 'decimal' | 'json';
export interface TypedValue {
  kind: TypedKind;
  raw: string;
}
/** Everything parked for one row: whole-cell drafts (`cells`, by column index —
 *  the grid / cell viewer) plus the Vertical editor's path-level ops on dotted
 *  paths relative to the row (`set` typed values, `unset`, `rename` old → new).
 *  For SQL engines only `cells` is ever populated: a nested edit inside a JSON
 *  column is folded into a whole-column draft (see `EditFlow.parkPath`). */
export interface RowPatch {
  cells: Map<number, string>;
  set: Map<string, TypedValue>;
  unset: Set<string>;
  rename: Map<string, string>;
}
/** What a Vertical-view field row hands to the field context menu. */
export interface FieldCtx {
  rowIdx: number;
  /** Column the path lives in; -1 for a new top-level field (Mongo only). */
  colIdx: number;
  /** Dotted path relative to the row; the column name is the first segment. */
  path: string;
  /** Last path segment (the key / index shown as the row label). */
  label: string;
  /** Live value at the path (undefined for a phantom row of a pending set). */
  value: unknown;
  container: boolean;
  /** The row IS a result column (SQL: can't be removed / renamed). */
  topLevel: boolean;
  /** Any numeric segment — Mongo can't `$rename` through arrays. */
  inArray: boolean;
  /** Open the row's inline editor (leaves only). */
  edit: () => void;
}
/** One line of the review modal's diff table (`∅` for an absent side). */
export interface DiffLine {
  row: number;
  path: string;
  op: 'set' | 'unset' | 'rename' | 'cell';
  before: string;
  after: string;
}

/** The table/collection a result's rows are attributed to. `pkCols` is empty
 *  until the (SQL) primary-key lookup resolves. */
export interface EditTarget {
  db: string | null;
  table: string;
  pkCols: string[];
  reason?: never;
}
export interface EditCtx {
  engine: DbEngine | null;
  columns: { name: string; type_hint?: string | null }[];
  liveRows: unknown[][];
  target: EditTarget;
  /** Identifier quoter for the active engine (double quotes on Postgres). */
  qid: (s: string) => string;
}

export interface EditAdapter {
  /** Synchronous part of target resolution: parse the statement into a
   *  table/collection (+ the key columns it can already name). The SQL
   *  primary-key lookup stays async in `EditFlow.resolveTarget`. Exactly one of
   *  `target` / `reason` is set. */
  target(
    statement: string,
    columns: string[],
    ctx: { engine: DbEngine | null; activeDb: string | null },
  ): { target: EditTarget | null; reason: string | null };
  /** ONE statement per pending row, joined by newlines (multi-row → batch). */
  buildUpdate(
    rows: { rowIdx: number; patch: RowPatch }[],
    ctx: EditCtx,
  ): { title: string; sql: string; diff: DiffLine[] } | null;
  buildDelete(idxs: number[], ctx: EditCtx): { title: string; sql: string } | null;
  /** "Copy as INSERT": insert statements for the given rows, every column. */
  buildInsert(idxs: number[], ctx: EditCtx): string | null;
  /** "Duplicate row": an INSERT / insertOne cloning one row, identity omitted so
   *  it regenerates (single PK / `_id`). Differs from `buildInsert` on purpose. */
  buildDuplicate(rowIdx: number, ctx: EditCtx): { title: string; sql: string } | null;
  /** Insert from a JSON document (toolbar "Insert from JSON…"). */
  buildInsertDoc?(doc: Record<string, unknown>, ctx: EditCtx): string | null;
  /** Whole-document save (doc editor): replaceOne / per-changed-column UPDATE. */
  buildReplace?(
    rowIdx: number,
    doc: Record<string, unknown>,
    ctx: EditCtx,
  ): { title: string; sql: string; diff: DiffLine[] } | null;
}

/** mongodb → mongoAdapter, redis → redisAdapter, everything else (incl. an
 *  unknown/null engine) → sqlAdapter. */
export function adapterFor(engine: DbEngine | null): EditAdapter {
  if (engine === 'mongodb') return mongoAdapter;
  if (engine === 'redis') return redisAdapter;
  return sqlAdapter;
}

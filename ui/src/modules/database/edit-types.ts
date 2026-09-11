// Engine adapters for the inline-edit flow. `EditFlow` decides WHEN a statement
// is built (pending cells, selection, duplicate, doc save); the adapter for the
// active engine decides WHAT it looks like (UPDATE vs `$set`, `ALTER TABLE …
// DELETE` vs `deleteMany`, …). Adapters are pure: every input arrives in the
// ctx, nothing reads the store.
import type { DbEngine } from '../../lib/api/types';
import { sqlAdapter } from './edit-sql';
import { mongoAdapter } from './edit-mongo';
import { redisAdapter } from './edit-redis';

/** Typed values for the nested-field editors (Vertical view). Phase 0 declares
 *  the shape only — `pendingEdits` still parks raw cell strings. */
export type TypedKind = 'string' | 'number' | 'bool' | 'null' | 'objectId' | 'date' | 'long' | 'decimal' | 'json';
export interface TypedValue {
  kind: TypedKind;
  raw: string;
}
/** Everything parked for one row: whole-cell drafts (`cells`, by column index)
 *  plus the path-level ops the Vertical editor adds later. */
export interface RowPatch {
  cells: Map<number, string>;
  set: Map<string, TypedValue>;
  unset: Set<string>;
  rename: Map<string, string>;
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

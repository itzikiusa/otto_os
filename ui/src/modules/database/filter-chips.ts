// Quick-filter chips: the value model (`FilterVal` / `FilterCond`), how a
// clicked cell becomes a value, how a chip renders as SQL for the active
// engine, and how a chip matches a cell client-side. Extracted from the
// database store (which re-exports everything here) so it is unit-testable and
// so the SQL rendering goes through the ONE dialect module (`sql-dialect.ts`)
// shared with the edit builders and "Query by value".
import { bsonScalar } from './bson.ts';
import { boolLiteral, quoteIdent, stringLiteral, type EngineLike } from './sql-dialect.ts';

/** A single value in a column filter condition. */
export interface FilterVal {
  /** Literal text (already SQL-unquoted); rendered quoted unless `numeric`. */
  raw: string;
  numeric: boolean;
  isNull: boolean;
  /** Set when the value came from a BOOLEAN cell: renders as the engine's
   *  boolean literal (TRUE/FALSE on Postgres, 1/0 elsewhere) and matches cells
   *  by value, not text. `raw` is then `'true'` / `'false'` (the cell's text).
   *  Absent on chips persisted before it existed (those carry `'1'`/`'0'`). */
  bool?: boolean;
}
/**
 * A quick-filter condition. `col` conditions group all values for one column +
 * direction so repeated equals collapse into IN / NOT IN. `raw` preserves a
 * pre-existing hand-written WHERE as a removable chip.
 */
export type FilterCond =
  | { kind: 'col'; column: string; op: 'in' | 'not_in'; values: FilterVal[] }
  | { kind: 'raw'; text: string };

/** Derive a filter value from a result cell value. */
export function toFilterVal(value: unknown): FilterVal {
  if (value === null || value === undefined) return { raw: 'NULL', numeric: false, isNull: true };
  if (typeof value === 'number' || typeof value === 'bigint')
    return { raw: String(value), numeric: true, isNull: false };
  if (typeof value === 'boolean') return { raw: String(value), numeric: false, isNull: false, bool: value };
  if (typeof value === 'object') {
    // A BSON sentinel filters by its display form (ObjectId("…")/ISODate("…")),
    // matching how the cell renders, so a "Filter: _id = …" actually narrows.
    const b = bsonScalar(value);
    if (b !== null) return { raw: b, numeric: false, isNull: false };
    return { raw: JSON.stringify(value), numeric: false, isNull: false };
  }
  return { raw: String(value), numeric: false, isNull: false };
}

/** Parse a value typed into the filter bar (numbers stay bare, NULL → IS NULL). */
export function parseFilterValText(text: string): FilterVal {
  const t = text.trim();
  if (t.toUpperCase() === 'NULL') return { raw: 'NULL', numeric: false, isNull: true };
  if (/^-?\d+(\.\d+)?$/.test(t)) return { raw: t, numeric: true, isNull: false };
  return { raw: text, numeric: false, isNull: false };
}

function quoteFilterVal(v: FilterVal, engine: EngineLike): string {
  if (typeof v.bool === 'boolean') return boolLiteral(engine, v.bool);
  if (v.numeric) return v.raw;
  return stringLiteral(engine, v.raw);
}

/** Render one filter condition as a SQL boolean expression for `engine` (empty
 * when it has no usable values). Equals collapse to `IN`; NULLs become
 * `IS [NOT] NULL`. Identifier quoting, string escaping and boolean literals
 * follow the engine (`sql-dialect.ts`); no engine = MySQL style (labels). */
export function condToSql(c: FilterCond, engine: EngineLike = null): string {
  if (c.kind === 'raw') return c.text.trim();
  const col = quoteIdent(engine, c.column);
  const quote = (v: FilterVal): string => quoteFilterVal(v, engine);
  const nonNull = c.values.filter((v) => !v.isNull);
  const hasNull = c.values.some((v) => v.isNull);
  const parts: string[] = [];
  if (c.op === 'in') {
    if (nonNull.length === 1) parts.push(`${col} = ${quote(nonNull[0])}`);
    else if (nonNull.length > 1) parts.push(`${col} IN (${nonNull.map(quote).join(', ')})`);
    if (hasNull) parts.push(`${col} IS NULL`);
    if (parts.length === 0) return '';
    return parts.length > 1 ? `(${parts.join(' OR ')})` : parts[0];
  } else {
    if (nonNull.length === 1) parts.push(`${col} <> ${quote(nonNull[0])}`);
    else if (nonNull.length > 1) parts.push(`${col} NOT IN (${nonNull.map(quote).join(', ')})`);
    if (hasNull) parts.push(`${col} IS NOT NULL`);
    return parts.join(' AND ');
  }
}

/** Human label for a filter chip (e.g. `currency = 'EUR'`, `id IN (1, 2)`). */
export function condLabel(c: FilterCond): string {
  if (c.kind === 'raw') return c.text;
  return condToSql(c) || `${c.column} …`;
}

/** Whether a result cell matches a chip value client-side (the grid narrows
 *  the loaded rows before the rewritten query is re-run). `text` is the
 *  grid's display stringifier (`cellStr`). A boolean value matches by VALUE —
 *  its text is `'true'`, never the `'1'` an older chip carried. */
export function filterValMatches(cell: unknown, val: FilterVal, text: (v: unknown) => string): boolean {
  if (val.isNull) return cell === null || cell === undefined;
  if (cell === null || cell === undefined) return false;
  if (typeof val.bool === 'boolean') return cell === val.bool || (typeof cell !== 'boolean' && text(cell) === val.raw);
  return text(cell) === val.raw;
}

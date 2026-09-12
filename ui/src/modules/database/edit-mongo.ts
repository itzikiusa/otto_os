// MongoDB edit adapter: a single-collection `find` / SELECT is editable by
// `_id`; statements are mongosh calls (`updateOne` / `deleteMany` /
// `insertMany` / `insertOne` / `replaceOne`) the runner's parser understands.
// Typed values travel as Extended-JSON sentinels (`{"$oid": …}`, `{"$date":
// …}`, …) — the same ones the results carry — so a round-trip is lossless.
import { cellStr, isComplex, SET_EMPTY, SET_NULL } from './results-format';
import { parseSimpleSelect } from './edit-sql';
import { flattenPaths, getAtPath } from './expansion-plan';
import type { DiffLine, EditAdapter, RowPatch, TypedValue } from './edit-types';

/** Collection name for an editable Mongo result: a `db.<coll>.find(...)` or a
 * single-collection SELECT (which translates to a find). Null otherwise. */
export function mongoCollectionForEdit(s: string): string | null {
  const t = s.trim();
  // Same multi-statement rejection as parseSimpleSelect — a batch's first
  // `db.<coll>.find` must not make a LATER result set's rows "editable".
  if (/;\s*\S/.test(t.replace(/;\s*$/, ''))) return null;
  const m = t.match(/^db\.([A-Za-z0-9_$.-]+)\.find\s*\(/i);
  if (m) return m[1];
  return parseSimpleSelect(t)?.table ?? null;
}

/** JSON-encode a value typed into a Mongo cell editor: keep numbers/bools when
 * the prior value was one; valid JSON when editing a nested object/array;
 * empty → null; else a quoted string. */
export function mongoLiteral(raw: string, prev: unknown): string {
  if (raw === '' || raw === SET_NULL) return 'null';
  if (raw === SET_EMPTY) return '""';
  if (typeof prev === 'number' && /^-?\d+(\.\d+)?$/.test(raw)) return raw;
  if (typeof prev === 'boolean' && (raw === 'true' || raw === 'false')) return raw;
  if (isComplex(prev)) {
    try {
      JSON.parse(raw);
      return raw;
    } catch {
      /* not valid JSON — fall through to a string */
    }
  }
  return JSON.stringify(raw);
}

/** JSON for a row's `_id` — verbatim, whatever type it is. The wire already
 *  carries a real ObjectId as the `{"$oid": …}` sentinel, so a plain string
 *  `_id` IS a string: coercing a 24-hex one to an ObjectId would make the
 *  generated `updateOne` / `deleteMany` / `replaceOne` miss it (or hit the
 *  ObjectId document of the same hex when both exist). */
export function mongoIdValueFor(idVal: unknown): string {
  return JSON.stringify(idVal);
}
/** `{"_id": …}` filter for a row — the `_id` exactly as the result carries it. */
export function mongoIdFilterFor(idVal: unknown): string {
  return `{"_id": ${mongoIdValueFor(idVal)}}`;
}

/** JSON for a stored Mongo value inside a generated document. Typed values
 *  already arrive as Extended-JSON sentinels (`{"$oid"}`, `{"$date"}`, …), so
 *  every value — `_id` included — round-trips verbatim. */
export function mongoValueLiteral(v: unknown): string {
  if (v === undefined || v === null) return 'null';
  return JSON.stringify(v);
}

/** JSON for a value from the typed editor (Vertical view). The BSON kinds
 *  emit the Extended-JSON sentinel the runner's `decode_ejson` turns back into
 *  the real type; `json` is passed through verbatim (validated by the editor). */
export function mongoTypedLiteral(tv: TypedValue): string {
  switch (tv.kind) {
    case 'null':
      return 'null';
    case 'bool':
      return tv.raw === 'true' ? 'true' : 'false';
    case 'number':
      return String(Number(tv.raw));
    case 'objectId':
      return `{"$oid": ${JSON.stringify(tv.raw)}}`;
    case 'date':
      return `{"$date": ${JSON.stringify(tv.raw)}}`;
    case 'long':
      return `{"$numberLong": ${JSON.stringify(tv.raw)}}`;
    case 'decimal':
      return `{"$numberDecimal": ${JSON.stringify(tv.raw)}}`;
    case 'json':
      return tv.raw;
    default:
      return JSON.stringify(tv.raw);
  }
}

/** Display form of a stored value for a diff line (`∅` when absent). */
function shown(v: unknown): string {
  return v === undefined ? '∅' : v === null ? 'null' : cellStr(v);
}

/** ONE `updateOne` for a row's patch: whole-cell drafts fold into `$set` (typed
 *  from the previous value, as the grid always did), the Vertical editor's
 *  path ops become `$set` (typed literals) / `$unset` / `$rename` on dotted
 *  paths. Empty operators are omitted. Returns null when the patch is empty. */
export function buildUpdateOne(
  coll: string,
  idFilter: string,
  patch: RowPatch,
  columns: { name: string }[],
  row: unknown[],
): { sql: string; diff: Omit<DiffLine, 'row'>[] } | null {
  const sets: string[] = [];
  const diff: Omit<DiffLine, 'row'>[] = [];
  const obj = rowObject(columns, row);
  for (const [ci, value] of [...patch.cells.entries()].sort((a, b) => a[0] - b[0])) {
    const prev = row[ci];
    sets.push(`${JSON.stringify(columns[ci].name)}: ${mongoLiteral(value, prev)}`);
    diff.push({ path: columns[ci].name, op: 'cell', before: shown(prev), after: mongoLiteral(value, prev) });
  }
  for (const [path, tv] of patch.set) {
    const lit = mongoTypedLiteral(tv);
    sets.push(`${JSON.stringify(path)}: ${lit}`);
    diff.push({ path, op: 'set', before: shown(getAtPath(obj, path)), after: lit });
  }
  const unsets = [...patch.unset].map((path) => {
    diff.push({ path, op: 'unset', before: shown(getAtPath(obj, path)), after: '∅' });
    return `${JSON.stringify(path)}: ""`;
  });
  const renames = [...patch.rename].map(([from, to]) => {
    diff.push({ path: from, op: 'rename', before: from, after: to });
    return `${JSON.stringify(from)}: ${JSON.stringify(to)}`;
  });
  const ops: string[] = [];
  if (sets.length) ops.push(`"$set": {${sets.join(', ')}}`);
  if (unsets.length) ops.push(`"$unset": {${unsets.join(', ')}}`);
  if (renames.length) ops.push(`"$rename": {${renames.join(', ')}}`);
  if (ops.length === 0) return null;
  return { sql: `db.${coll}.updateOne(${idFilter}, {${ops.join(', ')}})`, diff };
}

/** The row as a `column → value` object (dotted paths resolve against it). */
function rowObject(columns: { name: string }[], row: unknown[]): Record<string, unknown> {
  const o: Record<string, unknown> = {};
  columns.forEach((c, i) => (o[c.name] = row[i]));
  return o;
}

/** The `_id` of a live row, by column lookup. */
function idOf(ctx: { columns: { name: string }[]; liveRows: unknown[][] }, rowIdx: number): unknown {
  const idIdx = ctx.columns.findIndex((c) => c.name === '_id');
  return ctx.liveRows[rowIdx][idIdx];
}

export const mongoAdapter: EditAdapter = {
  // A single-collection find/SELECT is editable by `_id` — no object_detail
  // lookup (which would error on a SQL-style node path).
  target(statement, columns, ctx) {
    const coll = mongoCollectionForEdit(statement);
    if (!coll) {
      return { target: null, reason: 'Editing needs a single-collection find or SELECT (no aggregate/join).' };
    }
    if (!columns.includes('_id')) {
      return { target: null, reason: 'Include _id in the result to enable editing.' };
    }
    return { target: { db: ctx.activeDb, table: coll, pkCols: ['_id'] }, reason: null };
  },

  /** One `updateOne` per pending row — every parked column / path of the row
   *  in a single `{$set, $unset, $rename}` (see `buildUpdateOne`). */
  buildUpdate(rows, ctx) {
    const stmts: string[] = [];
    const diff: DiffLine[] = [];
    for (const { rowIdx, patch } of rows) {
      const built = buildUpdateOne(
        ctx.target.table,
        mongoIdFilterFor(idOf(ctx, rowIdx)),
        patch,
        ctx.columns,
        ctx.liveRows[rowIdx],
      );
      if (!built) continue;
      stmts.push(built.sql);
      for (const d of built.diff) diff.push({ row: rowIdx, ...d });
    }
    if (stmts.length === 0) return null;
    return { title: 'Review updateOne', sql: stmts.join('\n'), diff };
  },

  buildDelete(idxs, ctx) {
    if (idxs.length === 0) return null;
    const n = idxs.length;
    const noun = `${n} row${n === 1 ? '' : 's'}`;
    const ids = idxs.map((i) => mongoIdValueFor(idOf(ctx, i))).join(', ');
    return {
      title: `Review deleteMany (${noun})`,
      sql: `db.${ctx.target.table}.deleteMany({"_id": {"$in": [${ids}]}})`,
    };
  },

  /** `insertMany` of the rows' fields (all result columns). */
  buildInsert(idxs, ctx) {
    if (idxs.length === 0) return null;
    const docs = idxs.map((i) => {
      const fields = ctx.columns.map(
        (c, ci) => `${JSON.stringify(c.name)}: ${mongoValueLiteral(ctx.liveRows[i][ci])}`,
      );
      return `  { ${fields.join(', ')} }`;
    });
    return `db.${ctx.target.table}.insertMany([\n${docs.join(',\n')}\n])`;
  },

  /** insertOne of the row's fields, omitting `_id` so a fresh one is generated. */
  buildDuplicate(rowIdx, ctx) {
    const obj: Record<string, unknown> = {};
    ctx.columns.forEach((c, i) => {
      if (c.name === '_id') return;
      obj[c.name] = ctx.liveRows[rowIdx][i];
    });
    return {
      title: 'Review insertOne (duplicate row)',
      sql: `db.${ctx.target.table}.insertOne(${JSON.stringify(obj)})`,
    };
  },

  /** insertOne of a user-typed document (the "Insert document…" editor). Values
   *  are emitted verbatim — an ObjectId is typed as `{"$oid": …}`, a string
   *  `_id` stays a string. */
  buildInsertDoc(doc, ctx) {
    const fields = Object.entries(doc).map(([k, v]) => `${JSON.stringify(k)}: ${mongoValueLiteral(v)}`);
    return `db.${ctx.target.table}.insertOne({ ${fields.join(', ')} })`;
  },

  /** replaceOne by `_id`; the filter carries the id, so drop it from the body
   *  (replacing _id is rejected by the server anyway). The diff is path-level:
   *  the leaves of the old row vs the leaves of the new document. */
  buildReplace(rowIdx, doc, ctx) {
    const body = { ...doc };
    delete body._id;
    const before = flattenPaths(rowObject(ctx.columns, ctx.liveRows[rowIdx]));
    const after = flattenPaths(doc);
    const diff: DiffLine[] = [];
    for (const [path, v] of after) {
      if (!before.has(path)) diff.push({ row: rowIdx, path, op: 'set', before: '∅', after: shown(v) });
      else if (JSON.stringify(before.get(path)) !== JSON.stringify(v)) {
        diff.push({ row: rowIdx, path, op: 'set', before: shown(before.get(path)), after: shown(v) });
      }
    }
    for (const [path, v] of before) {
      if (!after.has(path)) diff.push({ row: rowIdx, path, op: 'unset', before: shown(v), after: '∅' });
    }
    return {
      title: 'Review replaceOne',
      sql: `db.${ctx.target.table}.replaceOne(${mongoIdFilterFor(idOf(ctx, rowIdx))}, ${JSON.stringify(body)})`,
      diff,
    };
  },
};

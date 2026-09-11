// MongoDB edit adapter: a single-collection `find` / SELECT is editable by
// `_id`; statements are mongosh calls (`updateOne` / `deleteMany` /
// `insertMany` / `insertOne` / `replaceOne`) the runner's parser understands.
import { isComplex, SET_EMPTY, SET_NULL } from './results-format';
import { parseSimpleSelect } from './edit-sql';
import type { DiffLine, EditAdapter } from './edit-types';

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

/** `{"$oid": "hex"}` (or raw JSON) for a row's `_id` — Mongo delete targeting. */
export function mongoIdValueFor(idVal: unknown): string {
  if (typeof idVal === 'string' && /^[a-f0-9]{24}$/i.test(idVal)) {
    return `{"$oid": ${JSON.stringify(idVal)}}`;
  }
  return JSON.stringify(idVal);
}
/** `{"_id": …}` filter for a row — ObjectId hex → `{"$oid": …}`, else raw. */
export function mongoIdFilterFor(idVal: unknown): string {
  return `{"_id": ${mongoIdValueFor(idVal)}}`;
}

/** JSON for a stored Mongo value inside a generated document. An `_id` that
 *  looks like an ObjectId hex is wrapped as `{"$oid": …}` — the same
 *  convention `mongoIdFilter` uses — so the document round-trips as a real
 *  ObjectId instead of degrading into a plain string. */
export function mongoValueLiteral(name: string, v: unknown): string {
  if (v === undefined || v === null) return 'null';
  if (name === '_id' && typeof v === 'string' && /^[a-f0-9]{24}$/i.test(v)) {
    return `{"$oid": ${JSON.stringify(v)}}`;
  }
  return JSON.stringify(v);
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

  /** One `updateOne` per pending row — every parked column in a single `$set`. */
  buildUpdate(rows, ctx) {
    const stmts: string[] = [];
    const diff: DiffLine[] = [];
    for (const { rowIdx, patch } of rows) {
      const entries = [...patch.cells.entries()].sort((a, b) => a[0] - b[0]);
      if (entries.length === 0) continue;
      const sets = entries
        .map(([ci, value]) =>
          `${JSON.stringify(ctx.columns[ci].name)}: ${mongoLiteral(value, ctx.liveRows[rowIdx][ci])}`)
        .join(', ');
      stmts.push(`db.${ctx.target.table}.updateOne(${mongoIdFilterFor(idOf(ctx, rowIdx))}, {"$set": {${sets}}})`);
      for (const [ci, value] of entries) {
        const prev = ctx.liveRows[rowIdx][ci];
        diff.push({
          row: rowIdx,
          path: ctx.columns[ci].name,
          op: 'cell',
          before: prev === null || prev === undefined ? 'null' : JSON.stringify(prev),
          after: mongoLiteral(value, prev),
        });
      }
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
        (c, ci) => `${JSON.stringify(c.name)}: ${mongoValueLiteral(c.name, ctx.liveRows[i][ci])}`,
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

  /** replaceOne by `_id`; the filter carries the id, so drop it from the body
   *  (replacing _id is rejected by the server anyway). The path-level diff is
   *  computed by the Vertical-editing work (flattenPaths old vs new). */
  buildReplace(rowIdx, doc, ctx) {
    const body = { ...doc };
    delete body._id;
    return {
      title: 'Review replaceOne',
      sql: `db.${ctx.target.table}.replaceOne(${mongoIdFilterFor(idOf(ctx, rowIdx))}, ${JSON.stringify(body)})`,
      diff: [],
    };
  },
};

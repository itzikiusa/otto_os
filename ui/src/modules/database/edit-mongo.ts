// MongoDB edit adapter: a single-collection `find` / SELECT is editable by
// `_id`; statements are mongosh calls (`updateOne` / `deleteMany` /
// `insertMany` / `insertOne` / `replaceOne`) the runner's parser understands.
// Typed values travel as Extended-JSON sentinels (`{"$oid": …}`, `{"$date":
// …}`, …) — the same ones the results carry — so a round-trip is lossless.
import { cellStr, isComplex, SET_EMPTY, SET_NULL } from './results-format';
import { parseSimpleSelect } from './edit-sql';
import { getAtPath } from './expansion-plan';
import type { DiffLine, EditAdapter, RowPatch, TypedValue } from './edit-types';
import { plural } from '../../lib/plural';

/** Read one balanced method call without confusing commas/parens in literals
 * for method boundaries. Unknown JavaScript syntax is deliberately read-only. */
function mongoCall(source: string): { args: string[]; rest: string } | null {
  if (!source.startsWith('(')) return null;
  const stack = ['('];
  const args: string[] = [];
  let start = 1;
  for (let i = 1; i < source.length; i++) {
    const c = source[i];
    if (c === '"' || c === "'") {
      const quote = c;
      for (i++; i < source.length; i++) {
        if (source[i] === '\\') i++;
        else if (source[i] === quote) break;
      }
      if (i >= source.length) return null;
    } else if (c === '/') {
      // Filters may contain regular expressions. Comments are not part of the
      // small proven grammar; the query still runs, but cannot enable editing.
      if (source[i + 1] === '/' || source[i + 1] === '*') return null;
      let inClass = false;
      for (i++; i < source.length; i++) {
        if (source[i] === '\\') i++;
        else if (source[i] === '[') inClass = true;
        else if (source[i] === ']') inClass = false;
        else if (source[i] === '/' && !inClass) break;
      }
      if (i >= source.length) return null;
    } else if (c === '`' || c === ';') return null;
    else if ('([{'.includes(c)) stack.push(c);
    else if (')]}'.includes(c)) {
      if (stack.pop() !== ({ ')': '(', ']': '[', '}': '{' } as Record<string, string>)[c]) return null;
      if (!stack.length) {
        const last = source.slice(start, i).trim();
        if (last || args.length) args.push(last);
        return { args, rest: source.slice(i + 1).trim() };
      }
    } else if (c === ',' && stack.length === 1) {
      args.push(source.slice(start, i).trim());
      start = i + 1;
    }
  }
  return null;
}

/** Only direct inclusion/exclusion projections preserve field provenance.
 * Computed expressions can replace _id with another document's key. */
function directMongoProjection(source: string): boolean {
  const key = `(?:[A-Za-z_][\\w-]*|"[^".$\\\\]*"|'[^'.$\\\\]*')`;
  const field = `${key}\\s*:\\s*(?:0|1|true|false)`;
  return new RegExp(`^\\{\\s*(?:${field}(?:\\s*,\\s*${field})*\\s*,?)?\\s*\\}$`).test(source);
}

/** Prove a complete single-collection find and direct projection before any
 * mutation builder trusts the result's _id. A prefix alone is insufficient:
 * the backend accepts subsequent operations and computed find projections. */
export function mongoCollectionForEdit(s: string): string | null {
  const t = s.trim().replace(/;\s*$/, '');
  const m = t.match(/^db\.([A-Za-z0-9_$-]+)\.find\s*(\()/);
  if (!m) return parseSimpleSelect(t)?.table ?? null;
  const first = mongoCall(t.slice(m[0].length - 1));
  if (!first || first.args.length > 2 || (first.args[1] && !directMongoProjection(first.args[1]))) return null;
  let rest = first.rest;
  while (rest) {
    const method = rest.match(/^\.(sort|limit|projection)\s*(\()/);
    if (!method) return null;
    const call = mongoCall(rest.slice(method[0].length - 1));
    if (!call || call.args.length !== 1) return null;
    if (method[1] === 'projection' && !directMongoProjection(call.args[0])) return null;
    if (method[1] === 'limit' && !/^-?\d+$/.test(call.args[0])) return null;
    rest = call.rest;
  }
  return m[1];
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
      return { target: null, reason: 'Editing needs a single-collection find or SELECT with direct fields (no computed projection, aggregate or join).' };
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
    const noun = `${plural(n, 'row')}`;
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

  /** Patch the displayed fields by _id. A find/SELECT can project only part
   * of a document, so replacing the row would delete undisplayed fields. Direct
   * top-level projections are required above: nested projections could hide
   * siblings inside an object that this editor replaces as one field. */
  buildReplace(rowIdx, doc, ctx) {
    const original = rowObject(ctx.columns, ctx.liveRows[rowIdx]);
    const sets: Record<string, unknown> = Object.create(null);
    const unsets: Record<string, string> = Object.create(null);
    const diff: DiffLine[] = [];
    for (const [key, value] of Object.entries(doc)) {
      if (key === '_id') continue;
      if (JSON.stringify(original[key]) !== JSON.stringify(value)) {
        // Mongo update paths cannot name literal dotted/dollar field names.
        if (key.includes('.') || key.startsWith('$')) return null;
        sets[key] = value;
        diff.push({ row: rowIdx, path: key, op: 'set', before: shown(original[key]), after: shown(value) });
      }
    }
    for (const [key, value] of Object.entries(original)) {
      if (key === '_id' || Object.hasOwn(doc, key)) continue;
      if (key.includes('.') || key.startsWith('$')) return null;
      unsets[key] = '';
      diff.push({ row: rowIdx, path: key, op: 'unset', before: shown(value), after: '∅' });
    }
    const update: Record<string, unknown> = {};
    if (Object.keys(sets).length) update.$set = sets;
    if (Object.keys(unsets).length) update.$unset = unsets;
    if (!Object.keys(update).length) return null;
    return {
      title: 'Review updateOne',
      sql: `db.${ctx.target.table}.updateOne(${mongoIdFilterFor(idOf(ctx, rowIdx))}, ${JSON.stringify(update)})`,
      diff,
    };
  },
};

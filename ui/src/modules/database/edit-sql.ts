// SQL-engine edit adapter (MySQL / Postgres / ClickHouse): statement parsing
// plus the UPDATE / DELETE / INSERT builders the edit flow used to inline. The
// literal/identifier helpers are exported on their own because ResultsGrid's
// FK navigation and the "WHERE pk IN (…)" copy build predicates with them too.
import type { DbEngine } from '../../lib/api/types';
import { escapeSqlString } from './sql-util';
import { cellStr, compactJson, isComplex, SET_EMPTY, SET_NULL } from './results-format';
import type { DiffLine, EditAdapter, EditCtx, TypedValue } from './edit-types';

/** Quote a SQL identifier for the active engine — double-quotes for Postgres
 *  (backticks are invalid there), backticks for MySQL/ClickHouse. */
export function qid(engine: DbEngine | null, name: string): string {
  return engine === 'postgres'
    ? '"' + name.replace(/"/g, '""') + '"'
    : '`' + name.replace(/`/g, '``') + '`';
}

// MySQL (default modes) and ClickHouse treat `\` as an escape character inside
// a string literal — a value containing one must double it or the emitted SQL
// corrupts (a trailing `\` even swallows the closing quote). Postgres standard
// strings don't, so only the quote is doubled there.
function backslashEscapes(engine: DbEngine | null): boolean {
  return engine === 'mysql' || engine === 'clickhouse';
}

/** SQL-quote a scalar value typed into the cell editor: numbers bare (when
 * the previous value was numeric), empty → NULL, else 'escaped'. */
export function sqlLiteral(engine: DbEngine | null, raw: string, asNumber: boolean): string {
  if (raw === '' || raw === SET_NULL) return 'NULL';
  if (raw === SET_EMPTY) return "''";
  if (asNumber && /^-?\d+(\.\d+)?$/.test(raw)) return raw;
  return `'${escapeSqlString(raw, backslashEscapes(engine))}'`;
}
/** SQL-quote an existing typed value (for WHERE / INSERT values). */
export function valueLiteral(engine: DbEngine | null, v: unknown): string {
  if (v === null || v === undefined) return 'NULL';
  if (typeof v === 'number' || typeof v === 'bigint') return String(v);
  if (typeof v === 'boolean') return engine === 'postgres' ? (v ? 'TRUE' : 'FALSE') : v ? '1' : '0';
  if (isComplex(v)) return `'${escapeSqlString(compactJson(v), backslashEscapes(engine))}'`;
  return `'${escapeSqlString(String(v), backslashEscapes(engine))}'`;
}
/** The cell DRAFT a typed value (Vertical editor) parks for a SQL column — the
 *  same raw text the grid's inline input would have produced, so it flows
 *  through `sqlLiteral` unchanged: NULL / '' as the sentinels, booleans as the
 *  engine's literal form (`valueLiteral`'s), everything else as typed. */
export function typedCellDraft(engine: DbEngine | null, tv: TypedValue): string {
  switch (tv.kind) {
    case 'null':
      return SET_NULL;
    case 'bool':
      return engine === 'postgres' ? (tv.raw === 'true' ? 'TRUE' : 'FALSE') : tv.raw === 'true' ? '1' : '0';
    case 'number':
      return String(Number(tv.raw));
    default:
      return tv.raw === '' ? SET_EMPTY : tv.raw;
  }
}
/** Qualified `db.table` (db optional), quoted for the active engine. */
export function tableRef(ctx: EditCtx): string {
  const t = ctx.qid(ctx.target.table);
  return ctx.target.db ? `${ctx.qid(ctx.target.db)}.${t}` : t;
}
/** `\`pk1\` = v1 AND \`pk2\` = v2` targeting one row by its primary key. */
export function whereByPk(ctx: EditCtx, rowIdx: number): string {
  return ctx.target.pkCols
    .map((pk) => {
      const ci = ctx.columns.findIndex((c) => c.name === pk);
      return `${ctx.qid(pk)} = ${valueLiteral(ctx.engine, ctx.liveRows[rowIdx][ci])}`;
    })
    .join(' AND ');
}

/** Parse a simple SELECT … FROM <table>. Returns {db, table} or null. */
export function parseSimpleSelect(sql: string): { db: string | null; table: string } | null {
  const s = sql.trim().replace(/;\s*$/, '');
  // A multi-statement batch can't be attributed to ONE table: the FROM matched
  // below would be statement 1's even while result set 2+ is shown, so edits
  // would target the wrong table. Mirrors the store's splitStatement rejection.
  if (/;\s*\S/.test(s)) return null;
  if (!/^select\b/i.test(s)) return null;
  // Reject anything that makes a row non-1:1 with a base-table row.
  if (/\bjoin\b|\bgroup\s+by\b|\bunion\b|\bdistinct\b|\bhaving\b/i.test(s)) return null;
  // Reject aggregates in the projection (between SELECT and FROM).
  const proj = s.slice(0, s.search(/\bfrom\b/i));
  if (/\b(count|sum|avg|min|max|group_concat|array_agg)\s*\(/i.test(proj)) return null;
  // Capture the first table after FROM: optional `db`.`table` with backticks.
  const m = s.match(
    /\bfrom\s+`?([\w$]+)`?(?:\s*\.\s*`?([\w$]+)`?)?/i,
  );
  if (!m) return null;
  if (m[2]) return { db: m[1], table: m[2] };
  return { db: null, table: m[1] };
}

/** Display form of a stored value for a diff line. */
function shown(v: unknown): string {
  return v === null || v === undefined ? 'NULL' : cellStr(v);
}
/** Display form of a parked cell draft (the Set NULL / Set empty sentinels). */
function draftShown(raw: string): string {
  return raw === '' || raw === SET_NULL ? 'NULL' : raw === SET_EMPTY ? "''" : raw;
}

export const sqlAdapter: EditAdapter = {
  target(statement) {
    const parsed = parseSimpleSelect(statement);
    if (!parsed) {
      return {
        target: null,
        reason: 'Editing needs a single-table SELECT (no JOIN, GROUP BY, DISTINCT, UNION or aggregates).',
      };
    }
    // `db` stays null when the SQL omits it — EditFlow defaults it from the
    // schema root; `pkCols` is filled by the async object_detail lookup.
    return { target: { db: parsed.db, table: parsed.table, pkCols: [] }, reason: null };
  },

  /** ONE statement per pending row — every parked column in a single SET
   * (ClickHouse uses `ALTER TABLE … UPDATE`, a mutation). Multiple rows become
   * a multi-statement batch (each driver splits and runs them in order). */
  buildUpdate(rows, ctx) {
    const stmts: string[] = [];
    const diff: DiffLine[] = [];
    for (const { rowIdx, patch } of rows) {
      const entries = [...patch.cells.entries()].sort((a, b) => a[0] - b[0]);
      if (entries.length === 0) continue;
      const sets = entries
        .map(([ci, value]) =>
          `${ctx.qid(ctx.columns[ci].name)} = ${sqlLiteral(ctx.engine, value, typeof ctx.liveRows[rowIdx][ci] === 'number')}`)
        .join(', ');
      const where = whereByPk(ctx, rowIdx);
      stmts.push(
        ctx.engine === 'clickhouse'
          ? `ALTER TABLE ${tableRef(ctx)} UPDATE ${sets} WHERE ${where};`
          : `UPDATE ${tableRef(ctx)} SET ${sets} WHERE ${where};`,
      );
      for (const [ci, value] of entries) {
        diff.push({
          row: rowIdx,
          path: ctx.columns[ci].name,
          op: 'cell',
          before: shown(ctx.liveRows[rowIdx][ci]),
          after: draftShown(value),
        });
      }
    }
    if (stmts.length === 0) return null;
    return {
      title: ctx.engine === 'clickhouse' ? 'Review ALTER … UPDATE (mutation)' : 'Review UPDATE',
      sql: stmts.join('\n'),
      diff,
    };
  },

  /** Build a DELETE targeting the given rows (by liveRows index). */
  buildDelete(idxs, ctx) {
    if (idxs.length === 0) return null;
    const n = idxs.length;
    const noun = `${n} row${n === 1 ? '' : 's'}`;
    let where: string;
    if (ctx.target.pkCols.length === 1) {
      const pk = ctx.target.pkCols[0];
      const ci = ctx.columns.findIndex((c) => c.name === pk);
      const list = idxs.map((i) => valueLiteral(ctx.engine, ctx.liveRows[i][ci])).join(', ');
      where = `${ctx.qid(pk)} IN (${list})`;
    } else {
      // Composite key: OR a per-row AND of every key column.
      where = idxs.map((i) => `(${whereByPk(ctx, i)})`).join(' OR ');
    }
    const sql =
      ctx.engine === 'clickhouse'
        ? `ALTER TABLE ${tableRef(ctx)} DELETE WHERE ${where};`
        : `DELETE FROM ${tableRef(ctx)} WHERE ${where};`;
    return {
      title:
        ctx.engine === 'clickhouse' ? `Review ALTER … DELETE (${noun})` : `Review DELETE (${noun})`,
      sql,
    };
  },

  /** `INSERT INTO … VALUES` per row, all result columns (ClickHouse included —
   *  it accepts the same backtick quoting). */
  buildInsert(idxs, ctx) {
    if (idxs.length === 0) return null;
    const ref = tableRef(ctx);
    const cols = ctx.columns.map((c) => ctx.qid(c.name)).join(', ');
    return idxs
      .map((i) => {
        const vals = ctx.columns.map((_, ci) => valueLiteral(ctx.engine, ctx.liveRows[i][ci])).join(', ');
        return `INSERT INTO ${ref} (${cols}) VALUES (${vals});`;
      })
      .join('\n');
  },

  /** Build an INSERT cloning a row. With a single (likely auto-increment) PK we
   * omit it so identity regenerates; with a composite key we include every
   * column so the user can adjust the key in the review SQL. */
  buildDuplicate(rowIdx, ctx) {
    const omitPk = ctx.target.pkCols.length === 1;
    const cols: string[] = [];
    const vals: string[] = [];
    ctx.columns.forEach((c, i) => {
      if (omitPk && ctx.target.pkCols.includes(c.name)) return; // single PK → regenerate
      cols.push(ctx.qid(c.name));
      vals.push(valueLiteral(ctx.engine, ctx.liveRows[rowIdx][i]));
    });
    const sql = `INSERT INTO ${tableRef(ctx)} (${cols.join(', ')}) VALUES (${vals.join(', ')});`;
    return { title: 'Review INSERT (duplicate row)', sql };
  },

  /** `INSERT INTO … VALUES` from a user-typed JSON object (the "Insert
   *  document…" editor): only keys that name a result column are used, in
   *  column order; unknown keys are ignored. Null when nothing matches. */
  buildInsertDoc(doc, ctx) {
    const cols: string[] = [];
    const vals: string[] = [];
    for (const c of ctx.columns) {
      if (!(c.name in doc)) continue;
      cols.push(ctx.qid(c.name));
      vals.push(valueLiteral(ctx.engine, doc[c.name]));
    }
    if (cols.length === 0) return null;
    return `INSERT INTO ${tableRef(ctx)} (${cols.join(', ')}) VALUES (${vals.join(', ')});`;
  },

  /** Whole-document save: SET only the columns whose value actually changed.
   *  Null when nothing changed (the caller just closes the editor). */
  buildReplace(rowIdx, doc, ctx) {
    const sets: string[] = [];
    const diff: DiffLine[] = [];
    ctx.columns.forEach((c, i) => {
      if (ctx.target.pkCols.includes(c.name)) return; // key is the row's identity
      if (!(c.name in doc)) return;
      const prev = ctx.liveRows[rowIdx]?.[i];
      const next = doc[c.name];
      if (compactJson(prev ?? null) === compactJson(next ?? null)) return;
      sets.push(`${ctx.qid(c.name)} = ${valueLiteral(ctx.engine, next)}`);
      diff.push({ row: rowIdx, path: c.name, op: 'cell', before: shown(prev), after: shown(next) });
    });
    if (sets.length === 0) return null;
    const where = whereByPk(ctx, rowIdx);
    const sql =
      ctx.engine === 'clickhouse'
        ? `ALTER TABLE ${tableRef(ctx)} UPDATE ${sets.join(', ')} WHERE ${where};`
        : `UPDATE ${tableRef(ctx)} SET ${sets.join(', ')} WHERE ${where};`;
    return {
      title: ctx.engine === 'clickhouse' ? 'Review ALTER … UPDATE (mutation)' : 'Review UPDATE',
      sql,
      diff,
    };
  },
};

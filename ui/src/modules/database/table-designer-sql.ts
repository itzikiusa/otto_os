// The Table Designer's ALTER generators (`TableDesigner.svelte` is the UI):
// diff the edited column rows against the table's original columns and emit
// engine-correct DDL for the user to review in a query tab. MySQL uses
// CHANGE COLUMN clauses in one ALTER, Postgres a sequence of standard ALTER
// statements (RENAME / ALTER COLUMN TYPE / SET-DROP NOT NULL / SET DEFAULT),
// ClickHouse RENAME/MODIFY COLUMN (no FKs; nullability lives in the type).
//
// Pure (quoting comes from the ONE dialect module) so node:test runs it
// directly — unit/dbTableDesigner.test.ts.
import type { DbColumnDef } from '../../lib/api/types';
import { quoteIdent, stringLiteral, type EngineLike } from './sql-dialect.ts';

export interface DesignerRow {
  orig: string | null; // existing column name, or null for a new column
  name: string;
  type: string;
  notNull: boolean;
  /** The DEFAULT as SQL text (a literal or an expression), '' = no default. */
  def: string;
  /** `def` as seeded from the original column — what "unchanged" compares to. */
  origDef: string;
  drop: boolean;
}

// New indexes / foreign keys to ADD (the designer adds — it doesn't edit
// existing index/FK metadata, which isn't passed in).
export interface DesignerIndex {
  name: string;
  cols: string; // comma-separated column names
  unique: boolean;
}
export interface DesignerFk {
  name: string;
  cols: string; // comma-separated local columns
  refTable: string;
  refCols: string; // comma-separated referenced columns
}

export interface DesignerInput {
  engine: EngineLike;
  /** The quoted, schema-qualified table reference. */
  tableRef: string;
  /** The table's original columns (as `object_detail` returned them). */
  columns: DbColumnDef[];
  rows: DesignerRow[];
  indexes: DesignerIndex[];
  fks: DesignerFk[];
}

const NUMERIC_TYPE = /^(?:tiny|small|medium|big)?int|^(?:integer|decimal|numeric|float|double|real|bit|bool|boolean|year)\b/i;
const NUMBER = /^-?(?:\d+(?:\.\d*)?|\.\d+)(?:e[-+]?\d+)?$/i;
const NOW_FN = /^(?:current_timestamp|now|localtime|localtimestamp)(?:\(\d*\))?$/i;
const STRING_TYPE = /char|text|enum|set\(/i;

/**
 * The editable DEFAULT text for an existing column, as SQL. Postgres and
 * ClickHouse report the default as an expression already (`'x'::text`,
 * `now()`), so it is used verbatim. MySQL's `information_schema.column_default`
 * is the UNQUOTED value (`pending` for `DEFAULT 'pending'`, an empty string for
 * `DEFAULT ''`, `uuid()` for an expression default) — re-emitting it verbatim
 * is a syntax error (or, for `''`, silently drops the default), so it becomes a
 * literal here: numbers / bit values stay bare, CURRENT_TIMESTAMP stays bare,
 * other DEFAULT_GENERATED expressions are parenthesised, everything else is a
 * string literal (which keeps `''` as a real, non-empty default text).
 */
export function seedDefault(engine: EngineLike, c: DbColumnDef): string {
  const d = c.default;
  if (d == null) return '';
  if (engine !== 'mysql') return d;
  if (NOW_FN.test(d.trim())) return d.trim();
  if (/DEFAULT_GENERATED/i.test(c.extra ?? '')) return `(${d})`;
  if (NUMERIC_TYPE.test(c.data_type) && (NUMBER.test(d) || /^b'[01]*'$/i.test(d))) return d;
  return stringLiteral('mysql', d);
}

/** Editable rows seeded from the table's columns. */
export function rowsFromColumns(engine: EngineLike, cols: DbColumnDef[]): DesignerRow[] {
  return cols.map((c) => {
    const def = seedDefault(engine, c);
    return {
      orig: c.name,
      name: c.name,
      type: c.data_type,
      notNull: !c.nullable,
      def,
      origDef: def,
      drop: false,
    };
  });
}

/** A blank new-column row. */
export function newRow(): DesignerRow {
  return { orig: null, name: '', type: 'VARCHAR(255)', notNull: false, def: '', origDef: '', drop: false };
}

interface RowDiff {
  r: DesignerRow;
  orig: DbColumnDef;
  renamed: boolean;
  typeChanged: boolean;
  nullChanged: boolean;
  defChanged: boolean;
}

/** Generate the engine-correct ALTER SQL ('' when nothing changed). */
export function designerSql(input: DesignerInput): string {
  const { engine, tableRef, columns, rows, indexes, fks } = input;
  const isClickhouse = engine === 'clickhouse';
  const qi = (s: string): string => quoteIdent(engine, s);
  /** Quote a comma-separated identifier list: `a, b ` → `` `a`, `b` ``. */
  const quoteCols = (csv: string): string =>
    csv
      .split(',')
      .map((c) => c.trim())
      .filter(Boolean)
      .map(qi)
      .join(', ');

  function colDef(r: DesignerRow, orig: DbColumnDef | null): string {
    let s = `${qi(r.name)} ${r.type}`;
    if (engine === 'mysql' && orig?.collation && /^\w+$/.test(orig.collation) && STRING_TYPE.test(r.type) && !/\bcollate\b/i.test(r.type)) {
      // CHANGE COLUMN restates the whole column: without its collation a
      // rename silently re-collates it to the table default (which changes
      // uniqueness / comparisons and rewrites the table). The collation names
      // its character set, so COLLATE alone carries both.
      s += ` COLLATE ${orig.collation}`;
    }
    if (!isClickhouse) s += r.notNull ? ' NOT NULL' : ' NULL';
    if (r.def.trim() !== '') s += ` DEFAULT ${r.def.trim()}`;
    if (engine === 'mysql' && orig) {
      // CHANGE COLUMN replaces the WHOLE definition — without re-stating the
      // original attributes a rename silently drops AUTO_INCREMENT /
      // ON UPDATE CURRENT_TIMESTAMP and the comment. (DEFAULT_GENERATED is
      // information_schema bookkeeping, not valid DDL.)
      const extra = (orig.extra ?? '').replace(/DEFAULT_GENERATED/gi, '').trim();
      if (extra) s += ` ${extra}`;
      if (orig.comment) s += ` COMMENT ${stringLiteral('mysql', orig.comment)}`;
    }
    return s;
  }

  /** Diff one edited row against its original column (null = unchanged). */
  function diffRow(r: DesignerRow): RowDiff | null {
    const orig = columns.find((c) => c.name === r.orig);
    if (!orig || !r.name.trim() || !r.type.trim()) return null;
    const d: RowDiff = {
      r,
      orig,
      renamed: r.name !== r.orig,
      typeChanged: r.type !== orig.data_type,
      nullChanged: r.notNull === orig.nullable,
      defChanged: r.def !== r.origDef,
    };
    return d.renamed || d.typeChanged || d.nullChanged || d.defChanged ? d : null;
  }

  /** MySQL: one ALTER with comma-separated clauses (CHANGE COLUMN carries all). */
  function mysqlSql(): string {
    const parts: string[] = [];
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) parts.push(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        parts.push(`DROP COLUMN ${qi(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (d) parts.push(`CHANGE COLUMN ${qi(r.orig)} ${colDef(r, d.orig)}`);
    }
    // New indexes: ADD [UNIQUE] INDEX [name] (cols).
    for (const ix of indexes) {
      const cols = quoteCols(ix.cols);
      if (!cols) continue;
      const kw = ix.unique ? 'UNIQUE INDEX' : 'INDEX';
      const named = ix.name.trim() ? `${qi(ix.name.trim())} ` : '';
      parts.push(`ADD ${kw} ${named}(${cols})`);
    }
    // New foreign keys: ADD [CONSTRAINT name] FOREIGN KEY (cols) REFERENCES t (refcols).
    for (const fk of fks) {
      const cols = quoteCols(fk.cols);
      const refCols = quoteCols(fk.refCols);
      if (!cols || !fk.refTable.trim() || !refCols) continue;
      const named = fk.name.trim() ? `CONSTRAINT ${qi(fk.name.trim())} ` : '';
      parts.push(`ADD ${named}FOREIGN KEY (${cols}) REFERENCES ${qi(fk.refTable.trim())} (${refCols})`);
    }
    return parts.length ? `ALTER TABLE ${tableRef}\n  ${parts.join(',\n  ')};` : '';
  }

  /** Postgres: standard SQL has no CHANGE COLUMN — each change is its own
   *  ALTER statement (rename first, then the rest address the new name). */
  function postgresSql(): string {
    const stmts: string[] = [];
    const alter = (clause: string): void => {
      stmts.push(`ALTER TABLE ${tableRef} ${clause};`);
    };
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) alter(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        alter(`DROP COLUMN ${qi(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (!d) continue;
      if (d.renamed) alter(`RENAME COLUMN ${qi(r.orig)} TO ${qi(r.name)}`);
      const col = qi(r.name);
      if (d.typeChanged) alter(`ALTER COLUMN ${col} TYPE ${r.type}`);
      if (d.nullChanged) alter(`ALTER COLUMN ${col} ${r.notNull ? 'SET' : 'DROP'} NOT NULL`);
      if (d.defChanged) {
        alter(
          r.def.trim() !== ''
            ? `ALTER COLUMN ${col} SET DEFAULT ${r.def.trim()}`
            : `ALTER COLUMN ${col} DROP DEFAULT`,
        );
      }
    }
    for (const ix of indexes) {
      const cols = quoteCols(ix.cols);
      if (!cols) continue;
      const named = ix.name.trim() ? `${qi(ix.name.trim())} ` : '';
      stmts.push(`CREATE ${ix.unique ? 'UNIQUE ' : ''}INDEX ${named}ON ${tableRef} (${cols});`);
    }
    for (const fk of fks) {
      const cols = quoteCols(fk.cols);
      const refCols = quoteCols(fk.refCols);
      if (!cols || !fk.refTable.trim() || !refCols) continue;
      const named = fk.name.trim() ? `CONSTRAINT ${qi(fk.name.trim())} ` : '';
      alter(`ADD ${named}FOREIGN KEY (${cols}) REFERENCES ${qi(fk.refTable.trim())} (${refCols})`);
    }
    return stmts.join('\n');
  }

  /** ClickHouse: RENAME COLUMN + MODIFY COLUMN clauses; nullability is part of
   *  the type (`Nullable(T)`), FKs don't exist, indexes need a skipping TYPE. */
  function clickhouseSql(): string {
    const parts: string[] = [];
    for (const r of rows) {
      if (r.orig === null) {
        if (!r.drop && r.name.trim() && r.type.trim()) parts.push(`ADD COLUMN ${colDef(r, null)}`);
        continue;
      }
      if (r.drop) {
        parts.push(`DROP COLUMN ${qi(r.orig)}`);
        continue;
      }
      const d = diffRow(r);
      if (!d) continue;
      if (d.renamed) parts.push(`RENAME COLUMN ${qi(r.orig)} TO ${qi(r.name)}`);
      if (d.typeChanged || d.defChanged) parts.push(`MODIFY COLUMN ${colDef(r, d.orig)}`);
    }
    return parts.length ? `ALTER TABLE ${tableRef}\n  ${parts.join(',\n  ')};` : '';
  }

  if (engine === 'postgres') return postgresSql();
  if (engine === 'clickhouse') return clickhouseSql();
  return mysqlSql();
}

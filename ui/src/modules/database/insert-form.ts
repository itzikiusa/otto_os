// The "Add row" form behind the results grid: one field per column of the
// target table, derived from its SCHEMA (the object detail's columns — name,
// type, nullability, default, auto-increment / generated, key) so it works on
// an EMPTY table, where the result set has no rows to clone and the old
// "Insert row from JSON…" path had nothing to template from. The form only
// produces text: `buildFormInsert` emits ONE `INSERT INTO … VALUES …` that the
// edit flow hands to the normal review modal (nothing runs until confirmed).
//
// Each field is in one of three modes:
//   value   — the typed text, as a literal for the column's type
//   null    — an explicit NULL
//   default — the column is left OUT of the column list, so the engine applies
//             its default / auto-increment / generated value
// Generated columns (MySQL VIRTUAL/STORED GENERATED, ClickHouse MATERIALIZED /
// ALIAS) cannot be written at all and stay locked on `default`.
//
// Pure (quoting comes from the ONE dialect module, value-free imports only) so
// node:test runs it directly — unit/dbInsertForm.test.ts.
import type { DbColumn, DbColumnDef } from '../../lib/api/types';
import { backslashEscapes, boolLiteral, escapeSqlText, quoteIdent, type EngineLike } from '../../lib/database/sql-dialect.ts';

export type InsertMode = 'value' | 'null' | 'default';

export interface InsertField {
  name: string;
  /** Declared type ('' when only the result's column name is known). */
  dataType: string;
  nullable: boolean;
  /** The column's default expression as the catalog reports it. */
  defaultExpr: string | null;
  /** Auto-increment / serial: the engine assigns it. */
  auto: boolean;
  /** Computed by the engine — never insertable (mode locked to `default`). */
  generated: boolean;
  pk: boolean;
  mode: InsertMode;
  raw: string;
}

export type FieldKind = 'number' | 'bool' | 'text';

/** How a column's typed text becomes a literal. Integer / decimal / float
 *  types (MySQL, Postgres, ClickHouse `Nullable(Int32)` / `Decimal(10,2)`)
 *  are numbers; boolean types (and MySQL's `tinyint(1)`) are booleans. */
export function fieldKind(dataType: string): FieldKind {
  const t = dataType.trim().toLowerCase().replace(/^(nullable|lowcardinality)\((.*)\)$/, '$2');
  if (/^(bool|boolean)$/.test(t) || /^tinyint\(1\)(\s+unsigned)?$/.test(t)) return 'bool';
  if (/(^|[^a-z])u?(tiny|small|medium|big)?int(eger|[0-9]*)?([^a-z]|$)/.test(t)) return 'number';
  if (/^(serial|bigserial|smallserial)$/.test(t)) return 'number';
  if (/^(decimal|numeric|float|double|real|money|dec)\b|^float\d+|^decimal\d+|^double precision/.test(t)) return 'number';
  return 'text';
}

function isGenerated(c: DbColumnDef): boolean {
  const extra = (c.extra ?? '').toUpperCase();
  if (/\b(VIRTUAL|STORED) GENERATED\b/.test(extra)) return true;
  return /^(MATERIALIZED|ALIAS)\b/i.test((c.default ?? '').trim());
}

function isAuto(c: DbColumnDef): boolean {
  if (/auto_increment/i.test(c.extra ?? '')) return true;
  const d = (c.default ?? '').trim();
  return /^nextval\(/i.test(d) || /^(bigserial|serial|smallserial)$/i.test(c.data_type.trim());
}

/** The mode a field opens in. A required column (NOT NULL, no default, not
 *  assigned by the engine) asks for a value; a nullable one without a default
 *  is an explicit NULL; anything with a default leaves it to the engine. */
function initialMode(f: Omit<InsertField, 'mode' | 'raw'>): InsertMode {
  if (f.generated || f.auto || f.defaultExpr !== null) return 'default';
  return f.nullable ? 'null' : 'value';
}

/** Form fields from the table's schema columns (`ObjectDetail.columns`). */
export function fieldsFromSchema(columns: DbColumnDef[], primaryKey: string[] = []): InsertField[] {
  return columns.map((c) => {
    const base = {
      name: c.name,
      dataType: c.data_type,
      nullable: c.nullable,
      defaultExpr: c.default ?? null,
      auto: isAuto(c),
      generated: isGenerated(c),
      pk: primaryKey.includes(c.name) || c.key === 'PRI',
    };
    return { ...base, mode: initialMode(base), raw: '' };
  });
}

/** Fallback fields from the result's columns alone (schema unavailable): only
 *  the name and a type hint are known, so every field starts as `default`
 *  (left out) and the user opts columns in by typing. */
export function fieldsFromResult(columns: DbColumn[]): InsertField[] {
  return columns.map((c) => ({
    name: c.name,
    dataType: c.type_hint ?? '',
    nullable: true,
    defaultExpr: null,
    auto: false,
    generated: false,
    pk: false,
    mode: 'default' as InsertMode,
    raw: '',
  }));
}

/** Short badge text for a field's constraints (`PK`, `auto`, `default …`). */
export function fieldBadges(f: InsertField): string[] {
  const out: string[] = [];
  if (f.pk) out.push('PK');
  if (f.generated) out.push('generated');
  else if (f.auto) out.push('auto');
  else if (f.defaultExpr !== null) out.push(`default ${f.defaultExpr === '' ? "''" : f.defaultExpr}`);
  if (!f.nullable && !f.generated && !f.auto && f.defaultExpr === null) out.push('required');
  return out;
}

const NUMBER_RE = /^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/;
const BOOL_RE = /^(true|false|1|0)$/i;

/** Why a field can't be inserted as typed (null = fine). */
export function fieldError(f: InsertField): string | null {
  if (f.generated) return null;
  if (f.mode === 'null' && !f.nullable) return 'NOT NULL column — enter a value or use the default';
  if (f.mode !== 'value') return null;
  const kind = fieldKind(f.dataType);
  const v = f.raw.trim();
  if (kind === 'number' && !NUMBER_RE.test(v)) return v ? 'Not a number' : 'Enter a number';
  if (kind === 'bool' && !BOOL_RE.test(v)) return 'Use true or false';
  return null;
}

/** The SQL literal a `value`-mode field emits. */
export function fieldLiteral(engine: EngineLike, f: InsertField): string {
  if (f.mode === 'null') return 'NULL';
  const kind = fieldKind(f.dataType);
  const v = f.raw.trim();
  if (kind === 'number' && NUMBER_RE.test(v)) return v;
  if (kind === 'bool' && BOOL_RE.test(v)) return boolLiteral(engine, /^(true|1)$/i.test(v));
  // Text (dates, JSON, enums, UUIDs, …) is quoted exactly as typed — no trim.
  return `'${escapeSqlText(f.raw, backslashEscapes(engine))}'`;
}

export interface FormInsertDiff {
  row: number;
  path: string;
  op: 'cell';
  before: string;
  after: string;
}

/** Build the INSERT for the form. `tableRef` is the already-quoted
 *  (`db`.`t` / "schema"."t") target. Returns the first validation error
 *  instead when a field can't be written. */
export function buildFormInsert(
  engine: EngineLike,
  tableRef: string,
  fields: InsertField[],
): { sql: string; diff: FormInsertDiff[] } | { error: string; field?: string } {
  for (const f of fields) {
    const err = fieldError(f);
    if (err) return { error: `${f.name}: ${err}`, field: f.name };
  }
  const set = fields.filter((f) => !f.generated && f.mode !== 'default');
  const diff = set.map((f) => ({ row: -1, path: f.name, op: 'cell' as const, before: '∅', after: fieldLiteral(engine, f) }));
  if (set.length === 0) {
    // Every column takes its default.
    if (engine === 'postgres') return { sql: `INSERT INTO ${tableRef} DEFAULT VALUES;`, diff };
    if (engine === 'clickhouse') return { error: 'Set at least one column — ClickHouse has no all-default INSERT' };
    return { sql: `INSERT INTO ${tableRef} () VALUES ();`, diff };
  }
  const cols = set.map((f) => quoteIdent(engine, f.name)).join(', ');
  const vals = set.map((f) => fieldLiteral(engine, f)).join(', ');
  return { sql: `INSERT INTO ${tableRef} (${cols}) VALUES (${vals});`, diff };
}

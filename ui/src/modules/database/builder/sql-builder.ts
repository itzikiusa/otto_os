// The visual query builder's model + SQL generator + round-trip parser, for
// the SQL engines (MySQL, PostgreSQL, ClickHouse). Pure and self-contained —
// no Svelte, no relative value imports — so node:test runs it directly
// (unit/dbQueryBuilder.test.ts).
//
// Safety model: every identifier goes through `quoteIdent` and every VALUE the
// user types goes through `literal` (or `likeLiteral` for the contains /
// starts-with / ends-with operators), escaped for the dialect. The only text
// emitted verbatim is an explicit "expression" select item — raw SQL the user
// writes on purpose, exactly like the editor.

// ── Model ─────────────────────────────────────────────────────────────────────

export type Dialect = 'mysql' | 'postgres' | 'clickhouse';

export type AggFn =
  | 'COUNT'
  | 'COUNT_DISTINCT'
  | 'SUM'
  | 'AVG'
  | 'MIN'
  | 'MAX'
  | 'GROUP_CONCAT'
  | 'STRING_AGG'
  | 'groupArray'
  | 'uniq';

/** `column === '*'` only with COUNT. `alias` is the table alias in FROM/JOIN. */
export interface ColRef {
  alias: string;
  column: string;
}

export interface SourceTable {
  alias: string;
  schema?: string | null;
  table: string;
}

export type JoinType = 'INNER' | 'LEFT' | 'RIGHT' | 'FULL';

export interface JoinSpec {
  type: JoinType;
  table: SourceTable;
  /** Equality pairs, ANDed. `left` is on an already-joined table. */
  on: { left: ColRef; right: ColRef }[];
}

export type SelectItem =
  | { kind: 'column'; id: string; ref: ColRef; as?: string }
  | { kind: 'aggregate'; id: string; fn: AggFn; ref: ColRef | null; as?: string }
  | { kind: 'expr'; id: string; sql: string; as?: string };

export type Op =
  | '='
  | '!='
  | '<'
  | '<='
  | '>'
  | '>='
  | 'CONTAINS'
  | 'NOT_CONTAINS'
  | 'STARTS_WITH'
  | 'ENDS_WITH'
  | 'LIKE'
  | 'NOT_LIKE'
  | 'IN'
  | 'NOT_IN'
  | 'BETWEEN'
  | 'NOT_BETWEEN'
  | 'IS_NULL'
  | 'IS_NOT_NULL'
  | 'IS_TRUE'
  | 'IS_FALSE';

/** A predicate's left-hand side: a column, or (HAVING) an aggregate of one. */
export interface CondTarget {
  ref: ColRef;
  fn?: AggFn | null;
}

export interface Condition {
  kind: 'cond';
  id: string;
  target: CondTarget;
  op: Op;
  value?: string;
  /** Upper bound for BETWEEN. */
  value2?: string;
  /** Items for IN / NOT IN. */
  values?: string[];
}

export interface CondGroup {
  kind: 'group';
  id: string;
  conj: 'AND' | 'OR';
  items: (Condition | CondGroup)[];
}

export type OrderTarget = { kind: 'column'; ref: ColRef } | { kind: 'select'; id: string };

export interface OrderItem {
  id: string;
  target: OrderTarget;
  dir: 'ASC' | 'DESC';
  nulls: 'default' | 'first' | 'last';
}

export interface BuilderQuery {
  dialect: Dialect;
  distinct: boolean;
  from: SourceTable | null;
  joins: JoinSpec[];
  select: SelectItem[];
  where: CondGroup;
  groupBy: ColRef[];
  having: CondGroup;
  orderBy: OrderItem[];
  limit: number | null;
  offset: number | null;
  /** `alias.column` → engine type, for value typing and operator choice. */
  types?: Record<string, string>;
}

let seq = 0;
/** Monotonic id for model nodes (unique within a session). */
export function nid(prefix = 'n'): string {
  seq += 1;
  return `${prefix}${seq}`;
}

export function emptyGroup(conj: 'AND' | 'OR' = 'AND'): CondGroup {
  return { kind: 'group', id: nid('g'), conj, items: [] };
}

export function emptyQuery(dialect: Dialect): BuilderQuery {
  return {
    dialect,
    distinct: false,
    from: null,
    joins: [],
    select: [],
    where: emptyGroup(),
    groupBy: [],
    having: emptyGroup(),
    orderBy: [],
    limit: 100,
    offset: null,
  };
}

// ── Types → value kinds and operators ─────────────────────────────────────────

export type ValueKind = 'number' | 'string' | 'bool' | 'time' | 'json';

export function valueKind(type: string | null | undefined): ValueKind {
  const t = (type ?? '').trim().toLowerCase().replace(/^nullable\(|^lowcardinality\(/g, '');
  if (/^(bool|boolean|bit\(1\)|tinyint\(1\))/.test(t)) return 'bool';
  if (/^(u?int\d*|tinyint|smallint|mediumint|bigint|int|integer|serial|bigserial|smallserial|dec|decimal|numeric|float\d*|double|real|money)/.test(t)) return 'number';
  if (/^(date|datetime|timestamp|time|year|interval)/.test(t)) return 'time';
  if (/^(json|jsonb|map|array|tuple|object)/.test(t)) return 'json';
  return 'string';
}

export interface OpInfo {
  op: Op;
  label: string;
  /** How many value inputs: 0, 1, 2 (between) or 'list' (IN). */
  arity: 0 | 1 | 2 | 'list';
}

export const OPS: Record<Op, OpInfo> = {
  '=': { op: '=', label: '=', arity: 1 },
  '!=': { op: '!=', label: '≠', arity: 1 },
  '<': { op: '<', label: '<', arity: 1 },
  '<=': { op: '<=', label: '≤', arity: 1 },
  '>': { op: '>', label: '>', arity: 1 },
  '>=': { op: '>=', label: '≥', arity: 1 },
  CONTAINS: { op: 'CONTAINS', label: 'contains', arity: 1 },
  NOT_CONTAINS: { op: 'NOT_CONTAINS', label: 'does not contain', arity: 1 },
  STARTS_WITH: { op: 'STARTS_WITH', label: 'starts with', arity: 1 },
  ENDS_WITH: { op: 'ENDS_WITH', label: 'ends with', arity: 1 },
  LIKE: { op: 'LIKE', label: 'LIKE', arity: 1 },
  NOT_LIKE: { op: 'NOT_LIKE', label: 'NOT LIKE', arity: 1 },
  IN: { op: 'IN', label: 'in', arity: 'list' },
  NOT_IN: { op: 'NOT_IN', label: 'not in', arity: 'list' },
  BETWEEN: { op: 'BETWEEN', label: 'between', arity: 2 },
  NOT_BETWEEN: { op: 'NOT_BETWEEN', label: 'not between', arity: 2 },
  IS_NULL: { op: 'IS_NULL', label: 'is NULL', arity: 0 },
  IS_NOT_NULL: { op: 'IS_NOT_NULL', label: 'is not NULL', arity: 0 },
  IS_TRUE: { op: 'IS_TRUE', label: 'is true', arity: 0 },
  IS_FALSE: { op: 'IS_FALSE', label: 'is false', arity: 0 },
};

/** Operators that make sense for a value kind, most useful first. */
export function opsFor(kind: ValueKind): Op[] {
  const nulls: Op[] = ['IS_NULL', 'IS_NOT_NULL'];
  switch (kind) {
    case 'number':
    case 'time':
      return ['=', '!=', '<', '<=', '>', '>=', 'BETWEEN', 'NOT_BETWEEN', 'IN', 'NOT_IN', ...nulls];
    case 'bool':
      return ['IS_TRUE', 'IS_FALSE', '=', '!=', ...nulls];
    case 'json':
      return [...nulls, '=', '!=', 'CONTAINS', 'LIKE'];
    default:
      return [
        '=', '!=', 'CONTAINS', 'NOT_CONTAINS', 'STARTS_WITH', 'ENDS_WITH', 'IN', 'NOT_IN',
        'LIKE', 'NOT_LIKE', '<', '<=', '>', '>=', 'BETWEEN', ...nulls,
      ];
  }
}

export interface AggInfo {
  fn: AggFn;
  label: string;
  /** Needs a numeric argument (SUM/AVG). */
  numeric?: boolean;
}

/** Aggregates available on a dialect (COUNT DISTINCT spelled per engine). */
export function aggregatesFor(dialect: Dialect): AggInfo[] {
  const base: AggInfo[] = [
    { fn: 'COUNT', label: 'COUNT' },
    { fn: 'COUNT_DISTINCT', label: 'COUNT DISTINCT' },
    { fn: 'SUM', label: 'SUM', numeric: true },
    { fn: 'AVG', label: 'AVG', numeric: true },
    { fn: 'MIN', label: 'MIN' },
    { fn: 'MAX', label: 'MAX' },
  ];
  if (dialect === 'mysql') base.push({ fn: 'GROUP_CONCAT', label: 'GROUP_CONCAT' });
  if (dialect === 'postgres') base.push({ fn: 'STRING_AGG', label: 'STRING_AGG' });
  if (dialect === 'clickhouse') base.push({ fn: 'uniq', label: 'uniq (approx.)' }, { fn: 'groupArray', label: 'groupArray' });
  return base;
}

// ── Quoting / escaping ───────────────────────────────────────────────────────

/** Quote an identifier for the dialect. Postgres uses the standard double
 *  quotes; MySQL and ClickHouse backticks. The quote character is doubled; on
 *  ClickHouse (where `\` escapes inside quoted names too) backslashes and the
 *  backtick are backslash-escaped instead. */
export function quoteIdent(dialect: Dialect, name: string): string {
  if (dialect === 'postgres') return `"${name.replace(/"/g, '""')}"`;
  if (dialect === 'clickhouse') return `\`${name.replace(/\\/g, '\\\\').replace(/`/g, '\\`')}\``;
  return `\`${name.replace(/`/g, '``')}\``;
}

/** A string literal. MySQL/ClickHouse treat `\` as an escape inside strings
 *  (MySQL's default sql_mode), so it is doubled there; the quote is doubled
 *  (MySQL, Postgres) or backslash-escaped (ClickHouse). Control characters that
 *  could terminate or confuse a literal (NUL, CR/LF) are escaped where the
 *  dialect has a spelling for them. */
export function stringLiteral(dialect: Dialect, s: string): string {
  if (dialect === 'postgres') {
    // standard_conforming_strings (on since 9.1): only the quote is special.
    // NUL cannot appear in a Postgres text value at all — drop it.
    return `'${s.replace(/\0/g, '').replace(/'/g, "''")}'`;
  }
  let out = s.replace(/\\/g, '\\\\');
  out = dialect === 'clickhouse' ? out.replace(/'/g, "\\'") : out.replace(/'/g, "''");
  out = out.replace(/\0/g, '\\0');
  return `'${out}'`;
}

const NUM_RE = /^-?(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][-+]?\d+)?$/;

/** A typed value: numbers stay bare ONLY when they parse as a number (anything
 *  else becomes a quoted string — never raw text); booleans become TRUE/FALSE;
 *  everything else is a string literal. A string-typed column always gets a
 *  quoted value, even `123`, so MySQL compares it as text and can use the index. */
export function literal(dialect: Dialect, raw: string, kind: ValueKind): string {
  const v = raw.trim();
  if (kind === 'number' && NUM_RE.test(v)) return v;
  if (kind === 'bool') {
    const l = v.toLowerCase();
    if (l === 'true' || l === '1' || l === 't' || l === 'yes') return dialect === 'clickhouse' ? 'true' : 'TRUE';
    if (l === 'false' || l === '0' || l === 'f' || l === 'no') return dialect === 'clickhouse' ? 'false' : 'FALSE';
  }
  return stringLiteral(dialect, raw);
}

/** Escape LIKE wildcards so user text matches literally (`\` is the default
 *  LIKE escape on all three engines). */
export function escapeLike(s: string): string {
  return s.replace(/\\/g, '\\\\').replace(/%/g, '\\%').replace(/_/g, '\\_');
}

/** Split an IN list typed as `a, b, 'c, d'` — quotes group, and wrap, an item. */
export function splitList(text: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: string | null = null;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quote) {
      if (ch === quote) {
        if (text[i + 1] === quote) {
          cur += ch;
          i++;
        } else quote = null;
      } else cur += ch;
    } else if (ch === "'" || ch === '"') {
      quote = ch;
    } else if (ch === ',' || ch === '\n') {
      if (cur.trim() !== '') out.push(cur.trim());
      cur = '';
    } else cur += ch;
  }
  if (cur.trim() !== '') out.push(cur.trim());
  return out;
}

// ── Generation ───────────────────────────────────────────────────────────────

function typeOf(q: BuilderQuery, ref: ColRef): string | undefined {
  return q.types?.[`${ref.alias}.${ref.column}`];
}

/** Column references are qualified by alias only when the query joins —
 *  a single-table query reads `status`, not `orders`.`status`. */
function colSql(q: BuilderQuery, ref: ColRef): string {
  if (ref.column === '*') return q.joins.length ? `${quoteIdent(q.dialect, ref.alias)}.*` : '*';
  const c = quoteIdent(q.dialect, ref.column);
  return q.joins.length ? `${quoteIdent(q.dialect, ref.alias)}.${c}` : c;
}

export function aggSql(q: BuilderQuery, fn: AggFn, ref: ColRef | null): string {
  const arg = ref ? colSql(q, ref) : '*';
  switch (fn) {
    case 'COUNT':
      return `COUNT(${arg})`;
    case 'COUNT_DISTINCT':
      return q.dialect === 'clickhouse' ? `uniqExact(${arg})` : `COUNT(DISTINCT ${arg})`;
    case 'GROUP_CONCAT':
      return `GROUP_CONCAT(${arg})`;
    case 'STRING_AGG':
      return `STRING_AGG(${arg}::text, ', ')`;
    default:
      return `${fn}(${arg})`;
  }
}

function tableSql(q: BuilderQuery, t: SourceTable): string {
  const name = t.schema ? `${quoteIdent(q.dialect, t.schema)}.${quoteIdent(q.dialect, t.table)}` : quoteIdent(q.dialect, t.table);
  // The alias is only spelled when it differs from the table name.
  return t.alias && t.alias !== t.table ? `${name} AS ${quoteIdent(q.dialect, t.alias)}` : name;
}

/** Default output alias for an aggregate, e.g. SUM(total) → `sum_total`. */
export function defaultAggAlias(fn: AggFn, ref: ColRef | null): string {
  const f = fn.toLowerCase();
  return ref && ref.column !== '*' ? `${f}_${ref.column}` : f === 'count' ? 'count' : `${f}_all`;
}

function selectItemSql(q: BuilderQuery, it: SelectItem): string {
  const as = it.as?.trim();
  const alias = as ? ` AS ${quoteIdent(q.dialect, as)}` : '';
  if (it.kind === 'column') return `${colSql(q, it.ref)}${alias}`;
  if (it.kind === 'aggregate') return `${aggSql(q, it.fn, it.ref)}${alias}`;
  return `${it.sql.trim()}${alias}`;
}

function condIsComplete(c: Condition): boolean {
  const a = OPS[c.op].arity;
  if (a === 0) return true;
  // An empty value is a half-typed row, not "= ''" — NULL / empty-string
  // checks are explicit operators (IS NULL) or a quoted value.
  if (a === 1) return (c.value ?? '') !== '';
  if (a === 2) return (c.value ?? '').trim() !== '' && (c.value2 ?? '').trim() !== '';
  return (c.values ?? []).length > 0;
}

function condSql(q: BuilderQuery, c: Condition): string | null {
  if (!condIsComplete(c)) return null;
  const lhs = c.target.fn ? aggSql(q, c.target.fn, c.target.ref.column === '*' ? null : c.target.ref) : colSql(q, c.target.ref);
  // An aggregate's value is numeric (COUNT/SUM/AVG); MIN/MAX follow the column.
  const kind: ValueKind =
    c.target.fn && c.target.fn !== 'MIN' && c.target.fn !== 'MAX'
      ? 'number'
      : valueKind(typeOf(q, c.target.ref));
  const lit = (v: string): string => literal(q.dialect, v, kind);
  const v = c.value ?? '';
  switch (c.op) {
    case 'IS_NULL':
      return `${lhs} IS NULL`;
    case 'IS_NOT_NULL':
      return `${lhs} IS NOT NULL`;
    case 'IS_TRUE':
      return q.dialect === 'clickhouse' ? `${lhs} = true` : `${lhs} IS TRUE`;
    case 'IS_FALSE':
      return q.dialect === 'clickhouse' ? `${lhs} = false` : `${lhs} IS FALSE`;
    case 'CONTAINS':
      return `${lhs} LIKE ${stringLiteral(q.dialect, `%${escapeLike(v)}%`)}`;
    case 'NOT_CONTAINS':
      return `${lhs} NOT LIKE ${stringLiteral(q.dialect, `%${escapeLike(v)}%`)}`;
    case 'STARTS_WITH':
      return `${lhs} LIKE ${stringLiteral(q.dialect, `${escapeLike(v)}%`)}`;
    case 'ENDS_WITH':
      return `${lhs} LIKE ${stringLiteral(q.dialect, `%${escapeLike(v)}`)}`;
    case 'LIKE':
      return `${lhs} LIKE ${stringLiteral(q.dialect, v)}`;
    case 'NOT_LIKE':
      return `${lhs} NOT LIKE ${stringLiteral(q.dialect, v)}`;
    case 'IN':
    case 'NOT_IN':
      return `${lhs} ${c.op === 'IN' ? 'IN' : 'NOT IN'} (${(c.values ?? []).map(lit).join(', ')})`;
    case 'BETWEEN':
    case 'NOT_BETWEEN':
      return `${lhs} ${c.op === 'BETWEEN' ? 'BETWEEN' : 'NOT BETWEEN'} ${lit(v)} AND ${lit(c.value2 ?? '')}`;
    case '=':
    case '!=':
      // "= (empty)" on a text column compares to '' — say so in the SQL rather
      // than guessing NULL (the NULL operators are explicit).
      return `${lhs} ${c.op === '=' ? '=' : '<>'} ${lit(v)}`;
    default:
      return `${lhs} ${c.op} ${lit(v)}`;
  }
}

/** Render a condition group; nested groups are parenthesized. Empty or
 *  incomplete members are skipped (a half-typed row never breaks the SQL). */
export function groupSql(q: BuilderQuery, g: CondGroup, nested = false): string | null {
  const parts: string[] = [];
  for (const it of g.items) {
    const s = it.kind === 'cond' ? condSql(q, it) : groupSql(q, it, true);
    if (s) parts.push(s);
  }
  if (!parts.length) return null;
  if (parts.length === 1) return parts[0];
  const joined = parts.join(nested ? ` ${g.conj} ` : `\n  ${g.conj} `);
  return nested ? `(${joined})` : joined;
}

function orderSql(q: BuilderQuery, o: OrderItem): string | null {
  let expr: string;
  if (o.target.kind === 'column') expr = colSql(q, o.target.ref);
  else {
    const id = o.target.id;
    const it = q.select.find((s) => s.id === id);
    if (!it) return null;
    // An aliased output column is ordered by its alias (valid on all three);
    // otherwise by the expression itself.
    if (it.as?.trim()) expr = quoteIdent(q.dialect, it.as.trim());
    else if (it.kind === 'column') expr = colSql(q, it.ref);
    else if (it.kind === 'aggregate') expr = aggSql(q, it.fn, it.ref);
    else expr = it.sql.trim();
  }
  if (o.nulls === 'default') return `${expr} ${o.dir}`;
  if (q.dialect === 'mysql') {
    // MySQL has no NULLS FIRST/LAST: sort on `expr IS NULL` first (false < true).
    return `${expr} IS NULL ${o.nulls === 'first' ? 'DESC' : 'ASC'}, ${expr} ${o.dir}`;
  }
  return `${expr} ${o.dir} NULLS ${o.nulls === 'first' ? 'FIRST' : 'LAST'}`;
}

/** The full SELECT for the model, or '' when there is no FROM table yet. */
export function buildSelect(q: BuilderQuery): string {
  if (!q.from) return '';
  const lines: string[] = [];
  const items = q.select.filter((it) => it.kind !== 'expr' || it.sql.trim() !== '');
  const list = items.length ? items.map((it) => selectItemSql(q, it)) : ['*'];
  const head = `SELECT ${q.distinct ? 'DISTINCT ' : ''}`;
  lines.push(list.length > 3 ? `${head}\n  ${list.join(',\n  ')}` : `${head}${list.join(', ')}`);
  lines.push(`FROM ${tableSql(q, q.from)}`);
  for (const j of q.joins) {
    const on = j.on
      .map((p) => `${colSqlQ(q, p.left)} = ${colSqlQ(q, p.right)}`)
      .join(' AND ');
    const kw = j.type === 'FULL' ? 'FULL OUTER JOIN' : `${j.type} JOIN`;
    lines.push(on ? `${kw} ${tableSql(q, j.table)} ON ${on}` : `CROSS JOIN ${tableSql(q, j.table)}`);
  }
  const where = groupSql(q, q.where);
  if (where) lines.push(`WHERE ${where}`);
  if (q.groupBy.length) lines.push(`GROUP BY ${q.groupBy.map((r) => colSql(q, r)).join(', ')}`);
  const having = groupSql(q, q.having);
  if (having) lines.push(`HAVING ${having}`);
  const order = q.orderBy.map((o) => orderSql(q, o)).filter((s): s is string => !!s);
  if (order.length) lines.push(`ORDER BY ${order.join(', ')}`);
  if (q.limit !== null && Number.isFinite(q.limit) && q.limit >= 0) {
    lines.push(`LIMIT ${Math.floor(q.limit)}${q.offset ? ` OFFSET ${Math.floor(q.offset)}` : ''}`);
  } else if (q.offset) {
    // Postgres/ClickHouse accept a bare OFFSET; MySQL needs a LIMIT with it.
    lines.push(q.dialect === 'mysql' ? `LIMIT 18446744073709551615 OFFSET ${Math.floor(q.offset)}` : `OFFSET ${Math.floor(q.offset)}`);
  }
  return `${lines.join('\n')};`;
}

/** JOIN ON sides are always qualified (both tables are in scope). */
function colSqlQ(q: BuilderQuery, ref: ColRef): string {
  return `${quoteIdent(q.dialect, ref.alias)}.${quoteIdent(q.dialect, ref.column)}`;
}

// ── Validation ───────────────────────────────────────────────────────────────

export interface Issue {
  level: 'error' | 'warn';
  message: string;
  /** Machine hint the UI can offer as a one-click fix. */
  fix?: { kind: 'group-by'; refs: ColRef[] };
}

const refKey = (r: ColRef): string => `${r.alias}.${r.column}`;

/** Things that would make the server reject the statement (or surprise the
 *  user), stated in words. */
export function validate(q: BuilderQuery): Issue[] {
  const out: Issue[] = [];
  if (!q.from) return out;
  const aggregated = q.select.some((s) => s.kind === 'aggregate') || q.groupBy.length > 0;
  if (aggregated) {
    const grouped = new Set(q.groupBy.map(refKey));
    const loose = q.select
      .filter((s): s is Extract<SelectItem, { kind: 'column' }> => s.kind === 'column')
      .filter((s) => s.ref.column !== '*' && !grouped.has(refKey(s.ref)))
      .map((s) => s.ref);
    if (loose.length) {
      out.push({
        level: 'error',
        message: `${loose.map((r) => r.column).join(', ')} ${loose.length === 1 ? 'is' : 'are'} selected but neither grouped nor aggregated.`,
        fix: { kind: 'group-by', refs: loose },
      });
    }
    if (q.select.some((s) => s.kind === 'column' && s.ref.column === '*')) {
      out.push({ level: 'error', message: 'SELECT * can’t be combined with GROUP BY — pick columns.' });
    }
  }
  const hasHaving = groupSql(q, q.having) !== null;
  if (hasHaving && !aggregated) {
    out.push({ level: 'warn', message: 'HAVING without GROUP BY or an aggregate filters the whole result as one group.' });
  }
  for (const it of q.select) {
    if (it.kind === 'aggregate' && (it.fn === 'SUM' || it.fn === 'AVG') && it.ref) {
      const k = valueKind(typeOf(q, it.ref));
      if (k !== 'number' && typeOf(q, it.ref)) {
        out.push({ level: 'warn', message: `${it.fn} of ${it.ref.column} (${typeOf(q, it.ref)}) — not a numeric column.` });
      }
    }
  }
  const aliases = new Map<string, number>();
  for (const it of q.select) {
    const a = it.as?.trim();
    if (a) aliases.set(a, (aliases.get(a) ?? 0) + 1);
  }
  for (const [a, n] of aliases) if (n > 1) out.push({ level: 'error', message: `The output name “${a}” is used ${n} times.` });
  if (q.offset && q.orderBy.length === 0) {
    out.push({ level: 'warn', message: 'OFFSET without ORDER BY — pages can repeat or skip rows.' });
  }
  return out;
}

/** Plain column items not covered by GROUP BY — what "Group by them" adds. */
export function suggestGroupBy(q: BuilderQuery): ColRef[] {
  const grouped = new Set(q.groupBy.map(refKey));
  return q.select
    .filter((s): s is Extract<SelectItem, { kind: 'column' }> => s.kind === 'column' && s.ref.column !== '*')
    .map((s) => s.ref)
    .filter((r) => !grouped.has(refKey(r)));
}

// ── Round trip: parse a SELECT back into the model ───────────────────────────
//
// Handles the shape the builder emits (and the common hand-written subset):
// SELECT [DISTINCT] cols/aggregates/expressions FROM t [AS a] [JOINs ON a.x =
// b.y [AND …]] [WHERE …] [GROUP BY …] [HAVING …] [ORDER BY … [NULLS …]]
// [LIMIT n [OFFSET m] | LIMIT m, n]. Anything else (sub-queries, UNION, CTEs,
// functions in WHERE) returns an error naming what didn't fit — the editor
// keeps the statement untouched.

type Tok =
  | { t: 'id'; v: string; quoted: boolean }
  | { t: 'str'; v: string }
  | { t: 'num'; v: string }
  | { t: 'op'; v: string }
  | { t: 'eof'; v: '' };

function tokenize(sql: string, dialect: Dialect): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const ch = sql[i];
    if (/\s/.test(ch)) {
      i++;
      continue;
    }
    if (ch === '-' && sql[i + 1] === '-') {
      while (i < n && sql[i] !== '\n') i++;
      continue;
    }
    if (ch === '/' && sql[i + 1] === '*') {
      const end = sql.indexOf('*/', i + 2);
      i = end < 0 ? n : end + 2;
      continue;
    }
    if (ch === "'") {
      let v = '';
      i++;
      while (i < n) {
        const c = sql[i];
        if (c === '\\' && dialect !== 'postgres') {
          const nx = sql[i + 1] ?? '';
          v += nx === '0' ? '\0' : nx === 'n' ? '\n' : nx === 't' ? '\t' : nx === 'r' ? '\r' : nx;
          i += 2;
          continue;
        }
        if (c === "'") {
          if (sql[i + 1] === "'") {
            v += "'";
            i += 2;
            continue;
          }
          i++;
          break;
        }
        v += c;
        i++;
      }
      out.push({ t: 'str', v });
      continue;
    }
    if (ch === '`' || ch === '"') {
      let v = '';
      i++;
      while (i < n) {
        const c = sql[i];
        if (c === '\\' && dialect === 'clickhouse' && ch === '`') {
          v += sql[i + 1] ?? '';
          i += 2;
          continue;
        }
        if (c === ch) {
          if (sql[i + 1] === ch) {
            v += ch;
            i += 2;
            continue;
          }
          i++;
          break;
        }
        v += c;
        i++;
      }
      out.push({ t: 'id', v, quoted: true });
      continue;
    }
    const num = /^(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][-+]?\d+)?/.exec(sql.slice(i));
    if (num) {
      out.push({ t: 'num', v: num[0] });
      i += num[0].length;
      continue;
    }
    const word = /^[A-Za-z_][A-Za-z0-9_$]*/.exec(sql.slice(i));
    if (word) {
      out.push({ t: 'id', v: word[0], quoted: false });
      i += word[0].length;
      continue;
    }
    const op = /^(<>|!=|<=|>=|::|[=<>(),.*;+\-/%])/.exec(sql.slice(i));
    if (op) {
      out.push({ t: 'op', v: op[0] });
      i += op[0].length;
      continue;
    }
    throw new Error(`unexpected character “${ch}”`);
  }
  out.push({ t: 'eof', v: '' });
  return out;
}

const CLAUSE_WORDS = new Set([
  'from', 'where', 'group', 'having', 'order', 'limit', 'offset', 'join', 'inner', 'left', 'right',
  'full', 'cross', 'on', 'union', 'as', 'and', 'or', 'not', 'settings', 'format', 'with',
]);
const AGG_NAMES: Record<string, AggFn> = {
  count: 'COUNT',
  sum: 'SUM',
  avg: 'AVG',
  min: 'MIN',
  max: 'MAX',
  group_concat: 'GROUP_CONCAT',
  string_agg: 'STRING_AGG',
  grouparray: 'groupArray',
  uniq: 'uniq',
  uniqexact: 'COUNT_DISTINCT',
};

class Parser {
  i = 0;
  toks: Tok[];
  dialect: Dialect;
  constructor(toks: Tok[], dialect: Dialect) {
    this.toks = toks;
    this.dialect = dialect;
  }
  peek(k = 0): Tok {
    return this.toks[Math.min(this.i + k, this.toks.length - 1)];
  }
  next(): Tok {
    const t = this.peek();
    this.i++;
    return t;
  }
  isKw(w: string, k = 0): boolean {
    const t = this.peek(k);
    return t.t === 'id' && !t.quoted && t.v.toLowerCase() === w;
  }
  eatKw(w: string): boolean {
    if (this.isKw(w)) {
      this.i++;
      return true;
    }
    return false;
  }
  expectKw(w: string): void {
    if (!this.eatKw(w)) throw new Error(`expected ${w.toUpperCase()}`);
  }
  isOp(v: string, k = 0): boolean {
    const t = this.peek(k);
    return t.t === 'op' && t.v === v;
  }
  eatOp(v: string): boolean {
    if (this.isOp(v)) {
      this.i++;
      return true;
    }
    return false;
  }
  expectOp(v: string): void {
    if (!this.eatOp(v)) throw new Error(`expected “${v}”`);
  }
  ident(): string {
    const t = this.next();
    if (t.t !== 'id') throw new Error('expected a name');
    return t.v;
  }
  /** `a`, `a.b`, `a.b.c` → the name parts. */
  dotted(): string[] {
    const parts = [this.ident()];
    while (this.isOp('.')) {
      this.i++;
      if (this.isOp('*')) {
        this.i++;
        parts.push('*');
        break;
      }
      parts.push(this.ident());
    }
    return parts;
  }
}

function isAggStart(p: Parser): AggFn | null {
  const t = p.peek();
  if (t.t !== 'id' || t.quoted || !p.isOp('(', 1)) return null;
  return AGG_NAMES[t.v.toLowerCase()] ?? null;
}

export type ParseResult = { ok: true; query: BuilderQuery } | { ok: false; error: string };

/**
 * Parse a SELECT back into a builder model. `defaultSchema` fills a missing
 * schema on unqualified tables. Column refs resolve to table aliases; an
 * unqualified column belongs to the FROM table when there are no joins (the
 * builder's own output) — with joins, unqualified names are rejected as
 * ambiguous.
 */
export function parseSelect(sql: string, dialect: Dialect): ParseResult {
  let toks: Tok[];
  try {
    toks = tokenize(sql, dialect);
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  const p = new Parser(toks, dialect);
  const q = emptyQuery(dialect);
  q.limit = null;
  try {
    p.expectKw('select');
    if (p.eatKw('distinct')) q.distinct = true;
    // Defer resolving select items until FROM names the aliases.
    const rawItems: { start: number; end: number }[] = [];
    let depth = 0;
    let start = p.i;
    for (;;) {
      const t = p.peek();
      if (t.t === 'eof') throw new Error('expected FROM');
      if (t.t === 'op' && t.v === '(') depth++;
      else if (t.t === 'op' && t.v === ')') depth--;
      else if (depth === 0 && t.t === 'op' && t.v === ',') {
        rawItems.push({ start, end: p.i });
        p.i++;
        start = p.i;
        continue;
      } else if (depth === 0 && p.isKw('from')) {
        rawItems.push({ start, end: p.i });
        break;
      }
      p.i++;
    }
    p.expectKw('from');
    const tableRef = (): SourceTable => {
      const parts = p.dotted();
      const table = parts[parts.length - 1];
      const schema = parts.length > 1 ? parts[parts.length - 2] : null;
      let alias = table;
      if (p.eatKw('as')) alias = p.ident();
      else {
        const t = p.peek();
        if (t.t === 'id' && (t.quoted || !CLAUSE_WORDS.has(t.v.toLowerCase()))) alias = p.ident();
      }
      return { alias, schema, table };
    };
    q.from = tableRef();
    if (p.isOp('(') || p.isOp(',')) throw new Error('sub-queries and comma joins are not supported');
    const aliases = new Set([q.from.alias]);
    // JOINs
    for (;;) {
      let type: JoinType | null = null;
      if (p.eatKw('join')) type = 'INNER';
      else if (p.isKw('inner') && p.isKw('join', 1)) {
        p.i += 2;
        type = 'INNER';
      } else if ((p.isKw('left') || p.isKw('right') || p.isKw('full')) && (p.isKw('join', 1) || (p.isKw('outer', 1) && p.isKw('join', 2)))) {
        const w = p.ident().toUpperCase() as JoinType;
        p.eatKw('outer');
        p.expectKw('join');
        type = w;
      } else break;
      const table = tableRef();
      if (aliases.has(table.alias)) throw new Error(`duplicate table alias “${table.alias}”`);
      aliases.add(table.alias);
      p.expectKw('on');
      const on: { left: ColRef; right: ColRef }[] = [];
      do {
        const a = p.dotted();
        p.expectOp('=');
        const b = p.dotted();
        if (a.length < 2 || b.length < 2) throw new Error('JOIN conditions must name table.column on both sides');
        const l = { alias: a[a.length - 2], column: a[a.length - 1] };
        const r = { alias: b[b.length - 2], column: b[b.length - 1] };
        // Keep `left` on the side that was already in scope.
        if (r.alias !== table.alias && l.alias === table.alias) on.push({ left: r, right: l });
        else on.push({ left: l, right: r });
      } while (p.eatKw('and'));
      q.joins.push({ type, table, on });
    }
    const resolve = (parts: string[]): ColRef => {
      if (parts.length === 1) {
        if (q.joins.length) throw new Error(`“${parts[0]}” is ambiguous — qualify it with a table`);
        return { alias: q.from!.alias, column: parts[0] };
      }
      const alias = parts[parts.length - 2];
      if (!aliases.has(alias)) {
        // `schema.table.col` or `table.col` where the table has an alias.
        const byTable = [q.from!, ...q.joins.map((j) => j.table)].find((t) => t.table === alias);
        if (!byTable) throw new Error(`unknown table “${alias}”`);
        return { alias: byTable.alias, column: parts[parts.length - 1] };
      }
      return { alias, column: parts[parts.length - 1] };
    };
    // Resolve the deferred select list now that aliases are known.
    const saved = p.i;
    for (const r of rawItems) {
      const sub = new Parser(toks.slice(r.start, r.end).concat([{ t: 'eof', v: '' }]), dialect);
      const id = nid('s');
      const aliasOf = (): string | undefined => {
        if (sub.eatKw('as')) return sub.ident();
        const t = sub.peek();
        if (t.t === 'id' && sub.peek(1).t === 'eof') return sub.ident();
        return undefined;
      };
      if (sub.isOp('*') && sub.peek(1).t === 'eof') continue; // SELECT *
      const fn = isAggStart(sub);
      if (fn) {
        sub.i += 2;
        let f: AggFn = fn;
        let ref: ColRef | null = null;
        if (sub.eatKw('distinct')) {
          if (fn !== 'COUNT') throw new Error(`${fn}(DISTINCT …) is not supported`);
          f = 'COUNT_DISTINCT';
        }
        if (sub.eatOp('*')) ref = null;
        else ref = resolve(sub.dotted());
        if (fn === 'STRING_AGG') {
          // STRING_AGG(x::text, ', ') — accept the builder's own spelling.
          if (sub.eatOp('::')) sub.ident();
          if (sub.eatOp(',')) sub.next();
        }
        if (sub.eatOp(')')) {
          const as = aliasOf();
          if (sub.peek().t === 'eof') {
            q.select.push({ kind: 'aggregate', id, fn: f, ref, as });
            continue;
          }
        }
      } else {
        const t0 = sub.peek();
        if (t0.t === 'id') {
          const at = sub.i;
          try {
            const parts = sub.dotted();
            const as = aliasOf();
            if (sub.peek().t === 'eof') {
              const ref = resolve(parts);
              q.select.push({ kind: 'column', id, ref, as });
              continue;
            }
          } catch {
            /* not a plain column — fall through to an expression */
          }
          sub.i = at;
        }
      }
      // Anything else is kept verbatim as an expression item (+ trailing alias).
      const slice = toks.slice(r.start, r.end);
      let as: string | undefined;
      let body = slice;
      const last = slice[slice.length - 1];
      const beforeLast = slice[slice.length - 2];
      if (last?.t === 'id' && beforeLast?.t === 'id' && !beforeLast.quoted && beforeLast.v.toLowerCase() === 'as') {
        as = last.v;
        body = slice.slice(0, -2);
      }
      q.select.push({ kind: 'expr', id, sql: renderTokens(body, dialect), as });
    }
    p.i = saved;

    const valueTok = (): string => {
      const t = p.next();
      if (t.t === 'str' || t.t === 'num') return t.v;
      if (t.t === 'op' && t.v === '-' && p.peek().t === 'num') return `-${p.next().v}`;
      if (t.t === 'id' && !t.quoted && /^(true|false)$/i.test(t.v)) return t.v.toLowerCase();
      throw new Error('expected a value (functions and sub-queries are not supported)');
    };
    const predicate = (allowAgg: boolean): Condition => {
      let target: CondTarget;
      const fn = allowAgg ? isAggStart(p) : null;
      if (fn) {
        p.i += 2;
        let f: AggFn = fn;
        if (p.eatKw('distinct')) f = 'COUNT_DISTINCT';
        const ref = p.eatOp('*') ? { alias: q.from!.alias, column: '*' } : resolve(p.dotted());
        p.expectOp(')');
        target = { ref, fn: f };
      } else {
        if (p.peek().t !== 'id') throw new Error('expected a column');
        target = { ref: resolve(p.dotted()) };
      }
      const id = nid('c');
      if (p.eatKw('is')) {
        const not = p.eatKw('not');
        if (p.eatKw('null')) return { kind: 'cond', id, target, op: not ? 'IS_NOT_NULL' : 'IS_NULL' };
        if (p.eatKw('true')) return { kind: 'cond', id, target, op: not ? 'IS_FALSE' : 'IS_TRUE' };
        if (p.eatKw('false')) return { kind: 'cond', id, target, op: not ? 'IS_TRUE' : 'IS_FALSE' };
        throw new Error('expected NULL, TRUE or FALSE after IS');
      }
      const not = p.eatKw('not');
      if (p.eatKw('in')) {
        p.expectOp('(');
        const values: string[] = [];
        do values.push(valueTok());
        while (p.eatOp(','));
        p.expectOp(')');
        return { kind: 'cond', id, target, op: not ? 'NOT_IN' : 'IN', values };
      }
      if (p.eatKw('between')) {
        const value = valueTok();
        p.expectKw('and');
        const value2 = valueTok();
        return { kind: 'cond', id, target, op: not ? 'NOT_BETWEEN' : 'BETWEEN', value, value2 };
      }
      if (p.eatKw('like')) {
        const pat = valueTok();
        return { kind: 'cond', id, target, ...likeToOp(pat, not) };
      }
      if (not) throw new Error('expected IN, BETWEEN or LIKE after NOT');
      const t = p.next();
      if (t.t !== 'op' || !['=', '!=', '<>', '<', '<=', '>', '>='].includes(t.v)) throw new Error('expected a comparison');
      const op = (t.v === '<>' ? '!=' : t.v) as Op;
      if (p.isKw('true') || p.isKw('false')) {
        const b = p.ident().toLowerCase() === 'true';
        if (op === '=' || op === '!=') return { kind: 'cond', id, target, op: (op === '=') === b ? 'IS_TRUE' : 'IS_FALSE' };
      }
      return { kind: 'cond', id, target, op, value: valueTok() };
    };
    const orExpr = (allowAgg: boolean): CondGroup => {
      const g: CondGroup = { kind: 'group', id: nid('g'), conj: 'OR', items: [andExpr(allowAgg)] };
      while (p.eatKw('or')) g.items.push(andExpr(allowAgg));
      return flatten(g);
    };
    const andExpr = (allowAgg: boolean): Condition | CondGroup => {
      const g: CondGroup = { kind: 'group', id: nid('g'), conj: 'AND', items: [primary(allowAgg)] };
      while (p.isKw('and')) {
        p.i++;
        g.items.push(primary(allowAgg));
      }
      return g.items.length === 1 ? g.items[0] : g;
    };
    const primary = (allowAgg: boolean): Condition | CondGroup => {
      if (p.eatOp('(')) {
        const g = orExpr(allowAgg);
        p.expectOp(')');
        return g;
      }
      if (p.isKw('not')) throw new Error('NOT (…) groups are not supported');
      return predicate(allowAgg);
    };
    if (p.eatKw('where')) q.where = asTopGroup(orExpr(false));
    if (p.isKw('group') && p.isKw('by', 1)) {
      p.i += 2;
      do q.groupBy.push(resolve(p.dotted()));
      while (p.eatOp(','));
    }
    if (p.eatKw('having')) q.having = asTopGroup(orExpr(true));
    if (p.isKw('order') && p.isKw('by', 1)) {
      p.i += 2;
      do {
        let target: OrderTarget;
        const fn = isAggStart(p);
        if (fn) {
          const at = p.i;
          p.i += 2;
          let f: AggFn = fn;
          if (p.eatKw('distinct')) f = 'COUNT_DISTINCT';
          const ref = p.eatOp('*') ? null : resolve(p.dotted());
          p.expectOp(')');
          const hit = q.select.find(
            (s) => s.kind === 'aggregate' && s.fn === f && (s.ref ? ref && refKey(s.ref) === refKey(ref) : !ref),
          );
          if (!hit) {
            p.i = at;
            throw new Error('ORDER BY an aggregate that is not in the SELECT list');
          }
          target = { kind: 'select', id: hit.id };
        } else {
          const parts = p.dotted();
          const byAlias = parts.length === 1 ? q.select.find((s) => s.as === parts[0]) : undefined;
          target = byAlias ? { kind: 'select', id: byAlias.id } : { kind: 'column', ref: resolve(parts) };
          // MySQL's NULLS FIRST/LAST emulation (as emitted by buildSelect):
          //   `x IS NULL DESC, x ASC` → x ASC NULLS FIRST.
          if (p.isKw('is') && p.isKw('null', 1)) {
            p.i += 2;
            const first = p.eatKw('desc');
            if (!first) p.eatKw('asc');
            p.expectOp(',');
            const again = p.dotted();
            if (again.join('.') !== parts.join('.')) throw new Error('unsupported ORDER BY … IS NULL');
            let dir2: 'ASC' | 'DESC' = 'ASC';
            if (p.eatKw('desc')) dir2 = 'DESC';
            else p.eatKw('asc');
            q.orderBy.push({ id: nid('o'), target, dir: dir2, nulls: first ? 'first' : 'last' });
            continue;
          }
        }
        let dir: 'ASC' | 'DESC' = 'ASC';
        if (p.eatKw('desc')) dir = 'DESC';
        else p.eatKw('asc');
        let nulls: OrderItem['nulls'] = 'default';
        if (p.eatKw('nulls')) {
          if (p.eatKw('first')) nulls = 'first';
          else {
            p.expectKw('last');
            nulls = 'last';
          }
        }
        q.orderBy.push({ id: nid('o'), target, dir, nulls });
      } while (p.eatOp(','));
    }
    if (p.eatKw('limit')) {
      const a = p.next();
      if (a.t !== 'num') throw new Error('expected a LIMIT number');
      if (p.eatOp(',')) {
        const b = p.next();
        if (b.t !== 'num') throw new Error('expected a LIMIT number');
        q.offset = Number(a.v);
        q.limit = Number(b.v);
      } else {
        q.limit = Number(a.v);
        if (p.eatKw('offset')) {
          const o = p.next();
          if (o.t !== 'num') throw new Error('expected an OFFSET number');
          q.offset = Number(o.v);
        }
      }
    } else if (p.eatKw('offset')) {
      const o = p.next();
      if (o.t !== 'num') throw new Error('expected an OFFSET number');
      q.offset = Number(o.v);
    }
    p.eatOp(';');
    if (p.peek().t !== 'eof') {
      const t = p.peek();
      throw new Error(`“${t.v}” is beyond what the builder can edit`);
    }
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  return { ok: true, query: q };
}

/** Map a LIKE pattern back onto contains / starts / ends when it is one. */
function likeToOp(pat: string, not: boolean): Pick<Condition, 'op' | 'value'> {
  const unesc = (s: string): string | null => {
    let out = '';
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      if (c === '\\') {
        out += s[i + 1] ?? '';
        i++;
      } else if (c === '%' || c === '_') return null; // a real wildcard inside
      else out += c;
    }
    return out;
  };
  if (pat.length >= 2 && pat.startsWith('%') && pat.endsWith('%') && !pat.endsWith('\\%')) {
    const inner = unesc(pat.slice(1, -1));
    if (inner !== null) return { op: not ? 'NOT_CONTAINS' : 'CONTAINS', value: inner };
  }
  if (!not && pat.endsWith('%') && !pat.endsWith('\\%')) {
    const inner = unesc(pat.slice(0, -1));
    if (inner !== null) return { op: 'STARTS_WITH', value: inner };
  }
  if (!not && pat.startsWith('%')) {
    const inner = unesc(pat.slice(1));
    if (inner !== null) return { op: 'ENDS_WITH', value: inner };
  }
  return { op: not ? 'NOT_LIKE' : 'LIKE', value: pat };
}

/** Merge same-conjunction children into their parent (a OR (b OR c) → a OR b OR c). */
function flatten(g: CondGroup): CondGroup {
  const items: (Condition | CondGroup)[] = [];
  for (const it of g.items) {
    if (it.kind === 'group' && it.conj === g.conj) items.push(...flatten(it).items);
    else items.push(it.kind === 'group' ? flatten(it) : it);
  }
  return { ...g, items };
}

/** The top-level WHERE/HAVING is always a group; a single AND chain becomes it. */
function asTopGroup(g: CondGroup): CondGroup {
  if (g.items.length === 1) {
    const only = g.items[0];
    if (only.kind === 'group') return only;
    return { kind: 'group', id: g.id, conj: 'AND', items: [only] };
  }
  return g;
}

function renderTokens(toks: Tok[], dialect: Dialect): string {
  let out = '';
  let prev: Tok | null = null;
  for (const t of toks) {
    const text =
      t.t === 'str' ? stringLiteral(dialect, t.v) : t.t === 'id' && t.quoted ? quoteIdent(dialect, t.v) : t.v;
    const tight =
      prev === null ||
      (t.t === 'op' && (t.v === ')' || t.v === ',' || t.v === '.' || t.v === '::')) ||
      (prev.t === 'op' && (prev.v === '(' || prev.v === '.' || prev.v === '::')) ||
      (t.t === 'op' && t.v === '(' && prev.t === 'id');
    out += (tight ? '' : ' ') + text;
    prev = t;
  }
  return out;
}

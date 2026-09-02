// Pure helpers for the visual JOIN canvas (QueryBuilder.svelte). No Svelte /
// DOM here — just the data model + SQL generation + graph walking so the
// component stays focused on layout and pointer interaction.

import type { ObjectDetail } from '../../lib/api/types';

/** Aggregate applied to a selected column; '' = plain (no aggregate). */
export type Aggregate =
  | ''
  | 'COUNT'
  | 'COUNT DISTINCT'
  | 'SUM'
  | 'AVG'
  | 'MIN'
  | 'MAX'
  | 'GROUP_CONCAT'
  | 'groupArray';

/**
 * A table dropped on the canvas. `cols` = columns chosen for the SELECT;
 * `aggs` maps a selected column → its aggregate (absent/'' = plain column).
 */
export interface CanvasTable {
  uid: string;
  db: string;
  table: string;
  alias: string;
  path: string;
  detail: ObjectDetail | null;
  cols: Set<string>;
  aggs: Map<string, Aggregate>;
  x: number;
  y: number;
}

/** A free-text computed/expression column appended to SELECT as `<expr> AS alias`. */
export interface ExprCol {
  id: string;
  expression: string;
  alias: string;
}

export type JoinType = 'INNER' | 'LEFT' | 'RIGHT' | 'FULL';

/** A directed join between two canvas tables, on one column each. */
export interface JoinEdge {
  id: string;
  fromUid: string;
  fromCol: string;
  toUid: string;
  toCol: string;
  type: JoinType;
}

/** A WHERE filter row; column is stored as `alias.col`. */
export interface WhereRow {
  ref: string; // "alias.col"
  op: string;
  value: string;
}

/** An ORDER BY row; column is stored as `alias.col`. */
export interface OrderRow {
  ref: string; // "alias.col"
  dir: 'ASC' | 'DESC';
}

export const JOIN_TYPES: JoinType[] = ['INNER', 'LEFT', 'RIGHT', 'FULL'];

export const OPS = ['=', '!=', '>', '<', '>=', '<=', 'LIKE', 'IN', 'IS NULL', 'IS NOT NULL'];

/**
 * Heuristic: does a free-text expression contain an aggregate call? Used so an
 * `AVG(id)`-style expression column also triggers GROUP BY of the plain columns
 * (otherwise the generated SQL mixes aggregates with bare columns and is wrong).
 */
const AGG_FN_RE =
  /\b(?:count|sum|avg|min|max|group_concat|grouparray|stddev\w*|var\w*|variance|median|quantile\w*|any|anylast|argmin|argmax|topk)\s*\(/i;
export function isAggregateExpr(expr: string): boolean {
  return AGG_FN_RE.test(expr);
}

/** Aggregate options for the per-column dropdown, varying by engine. */
export function aggregateOptions(engine?: string): Aggregate[] {
  const base: Aggregate[] = ['', 'COUNT', 'COUNT DISTINCT', 'SUM', 'AVG', 'MIN', 'MAX'];
  if (engine === 'clickhouse') return [...base, 'groupArray'];
  if (engine === 'mysql') return [...base, 'GROUP_CONCAT'];
  return base;
}

/** Auto SELECT alias for an aggregated column, e.g. SUM/amount → `sum_amount`. */
export function aggAlias(agg: Aggregate, col: string): string {
  const prefix = agg.toLowerCase().replace(/\s+/g, '_'); // "count_distinct"
  return `${prefix}_${col}`;
}

/** Render the aggregate call for a column ref, honoring COUNT DISTINCT. */
function aggCall(agg: Aggregate, colRef: string): string {
  if (agg === 'COUNT DISTINCT') return `COUNT(DISTINCT ${colRef})`;
  return `${agg}(${colRef})`;
}

let counter = 0;
/** Monotonic id (uids for tables, ids for edges). */
export function nextId(prefix: string): string {
  counter += 1;
  return `${prefix}_${counter}`;
}

/**
 * Derive a unique alias for `base`, suffixing `_2`, `_3`, … against aliases
 * already taken. Lets the same table be added repeatedly (self-joins).
 */
export function uniqueAlias(base: string, taken: Set<string>): string {
  const root = base.replace(/[^A-Za-z0-9_]/g, '_') || 't';
  if (!taken.has(root)) return root;
  let i = 2;
  while (taken.has(`${root}_${i}`)) i += 1;
  return `${root}_${i}`;
}

/** The active connection's engine, for identifier quoting + literal escaping. */
export type SqlEngine = string | null | undefined;

/**
 * Engine-correct identifier quoter: double quotes for postgres/sqlite (the
 * standard), backticks for mysql/clickhouse. Emitting backticks on Postgres
 * produced SQL the server rejects outright.
 */
function quoter(engine: SqlEngine): (ident: string) => string {
  if (engine === 'postgres' || engine === 'sqlite') {
    return (ident) => `"${ident.replace(/"/g, '""')}"`;
  }
  return (ident) => `\`${ident.replace(/`/g, '``')}\``;
}

/** `db`.`table` (db omitted when empty), used for FROM / JOIN targets. */
function qualified(t: CanvasTable, q: (s: string) => string): string {
  return t.db ? `${q(t.db)}.${q(t.table)}` : q(t.table);
}

/**
 * Render a filter value as a SQL literal. Numbers stay bare; everything else is
 * single-quoted with '' doubling, plus backslash doubling on mysql/clickhouse
 * (where `\` is an escape character inside string literals by default).
 */
function literal(raw: string, engine: SqlEngine): string {
  if (/^-?\d+(\.\d+)?$/.test(raw)) return raw;
  let s = raw;
  if (engine === 'mysql' || engine === 'clickhouse') s = s.replace(/\\/g, '\\\\');
  return `'${s.replace(/'/g, "''")}'`;
}

/**
 * BFS from the FROM table over the (undirected) edge graph. Returns the join
 * order — each reached table paired with the edge that connected it — plus the
 * set of unreachable table uids (which must be excluded from SQL to avoid a
 * silent cartesian product).
 */
export function walkJoins(
  tables: CanvasTable[],
  edges: JoinEdge[],
  fromUid: string,
): { order: { table: CanvasTable; edge: JoinEdge }[]; unreached: string[] } {
  const byUid = new Map(tables.map((t) => [t.uid, t]));
  const order: { table: CanvasTable; edge: JoinEdge }[] = [];
  const visited = new Set<string>([fromUid]);
  const queue: string[] = [fromUid];

  while (queue.length) {
    const cur = queue.shift() as string;
    for (const e of edges) {
      let nextUid: string | null = null;
      if (e.fromUid === cur && !visited.has(e.toUid)) nextUid = e.toUid;
      else if (e.toUid === cur && !visited.has(e.fromUid)) nextUid = e.fromUid;
      if (nextUid && byUid.has(nextUid)) {
        visited.add(nextUid);
        order.push({ table: byUid.get(nextUid) as CanvasTable, edge: e });
        queue.push(nextUid);
      }
    }
  }

  const unreached = tables.filter((t) => !visited.has(t.uid)).map((t) => t.uid);
  return { order, unreached };
}

/** Render one ON condition for an edge given the alias of each side. */
function onClause(
  edge: JoinEdge,
  fromAlias: string,
  toAlias: string,
  q: (s: string) => string,
): string {
  // The edge is directed (from → to); align column to alias accordingly.
  return `${q(fromAlias)}.${q(edge.fromCol)} = ${q(toAlias)}.${q(edge.toCol)}`;
}

/**
 * Build the full SELECT statement from the canvas. `fromUid` is the base table
 * (first added). Tables with no path to the base are skipped (see walkJoins).
 *
 * Aggregates: a column with an aggregate emits `AGG(`alias`.`col`) AS `agg_col``.
 * When ANY selected column is aggregated, a GROUP BY of the plain (non-aggregated)
 * selected columns is appended. Expression columns are inserted verbatim and are
 * never added to GROUP BY (the user owns their correctness).
 */
export function buildSql(
  tables: CanvasTable[],
  edges: JoinEdge[],
  wheres: WhereRow[],
  exprs: ExprCol[],
  orders: OrderRow[],
  limit: number,
  fromUid: string,
  engine?: SqlEngine,
): string {
  const q = quoter(engine);
  const ref = (alias: string, col: string): string => `${q(alias)}.${q(col)}`;
  const byUid = new Map(tables.map((t) => [t.uid, t]));
  const from = byUid.get(fromUid);
  if (!from) return '';

  const { order } = walkJoins(tables, edges, fromUid);
  const included = [from, ...order.map((o) => o.table)];

  // SELECT — split chosen columns into plain vs aggregated, then append
  // expression columns. Track plain refs for the GROUP BY clause. Auto aliases
  // are deduped with numeric suffixes (SUM(a.id) + SUM(b.id) would otherwise
  // both claim `sum_id` and the statement fails).
  const selectParts: string[] = [];
  const groupBy: string[] = [];
  const takenAliases = new Set<string>();
  const uniqueSelectAlias = (base: string): string => {
    let alias = base;
    let i = 2;
    while (takenAliases.has(alias)) alias = `${base}_${i++}`;
    takenAliases.add(alias);
    return alias;
  };
  let hasAgg = false;
  for (const t of included) {
    for (const c of t.cols) {
      const colRef = ref(t.alias, c);
      const agg = t.aggs.get(c) ?? '';
      if (agg) {
        hasAgg = true;
        selectParts.push(`${aggCall(agg, colRef)} AS ${q(uniqueSelectAlias(aggAlias(agg, c)))}`);
      } else {
        selectParts.push(colRef);
        groupBy.push(colRef);
      }
    }
  }
  // Expression columns (verbatim) — `<expr> AS `alias``; alias defaults applied
  // by the caller, but guard against an empty one here too.
  exprs.forEach((e, i) => {
    const expr = e.expression.trim();
    if (!expr) return;
    // An aggregate inside an expression column (e.g. AVG(id)) also puts the
    // query into aggregation mode, so the plain columns get a GROUP BY.
    if (isAggregateExpr(expr)) hasAgg = true;
    const alias = (e.alias.trim() || `expr_${i + 1}`).replace(/[^A-Za-z0-9_]/g, '_');
    selectParts.push(`${expr} AS ${q(uniqueSelectAlias(alias))}`);
  });

  const select = selectParts.length ? selectParts.join(', ') : '*';
  let sql = `SELECT ${select}\nFROM ${qualified(from, q)} AS ${q(from.alias)}`;

  for (const { table, edge } of order) {
    const fromSide = byUid.get(edge.fromUid);
    const toSide = byUid.get(edge.toUid);
    if (!fromSide || !toSide) continue;
    // The edge the user drew is directed, but the JOIN emits the newly-reached
    // table on the RIGHT. When the edge points backwards (drawn from the new
    // table into the already-joined side), LEFT/RIGHT must flip or the query
    // keeps the wrong side's rows. INNER/FULL are symmetric.
    let joinType: JoinType = edge.type;
    if (edge.fromUid === table.uid) {
      if (joinType === 'LEFT') joinType = 'RIGHT';
      else if (joinType === 'RIGHT') joinType = 'LEFT';
    }
    sql += `\n${joinType} JOIN ${qualified(table, q)} AS ${q(table.alias)} ON ${onClause(
      edge,
      fromSide.alias,
      toSide.alias,
      q,
    )}`;
  }

  // WHERE — only rows that reference an included table and have a value
  // (NULL ops need none). Numeric values stay bare; others are single-quoted.
  const includedAliases = new Set(included.map((t) => t.alias));
  const active = wheres.filter((w) => {
    const alias = w.ref.split('.')[0];
    if (!w.ref.includes('.') || !includedAliases.has(alias)) return false;
    if (w.op === 'IS NULL' || w.op === 'IS NOT NULL') return true;
    return w.value !== '';
  });
  if (active.length) {
    const clauses = active.map((w) => {
      const [alias, ...rest] = w.ref.split('.');
      const colRef = ref(alias, rest.join('.'));
      if (w.op === 'IS NULL' || w.op === 'IS NOT NULL') return `${colRef} ${w.op}`;
      if (w.op === 'IN') {
        // Comma-separated user input → one escaped literal per element (raw
        // pass-through was an injection hole and broke on quoted strings).
        const items = w.value
          .split(',')
          .map((s) => s.trim())
          .filter((s) => s.length > 0)
          .map((s) => literal(s, engine));
        return `${colRef} IN (${items.join(', ')})`;
      }
      return `${colRef} ${w.op} ${literal(w.value, engine)}`;
    });
    sql += `\nWHERE ${clauses.join('\n  AND ')}`;
  }

  // GROUP BY — only when something is aggregated; group the plain columns.
  if (hasAgg && groupBy.length) {
    sql += `\nGROUP BY ${groupBy.join(', ')}`;
  }

  // ORDER BY — rows that reference an included table.
  const activeOrders = orders.filter(
    (o) => o.ref.includes('.') && includedAliases.has(o.ref.split('.')[0]),
  );
  if (activeOrders.length) {
    const parts = activeOrders.map((o) => {
      const [alias, ...rest] = o.ref.split('.');
      return `${ref(alias, rest.join('.'))} ${o.dir}`;
    });
    sql += `\nORDER BY ${parts.join(', ')}`;
  }

  if (limit > 0) sql += `\nLIMIT ${limit}`;
  return `${sql};`;
}

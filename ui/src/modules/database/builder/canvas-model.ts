// Bridges the builder's JOIN canvas (cards + drawn edges) and its clause
// model (sql-builder.ts). Pure; type-only imports, so node:test can load it.

import type {
  BuilderQuery,
  ColRef,
  CondGroup,
  Condition,
  Dialect,
  JoinSpec,
  JoinType,
  OrderItem,
  SelectItem,
} from './sql-builder';

/** A table card on the canvas. `db` is the schema/database qualifier ('' =
 *  the connection's implicit one); `columns` come from its object detail. */
export interface CardTable {
  uid: string;
  db: string;
  table: string;
  alias: string;
  path: string;
  columns: { name: string; type: string }[] | null;
  pk: string[];
  fks: { columns: string[]; ref_table: string; ref_columns: string[] }[];
  x: number;
  y: number;
}

export interface CanvasEdge {
  id: string;
  fromUid: string;
  fromCol: string;
  toUid: string;
  toCol: string;
  type: JoinType;
}

/** Everything the clause panel edits — the model minus FROM/JOIN (the canvas). */
export interface Clauses {
  distinct: boolean;
  select: SelectItem[];
  where: CondGroup;
  groupBy: ColRef[];
  having: CondGroup;
  orderBy: OrderItem[];
  limit: number | null;
  offset: number | null;
}

/**
 * Walk the edge graph from the base card (the first one) and emit JOINs in
 * reach order. Every edge between a newly reached card and the cards already
 * in scope becomes one ANDed ON pair (a composite key is several edges). A
 * drawn edge is directed; when it points from the new card back into scope,
 * LEFT/RIGHT flip so the kept side stays the one the user meant.
 */
export function walkCanvas(
  tables: CardTable[],
  edges: CanvasEdge[],
): { joins: JoinSpec[]; unreached: CardTable[] } {
  const base = tables[0];
  if (!base) return { joins: [], unreached: [] };
  const byUid = new Map(tables.map((t) => [t.uid, t]));
  const inScope = new Set([base.uid]);
  const joins: JoinSpec[] = [];
  const queue = [base.uid];
  while (queue.length) {
    const cur = queue.shift() as string;
    for (const e of edges) {
      const other = e.fromUid === cur ? e.toUid : e.toUid === cur ? e.fromUid : null;
      if (!other || inScope.has(other) || !byUid.has(other)) continue;
      const t = byUid.get(other) as CardTable;
      inScope.add(other);
      queue.push(other);
      // All edges tying `t` to the tables already in scope.
      const ties = edges.filter(
        (x) =>
          (x.fromUid === t.uid && inScope.has(x.toUid) && x.toUid !== t.uid) ||
          (x.toUid === t.uid && inScope.has(x.fromUid) && x.fromUid !== t.uid),
      );
      const first = ties[0] ?? e;
      let type = first.type;
      if (first.fromUid === t.uid) type = type === 'LEFT' ? 'RIGHT' : type === 'RIGHT' ? 'LEFT' : type;
      joins.push({
        type,
        table: { alias: t.alias, schema: t.db || null, table: t.table },
        on: ties.map((x) => {
          const scoped = x.fromUid === t.uid ? { uid: x.toUid, col: x.toCol } : { uid: x.fromUid, col: x.fromCol };
          const mine = x.fromUid === t.uid ? x.fromCol : x.toCol;
          return {
            left: { alias: (byUid.get(scoped.uid) as CardTable).alias, column: scoped.col },
            right: { alias: t.alias, column: mine },
          };
        }),
      });
    }
  }
  return { joins, unreached: tables.filter((t) => !inScope.has(t.uid)) };
}

/** The full builder model for the canvas + clauses. Clauses that reference a
 *  card not reached by a join are left out of the SQL (they'd name a table
 *  that isn't in FROM). */
export function toQuery(dialect: Dialect, tables: CardTable[], edges: CanvasEdge[], c: Clauses): BuilderQuery {
  const base = tables[0];
  const { joins, unreached } = walkCanvas(tables, edges);
  const out = new Set(unreached.map((t) => t.alias));
  const ok = (r: ColRef | null | undefined): boolean => !r || !out.has(r.alias);
  const types: Record<string, string> = {};
  for (const t of tables) for (const col of t.columns ?? []) types[`${t.alias}.${col.name}`] = col.type;
  const keepGroup = (g: CondGroup): CondGroup => ({
    ...g,
    items: g.items
      .filter((it) => it.kind === 'group' || ok(it.target.ref))
      .map((it) => (it.kind === 'group' ? keepGroup(it) : it)),
  });
  return {
    dialect,
    distinct: c.distinct,
    from: base ? { alias: base.alias, schema: base.db || null, table: base.table } : null,
    joins,
    select: c.select.filter((s) => s.kind === 'expr' || ok(s.ref)),
    where: keepGroup(c.where),
    groupBy: c.groupBy.filter(ok),
    having: keepGroup(c.having),
    orderBy: c.orderBy.filter((o) => o.target.kind === 'select' || ok(o.target.ref)),
    limit: c.limit,
    offset: c.offset,
    types,
  };
}

function mapGroup(g: CondGroup, f: (r: ColRef) => ColRef | null): CondGroup {
  const items: (Condition | CondGroup)[] = [];
  for (const it of g.items) {
    if (it.kind === 'group') items.push(mapGroup(it, f));
    else {
      const ref = f(it.target.ref);
      if (ref) items.push({ ...it, target: { ...it.target, ref } });
    }
  }
  return { ...g, items };
}

/** Apply `f` to every column ref in the clauses; `null` drops the node. */
export function mapRefs(c: Clauses, f: (r: ColRef) => ColRef | null): Clauses {
  const select: SelectItem[] = [];
  for (const s of c.select) {
    if (s.kind === 'expr') select.push(s);
    else if (s.kind === 'aggregate' && !s.ref) select.push(s);
    else {
      const ref = f(s.ref as ColRef);
      if (ref) select.push({ ...s, ref } as SelectItem);
    }
  }
  const selIds = new Set(select.map((s) => s.id));
  return {
    ...c,
    select,
    where: mapGroup(c.where, f),
    groupBy: c.groupBy.map(f).filter((r): r is ColRef => !!r),
    having: mapGroup(c.having, f),
    orderBy: c.orderBy.flatMap((o): OrderItem[] => {
      if (o.target.kind === 'select') return selIds.has(o.target.id) ? [o] : [];
      const ref = f(o.target.ref);
      return ref ? [{ ...o, target: { kind: 'column', ref } }] : [];
    }),
  };
}

/** A card was renamed: every ref to its alias follows. */
export function renameAlias(c: Clauses, from: string, to: string): Clauses {
  return mapRefs(c, (r) => (r.alias === from ? { ...r, alias: to } : r));
}

/** A card was removed: drop everything that referenced it. */
export function dropAlias(c: Clauses, alias: string): Clauses {
  return mapRefs(c, (r) => (r.alias === alias ? null : r));
}

/** Derive a unique alias for `base` (`orders`, `orders_2`, …) — self-joins. */
export function uniqueAlias(base: string, taken: Set<string>): string {
  const root = base.replace(/[^A-Za-z0-9_]/g, '_') || 't';
  if (!taken.has(root)) return root;
  let i = 2;
  while (taken.has(`${root}_${i}`)) i += 1;
  return `${root}_${i}`;
}

/** FK-derived join suggestions between cards on the canvas that aren't
 *  joined yet — composite keys come back as one suggestion with every pair. */
export function fkSuggestions(
  tables: CardTable[],
  edges: CanvasEdge[],
): { fromUid: string; toUid: string; pairs: [string, string][]; label: string }[] {
  const out: { fromUid: string; toUid: string; pairs: [string, string][]; label: string }[] = [];
  const linked = (a: string, b: string): boolean =>
    edges.some((e) => (e.fromUid === a && e.toUid === b) || (e.fromUid === b && e.toUid === a));
  for (const t of tables) {
    for (const fk of t.fks) {
      const target = tables.find((x) => x.uid !== t.uid && x.table.toLowerCase() === fk.ref_table.toLowerCase());
      if (!target || linked(t.uid, target.uid)) continue;
      const pairs = fk.columns.map((c, i): [string, string] => [c, fk.ref_columns[i] ?? fk.ref_columns[0]]);
      if (!pairs.length) continue;
      out.push({
        fromUid: target.uid,
        toUid: t.uid,
        pairs: pairs.map(([a, b]) => [b, a]),
        label: `${t.alias}.${pairs.map((p) => p[0]).join('+')} → ${target.alias}.${pairs.map((p) => p[1]).join('+')}`,
      });
    }
  }
  return out;
}

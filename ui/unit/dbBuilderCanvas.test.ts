import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  dropAlias,
  fkSuggestions,
  renameAlias,
  toQuery,
  uniqueAlias,
  walkCanvas,
  type CanvasEdge,
  type CardTable,
  type Clauses,
} from '../src/modules/database/builder/canvas-model.ts';
import { buildSelect, emptyGroup } from '../src/modules/database/builder/sql-builder.ts';

// The builder canvas (cards + drawn edges) → FROM/JOIN, and clause upkeep when
// a card is renamed or removed.

function card(uid: string, table: string, cols: string[], extra: Partial<CardTable> = {}): CardTable {
  return {
    uid,
    db: 'shop',
    table,
    alias: table,
    path: `db:shop/table:${table}`,
    columns: cols.map((name) => ({ name, type: name === 'id' ? 'int' : 'varchar(20)' })),
    pk: ['id'],
    fks: [],
    x: 0,
    y: 0,
    ...extra,
  };
}
const edge = (id: string, f: string, fc: string, t: string, tc: string, type: CanvasEdge['type'] = 'INNER'): CanvasEdge => ({
  id,
  fromUid: f,
  fromCol: fc,
  toUid: t,
  toCol: tc,
  type,
});
function clauses(): Clauses {
  return { distinct: false, select: [], where: emptyGroup(), groupBy: [], having: emptyGroup(), orderBy: [], limit: 10, offset: null };
}

test('walkCanvas: reach order, composite keys ANDed, unconnected cards reported', () => {
  const tables = [
    card('a', 'orders', ['id', 'customer_id', 'region']),
    card('b', 'customers', ['id', 'region']),
    card('c', 'audit', ['id']),
  ];
  const edges = [edge('e1', 'a', 'customer_id', 'b', 'id', 'LEFT'), edge('e2', 'a', 'region', 'b', 'region', 'LEFT')];
  const { joins, unreached } = walkCanvas(tables, edges);
  assert.equal(joins.length, 1);
  assert.equal(joins[0].type, 'LEFT');
  assert.deepEqual(joins[0].on, [
    { left: { alias: 'orders', column: 'customer_id' }, right: { alias: 'customers', column: 'id' } },
    { left: { alias: 'orders', column: 'region' }, right: { alias: 'customers', column: 'region' } },
  ]);
  assert.deepEqual(unreached.map((t) => t.uid), ['c']);
});

test('walkCanvas: an edge drawn back into scope flips LEFT/RIGHT', () => {
  const tables = [card('a', 'orders', ['id', 'customer_id']), card('b', 'customers', ['id'])];
  // Drawn FROM customers TO orders as LEFT (keep customers) → orders is FROM,
  // so the JOIN of customers must be RIGHT to keep the customers side.
  const { joins } = walkCanvas(tables, [edge('e', 'b', 'id', 'a', 'customer_id', 'LEFT')]);
  assert.equal(joins[0].type, 'RIGHT');
  assert.deepEqual(joins[0].on[0], { left: { alias: 'orders', column: 'customer_id' }, right: { alias: 'customers', column: 'id' } });
});

test('toQuery leaves clauses on unjoined cards out of the SQL', () => {
  const tables = [card('a', 'orders', ['id', 'status']), card('b', 'loose', ['id'])];
  const c = clauses();
  c.select = [
    { kind: 'column', id: 's1', ref: { alias: 'orders', column: 'status' } },
    { kind: 'column', id: 's2', ref: { alias: 'loose', column: 'id' } },
  ];
  const sql = buildSelect(toQuery('mysql', tables, [], c));
  assert.equal(sql, 'SELECT `status`\nFROM `shop`.`orders`\nLIMIT 10;');
});

test('renameAlias / dropAlias keep every clause consistent', () => {
  const c = clauses();
  c.select = [
    { kind: 'column', id: 's1', ref: { alias: 'o', column: 'id' } },
    { kind: 'aggregate', id: 's2', fn: 'COUNT', ref: null },
    { kind: 'column', id: 's3', ref: { alias: 'x', column: 'id' } },
  ];
  c.where.items.push({ kind: 'cond', id: 'c1', target: { ref: { alias: 'o', column: 'id' } }, op: '=', value: '1' });
  c.groupBy = [{ alias: 'o', column: 'id' }];
  c.orderBy = [
    { id: 'o1', target: { kind: 'column', ref: { alias: 'o', column: 'id' } }, dir: 'ASC', nulls: 'default' },
    { id: 'o2', target: { kind: 'select', id: 's3' }, dir: 'ASC', nulls: 'default' },
  ];
  const r = renameAlias(c, 'o', 'ord');
  assert.equal((r.select[0] as { ref: { alias: string } }).ref.alias, 'ord');
  assert.equal(r.groupBy[0].alias, 'ord');
  assert.equal((r.where.items[0] as { target: { ref: { alias: string } } }).target.ref.alias, 'ord');
  const d = dropAlias(r, 'x');
  assert.deepEqual(d.select.map((s) => s.id), ['s1', 's2']);
  // The ORDER BY on the dropped output column goes with it.
  assert.deepEqual(d.orderBy.map((o) => o.id), ['o1']);
});

test('FK suggestions: one per unjoined pair, composite keys kept together', () => {
  const orders = card('a', 'orders', ['id', 'cust_id', 'cust_region'], {
    fks: [{ columns: ['cust_id', 'cust_region'], ref_table: 'customers', ref_columns: ['id', 'region'] }],
  });
  const customers = card('b', 'customers', ['id', 'region']);
  const s = fkSuggestions([orders, customers], []);
  assert.equal(s.length, 1);
  assert.deepEqual(s[0].pairs, [
    ['id', 'cust_id'],
    ['region', 'cust_region'],
  ]);
  assert.equal(fkSuggestions([orders, customers], [edge('e', 'a', 'cust_id', 'b', 'id')]).length, 0);
});

test('uniqueAlias sanitizes and suffixes for self-joins', () => {
  assert.equal(uniqueAlias('orders', new Set()), 'orders');
  assert.equal(uniqueAlias('orders', new Set(['orders'])), 'orders_2');
  assert.equal(uniqueAlias('my-table', new Set()), 'my_table');
});

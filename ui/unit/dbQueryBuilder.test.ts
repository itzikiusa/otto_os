import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildSelect,
  emptyGroup,
  emptyQuery,
  escapeLike,
  literal,
  opsFor,
  parseSelect,
  quoteIdent,
  splitList,
  stringLiteral,
  suggestGroupBy,
  validate,
  valueKind,
  type BuilderQuery,
  type Condition,
  type Dialect,
} from '../src/modules/database/builder/sql-builder.ts';

// The visual query builder's SQL layer: dialect quoting, value escaping
// (injection-safety), clause generation and the parse-back round trip.

const DIALECTS: Dialect[] = ['mysql', 'postgres', 'clickhouse'];

function ordersQuery(d: Dialect): BuilderQuery {
  const q = emptyQuery(d);
  q.from = { alias: 'orders', schema: 'shop', table: 'orders' };
  q.types = {
    'orders.id': 'bigint',
    'orders.status': 'varchar(20)',
    'orders.total': 'decimal(10,2)',
    'orders.region': 'varchar(32)',
    'orders.is_gift': 'tinyint(1)',
    'orders.created_at': 'datetime',
    'orders.customer_id': 'int',
  };
  return q;
}
const col = (column: string, alias = 'orders') => ({ alias, column });
let n = 0;
const cond = (c: Omit<Condition, 'kind' | 'id'>): Condition => ({ kind: 'cond', id: `t${++n}`, ...c });

// ── Quoting & escaping ──────────────────────────────────────────────────────

test('identifiers: backticks for MySQL/ClickHouse, double quotes for Postgres, quote chars escaped', () => {
  assert.equal(quoteIdent('mysql', 'order'), '`order`');
  assert.equal(quoteIdent('mysql', 'we`ird'), '`we``ird`');
  assert.equal(quoteIdent('postgres', 'Order'), '"Order"');
  assert.equal(quoteIdent('postgres', 'we"ird'), '"we""ird"');
  assert.equal(quoteIdent('clickhouse', 'we`ird'), '`we\\`ird`');
  assert.equal(quoteIdent('clickhouse', 'back\\slash'), '`back\\\\slash`');
});

test('string literals: quotes and backslashes escaped per dialect', () => {
  assert.equal(stringLiteral('mysql', "O'Brien"), "'O''Brien'");
  assert.equal(stringLiteral('mysql', 'a\\b'), "'a\\\\b'");
  assert.equal(stringLiteral('postgres', "O'Brien"), "'O''Brien'");
  assert.equal(stringLiteral('postgres', 'a\\b'), "'a\\b'"); // standard_conforming_strings
  assert.equal(stringLiteral('clickhouse', "O'Brien"), "'O\\'Brien'");
  assert.equal(stringLiteral('clickhouse', 'a\\b'), "'a\\\\b'");
  assert.equal(stringLiteral('mysql', 'nul\0byte'), "'nul\\0byte'");
  assert.equal(stringLiteral('postgres', 'nul\0byte'), "'nulbyte'");
});

test('injection attempts stay inside one literal on every dialect', () => {
  const evil = "x'); DROP TABLE orders; --";
  for (const d of DIALECTS) {
    const q = ordersQuery(d);
    q.where.items.push(cond({ target: { ref: col('status') }, op: '=', value: evil }));
    const sql = buildSelect(q);
    // Re-tokenising the generated SQL must give back exactly the typed value.
    const back = parseSelect(sql, d);
    assert.ok(back.ok, `${d}: ${sql}`);
    if (!back.ok) return;
    const c = back.query.where.items[0] as Condition;
    assert.equal(c.value, evil, d);
    // …and it is ONE predicate: nothing leaked out of the literal into the
    // statement (a leak would add clauses or fail to parse at all).
    assert.equal(back.query.where.items.length, 1, d);
    assert.equal(back.query.limit, 100, `${d}: the LIMIT still ends the statement`);
  }
  // Backslash-quote tricks for the backslash-escaping dialects.
  for (const d of ['mysql', 'clickhouse'] as const) {
    const q = ordersQuery(d);
    q.where.items.push(cond({ target: { ref: col('status') }, op: '=', value: "\\' OR 1=1 -- " }));
    const back = parseSelect(buildSelect(q), d);
    assert.ok(back.ok);
    if (back.ok) assert.equal((back.query.where.items[0] as Condition).value, "\\' OR 1=1 -- ");
  }
});

test('values: numbers bare only when numeric; text columns always quoted', () => {
  assert.equal(literal('mysql', '42', 'number'), '42');
  assert.equal(literal('mysql', '-3.5e2', 'number'), '-3.5e2');
  assert.equal(literal('mysql', '42; DROP TABLE t', 'number'), "'42; DROP TABLE t'");
  assert.equal(literal('mysql', '0x1F', 'number'), "'0x1F'");
  assert.equal(literal('mysql', '123', 'string'), "'123'");
  assert.equal(literal('postgres', 'true', 'bool'), 'TRUE');
  assert.equal(literal('clickhouse', '0', 'bool'), 'false');
});

test('LIKE helpers escape wildcards so user text matches literally', () => {
  assert.equal(escapeLike('50%_off\\'), '50\\%\\_off\\\\');
  const q = ordersQuery('mysql');
  q.where.items.push(cond({ target: { ref: col('status') }, op: 'CONTAINS', value: '50%' }));
  assert.match(buildSelect(q), /WHERE `status` LIKE '%50\\\\%%'/);
  const pg = ordersQuery('postgres');
  pg.where.items.push(cond({ target: { ref: col('status') }, op: 'STARTS_WITH', value: 'a_b' }));
  assert.match(buildSelect(pg), /WHERE "status" LIKE 'a\\_b%'/);
});

test('IN lists: quoted items keep commas; every item escaped', () => {
  assert.deepEqual(splitList("paid, shipped, 'a, b', \"x\"\"y\""), ['paid', 'shipped', 'a, b', 'x"y']);
  const q = ordersQuery('mysql');
  q.where.items.push(cond({ target: { ref: col('status') }, op: 'IN', values: ['paid', "it's"] }));
  q.where.items.push(cond({ target: { ref: col('customer_id') }, op: 'NOT_IN', values: ['1', '2', 'x'] }));
  const sql = buildSelect(q);
  assert.match(sql, /`status` IN \('paid', 'it''s'\)/);
  assert.match(sql, /`customer_id` NOT IN \(1, 2, 'x'\)/);
});

test('operators follow the column type', () => {
  assert.equal(valueKind('BIGINT UNSIGNED'), 'number');
  assert.equal(valueKind('Nullable(UInt64)'), 'number');
  assert.equal(valueKind('tinyint(1)'), 'bool');
  assert.equal(valueKind('timestamptz'), 'time');
  assert.equal(valueKind('varchar(20)'), 'string');
  assert.ok(opsFor('number').includes('BETWEEN'));
  assert.ok(!opsFor('number').includes('CONTAINS'));
  assert.ok(opsFor('string').includes('CONTAINS'));
  assert.equal(opsFor('bool')[0], 'IS_TRUE');
});

// ── Clauses ─────────────────────────────────────────────────────────────────

function reportQuery(d: Dialect): BuilderQuery {
  const q = ordersQuery(d);
  q.select = [
    { kind: 'column', id: 's1', ref: col('region') },
    { kind: 'aggregate', id: 's2', fn: 'COUNT', ref: null, as: 'orders' },
    { kind: 'aggregate', id: 's3', fn: 'SUM', ref: col('total'), as: 'revenue' },
    { kind: 'aggregate', id: 's4', fn: 'COUNT_DISTINCT', ref: col('customer_id'), as: 'buyers' },
  ];
  const g = emptyGroup('OR');
  g.items.push(cond({ target: { ref: col('status') }, op: '=', value: 'paid' }));
  g.items.push(cond({ target: { ref: col('status') }, op: '=', value: 'shipped' }));
  q.where.items.push(g);
  q.where.items.push(cond({ target: { ref: col('created_at') }, op: '>=', value: '2026-01-01' }));
  q.groupBy = [col('region')];
  q.having.items.push(cond({ target: { ref: col('total'), fn: 'SUM' }, op: '>', value: '1000' }));
  q.orderBy = [{ id: 'o1', target: { kind: 'select', id: 's3' }, dir: 'DESC', nulls: 'last' }];
  q.limit = 20;
  q.offset = 40;
  return q;
}

test('GROUP BY / aggregates / HAVING / ORDER BY / LIMIT — MySQL', () => {
  assert.equal(
    buildSelect(reportQuery('mysql')),
    [
      'SELECT ',
      '  `region`,',
      '  COUNT(*) AS `orders`,',
      '  SUM(`total`) AS `revenue`,',
      '  COUNT(DISTINCT `customer_id`) AS `buyers`',
      'FROM `shop`.`orders`',
      "WHERE (`status` = 'paid' OR `status` = 'shipped')",
      "  AND `created_at` >= '2026-01-01'",
      'GROUP BY `region`',
      'HAVING SUM(`total`) > 1000',
      'ORDER BY `revenue` IS NULL ASC, `revenue` DESC',
      'LIMIT 20 OFFSET 40;',
    ].join('\n'),
  );
});

test('same model — PostgreSQL (double quotes, native NULLS LAST)', () => {
  const sql = buildSelect(reportQuery('postgres'));
  assert.match(sql, /FROM "shop"\."orders"/);
  assert.match(sql, /COUNT\(DISTINCT "customer_id"\) AS "buyers"/);
  assert.match(sql, /ORDER BY "revenue" DESC NULLS LAST/);
  assert.match(sql, /HAVING SUM\("total"\) > 1000/);
});

test('same model — ClickHouse (uniqExact, native NULLS LAST)', () => {
  const sql = buildSelect(reportQuery('clickhouse'));
  assert.match(sql, /uniqExact\(`customer_id`\) AS `buyers`/);
  assert.match(sql, /ORDER BY `revenue` DESC NULLS LAST/);
});

test('joins qualify every column; multi-column ON; aliases only when they differ', () => {
  const q = ordersQuery('postgres');
  q.from = { alias: 'o', schema: 'public', table: 'orders' };
  q.joins = [
    {
      type: 'LEFT',
      table: { alias: 'customers', schema: 'public', table: 'customers' },
      on: [
        { left: col('customer_id', 'o'), right: col('id', 'customers') },
        { left: col('region', 'o'), right: col('region', 'customers') },
      ],
    },
  ];
  q.select = [
    { kind: 'column', id: 'a', ref: col('id', 'o') },
    { kind: 'column', id: 'b', ref: col('email', 'customers'), as: 'customer' },
  ];
  q.limit = null;
  assert.equal(
    buildSelect(q),
    [
      'SELECT "o"."id", "customers"."email" AS "customer"',
      'FROM "public"."orders" AS "o"',
      'LEFT JOIN "public"."customers" ON "o"."customer_id" = "customers"."id" AND "o"."region" = "customers"."region";',
    ].join('\n'),
  );
});

test('incomplete rows are skipped, never emitted half-typed', () => {
  const q = ordersQuery('mysql');
  q.where.items.push(cond({ target: { ref: col('status') }, op: '=', value: '' }));
  q.where.items.push(cond({ target: { ref: col('total') }, op: 'BETWEEN', value: '1', value2: '' }));
  q.where.items.push(cond({ target: { ref: col('shipped_at') }, op: 'IS_NULL' }));
  q.where.items.push(emptyGroup('OR'));
  assert.equal(buildSelect(q), 'SELECT *\nFROM `shop`.`orders`\nWHERE `shipped_at` IS NULL\nLIMIT 100;');
});

test('booleans: IS TRUE on MySQL/Postgres, = true on ClickHouse', () => {
  for (const [d, re] of [
    ['mysql', /`is_gift` IS TRUE/],
    ['postgres', /"is_gift" IS TRUE/],
    ['clickhouse', /`is_gift` = true/],
  ] as const) {
    const q = ordersQuery(d);
    q.where.items.push(cond({ target: { ref: col('is_gift') }, op: 'IS_TRUE' }));
    assert.match(buildSelect(q), re, d);
  }
});

test('OFFSET without LIMIT: MySQL gets the max-limit idiom', () => {
  const q = ordersQuery('mysql');
  q.limit = null;
  q.offset = 10;
  assert.match(buildSelect(q), /LIMIT 18446744073709551615 OFFSET 10;$/);
  const pg = ordersQuery('postgres');
  pg.limit = null;
  pg.offset = 10;
  assert.match(buildSelect(pg), /\nOFFSET 10;$/);
});

// ── Validation ──────────────────────────────────────────────────────────────

test('validate: loose columns under GROUP BY, with a one-click fix', () => {
  const q = ordersQuery('mysql');
  q.select = [
    { kind: 'column', id: 'a', ref: col('region') },
    { kind: 'column', id: 'b', ref: col('status') },
    { kind: 'aggregate', id: 'c', fn: 'COUNT', ref: null },
  ];
  q.groupBy = [col('region')];
  const issues = validate(q);
  const err = issues.find((i) => i.level === 'error');
  assert.ok(err);
  assert.match(err!.message, /status is selected but neither grouped nor aggregated/);
  assert.deepEqual(err!.fix?.refs, [col('status')]);
  assert.deepEqual(suggestGroupBy(q), [col('status')]);
});

test('validate: duplicate output names, SUM of text, OFFSET without ORDER BY', () => {
  const q = ordersQuery('mysql');
  q.select = [
    { kind: 'aggregate', id: 'a', fn: 'SUM', ref: col('status'), as: 'x' },
    { kind: 'aggregate', id: 'b', fn: 'COUNT', ref: null, as: 'x' },
  ];
  q.offset = 5;
  const msgs = validate(q).map((i) => i.message).join('\n');
  assert.match(msgs, /“x” is used 2 times/);
  assert.match(msgs, /SUM of status/);
  assert.match(msgs, /OFFSET without ORDER BY/);
});

// ── Round trip ──────────────────────────────────────────────────────────────

test('round trip: build → parse → build is stable on every dialect', () => {
  for (const d of DIALECTS) {
    for (const make of [reportQuery, ordersQuery]) {
      const q = make(d);
      const sql = buildSelect(q);
      const back = parseSelect(sql, d);
      assert.ok(back.ok, `${d}: ${back.ok ? '' : back.error}\n${sql}`);
      if (!back.ok) continue;
      back.query.types = q.types;
      assert.equal(buildSelect(back.query), sql, d);
    }
  }
});

test('round trip: contains / starts-with / IN / BETWEEN / NULL come back as builder operators', () => {
  const q = ordersQuery('mysql');
  q.where.items.push(cond({ target: { ref: col('status') }, op: 'CONTAINS', value: '50%' }));
  q.where.items.push(cond({ target: { ref: col('region') }, op: 'STARTS_WITH', value: 'eu' }));
  q.where.items.push(cond({ target: { ref: col('total') }, op: 'BETWEEN', value: '10', value2: '20' }));
  q.where.items.push(cond({ target: { ref: col('customer_id') }, op: 'IN', values: ['1', '2'] }));
  q.where.items.push(cond({ target: { ref: col('notes') }, op: 'IS_NOT_NULL' }));
  const back = parseSelect(buildSelect(q), 'mysql');
  assert.ok(back.ok);
  if (!back.ok) return;
  const ops = back.query.where.items.map((c) => (c as Condition).op);
  assert.deepEqual(ops, ['CONTAINS', 'STARTS_WITH', 'BETWEEN', 'IN', 'IS_NOT_NULL']);
  assert.equal((back.query.where.items[0] as Condition).value, '50%');
});

test('parse: hand-written SQL the builder can hold', () => {
  const r = parseSelect(
    'select o.region, count(*) as n from shop.orders o join shop.customers c on c.id = o.customer_id ' +
      "where o.status <> 'x' and (o.total > 5 or o.total < 1) group by o.region having count(*) >= 2 " +
      'order by n desc limit 5, 10',
    'mysql',
  );
  assert.ok(r.ok, r.ok ? '' : r.error);
  if (!r.ok) return;
  assert.equal(r.query.from?.alias, 'o');
  assert.equal(r.query.joins[0].table.alias, 'c');
  assert.deepEqual(r.query.joins[0].on[0], { left: col('customer_id', 'o'), right: col('id', 'c') });
  assert.equal(r.query.where.conj, 'AND');
  assert.equal(r.query.where.items[1].kind, 'group');
  assert.equal(r.query.groupBy.length, 1);
  assert.equal(r.query.having.items.length, 1);
  assert.equal(r.query.orderBy[0].target.kind, 'select');
  assert.equal(r.query.limit, 10);
  assert.equal(r.query.offset, 5);
});

test('parse: what the builder cannot hold is refused with a reason', () => {
  for (const sql of [
    'SELECT * FROM a UNION SELECT * FROM b',
    'SELECT * FROM (SELECT 1) t',
    'SELECT * FROM t WHERE lower(name) = 1',
    'WITH x AS (SELECT 1) SELECT * FROM x',
    'SELECT id FROM a JOIN b ON a.id = b.id WHERE id = 1',
  ]) {
    const r = parseSelect(sql, 'mysql');
    assert.equal(r.ok, false, sql);
  }
  // Expressions in the SELECT list are kept verbatim, not refused.
  const r = parseSelect("SELECT CONCAT(first, ' ', last) AS full_name, id FROM people", 'mysql');
  assert.ok(r.ok);
  if (r.ok) {
    assert.equal(r.query.select[0].kind, 'expr');
    assert.equal(r.query.select[0].as, 'full_name');
    assert.match(buildSelect(r.query), /CONCAT\(first, ' ', last\) AS `full_name`, `id`/);
  }
});

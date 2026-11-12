import { test } from 'node:test';
import assert from 'node:assert/strict';
import { serverSortSql, withServerOrder } from '../src/modules/database/server-sort.ts';

// The grid sorts only the loaded rows; on a capped / paged result "Sort on
// server" re-runs the statement with an ORDER BY — only for a single plain
// SELECT the "Query by value" splicer can parse.
test('adds, replaces or places ORDER BY before LIMIT', () => {
  assert.equal(withServerOrder('SELECT * FROM orders', 'id', 'desc', 'mysql'), 'SELECT * FROM orders\nORDER BY `id` DESC');
  assert.equal(
    withServerOrder('SELECT * FROM orders WHERE status = 1 LIMIT 50;', 'total', 'asc', 'mysql'),
    'SELECT * FROM orders WHERE status = 1\nORDER BY `total` ASC\nLIMIT 50',
  );
  assert.equal(
    withServerOrder('SELECT * FROM orders ORDER BY id LIMIT 10 OFFSET 20', 'created_at', 'desc', 'postgres'),
    'SELECT * FROM orders\nORDER BY "created_at" DESC\nLIMIT 10 OFFSET 20',
  );
  assert.equal(
    withServerOrder('SELECT user_id, count() AS n FROM events GROUP BY user_id', 'n', 'desc', 'clickhouse'),
    'SELECT user_id, count() AS n FROM events GROUP BY user_id\nORDER BY `n` DESC',
  );
  // A subquery's ORDER BY / LIMIT is not the outer one.
  assert.equal(
    withServerOrder('SELECT * FROM t WHERE id IN (SELECT id FROM u ORDER BY id LIMIT 5)', 'id', 'asc', 'mysql'),
    'SELECT * FROM t WHERE id IN (SELECT id FROM u ORDER BY id LIMIT 5)\nORDER BY `id` ASC',
  );
});

test('refuses what it cannot rewrite safely', () => {
  for (const sql of [
    'SELECT a FROM t UNION SELECT a FROM u',
    'SELECT 1; SELECT 2',
    'WITH x AS (SELECT 1) SELECT * FROM x',
    'UPDATE t SET a = 1',
    'SELECT 1',
    'SELECT * FROM t ORDER  BY id',
  ]) assert.equal(withServerOrder(sql, 'a', 'asc', 'mysql'), null, sql);
  assert.equal(withServerOrder('SELECT * FROM t', 'count(*)', 'asc', 'mysql'), null, 'an expression column');
  assert.equal(withServerOrder('db.t.find({})', 'a', 'asc', 'mongodb'), null);
});

test('only when the editor still holds the statement on screen and the column is unique', () => {
  const cols = [{ name: 'id' }, { name: 'name' }];
  assert.equal(serverSortSql('SELECT * FROM t;', 'SELECT * FROM t', cols, 1, 'asc', 'mysql'), 'SELECT * FROM t\nORDER BY `name` ASC');
  assert.equal(serverSortSql('SELECT * FROM t; SELECT 2', 'SELECT * FROM t', cols, 1, 'asc', 'mysql'), null);
  assert.equal(serverSortSql('SELECT * FROM t', 'SELECT * FROM t', [{ name: 'id' }, { name: 'id' }], 0, 'asc', 'mysql'), null);
  assert.equal(serverSortSql('SELECT * FROM t', null, cols, 0, 'asc', 'mysql'), null);
});

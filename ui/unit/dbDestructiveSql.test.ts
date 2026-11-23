import { test } from 'node:test';
import assert from 'node:assert/strict';
import { destructiveMessage, destructiveStatements } from '../src/modules/database/destructive-sql.ts';

// The query editor asks before running these on a connection the daemon does
// not guard (not Prod / read-only). Strings and comments never trigger it, and
// any WHERE — `WHERE 1=1` included — counts as a WHERE.
const kinds = (sql: string, engine = 'mysql') => destructiveStatements(sql, engine as never).map((r) => r.kind);

test('DELETE / UPDATE without a WHERE are destructive; with any WHERE they are not', () => {
  assert.deepEqual(kinds('DELETE FROM orders'), ['delete-all']);
  assert.deepEqual(kinds('delete from `orders`;'), ['delete-all']);
  assert.deepEqual(kinds('UPDATE users SET active = 0'), ['update-all']);
  assert.deepEqual(kinds('DELETE FROM orders WHERE id = 7'), []);
  assert.deepEqual(kinds('UPDATE users SET active = 0 WHERE 1=1'), [], 'WHERE 1=1 still has a WHERE');
  assert.deepEqual(kinds('UPDATE users\nSET active = 0\nwhere id IN (1, 2)'), []);
});

test('DROP, TRUNCATE and ALTER TABLE … DROP are destructive', () => {
  assert.deepEqual(kinds('DROP TABLE tmp'), ['drop']);
  assert.deepEqual(kinds('drop database shop'), ['drop']);
  assert.deepEqual(kinds('TRUNCATE TABLE logs'), ['truncate']);
  assert.deepEqual(kinds('ALTER TABLE users DROP COLUMN email'), ['alter-drop']);
  assert.deepEqual(kinds('ALTER TABLE events DROP PARTITION 202401', 'clickhouse'), ['alter-drop']);
  assert.deepEqual(kinds('ALTER TABLE users ADD COLUMN email TEXT', 'postgres'), []);
  assert.deepEqual(kinds('SELECT * FROM orders; INSERT INTO t VALUES (1); CREATE TABLE x (id INT)'), []);
});

test('string literals and comments never trigger it', () => {
  assert.deepEqual(kinds("SELECT 'DROP TABLE users' AS note"), []);
  assert.deepEqual(kinds('-- DROP TABLE users\nSELECT 1'), []);
  assert.deepEqual(kinds('/* TRUNCATE logs */ SELECT 1'), []);
  assert.deepEqual(kinds('# DELETE FROM t\nSELECT 1'), []);
  // A WHERE inside a string is not a WHERE.
  assert.deepEqual(kinds("UPDATE notes SET body = 'see where it says'"), ['update-all']);
  // A ; inside a string does not split the statement.
  assert.deepEqual(kinds("UPDATE t SET v = 'a; DROP TABLE x' WHERE id = 1"), []);
  // Postgres: dollar quotes and nested comments are not code.
  assert.deepEqual(kinds('SELECT $$DROP TABLE x$$', 'postgres'), []);
  assert.deepEqual(kinds('/* a /* DROP TABLE x */ b */ SELECT 1', 'postgres'), []);
  // A leading comment does not hide the verb.
  assert.deepEqual(kinds('-- clean up\nDELETE FROM sessions'), ['delete-all']);
});

test('a batch lists every risky statement in order, with its first line', () => {
  const risky = destructiveStatements('SELECT 1;\n-- why\nDELETE FROM a\n  -- trailing\n;\nDROP TABLE b;\nUPDATE c SET x = 1 WHERE id = 2;', 'mysql');
  assert.deepEqual(risky.map((r) => r.preview), ['DELETE FROM a', 'DROP TABLE b']);
});

test('CTE-prefixed DELETE / UPDATE and data-modifying CTEs are classified', () => {
  // CTE has a WHERE, but the main DELETE does not -> destructive!
  assert.deepEqual(
    kinds('WITH stale AS (SELECT id FROM sessions WHERE expired = 1) DELETE FROM orders', 'postgres'),
    ['delete-all'],
  );
  // Multi-CTE + RECURSIVE + column list + MATERIALIZED
  assert.deepEqual(
    kinds(
      'WITH RECURSIVE a(x) AS MATERIALIZED (SELECT 1), b AS NOT MATERIALIZED (SELECT * FROM a WHERE x = 1) UPDATE users SET active = 0',
      'postgres',
    ),
    ['update-all'],
  );
  // CTE with WHERE on the main statement -> safe
  assert.deepEqual(
    kinds('WITH stale AS (SELECT id FROM sessions) DELETE FROM orders WHERE id IN (SELECT id FROM stale)', 'postgres'),
    [],
  );
  // Postgres data-modifying CTE without WHERE -> destructive
  assert.deepEqual(
    kinds('WITH purged AS (DELETE FROM orders RETURNING id) SELECT * FROM purged', 'postgres'),
    ['delete-all'],
  );
  // Postgres data-modifying CTE with WHERE -> safe
  assert.deepEqual(
    kinds('WITH purged AS (DELETE FROM orders WHERE status = 0 RETURNING id) SELECT * FROM purged', 'postgres'),
    [],
  );
  // Plain CTE SELECT is safe
  assert.deepEqual(kinds('WITH cte AS (SELECT 1) SELECT * FROM cte', 'mysql'), []);
});

test('MongoDB and Redis wholesale removals are classified; unknown engines are ignored', () => {
  assert.deepEqual(kinds('db.users.deleteMany({})', 'mongodb'), ['delete-all']);
  assert.deepEqual(kinds('db.users.remove({})', 'mongodb'), ['delete-all']);
  assert.deepEqual(kinds('db.users.updateMany({}, { $set: { active: false } })', 'mongodb'), ['update-all']);
  assert.deepEqual(kinds('db.users.drop()', 'mongodb'), ['drop']);
  assert.deepEqual(kinds('db.dropDatabase()', 'mongodb'), ['drop']);
  // Filtered Mongo writes and comments/strings are safe
  assert.deepEqual(kinds('db.users.deleteMany({ active: false })', 'mongodb'), []);
  assert.deepEqual(kinds('db.users.updateMany({ id: 1 }, { $set: { a: 2 } })', 'mongodb'), []);
  assert.deepEqual(kinds('// db.users.drop()\ndb.users.find({})', 'mongodb'), []);
  // Redis FLUSHALL / FLUSHDB are destructive; normal commands are not
  assert.deepEqual(kinds('FLUSHALL', 'redis'), ['flush']);
  assert.deepEqual(kinds('flushdb async', 'redis'), ['flush']);
  assert.deepEqual(kinds('GET mykey\nDEL mykey', 'redis'), []);
  // Unknown / empty engine is ignored
  assert.deepEqual(kinds('DROP TABLE x', ''), []);
});

test('the confirm names the connection and caps the list', () => {
  const one = destructiveMessage([{ kind: 'drop', preview: 'DROP TABLE b' }], 'shop-staging');
  assert.match(one, /“shop-staging”/);
  assert.match(one, /• DROP TABLE b {2}\(drops the object\)/);
  const many = destructiveMessage(Array.from({ length: 7 }, (_, i) => ({ kind: 'truncate' as const, preview: `TRUNCATE t${i}` })), 'db');
  assert.match(many, /^7 statements remove data/);
  assert.match(many, /…and 2 more$/);
  assert.doesNotMatch(many, /TRUNCATE t5/);
});

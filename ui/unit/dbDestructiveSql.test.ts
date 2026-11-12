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

test('non-SQL engines are never classified', () => {
  assert.deepEqual(kinds('db.users.deleteMany({})', 'mongodb'), []);
  assert.deepEqual(kinds('FLUSHALL', 'redis'), []);
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

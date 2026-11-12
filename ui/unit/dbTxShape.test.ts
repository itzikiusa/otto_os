import { test } from 'node:test';
import assert from 'node:assert/strict';
import { txNotice, txShape, txUnclosedWarning } from '../src/modules/database/tx-shape.ts';

// Each Run has its own session, closed after a BEGIN: a transaction cannot span
// runs. The editor flags runs that look like they expect it to.
const shape = (sql: string, engine = 'mysql') => JSON.parse(JSON.stringify(txShape(sql, engine as never)));

test('a run of only transaction control is flagged', () => {
  assert.deepEqual(shape('START TRANSACTION'), { controlOnly: true, unclosed: true });
  assert.deepEqual(shape('begin;'), { controlOnly: true, unclosed: true });
  assert.deepEqual(shape('COMMIT'), { controlOnly: true, unclosed: false });
  assert.deepEqual(shape('rollback'), { controlOnly: true, unclosed: false });
  assert.deepEqual(shape('SAVEPOINT a; RELEASE SAVEPOINT a'), { controlOnly: true, unclosed: false });
  assert.deepEqual(shape('ROLLBACK TO SAVEPOINT a', 'postgres'), { controlOnly: true, unclosed: false });
  assert.deepEqual(shape('END', 'postgres'), { controlOnly: true, unclosed: false });
});

test('a batch that opens a transaction and does not close it is flagged; a closed one is not', () => {
  assert.deepEqual(shape('BEGIN; UPDATE t SET v = 1 WHERE id = 1;'), { controlOnly: false, unclosed: true });
  assert.equal(txShape('BEGIN; UPDATE t SET v = 1 WHERE id = 1; COMMIT;', 'mysql')?.unclosed, false);
  assert.equal(txShape('START TRANSACTION; DELETE FROM t WHERE id = 2; ROLLBACK;', 'mysql')?.unclosed, false);
  assert.equal(txShape('BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE t SET v = 1; END;', 'postgres')?.unclosed, false);
  // ROLLBACK TO keeps it open.
  assert.equal(txShape('BEGIN; SAVEPOINT s; UPDATE t SET v = 1; ROLLBACK TO s;', 'postgres')?.unclosed, true);
});

test('ordinary runs, other engines, strings and comments are not flagged', () => {
  assert.equal(txShape('SELECT 1', 'mysql'), null);
  assert.equal(txShape('UPDATE t SET v = 1 WHERE id = 1', 'postgres'), null);
  assert.equal(txShape("SELECT 'BEGIN'", 'mysql'), null);
  assert.equal(txShape('-- BEGIN\nSELECT 1', 'mysql'), null);
  assert.equal(txShape('BEGIN', 'clickhouse'), null);
  assert.equal(txShape('BEGIN', 'mongodb'), null);
  assert.equal(txShape('', 'mysql'), null);
});

test('the copy names the engine and what really happened', () => {
  const lone = txNotice({ controlOnly: true, unclosed: false }, 'mysql');
  assert.match(lone, /^Each run uses its own session, so a transaction cannot span runs\./);
  assert.match(lone, /Run all \(⇧⌘↵\)/);
  assert.match(lone, /changed nothing/);
  assert.match(txNotice({ controlOnly: false, unclosed: true }, 'postgres'), /PostgreSQL rolled back everything/);
  assert.match(txUnclosedWarning('mysql'), /MySQL then rolls the open transaction back/);
});

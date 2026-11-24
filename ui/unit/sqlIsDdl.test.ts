import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isDdl } from '../src/lib/database/sql-util.ts';

test('isDdl: schema changes after comments; reads and data writes are not', () => {
  for (const s of ['CREATE TABLE t (id int)', '  alter table t add c int', '-- note\nDROP VIEW v', '/* x */ RENAME TABLE a TO b', 'truncate t']) assert.ok(isDdl(s), s);
  // A script (Run all): a schema change after any statement, and MySQL # comments.
  for (const s of ['USE app;\nCREATE TABLE t (a int);', 'SELECT 1;\nALTER TABLE t ADD c int;', '# note\nDROP TABLE t', 'SELECT 1; -- x\n# y\ntruncate t']) assert.ok(isDdl(s), s);
  for (const s of ['SELECT 1', 'INSERT INTO t VALUES (1)', 'UPDATE t SET a = 1', "SELECT 'drop table x'", '']) assert.ok(!isDdl(s), s);
});

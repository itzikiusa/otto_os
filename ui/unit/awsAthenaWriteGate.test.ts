import { test } from 'node:test';
import assert from 'node:assert/strict';
import { athenaIsRead, athenaLeadKeyword } from '../src/modules/aws/athena-sql.ts';

// Athena runs DDL/DML through the same call as a SELECT; the editor confirms
// every non-read statement (typed on prod). Reads must not be nagged.
test('reads pass without a confirm', () => {
  for (const sql of [
    'SELECT 1',
    '  select * from t',
    '-- note\nSELECT 1',
    '/* hdr */ (SELECT 1) UNION (SELECT 2)',
    'WITH a AS (SELECT 1) SELECT * FROM a',
    'SHOW TABLES',
    'DESCRIBE t',
    'EXPLAIN SELECT 1',
    'EXPLAIN (FORMAT JSON) SELECT 1',
    'VALUES 1, 2',
  ]) {
    assert.equal(athenaIsRead(sql), true, sql);
  }
});

test('DDL, DML and unknown statements need a confirm', () => {
  for (const sql of [
    'DROP TABLE t',
    'insert into t select 1',
    'CREATE TABLE x AS SELECT 1',
    'ALTER TABLE t ADD COLUMNS (c int)',
    'MSCK REPAIR TABLE t',
    'DELETE FROM t WHERE 1=1',
    'MERGE INTO t USING s ON …',
    'UNLOAD (SELECT 1) TO \'s3://b/\'',
    '-- SELECT\nDROP TABLE t',
    '/* SELECT */ DROP TABLE t',
    'EXPLAIN ANALYZE INSERT INTO t SELECT 1',
    'EXPLAIN (TYPE IO) DELETE FROM t',
    '',
    '   ',
  ]) {
    assert.equal(athenaIsRead(sql), false, sql);
  }
  assert.equal(athenaLeadKeyword('/* x */ drop table t'), 'DROP');
});

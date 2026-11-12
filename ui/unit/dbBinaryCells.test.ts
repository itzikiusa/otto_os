import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import * as dialect from '../src/lib/database/sql-dialect.ts';
import { binaryCellBytes, binaryFilterOff, bytesToHex, bytesToHexCompact } from '../src/lib/database/binary-cells.ts';
import { plural } from '../src/lib/plural.ts';

// MySQL BINARY / VARBINARY / BLOB and Postgres BYTEA cells arrive as base64
// TEXT. Every statement the grid builds from them must write / compare the
// BYTES — quoted, the base64 text was stored instead (Duplicate row, Copy as
// INSERT) and a BINARY(16) key matched nothing (`WHERE id = '<base64>'`).
function sqlModule(): Record<string, any> {
  const ctx = { exports: {} as Record<string, any>, require: () => ({ isComplex: () => false, cellStr: String, compactJson: JSON.stringify, plural, ...dialect }) };
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8'),
    { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx.exports;
}

// A 16-byte UUID key and a small blob, as the daemon sends them.
const KEY = Buffer.from('0123456789abcdef0123456789abcdef', 'hex').toString('base64');
const BLOB = Buffer.from([0, 1, 2, 255]).toString('base64');

function ctxFor(engine: 'mysql' | 'postgres', keyType: string, blobType: string) {
  const { qid } = sqlModule();
  return {
    engine,
    target: { db: 'app', table: 'files', pkCols: ['id'] },
    columns: [{ name: 'id', type_hint: keyType, binary: true }, { name: 'body', type_hint: blobType, binary: true }, { name: 'name', type_hint: 'VARCHAR' }],
    liveRows: [[KEY, BLOB, 'a.bin'], [KEY, null, 'b.bin']],
    qid: (s: string) => qid(engine, s),
  };
}

const CASES = [
  { engine: 'mysql', key: 'BINARY', blob: 'BLOB', dec: (b: string) => `FROM_BASE64('${b}')`, q: (s: string) => `\`${s}\``, ref: '`app`.`files`' },
  { engine: 'postgres', key: 'BYTEA', blob: 'BYTEA', dec: (b: string) => `decode('${b}', 'base64')`, q: (s: string) => `"${s}"`, ref: '"app"."files"' },
] as const;

for (const c of CASES) {
  test(`${c.engine}: INSERT / Copy as INSERT decode binary cells`, () => {
    const { sqlAdapter } = sqlModule();
    const sql = sqlAdapter.buildInsert([0, 1], ctxFor(c.engine, c.key, c.blob));
    assert.equal(sql, [
      `INSERT INTO ${c.ref} (${c.q('id')}, ${c.q('body')}, ${c.q('name')}) VALUES (${c.dec(KEY)}, ${c.dec(BLOB)}, 'a.bin');`,
      `INSERT INTO ${c.ref} (${c.q('id')}, ${c.q('body')}, ${c.q('name')}) VALUES (${c.dec(KEY)}, NULL, 'b.bin');`,
    ].join('\n'));
  });

  test(`${c.engine}: Duplicate row decodes the blob (single key regenerates)`, () => {
    const { sqlAdapter } = sqlModule();
    const { sql } = sqlAdapter.buildDuplicate(0, ctxFor(c.engine, c.key, c.blob));
    assert.equal(sql, `INSERT INTO ${c.ref} (${c.q('body')}, ${c.q('name')}) VALUES (${c.dec(BLOB)}, 'a.bin');`);
  });

  test(`${c.engine}: UPDATE targets a binary primary key by its bytes`, () => {
    const { sqlAdapter } = sqlModule();
    const { sql } = sqlAdapter.buildUpdate([{ rowIdx: 0, patch: { cells: new Map([[2, 'c.bin']]) } }], ctxFor(c.engine, c.key, c.blob));
    assert.equal(sql, `UPDATE ${c.ref} SET ${c.q('name')} = 'c.bin' WHERE ${c.q('id')} = ${c.dec(KEY)};`);
  });

  test(`${c.engine}: DELETE targets binary primary keys by their bytes`, () => {
    const { sqlAdapter } = sqlModule();
    const { sql } = sqlAdapter.buildDelete([0, 1], ctxFor(c.engine, c.key, c.blob));
    assert.equal(sql, `DELETE FROM ${c.ref} WHERE ${c.q('id')} IN (${c.dec(KEY)}, ${c.dec(KEY)});`);
  });

  test(`${c.engine}: a composite key with a binary part decodes it in the per-row predicate`, () => {
    const { sqlAdapter } = sqlModule();
    const ctx = { ...ctxFor(c.engine, c.key, c.blob), target: { db: 'app', table: 'files', pkCols: ['id', 'name'] } };
    const { sql } = sqlAdapter.buildDelete([0], ctx);
    assert.equal(sql, `DELETE FROM ${c.ref} WHERE (${c.q('id')} = ${c.dec(KEY)} AND ${c.q('name')} = 'a.bin');`);
  });
}

test('text columns are untouched; non-base64 text in a binary column is quoted as typed', () => {
  const { valueLiteral } = sqlModule();
  assert.equal(valueLiteral('mysql', KEY, 'VARCHAR'), `'${KEY}'`);
  assert.equal(valueLiteral('mysql', 'not base64!', 'BLOB'), "'not base64!'");
  assert.equal(valueLiteral('mysql', null, 'BLOB'), 'NULL');
  assert.equal(valueLiteral('mysql', '', 'VARBINARY'), "FROM_BASE64('')");
});

test('isBinaryType mirrors the daemon list exactly', () => {
  for (const t of ['BINARY', 'VARBINARY', 'TINYBLOB', 'BLOB', 'MEDIUMBLOB', 'LONGBLOB', 'GEOMETRY', 'BYTEA', 'bytea']) assert.equal(dialect.isBinaryType(t), true, t);
  for (const t of ['VARCHAR', 'TEXT', 'String', 'BYTEA[]', 'JSON', 'geometry', 'BIT', null, undefined]) assert.equal(dialect.isBinaryType(t), false, String(t));
  assert.equal(dialect.binaryLiteral('mongodb', KEY), null);
  assert.equal(dialect.binaryLiteral('mysql', 'abc'), null, 'length not a multiple of 4');
});

test('the cell viewer renders bytes as hex with their length', () => {
  const bytes = binaryCellBytes(KEY);
  assert.ok(bytes);
  assert.equal(bytes.length, 16);
  assert.equal(bytesToHexCompact(bytes), '0123456789abcdef0123456789abcdef');
  assert.equal(bytesToHex(bytes, 8), '01 23 45 67 89 ab cd ef\n01 23 45 67 89 ab cd ef');
  assert.equal(binaryCellBytes('…[truncated 9 chars]'), null);
});

test('WHERE-building cell actions are disabled for binary cells, with the reason', () => {
  const off = binaryFilterOff({ binary: true }, KEY);
  assert.equal(off.disabled, true);
  assert.match(off.title ?? '', /base64/);
  assert.deepEqual({ ...binaryFilterOff({ binary: true }, null) }, {}, 'NULL still filters as IS NULL');
  assert.deepEqual({ ...binaryFilterOff({ name: 'x' } as never, 'abc') }, {});
  assert.deepEqual({ ...binaryFilterOff(undefined, 'abc') }, {});
});

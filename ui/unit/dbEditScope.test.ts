import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// Grid edits, "Copy as INSERT" and the post-apply refresh resolve the database
// a result RAN in from the tab's `ran_node` through this helper. It must mirror
// the daemon's `Scope` parse: tagged paths are paths, anything else a name.
function scopeDatabase(): (node: string | null | undefined) => string | null {
  const text = readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8');
  const start = text.indexOf('export function scopeDatabase(');
  const end = text.indexOf('\n}\n', start) + 3;
  const ctx: Record<string, any> = {};
  const source = text.slice(start, end).replace('export function', 'function');
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx.scopeDatabase;
}

test('scope database: plain names, db: paths, Redis keyspaces', () => {
  const f = scopeDatabase();
  assert.equal(f(null), null);
  assert.equal(f(undefined), null);
  assert.equal(f('  '), null);
  assert.equal(f('shop'), 'shop');
  assert.equal(f('db:shop/table:orders'), 'shop');
  assert.equal(f('db:'), null);
  // A Redis keyspace is not a database — Redis edits carry their own scope.
  assert.equal(f('kdb:3'), null);
  // Databases literally named like a path tag keep their scope.
  assert.equal(f('db'), 'db');
  assert.equal(f('kdb'), 'kdb');
  assert.equal(f('games:v2'), 'games:v2');
});

// 64-bit integers beyond 2^53 arrive as exact digit strings; edits must emit
// them BARE (a quoted value is compared as a double by MySQL) and verbatim.
function literals(): Record<string, any> {
  const text = readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8');
  const start = text.indexOf('export function isIntegerType(');
  const end = text.indexOf('/** The cell DRAFT');
  const ctx: Record<string, any> = {
    escapeSqlString: (s: string) => s.replace(/'/g, "''"),
    backslashEscapes: () => false,
    isComplex: () => false,
    compactJson: (v: unknown) => JSON.stringify(v),
  };
  const source = text.slice(start, end).replace(/export function/g, 'function');
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx;
}

test('big integer keys keep every digit and stay unquoted', () => {
  const c = literals();
  for (const t of ['BIGINT', 'BIGINT UNSIGNED', 'INT8', 'int4', 'Nullable(UInt64)', 'Int64', 'INTEGER']) {
    assert.equal(c.isIntegerType(t), true, t);
  }
  for (const t of ['POINT', 'INTERVAL', 'VARCHAR', 'TEXT', null, undefined]) {
    assert.equal(c.isIntegerType(t), false, String(t));
  }
  assert.equal(c.valueLiteral('mysql', '9007199254740993', 'BIGINT'), '9007199254740993');
  assert.equal(c.valueLiteral('postgres', '-9007199254740993', 'INT8'), '-9007199254740993');
  // A digit string in a text column is still a quoted string.
  assert.equal(c.valueLiteral('mysql', '9007199254740993', 'VARCHAR'), "'9007199254740993'");
  assert.equal(c.valueLiteral('mysql', 42, 'BIGINT'), '42');
});

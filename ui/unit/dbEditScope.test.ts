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

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EditorState } from '@codemirror/state';
import { sql } from '@codemirror/lang-sql';
import { isInertAt } from '../src/modules/database/completion-gate.ts';
import { sqlDialect, sqlDialectForKind, type SqlDialectName } from '../src/lib/sql-dialects.ts';

// The query editor gates auto-completion on the syntax tree (no identifier
// completion inside literals / comments). That tree is only right when the
// editor tokenizes with the connection's dialect — StandardSQL ends MySQL's
// `'O\'Brien'` at the `\'` and inverts every later string/code boundary.

function stateFor(doc: string, dialect: SqlDialectName): EditorState {
  return EditorState.create({ doc, extensions: [sql({ dialect: sqlDialect(dialect) })] });
}

/** Inert-ness at the `|` marker (removed from the doc). */
function inertAt(marked: string, dialect: SqlDialectName): boolean {
  const pos = marked.indexOf('|');
  assert.ok(pos >= 0, 'fixture needs a | cursor marker');
  const doc = marked.slice(0, pos) + marked.slice(pos + 1);
  return isInertAt(stateFor(doc, dialect), pos, false);
}

test('connection kinds map to their SQL dialect', () => {
  assert.equal(sqlDialectForKind('mysql'), 'mysql');
  assert.equal(sqlDialectForKind('postgres'), 'postgres');
  assert.equal(sqlDialectForKind('clickhouse'), 'clickhouse');
  assert.equal(sqlDialectForKind('redis'), 'standard');
  assert.equal(sqlDialectForKind(undefined), 'standard');
});

test('MySQL: a backslash-escaped quote does not end the string', () => {
  // After the literal closes, `, na|` is code — completion must fire there.
  const doc = "INSERT INTO t VALUES ('O\\'Brien', na|";
  assert.equal(inertAt(doc, 'mysql'), false);
  // Inside the literal, past the escape: still a string.
  assert.equal(inertAt("SELECT 'O\\'Bri|en'", 'mysql'), true);
  // StandardSQL gets this wrong (the regression the dialect fixes).
  assert.equal(inertAt(doc, 'standard'), true);
});

test('MySQL and ClickHouse: # starts a comment', () => {
  assert.equal(inertAt('SELECT 1 # a com|ment', 'mysql'), true);
  assert.equal(inertAt('SELECT 1 # a com|ment', 'clickhouse'), true);
  assert.equal(inertAt('SELECT 1 -- a com|ment', 'postgres'), true);
});

test('ClickHouse: a double quote is an identifier, not a string', () => {
  assert.equal(inertAt('SELECT "us|er_id" FROM t', 'clickhouse'), false);
  assert.equal(inertAt("SELECT 'us|er_id' FROM t", 'clickhouse'), true);
});

test('code positions stay completable', () => {
  for (const d of ['standard', 'mysql', 'postgres', 'clickhouse'] as const) {
    assert.equal(inertAt('SELECT id FROM us|', d), false, d);
    assert.equal(inertAt('SELECT 12|3', d), true, d);
  }
});

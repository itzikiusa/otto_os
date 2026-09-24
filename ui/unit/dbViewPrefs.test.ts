import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  AUTO_VERTICAL_DEFAULTS,
  autoVerticalFor,
  resolveAutoVertical,
} from '../src/lib/db-view-prefs.ts';

// Auto-Vertical is per engine: MongoDB on by default, SQL engines off — a wide
// SQL table stays a grid unless the user opted in.

test('defaults: Mongo auto-verticals past 10 columns, SQL never', () => {
  const { value, migrated } = resolveAutoVertical(null, null);
  assert.equal(migrated, false);
  assert.deepEqual(value, AUTO_VERTICAL_DEFAULTS);
  assert.equal(autoVerticalFor(value, 'mongodb'), 10);
  for (const e of ['mysql', 'postgres', 'clickhouse', 'redis']) assert.equal(autoVerticalFor(value, e), 0, e);
  assert.equal(autoVerticalFor(value, null), 0);
  assert.equal(autoVerticalFor(value, 'sqlite'), 0);
});

test('stored per-engine prefs merge over the defaults and clamp', () => {
  const { value, migrated } = resolveAutoVertical(JSON.stringify({ mysql: 4, mongodb: 0, postgres: 9999 }), null);
  assert.equal(migrated, false);
  assert.equal(value.mysql, 4);
  assert.equal(value.mongodb, 0);
  assert.equal(value.postgres, 500);
  assert.equal(value.clickhouse, 0); // untouched engine keeps its default
});

test('the new key wins over a leftover legacy value', () => {
  const { value, migrated } = resolveAutoVertical(JSON.stringify({ mysql: 3 }), '7');
  assert.equal(migrated, false);
  assert.equal(value.mysql, 3);
  assert.equal(value.postgres, 0);
});

test('legacy custom threshold: an explicit choice carries to every engine', () => {
  const { value, migrated } = resolveAutoVertical(null, '6');
  assert.equal(migrated, true);
  for (const e of ['mongodb', 'mysql', 'postgres', 'clickhouse', 'redis'] as const) assert.equal(value[e], 6, e);
});

test('legacy "never" stays never everywhere', () => {
  const { value, migrated } = resolveAutoVertical(null, '0');
  assert.equal(migrated, true);
  for (const e of ['mongodb', 'mysql', 'postgres', 'clickhouse', 'redis'] as const) assert.equal(value[e], 0, e);
});

test('legacy value equal to the old default takes the new defaults', () => {
  const { value, migrated } = resolveAutoVertical(null, '10');
  assert.equal(migrated, true);
  assert.deepEqual(value, AUTO_VERTICAL_DEFAULTS);
});

test('garbage never throws', () => {
  assert.deepEqual(resolveAutoVertical('{not json', 'abc').value, AUTO_VERTICAL_DEFAULTS);
  assert.deepEqual(resolveAutoVertical('[1,2]', '').value, AUTO_VERTICAL_DEFAULTS);
  assert.deepEqual(resolveAutoVertical('null', null).value, AUTO_VERTICAL_DEFAULTS);
});

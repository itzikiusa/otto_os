import { test } from 'node:test';
import assert from 'node:assert/strict';
import { columnZone } from '../src/modules/database/temporal-zone.ts';
import { columnZone as reExported } from '../src/modules/database/grid-format.ts';

// TIMESTAMP values were shown in the session zone (UTC by default) with no
// marker. The header now names the zone, and says so only where one applies.

test('MySQL TIMESTAMP is shown in the session zone; DATETIME has none', () => {
  assert.equal(columnZone('mysql', 'TIMESTAMP')?.label, 'UTC');
  assert.equal(columnZone('mysql', 'TIMESTAMP', '+00:00')?.label, 'UTC');
  assert.equal(columnZone('mysql', 'TIMESTAMP', 'Europe/Berlin')?.label, 'Europe/Berlin');
  assert.match(columnZone('mysql', 'TIMESTAMP', 'Europe/Berlin')!.note, /Time zone setting.*does not convert/);
  assert.equal(columnZone('mysql', 'DATETIME')?.label, 'no zone');
  assert.equal(columnZone('mysql', 'INT'), null);
  assert.equal(columnZone('mysql', null), null);
});

test('ClickHouse uses the zone named in the type, else the session zone', () => {
  assert.equal(columnZone('clickhouse', "DateTime('Asia/Tokyo')")?.label, 'Asia/Tokyo');
  assert.equal(columnZone('clickhouse', "Nullable(DateTime64(3, 'Europe/Paris'))")?.label, 'Europe/Paris');
  assert.equal(columnZone('clickhouse', 'DateTime')?.label, 'UTC');
  assert.equal(columnZone('clickhouse', 'DateTime64(3)', 'America/New_York')?.label, 'America/New_York');
  assert.equal(columnZone('clickhouse', 'Date'), null);
  assert.equal(columnZone('clickhouse', 'String'), null);
});

test('Postgres: TIMESTAMPTZ is UTC with its offset, TIMESTAMP has no zone', () => {
  assert.equal(columnZone('postgres', 'TIMESTAMPTZ')?.label, 'UTC');
  assert.equal(columnZone('postgres', 'TIMESTAMP')?.label, 'no zone');
});

test('engines without typed date-time columns get no marker', () => {
  assert.equal(columnZone('mongodb', 'date'), null);
  assert.equal(columnZone('redis', 'string'), null);
  assert.equal(reExported, columnZone);
});

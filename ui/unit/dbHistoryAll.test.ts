import { test } from 'node:test';
import assert from 'node:assert/strict';
import { historyConnections, mergeHistory } from '../src/modules/database/history-all.ts';

// History used to be per connection only; "All connections" merges every
// connection's recent rows, newest first, with a connection filter.
const h = (id: string, conn: string, at: string, statement: string, extra: Record<string, unknown> = {}) => ({
  id,
  connection_id: conn,
  statement,
  ok: true,
  duration_ms: 1,
  row_count: 1,
  created_at: at,
  ...extra,
});
const a = { id: 'a', name: 'orders-db' };
const b = { id: 'b', name: 'events-ch' };
const lists = [
  { conn: a, rows: [h('1', 'a', '2026-11-12T10:00:00Z', 'SELECT * FROM orders'), h('2', 'a', '2026-11-10T10:00:00Z', 'SELECT 1')] },
  { conn: b, rows: [h('3', 'b', '2026-11-11T10:00:00Z', 'SELECT count() FROM events', { ok: false, error: 'boom timeout' })] },
];

test('rows from every connection, newest first, labelled with their connection', () => {
  const rows = mergeHistory(lists, '', '');
  assert.deepEqual(rows.map((r) => r.id), ['1', '3', '2']);
  assert.equal(rows[1].connName, 'events-ch');
});

test('the connection filter narrows to one connection', () => {
  assert.deepEqual(mergeHistory(lists, 'b', '').map((r) => r.id), ['3']);
});

test('the search matches statement or error text, case-insensitively', () => {
  assert.deepEqual(mergeHistory(lists, '', 'ORDERS').map((r) => r.id), ['1']);
  assert.deepEqual(mergeHistory(lists, '', 'timeout').map((r) => r.id), ['3']);
  assert.deepEqual(mergeHistory(lists, 'a', 'timeout'), []);
});

test('connections are read most-recently-opened first, capped', () => {
  const conns = [
    { id: '1', name: 'zeta', last_opened_at: null },
    { id: '2', name: 'beta', last_opened_at: '2026-11-01T00:00:00Z' },
    { id: '3', name: 'alpha', last_opened_at: '2026-11-12T00:00:00Z' },
    { id: '4', name: 'acme' },
  ];
  assert.deepEqual(historyConnections(conns).map((c) => c.id), ['3', '2', '4', '1']);
  assert.deepEqual(historyConnections(conns, 2).map((c) => c.id), ['3', '2']);
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  groupItems, hostOf, matches, rowWord, sectionPath, shortReason, fromConnection, fromCluster, CHECK_FRESH_MS,
} from '../src/modules/connections/connectionRows.ts';
import type { BrokerCluster, Connection, ConnectionSection } from '../src/lib/api/types';

const conn = (over: Partial<Connection>): Connection => ({
  id: 'c1', workspace_id: null, name: 'orders', kind: 'mysql', params: { host: 'db.internal', port: 3306 }, secret_ref: null,
  first_command: null, section_id: null, environment: 'dev', read_only: false, created_by: 'u', created_at: '2026-01-01T00:00:00Z', ...over,
});
const ago = (at: number): string => `at ${at}`;
const sec = (id: string, name: string, parent_id: string | null = null, position = 0): ConnectionSection => ({
  id, workspace_id: 'w', parent_id, name, position, scope: 'connections', created_by: 'u', created_at: '',
});

test('hostOf: host:port, a tunnel reads via → target, a URI never shows credentials', () => {
  assert.equal(hostOf(conn({})), 'db.internal:3306');
  assert.equal(hostOf(conn({ params: { host: 'prod-mysql', port: 3306, ssh: { host: 'bastion' } } })), 'bastion → prod-mysql:3306');
  assert.equal(hostOf(conn({ kind: 'mongodb', params: { uri: 'mongodb+srv://admin:s3cret@cluster0.mongodb.net/app?retryWrites=true' } })), 'cluster0.mongodb.net');
  assert.equal(hostOf(conn({ kind: 'custom', params: {} })), '');
});

test('rowWord: no check is "Not checked"; a fresh success is Connected with its latency; a stale one is Not checked again', () => {
  const now = 10_000_000;
  assert.equal(rowWord(undefined, now, ago).word, 'Not checked');
  const ok = rowWord({ ok: true, latencyMs: 14.2, at: now - 60_000 }, now, ago);
  assert.equal(ok.word, 'Connected');
  assert.equal(ok.tone, 'success');
  assert.equal(ok.latency, '14 ms');
  assert.equal(rowWord({ ok: true, latencyMs: 14, at: now - CHECK_FRESH_MS - 1 }, now, ago).word, 'Not checked');
});

test('rowWord: a failure is Can’t connect with a short reason; an auth failure is Needs sign-in — whatever its age', () => {
  const now = 10_000_000;
  const err = rowWord({ ok: false, message: 'dial tcp 10.0.0.4:3306: connect: connection refused', at: now - 5 * 86_400_000 }, now, ago);
  assert.equal(err.word, 'Can’t connect');
  assert.equal(err.tone, 'danger');
  assert.ok(err.reason && err.reason.length <= 60);
  assert.equal(rowWord({ ok: false, message: "Access denied for user 'app'@'10.0.0.1' (using password: YES)", at: now }, now, ago).word, 'Needs sign-in');
  assert.equal(shortReason(''), null);
});

test('sectionPath follows parents; matches searches name, kind, host and section path', () => {
  const secs = [sec('p', 'PLATFORM'), sec('s', 'STG', 'p')];
  assert.equal(sectionPath(secs, 's'), 'PLATFORM / STG');
  const item = fromConnection(conn({ section_id: 's' }));
  const f = (q: string, kind = 'all', section: string | null = null) => matches(item, { q, kind, section }, secs);
  assert.ok(f('orders') && f('MYSQL') && f('db.internal') && f('platform / stg'));
  assert.ok(!f('postgres'));
  assert.ok(f('', 'mysql') && !f('', 'redis'));
  assert.ok(f('', 'all', 'p'), 'a parent section includes its sub-sections');
  assert.ok(!f('', 'all', ''), 'a filed row is not ungrouped');
});

test('groupItems: sections in tree order with their path, Ungrouped last; every row listed once', () => {
  const secs = [sec('b', 'B', null, 1), sec('a', 'A', null, 0), sec('a1', 'A1', 'a')];
  const items = [
    fromConnection(conn({ id: '1', name: 'z', section_id: 'a1' })),
    fromConnection(conn({ id: '2', name: 'y' })),
    fromCluster({ id: 'k', name: 'kafka', bootstrap_servers: 'b1:9092,b2:9092', section_id: 'b', environment: 'dev', read_only: false } as BrokerCluster),
  ];
  const g = groupItems(items, secs, false);
  assert.deepEqual(g.map((x) => x.label), ['A', 'A / A1', 'B', 'Ungrouped']);
  assert.equal(g[2].items[0].host, 'b1:9092');
  assert.deepEqual(groupItems(items, secs, true).map((x) => x.label), ['A / A1', 'B', 'Ungrouped'], 'narrowed: empty sections drop out');
  assert.deepEqual(groupItems(items, [], false).map((x) => x.label), [null], 'no sections: one heading-less group');
  const all = groupItems(items, secs, false).flatMap((x) => x.items.map((i) => i.id));
  assert.deepEqual([...all].sort(), ['1', '2', 'k']);
});

// perf K8s (review K2/K6): adaptive resources cadence, the conditional-read
// validator, and the Fleet's stable row / event identities.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resourcesPollMs, resourcesValidator } from '../src/modules/kubernetes/resourcePoll.ts';
import { fleetEventKey, fleetRowKey, reuseByKey, uniqueKeys } from '../src/modules/kubernetes/monitor/monitor-util.ts';
import { loadSource } from './sourceHarness.ts';

test('a small list polls every 10 s however slow the load was', () => {
  assert.equal(resourcesPollMs(0, 0), 10_000);
  assert.equal(resourcesPollMs(999, 25_000), 10_000);
});

test('a big list waits max(30 s, 3 × the last load)', () => {
  assert.equal(resourcesPollMs(1000, 0), 30_000);
  assert.equal(resourcesPollMs(5000, 4_000), 30_000);
  assert.equal(resourcesPollMs(5000, 12_000), 36_000);
  assert.equal(resourcesPollMs(5000, Number.NaN), 30_000);
});

test('the validator prefers the ETag header, else quotes the body version', () => {
  assert.equal(resourcesValidator('"abc"', 'zzz'), '"abc"');
  assert.equal(resourcesValidator(null, 'abc'), '"abc"');
  assert.equal(resourcesValidator(null, undefined), null);
  assert.equal(resourcesValidator(null, ''), null);
});

const row = (workload: string, restarts = 0, pod = '') => ({ cluster_id: 'c1', namespace: 'shop', workload, pod, restarts });

test('fleet row keys are cluster/ns/workload (+ pod in pod grouping)', () => {
  assert.equal(fleetRowKey(row('web')), 'c1/shop/web');
  assert.equal(fleetRowKey(row('web', 0, 'web-1')), 'c1/shop/web/web-1');
});

test('reuseByKey keeps unchanged objects and returns prev when nothing changed', () => {
  const prev = [row('a'), row('b'), row('c')];
  const same = reuseByKey(prev, prev.map((r) => ({ ...r })), fleetRowKey);
  assert.equal(same, prev, 'identical refresh ⇒ the very same array');

  const next = reuseByKey(prev, [row('a'), row('b', 3), row('c')], fleetRowKey);
  assert.notEqual(next, prev);
  assert.equal(next[0], prev[0]);
  assert.notEqual(next[1], prev[1], 'a changed row is the fresh object');
  assert.equal(next[1].restarts, 3);
  assert.equal(next[2], prev[2]);

  const resorted = reuseByKey(prev, [row('c'), row('a'), row('b')], fleetRowKey);
  assert.notEqual(resorted, prev, 'a new order is a new array…');
  assert.deepEqual(resorted, [prev[2], prev[0], prev[1]], '…of the same objects');

  const fresh = [row('x')];
  assert.equal(reuseByKey([], fresh, fleetRowKey), fresh);
});

test('event keys are stable and made unique', () => {
  const e = { ts: '2026-10-03T10:00:00Z', cluster_id: 'c1', namespace: 'shop', pod: 'web-1', container: 'app', reason: 'OOMKilled', kind: 'restart', class: 'oom' };
  assert.equal(fleetEventKey(e), fleetEventKey({ ...e }));
  assert.notEqual(fleetEventKey(e), fleetEventKey({ ...e, pod: 'web-2' }));
  assert.deepEqual(uniqueKeys(['a', 'b', 'a', 'a']), ['a', 'b', 'a#2', 'a#3']);
  assert.deepEqual(uniqueKeys(['a', 'b']), ['a', 'b']);
});

// ── api.getConditional (client.ts) ────────────────────────────────────────────
function clientWith(fetch: (url: string, init: RequestInit) => Promise<Response>) {
  return loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch });
}

test('a conditional GET sends If-None-Match and a 304 resolves notModified (no body read)', async () => {
  const sent: (string | undefined)[] = [];
  const { api } = clientWith(async (_url, init) => {
    sent.push((init.headers as Record<string, string>)['If-None-Match']);
    return new Response(null, { status: 304, headers: { ETag: '"v1"' } });
  });
  const r = await api.getConditional('/k8s/clusters/c1/resources?kind=pods', '"v1"');
  assert.deepEqual(sent, ['"v1"']);
  assert.equal(r.notModified, true);
  assert.equal(r.etag, '"v1"');
});

test('a conditional GET without a validator is a plain read that returns the body + ETag', async () => {
  const sent: (string | undefined)[] = [];
  const { api } = clientWith(async (_url, init) => {
    sent.push((init.headers as Record<string, string>)['If-None-Match']);
    return new Response(JSON.stringify({ kind: 'pods', items: [], has_metrics: false, version: 'v2' }), { status: 200, headers: { ETag: '"v2"' } });
  });
  const r = await api.getConditional('/k8s/clusters/c1/resources?kind=pods', null);
  assert.deepEqual(sent, [undefined]);
  assert.equal(r.notModified, false);
  assert.equal(r.etag, '"v2"');
  assert.equal(r.data.version, 'v2');
});

test('a conditional GET still throws on an error status', async () => {
  const { api } = clientWith(async () => new Response(JSON.stringify({ code: 'internal', message: 'boom' }), { status: 500 }));
  await assert.rejects(api.getConditional('/k8s/clusters/c1/resources?kind=pods', '"v1"'));
});

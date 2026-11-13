// Home, round 3: threads reach Home (Today's "Needs you" / "Working" and the
// Agents widget), the first-visit seed only places widgets that have data,
// and the ⌘K snip entry is offered only where it can work. Zoom-in-place (no
// second mount, no re-fetch) is proven in e2e/desktop-home.spec.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

const src = (p: string) => new URL(`../src/${p}`, import.meta.url);
const read = (p: string) => readFileSync(src(p), 'utf8');
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v));

const buckets = loadSource(src('lib/stores/sessionBuckets.ts'), {});
const threads = loadSource(src('modules/home/homeThreads.ts'), { '../../lib/stores/sessionBuckets': buckets });

const s = (id: string, meta: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) =>
  ({ id, kind: 'agent', archived: false, workspace_id: 'w', title: id, meta, ...extra }) as never;

test('threadSessions lists Threads-scope sessions once — delegated children included, background/archived/connections not', () => {
  const list = [
    s('agent'),
    s('t1', { surface: 'threads' }),
    s('child', { delegated_by: 't1' }),
    s('bg', { surface: 'threads', source: 'workflow' }),
    s('gone', { surface: 'threads' }, { archived: true }),
    s('conn', { surface: 'threads' }, { kind: 'connection' }),
    s('back', { surface: 'agents', delegated_by: 't1' }),
  ];
  const pool = new Map(list.map((x: { id: string }) => [x.id, x]));
  assert.deepEqual(plain(threads.threadSessions(list, pool).map((x: { id: string }) => x.id)), ['t1', 'child']);
  assert.deepEqual(plain(threads.threadSessions(undefined, undefined)), []);
});

test('a thread opens on #/threads/<id>; unlisted runtime waits never go negative', () => {
  assert.equal(threads.threadRoute('a b'), 'threads/a%20b');
  assert.equal(threads.unlistedWaiting(3, 1), 2);
  assert.equal(threads.unlistedWaiting(1, 2), 0);
  assert.equal(threads.unlistedWaiting(undefined, 0), 0);
});

test('Today builds on the thread list (labelled, routed to Threads)', () => {
  const today = read('modules/home/today.svelte.ts');
  assert.match(today, /threadSessions\(ws\.activeSessions, ws\.sessionById\)/);
  assert.match(today, /detail: 'Thread · waiting for your input'/);
  assert.match(today, /open: \(\) => router\.go\(threadRoute\(s\.id\)\)/);
  assert.match(today, /unlistedWaiting\(ws\.threadsNeedsYouCount/);
});

function seedModule() {
  const calls: string[] = [];
  const mod = loadSource(src('modules/home/seed.ts'), {
    '../../lib/api/client': { api: { get: async (u: string) => (calls.push(u), [{ id: 'd1' }, { id: 'd2' }]) } },
    '../../lib/api/k8s': { k8sApi: { listClusters: async () => (calls.push('clusters'), []) } },
  });
  return { mod, calls };
}

test('first-visit seed: Agents always; Kubernetes / DB dashboard only with data', async () => {
  const { mod, calls } = seedModule();
  const all = () => true;
  const kinds = (d: unknown) => plain(mod.seedBoxes(all, d)).map((b: { kind: string }) => b.kind);
  assert.deepEqual(kinds({ clusters: false, dashboardId: null }), ['sessions']);
  assert.deepEqual(kinds({ clusters: true, dashboardId: null }), ['sessions', 'k8s']);
  assert.deepEqual(plain(mod.seedBoxes(all, { clusters: false, dashboardId: 'd1' }))[1], { kind: 'db-dashboard', config: { dashboardId: 'd1' } });
  // RBAC still gates every kind.
  assert.deepEqual(plain(mod.seedBoxes((f: string) => f === 'database', { clusters: true, dashboardId: 'd1' })).map((b: { kind: string }) => b.kind), ['db-dashboard']);
  // The probe asks only for what the role can see, and picks the first dashboard.
  assert.deepEqual(plain(await mod.probeSeedData('w 1', all)), { clusters: false, dashboardId: 'd1' });
  assert.deepEqual(calls.sort(), ['/workspaces/w%201/db/dashboards', 'clusters']);
  calls.length = 0;
  assert.deepEqual(plain(await mod.probeSeedData(null, (f: string) => f === 'agents')), { clusters: false, dashboardId: null });
  assert.deepEqual(calls, []);
});

test('the seed never touches a Home the person already arranged', () => {
  const home = read('modules/home/home.svelte.ts');
  const body = home.slice(home.indexOf('ensureDefault(can'), home.indexOf('// ── Views'));
  assert.match(body, /if \(this\.views\.length > 0\) return;/);
  assert.match(body, /JSON\.stringify\(cur\[0\]\.boxes\) !== seeded\) return;/);
});

test('⌘K snip is registered only in the desktop app and for Agents:edit', () => {
  const sources = read('shell/paletteSources.svelte.ts');
  assert.match(sources, /\.\.\.\(isTauri && auth\.can\('agents', 'edit'\) \? \[\{ id: 'core\.snip'/);
});

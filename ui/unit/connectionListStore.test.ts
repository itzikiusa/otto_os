// The shared connection list (lib/stores/connectionList.svelte.ts): a re-read
// asked for while one is on the wire is not folded into it, a workspace switch
// mid-read never hangs the new one and a change during the first load is read
// (R1-046), a saved
// edit forgets the old health verdict (R1-034).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function store(ws = { currentId: 'w1' }) {
  const calls: { path: string; response: ReturnType<typeof deferred<any>> }[] = [];
  const get = (path: string) => { const response = deferred<any>(); calls.push({ path, response }); return response.promise; };
  const { connectionList } = loadSource(new URL('../src/lib/stores/connectionList.svelte.ts', import.meta.url), {
    '../api/client': { api: { get } },
    './workspace.svelte': { ws },
    '../live': { appLive: { on() {}, onResync() {} } },
    '../storage': { lsGet: () => null, lsSet() {} },
  });
  return { connectionList, calls };
}
const tick = () => new Promise((r) => setImmediate(r));
const conn = (id: string) => ({ id, name: id, kind: 'mysql', params: {} });

test('a refresh asked for during an in-flight read reads again, and the older answer never erases a saved row', async () => {
  const { connectionList, calls } = store();
  const seen: string[][] = [];
  connectionList.onRefreshed((rows: { id: string }[]) => seen.push(rows.map((r) => r.id)));
  void connectionList.refresh();
  const rows = () => calls.filter((c) => c.path.endsWith('/connections'));
  assert.equal(rows().length, 1);
  // Saved while the first read is on the wire (onSaved: upsert, then refresh).
  connectionList.upsert(conn('new'));
  const second = connectionList.refresh();
  calls[0].response.resolve([conn('old')]);
  calls[1].response.resolve([]);
  await tick();
  assert.deepEqual(Array.from(connectionList.rows, (r: { id: string }) => r.id), ['new'], 'the pre-save answer is not applied');
  assert.equal(rows().length, 2, 'a second read follows the first');
  calls[2].response.resolve([conn('old'), conn('new')]);
  calls[3].response.resolve([]);
  await second;
  assert.deepEqual(Array.from(connectionList.rows, (r: { id: string }) => r.id), ['old', 'new']);
  assert.ok(seen.every((ids) => ids.includes('new')), 'no listener ever saw the list without the saved row');
});

test('a saved edit drops the connection\'s old health verdict; a move keeps it', () => {
  const { connectionList } = store();
  connectionList.recordCheck('a', { ok: false, message: 'Access denied' });
  connectionList.upsert(conn('a'));
  assert.equal(connectionList.checks.a?.ok, false, 'a move/stamp keeps what is known');
  connectionList.upsert(conn('a'), true);
  assert.equal(connectionList.checks.a, undefined);
});

test('a workspace switch while W1 has a re-read queued shows W2 at once and never reads W2 twice', async () => {
  const ws = { currentId: 'w1' };
  const { connectionList, calls } = store(ws);
  const rows = () => calls.filter((c) => c.path.endsWith('/connections'));
  void connectionList.refresh();
  void connectionList.refresh(); // a re-read asked for during W1's read
  ws.currentId = 'w2';
  void connectionList.refresh();
  const w2 = rows().find((c) => c.path.includes('/w2/'))!;
  const w2Sections = calls.find((c) => c.path === '/workspaces/w2/connection-sections')!;
  w2.response.resolve([conn('two')]);
  w2Sections.response.resolve([]);
  await tick();
  assert.deepEqual(Array.from(connectionList.rows, (r: { id: string }) => r.id), ['two']);
  assert.equal(connectionList.loaded, true);
  assert.equal(connectionList.loading, false);
  for (const c of calls.filter((c) => c.path.includes('/w1/'))) c.response.resolve([conn('one')]);
  await tick();
  assert.deepEqual(Array.from(connectionList.rows, (r: { id: string }) => r.id), ['two'], 'W1\'s late answer is not applied');
  assert.equal(rows().filter((c) => c.path.includes('/w2/')).length, 1, 'no spurious W2 re-read');
});

test('a change announced during the first load is read', async () => {
  const { connectionList, calls } = store();
  const rows = () => calls.filter((c) => c.path.endsWith('/connections'));
  void connectionList.refresh();
  connectionList.invalidate(); // connections_changed before the first answer
  assert.equal(rows().length, 2, 'the change starts another read');
  calls[0].response.resolve([conn('old')]);
  calls[1].response.resolve([]);
  await tick();
  assert.equal(connectionList.loaded, false, 'the older answer is not taken as the load');
  calls[2].response.resolve([conn('old'), conn('new')]);
  calls[3].response.resolve([]);
  await tick();
  assert.deepEqual(Array.from(connectionList.rows, (r: { id: string }) => r.id), ['old', 'new']);
});

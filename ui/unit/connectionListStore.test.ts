// The shared connection list (lib/stores/connectionList.svelte.ts): a re-read
// asked for while one is on the wire is not folded into it (R1-046), a saved
// edit forgets the old health verdict (R1-034).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function store() {
  const calls: { path: string; response: ReturnType<typeof deferred<any>> }[] = [];
  const get = (path: string) => { const response = deferred<any>(); calls.push({ path, response }); return response.promise; };
  const { connectionList } = loadSource(new URL('../src/lib/stores/connectionList.svelte.ts', import.meta.url), {
    '../api/client': { api: { get } },
    './workspace.svelte': { ws: { currentId: 'w1' } },
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

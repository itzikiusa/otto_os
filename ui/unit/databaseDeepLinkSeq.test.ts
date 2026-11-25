// #/database/<id>: only the latest deep open counts. A → B → A on a Kafka
// cluster not loaded yet: the first A's superseded cluster read must neither
// claim the link (the URL then snaps back to B) nor open the cluster (R2-011).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

test('a superseded deep open of a Kafka cluster neither claims the link nor opens it', async () => {
  const loads: { d: ReturnType<typeof deferred<void>>; gen: number }[] = [];
  let gen = 0;
  const opened: string[] = [];
  const brokers = {
    clusters: [] as { id: string }[],
    load: () => { const d = deferred<void>(); loads.push({ d, gen: ++gen }); return d.promise; },
  };
  const s = componentFunctions(new URL('../src/modules/database/DatabasePage.svelte', import.meta.url), ['startDeep', 'openDeep'], {
    database: { connections: [{ id: 'B' }], otherConnections: [], openConnection: async () => {} },
    connectionList: { stampOpened() {} },
    brokers,
    ws: { currentId: 'w' },
    openCluster: (c: { id: string }) => void opened.push(c.id),
    openTerminal: () => {},
    deepSeq: 0,
    deepTried: '',
    deepId: 'A',
  });
  const settle = () => new Promise((r) => setImmediate(r));
  s.startDeep('A');
  s.deepId = 'B';
  s.startDeep('B');
  await settle();
  assert.equal(s.deepTried, 'B');
  s.deepId = 'A';
  s.startDeep('A');
  loads[0].d.resolve(); // the superseded read answers first and lists nothing
  await settle();
  assert.equal(s.deepTried, '', 'the old open does not claim the link the new one is still opening');
  brokers.clusters = [{ id: 'A' }];
  loads[1].d.resolve();
  await settle();
  assert.equal(s.deepTried, 'A');
  assert.deepEqual(opened, ['A'], 'opened once, by the latest open');
});

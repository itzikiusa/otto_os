import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const summaries: ReturnType<typeof deferred<any>>[] = [];
  const details: ReturnType<typeof deferred<any>>[] = [];
  const { proof } = loadSource(new URL('../src/lib/stores/proof.svelte.ts', import.meta.url), {
    '../api/proof': {
      PROOF_SUMMARY_CHUNK: 100,
      listProofPacksPage: async () => ({ packs: [], next: null }),
      proofSummary: () => { const d = deferred<any>(); summaries.push(d); return d.promise; },
      getProofPack: () => { const d = deferred<any>(); details.push(d); return d.promise; },
    }, '../loadError': { loadErrorText: String },
  });
  return { proof, summaries, details };
}
const row = (id: string) => ({ work_item_kind: 'session', work_item_id: id, proof_pack_id: id });

test('Proof list workspace change clears summaries before sidebar loads', async () => {
  const { proof, summaries } = fixture();
  const first = proof.loadSummary('A', ['session:shared']);
  summaries[0].resolve({ rows: [row('shared')] }); await first;
  await proof.loadPacks('B');
  assert.equal(proof.summaryFor('session', 'shared'), null);
  const second = proof.loadSummary('B', ['session:shared']);
  assert.equal(summaries.length, 2, 'new workspace must request its own summary');
  summaries[1].resolve({ rows: [] }); await second;
});

test('Proof detail cannot publish after its workspace changes', async () => {
  const { proof, details } = fixture();
  await proof.loadPacks('A'); const old = proof.open('pack-A');
  await proof.loadPacks('B'); details[0].resolve({ pack: { id: 'pack-A' } }); await old;
  assert.equal(proof.detail, null);
});

test('Proof identity change invalidates pending summary reads in the same workspace', async () => {
  const { proof, summaries } = fixture();
  const old = proof.loadSummary('A', ['session:private']);
  proof.identityChanged();
  summaries[1].resolve({ rows: [] }); await Promise.resolve();
  summaries[0].resolve({ rows: [row('private')] }); await old;
  assert.equal(proof.summaryFor('session', 'private'), null);
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// "Run on…" multi-target helpers (src/modules/database/multi-run.ts): value
// lists, target fan-out, the typed-confirmation phrase and the summary line.

const mr = loadSource(new URL('../src/modules/database/multi-run.ts', import.meta.url), {});
const plain = (v: unknown): unknown => JSON.parse(JSON.stringify(v));

test('value lists split by line when multi-line, else by comma', () => {
  assert.deepEqual(plain(mr.parseValues('1, 2,3')), ['1', '2', '3']);
  assert.deepEqual(plain(mr.parseValues('a,b\nc\n\n')), ['a,b', 'c']);
  assert.deepEqual(plain(mr.parseValues('   ')), []);
});

test('scope options come from database / schema / keyspace roots', () => {
  const root = [
    { id: 'db:shop', label: 'shop', kind: 'database', has_children: true },
    { id: 'kdb:3', label: 'db3', kind: 'keyspace', has_children: true },
    { id: 'x', label: 'x', kind: 'folder', has_children: true },
  ];
  assert.deepEqual(plain(mr.scopeOptions(root)), [
    { value: 'shop', label: 'shop' },
    { value: 'kdb:3', label: 'db3' },
  ]);
});

test('targets fan out per picked scope; no scope = the default database', () => {
  const targets = mr.buildTargets(
    ['stg', 'prod', 'unpicked'],
    { stg: ['a', 'b'], prod: [] },
    { mode: 'auto' },
    { [mr.targetKey('stg', 'b')]: { mode: 'custom', name: ' main ' } },
  );
  assert.deepEqual(plain(targets), [
    { connection_id: 'stg', node: 'a', cluster_mode: 'auto' },
    { connection_id: 'stg', node: 'b', cluster_mode: 'custom', cluster_name: 'main' },
    { connection_id: 'prod', node: null, cluster_mode: 'auto' },
  ]);
});

test('the confirm phrase is the connection name for one guarded connection, else RUN n', () => {
  const target = (name: string) => ({ connection_name: name });
  const run = (t: number, needs: boolean) => ({ target: t, needs_confirm: needs });
  const one = { targets: [target('ch-prod'), target('ch-stg')], runs: [run(0, true), run(0, true), run(1, false)] };
  assert.equal(mr.confirmPhrase(one), 'ch-prod');
  const two = { targets: [target('a'), target('b')], runs: [run(0, true), run(1, true), run(1, true)] };
  assert.equal(mr.confirmPhrase(two), 'RUN 3');
  assert.ok(mr.phraseMatches('  run 3 ', 'RUN 3'));
  assert.ok(!mr.phraseMatches('run 2', 'RUN 3'));
});

test('summary omits zero counts', () => {
  const s = { total: 6, ok: 3, failed: 1, running: 0, pending: 2, skipped: 0, cancelled: 0 };
  assert.equal(mr.summaryText(s), '3 ok · 1 failed · 2 pending');
  assert.equal(mr.summaryText({ ...s, ok: 0, failed: 0, pending: 0 }), '6 runs');
});

test('cluster lines explain the decision', () => {
  const base = { mode: 'auto', detected: null, candidates: [], database_engine: null, applied: null, note: null };
  assert.equal(mr.clusterLine({ ...base, source: 'macro', detected: 'main', applied: 'main' }), 'ON CLUSTER main (from the {cluster} macro)');
  assert.equal(mr.clusterLine({ ...base, source: 'not_cluster' }), 'Single node — no ON CLUSTER');
  assert.equal(
    mr.clusterLine({ ...base, source: 'replicated_database', database_engine: 'Replicated' }),
    'Replicated database — DDL replicates itself',
  );
  assert.equal(mr.clusterLine({ ...base, mode: 'off', source: 'not_needed' }), 'ON CLUSTER off');
});

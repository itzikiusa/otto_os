import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { revalidateAfterRemove, validateMonitorConfig } from '../src/modules/kubernetes/monitor/monitor-settings.ts';
import { credentialsRejected, rbacMessage } from '../src/modules/kubernetes/monitor/monitor-util.ts';

const cfg = (names: string[]) => ({
  interval_secs: 60, concurrency: 4, retention_days: 7, series_cap: 1000, namespaces: ['default'], exclusions: [],
  probes: names.map((name) => ({ name, port: null, path: '/metrics', format: 'prometheus', mappings: [], include: [], exclude: [], timeout_ms: 3000 })),
}) as any;

// UC-6-05: errors are keyed by row index; removing the invalid row must not
// leave its message on the row that slid into its slot.
test('removing the invalid probe clears its message instead of moving it to the next row', () => {
  const c = cfg(['', 'named']);
  const errors = validateMonitorConfig(c, false);
  assert.equal(errors['probe.0.name'], 'Name is required.');
  c.probes = c.probes.filter((_: unknown, j: number) => j !== 0); // the editor's removeProbe(0)
  const after = revalidateAfterRemove(errors, c, false);
  assert.equal(after['probe.0.name'], undefined, 'the valid probe now at index 0 is not flagged');
  assert.deepEqual(after, {}, 'and the banner has nothing left to count');
});

test('a remove while no errors show stays quiet until Save', () => {
  const c = cfg(['', 'x']);
  assert.deepEqual(revalidateAfterRemove({}, c, false), {}, 'no fresh errors before the person pressed Save');
});

test('removing a valid row re-indexes the remaining error onto its row', () => {
  const c = cfg(['ok', '']);
  const errors = validateMonitorConfig(c, false);
  assert.ok(errors['probe.1.name']);
  c.probes = c.probes.filter((_: unknown, j: number) => j !== 0);
  assert.equal(revalidateAfterRemove(errors, c, false)['probe.0.name'], 'Name is required.');
});

// integrations-6-04: a gateway 401 is a rejected credential, not RBAC.
test('a rejected gateway credential is not shown as RBAC', () => {
  const rejected = 'forbidden: credentials rejected (metrics-server answered HTTP 401); restarting the proxy';
  assert.equal(credentialsRejected(rejected), true);
  assert.equal(rbacMessage(rejected), null);
  const rbac = 'forbidden: cluster RBAC: metrics-server answered HTTP 403';
  assert.equal(credentialsRejected(rbac), false);
  assert.equal(rbacMessage(rbac), 'metrics-server answered HTTP 403');
});

// UC-6-04: two series may share a label (two clusters named alike); a
// label-keyed {#each} throws each_key_duplicate. Every series block keys by
// position.
test('MetricChart never keys its series by label', () => {
  const src = readFileSync(new URL('../src/lib/components/MetricChart.svelte', import.meta.url), 'utf8');
  const blocks = [...src.matchAll(/\{#each series as [^}]*\}/g)].map((m) => m[0]);
  assert.ok(blocks.length >= 4, `found the series blocks: ${blocks.join(' | ')}`);
  for (const b of blocks) assert.match(b, /\(i\)\}$/, `${b} keys by position`);
  assert.doesNotMatch(src, /\(s\.label\)/);
});

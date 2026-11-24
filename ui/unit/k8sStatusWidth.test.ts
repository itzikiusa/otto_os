// Kubernetes table: a long status ("CrashLoopBackOff") widens its column
// instead of being cut to "CrashLoopBa…".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import type { K8sRow } from '../src/lib/api/types.ts';

const { columnsFor, statusWidth } = loadSource(new URL('../src/modules/kubernetes/columns.ts', import.meta.url), { svelte: {} }, {}, { relative: true });

const row = (status: string) => ({ name: 'p', namespace: 'n', status }) as unknown as K8sRow;

test('short statuses keep the default status column', () => {
  assert.equal(statusWidth([row('Running'), row('Pending')]), 'minmax(104px, 0.9fr)');
});

test('the status column fits the longest status in the list', () => {
  assert.equal(statusWidth([row('Running'), row('CrashLoopBackOff')]), 'minmax(calc(16ch + 34px), 0.9fr)');
  const pods = columnsFor('pods', [row('Running'), row('ImagePullBackOff')], false, false);
  assert.equal(pods.find((c: { key: string }) => c.key === 'status')?.width, 'minmax(calc(16ch + 34px), 0.9fr)');
});

test('the status follows the name, so a clipped table never hides it', () => {
  const pods = columnsFor('pods', [row('CrashLoopBackOff')], true, true);
  assert.equal(pods.slice(0, 3).map((c: { key: string }) => c.key).join(), 'name,status,namespace');
});

test('events keep their fixed Type column', () => {
  const ev = columnsFor('events', [row('Warning-but-very-long')], false, false);
  assert.equal(ev.find((c: { key: string }) => c.key === 'status')?.width, '100px');
});

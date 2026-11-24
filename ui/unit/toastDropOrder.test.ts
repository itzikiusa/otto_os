// A burst of toasts drops a plain one before a toast whose close commits
// something (Delete with Undo): the Undo must not end early (R1-006).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('a burst drops plain toasts before an undo toast', () => {
  const { toasts } = loadSource(new URL('../src/lib/toast.svelte.ts', import.meta.url), {});
  const closed: string[] = [];
  toasts.push('info', 'Deleted “prod”', undefined, 6000, { action: { label: 'Undo', run() {} }, onClose: (r: string) => closed.push(r) });
  for (let i = 0; i < 6; i++) toasts.info(`other ${i}`);
  assert.deepEqual(closed, [], 'the undo toast is still up');
  assert.ok(toasts.toasts.some((t: { title: string }) => t.title === 'Deleted “prod”'));
  assert.equal(toasts.toasts.length, 5);
  for (const t of [...toasts.toasts]) toasts.dismiss(t.id);
});

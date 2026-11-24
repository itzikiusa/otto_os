// A burst of toasts — errors too — drops a plain one before a toast whose
// close commits something (a session's Delete with Undo): the Undo must not
// end early and commit the delete (R1-006).
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

test('a burst of errors drops an error before an undo toast', () => {
  const { toasts } = loadSource(new URL('../src/lib/toast.svelte.ts', import.meta.url), {});
  const closed: string[] = [];
  toasts.push('info', 'Session deleted', undefined, 6000, { action: { label: 'Undo', run() {} }, onClose: (r: string) => closed.push(r) });
  for (let i = 0; i < 6; i++) toasts.error(`failed ${i}`);
  assert.deepEqual(closed, [], 'the undo toast is still up');
  assert.equal(toasts.toasts.length, 5);
  for (const t of [...toasts.toasts]) toasts.dismiss(t.id);
});

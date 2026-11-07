import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function setup() {
  const calls: string[] = [];
  const pending = new Map<string, ReturnType<typeof deferred>>();
  const { startThreadTerminal } = loadSource(new URL('../src/modules/threads/runtime/threadTerminalStart.ts', import.meta.url), {
    '../../../lib/stores/workspace.svelte': { SCRATCH_WORKSPACE_ID: 'scratch', ws: {
      currentId: 'workspace',
      createSessionQuiet: (req: { meta: { terminal_of: string } }) => {
        calls.push(req.meta.terminal_of);
        const result = deferred(); pending.set(req.meta.terminal_of, result); return result.promise;
      },
    } },
  });
  const row = (id: string) => ({ id, title: id, cwd: '/tmp', workspace_id: 'workspace' });
  return { startThreadTerminal, row, calls, pending };
}

test('A to B to A while starts are pending creates only one shell per thread across mounts', async () => {
  const s = setup();
  const a = s.startThreadTerminal(s.row('A'));
  const b = s.startThreadTerminal(s.row('B'));
  assert.equal(s.startThreadTerminal(s.row('A')), a);
  assert.deepEqual(s.calls, ['A', 'B']);
  s.pending.get('A')!.resolve({ id: 'shell-A' });
  s.pending.get('B')!.resolve({ id: 'shell-B' });
  await Promise.all([a, b]);
});

test('a failed start releases its slot so explicit close and reopen can retry', async () => {
  const s = setup();
  const a = s.startThreadTerminal(s.row('A'));
  s.pending.get('A')!.reject(new Error('offline'));
  await assert.rejects(a, /offline/);
  const retry = s.startThreadTerminal(s.row('A'));
  assert.deepEqual(s.calls, ['A', 'A']);
  s.pending.get('A')!.resolve({ id: 'shell-A' });
  await retry;
});

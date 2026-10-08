import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  const pending = deferred<any[]>();
  const ws = { currentId: 'A', listSettled: true, foregroundActive: [], activeWorkflowRuns: [], needsYou: {}, statusMap: {} };
  const { today } = loadSource(new URL('../src/modules/home/today.svelte.ts', import.meta.url), {
    '../../lib/api/client': { api: { get: () => pending.promise } }, '../../lib/api/design': { listArtifacts: async () => [] },
    '../../lib/api/missionControl': { missionControlApi: { summary: async () => null, items: async () => [] } },
    '../../lib/api/scheduledTasks': { scheduledTasksApi: { list: async () => [] } }, svelte: { untrack: (fn: () => unknown) => fn() },
    '../../lib/router.svelte': { router: { go() {} } }, '../../lib/stores/auth.svelte': { auth: { can: () => true } },
    '../../lib/stores/assistant.svelte': { assistant: { tasks: { data: [], state: 'loaded' }, needs: { items: [] }, loadNeedsYou() {}, loadTasks() {} } },
    '../../lib/stores/notifications.svelte': { notifications: { notices: [], ensureLoaded() {} } },
    '../../lib/stores/workspace.svelte': { ws }, './boxes/poll': { livePoll: () => ({ now() {}, stop() {} }) },
    '../../lib/plural': { plural: () => '' },
  });
  return { today, ws, pending };
}

test('Home pending approvals cannot publish after workspace changes while its poll is busy', async () => {
  const { today, ws, pending } = fixture(); today.start();
  const loading = today.load(); ws.currentId = 'B'; today.refresh();
  pending.resolve([{ id: 'old', workspace_id: 'A' }]); await loading;
  assert.equal(today.approvals.length, 0);
  assert.equal(today.loaded, false);
  today.stop();
});

test('Home stops displaying old scoped rows immediately on workspace switch', async () => {
  const { today, ws, pending } = fixture(); today.start();
  const loading = today.load(); pending.resolve([{ id: 'old', workspace_id: 'A' }]); await loading;
  assert.equal(today.approvals.length, 1);
  ws.currentId = 'B'; today.refresh(); assert.equal(today.approvals.length, 0); assert.equal(today.loaded, false);
  today.stop();
});

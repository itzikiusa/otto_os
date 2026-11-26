// The run page's writes (modules/workflows/runs/runActions.ts): a step's
// approval is answered where it was raised — a provider request on the step's
// thread, a governed Otto MCP call in the MCP approvals queue (CR1-017).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function actions() {
  const calls: unknown[][] = [];
  const mod = loadSource(new URL('../src/modules/workflows/runs/runActions.ts', import.meta.url), {
    '../../../lib/api/client': { api: {} },
    '../../../lib/api/mcp': { mcpCpApi: { cpDecide: async (...a: unknown[]) => { calls.push(['mcp', ...a]); } } },
    '../../../lib/api/workflows': {},
    '../../../lib/confirm.svelte': {},
    '../../../lib/runtimeThreadPort': { loadIdeRuntimeSessionPort: async () => ({ runtimeThreads: { respond: async (...a: unknown[]) => { calls.push(['thread', ...a]); } } }) },
    '../../../lib/toast.svelte': { toasts: { success() {} } },
    '../../../lib/toastError': { toastError: (...a: unknown[]) => calls.push(['error', ...a]) },
    '../../../lib/router.svelte': { router: {} },
  });
  return { answerRequest: mod.answerRequest as (wait: object, approve: boolean, note?: string | null) => Promise<boolean>, calls };
}

test('a provider approval is answered on the step thread', async () => {
  const { answerRequest, calls } = actions();
  assert.equal(await answerRequest({ via: 'provider', thread_id: 't1', request_id: 'req-1' }, false, 'not now'), true);
  assert.deepEqual(calls, [['thread', 't1', 'req-1', 'decline']]);
});

test('a governed MCP approval is decided in the MCP queue, with the note', async () => {
  const { answerRequest, calls } = actions();
  assert.equal(await answerRequest({ via: 'mcp', thread_id: 't1', request_id: 'appr-9' }, false, 'wrong channel'), true);
  assert.deepEqual(calls.map((c) => JSON.parse(JSON.stringify(c))), [['mcp', 'appr-9', { approved: false, note: 'wrong channel' }]]);
  calls.length = 0;
  await answerRequest({ via: 'mcp', thread_id: 't1', request_id: 'appr-9' }, true);
  assert.deepEqual(calls.map((c) => JSON.parse(JSON.stringify(c))), [['mcp', 'appr-9', { approved: true }]]);
});

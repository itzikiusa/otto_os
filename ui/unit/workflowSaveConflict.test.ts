// A1: a graph save never overwrites a newer save made elsewhere. The daemon's
// 409 asks: reload theirs, overwrite with mine, or cancel.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

function fixture(choice: string | null) {
  const patches: Record<string, unknown>[] = [];
  const { workflowsUi } = loadSource(new URL('../src/modules/workflows/workflowsUiState.svelte.ts', import.meta.url), {
    '../../lib/api/client': { ApiError, api: { get: async () => ({ id: 'wf', version: 3, graph: 'theirs' }) } },
    '../../lib/api/workflows': {
      patchWorkflow: async (_id: string, body: Record<string, unknown>) => {
        patches.push(body);
        if (body.expected_version !== undefined) throw new ApiError(409, 'conflict: workflow changed elsewhere');
        return { id: 'wf', version: 4, graph: body.graph };
      },
    },
    '../../lib/confirm.svelte': { confirmer: { choose: async () => ({ value: choice, remember: false }) } },
    '../../lib/lastSelection': { rememberSelection() {} },
    '../../lib/toast.svelte': { toasts: { success() {} } },
    '../../lib/toastError': { toastError() {} },
  });
  const saved: unknown[] = [];
  const reloaded: unknown[] = [];
  const save = () => workflowsUi.saveGraph('wf', 'mine', 2, (w: unknown) => saved.push(w), (w: unknown) => reloaded.push(w));
  return { save, patches, saved, reloaded };
}

test('a stale save sends its base version and, on Reload, saves nothing and hands back theirs', async () => {
  const f = fixture('reload');
  assert.equal(await f.save(), false);
  assert.equal(JSON.stringify(f.patches), JSON.stringify([{ graph: 'mine', expected_version: 2 }]));
  assert.equal(f.saved.length, 0);
  assert.equal(JSON.stringify(f.reloaded), JSON.stringify([{ id: 'wf', version: 3, graph: 'theirs' }]));
});

test('Overwrite saves without the version check; Cancel saves nothing', async () => {
  const over = fixture('overwrite');
  assert.equal(await over.save(), true);
  assert.equal(JSON.stringify(over.patches.at(-1)), JSON.stringify({ graph: 'mine' }));
  assert.equal(over.saved.length, 1);
  const cancel = fixture(null);
  assert.equal(await cancel.save(), false);
  assert.equal(cancel.saved.length + cancel.reloaded.length, 0);
});

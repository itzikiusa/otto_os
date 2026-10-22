// ui-correctness-4-05: one failed progress poll used to end the run's
// tracking — "Couldn’t run automation", running=false, and the partial report
// (passed:false) read "Last run failed" while the run went on server-side.
// ux-design-4-03: a failed automations / run-history / cookies load set an
// inline error (no toast) instead of leaving an empty list.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { strictRequire } from './strictRequire.ts';
import * as loadError from '../src/lib/loadError.ts';
import { HistoryRefresh, HistoryDetail } from '../src/lib/stores/apiHistory.ts';
import * as scriptRuntime from '../src/lib/api/scripts.ts';
import * as graphqlVars from '../src/modules/api/graphqlVars.ts';
import * as secretShapes from '../src/lib/api/apiSecretShapes.ts';

function setup(api: Record<string, any>) {
  const notices: string[] = [];
  api.long = { post: (...a: unknown[]) => api.post(...a), get: (...a: unknown[]) => api.get(...a) };
  // Every wait (poll fallback, retry backoff) runs on the next macrotask.
  const fastTimeout = (fn: () => void) => setTimeout(fn, 0);
  const context = {
    exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, { snapshot: (v: unknown) => v, raw: (v: unknown) => v }), $derived: (v: unknown) => v,
    crypto: { randomUUID }, URL, URLSearchParams, AbortController, DOMException, setTimeout: fastTimeout, clearTimeout, performance, encodeURIComponent,
    localStorage: { getItem() { return null; }, setItem() {} },
    require: strictRequire([
      ['/api/client', { api, isAbortError: () => false, newHostConfirmHost: () => null }],
      ['/workspace.svelte', { ws: { currentId: 'A' } }],
      ['/toast.svelte', { toasts: { error: (m: string) => { notices.push(m); }, success() {}, info() {} } }],
      ['/confirm.svelte', { confirmer: {} }],
      ['/apiHistory', { HistoryRefresh, HistoryDetail }],
      ['/apiSecretShapes', secretShapes],
      ['/scriptRunner', { runScript: async () => ({}) }],
      ['/graphqlVars', graphqlVars],
      ['/lazyModule', { announceModule() {} }],
      ['/plural', { plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}` }],
      ['/api/scripts', scriptRuntime],
      ['/importers', { isImportedEnvironment: () => false }],
      ['/api/types', { isSecretRef: (v: any) => !!v?.$secret }],
      ['/loadError', loadError],
    ]),
  };
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/lib/stores/apiClient.svelte.ts', import.meta.url), 'utf8'),
    { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return { v: context.exports.apiClient, notices };
}

const runRow = (status: string, steps: number, passed = false) => ({
  id: 'run1', automation_id: 'au1', status, error: null, snapshot: {}, result_rows: [], result_ids: [],
  report: { automation_id: 'au1', passed, steps: Array.from({ length: steps }, () => ({ ok: true })) },
});

test('a transient poll failure keeps following the run to its real end', async () => {
  let polls = 0;
  const { v, notices } = setup({
    post: async () => runRow('running', 0),
    get: async (path: string) => {
      if (path.includes('automation-runs/run1?after=')) {
        polls++;
        if (polls <= 2) throw new TypeError('Failed to fetch');
        return runRow('passed', 2, true);
      }
      return [];
    },
  });
  const report = await v.runAutomation('au1');
  assert.equal(report?.passed, true, 'the run is followed to its terminal status');
  assert.deepEqual(notices, [], 'no "Couldn’t run automation" for a blip');
  assert.equal(v.runLost, null);
  assert.equal(v.running, false);
});

test('repeated poll failures give up as "lost track", not a failed run', async () => {
  let runsReloaded = 0;
  const { v, notices } = setup({
    post: async () => runRow('running', 0),
    get: async (path: string) => {
      if (path.includes('?after=')) throw new TypeError('Failed to fetch');
      if (path.includes('automation-runs?')) { runsReloaded++; return []; }
      return [];
    },
  });
  assert.equal(await v.runAutomation('au1'), null);
  assert.equal(v.runLost, 'run1');
  assert.deepEqual(notices, ['Lost track of this run']);
  assert.equal(runsReloaded, 1, 'run history is reloaded so the real status shows when it lands');
  assert.equal(v.running, false);
});

test('failed list loads are inline errors, not toasts or empty lists', async () => {
  const boom = Object.assign(new Error('nope'), { status: 500 });
  const { v, notices } = setup({ get: async () => { throw boom; } });
  v.workspaceId = 'A';
  await v.loadCookies();
  await v.loadAutomationRuns('au1');
  assert.equal(v.cookiesError, 'nope');
  assert.equal(v.runsError, 'nope');
  assert.equal(v.cookiesLoading, false);
  assert.equal(v.runsLoading, false);
  assert.deepEqual(notices, []);
});

// architecture-2-03: the workspace `settings.api_client` keys are read and
// written by the apiClient store, not the shell's ws store. Getters parse the
// untyped JSON defensively; setters send one atomic `api_client` key patch.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { strictRequire, unused } from './strictRequire.ts';
import * as loadError from '../src/lib/loadError.ts';
import { HistoryRefresh, HistoryDetail } from '../src/lib/stores/apiHistory.ts';
import * as scriptRuntime from '../src/lib/api/scripts.ts';
import * as graphqlVars from '../src/modules/api/graphqlVars.ts';
import * as secretShapes from '../src/lib/api/apiSecretShapes.ts';

function setup(settings: unknown) {
  const patches: { wsId: string; patch: unknown }[] = [];
  const ws: Record<string, any> = {
    currentId: 'A',
    current: { id: 'A', settings },
    patchSettings: async (wsId: string, patch: unknown) => { patches.push({ wsId, patch }); },
  };
  const context = {
    exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, { snapshot: (v: unknown) => v, raw: (v: unknown) => v }), $derived: (v: unknown) => v,
    crypto: { randomUUID }, URL, URLSearchParams, AbortController, setTimeout, clearTimeout,
    localStorage: { getItem() { return null; }, setItem() {} },
    require: strictRequire([
      ['/api/client', { api: {}, isAbortError: () => false, newHostConfirmHost: () => null }],
      ['/workspace.svelte', { ws }],
      ['/toast.svelte', unused('toast')],
      ['/confirm.svelte', unused('confirm')],
      ['/apiHistory', { HistoryRefresh, HistoryDetail }],
      ['/apiSecretShapes', secretShapes],
      ['/scriptRunner', { runScript: async () => ({}) }],
      ['/graphqlVars', graphqlVars],
      ['/lazyModule', { announceModule() {} }],
      ['/plural', { plural: (n: number, w: string) => `${n} ${w}` }],
      ['/api/scripts', scriptRuntime],
      ['/importers', {}],
      ['/api/types', { isSecretRef: (v: any) => !!v?.$secret }],
      ['/loadError', loadError],
    ]),
  };
  const src = readFileSync(new URL('../src/lib/stores/apiClient.svelte.ts', import.meta.url), 'utf8');
  runInNewContext(ts.transpileModule(src, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return { v: context.exports.apiClient, ws, patches };
}

test('getters read settings.api_client and fall back to the opt-in defaults', () => {
  const { v } = setup({ api_client: { allow_local: true, history_max_rows: 200, history_max_days: 7, automation_runs_keep: 5 } });
  assert.equal(v.allowLocal, true);
  assert.deepEqual({ ...v.historyRetention }, { rows: 200, days: 7 });
  assert.equal(v.runsKeep, 5);
  // Malformed or missing values mean "no limit" / off, never a crash.
  const bad = setup({ api_client: { allow_local: 'yes', history_max_rows: -1, history_max_days: 1.5, automation_runs_keep: '3' } }).v;
  assert.equal(bad.allowLocal, false);
  assert.deepEqual({ ...bad.historyRetention }, { rows: 0, days: 0 });
  assert.equal(bad.runsKeep, 0);
  assert.equal(setup({}).v.allowLocal, false);
});

test('setters send one atomic api_client key patch for the current workspace', async () => {
  const { v, patches } = setup({ api_client: { allow_local: false } });
  await v.setAllowLocal(true);
  await v.setHistoryRetention(0, 7);
  await v.setRunsKeep(5);
  assert.deepEqual(JSON.parse(JSON.stringify(patches)), [
    { wsId: 'A', patch: { api_client: { allow_local: true } } },
    { wsId: 'A', patch: { api_client: { history_max_rows: 0, history_max_days: 7 } } },
    { wsId: 'A', patch: { api_client: { automation_runs_keep: 5 } } },
  ]);
});

test('no workspace selected: setters write nothing', async () => {
  const { v, ws, patches } = setup({});
  ws.currentId = null;
  ws.current = null;
  await v.setAllowLocal(true);
  assert.equal(patches.length, 0);
});

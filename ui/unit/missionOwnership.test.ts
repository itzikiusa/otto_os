import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

// Actual component methods with deferred HTTP: asynchronous ownership, not DOM.
function setup() {
  const source = readFileSync(new URL('../src/modules/agents/MissionControl.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('mission.ts', source, ts.ScriptTarget.Latest, true);
  const methods = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && ['load', 'createView'].includes(node.name?.text ?? '')).map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  const requests: { path: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const request = (path: string) => { const result = deferred<any>(); requests.push({ path, result }); return result.promise; };
  const compiled = ts.transpileModule(`
    let wsId = 'A', view = null, savedViews = [], loading = false;
    let newViewName = 'A view', newViewFilter = '{}', advancedFilter = false;
    let filterBucket = '', filterProvider = '', filterRepo = '', showNewViewForm = true;
    let loadGeneration = 0, alive = true, loadError = '';
    ${methods}
    return { load, createView, switchTo(id) { wsId = id; newViewName = id + ' view'; },
      read() { return { view, savedViews, loading, newViewName, showNewViewForm }; } };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  const state = new Function('api', 'activity', 'toasts', compiled)({ get: request, post: request }, { loadSummary: async () => {} }, { error() {} });
  return { state, requests };
}

test('old Mission load cannot replace a newer workspace or its loading state', async () => {
  const { state, requests } = setup();
  const old = state.load(); state.switchTo('B'); const current = state.load();
  requests[0].result.resolve({ working: [{ id: 'A' }] }); requests[1].result.resolve([{ id: 'A-view' }]);
  await old;
  assert.equal(state.read().loading, true);
  requests[2].result.resolve({ working: [{ id: 'B' }] }); requests[3].result.resolve([{ id: 'B-view' }]);
  await current;
  assert.equal(state.read().view.working[0].id, 'B');
  assert.equal(state.read().savedViews[0].id, 'B-view');
});

test('late saved-view creation does not clear another workspace form or refresh it', async () => {
  const { state, requests } = setup();
  const saving = state.createView(); state.switchTo('B');
  requests[0].result.resolve({ id: 'A-view' });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(state.read().newViewName, 'B view');
  assert.equal(state.read().showNewViewForm, true);
  assert.equal(requests.length, 1);
  await saving;
});

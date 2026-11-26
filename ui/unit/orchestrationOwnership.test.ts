import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { loadErrorText } from '../src/lib/loadError.ts';
import { loadSource } from './sourceHarness.ts';

function methods(file: string, names: string[]) {
  const script = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const ast = ts.createSourceFile('component.ts', script, ts.ScriptTarget.Latest, true);
  return ast.statements.filter((n) => ts.isFunctionDeclaration(n) && names.includes(n.name?.text ?? ''))
    .map((n) => script.slice(n.getStart(ast), n.end).replace(/^export /, '')).join('\n');
}
function compile(body: string, bindings: Record<string, unknown>) {
  const js = ts.transpileModule(body, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  return new Function(...Object.keys(bindings), js)(...Object.values(bindings));
}
const toast = { error() {}, success() {} };
const flush = () => new Promise((resolve) => setImmediate(resolve));
test('replacement import fetches global scope, confirms empty visible workspace, and fails closed', async () => {
  const prompts: string[] = [], imports: unknown[] = [];
  let reject = false, approve = false;
  const state = compile(`let importing=false, importReplace=true, importText='[]', importOpen=true, policies=[];
    ${methods('modules/mcp/PoliciesTab.svelte', ['doImport'])} return {doImport};`, {
    mcpCpApi: { cpPolicies: async (...args: unknown[]) => { assert.equal(args.length, 0); if (reject) throw Error('offline'); return [{workspace_id:'B'}, {workspace_id:null}]; },
      cpImportPolicies: async (req: unknown) => { imports.push(req); return { imported: 0 }; } },
    confirmer: { ask: async (text: string) => { prompts.push(text); return approve; } }, toasts: toast, toastError: (title: string) => (toast.error as (t: string) => void)(title), load: async () => {},
  });
  await state.doImport(); assert.match(prompts[0], /2 existing rules across every workspace/); assert.equal(imports.length, 0);
  reject = true; approve = true; await state.doImport(); assert.equal(imports.length, 0);
  reject = false; await state.doImport(); assert.equal(imports.length, 1);
});
test('mounted tool catalogue batches 1000 decisions and releases refresh work', async () => {
  let batches = 0, singles = 0;
  const { resourceAccess: store } = loadSource(new URL('../src/lib/stores/resource-access.svelte.ts', import.meta.url), {
    svelte: {untrack: (fn: () => unknown) => fn()}, '../api/access': {accessApi: {
      capabilitiesBatch: async (_kind: string, _id: string, children: string[]) => { batches++; return children.map((child) => ({child,mode:'enforced',operations:{invoke:{allowed:true}}})); },
      capabilities: async () => { singles++; },
    }}, '../api/client': {getToken: () => null, ApiError: class extends Error {}}, './auth.svelte': {auth: {me:{id:'user'},can:()=>true}},
    '../poll': {createLimiter: () => (fn: () => unknown) => fn(), mapLimit: async (items: unknown[], _limit: number, fn: (x: unknown) => unknown) => Promise.all(items.map(fn))},
    '../live': {appLive:{connected:()=>true},liveQuery(){},LIVE_SAFETY_MS:300000},
  });
  const release = store.retainChildren('mcp_server', 'server', Array.from({length:1000}, (_,i)=>`tool-${i}`)); await flush();
  assert.equal(batches, 1); assert.equal(store.can('mcp_server','server','invoke','mcp','edit','tool-999'), true);
  await store.refresh(); assert.equal(batches, 2); assert.equal(singles, 0);
  release(); await store.refresh(); assert.equal(batches, 2); assert.equal(store.get('mcp_server','server','tool-999'), null);
});
test('top-level capability checks coalesce into one batch request per kind', async () => {
  class ApiError extends Error { status: number; constructor(status: number) { super(String(status)); this.status = status; } }
  const calls: string[][] = [];
  let singles = 0, missingRoute = false;
  const decision = (id: string) => ({ kind: 'connection', resource_id: id, child: null, mode: 'enforced', operations: { discover: { allowed: true } } });
  const { resourceAccess: store } = loadSource(new URL('../src/lib/stores/resource-access.svelte.ts', import.meta.url), {
    svelte: {untrack: (fn: () => unknown) => fn()}, '../api/access': {accessApi: {
      capabilitiesMany: async (_kind: string, ids: string[]) => {
        if (missingRoute) throw new ApiError(404);
        calls.push(ids);
        return ids.map((id) => (id === 'hidden' ? null : decision(id)));
      },
      capabilities: async (_kind: string, id: string) => { singles++; return decision(id); },
    }}, '../api/client': {getToken: () => null, ApiError}, './auth.svelte': {auth: {me:{id:'user'},can:()=>true}},
    '../poll': {createLimiter: () => (fn: () => unknown) => fn(), mapLimit: async (items: unknown[], _limit: number, fn: (x: unknown) => unknown) => Promise.all(items.map(fn))},
    '../live': {appLive:{connected:()=>true},liveQuery(){},LIVE_SAFETY_MS:300000},
  });
  const ids = [...Array.from({length: 99}, (_, i) => `c${i}`), 'hidden'];
  await Promise.all(ids.map((id) => store.load('connection', id)));
  assert.deepEqual(calls.map((c) => c.length), [100], 'one request for the whole page of rows');
  assert.equal(singles, 0);
  assert.equal(store.can('connection', 'c42', 'discover', 'database', 'view'), true);
  assert.equal(store.get('connection', 'hidden'), null, 'an invisible resource stays denied');
  // The 5-minute safety net re-checks every cached row in one request too.
  await store.refresh();
  assert.deepEqual(calls.map((c) => c.length), [100, 100]);
  assert.equal(singles, 0);
  // A daemon without the route still answers, one resource at a time.
  missingRoute = true;
  await store.refresh();
  assert.equal(singles, 100);
});

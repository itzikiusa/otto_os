import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { deferred, loadSource } from './sourceHarness.ts';

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
function workflow() {
  const validation = deferred<boolean>(), saving = deferred<void>(), confirmation = deferred<boolean>();
  const posts: { path: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const histories: { id: string; result: ReturnType<typeof deferred<any>> }[] = [];
  const restorations: string[] = [];
  const state = compile(`
    let current = { id: 'A' }, running = false, dirty = true, destroyed = false;
    let ws = {currentId: 'ws'}, viewGeneration = 0, versionsGeneration = 0;
    let versions = [], versionsLoading = false, versionsError = null, workflows = [];
    let run = null, requestedRunId = null;
    async function save() { await saving.promise; dirty = false; }
    function open(wf) { current = wf; viewGeneration++; }
    ${methods('modules/workflows/WorkflowsPage.svelte', ['startRun', 'loadVersions', 'restoreVersion'])}
    return {startRun, loadVersions, restoreVersion, switchTo(id) { current = id ? {id} : null; dirty = false; viewGeneration++; },
      read() { return {run, running, versions, versionsLoading}; }};
  `, { saving, validateGraph: () => validation.promise, discardEditsOk: () => confirmation.promise, toasts: toast, loadErrorText: String,
    api: { post(path: string) { const result = deferred<any>(); posts.push({ path, result }); return result.promise; } },
    listWorkflowVersions(id: string) { const result = deferred<any>(); histories.push({ id, result }); return result.promise; },
    restoreWorkflowVersion: async (id: string) => { restorations.push(id); return { id }; },
  });
  return { state, validation, saving, confirmation, posts, histories, restorations };
}
for (const target of ['B', null]) test(`pending workflow save cannot retarget Run to ${target}`, async () => {
  const h = workflow(); const started = h.state.startRun({});
  h.validation.resolve(true); await flush(); h.state.switchTo(target); h.saving.resolve();
  assert.equal(await started, null); assert.equal(h.posts.length, 0);
});
test('Run admission covers validation and releases after validation failure', async () => {
  const h = workflow(); const first = h.state.startRun({});
  assert.equal(await h.state.startRun({}), null);
  h.validation.resolve(false); assert.equal(await first, null); assert.equal(h.state.read().running, false);
});
test('late Run response cannot install under a different workflow', async () => {
  const h = workflow(); const started = h.state.startRun({}); h.validation.resolve(true); h.saving.resolve(); await flush();
  assert.equal(h.posts[0].path, '/workflows/A/run'); h.state.switchTo('B'); h.posts[0].result.resolve({ id: 'run-A' }); await started;
  assert.equal(h.state.read().run, null);
});
test('late version read cannot replace current drawer or its loading state', async () => {
  const h = workflow(); const a = h.state.loadVersions(); h.state.switchTo('B'); const b = h.state.loadVersions();
  h.histories[0].result.resolve([{ workflow_id: 'A' }]); await a;
  assert.equal(h.state.read().versionsLoading, true);
  h.histories[1].result.resolve([{ workflow_id: 'B' }]); await b;
  assert.equal(h.state.read().versions[0].workflow_id, 'B');
});
test('Restore captures version identity before discard confirmation', async () => {
  const h = workflow(); const restore = h.state.restoreVersion({ workflow_id: 'A', version: 2 });
  h.state.switchTo('B'); h.confirmation.resolve(true); await restore;
  assert.equal(h.restorations.length, 0);
  await h.state.restoreVersion({ workflow_id: 'A', version: 2 }); assert.equal(h.restorations.length, 0);
});
test('replacement import fetches global scope, confirms empty visible workspace, and fails closed', async () => {
  const prompts: string[] = [], imports: unknown[] = [];
  let reject = false, approve = false;
  const state = compile(`let importing=false, importReplace=true, importText='[]', importOpen=true, policies=[];
    ${methods('modules/mcp/PoliciesTab.svelte', ['doImport'])} return {doImport};`, {
    mcpCpApi: { cpPolicies: async (...args: unknown[]) => { assert.equal(args.length, 0); if (reject) throw Error('offline'); return [{workspace_id:'B'}, {workspace_id:null}]; },
      cpImportPolicies: async (req: unknown) => { imports.push(req); return { imported: 0 }; } },
    confirmer: { ask: async (text: string) => { prompts.push(text); return approve; } }, toasts: toast, load: async () => {},
  });
  await state.doImport(); assert.match(prompts[0], /2 existing rules across every workspace/); assert.equal(imports.length, 0);
  reject = true; approve = true; await state.doImport(); assert.equal(imports.length, 0);
  reject = false; await state.doImport(); assert.equal(imports.length, 1);
});
test('extended budget survives Resume failure and retry skips repeat PATCH', async () => {
  let updates = 0, resumes = 0;
  const state = compile(`let loop={iterations_started:1,limits:{}}, id='loop', acting=false, extIters=5, extMinutes=10;
    let extError='', savedExtension='', extendOpen=true;
    ${methods('modules/loops/LoopDetail.svelte', ['extendAndResume'])}
    return {extendAndResume, read(){return {extError,extendOpen,acting}}};`, {
    loops: { updateLimits: async () => { updates++; }, resume: async () => { if (++resumes === 1) throw Error('offline'); } },
    elapsedSecs: () => 1, formatSeconds: String, errText: String, toasts: toast,
  });
  await state.extendAndResume(); assert.match(state.read().extError, /Budget saved/); assert.equal(state.read().extendOpen, true);
  await state.extendAndResume(); assert.equal(updates, 1); assert.equal(resumes, 2); assert.equal(state.read().extendOpen, false);
});
test('dirty work item selection retains draft and route until discard is accepted', async () => {
  let discard = false; const routes: string[] = [];
  const state = compile(`let detail={goal:'old',result_summary:'',risk_level:'low'}, editing=true, editGoal='draft', editResult='',editRisk='low',busy=false;
    let selectedId='A', userClosed=false, selectionGeneration=0;
    ${methods('modules/mission-control/WorkItemDetail.svelte', ['isDirty', 'canLeave'])}
    const detailPane={canLeave};
    ${methods('modules/mission-control/MissionControlPage.svelte', ['select'])}
    return {select, read(){return {selectedId,editGoal,editing}}};`, {
    confirmer: {ask: async () => discard}, rememberSelection() {}, router: {module:'mission-control',replace:(path: string) => routes.push(path)},
  });
  await state.select('B'); assert.equal(state.read().selectedId, 'A'); assert.equal(state.read().editGoal, 'draft'); assert.equal(routes.length, 0);
  discard = true; await state.select('B'); assert.equal(state.read().selectedId, 'B'); assert.deepEqual(routes, ['mission-control/B']);
});
test('mounted tool catalogue batches 1000 decisions and releases refresh work', async () => {
  let batches = 0, singles = 0;
  const { resourceAccess: store } = loadSource(new URL('../src/lib/stores/resource-access.svelte.ts', import.meta.url), {
    svelte: {untrack: (fn: () => unknown) => fn()}, '../api/access': {accessApi: {
      capabilitiesBatch: async (_kind: string, _id: string, children: string[]) => { batches++; return children.map((child) => ({child,mode:'enforced',operations:{invoke:{allowed:true}}})); },
      capabilities: async () => { singles++; },
    }}, '../api/client': {getToken: () => null}, './auth.svelte': {auth: {me:{id:'user'},can:()=>true}},
    '../poll': {createLimiter: () => (fn: () => unknown) => fn(), mapLimit: async (items: unknown[], _limit: number, fn: (x: unknown) => unknown) => Promise.all(items.map(fn))},
    '../live': {appLive:{connected:()=>true},liveQuery(){},LIVE_SAFETY_MS:300000},
  });
  const release = store.retainChildren('mcp_server', 'server', Array.from({length:1000}, (_,i)=>`tool-${i}`)); await flush();
  assert.equal(batches, 1); assert.equal(store.can('mcp_server','server','invoke','mcp','edit','tool-999'), true);
  await store.refresh(); assert.equal(batches, 2); assert.equal(singles, 0);
  release(); await store.refresh(); assert.equal(batches, 2); assert.equal(store.get('mcp_server','server','tool-999'), null);
});

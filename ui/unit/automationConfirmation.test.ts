import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

// Execute the actual confirmation continuations with a route change while the
// sheet is open. HTTP transports record only target identity; no actions run.
function fixture(component: string, method: string) {
  const source = readFileSync(new URL(`../src/modules/${component}.svelte`, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('confirmation.ts', source, ts.ScriptTarget.Latest, true);
  const methods = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && [method, 'act', 'errText'].includes(node.name?.text ?? '')).map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  const confirmation = deferred<boolean>(); const targets: string[] = [];
  const compiled = ts.transpileModule(`
    let id = 'A', loop = { name: 'Loop A' }, run = { id: 'A', title: 'Run A' };
    let acting = false, busy = false, deciding = null, error = '', alive = true, prDraft = null;
    ${methods}
    return { invoke: ${method}, switchToB() { id = 'B'; loop = { name: 'Loop B' }; run = { id: 'B', title: 'Run B' }; } };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  const record = async (id: string) => { targets.push(id); };
  const state = new Function('confirmer', 'loops', 'runWithOtto', 'toasts', 'onback', compiled)(
    { ask: () => confirmation.promise }, { remove: record, stop: record }, { cancel: record, openPr: record },
    { error() {}, success() {} }, () => {},
  );
  return { state, confirmation, targets };
}
for (const [component, method] of [
  ['loops/LoopDetail', 'del'], ['loops/LoopDetail', 'stop'],
  ['run-with-otto/RunDetail', 'cancel'], ['run-with-otto/RunDetail', 'openPr'],
]) {
  test(`${component} ${method} acts on the target named by its confirmation`, async () => {
    const { state, confirmation, targets } = fixture(component, method);
    const pending = state.invoke(); state.switchToB(); confirmation.resolve(true); await pending;
    assert.deepEqual(targets, ['A']);
  });
}

function pageFixture(component: string, method: string) {
  const source = readFileSync(new URL(`../src/modules/${component}.svelte`, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('confirmation.ts', source, ts.ScriptTarget.Latest, true);
  const body = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && node.name?.text === method).map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  const confirmation = deferred<any>(); const targets: string[] = []; const followups: string[] = [];
  const compiled = ts.transpileModule(`
    let detail = { pack: { id: 'A', title: 'Pack A' }, artifacts: [] }, run = { id: 'A' }, current = { name: 'Workflow A' };
    let ws = { currentId: 'workspace-A' }, filter = {}, assembling = false;
    ${body}
    return { invoke: ${method}, switchToB() { detail = { pack: { id: 'B', title: 'Pack B' }, artifacts: [] }; run = { id: 'B' }; ws.currentId = 'workspace-B'; } };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  const record = async (id: string) => { targets.push(id); return { id: 'created-A' }; };
  const state = new Function('confirmer', 'deleteProofPack', 'assembleProof', 'createProofPack', 'api', 'proof', 'toasts', 'plural', 'loadList', 'viewport', 'open', 'loadErrorText', 'toastError', 'packRepoPath', compiled)(
    { ask: () => confirmation.promise, promptText: () => confirmation.promise }, record, record, record,
    { post: record }, { closeDetail: () => followups.push('close'), refreshDetail: async () => followups.push('refresh'), packs: [] },
    { error() {}, success() {}, info() {}, warn() {} }, (n: number) => String(n), async (id: string) => followups.push(id), { isPhone: false }, async (id: string) => followups.push(id), String, () => {}, async () => '/tmp/repo-A',
  );
  return { state, confirmation, targets, followups };
}
for (const [component, method, answer, target] of [
  ['proof/ProofPage', 'removePack', true, 'A'],
  ['proof/ProofPage', 'assemble', '/tmp/repo-A', 'A'],
  ['proof/ProofPage', 'newPack', 'New pack', 'workspace-A'],
  ['workflows/WorkflowsPage', 'stop', true, '/workflow-runs/A/cancel'],
] as const) {
  test(`${component} ${method} keeps its confirmed target after navigation`, async () => {
    const { state, confirmation, targets, followups } = pageFixture(component, method);
    const pending = state.invoke(); await Promise.resolve(); state.switchToB(); confirmation.resolve(answer); await pending;
    assert.deepEqual(targets, [target]);
    assert.deepEqual(followups, [], 'old action must not close or reload the new selection');
  });
}

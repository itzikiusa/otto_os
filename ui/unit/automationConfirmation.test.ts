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
    let id = 'A', loop = { name: 'Loop A' };
    let acting = false, busy = false, deciding = null, error = '', alive = true;
    ${methods}
    return { invoke: ${method}, switchToB() { id = 'B'; loop = { name: 'Loop B' }; } };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  const record = async (id: string) => { targets.push(id); };
  const state = new Function('confirmer', 'loops', 'toasts', 'onback', compiled)(
    { ask: () => confirmation.promise }, { remove: record, stop: record },
    { error() {}, success() {} }, () => {},
  );
  return { state, confirmation, targets };
}
for (const [component, method] of [
  ['loops/LoopDetail', 'del'], ['loops/LoopDetail', 'stop'],
]) {
  test(`${component} ${method} acts on the target named by its confirmation`, async () => {
    const { state, confirmation, targets } = fixture(component, method);
    const pending = state.invoke(); state.switchToB(); confirmation.resolve(true); await pending;
    assert.deepEqual(targets, ['A']);
  });
}

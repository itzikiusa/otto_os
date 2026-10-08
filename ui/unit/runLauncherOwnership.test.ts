import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';
import { latestOnly } from '../src/lib/latest.ts';

function fixture() {
  const source = readFileSync(new URL('../src/modules/run-with-otto/RunLauncher.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('launcher.ts', source, ts.ScriptTarget.Latest, true);
  const methods = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && ['runDetect', 'launch'].includes(node.name?.text ?? '')).map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  const requests: ReturnType<typeof deferred<any>>[] = []; const launched: string[] = [];
  const launchResult = deferred<any>();
  const compiled = ts.transpileModule(`
    let query = 'ABC-1', wsId = 'A', detected = null, detecting = false, detectFailed = false;
    let busy = false, error = '', alive = true, debounceTimer = null, detectAbort = null;
    const detectSeq = latestOnly();
    let mode = 'single_agent', effectiveProvider = 'codex', model = '', repoId = '', repos = [];
    ${methods}
    return { runDetect, launch, dispose() { alive = false; detectSeq.cancel(); detectAbort?.abort(); },
      read() { return { detected, detecting, detectFailed, query }; } };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
  const state = new Function('latestOnly', 'runWithOttoApi', 'runWithOtto', 'onLaunched', compiled)(
    latestOnly, { detect() { const result = deferred<any>(); requests.push(result); return result.promise; } },
    { launch: () => launchResult.promise }, (run: {id: string}) => launched.push(run.id),
  );
  return { state, requests, launchResult, launched };
}

test('older same-query detection cannot overwrite a newer result', async () => {
  const { state, requests } = fixture();
  const old = state.runDetect('ABC-1'), current = state.runDetect('ABC-1');
  requests[1].resolve({ detected: { source_ref: 'new' } }); await current;
  requests[0].resolve({ detected: { source_ref: 'old' } }); await old;
  assert.equal(state.read().detected.source_ref, 'new');
});

test('completed launch in an unmounted workspace cannot reopen its detail', async () => {
  const { state, launchResult, launched } = fixture();
  const pending = state.launch(); state.dispose(); launchResult.resolve({ id: 'old-run' }); await pending;
  assert.deepEqual(launched, []);
});

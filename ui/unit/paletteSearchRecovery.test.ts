import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
import {deferred} from './sourceHarness.ts';
function setup() {
  const text = readFileSync(new URL('../src/shell/Palette.svelte', import.meta.url), 'utf8');
  const source = text.slice(text.indexOf('>') + 1, text.indexOf('</script>'));
  const ast = ts.createSourceFile('palette.ts', source, ts.ScriptTarget.Latest, true);
  const fn = ast.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'doSearch')!;
  const requests: ReturnType<typeof deferred<any[]>>[] = [];
  const context: Record<string, any> = {searchHits: [], searchError: null, searchBusy: false, searchAbort: null,
    api: {get: () => {const r = deferred<any[]>(); requests.push(r); return r.promise;}},
    AbortController, isAbortError: (e: any) => e?.name === 'AbortError'};
  runInNewContext(ts.transpileModule(fn.getText(ast), {compilerOptions: {target: ts.ScriptTarget.ES2022}}).outputText, context);
  return {context, requests};
}
test('search error remains visible and a successful retry clears it', async () => {
  const {context: c, requests} = setup(); const first = c.doSearch('known', 'ws');
  requests[0].reject(new Error('offline')); await first;
  assert.match(c.searchError, /offline/); assert.equal(c.searchBusy, false);
  const retry = c.doSearch('known', 'ws'); requests[1].resolve([{id: 'found'}]); await retry;
  assert.equal(c.searchError, null); assert.equal(c.searchHits[0].id, 'found');
});
test('superseded search cannot overwrite a newer result or raise its error', async () => {
  const {context: c, requests} = setup(); const old = c.doSearch('old', 'a');
  c.searchAbort.abort(); const current = c.doSearch('new', 'b');
  requests[1].resolve([{id: 'new'}]); await current; requests[0].reject(new Error('old failure')); await old;
  assert.equal(c.searchError, null); assert.equal(c.searchHits[0].id, 'new');
});

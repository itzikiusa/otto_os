import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

function fixture(overrides: Record<string, any> = {}) {
  const source = readFileSync(new URL('../src/modules/workbench/WorkbenchPage.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('page.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const body = file.statements.filter(ts.isFunctionDeclaration).map((n) => n.getText(file)).join('\n');
  const edits: any[][] = [];
  const active = { id: 'A', buffer: '{"a":1}' };
  const c: Record<string, any> = { active, doc: { id: 'A' }, wsId: 'workspace-A', lang: 'json', isImage: false, formatting: false, issue: null,
    issueFromFormat: false, cursor: 0, canFormat: () => true, formatContent: async () => ({}),
    workbench: { setBuffer: (...args: any[]) => edits.push(args), reload: (...args: any[]) => edits.push(args), create: (...args: any[]) => edits.push(args) },
    confirmer: { ask: async () => true }, toasts: { info() {}, error() {} }, loadErrorText: String,
    ...overrides };
  runInNewContext(ts.transpileModule(body, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, c);
  return { c, edits };
}

test('format completion cannot replace the newly selected file or later keystrokes', async () => {
  for (const switchDoc of [true, false]) {
    const pending = deferred<any>(); const { c, edits } = fixture({ formatContent: () => pending.promise });
    const formatting = c.format();
    if (switchDoc) { c.active = { id: 'B', buffer: 'other' }; c.doc = { id: 'B' }; }
    else c.active.buffer = 'new typing';
    pending.resolve({ ok: true, changed: true, text: 'old formatted text' }); await formatting;
    assert.equal(edits.length, 0);
  }
});

test('reload confirmation cannot discard a newly selected file', async () => {
  const pending = deferred<boolean>(); const { c, edits } = fixture({ confirmer: { ask: () => pending.promise } });
  const reloading = c.reloadRemote(); c.doc = { id: 'B' }; c.active = { id: 'B', buffer: 'keep' };
  pending.resolve(true); await reloading; assert.equal(edits.length, 0);
});

test('image upload cannot insert an old workspace asset into a different workspace', async () => {
  const pending = deferred<any>(); const { c, edits } = fixture({ uploadWorkbenchAsset: () => pending.promise });
  const adding = c.addImages([{ name: 'a.png', type: 'image/png' }], true);
  c.wsId = 'workspace-B'; c.doc = { id: 'B' }; c.active = { id: 'B', buffer: 'keep' };
  pending.resolve({ id: 'asset-A' }); await adding; assert.equal(edits.length, 0);
});

test('unchanged editor still receives its format result', async () => {
  const { c, edits } = fixture({ formatContent: async () => ({ ok: true, changed: true, text: 'formatted' }) });
  await c.format(); assert.deepEqual(edits, [['A', 'formatted']]); assert.equal(c.formatting, false);
});

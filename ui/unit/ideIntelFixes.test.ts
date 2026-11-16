// IDE code intelligence: completion filtering/trigger/dedupe, the LSP attach set,
// and the deterministic quick fixes that used to fire on any identifier or import.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { javascript } from '@codemirror/lang-javascript';
import { EditorState } from '@codemirror/state';
import { filterLspCompletionItems, LSP_COMPLETION_CAP, lspFirstCompletionSource } from '../src/lib/components/cm-ide-lsp-complete-source.ts';
import {
  detectLspCompletionTrigger,
  registerIdeLspPluginLanguage,
  shouldAttachIdeLsp,
  type RawLspCompletionItem,
} from '../src/lib/components/cm-ide-lsp-completion.ts';

test('completion filters by the typed prefix before the cap, so Prom finds Promise past item 150', () => {
  // tsserver's shape: ~1060 items, locals first, then globals A→Z.
  const items: RawLspCompletionItem[] = Array.from({ length: 1062 }, (_, i) => ({ label: `g${String(i).padStart(4, '0')}` }));
  items[901] = { label: 'Promise' };
  items[902] = { label: 'PromiseRejectionEvent' };
  assert.equal(items.slice(0, LSP_COMPLETION_CAP).some((i) => i.label === 'Promise'), false);
  const got = filterLspCompletionItems(items, 'Prom');
  assert.deepEqual(got.items.map((i) => i.label), ['Promise', 'PromiseRejectionEvent']);
  assert.equal(got.truncated, false);
  // No prefix (after `.`): capped, and flagged so the list is asked again as the user types.
  const all = filterLspCompletionItems(items, '');
  assert.equal(all.items.length, LSP_COMPLETION_CAP);
  assert.equal(all.truncated, true);
});

test('an explicit completion session never falls back to the global list after `(`', () => {
  const doc = EditorState.create({ doc: 'w.deposit(' }).doc;
  assert.equal(detectLspCompletionTrigger(doc, doc.length, true, 'typescript'), null);
  const sp = EditorState.create({ doc: 'const a = ' }).doc;
  assert.deepEqual(detectLspCompletionTrigger(sp, sp.length, true, 'typescript'), { from: sp.length, context: { triggerKind: 1 } });
  const empty = EditorState.create({ doc: '' }).doc;
  assert.deepEqual(detectLspCompletionTrigger(empty, 0, true, 'typescript'), { from: 0, context: { triggerKind: 1 } });
});

test('with a ready server only its list is offered (no second `const`); before that the language package stands in', async () => {
  const state = EditorState.create({ doc: 'cons', extensions: [javascript({ typescript: true })] });
  const ctx = new CompletionContext(state, 4, true);
  const lspResult: CompletionResult = { from: 0, options: [{ label: 'const', type: 'keyword', detail: 'keyword' }] };
  const ready = await lspFirstCompletionSource(() => true, () => lspResult)(ctx);
  assert.deepEqual(ready?.options.map((o) => o.label), ['const']);
  const starting = await lspFirstCompletionSource(() => false, () => lspResult)(ctx);
  assert.ok(starting && starting !== lspResult && starting.options.some((o) => o.label === 'const'));
});

test('shell, CSS and HTML buffers attach their servers; a plugin language attaches once the daemon lists it', () => {
  assert.equal(shouldAttachIdeLsp('/ws/scripts/deploy.sh', 'shellscript', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/web/x.css', 'css', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/web/x.scss', 'scss', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/web/index.html', 'html', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/README.md', 'markdown', '/ws'), false);
  assert.equal(shouldAttachIdeLsp('/ws/src/Main.kt', 'kotlin', '/ws'), false);
  registerIdeLspPluginLanguage('kotlin');
  assert.equal(shouldAttachIdeLsp('/ws/src/Main.kt', 'kotlin', '/ws'), true);
});

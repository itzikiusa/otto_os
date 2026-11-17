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
import { computeDeterministicQuickFixes } from '../src/lib/components/cm-ide-quickfix.ts';

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

test('quick fixes: a used import gets no "Remove unused import", and "Prefix with _" needs an unused diagnostic', () => {
  const ts = "import { Wallet, formatMoney } from './money';\nimport { unused } from './x';\nconst w = new Wallet();\nconsole.log(formatMoney(w), label);\n";
  const ids = (line: number, symbol?: string, diags?: { line: number; message: string }[]) =>
    computeDeterministicQuickFixes(ts, '/ws/main.ts', 'typescript', line, symbol, diags);
  assert.equal(ids(1).some((f) => f.id.startsWith('ts-rm-imp')), false);
  const rm = ids(2).find((f) => f.id.startsWith('ts-rm-imp'));
  assert.ok(rm && !rm.isPreferred, 'an unused import is offered, never preferred');
  // ⌥⇧⏎ applies only a preferred fix: none here, so `label` is never renamed to `_label`.
  assert.equal(ids(4, 'label').some((f) => f.id.startsWith('ts-underscore')), false);
  assert.equal(ids(4, 'label').some((f) => f.isPreferred), false);
  assert.ok(ids(4, 'label', [{ line: 4, message: "'label' is declared but its value is never read." }]).some((f) => f.id.startsWith('ts-underscore')));

  const py = 'import os\nfrom typing import Any, Dict\nimport sys\nprint(os.getcwd(), Dict)\n';
  const pyIds = (line: number) => computeDeterministicQuickFixes(py, '/ws/a.py', 'python', line).map((f) => f.id);
  assert.equal(pyIds(1).some((id) => id.startsWith('py-rm-imp')), false);
  assert.equal(pyIds(2).some((id) => id.startsWith('py-rm-imp')), false);
  assert.ok(pyIds(3).some((id) => id.startsWith('py-rm-imp')));

  const rs = 'use std::sync::Arc;\nuse std::collections::{HashMap, BTreeMap};\nfn f() -> Arc<HashMap<u8, u8>> { todo!() }\n';
  const rsIds = (line: number) => computeDeterministicQuickFixes(rs, '/ws/lib.rs', 'rust', line).map((f) => f.id);
  assert.equal(rsIds(1).some((id) => id.startsWith('rs-rm-use')), false);
  assert.equal(rsIds(2).some((id) => id.startsWith('rs-rm-use')), false);
});

test('every Standard keymap row that differs from JetBrains is bound in the Standard preset', async () => {
  const { KEYMAP_ROWS } = await import('../src/modules/ide/settings/ideKeymapRows.ts');
  const { buildIdeKeymapSpecs } = await import('../src/lib/components/cm-ide-keymap.ts');
  const norm = (k: string) => k.split('-').map((p, i, a) => (i < a.length - 1 ? p : p.toLowerCase())).sort().join('-');
  const glyph = (g: string) => norm([...g].map((c) => ({ '⌘': 'Mod-', '⇧': 'Shift-', '⌥': 'Alt-', '⌃': 'Ctrl-' })[c] ?? c).join(''));
  const bound = new Set(buildIdeKeymapSpecs('standard', () => ({})).map((b) => norm(b.key ?? '')));
  const differing = KEYMAP_ROWS.filter((r) => r.standard !== r.jetbrains);
  assert.ok(differing.length >= 5);
  for (const r of differing) assert.ok(bound.has(glyph(r.standard)), `${r.action}: ${r.standard} is not bound`);
});

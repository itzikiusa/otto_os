import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState, type Transaction, type TransactionSpec } from '@codemirror/state';
import type { DecorationSet, EditorView, ViewPlugin, ViewUpdate } from '@codemirror/view';
import { createIdeSemanticExtensions, SEMANTIC_REFRESH_MS } from '../src/lib/components/cm-ide-semantic.ts';
import { cloneIdePreferences, DEFAULT_IDE_PREFERENCES } from '../src/lib/components/cm-ide-prefs.ts';

// The semantic plugin rebuilt its whole-file model (doc.toString, symbols, brackets, identifier
// counts) synchronously on every keystroke. Typing now shifts the marks and rebuilds once the
// keys pause; cursor moves and refreshes still compute immediately.
test('typing maps semantic marks and rebuilds the model once after the keys pause', async () => {
  let computes = 0;
  const prefs = cloneIdePreferences(DEFAULT_IDE_PREFERENCES);
  const ext = createIdeSemanticExtensions({ filePath: '/p/main.rs', getPreferences: () => { computes++; return prefs; } }) as unknown[];
  const plugin = ext[ext.length - 1] as ViewPlugin<{ decorations: DecorationSet; update(u: ViewUpdate): void; destroy(): void }> & { create(v: EditorView): { decorations: DecorationSet; update(u: ViewUpdate): void; destroy(): void } };
  let state = EditorState.create({ doc: 'fn main() { let total = add(1, 2); }\nfn add(a: i32, b: i32) -> i32 { a + b }\n', extensions: ext as never });
  const dispatched: TransactionSpec[] = [];
  const view = {
    get state() { return state; },
    get visibleRanges() { return [{ from: 0, to: state.doc.length }]; },
    dom: { isConnected: false },
    dispatch: (spec: TransactionSpec) => { dispatched.push(spec); },
  } as unknown as EditorView;
  const inst = plugin.create(view);
  const upd = (tr: Transaction): void => {
    state = tr.state;
    inst.update({ view, state, changes: tr.changes, transactions: [tr], docChanged: tr.docChanged, selectionSet: Boolean(tr.selection), viewportChanged: false } as unknown as ViewUpdate);
  };
  assert.equal(computes, 1);
  const firstFrom = (): number => { let f = -1; inst.decorations.between(0, state.doc.length, (from) => { f = from; return false; }); return f; };
  const before = firstFrom();
  assert.ok(before >= 0, 'the snippet has semantic marks');

  for (const ch of 'abc') upd(state.update({ changes: { from: 0, insert: ch }, selection: { anchor: 1 } }));
  assert.equal(computes, 1, 'no whole-file rebuild while typing');
  assert.equal(firstFrom(), before + 3, 'existing marks shift with the edit');
  upd(state.update({ selection: { anchor: 2 } }));
  assert.equal(computes, 1, 'a cursor move during the pause waits for the rebuild');

  await new Promise((r) => setTimeout(r, SEMANTIC_REFRESH_MS + 60));
  assert.equal(dispatched.length, 1, 'one refresh after the pause');
  upd(state.update(dispatched[0]));
  assert.equal(computes, 2);
  upd(state.update({ selection: { anchor: 4 } }));
  assert.equal(computes, 3, 'cursor moves compute at once again');
  inst.destroy();
});

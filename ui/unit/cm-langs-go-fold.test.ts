import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ensureSyntaxTree, foldable } from '@codemirror/language';
import { EditorState } from '@codemirror/state';
import { cmLangNow, loadCmLang } from '../src/lib/components/cm-langs.ts';

test('a Go struct and interface type fold over their bodies', async () => {
  await loadCmLang('go');
  const doc = 'package x\n\ntype Item struct {\n\tA int\n\tB string\n}\n\ntype Doer interface {\n\tDo() error\n}\n';
  const state = EditorState.create({ doc, extensions: [cmLangNow('go', 'mysql')!] });
  ensureSyntaxTree(state, state.doc.length, 5000);
  for (const [ln, closing] of [[3, 6], [8, 10]]) {
    const line = state.doc.line(ln);
    const range = foldable(state, line.from, line.to);
    assert.ok(range, `line ${ln} (${line.text}) has no fold`);
    assert.equal(state.doc.lineAt(range.to).number, closing, line.text);
  }
});

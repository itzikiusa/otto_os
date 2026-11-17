import assert from 'node:assert/strict';
import { test } from 'node:test';
import { workspaceEditToTextEdits } from '../src/lib/components/cm-ide-lsp-completion.ts';
import { renameBody } from '../src/modules/ide/shell/ideRenameRequest.ts';

const range = (l: number, c: number, len: number) => ({ start: { line: l, character: c }, end: { line: l, character: c + len } });

test('a rename WorkspaceEdit (changes or documentChanges) maps to 1-based per-file edits', () => {
  const fromChanges = workspaceEditToTextEdits({
    changes: { 'file:///ws/src/lib.rs': [{ range: range(0, 7, 3), newText: 'sum' }, { range: range(4, 2, 3), newText: 'sum' }] },
  });
  assert.deepEqual(fromChanges, [
    { path: '/ws/src/lib.rs', start_line: 1, start_col: 8, end_line: 1, end_col: 11, new_text: 'sum' },
    { path: '/ws/src/lib.rs', start_line: 5, start_col: 3, end_line: 5, end_col: 6, new_text: 'sum' },
  ]);
  const fromDocChanges = workspaceEditToTextEdits({
    documentChanges: [
      { textDocument: { uri: 'file:///ws/a%20b/Main.java', version: 3 }, edits: [{ range: range(2, 4, 5), newText: 'total' }] },
      { kind: 'rename', oldUri: 'file:///ws/Old.java', newUri: 'file:///ws/New.java' },
    ],
  });
  assert.deepEqual(fromDocChanges, [
    { path: '/ws/a b/Main.java', start_line: 3, start_col: 5, end_line: 3, end_col: 10, new_text: 'total' },
  ]);
  assert.deepEqual(workspaceEditToTextEdits(null), []);
});

test('the rename request carries the language server edits, except for an in-place rename', async () => {
  const lspEdit = { path: '/ws/lib.rs', start_line: 1, start_col: 8, end_line: 1, end_col: 11, new_text: 'sum' };
  const host = (symbol: string, edits: typeof lspEdit[] | null) => ({
    renameState: { path: '/ws/lib.rs', line: 1, column: 8, symbol } as never,
    targetPayload: { kind: 'local', root: '/ws' } as never,
    lspRenameEdits: async () => edits,
  });
  const content = 'pub fn add(a: i32) -> i32 { a }\n';
  assert.deepEqual((await renameBody(host('add', [lspEdit]), 'sum', content, false)).lsp_edits, [lspEdit]);
  // No server answer: the daemon's text match.
  assert.equal((await renameBody(host('add', null), 'sum', content, false)).lsp_edits, undefined);
  // The buffer already says `sum` where `add` was: the server must not be asked.
  const inPlace = await renameBody(host('add', [lspEdit]), 'sum', 'pub fn sum(a: i32) -> i32 { a }\n', true);
  assert.equal(inPlace.lsp_edits, undefined);
  assert.equal(inPlace.apply, true);
});

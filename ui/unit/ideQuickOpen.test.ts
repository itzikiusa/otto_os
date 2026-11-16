// IDE quick-open rules (review I1-03 / I1-04 / I1-08): `:42` is a line in the active
// file, "All" shows a declaration once (as the symbol), and an empty query lists
// recent files, then the other open tabs.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  dropTextHitsShownAsSymbols,
  matchesFileMask,
  parentFolderOf,
  parseQuickOpenQuery,
  recentFileRows,
} from '../src/modules/ide/search/ideQuickOpenModel.ts';

test(':line and file:line parse on All and Files; other tabs search the text as typed', () => {
  assert.deepEqual(parseQuickOpenQuery('files', ':42'), { cleanQuery: '', line: 42, col: null, lineOnly: true });
  assert.deepEqual(parseQuickOpenQuery('all', ' :42:7 '), { cleanQuery: '', line: 42, col: 7, lineOnly: true });
  assert.deepEqual(parseQuickOpenQuery('files', ':'), { cleanQuery: '', line: null, col: null, lineOnly: true }, 'the ⌘G starting state');
  assert.deepEqual(parseQuickOpenQuery('files', ':0'), { cleanQuery: '', line: null, col: null, lineOnly: true }, 'line 0 is no line');
  assert.deepEqual(parseQuickOpenQuery('files', 'main.go:42'), { cleanQuery: 'main.go', line: 42, col: null, lineOnly: false });
  assert.deepEqual(parseQuickOpenQuery('all', 'app/main.go:3:9'), { cleanQuery: 'app/main.go', line: 3, col: 9, lineOnly: false });
  assert.deepEqual(parseQuickOpenQuery('files', 'main'), { cleanQuery: 'main', line: null, col: null, lineOnly: false });
  assert.equal(parseQuickOpenQuery('symbols', ':42').lineOnly, false);
  assert.equal(parseQuickOpenQuery('text', 'a:1').cleanQuery, 'a:1', 'Find in path keeps the colon');
});

test('All drops a text hit whose path:line is already a symbol hit, keeps the rest in order', () => {
  const items = [
    { kind: 'file' as const, path: '/r/app/main.go', preview: 'main.go' },
    { kind: 'symbol' as const, path: '/r/app/main.go', line: 34, preview: 'func main() {', symbol_kind: 'func' },
    { kind: 'text' as const, path: '/r/app/main.go', line: 1, preview: 'package main' },
    { kind: 'text' as const, path: '/r/app/main.go', line: 34, preview: 'func main() {' },
    { kind: 'text' as const, path: '/r/py/pipeline.py', line: 34, preview: 'if __name__ == "__main__":' },
  ];
  assert.deepEqual(
    dropTextHitsShownAsSymbols(items).map((i) => `${i.kind}:${i.path}:${i.line ?? ''}`),
    ['file:/r/app/main.go:', 'symbol:/r/app/main.go:34', 'text:/r/app/main.go:1', 'text:/r/py/pipeline.py:34'],
  );
  const noSymbols = items.filter((i) => i.kind !== 'symbol');
  assert.equal(dropTextHitsShownAsSymbols(noSymbols).length, noSymbols.length);
});

test('empty query: recent files newest first, then open tabs, each once, without the active file', () => {
  const rows = recentFileRows(
    ['/r/b.go', '/r/a.go', '/r/notes/n1.md'],
    ['/r/a.go', '/r/c.go', '/r/b.go', '/r/d.go'],
    '/r/b.go',
  );
  assert.deepEqual(rows, [
    { path: '/r/a.go', source: 'recent' },
    { path: '/r/notes/n1.md', source: 'recent' },
    { path: '/r/c.go', source: 'open' },
    { path: '/r/d.go', source: 'open' },
  ]);
  assert.equal(recentFileRows(Array.from({ length: 50 }, (_, i) => `/r/f${i}`), [], null, 30).length, 30);
  assert.deepEqual(recentFileRows([], [], null), []);
});

test('rows show the parent folder, and the file mask filters Find in path', () => {
  assert.equal(parentFolderOf('notes/2026/note-01-meeting-summary.md'), 'notes/2026/');
  assert.equal(parentFolderOf('README.md'), '');
  assert.equal(matchesFileMask('a_test.go', 'pkg/a_test.go', '*.go, !*_test.go'), false);
  assert.equal(matchesFileMask('a.go', 'pkg/a.go', '*.go, !*_test.go'), true);
  assert.equal(matchesFileMask('a.ts', 'web/a.ts', 'web/*'), true);
  assert.equal(matchesFileMask('a.ts', 'web/a.ts', ''), true);
});

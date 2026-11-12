// An inline PR comment whose line is on no rendered row (outside the context
// the forge sent, or moved by a push the forge didn't flag outdated) used to
// vanish; it is now listed in the file's comments block, marked "not shown".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildFileRows, indexComments, orphanComments } from '../src/modules/git/diff-model.ts';
import type { FileDiff, PrComment } from '../src/lib/api/types.ts';

const c = (id: string, line: number | null): PrComment => ({
  id,
  author: 'a',
  body: id,
  path: 'f.ts',
  line,
  created_at: '2026-11-12T00:00:00Z',
  replies: [],
  resolved: false,
  side: 'new',
});
const file: FileDiff = {
  path: 'f.ts',
  old_path: null,
  is_binary: false,
  added: 1,
  deleted: 0,
  hunks: [{ header: '@@ -1,1 +1,2 @@', lines: [
    { origin: 'context', content: 'a', old_line: 1, new_line: 1 },
    { origin: 'add', content: 'b', old_line: null, new_line: 2 },
  ] }],
};

test('orphanComments: only anchors on no line of the hunks', () => {
  const fc = indexComments([c('on2', 2), c('on40', 40), c('file', null)]).get('f.ts');
  assert.deepEqual(orphanComments(fc, file).map((x) => x.id), ['on40']);
  assert.deepEqual(orphanComments(indexComments([c('on1', 1)]).get('f.ts'), file), []);
  assert.deepEqual(orphanComments(undefined, file), []);
});

test('buildFileRows lists the orphan in the file block, flagged, and the rest inline', () => {
  const fc = indexComments([c('on2', 2), c('on40', 40), c('file', null)]).get('f.ts');
  const rows = buildFileRows({ file, eff: file, collapsed: false, first: true, split: false, fc, uncapped: undefined, composer: null, loadError: undefined, forceFull: false, canLoad: false });
  const block = rows.find((r) => r.kind === 'fcomments');
  assert.ok(block && block.kind === 'fcomments');
  assert.deepEqual(block.comments.map((x) => x.id), ['file', 'on40']);
  assert.deepEqual([...(block.notShown ?? [])].map((x) => x.id), ['on40']);
  const inline = rows.filter((r) => r.kind === 'comment').flatMap((r) => (r.kind === 'comment' ? r.comments.map((x) => x.id) : []));
  assert.deepEqual(inline, ['on2']);
});

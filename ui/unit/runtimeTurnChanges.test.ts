// The end-of-turn changes card (turnChanges.ts) over every provider's file-change shape.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const { changeTotals, diffCounts, turnChanges, turnChangesByTurn } = await import('../src/modules/threads/runtime/turnChanges.ts');

const fc = (id: string, file_name: string, extra: Record<string, unknown> = {}) =>
  ({ id, turn_id: 't1', ordinal: 0, status: 'completed', title: null, started_at: null, completed_at: null, updated_at: '', type: 'file_change', file_name, ...extra }) as never;

test('a unified diff (Codex, ACP) counts + and − lines, not the headers', () => {
  assert.deepEqual(diffCounts({ diff_str: '--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n-a\n+b\n+c\n d' }), { added: 2, removed: 1 });
});

test('old / new text (Claude Edit, pi) counts the changed middle; a new file is all added', () => {
  assert.deepEqual(diffCounts({ old_str: 'a\nb\nc\n', new_str: 'a\nB\nB2\nc\n' }), { added: 2, removed: 1 });
  assert.deepEqual(diffCounts({ new_str: 'one\ntwo\n' }), { added: 2, removed: 0 });
  assert.equal(diffCounts({}), null);
});

test('a turn’s files: one row per file, relative paths, failed and other turns left out, names only without text', () => {
  const items = [
    fc('1', '/repo/src/a.ts', { diff_str: '+x\n+y\n-z' }),
    fc('2', '/repo/src/a.ts', { old_str: 'q', new_str: 'r' }),
    fc('3', '/repo/README.md'),
    fc('4', '/repo/bad.ts', { status: 'failed', error: 'denied', diff_str: '+1' }),
    fc('5', '/repo/other.ts', { turn_id: 't2', diff_str: '+1' }),
    fc('6', '/repo/sub.ts', { parent_item_id: 'sa', diff_str: '+1' }),
  ];
  const files = turnChanges(items, 't1', '/repo');
  // The one-pass form (the timeline's cards) agrees for every turn.
  const all = turnChangesByTurn(items, '/repo');
  assert.deepEqual(all.get('t1'), files);
  assert.deepEqual(all.get('t2'), turnChanges(items, 't2', '/repo'));
  assert.deepEqual([...all.keys()].sort(), ['t1', 't2']);
  assert.deepEqual(files, [
    { path: 'src/a.ts', added: 3, removed: 2 },
    { path: 'README.md', added: null, removed: null },
  ]);
  assert.deepEqual(changeTotals(files), { added: 3, removed: 2 });
  assert.deepEqual(changeTotals([{ path: 'x', added: null, removed: null }]), { added: null, removed: null });
});

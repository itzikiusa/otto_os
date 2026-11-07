import { test } from 'node:test';
import assert from 'node:assert/strict';
import { addComposerContext, assembleComposerMessage, reviewCommentContext, diffContext } from '../src/modules/threads/runtime/composerContext.ts';
import type { PrComment, FileDiff } from '../src/lib/api/types';

const comment: PrComment = { id: '42', author: 'Alice', body: 'Check null input', path: 'src/a.ts', line: 9, created_at: '', resolved: false, replies: [{ id: '43', author: 'Bob', body: 'Agreed', path: null, line: null, created_at: '', resolved: false, replies: [] }] };
const file: FileDiff = { path: 'src/a.ts', old_path: null, is_binary: false, hunks: [{ header: '@@ -8,1 +8,2 @@', lines: [{ origin: 'context', content: 'before', old_line: 8, new_line: 8 }, { origin: 'add', content: 'after', old_line: null, new_line: 9 }] }] };

test('multiple context chips stack and repeat selection replaces in place', () => {
  const review = reviewCommentContext('r', 'https://github.com/a/b/pull/1', comment);
  const diff = diffContext('r', file);
  const first = addComposerContext([], review);
  const both = addComposerContext(first, diff);
  const again = addComposerContext(both, { ...review, text: 'updated' });
  assert.equal(first.length, 1);
  assert.equal(again.length, 2);
  assert.equal(again[0].text, 'updated');
  assert.equal(again[1].id, diff.id);
});

test('assembly preserves draft and attributes complete comment conversation without adding instructions', () => {
  const context = reviewCommentContext('r', 'https://github.com/a/b/pull/1', comment);
  const text = assembleComposerMessage('  My question\n', [context]);
  assert.ok(text.startsWith('  My question\n'));
  for (const expected of ['Alice', 'Bob', 'Check null input', 'Agreed', 'https://github.com/a/b/pull/1', 'src/a.ts:9']) assert.ok(text.includes(expected), expected);
  assert.equal(assembleComposerMessage('draft', []), 'draft');
  assert.doesNotMatch(text, /please fix|fix these|implement/i);
});

test('diff context carries selected file, hunk or line with side and line numbers', () => {
  const all = diffContext('r', file);
  const hunk = diffContext('r', file, { hunk: 0 });
  const line = diffContext('r', file, { hunk: 0, line: 1 });
  assert.match(all.text, /before/);
  assert.match(hunk.text, /@@ -8,1 \+8,2 @@/);
  assert.match(line.text, /new 9/);
  assert.match(line.text, /\+after/);
  assert.doesNotMatch(line.text, /before/);
  assert.notEqual(all.id, line.id);
  assert.notEqual(hunk.id, line.id);
});

test('inspector preserves the owning draft and rejects a selection from a previous thread', async () => {
  const { componentFunctions } = await import('./componentFunctions.ts');
  const drafts = { A: { text: 'Existing draft', context: [] as ReturnType<typeof reviewCommentContext>[] }, B: { text: 'Other draft', context: [] as ReturnType<typeof reviewCommentContext>[] } };
  let closed = false;
  const state = componentFunctions(new URL('../src/modules/threads/ThreadInspector.svelte', import.meta.url), ['addContext'], {
    contextThreadId: 'A',
    threads: { focusedSessionId: 'A', inspectorCompact: true, setInspectorOpen: () => { closed = true; } },
    runtimeDrafts: { for: (id: 'A' | 'B') => drafts[id] },
    addComposerContext,
    tick: () => Promise.resolve(), CSS: { escape: (s: string) => s }, document: { querySelector: () => null },
  });
  const selected = reviewCommentContext('r', 'https://github.com/a/b/pull/1', comment);
  state.addContext(selected, 'A');
  assert.equal(drafts.A.text, 'Existing draft');
  assert.equal(drafts.A.context.length, 1);
  assert.equal(closed, true);
  state.contextThreadId = 'B';
  state.threads.focusedSessionId = 'B';
  state.addContext(selected, 'A');
  assert.equal(drafts.B.context.length, 0);
  assert.equal(drafts.B.text, 'Other draft');
  await Promise.resolve();
});

// The ⌘K switcher's thread rows: the sidebar's inbox rank (threadInbox.ts)
// over the sidebar's own order, its words' tones, typing filters, and the
// order frozen while the bar is open.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { filterThreadRows, frozenOrder } from '../src/lib/threadSwitch.ts';
import type { InboxClass } from '../src/modules/threads/threadInbox.ts';

// threadInbox.ts imports its siblings without an extension (as the app does).
const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const { switcherOrder, wordTone } = await import('../src/modules/threads/threadSwitcher.ts');

const t = (id: string, cls: InboxClass, lastActive = '2026-11-23T11:00:00Z') => ({ id, cls, lastActive });

test('inbox rank first (needs, failed, done), then working, then the rest; the sidebar order inside a rank', () => {
  const rows = switcherOrder([t('old', 'rest'), t('work', 'working'), t('done', 'done'), t('fail', 'failed'), t('b', 'needs'), t('a', 'needs')], ['a', 'old', 'work', 'done', 'fail', 'b']);
  assert.deepEqual(rows.map((r) => r.id), ['a', 'b', 'fail', 'done', 'work', 'old']);
});

test('threads the sidebar does not show follow, newest first', () => {
  const rows = switcherOrder([t('x', 'rest', '2026-11-20T00:00:00Z'), t('y', 'rest', '2026-11-22T00:00:00Z'), t('z', 'rest')], ['z']);
  assert.deepEqual(rows.map((r) => r.id), ['z', 'y', 'x']);
});

test('the words keep the sidebar tones', () => {
  assert.equal(wordTone('Approval'), 'warning');
  assert.equal(wordTone('Question'), 'warning');
  assert.equal(wordTone('Failed'), 'danger');
  assert.equal(wordTone('Done'), 'success');
  assert.equal(wordTone('Working 18s'), null);
  assert.equal(wordTone('5m'), null);
});

test('the order the bar opened with holds while it is open', () => {
  const rows = [{ id: 'b' }, { id: 'new' }, { id: 'a' }];
  assert.deepEqual(frozenOrder(rows, ['a', 'gone', 'b']).map((r) => r.id), ['a', 'b', 'new']);
});

test('typing matches title and project together', () => {
  const rows = [
    { id: '1', title: 'Fix the flaky checkout test', project: 'harbor' },
    { id: '2', title: 'Speed up zsh startup', project: 'dotfiles' },
  ];
  assert.deepEqual(filterThreadRows(rows, 'dotfiles').map((r) => r.id), ['2']);
  assert.deepEqual(filterThreadRows(rows, 'flaky').map((r) => r.id), ['1']);
  assert.deepEqual(filterThreadRows(rows, 'harbor flaky').map((r) => r.id), ['1']);
  assert.equal(filterThreadRows(rows, '').length, 2);
});

test('a thread row needs the query to name it, so a command query keeps Enter', () => {
  const rows = [
    { id: '1', title: 'Fix sidebar navigation remount', project: 'otto' },
    { id: '2', title: 'Git status in the sidebar', project: 'otto' },
  ];
  // "git" is a subsequence of "Fix sidebar navigation remount": no longer a hit.
  assert.deepEqual(filterThreadRows(rows, 'git').map((r) => r.id), ['2']);
  assert.deepEqual(filterThreadRows(rows, 'set').map((r) => r.id), []);
  assert.deepEqual(filterThreadRows(rows, 'side nav').map((r) => r.id), ['1']);
});

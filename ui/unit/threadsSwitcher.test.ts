// The ⌘K switcher's thread rows (threadSwitcher.ts): what needs you first,
// then finished-unread, then working, then recent; the sidebar's order inside
// each; one status word; the order frozen while the palette is open.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shortAgo, switcherRows, type SwitcherThread } from '../src/modules/threads/threadSwitcher.ts';
import { filterThreadRows, frozenOrder } from '../src/lib/threadSwitch.ts';

const NOW = Date.parse('2026-11-23T12:00:00Z');
const t = (id: string, key: string, more: Partial<SwitcherThread> = {}): SwitcherThread => ({
  id, title: `T ${id}`, project: 'harbor', state: { key, label: key }, unread: false, asks: null, lastActive: '2026-11-23T11:00:00Z', ...more,
});

test('needs you and failed lead, then Done (unread), Working, then recent', () => {
  const rows = switcherRows(
    [t('old', 'idle'), t('work', 'working'), t('done', 'idle', { unread: true }), t('fail', 'failed'), t('ask', 'needs-you', { asks: 'question' }), t('ok', 'needs-you', { asks: 'approval' })],
    ['old', 'work', 'done', 'fail', 'ok', 'ask'],
    NOW,
  );
  assert.deepEqual(rows.map((r) => [r.id, r.word]), [['fail', 'Failed'], ['ok', 'Approval'], ['ask', 'Question'], ['done', 'Done'], ['work', 'Working'], ['old', '1h']]);
  assert.equal(rows[0].tone, 'danger');
  assert.equal(rows[1].tone, 'warning');
  assert.equal(rows[3].tone, 'success');
});

test('a waiting thread whose request is not loaded says "Needs you"; unknown order falls back to recency', () => {
  const rows = switcherRows([t('a', 'needs-you'), t('b', 'idle', { lastActive: '2026-11-23T11:59:30Z' }), t('c', 'idle', { lastActive: '2026-11-21T12:00:00Z' })], [], NOW);
  assert.deepEqual(rows.map((r) => [r.id, r.word]), [['a', 'Needs you'], ['b', 'now'], ['c', '2d']]);
});

test('the order the palette opened with holds while it is open', () => {
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
  assert.deepEqual(filterThreadRows(rows, 'harbor flaky').map((r) => r.id), []);
  assert.equal(filterThreadRows(rows, '').length, 2);
  assert.equal(shortAgo('2026-11-23T11:55:00Z', NOW), '5m');
});

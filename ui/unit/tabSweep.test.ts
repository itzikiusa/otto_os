// Tabs that outlive their session (owner report: "we deleted a session, but it
// wasn't removed from the tabs"): the window-return / post-bulk safety net in
// lib/stores/tabSweep.ts closes a tab only when the daemon PROVES its session
// gone (a successful by-id answer without it) or archived — never on a
// failed request.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { staleTabs, sweepDue, sweepIds, sweepTabs, TAB_SWEEP_THROTTLE_MS, type TabSweepHost } from '../src/lib/stores/tabSweep.ts';

test('staleTabs: absent from a successful answer = gone; archived row = archived; present + live = kept', () => {
  const got = staleTabs(['a', 'b', 'c'], [{ id: 'a', archived: false }, { id: 'c', archived: true }], new Set());
  assert.deepEqual(got, [
    { id: 'b', reason: 'gone' },
    { id: 'c', reason: 'archived' },
  ]);
});

test('staleTabs: a failed chunk proves nothing — its ids are never closed', () => {
  assert.deepEqual(staleTabs(['a', 'b'], [], new Set(['a', 'b'])), []);
  assert.deepEqual(staleTabs(['a', 'b'], [], new Set(['a'])), [{ id: 'b', reason: 'gone' }]);
});

test('sweepIds: tabs + panes once each, without the DB-Explorer sentinel', () => {
  assert.deepEqual(sweepIds(['a', '__db_explorer__', 'b'], ['b', 'c']), ['a', 'b', 'c']);
});

test('sweepDue: at most once per throttle window unless forced', () => {
  assert.equal(sweepDue(1000, 0), false);
  assert.equal(sweepDue(TAB_SWEEP_THROTTLE_MS, 0), true);
  assert.equal(sweepDue(1000, 0, true), true);
});

function host(tabs: string[], panes: string[] = []) {
  const h = {
    openTabs: tabs,
    panes,
    currentId: 'w1' as string | null,
    closed: [] as string[],
    statusMap: { a: 'idle', gone: 'working' } as Record<string, string>,
    getSession: (id: string) => (id === 'a' || id === 'gone' ? { title: `T-${id}`, status: 'idle' } : null),
    closeTab(id: string) {
      h.closed.push(id);
      h.openTabs = h.openTabs.filter((t) => t !== id);
    },
  };
  const typed: TabSweepHost = h;
  void typed;
  return h;
}

test('sweepTabs: closes the deleted session’s tab and logs id, title and last status', async () => {
  const h = host(['a', 'gone', '__db_explorer__']);
  const logs: Record<string, unknown>[] = [];
  const closed = await sweepTabs(h, async (ids) => ids.filter((id) => id === 'a').map((id) => ({ id })), (_m, d) => logs.push(d));
  assert.deepEqual(closed, ['gone']);
  assert.deepEqual(h.closed, ['gone']);
  assert.equal(logs.length, 1);
  assert.equal(logs[0].id, 'gone');
  assert.equal(logs[0].title, 'T-gone');
  assert.equal(logs[0].status, 'working');
  assert.equal(logs[0].reason, 'gone');
});

test('sweepTabs: a failed request closes nothing; a workspace switch mid-request closes nothing', async () => {
  const h = host(['a', 'gone']);
  assert.deepEqual(await sweepTabs(h, async () => { throw new Error('offline'); }, () => {}), []);
  assert.deepEqual(h.closed, []);
  const h2 = host(['gone']);
  const closed = await sweepTabs(h2, async () => { h2.currentId = 'w2'; return []; }, () => {});
  assert.deepEqual(closed, []);
  assert.deepEqual(h2.closed, []);
});

test('sweepTabs: a pane-only id is checked too; nothing open asks nothing', async () => {
  const h = host([], ['gone']);
  let asked: string[] = [];
  await sweepTabs(h, async (ids) => { asked = ids; return []; }, () => {});
  assert.deepEqual(asked, ['gone']);
  assert.deepEqual(h.closed, ['gone']);
  let called = false;
  await sweepTabs(host([]), async () => { called = true; return []; }, () => {});
  assert.equal(called, false);
});

test('parseTabIds: a corrupt saved tab list means no tabs, never a throw', async () => {
  const { parseTabIds } = await import('../src/lib/stores/sessionBuckets.ts');
  assert.deepEqual(parseTabIds('["a","b"]'), ['a', 'b']);
  assert.deepEqual(parseTabIds('{not json'), []);
  assert.deepEqual(parseTabIds('{"a":1}'), []);
  assert.deepEqual(parseTabIds('["a",3,null]'), ['a']);
  assert.deepEqual(parseTabIds(null), []);
});

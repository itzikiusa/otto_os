// The workspace switcher at a dozen workspaces (shell/workspaceSwitch.ts):
// recently used first, and live working / needs-you counts per workspace from
// rows the store already holds.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RECENT_MAX, activityText, noteRecent, parseRecent, switcherItems, switcherOrder, workspaceActivity } from '../src/shell/workspaceSwitch.ts';

const wsList = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `w${i}`, name: `W${i}`, root_path: `/p/${i}` }));

test('noteRecent / parseRecent: newest first, deduped, capped; corrupt storage = none', () => {
  let r: string[] = [];
  for (const id of ['a', 'b', 'c', 'a', 'd', 'e', 'f']) r = noteRecent(r, id);
  assert.deepEqual(r, ['f', 'e', 'd', 'a'].slice(0, RECENT_MAX));
  assert.deepEqual(parseRecent(JSON.stringify(r)), r);
  assert.deepEqual(parseRecent('{bad'), []);
  assert.deepEqual(parseRecent('[1,"x"]'), ['x']);
});

test('switcherOrder: recents (that still exist) lead a long list; a short list has none', () => {
  const list = wsList(13);
  const { recent, rest } = switcherOrder(list, ['w7', 'gone', 'w2']);
  assert.deepEqual(recent.map((w) => w.id), ['w7', 'w2']);
  assert.equal(rest.length, 11);
  assert.ok(!rest.some((w) => w.id === 'w7' || w.id === 'w2'));
  assert.deepEqual(switcherOrder(wsList(3), ['w2']).recent, []);
});

test('workspaceActivity: working and needs-you per workspace, each session once', () => {
  const rows = [
    { id: 'a', workspace_id: 'w1' },
    { id: 'b', workspace_id: 'w1' },
    { id: 'c', workspace_id: 'w2' },
    { id: 'a', workspace_id: 'w1' },
    { id: 'd', workspace_id: 'w3' },
  ];
  const act = workspaceActivity(rows, { a: 'working', b: 'idle', c: 'working', d: 'idle' }, { b: true, c: true });
  assert.deepEqual(act.get('w1'), { working: 1, needs: 1 });
  assert.deepEqual(act.get('w2'), { working: 1, needs: 1 });
  assert.equal(act.has('w3'), false);
  assert.equal(activityText(act.get('w1')), '1 working · 1 needs you');
  assert.equal(activityText({ working: 0, needs: 2 }), '2 need you');
  assert.equal(activityText(undefined), undefined);
});

test('switcherItems: recents, a divider, the rest; activity as the row hint; current checked', () => {
  const picked: string[] = [];
  const items = switcherItems(wsList(8), ['w5'], new Map([['w5', { working: 2, needs: 0 }]]), 'w1', (id) => picked.push(id));
  assert.equal(items[0].label, 'W5');
  assert.equal(items[0].hint, '2 working');
  assert.equal(items[1].separator, true);
  assert.equal(items.length, 9);
  assert.equal(items.find((i) => i.label === 'W1')?.checked, true);
  items[0].action?.();
  assert.deepEqual(picked, ['w5']);
});

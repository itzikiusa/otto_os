// The side-surface rules (lib/components/side/paneKeys.ts, listNav.ts): one
// key scheme with a one-time read of old keys, the tablet clamp, the Detail
// pane's inline / sheet switch, and the one keyboard walk.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  DETAIL_PANE,
  LIST_PANE,
  WIDE_LIST_PANE,
  detailFit,
  fitPaneMax,
  loadPaneHidden,
  loadPaneW,
  paneKey,
  savePaneHidden,
  savePaneW,
} from '../src/lib/components/side/paneKeys.ts';
import { moveIndex, treeStep } from '../src/lib/components/side/listNav.ts';

function mem(seed: Record<string, string> = {}) {
  const m = new Map(Object.entries(seed));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
}

test('one key scheme: otto.pane.<module>.<pane>.{w,hidden}', () => {
  assert.equal(paneKey('threads', 'list', 'w'), 'otto.pane.threads.list.w');
  assert.equal(paneKey('db', 'tables', 'hidden'), 'otto.pane.db.tables.hidden');
});

test('a width is read clamped into the range; nothing stored → the default (or the page default)', () => {
  const s = mem({ 'otto.pane.a.list.w': '9999', 'otto.pane.b.list.w': '10', 'otto.pane.c.list.w': 'x' });
  assert.equal(loadPaneW(s, 'a', 'list', LIST_PANE), 420);
  assert.equal(loadPaneW(s, 'b', 'list', LIST_PANE), 220);
  assert.equal(loadPaneW(s, 'c', 'list', LIST_PANE), 280);
  assert.equal(loadPaneW(s, 'none', 'list', LIST_PANE, undefined, 240), 240);
  assert.equal(loadPaneW(null, 'a', 'list', LIST_PANE), 280, 'no storage');
});

test('an old width key is read once and moved to the new key (the old one stays)', () => {
  const s = mem({ 'threads.sidebarW': '300' });
  assert.equal(loadPaneW(s, 'threads', 'list', LIST_PANE, { w: 'threads.sidebarW' }), 300);
  assert.equal(s.m.get('otto.pane.threads.list.w'), '300');
  assert.equal(s.m.get('threads.sidebarW'), '300', 'old key kept for a rollback');
  savePaneW(s, 'threads', 'list', 333);
  assert.equal(loadPaneW(s, 'threads', 'list', LIST_PANE, { w: 'threads.sidebarW' }), 333, 'new key wins afterwards');
});

test('hidden: new key wins, else the legacy reader once, else shown', () => {
  const s = mem();
  let reads = 0;
  const legacy = { hidden: () => (reads++, true) };
  assert.equal(loadPaneHidden(s, 'threads', 'list', legacy), true);
  assert.equal(s.m.get('otto.pane.threads.list.hidden'), '1');
  assert.equal(loadPaneHidden(s, 'threads', 'list', legacy), true);
  assert.equal(reads, 1, 'the old state is read once');
  savePaneHidden(s, 'threads', 'list', false);
  assert.equal(loadPaneHidden(s, 'threads', 'list', legacy), false);
  assert.equal(loadPaneHidden(mem(), 'x', 'list', { hidden: () => null }), false);
});

test('tablet clamp: the list yields so the content keeps 480 px, never below its min', () => {
  assert.equal(fitPaneMax(1440, LIST_PANE), 420, 'desktop: the range max');
  assert.equal(fitPaneMax(776, LIST_PANE), 296, '820 iPad minus the 44 px Rail');
  assert.equal(fitPaneMax(600, LIST_PANE), 220, 'never below min');
  assert.equal(fitPaneMax(776, WIDE_LIST_PANE), 296, 'the PR preset at 820 (mockup note 11)');
  assert.equal(fitPaneMax(0, LIST_PANE), 420, 'not measured yet');
});

test('Detail pane: inline while the content keeps 480 px beside its minimum, else a sheet', () => {
  assert.deepEqual(detailFit(1200, 380, DETAIL_PANE), { sheet: false, shown: 380 });
  assert.deepEqual(detailFit(800, 380, DETAIL_PANE), { sheet: false, shown: 320 }, 'yields to keep 480');
  assert.equal(detailFit(760, 380, DETAIL_PANE).sheet, true, '760 − 480 < 300');
  assert.deepEqual(detailFit(1400, 380, DETAIL_PANE, 640, true), { sheet: false, shown: 760 }, 'maximized takes the room');
  assert.deepEqual(detailFit(0, 380, DETAIL_PANE), { sheet: false, shown: 380 }, 'not measured yet');
});

test('the keyboard walk: ↑ ↓ Home End, never wrapping', () => {
  assert.equal(moveIndex('ArrowDown', 0, 3), 1);
  assert.equal(moveIndex('ArrowDown', 2, 3), 2);
  assert.equal(moveIndex('ArrowUp', 0, 3), 0);
  assert.equal(moveIndex('Home', 2, 3), 0);
  assert.equal(moveIndex('End', 0, 3), 2);
  assert.equal(moveIndex('Enter', 0, 3), null);
  assert.equal(moveIndex('ArrowDown', 0, 0), null);
});

test('→ / ← open, close, descend and climb a tree (mirrored in RTL)', () => {
  assert.equal(treeStep('ArrowRight', false, 1), 'expand');
  assert.equal(treeStep('ArrowRight', true, 1), 'child');
  assert.equal(treeStep('ArrowRight', null, 2), null);
  assert.equal(treeStep('ArrowLeft', true, 2), 'collapse');
  assert.equal(treeStep('ArrowLeft', null, 2), 'parent');
  assert.equal(treeStep('ArrowLeft', false, 1), null);
  assert.equal(treeStep('ArrowLeft', false, 1, true), 'expand', 'RTL: ← opens');
});

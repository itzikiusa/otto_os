// use:rowMenu (lib/rowMenu.ts): the keyboard chords that open a row's context
// menu, and where a keyboard-opened menu lands (the row's start edge, RTL too).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isMenuKey, keyboardMenuPoint } from '../src/lib/rowMenu.ts';

const k = (key: string, mods: Partial<Record<'shiftKey' | 'metaKey' | 'ctrlKey' | 'altKey', boolean>> = {}) => ({
  key,
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...mods,
});

test('ContextMenu key and ⇧F10 open the menu; nothing else does', () => {
  assert.equal(isMenuKey(k('ContextMenu')), true);
  assert.equal(isMenuKey(k('F10', { shiftKey: true })), true);
  assert.equal(isMenuKey(k('F10')), false, 'plain F10 is the menu bar, not ours');
  assert.equal(isMenuKey(k('F10', { shiftKey: true, metaKey: true })), false);
  assert.equal(isMenuKey(k('Enter')), false);
});

test('keyboard menu opens at the start edge, mirrored in RTL', () => {
  const r = { left: 100, right: 500, top: 40, bottom: 80 };
  assert.deepEqual(keyboardMenuPoint(r, false), { x: 108, y: 64 });
  assert.deepEqual(keyboardMenuPoint(r, true), { x: 492, y: 64 });
  assert.deepEqual(keyboardMenuPoint({ left: 0, right: 10, top: 0, bottom: 12 }, false), { x: 8, y: 12 }, 'short rows clamp to their bottom');
});

// Cheat-sheet chord → <kbd> chips (node:test, Node's built-in type stripping).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shortcutChips } from '../src/lib/shortcutChips.ts';

const chips = (s: string) =>
  shortcutChips(s).map((c) => (c.kind === 'sep' ? `(${c.text})` : c.text));

test('plain chords split modifier glyphs into separate keys', () => {
  assert.deepEqual(chips('⌘⇧B'), ['⌘', '⇧', 'B']);
  assert.deepEqual(chips('⌃Tab'), ['⌃', 'Tab']);
  assert.deepEqual(chips('?'), ['?']);
});

test('a range stays one readable chip', () => {
  assert.deepEqual(chips('⌃1…⌃9'), ['⌃1…⌃9']);
  assert.deepEqual(chips('⌃1…⌃4'), ['⌃1…⌃4']);
});

test('alternatives get a plain separator, not a "U / " key', () => {
  assert.deepEqual(chips('⌘U / ⌘⇧U'), ['⌘', 'U', '(/)', '⌘', '⇧', 'U']);
});

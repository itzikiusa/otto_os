// lib/optionMetaHint.ts: the one-time "Option is Meta" terminal hint fires on
// the first ⌥ chord that a non-US layout uses for an ASCII symbol, never for
// US-layout Meta chords, never when the setting is off, and only once.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { noteTerminalKey, optionComposesAscii } from '../src/lib/optionMetaHint.ts';

const key = (k: string, mods: Partial<Record<'altKey' | 'metaKey' | 'ctrlKey', boolean>> = { altKey: true }) => ({
  type: 'keydown',
  key: k,
  altKey: false,
  metaKey: false,
  ctrlKey: false,
  ...mods,
});

test('⌥ composing an ASCII symbol (DE ⌥L = @, ⌥5 = [) counts', () => {
  for (const k of ['@', '[', ']', '{', '}', '|', '\\', '~']) assert.equal(optionComposesAscii(key(k)), true, k);
});

test('US-layout ⌥ glyphs, letters, arrows and other chords do not', () => {
  for (const k of ['å', '∫', '“', '–', '≠', 'b', 'B', '5', 'ArrowLeft', 'Backspace', ' ']) assert.equal(optionComposesAscii(key(k)), false, k);
  assert.equal(optionComposesAscii(key('@', {})), false, 'no ⌥');
  assert.equal(optionComposesAscii(key('@', { altKey: true, metaKey: true })), false, '⌘⌥');
  assert.equal(optionComposesAscii({ ...key('@'), type: 'keyup' }), false);
});

test('the hint shows once, and only while Option is Meta', () => {
  let shown = false;
  const seen: string[] = [];
  let meta = true;
  const deps = { optionAsMeta: () => meta, shown: () => shown, markShown: () => (shown = true), show: (k: string) => seen.push(k) };
  assert.equal(noteTerminalKey(key('å'), deps), false);
  assert.equal(noteTerminalKey(key('@'), deps), true);
  assert.equal(noteTerminalKey(key('['), deps), false, 'second time: silent');
  assert.deepEqual(seen, ['@']);
  shown = false;
  meta = false;
  assert.equal(noteTerminalKey(key('@'), deps), false, 'setting already off: ⌥ composes normally');
});

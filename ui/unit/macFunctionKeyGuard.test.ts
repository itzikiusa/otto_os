import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  applyMacFunctionKeyFallback,
  hasMacFunctionKeyChar,
  stripMacFunctionKeyChars,
} from '../src/lib/macFunctionKeyGuard.ts';

test('hasMacFunctionKeyChar detects Apple PUA function keys and ignores normal text/emojis/RTL', () => {
  assert.equal(hasMacFunctionKeyChar('\uF703'), true); // Right Arrow
  assert.equal(hasMacFunctionKeyChar('hello\uF700world'), true); // Up Arrow
  assert.equal(hasMacFunctionKeyChar('\uF728'), true); // Forward Delete
  assert.equal(hasMacFunctionKeyChar('\uF8FF'), true);
  assert.equal(hasMacFunctionKeyChar('hello world 🚀 שלום \u200F'), false);
  assert.equal(hasMacFunctionKeyChar(''), false);
  assert.equal(hasMacFunctionKeyChar(null), false);
});

test('stripMacFunctionKeyChars removes U+F700..U+F8FF and adjusts caret offsets accurately', () => {
  const dirty = 'ab\uF703\uF703cd\uF702ef';
  const cleaned = stripMacFunctionKeyChars({
    value: dirty,
    selectionStart: 4, // right after the two \uF703 chars ('ab|cd\uF702ef')
    selectionEnd: 7, // right after \uF702 ('abcd|ef')
  });
  assert.equal(cleaned.value, 'abcdef');
  assert.equal(cleaned.selectionStart, 2);
  assert.equal(cleaned.selectionEnd, 4);
});

test('applyMacFunctionKeyFallback moves caret for Left/Right/Up/Down/Home/End and respects Shift/Cmd/Option', () => {
  // Right arrow moves forward one code point (including surrogate pairs)
  const right1 = applyMacFunctionKeyFallback(
    { value: 'a🚀b', selectionStart: 1, selectionEnd: 1 },
    '\uF703',
  );
  assert.ok(right1);
  assert.equal(right1!.mutatedValue, false);
  assert.equal(right1!.next.selectionStart, 3);
  assert.equal(right1!.next.selectionEnd, 3);

  // Left arrow moves back over surrogate pair
  const left1 = applyMacFunctionKeyFallback(right1!.next, '\uF702');
  assert.ok(left1);
  assert.equal(left1!.next.selectionStart, 1);

  // Shift + Right extends selection
  const shiftRight = applyMacFunctionKeyFallback(
    { value: 'hello world', selectionStart: 2, selectionEnd: 2 },
    '\uF703',
    { shiftKey: true },
  );
  assert.ok(shiftRight);
  assert.deepEqual(
    [shiftRight!.next.selectionStart, shiftRight!.next.selectionEnd],
    [2, 3],
  );

  // Cmd + Right jumps to end of current line
  const cmdRight = applyMacFunctionKeyFallback(
    { value: 'first line\nsecond line', selectionStart: 2, selectionEnd: 2 },
    '\uF703',
    { metaKey: true },
  );
  assert.equal(cmdRight!.next.selectionStart, 10);

  // Up/Down arrow moves between lines preserving column
  const down = applyMacFunctionKeyFallback(
    { value: 'abcdef\nxy\n123456', selectionStart: 4, selectionEnd: 4 },
    '\uF701',
  );
  assert.equal(down!.next.selectionStart, 9); // clamped to end of 'xy'

  // Forward delete removes next codepoint without inserting box char
  const fwdDel = applyMacFunctionKeyFallback(
    { value: 'he🚀llo', selectionStart: 2, selectionEnd: 2 },
    '\uF728',
  );
  assert.ok(fwdDel);
  assert.equal(fwdDel!.mutatedValue, true);
  assert.equal(fwdDel!.next.value, 'hello');
  assert.equal(fwdDel!.next.selectionStart, 2);

  // Non-navigation function key (F5 = \uF708) suppresses insertion with null fallback
  const f5 = applyMacFunctionKeyFallback(
    { value: 'abc', selectionStart: 1, selectionEnd: 1 },
    '\uF708',
  );
  assert.equal(f5, null);
});

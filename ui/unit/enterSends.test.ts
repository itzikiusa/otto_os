import { test } from 'node:test';
import assert from 'node:assert/strict';
import { enterSends } from '../src/lib/enterSends.ts';

const key = (init: Partial<KeyboardEvent>): KeyboardEvent =>
  ({ key: 'Enter', shiftKey: false, isComposing: false, metaKey: false, ctrlKey: false, ...init }) as KeyboardEvent;

test('Enter sends with a mouse/trackpad; Shift+Enter is a new line', () => {
  assert.equal(enterSends(key({}), false), true);
  assert.equal(enterSends(key({ shiftKey: true }), false), false);
});

test('on a touch-first device Enter is a new line; ⌘/Ctrl+Enter still sends', () => {
  assert.equal(enterSends(key({}), true), false);
  assert.equal(enterSends(key({ metaKey: true }), true), true);
  assert.equal(enterSends(key({ ctrlKey: true }), true), true);
});

test('never sends mid-IME composition or on other keys', () => {
  assert.equal(enterSends(key({ isComposing: true }), false), false);
  assert.equal(enterSends(key({ key: 'a' }), false), false);
});

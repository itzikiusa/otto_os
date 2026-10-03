import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resizeDecision } from '../src/lib/components/termFlow.ts';

const tui = { force: false, preferDom: true };

test('a new grid is sent and, in a TUI pane, compacted', () => {
  assert.deepEqual(resizeDecision({ ...tui, sentChanged: true, localReflowed: true }), { send: true, compact: true });
});

test('A→B→A: same final grid but a local reflow still asks for a snapshot', () => {
  // The PTY never saw B, so nothing is sent — but xterm reflowed its buffer
  // to B and back, and only a server snapshot restores the TUI screen.
  assert.deepEqual(resizeDecision({ ...tui, sentChanged: false, localReflowed: true }), { send: false, compact: true });
});

test('nothing changed anywhere: no send, no snapshot', () => {
  assert.deepEqual(resizeDecision({ ...tui, sentChanged: false, localReflowed: false }), { send: false, compact: false });
});

test('a forced sync re-sends the same grid without a snapshot unless the buffer reflowed', () => {
  assert.deepEqual(resizeDecision({ force: true, preferDom: true, sentChanged: false, localReflowed: false }), { send: true, compact: false });
  assert.deepEqual(resizeDecision({ force: true, preferDom: true, sentChanged: false, localReflowed: true }), { send: true, compact: true });
});

test('plain shells never compact (their scrollback reflows natively)', () => {
  assert.deepEqual(resizeDecision({ force: false, preferDom: false, sentChanged: true, localReflowed: true }), { send: true, compact: false });
  assert.deepEqual(resizeDecision({ force: false, preferDom: false, sentChanged: false, localReflowed: true }), { send: false, compact: false });
});

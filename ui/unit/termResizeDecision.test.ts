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

// ── perf F1: only meaningful grid changes compact ─────────────────────────

const grid = (cols: number, rows: number) => ({ cols, rows });

test('a wider grid compacts (the widen-leaves-a-void case)', () => {
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(120, 40), next: grid(160, 40) }).compact, true);
});

test('a narrower grid, or a row change of at most 2, does not compact', () => {
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(160, 40), next: grid(120, 40) }).compact, false);
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(120, 40), next: grid(120, 42) }).compact, false);
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(120, 40), next: grid(120, 38) }).compact, false);
  // Still sent: the PTY must learn the new grid.
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(160, 40), next: grid(120, 40) }).send, true);
});

test('more than 2 rows either way compacts; an unknown previous grid compacts', () => {
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(120, 40), next: grid(120, 43) }).compact, true);
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: true, prev: grid(120, 40), next: grid(118, 30) }).compact, true);
  assert.equal(resizeDecision({ ...tui, sentChanged: true, localReflowed: false, prev: grid(0, 0), next: grid(100, 30) }).compact, true);
});

test('A→B→A still compacts with grids given (nothing sent, buffer reflowed)', () => {
  assert.equal(resizeDecision({ ...tui, sentChanged: false, localReflowed: true, prev: grid(120, 40), next: grid(120, 40) }).compact, true);
});

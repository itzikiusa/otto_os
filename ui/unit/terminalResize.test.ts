// architecture-2-01 (resize half): Terminal.svelte's `deferLocalResize` and
// `sendResize` decisions now live in termGridSync.ts as pure functions. These
// pin the behaviour the component had inline.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as termGrid from '../src/lib/components/termGrid.ts';

const { shouldDeferLocalResize, resizeSendStep } = loadSource(
  new URL('../src/lib/components/termGridSync.ts', import.meta.url),
  { './termGrid': termGrid },
  { document: { visibilityState: 'visible' } },
);

test('only a connected TUI (DOM-preferring) pane off the phone defers its local resize', () => {
  assert.equal(shouldDeferLocalResize({ preferDom: true, connected: true, phone: false }), true);
  assert.equal(shouldDeferLocalResize({ preferDom: false, connected: true, phone: false }), false, 'a shell resizes at once');
  assert.equal(shouldDeferLocalResize({ preferDom: true, connected: false, phone: false }), false, 'not before the socket is up');
  assert.equal(shouldDeferLocalResize({ preferDom: true, connected: true, phone: true }), false, 'the soft keyboard must not wait');
});

const g = (cols: number, rows: number) => ({ cols, rows });

test('a forced resize confirms at once, whatever the grid', () => {
  assert.equal(resizeSendStep({ force: true, timerPending: false, grid: g(80, 24), last: g(80, 24), localReflowed: false }), 'confirm-now');
  assert.equal(resizeSendStep({ force: true, timerPending: true, grid: g(100, 30), last: g(80, 24), localReflowed: true }), 'confirm-now');
});

test('an unchanged grid with nothing pending is skipped', () => {
  assert.equal(resizeSendStep({ force: false, timerPending: false, grid: g(80, 24), last: g(80, 24), localReflowed: false }), 'skip');
});

test('a changed grid, a pending timer or a local reflow schedules the settle timer', () => {
  assert.equal(resizeSendStep({ force: false, timerPending: false, grid: g(81, 24), last: g(80, 24), localReflowed: false }), 'schedule');
  assert.equal(resizeSendStep({ force: false, timerPending: false, grid: g(80, 25), last: g(80, 24), localReflowed: false }), 'schedule');
  assert.equal(resizeSendStep({ force: false, timerPending: true, grid: g(80, 24), last: g(80, 24), localReflowed: false }), 'schedule', 'a pending confirmation restarts');
  assert.equal(resizeSendStep({ force: false, timerPending: false, grid: g(80, 24), last: g(80, 24), localReflowed: true }), 'schedule', 'xterm reflowed: ask for a snapshot');
});

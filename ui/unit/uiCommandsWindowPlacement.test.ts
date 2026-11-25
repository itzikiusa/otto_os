import { test } from 'node:test';
import assert from 'node:assert/strict';
import { windowOpenPlan } from '../src/lib/uiCommands/windowPlacement.ts';

const base = { tauri: true, embedded: false, detachable: true, sideShows: false, mainShows: false, fallback: false };

test('desktop: a module not on screen opens in a NEW window, not over the side pane', () => {
  // The side pane may show the person's own work (or be too narrow): neither matters.
  assert.equal(windowOpenPlan(base), 'popout');
  assert.equal(windowOpenPlan({ ...base, detachable: false }), 'popout');
});

test('the side pane already showing it detaches (same document moves out)', () => {
  assert.equal(windowOpenPlan({ ...base, sideShows: true }), 'detach');
  assert.equal(windowOpenPlan({ ...base, embedded: true }), 'detach');
});

test('the main pane already showing it stays there', () => {
  assert.equal(windowOpenPlan({ ...base, mainShows: true }), 'main');
});

test('web: an explicit window request is refused honestly; the daemon auto-open falls back to the side pane', () => {
  const web = { ...base, tauri: false, detachable: false };
  assert.equal(windowOpenPlan(web), 'refuse');
  assert.equal(windowOpenPlan({ ...web, fallback: true }), 'side');
  assert.equal(windowOpenPlan({ ...web, embedded: true }), 'refuse');
  assert.equal(windowOpenPlan({ ...web, embedded: true, fallback: true }), 'side');
});

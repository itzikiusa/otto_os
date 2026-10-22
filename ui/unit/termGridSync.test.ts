// architecture-2-01: the grid feed + claim negotiation moved out of
// Terminal.svelte into a controller (termGridSync.ts). Same behaviour, now
// driven without a DOM: a foreign grid offers the take-over, an un-owned grid
// is auto-claimed by the visible primary pane, and a claim records the grid.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as termGrid from '../src/lib/components/termGrid.ts';

function setup(opts: { primary?: boolean; readOnly?: boolean; visible?: boolean } = {}) {
  const sent: any[] = [];
  const recorded: any[] = [];
  let foreign: any = null;
  let rebuilds = 0;
  const term = { cols: 120, rows: 40 };
  const doc = { visibilityState: opts.visible === false ? 'hidden' : 'visible' };
  const { TermGridSync } = loadSource(new URL('../src/lib/components/termGridSync.ts', import.meta.url), { './termGrid': termGrid }, { document: doc });
  const g = new TermGridSync({
    term: () => term,
    readOnly: () => !!opts.readOnly,
    primary: () => opts.primary ?? true,
    connected: () => true,
    send: (f: unknown) => sent.push(f),
    sent: (x: unknown) => recorded.push(x),
    rebuild: () => rebuilds++,
    setForeign: (x: unknown) => (foreign = x),
    foreign: () => foreign,
  });
  return { g, sent, recorded, term, doc, get foreign() { return foreign; }, get rebuilds() { return rebuilds; } };
}

test('an un-owned differing grid is claimed with this pane’s grid', () => {
  const t = setup();
  t.g.onFrame({ type: 'grid', cols: 80, rows: 24, owner: 'none' });
  assert.deepEqual(JSON.parse(JSON.stringify(t.sent)), [{ type: 'claim', cols: 120, rows: 40 }]);
  assert.deepEqual(JSON.parse(JSON.stringify(t.recorded)), [{ cols: 120, rows: 40 }]);
});

test('another viewer’s grid offers the take-over instead of claiming', () => {
  const t = setup();
  t.g.onFrame({ type: 'grid', cols: 80, rows: 24, owner: 'other' });
  assert.equal(t.sent.length, 0);
  assert.deepEqual({ ...t.foreign }, { cols: 80, rows: 24 });
  assert.equal(t.g.owed, true);
  // Back at our grid and ours again: ONE rebuild, the offer clears.
  t.g.onFrame({ type: 'grid', cols: 120, rows: 40, owner: 'self' });
  assert.equal(t.rebuilds, 1);
  assert.equal(t.foreign, null);
  assert.equal(t.g.owed, false);
});

test('read-only and non-primary viewers never claim; reset forgets the feed', () => {
  for (const o of [{ readOnly: true }, { primary: false }, { visible: false }]) {
    const t = setup(o);
    t.g.onFrame({ type: 'grid', cols: 80, rows: 24, owner: 'none' });
    assert.equal(t.sent.length, 0, JSON.stringify(o));
  }
  const t = setup();
  t.g.onFrame({ type: 'grid', cols: 80, rows: 24, owner: 'other' });
  t.g.reset();
  assert.equal(t.g.pty, null);
  assert.equal(t.g.owner, 'none');
  assert.equal(t.foreign, null);
});

test('window back: the primary pane re-takes a grid another viewer changed', () => {
  const t = setup();
  t.g.onFrame({ type: 'grid', cols: 80, rows: 24, owner: 'other' });
  t.g.onWindowBack();
  assert.deepEqual(JSON.parse(JSON.stringify(t.sent)), [{ type: 'claim', cols: 120, rows: 40 }]);
});

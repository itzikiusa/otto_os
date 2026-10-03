// Agents page round 2 (perf section 13, R2/R6): the coalesced status write,
// the statusMap prune rule, and when an exited background row may leave.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyStatusPatches, canDropExited, staleStatusIds, type StatusPatch } from '../src/lib/stores/sessionPatch.ts';

const row = (id: string, status = 'idle') => ({ id, status, last_active_at: '2026-01-01T00:00:00Z', title: id });

test('applyStatusPatches: one pass, one new array, untouched rows keep identity', () => {
  const list = [row('a'), row('b'), row('c')];
  const pending = new Map<string, StatusPatch>([
    ['a', { status: 'working', at: '2026-01-02T00:00:00Z' }],
    ['c', { status: 'exited', at: '2026-01-03T00:00:00Z' }],
    ['zz', { status: 'working', at: '2026-01-04T00:00:00Z' }],
  ]);
  const out = applyStatusPatches(list, pending);
  assert.notEqual(out, list);
  assert.deepEqual(out.map((s) => [s.id, s.status, s.last_active_at]), [
    ['a', 'working', '2026-01-02T00:00:00Z'],
    ['b', 'idle', '2026-01-01T00:00:00Z'],
    ['c', 'exited', '2026-01-03T00:00:00Z'],
  ]);
  assert.equal(out[1], list[1]);
  assert.equal(out[0].title, 'a');
  // The input is never mutated.
  assert.equal(list[0].status, 'idle');
});

test('applyStatusPatches: same array when nothing applies', () => {
  const list = [row('a')];
  assert.equal(applyStatusPatches(list, new Map()), list);
  assert.equal(applyStatusPatches(list, new Map([['x', { status: 'idle', at: 't' }]])), list);
});

test('applyStatusPatches: a burst keeps only the newest stamp per id (Map semantics)', () => {
  const pending = new Map<string, StatusPatch>();
  for (let k = 0; k < 200; k++) pending.set(`s${k % 3}`, { status: k % 2 ? 'idle' : 'working', at: `t${k}` });
  const out = applyStatusPatches([row('s0'), row('s1'), row('s2')], pending);
  assert.deepEqual(out.map((s) => s.last_active_at), ['t198', 't199', 't197']);
});

test('staleStatusIds: drops unknown terminal ids, keeps loaded and live ones', () => {
  const map = { a: 'idle', b: 'exited', c: 'working', d: 'running', e: 'reconnectable', f: 'exited' };
  assert.deepEqual(staleStatusIds(map, new Set(['a', 'f'])).sort(), ['b', 'e']);
  assert.deepEqual(staleStatusIds({}, new Set()), []);
});

test('canDropExited: held by a tab, a pane or an in-flight fetch → stays', () => {
  const none = { tabs: [], panes: [], ensuring: new Set<string>() };
  assert.equal(canDropExited('x', none), true);
  assert.equal(canDropExited('x', { ...none, tabs: ['x'] }), false);
  assert.equal(canDropExited('x', { ...none, panes: ['x'] }), false);
  assert.equal(canDropExited('x', { ...none, ensuring: new Set(['x']) }), false);
});

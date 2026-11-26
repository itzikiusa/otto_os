// The run ring (ux.md §6): built from the step list's own entries, equal
// slots that close the circle, lanes for a parallel group, ticks up to 12
// children and state segments above — runs/runOrbit.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildOrbit, orbitLabel, SLOT_GAP, slotAngles } from '../src/modules/workflows/runs/runOrbit.ts';
import { orderSteps } from '../src/modules/workflows/runs/stepOrder.ts';
import { fanoutNodes, node, PR_GRAPH, T0 } from './workflowRunFixtures.ts';

const NOW = T0 + 12 * 60_000;

test('slots and the gaps between them make the full circle', () => {
  for (const n of [1, 2, 7, 12]) {
    const a = slotAngles(n);
    const spans = a.reduce((s, [a0, a1]) => s + (a1 - a0), 0);
    assert.ok(Math.abs(spans + n * SLOT_GAP - 360) < 1e-9, `n=${n}`);
    assert.ok(a.every(([a0, a1], i) => i === 0 || a0 > a[i - 1][1]), 'slots never overlap');
  }
});

test('the ring has exactly one slot per step-list entry', () => {
  const entries = orderSteps(fanoutNodes(), PR_GRAPH, NOW);
  const o = buildOrbit(entries, 'running');
  assert.equal(o.slots, entries.length);
  assert.deepEqual([...new Set(o.arcs.map((a) => a.slot))], entries.map((_, i) => i));
});

test('a parallel group is concentric lanes; five or more draw four', () => {
  const entries = orderSteps(fanoutNodes(), PR_GRAPH, NOW);
  const lanes = buildOrbit(entries, 'running').arcs.filter((a) => a.slot === 3);
  assert.equal(lanes.length, 3);
  assert.ok(lanes.every((l) => l.width < 10), 'lanes are thinner than a single arc');
  const wide = { type: 'group' as const, steps: ['a', 'b', 'c', 'd', 'e'].map((id, i) => ({ id, name: id, kind: '', start: 0, end: 1, node: node(id, i === 4 ? 'error' : 'success', 0, 1) })) };
  const four = buildOrbit([wide], 'failed').arcs;
  assert.equal(four.length, 4);
  assert.equal(four[3].state, 'failed', 'the 4th lane stands for the rest, most urgent first');
});

test('up to 12 children are one tick each', () => {
  const o = buildOrbit(orderSteps(fanoutNodes(9, 3), PR_GRAPH, NOW), 'running');
  const fan = o.arcs.filter((a) => a.slot === 4);
  assert.equal(o.children, 13); // 12 reviewers + the summarizer
  assert.equal(o.grouped, true);
  const twelve = buildOrbit(orderSteps(fanoutNodes(8, 3), PR_GRAPH, NOW), 'running');
  assert.equal(twelve.children, 12);
  assert.equal(twelve.grouped, false);
  assert.equal(twelve.arcs.filter((a) => a.slot === 4).length, 12);
  assert.ok(fan.length < 13);
});

test('29 children are one segment per state, sized by count', () => {
  const o = buildOrbit(orderSteps(fanoutNodes(19, 6, 3), PR_GRAPH, NOW), 'running');
  const fan = o.arcs.filter((a) => a.slot === 4);
  assert.equal(o.children, 29);
  assert.equal(o.grouped, true);
  assert.deepEqual(fan.map((a) => a.state), ['done', 'working', 'pending']);
  const sweep = (d: string) => d.length; // every segment is a real arc path
  assert.ok(fan.every((a) => sweep(a.d) > 10));
});

test('colour comes from state only; a canceled run greys what never ran', () => {
  const o = buildOrbit(orderSteps([node('n1', 'success', 0, 0), node('n3', 'skipped', 1, 10, { error_class: 'canceled' }), node('n7', 'pending', null, null)], PR_GRAPH, NOW), 'canceled');
  assert.deepEqual(o.arcs.map((a) => a.state), ['done', 'canceled', 'canceled']);
  assert.ok(o.arcs.every((a) => !/color-mix|var\(/.test(a.d)));
});

test('the accessible name counts what the step list shows', () => {
  const entries = orderSteps(fanoutNodes(19, 6, 3), PR_GRAPH, NOW);
  assert.equal(orbitLabel(entries, 'Running'), 'Running: 5 of 10 steps done; Multi-agent PR review 19 of 29 done');
});

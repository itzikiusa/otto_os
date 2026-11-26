// The run ring (ux.md §6): built from the step list's own entries, equal
// slots that close the circle, lanes for a parallel group, ticks up to 12
// children and state segments above — runs/runOrbit.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildOrbit, orbitLabel, SLOT_GAP, slotAngles } from '../src/modules/workflows/runs/runOrbit.ts';
import { orderSteps } from '../src/modules/workflows/runs/stepOrder.ts';
import { fanoutNodes, node, PR_GRAPH, T0 } from './workflowRunFixtures.ts';

const NOW = T0 + 12 * 60_000;

/** An arc path's sweep in degrees, read back from its two end points (the ring's centre is 60,60). */
function sweepOf(d: string): number {
  const [x0, y0, , , , , , x1, y1] = d.replace(/^M /, '').replace(' A ', ' ').split(' ').map(Number);
  const deg = (x: number, y: number) => (Math.atan2(x - 60, 60 - y) * 180) / Math.PI;
  return (deg(x1, y1) - deg(x0, y0) + 360) % 360;
}

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
  // 19 done, 6 working, 3 queued + the queued summarizer: sweeps in that proportion (CR1-079).
  const [done, working, pending] = fan.map((a) => sweepOf(a.d));
  assert.ok(Math.abs(done / working - 19 / 6) < 0.02, `done/working ${done / working}`);
  assert.ok(Math.abs(working / pending - 6 / 4) < 0.02, `working/pending ${working / pending}`);
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

test('a long workflow with a 12-child fan-out never draws an inverted arc (CR1-045)', () => {
  const steps = Array.from({ length: 16 }, (_, i) => ({ type: 'step' as const, step: { id: `s${i}`, name: `s${i}`, kind: '', start: 0, end: 1, node: node(`s${i}`, 'success', 0, 1) } }));
  steps[5].step.node = node('s5', 'running', 0, null, { children: Array.from({ length: 12 }, (_, i) => ({ id: `k${i}`, label: `k${i}`, provider: 'codex', status: i < 4 ? 'success' : 'running' })) });
  const span = slotAngles(16)[0][1] - slotAngles(16)[0][0];
  const o = buildOrbit(steps as never, 'running');
  for (const a of o.arcs) assert.ok(sweepOf(a.d) > 0 && sweepOf(a.d) <= span + 0.1, `slot ${a.slot}: sweep ${sweepOf(a.d)} of ${span}`);
  // Twelve ticks would be under 1° each here: the fan-out draws as state segments.
  assert.equal(o.grouped, true);
  assert.deepEqual(o.arcs.filter((a) => a.slot === 5).map((a) => a.state), ['done', 'working']);
});

test('a hundred steps still close the circle with positive slots (CR1-045)', () => {
  const a = slotAngles(100);
  assert.ok(a.every(([a0, a1]) => a1 > a0));
  assert.ok(a[99][1] <= 360);
});

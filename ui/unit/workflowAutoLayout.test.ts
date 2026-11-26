// The editor's auto-layout (modules/workflows/editor/autoLayout.ts): top to
// bottom by longest path, barycentre inside a layer with the stored (y, x)
// order as the tie-break, steps no trigger reaches in a bottom row, and the
// zoom a graph opens at. Run on the scrubbed clones of the exported graphs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { autoLayout, openZoom, CARD_H, CARD_W, LAYER_PITCH, type Layout } from '../src/modules/workflows/editor/autoLayout.ts';
import type { WorkflowGraph } from '../src/lib/api/types';

const node = (id: string, kind = 'agent_prompt', x = 0, y = 0) => ({ id, kind, name: id, x, y, params: {} });
const edge = (source: string, target: string) => ({ id: `${source}-${target}`, source, target });
/** The canvas beside a 360 px inspector in a 1440 × 900 window. */
const CANVAS = { w: 1440 - 232 - 360, h: 900 - 46 - 24 };

function assertSound(g: WorkflowGraph, l: Layout): void {
  const placed = Object.keys(l.pos);
  assert.equal(placed.length, g.nodes.length, 'every step is drawn');
  for (let i = 0; i < placed.length; i++) {
    for (let j = i + 1; j < placed.length; j++) {
      const a = l.pos[placed[i]];
      const b = l.pos[placed[j]];
      const apart = Math.abs(a.x - b.x) >= CARD_W || Math.abs(a.y - b.y) >= CARD_H;
      assert.ok(apart, `${placed[i]} and ${placed[j]} overlap`);
    }
  }
  for (const e of g.edges) {
    if (l.loose.includes(e.source) || l.loose.includes(e.target)) continue;
    assert.ok(l.pos[e.target].y > l.pos[e.source].y, `${e.source} → ${e.target} goes down`);
  }
}

test('the optimized PR reviewer shape: forks side by side, joins below, fits the canvas at 100 %', () => {
  const g: WorkflowGraph = {
    nodes: ['start', 'jira', 'checkout', 'gather', 'gates', 'multi', 'cross', 'review', 'post', 'notify'].map((id) => node(id, id === 'start' ? 'manual_trigger' : 'agent_prompt')),
    edges: [
      edge('start', 'jira'), edge('jira', 'checkout'), edge('checkout', 'gather'), edge('checkout', 'gates'), edge('checkout', 'multi'),
      edge('multi', 'cross'), edge('gather', 'review'), edge('gates', 'review'), edge('cross', 'review'), edge('multi', 'review'),
      edge('review', 'post'), edge('post', 'notify'),
    ],
  };
  const l = autoLayout(g);
  assert.deepEqual(l.layers, [['start'], ['jira'], ['checkout'], ['gather', 'gates', 'multi'], ['cross'], ['review'], ['post'], ['notify']]);
  assert.equal(l.pos.review.y, 5 * LAYER_PITCH);
  assert.equal(l.pos.start.x, l.pos.gates.x, 'a fan is centred under its source');
  assert.equal(l.pos.cross.x, l.pos.multi.x, 'a lone child sits under its parent');
  assert.equal(l.pos.review.x, l.pos.gates.x, 'a join sits under the middle of what it joins');
  assert.equal(l.pos.gather.x, 0);
  assert.equal(l.width, l.pos.multi.x + CARD_W);
  assert.deepEqual(l.loose, []);
  assert.equal(l.looseY, null);
  assertSound(g, l);
  assert.equal(openZoom(l, CANVAS.w, CANVAS.h), 1);
});

test('a fork keeps the stored order: y first (legacy left-to-right graphs), then x, then graph index', () => {
  const g: WorkflowGraph = {
    nodes: [node('t', 'manual_trigger'), node('c', 'agent_prompt', 0, 300), node('a', 'agent_prompt', 500, 10), node('b', 'agent_prompt', 100, 10), node('d', 'agent_prompt', 100, 10)],
    edges: [edge('t', 'c'), edge('t', 'a'), edge('t', 'b'), edge('t', 'd')],
  };
  assert.deepEqual(autoLayout(g).layers[1], ['b', 'd', 'a', 'c']);
});

test('a child follows its parent’s column (barycentre) before the stored order', () => {
  const g: WorkflowGraph = {
    nodes: [node('t', 'manual_trigger'), node('left', 'x', 0, 0), node('right', 'x', 0, 1), node('under-right', 'x', 0, 0), node('under-left', 'x', 0, 1)],
    edges: [edge('t', 'left'), edge('t', 'right'), edge('right', 'under-right'), edge('left', 'under-left')],
  };
  assert.deepEqual(autoLayout(g).layers[2], ['under-left', 'under-right']);
});

test('a step whose incoming edge was deleted, and a cycle, go to the "Not connected" row at the bottom', () => {
  const g: WorkflowGraph = {
    nodes: [node('t', 'manual_trigger'), node('a'), node('orphan', 'x', 5, 5), node('p'), node('q')],
    edges: [edge('t', 'a'), edge('orphan', 'a'), edge('p', 'q'), edge('q', 'p')],
  };
  const l = autoLayout(g);
  assert.deepEqual(l.layers, [['t'], ['a']]);
  assert.deepEqual(l.loose, ['p', 'q', 'orphan']);
  assert.ok(l.looseY! > l.pos.a.y + CARD_H, 'below the last layer, with room for its label');
  for (const id of l.loose) assert.equal(l.pos[id].y, l.looseY);
  assert.equal(l.height, l.looseY! + CARD_H);
  assertSound(g, l);
});

test('without a trigger the sources start the graph', () => {
  const l = autoLayout({ nodes: [node('b'), node('a')], edges: [edge('a', 'b')] });
  assert.deepEqual(l.layers, [['a'], ['b']]);
});

test('the scrubbed clones of the exported graphs: sound, and opened at a readable zoom', () => {
  const dir = new URL('../../crates/otto-server/tests/fixtures/workflows/compat/clones/', import.meta.url);
  const files = readdirSync(dir).filter((f) => f.endsWith('.json'));
  assert.ok(files.length >= 5);
  for (const f of files) {
    const g = (JSON.parse(readFileSync(new URL(f, dir), 'utf8')) as { graph: WorkflowGraph }).graph;
    const l = autoLayout(g);
    assertSound(g, l);
    assert.deepEqual(l.loose, [], `${f}: every step hangs off the trigger`);
    const z = openZoom(l, CANVAS.w, CANVAS.h);
    assert.ok(z >= 0.6 && z <= 1, `${f}: zoom ${z}`);
  }
});

test('a graph that would shrink below 60 % opens at 80 % (it scrolls)', () => {
  assert.equal(openZoom({ width: 208, height: 3000 }, CANVAS.w, CANVAS.h), 0.8);
  assert.equal(openZoom({ width: 208, height: 56 }, CANVAS.w, CANVAS.h), 1);
});

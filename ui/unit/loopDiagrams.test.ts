import assert from 'node:assert/strict';
import { test } from 'node:test';
import { diagramLayout, type DiagramNode } from '../src/modules/loops/diagramLayout.ts';
import { iterationNodes, iterationEdges } from '../src/modules/loops/iterationDiagram.ts';
import { componentFunctions } from './componentFunctions.ts';

const nodes: DiagramNode[] = ['scope', 'api', 'ui', 'verify'].map(id => ({ id, title: id }));
const edges = [{ from: 'scope', to: 'api' }, { from: 'scope', to: 'ui' }, { from: 'api', to: 'verify' }, { from: 'ui', to: 'verify' }];

test('DAG diagram uses every real dependency and places joins after their prerequisites', () => {
  const layout = diagramLayout(nodes.slice().reverse(), edges, 720);
  const positions = new Map(layout.nodes.map(node => [node.id, node]));
  assert.equal(layout.edges.length, 4);
  for (const edge of edges) assert.ok(positions.get(edge.from)!.y < positions.get(edge.to)!.y);
  assert.equal(positions.get('api')!.y, positions.get('ui')!.y);
  assert.notEqual(positions.get('api')!.x, positions.get('ui')!.x);
});

test('phone layout fits all nodes and invalid cyclic drafts remain selectable', () => {
  for (const width of [220, 300, 720]) {
    const layout = diagramLayout(nodes, edges, width);
    for (const node of layout.nodes) {
      assert.ok(node.x >= 0 && node.x + node.width <= width, `${node.id} at ${width}`);
      assert.ok(node.y >= 0 && node.y + node.height <= layout.height);
    }
  }
  const invalid = diagramLayout(nodes, [...edges, { from: 'verify', to: 'scope' }], 300);
  assert.equal(invalid.nodes.length, nodes.length, 'a cycle must not hide the tasks needed to repair it');
});

test('standard loop has a visible feedback edge and honest paused/waiting states', () => {
  assert.deepEqual(iterationEdges.at(-1), { from: 'digesting', to: 'planning', feedback: true });
  const paused = iterationNodes({ status: 'paused', phase: 'waiting' });
  assert.equal(paused.find(node => node.id === 'executing')!.note, 'Paused');
  assert.ok(paused.every(node => node.tone !== 'running'));
  const running = iterationNodes({ status: 'running', phase: 'waiting' });
  assert.equal(running.find(node => node.id === 'executing')!.note, 'Waiting on an agent');
  assert.equal(running.filter(node => node.tone === 'running').length, 1);
  assert.ok(iterationNodes().every(node => !node.tone), 'creation preview must not invent progress');
  const layout = diagramLayout(iterationNodes(), iterationEdges, 300, true);
  assert.ok(layout.edges.some(edge => edge.feedback && edge.path.length > 0));
});

test('diagram keyboard navigation moves focus in node order and supports Home/End', () => {
  let focused = -1;
  const buttons = [0, 1, 2].map(index => ({ focus: () => { focused = index; }, parentElement: { querySelectorAll: () => buttons } }));
  const context = componentFunctions(new URL('../src/modules/loops/ExecutionDiagram.svelte', import.meta.url), ['moveFocus'], {});
  let prevented = 0;
  context.moveFocus({ key: 'ArrowRight', currentTarget: buttons[0], preventDefault: () => prevented++ });
  assert.equal(focused, 1);
  context.moveFocus({ key: 'End', currentTarget: buttons[1], preventDefault: () => prevented++ });
  assert.equal(focused, 2);
  context.moveFocus({ key: 'Home', currentTarget: buttons[2], preventDefault: () => prevented++ });
  assert.equal(focused, 0);
  context.moveFocus({ key: 'Tab', currentTarget: buttons[0], preventDefault: () => prevented++ });
  assert.equal(prevented, 3, 'Tab must retain normal browser focus behavior');
});

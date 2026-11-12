import { test } from 'node:test';
import assert from 'node:assert/strict';
import { groupNodeTypes } from '../src/modules/workflows/nodePalette.ts';
import type { NodeTypeSpec } from '../src/lib/api/types';

const spec = (kind: string, label: string, category: string, description: string): NodeTypeSpec =>
  ({ kind, label, category, description, inputs: 1, outputs: 1, color: '', icon: '' });
const CATALOG = [
  spec('manual_trigger', 'Manual Trigger', 'Triggers', 'Starts the workflow.'),
  spec('agent_prompt', 'Agent', 'AI', 'Run an agent turn with a prompt.'),
  spec('delay', 'Delay', 'Flow', 'Wait a number of milliseconds.'),
  spec('review_run', 'Review Run', 'AI', 'Multi-agent code review.'),
];

test('the palette groups by category in catalog order', () => {
  const groups = groupNodeTypes(CATALOG, '');
  assert.deepEqual(groups.map((g) => g.category), ['Triggers', 'AI', 'Flow']);
  assert.deepEqual(groups[1].types.map((t) => t.kind), ['agent_prompt', 'review_run']);
});

test('the filter matches label, description, category and kind, every word', () => {
  assert.deepEqual(groupNodeTypes(CATALOG, 'code review').flatMap((g) => g.types.map((t) => t.kind)), ['review_run']);
  assert.deepEqual(groupNodeTypes(CATALOG, 'MILLISECONDS').flatMap((g) => g.types.map((t) => t.kind)), ['delay']);
  assert.deepEqual(groupNodeTypes(CATALOG, 'flow').map((g) => g.category), ['Triggers', 'Flow']);
  assert.deepEqual(groupNodeTypes(CATALOG, 'nothing like this'), []);
});

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { goalTemplates, templateDraft } from '../src/modules/loops/templates.ts';
import { graphError, graphLayers, removeGraphNode } from '../src/modules/loops/graph.ts';

test('seven local templates produce editable bounded graphs with provider-default models', () => {
  assert.equal(goalTemplates.length, 7);
  for (const template of goalTemplates) {
    const draft = templateDraft(template.id, 'codex', 'both');
    const graph = draft.suggested_config.graph!;
    assert.equal(graphError(graph, draft.definition.acceptance_criteria), null, template.id);
    assert.ok(draft.suggested_limits.max_iterations >= graph.nodes.length + 2);
    assert.equal(draft.suggested_config.allow_commits, false);
    for (const role of ['planner', 'evaluator', 'digester', 'definer'] as const) {
      assert.equal(draft.suggested_config[role].provider, 'codex');
      assert.equal(draft.suggested_config[role].model, '');
    }
    const second = templateDraft(template.id, 'claude', 'both');
    draft.definition.acceptance_criteria[0].text = 'edited';
    assert.notEqual(second.definition.acceptance_criteria[0].text, 'edited');
  }
});

test('POC scope changes concrete acceptance criteria and research stays research', () => {
  const backend = templateDraft('poc', 'codex', 'backend');
  const frontend = templateDraft('poc', 'codex', 'frontend');
  assert.notDeepEqual(backend.definition.acceptance_criteria, frontend.definition.acceptance_criteria);
  assert.equal(templateDraft('research', 'codex').suggested_config.mode, 'research');
});

test('graph validation rejects cyclic dependencies and criteria removed from the draft', () => {
  const draft = templateDraft('general', 'codex');
  const graph = draft.suggested_config.graph!;
  graph.nodes[0].depends_on = [graph.nodes.at(-1)!.id];
  assert.match(graphError(graph, draft.definition.acceptance_criteria)!, /cycle/i);
  graph.nodes[0].depends_on = [];
  assert.match(graphError(graph, draft.definition.acceptance_criteria.slice(1))!, /criterion/i);
});

test('every acceptance criterion is assigned and research report belongs to its final task', () => {
  const draft = templateDraft('research', 'codex');
  assert.ok(draft.suggested_config.graph!.nodes.at(-1)!.criterion_ids.includes('otto-research-report'));
  draft.definition.acceptance_criteria.push({ id: 'extra', text: 'Extra check', verify: 'Inspect result', verify_kind: 'agent' });
  assert.match(graphError(draft.suggested_config.graph, draft.definition.acceptance_criteria)!, /assign/i);
});

test('human approvals can only gate terminal graph tasks', () => {
  const draft = templateDraft('general', 'codex');
  draft.definition.acceptance_criteria[0].verify_kind = 'human';
  assert.match(graphError(draft.suggested_config.graph, draft.definition.acceptance_criteria)!, /human.*final task/i);
  draft.definition.acceptance_criteria[0].verify_kind = 'agent';
  draft.definition.acceptance_criteria[2].verify_kind = 'human';
  assert.equal(graphError(draft.suggested_config.graph, draft.definition.acceptance_criteria), null);
});

test('dependency preview uses graph edges, not array order; removing a node clears incoming references', () => {
  const draft = templateDraft('general', 'codex');
  const graph = draft.suggested_config.graph!;
  const ids = graph.nodes.map(n => n.id);
  graph.nodes.reverse();
  assert.deepEqual(graphLayers(graph).map(layer => layer.map(n => n.id)), ids.map(id => [id]));
  const removed = removeGraphNode(graph, ids[1]);
  assert.ok(removed.nodes.every(n => !n.depends_on.includes(ids[1])));
  assert.equal(graph.nodes.length, ids.length, 'the helper must preserve its input draft');
});

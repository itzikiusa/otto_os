// The editor's step rules (modules/workflows/editor/stepRules.ts): continue
// an earlier step's thread only from a same-provider ancestor agent step,
// name continuers that can run at once, "+ Step" wiring, reviewer picks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  continueChoices, continueClashes, missingSkills, resolvedReviewerPick, reviewerPick, wireAfter, withNodePick, withReviewerPick,
} from '../src/modules/workflows/editor/stepRules.ts';

const n = (id: string, params: Record<string, unknown> = {}, kind = 'agent_prompt', name = id) => ({ id, kind, name, x: 0, y: 0, params });
const e = (source: string, target: string, condition?: string) => ({ id: `${source}>${target}`, source, target, ...(condition ? { condition } : {}) });
const newId = (s: string, t: string) => `${s}>${t}`;

test('continue: only ancestor agent steps; another provider is listed with the reason', () => {
  const g = {
    nodes: [n('t', {}, 'manual_trigger'), n('co', { provider: 'claude' }, 'agent_prompt', 'Check out PR'), n('gc', { provider: 'codex' }, 'agent_prompt', 'Gather context'),
      n('cond', {}, 'condition'), n('me', {}), n('later', {})],
    edges: [e('t', 'co'), e('co', 'gc'), e('gc', 'cond'), e('cond', 'me'), e('me', 'later')],
  };
  const choices = continueChoices(g, g.nodes[4], 'claude');
  assert.deepEqual(choices.map((c) => c.node.id), ['co', 'gc'], 'no condition, nothing after it');
  assert.equal(choices[0].reason, null);
  assert.equal(choices[1].reason, 'Gather context is a Codex step — pick a Claude step');
  assert.deepEqual(continueChoices(g, g.nodes[1], 'claude'), [], 'nothing earlier to continue');
});

test('continue: two steps that can run at once may not continue the same thread', () => {
  const g = {
    nodes: [n('src', {}, 'agent_prompt', 'Check out PR'), n('a', { continue_from: 'src' }, 'agent_prompt', 'Gather context'), n('b', { continue_from: 'src' }, 'agent_prompt', 'Go PR gates'), n('c', { continue_from: 'src' })],
    edges: [e('src', 'a'), e('src', 'b'), e('a', 'c')],
  };
  assert.deepEqual(continueClashes(g), [
    'Gather context and Go PR gates both continue Check out PR’s thread and can run at the same time. Make one of them start a new thread.',
    'Go PR gates and c both continue Check out PR’s thread and can run at the same time. Make one of them start a new thread.',
  ]);
  assert.deepEqual(continueClashes({ nodes: g.nodes.slice(0, 2), edges: [e('src', 'a')] }), []);
});

test('+ Step goes after the step and takes over its plain edges; conditional edges stay', () => {
  const edges = [e('a', 'b'), e('a', 'c', 'output.result == true'), e('x', 'a')];
  assert.deepEqual(wireAfter(edges, 'a', 'new', newId), [
    { id: 'new>b', source: 'new', target: 'b' }, e('a', 'c', 'output.result == true'), e('x', 'a'), { id: 'a>new', source: 'a', target: 'new' },
  ]);
  assert.deepEqual(wireAfter(edges, null, 'new', newId), edges);
});

test('reviewer picks: a row writes picks[provider]; resolution falls back to the review-wide default', () => {
  const row = { lens: 'correctness', providers: ['claude', 'codex'] };
  const set = withReviewerPick(row, 'codex', { model: 'gpt-5.1-codex', effort: 'high' });
  assert.deepEqual(set, { lens: 'correctness', providers: ['claude', 'codex'], picks: { codex: { model: 'gpt-5.1-codex', effort: 'high' } } });
  assert.deepEqual(withReviewerPick(set, 'codex', {}), row, 'clearing removes the key (and an empty picks)');
  assert.deepEqual(reviewerPick({ lens: 'x', providers: ['claude'], model: 'opus', effort: 'low' }, 'claude'), { model: 'opus', effort: 'low' });
  assert.deepEqual(reviewerPick({ lens: 'x', providers: ['claude', 'codex'], model: 'opus' }, 'claude'), {}, 'row model/effort only bind a one-provider lens');
  assert.deepEqual(resolvedReviewerPick(row, 'claude', { claude: { effort: 'max' } }), { effort: 'max' });
  assert.deepEqual(resolvedReviewerPick(set, 'codex', { codex: { effort: 'max' } }), { model: 'gpt-5.1-codex', effort: 'high' });
  assert.deepEqual(withNodePick(undefined, 'codex', { effort: 'low' }), { codex: { effort: 'low' } });
  assert.equal(withNodePick({ codex: { effort: 'low' } }, 'codex', {}), undefined);
});

test('skills not installed for the provider are named (nothing known yet: none)', () => {
  assert.deepEqual(missingSkills(['okf-authoring', 'gone'], new Set(['okf-authoring'])), ['gone']);
  assert.deepEqual(missingSkills(['x'], null), []);
});

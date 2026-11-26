// The editor's step rules (modules/workflows/editor/stepRules.ts): continue
// an earlier step's thread only from a same-provider ancestor agent step,
// name continuers that can run at once, "+ Step" wiring, reviewer picks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  continueChoices, continueClashes, issueBlocks, missingSkills, slug, stepFile, resolvedReviewerPick, reviewerPick, wireAfter, withNodePick, withReviewerPick,
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
  assert.deepEqual(reviewerPick({ lens: 'x', providers: ['claude', 'codex'], model: 'opus' }, 'claude'), { model: 'opus', effort: undefined }, 'row model/effort bind the primary provider');
  assert.deepEqual(reviewerPick({ lens: 'x', providers: ['claude', 'codex'], model: 'opus' }, 'codex'), {}, 'a secondary provider uses picks, else the review-wide default');
  assert.deepEqual(reviewerPick({ lens: 'x', providers: ['claude', 'codex'], model: 'opus', picks: { claude: { effort: 'low' } } }, 'claude'), { effort: 'low' }, 'picks[provider] first');
  assert.deepEqual(resolvedReviewerPick({ lens: 'x', providers: ['claude'] }, 'claude', { claude: { effort: 'max' } }), { effort: 'max' }, 'an empty own pick falls to the node default');
  assert.deepEqual(resolvedReviewerPick(row, 'claude', { claude: { effort: 'max' } }), { effort: 'max' });
  assert.deepEqual(resolvedReviewerPick(set, 'codex', { codex: { effort: 'max' } }), { model: 'gpt-5.1-codex', effort: 'high' });
  assert.deepEqual(withNodePick(undefined, 'codex', { effort: 'low' }), { codex: { effort: 'low' } });
  assert.equal(withNodePick({ codex: { effort: 'low' } }, 'codex', {}), undefined);
});

test('the step file a step writes: the engine’s order and slug, the trigger is step 1', () => {
  const g = {
    nodes: [n('t', {}, 'manual_trigger', 'Start'), n('c', {}, 'agent_prompt', 'Check out PR'), n('j', {}, 'agent_prompt', 'Fetch Jira ticket'), n('g', {}, 'agent_prompt', 'Gather context from the Vault')],
    edges: [e('t', 'j'), e('j', 'c'), e('c', 'g')],
  };
  assert.equal(stepFile(g, 'g'), 'step4-gather-context-from-the-vault.md');
  assert.equal(stepFile(g, 'c'), 'step3-check-out-pr.md');
  assert.equal(stepFile({ nodes: g.nodes, edges: [...g.edges, e('g', 't')] }, 'g'), null, 'a cycle has no number');
  assert.equal(slug('  Ünïcode — Only!! '), 'n-code-only');
  assert.equal(slug('***'), 'step');
  assert.equal(slug('x'.repeat(39) + ' y'), 'x'.repeat(39));
});

test('skills not installed for the provider are named (nothing known yet: none)', () => {
  assert.deepEqual(missingSkills(['okf-authoring', 'gone'], new Set(['okf-authoring'])), ['gone']);
  assert.deepEqual(missingSkills(['x'], null), []);
});

test('Default on a legacy reviewer row clears its primary provider\'s model and effort (CR1-059)', () => {
  const legacy = { lens: 'correctness', providers: ['codex', 'claude'], model: 'gpt-5', effort: 'high' };
  const reset = withReviewerPick(legacy, 'codex', {});
  const pick = reviewerPick(reset, 'codex');
  assert.equal(pick.model ?? pick.effort, undefined, 'the legacy fields no longer win');
  assert.equal('model' in reset || 'effort' in reset, false);
  // Another provider's pick leaves the primary's legacy fields alone.
  const other = withReviewerPick(legacy, 'claude', {});
  assert.deepEqual(reviewerPick(other, 'codex'), { model: 'gpt-5', effort: 'high' });
});

test('save is blocked only by a save issue; a run by save and run issues; a warning never (DR-028, CR1-019/064)', () => {
  const at = (cls: 'save' | 'run' | 'warning') => ({ node_id: 'a', edge_id: null, field: 'prompt', message: cls, class: cls });
  assert.deepEqual(['save', 'run', 'warning'].map((c) => issueBlocks(at(c as 'save'), 'save')), [true, false, false]);
  assert.deepEqual(['save', 'run', 'warning'].map((c) => issueBlocks(at(c as 'save'), 'run')), [true, true, false]);
});

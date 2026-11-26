import { test } from 'node:test';
import assert from 'node:assert/strict';
import { afterMarks, canApply, estimateLines, groupChanges, initialSelection, savingText } from '../src/modules/workflows/optimize/optimizeView.ts';
import type { OptimizeChange } from '../src/lib/api/types';

const ch = (id: string, effect: OptimizeChange['effect'], preselected: boolean, rule: OptimizeChange['rule'] = 'parallel', nodes = ['n1']): OptimizeChange =>
  ({ id, rule, nodes, title: id, why: '', effect, preselected });
const changes = [ch('a', 'may_change_output', true), ch('b', 'same_contract', true, 'go_gates_script'), ch('c', 'changes_findings', false, 'review_declared_lenses'), ch('d', 'may_change_output', true, 'single_repo_skip', ['n3'])];

test('changes are grouped by what they may change, in a fixed order', () => {
  assert.deepEqual(groupChanges(changes).map((g) => [g.label, g.changes.map((c) => c.id)]), [
    ['Same result, faster', ['b']],
    ['Faster; the output may read differently', ['a', 'd']],
    ['May change findings', ['c']],
  ]);
  assert.deepEqual(groupChanges([]), []);
});

test('the first selection is the server\'s; the lens hold starts unticked; Apply needs one tick', () => {
  assert.deepEqual(initialSelection(changes), ['a', 'b', 'd']);
  assert.equal(canApply([]), false);
  assert.equal(canApply(['a']), true);
});

test('saving and estimate words', () => {
  assert.equal(savingText(282_000), '−4.7m');
  assert.equal(savingText(40_000), '−40s');
  assert.equal(savingText(undefined), '');
  const few = estimateLines({ runs: 2 });
  assert.equal(few.none, 'Not enough runs yet to estimate time (2 so far). The changes below are still checked.');
  assert.equal(few.typical, undefined);
  const e = estimateLines({ runs: 25, typical_before_ms: 17 * 60_000, typical_after_ms: 12 * 60_000, slow_before_ms: 29 * 60_000, slow_after_ms: 24 * 60_000 });
  assert.equal(e.typical, 'about 17m → 12m (typical)');
  assert.equal(e.slow, 'slow runs 29m → 24m');
  assert.equal(e.basis, 'from 25 runs');
});

test('the After view marks new steps and the steps a ticked change touches', () => {
  const node = (id: string) => ({ id, kind: 'agent_prompt', name: id, x: 0, y: 0, params: null });
  const base = { nodes: [node('n1'), node('n3')], edges: [] };
  const after = { nodes: [node('n1'), node('n3'), node('draft')], edges: [] };
  assert.deepEqual(afterMarks(base, after, changes, ['a']), { n1: 'side by side', draft: 'new' });
  assert.deepEqual(afterMarks(base, after, changes, ['d']), { n3: 'skipped on one repo', draft: 'new' });
});

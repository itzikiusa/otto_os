// The Goal form's fields ⇄ GoalSpec (goal.md §3.3, §5.3; GL21.1) — workflows/goal/goalForm.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  applyTemplate, emptyFields, firstProblem, fromLoop, fromStaged, moveCriterion, newCriterion, setGoal, titleFromGoal, toSpec, willLine,
} from '../src/modules/workflows/goal/goalForm.ts';
import type { GoalLoop } from '../src/lib/api/types.ts';

function filled() {
  const f = setGoal(emptyFields(), 'Make the export stream rows\nKeep the CSV contract.');
  return { ...f, repoId: 'r1', criteria: [{ id: 'c1', text: ' Tests pass ', verify: 'command' as const, how: 'ignored', command: ' go test ./... ' }, { id: 'c2', text: 'Memory stays flat', verify: 'agent' as const, how: ' profile it ' }] };
}

test('the title follows the goal until it is edited', () => {
  let f = setGoal(emptyFields(), 'Fix the flaky login test\nIt fails on CI.');
  assert.equal(f.title, 'Fix the flaky login test');
  f = setGoal({ ...f, title: 'Mine', titleEdited: true }, 'Something else');
  assert.equal(f.title, 'Mine');
  assert.equal(titleFromGoal('x'.repeat(200)).length, 120);
});

test('a filled form builds the spec the goals route takes', () => {
  const spec = toSpec(filled());
  assert.deepEqual(spec, {
    title: 'Make the export stream rows',
    goal: 'Make the export stream rows\nKeep the CSV contract.',
    criteria: [
      { id: 'c1', text: 'Tests pass', verify: 'command', how: '', command: 'go test ./...' },
      { id: 'c2', text: 'Memory stays flat', verify: 'agent', how: 'profile it' },
    ],
    work: { kind: 'build', repo_id: 'r1' },
    budget: { iterations: 5, active_s: 3600, step_timeout_s: 1200 },
  });
});

test('models, review, base, research and the Insights source reach the spec only when set', () => {
  const spec = toSpec({ ...filled(), base: 'main', worker: { provider: 'codex', model: 'gpt-x', effort: '' }, checker: { provider: '', model: '', effort: 'high' }, finalReview: true, from: 'insights' });
  assert.deepEqual(spec.work, { kind: 'build', repo_id: 'r1', base: 'main' });
  assert.deepEqual(spec.worker, { provider: 'codex', model: 'gpt-x' });
  assert.deepEqual(spec.checker, { effort: 'high' });
  assert.equal(spec.final_review, true);
  assert.equal(spec.from, 'insights');
  assert.deepEqual(toSpec({ ...filled(), work: 'research' }).work, { kind: 'research' });
});

test('the budget is clamped into the route ranges', () => {
  assert.deepEqual(toSpec({ ...filled(), iterations: 99, activeS: 60 }).budget, { iterations: 50, active_s: 900, step_timeout_s: 1200 });
});

test('Start goal stays off until the goal, every criterion and the repository are there', () => {
  assert.equal(firstProblem(emptyFields())?.field, 'goal');
  assert.equal(firstProblem({ ...filled(), criteria: [] })?.field, 'criteria');
  assert.match(firstProblem({ ...filled(), criteria: [{ id: 'c1', text: ' ', verify: 'agent', how: '' }] })!.text, /Criterion 1/);
  assert.match(firstProblem({ ...filled(), criteria: [{ id: 'c1', text: 'ok', verify: 'command', how: '' }] })!.text, /command/);
  assert.equal(firstProblem({ ...filled(), repoId: '' })?.field, 'work');
  assert.equal(firstProblem({ ...filled(), repoId: '', work: 'research' }), null);
  assert.equal(firstProblem(filled()), null);
});

test('criteria get the next free id and move with the buttons', () => {
  const list = [{ id: 'c1', text: 'a', verify: 'agent' as const, how: '' }, { id: 'c3', text: 'b', verify: 'agent' as const, how: '' }];
  assert.equal(newCriterion(list).id, 'c4');
  assert.deepEqual(moveCriterion(list, 1, -1).map((c) => c.id), ['c3', 'c1']);
  assert.deepEqual(moveCriterion(list, 0, -1).map((c) => c.id), ['c1', 'c3']);
});

test('a template fills the criteria and the budget, never the goal', () => {
  const f = applyTemplate(filled(), 'go');
  assert.equal(f.goal, filled().goal);
  assert.equal(f.criteria.length, 3);
  assert.ok(f.criteria.every((c) => c.verify === 'agent' && c.how));
  assert.equal(f.iterations, 5);
  assert.equal(f.activeS, 7200); // 75 minutes → the next choice up
  assert.equal(applyTemplate(filled(), 'research').work, 'research');
});

test('the Insights handoff fills the goal and the title and marks the source', () => {
  const f = fromStaged({ title: 'Cut review wait', prompt: 'Cut review wait\nMetric: wait (40m → 10m).' });
  assert.equal(f.goal, 'Cut review wait\nMetric: wait (40m → 10m).');
  assert.equal(f.title, 'Cut review wait');
  assert.equal(toSpec({ ...f, repoId: 'r' }).from, 'insights');
});

function loop(over: Partial<GoalLoop> = {}): GoalLoop {
  return {
    id: 'L1', workspace_id: 'w', name: 'Green nightly build', repo_path: '/src/app',
    definition: { title: 'Nightly', summary: 'Keep the nightly build green', objectives: [], constraints: [], out_of_scope: [], success_signal: '',
      acceptance_criteria: [
        { id: 'Build OK', text: 'The build passes', verify: 'exit 0', verify_kind: 'command', verify_cmd: 'make build' },
        { id: 'c2', text: 'No new warnings', verify: 'read the log', verify_kind: 'manual' },
        { id: 'c3', text: 'Looks right', verify: 'open the page', verify_kind: 'human' },
      ] },
    limits: { max_iterations: 6, max_runtime_secs: 7200, per_phase_timeout_secs: 900, max_attempts_per_executor: 3 },
    config: { executors: [{ name: 'Executor', provider: 'codex', model: 'm1', prompt_extra: '' }], evaluator: { provider: 'claude', model: '', prompt: '' },
      planner: { provider: '', model: '', prompt: '' }, digester: { provider: '', model: '', prompt: '' }, definer: { provider: '', model: '', prompt: '' }, require_review: true },
    status: 'paused', phase: 'done', iterations_started: 4, current_iteration: 4, progress_pct: 50, context_digest: '',
    worktree_path: '/data/goal-loops/L1/work', elapsed_secs: 5400, cost_usd: 0, created_by: 'u', created_at: '', updated_at: '',
    ...over,
  } as GoalLoop;
}

test('Continue as a goal carries the loop over with the budget it had left', () => {
  const f = fromLoop(loop(), [{ id: 'r9', path: '/src/app' }]);
  assert.equal(f.goal, 'Keep the nightly build green');
  assert.equal(f.title, 'Green nightly build');
  assert.deepEqual(f.criteria, [
    { id: 'build-ok', text: 'The build passes', verify: 'command', how: '', command: 'make build' },
    { id: 'c2', text: 'No new warnings', verify: 'agent', how: 'read the log' },
    { id: 'c3', text: 'Looks right', verify: 'human', how: 'open the page' },
  ]);
  assert.equal(f.iterations, 2);
  assert.equal(f.activeS, 1800);
  assert.equal(f.repoId, 'r9');
  assert.deepEqual(f.worker, { provider: 'codex', model: 'm1', effort: '' });
  assert.equal(f.checker.provider, 'claude');
  assert.equal(f.finalReview, true);
  const spec = toSpec(f);
  assert.equal(spec.continue_loop_id, 'L1');
  assert.deepEqual(spec.work, { kind: 'build', repo_id: 'r9' });
  assert.match(willLine(f, 'app'), /kept work \(\/data\/goal-loops\/L1\/work\)/);
});

test('a used-up loop still continues with at least 1 iteration and 15 minutes; research stays research', () => {
  const f = fromLoop(loop({ iterations_started: 9, elapsed_secs: 99_999, config: { ...loop().config, mode: 'research' } }));
  assert.equal(f.iterations, 1);
  assert.equal(f.activeS, 900);
  assert.deepEqual(toSpec(f).work, { kind: 'research' });
});


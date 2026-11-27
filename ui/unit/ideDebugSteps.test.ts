// The Debug All step line (rc.6): one chip per finished step with its result,
// the "Step 2 of 4" line for the live step, the end summary, the failed tests
// with the single-test launch their Debug starts, and the console separator a
// chip scrolls to. Hidden for an ordinary one-step session.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  currentStepLine,
  failedTestLaunch,
  failedTests,
  runSummary,
  showSteps,
  stepChips,
  stepSeparatorPrefix,
} from '../src/modules/ide/debug/ideDebugSteps.ts';
import type { IdeDebugStep } from '../src/lib/api/types/ide.ts';

const step = (label: string, state: IdeDebugStep['state'], extra: Partial<IdeDebugStep> = {}): IdeDebugStep => ({
  label,
  state,
  program: `/ws/${label.split(' ')[0]}.csproj`,
  ...extra,
});

const run: IdeDebugStep[] = [
  step('Core.Tests (net10.0)', 'passed', { summary: '42 passed' }),
  step('Api.Tests (net10.0)', 'running', { test_framework: 'net10.0' }),
  step('Api.Tests (net11.0)', 'pending', { test_framework: 'net11.0' }),
  step('Mtp.Tests (net10.0)', 'pending'),
];

describe('Debug All step line', () => {
  it('shows only for a run of several steps', () => {
    assert.equal(showSteps(undefined), false);
    assert.equal(showSteps([step('One (net10.0)', 'running')]), false);
    assert.equal(showSteps(run), true);
  });

  it('names the live step and gives every other step a chip', () => {
    assert.equal(currentStepLine(run, 1), 'Step 2 of 4 · Api.Tests (net10.0)');
    assert.equal(currentStepLine(run, null), null);
    const chips = stepChips(run, 1);
    assert.deepEqual(chips.map((c) => c.index), [0, 2, 3]);
    assert.equal(chips[0].mark, '✓');
    assert.equal(chips[0].text, 'Core.Tests (net10.0) 42 passed');
    assert.equal(chips[1].mark, '○');
    assert.equal(chips[1].text, 'Api.Tests (net11.0)');
  });

  it('ends with the summary and the failed tests with their single-test launch', () => {
    const done: IdeDebugStep[] = [
      step('Core.Tests (net10.0)', 'passed', { summary: '3 passed' }),
      step('Api.Tests (net10.0)', 'passed'),
      step('Api.Tests (net11.0)', 'failed', { summary: '1 of 17 failed', failed_tests: ['Api.Tests.AddFails'], test_framework: 'net11.0' }),
      step('Mtp.Tests (net10.0)', 'error', { summary: 'exited with code 134' }),
    ];
    assert.equal(runSummary(done), '4 steps · 2 passed · 1 failed · 1 step couldn’t start');
    assert.equal(stepChips(done, null)[2].mark, '✗');
    assert.equal(stepChips(done, null)[3].mark, '!');
    const rows = failedTests(done);
    assert.deepEqual(rows.map((r) => [r.name, r.stepIndex]), [['Api.Tests.AddFails', 2]]);
    assert.deepEqual(failedTestLaunch(done[2], 'Api.Tests.AddFails', '/ws'), {
      program: '/ws/Api.Tests.csproj',
      args: [],
      cwd: '/ws',
      test_name: 'Api.Tests.AddFails',
      is_test: true,
      test_framework: 'net11.0',
    });
    const many = [step('A', 'failed', { failed_tests: ['a1', 'a2', 'a3'] }), step('B', 'failed', { failed_tests: ['b1', 'b2', 'b3'] })];
    assert.equal(failedTests(many).length, 5);
  });

  it('points a chip at the console separator the daemon prints', () => {
    assert.equal(stepSeparatorPrefix(1, 4), '── Step 2/4 ·');
  });
});

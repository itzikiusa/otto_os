import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const links = () => loadSource(new URL('../src/modules/mission-control/sourceLinks.ts', import.meta.url), {});

test('every kind names the module it opens in (no dead end)', () => {
  const { SOURCE_MODULE } = links();
  for (const k of ['session', 'swarm', 'goal_loop', 'workflow', 'review', 'product_story', 'pr', 'external_trigger']) {
    assert.ok(SOURCE_MODULE[k], `kind ${k} has no owning module`);
  }
});

test('id-only kinds route straight to their row', () => {
  const { directRoute } = links();
  assert.equal(directRoute('goal_loop', 'L1'), 'loops/L1');
  assert.equal(directRoute('product_story', 'S1'), 'product/S1');
  assert.equal(directRoute('pr', 'R1:42'), 'git/R1/pr/42');
  // Needs a lookup first.
  assert.equal(directRoute('workflow', 'run1'), null);
  assert.equal(directRoute('swarm', 'p1'), null);
});

test('pr source keys split on the last colon and reject junk', () => {
  const { prRoute, reviewRoute } = links();
  assert.equal(prRoute('a:b:7'), 'git/a%3Ab/pr/7');
  assert.equal(prRoute('R1:0'), null);
  assert.equal(prRoute('R1'), null);
  assert.equal(prRoute('R1:x'), null);
  assert.equal(reviewRoute('R1', 0), 'git/R1');
  assert.equal(reviewRoute('R1', 9), 'git/R1/pr/9');
});

test('stop uses the owner cancel endpoint only for running, stoppable kinds', () => {
  const { stopPath, isActive } = links();
  assert.equal(stopPath('goal_loop', 'L1'), '/goal-loops/L1/stop');
  assert.equal(stopPath('workflow', 'W1'), '/workflow-runs/W1/cancel');
  assert.equal(stopPath('review', 'V1'), '/reviews/V1/cancel');
  assert.equal(stopPath('pr', 'R1:1'), null);
  assert.equal(stopPath('session', 's'), null);
  assert.equal(isActive('running'), true);
  assert.equal(isActive('succeeded'), false);
  assert.equal(isActive('cancelled'), false);
});
